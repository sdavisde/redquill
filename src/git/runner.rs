//! [`GitRunner`]: shells out to `git` on `PATH` against a discovered repo root.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::branch::{BRANCH_LIST_FORMAT, LocalBranch, parse_branch_list};
use super::commit::{COMMIT_SUMMARY_FORMAT, CommitSummary, parse_commit_summary};
use super::diff::{DiffTarget, RawFilePatch, split_patches};
use super::error::{GitError, command_error, map_spawn_err};
use super::log::{COMMIT_LOG_FORMAT, CommitLogEntry, CommitLogRange, parse_commit_log};
use super::ls_files::parse_ls_files_z;
use super::stash::{STASH_LIST_FORMAT, StashEntry, parse_stash_list};
use super::status::{FileStatus, StatusSnapshot, parse_porcelain_v2, parse_porcelain_v2_full};
use super::worktree::{WorktreeEntry, parse_worktree_list};

/// Git's well-known empty-tree object id — the tree with no entries, present
/// in every repository. Used as the base of a root commit's diff (a root
/// commit has no parent to diff against), so [`GitRunner::diff`] can show
/// every file in that commit as added.
const EMPTY_TREE_OID: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// Runs `git` commands against a single repository working tree.
///
/// Construct via [`GitRunner::discover`] (or [`GitRunner::discover_in`] for a
/// specific starting directory). All subsequent commands run with the repo root
/// as their working directory, so the user's git config is respected exactly as
/// `git` on their `PATH` would apply it.
#[derive(Debug, Clone)]
pub struct GitRunner {
    root: PathBuf,
}

impl GitRunner {
    /// Discovers the repository containing the current working directory.
    pub fn discover() -> Result<Self, GitError> {
        let cwd = std::env::current_dir().map_err(GitError::Spawn)?;
        Self::discover_in(cwd)
    }

    /// Discovers the repository containing `start`.
    pub fn discover_in(start: impl AsRef<Path>) -> Result<Self, GitError> {
        let output = Command::new("git")
            .current_dir(start.as_ref())
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .map_err(map_spawn_err)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(GitError::NotARepo(stderr));
        }

        let root = String::from_utf8(output.stdout)
            .map_err(GitError::Utf8)?
            .trim()
            .to_string();
        Ok(GitRunner {
            root: PathBuf::from(root),
        })
    }

    /// The absolute path to the repository root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Runs a git subcommand at the repo root, returning raw stdout bytes.
    fn run_raw(&self, args: &[&str]) -> Result<Vec<u8>, GitError> {
        let output = Command::new("git")
            .current_dir(&self.root)
            .args(args)
            .output()
            .map_err(map_spawn_err)?;

        if !output.status.success() {
            return Err(command_error(args, &output.status, &output.stderr));
        }
        Ok(output.stdout)
    }

    /// Runs a git subcommand and decodes stdout as UTF-8.
    fn run_utf8(&self, args: &[&str]) -> Result<String, GitError> {
        String::from_utf8(self.run_raw(args)?).map_err(GitError::Utf8)
    }

    /// Returns the parsed working-tree/index status for every changed path.
    ///
    /// The underlying invocation includes `--branch` (see
    /// [`GitRunner::status_with_branch`] for the branch/upstream/ahead-behind
    /// data that comes along for free on the same call); `parse_porcelain_v2`
    /// skips the `# branch.*` header fields, so this keeps returning exactly
    /// the file-status list it always has.
    pub fn status(&self) -> Result<Vec<FileStatus>, GitError> {
        let out = self.run_utf8(&["status", "--porcelain=v2", "--branch", "-z"])?;
        parse_porcelain_v2(&out)
    }

    /// Returns the working-tree/index status alongside branch sync state
    /// (name, upstream, ahead/behind), parsed from one `git status
    /// --porcelain=v2 --branch -z` invocation.
    pub fn status_with_branch(&self) -> Result<StatusSnapshot, GitError> {
        let out = self.run_utf8(&["status", "--porcelain=v2", "--branch", "-z"])?;
        parse_porcelain_v2_full(&out)
    }

    /// Returns raw per-file patches for the given diff target.
    ///
    /// Rename detection (`-M`) is enabled and color/external-diff drivers are
    /// disabled so the output shape is stable regardless of user config. For
    /// [`DiffTarget::Commit`], the revision and its base are passed as
    /// discrete argv elements (never interpolated into a shell string); the
    /// base is `<rev>^` (the commit's first parent), or git's well-known
    /// empty-tree object when `<rev>^` doesn't resolve (a root commit has no
    /// parent), so a root commit's diff shows every file as added.
    pub fn diff(&self, target: &DiffTarget) -> Result<Vec<RawFilePatch>, GitError> {
        let mut args = vec![
            "diff".to_string(),
            "--no-color".to_string(),
            "--no-ext-diff".to_string(),
            "-M".to_string(),
        ];
        match target {
            DiffTarget::WorkingTree => {}
            DiffTarget::Staged => args.push("--staged".to_string()),
            DiffTarget::Range(range) => args.push(range.clone()),
            DiffTarget::Review { base, branch } => {
                // Merge-base (three-dot) semantics, as a single argv
                // element — never two separate revs interpolated with a
                // shell-level `...`, so this is exactly as safe as `Range`.
                args.push(format!("{base}...{branch}"));
            }
            DiffTarget::Commit(rev) => {
                let parent = format!("{rev}^");
                let base = if self.rev_exists(&parent) {
                    parent
                } else {
                    EMPTY_TREE_OID.to_string()
                };
                args.push(base);
                args.push(rev.clone());
            }
            DiffTarget::File(_) => {
                // Not a comparison at all: the read-only file view
                // synthesizes its whole-file body directly from worktree
                // content (see `ui::file_view`), so this never shells out to
                // `git diff`. Kept as a real (non-panicking) match arm per
                // the repo's error-handling rules rather than `unreachable!`.
                return Ok(Vec::new());
            }
        }
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = self.run_utf8(&arg_refs)?;
        Ok(split_patches(&out))
    }

    /// Whether `rev` resolves to an object (`git rev-parse --verify <rev>`).
    /// Used to detect a root commit's missing parent without treating that
    /// as an error worth surfacing — a non-zero exit here is an expected
    /// "doesn't exist" answer, not a failure.
    fn rev_exists(&self, rev: &str) -> bool {
        Command::new("git")
            .current_dir(&self.root)
            .args(["rev-parse", "--verify", rev])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Returns a page of the commit log for the current branch (or `HEAD`
    /// when detached), newest first: up to `count` commits, skipping the
    /// first `skip` (page 2 of size 100 -> `count = 100, skip = 100`).
    /// Parsed from a NUL-delimited `git log --format=<COMMIT_LOG_FORMAT>`
    /// payload. An empty repository (no commits yet) yields an empty list
    /// rather than an error, since `git log` exits non-zero there — the same
    /// "expected, not an error" treatment as [`GitRunner::last_commit`]. Sets
    /// `GIT_TERMINAL_PROMPT=0`, matching every other git invocation this
    /// runner makes.
    pub fn commit_log(&self, count: u32, skip: u32) -> Result<Vec<CommitLogEntry>, GitError> {
        let count_arg = format!("--max-count={count}");
        let skip_arg = format!("--skip={skip}");
        let format_arg = format!("--format={COMMIT_LOG_FORMAT}");
        let output = Command::new("git")
            .current_dir(&self.root)
            .args(["log", &count_arg, &skip_arg, &format_arg])
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .map_err(map_spawn_err)?;

        if !output.status.success() {
            // No commits yet: `git log` exits non-zero on an empty
            // repository — expected, not an error.
            return Ok(Vec::new());
        }
        let text = String::from_utf8(output.stdout).map_err(GitError::Utf8)?;
        parse_commit_log(&text)
    }

    /// Returns the commits reachable from `range.head` but not from
    /// `range.base` (`git log <base>..<head> --format=<COMMIT_LOG_FORMAT>`),
    /// newest first — the Review launcher's Commits-tab ahead-of-base
    /// source. Base resolution (`origin/HEAD` -> `main` -> `master`) is
    /// entirely the caller's job (the UI layer's `resolve_review_base`
    /// already does this for the Branches tab): this takes only refs the
    /// caller already trusts, so it never itself guesses wrong. An empty range
    /// (`head` reachable from `base` — e.g. the current branch *is* the
    /// base) is `git log`'s own empty output, not a nonzero exit, so it
    /// yields an empty vec rather than an error; an unresolvable ref on
    /// either side is a genuine error. Sets `GIT_TERMINAL_PROMPT=0`,
    /// matching every other invocation this runner makes.
    pub fn commit_log_range(
        &self,
        range: &CommitLogRange,
    ) -> Result<Vec<CommitLogEntry>, GitError> {
        let range_arg = format!("{}..{}", range.base, range.head);
        let format_arg = format!("--format={COMMIT_LOG_FORMAT}");
        let args = ["log", range_arg.as_str(), format_arg.as_str()];
        let output = Command::new("git")
            .current_dir(&self.root)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .map_err(map_spawn_err)?;

        if !output.status.success() {
            return Err(command_error(&args, &output.status, &output.stderr));
        }
        let text = String::from_utf8(output.stdout).map_err(GitError::Utf8)?;
        parse_commit_log(&text)
    }

    /// Returns the parsed stash list, newest first. An empty list (no
    /// stashes) is not an error.
    pub fn stash_list(&self) -> Result<Vec<StashEntry>, GitError> {
        let format_arg = format!("--format={STASH_LIST_FORMAT}");
        let out = self.run_utf8(&["stash", "list", &format_arg])?;
        parse_stash_list(&out)
    }

    /// Returns the local branches, in `for-each-ref`'s default order,
    /// marking the currently checked-out branch and (when applicable) which
    /// worktree each is checked out in.
    pub fn branch_list(&self) -> Result<Vec<LocalBranch>, GitError> {
        let format_arg = format!("--format={BRANCH_LIST_FORMAT}");
        let out = self.run_utf8(&["for-each-ref", "refs/heads", &format_arg])?;
        parse_branch_list(&out)
    }

    /// Returns every worktree of this repository (the main worktree first),
    /// parsed from `git worktree list --porcelain`.
    pub fn worktree_list(&self) -> Result<Vec<WorktreeEntry>, GitError> {
        let out = self.run_utf8(&["worktree", "list", "--porcelain"])?;
        parse_worktree_list(&out)
    }

    /// Returns the repository's common git directory (`git rev-parse
    /// --git-common-dir`), canonicalized to an absolute path. This is the
    /// *shared* administrative directory — the same for every linked
    /// worktree of a repository — unlike [`GitRunner::root`], which returns
    /// the current worktree's toplevel and would silently diverge per
    /// worktree. Review-session paths (the managed worktrees themselves,
    /// and later the persisted review state) must resolve through this
    /// method so a review started from any worktree shares the same state.
    pub fn git_common_dir(&self) -> Result<PathBuf, GitError> {
        let out = self.run_utf8(&["rev-parse", "--git-common-dir"])?;
        let raw = PathBuf::from(out.trim());
        let joined = if raw.is_absolute() {
            raw
        } else {
            self.root.join(raw)
        };
        std::fs::canonicalize(&joined).map_err(GitError::Io)
    }

    /// Resolves `--review`'s default base ref when `--base` isn't given:
    /// the branch `origin/HEAD` points to, else local `main`, else local
    /// `master`. Returns [`GitError::NoDefaultBase`] (naming the `--base`
    /// flag in its `Display`) when none of the three resolve.
    pub fn default_base(&self) -> Result<String, GitError> {
        if let Ok(out) = self.run_utf8(&["symbolic-ref", "refs/remotes/origin/HEAD"])
            && let Some(name) = out.trim().strip_prefix("refs/remotes/origin/")
            && !name.is_empty()
        {
            return Ok(name.to_string());
        }
        if self.rev_exists("refs/heads/main") {
            return Ok("main".to_string());
        }
        if self.rev_exists("refs/heads/master") {
            return Ok("master".to_string());
        }
        Err(GitError::NoDefaultBase)
    }

    /// Creates a new worktree at `path`, checked out to `branch`
    /// (`git worktree add <path> <branch>`, fixed argv, never `--force`).
    /// `path`'s parent directories are created by `git` itself if missing.
    /// Fails with [`GitError::Command`] (carrying git's own stderr verbatim)
    /// when the branch doesn't exist, is already checked out in another
    /// worktree, or `path` already exists — the caller decides how to
    /// surface it; nothing is retried or forced.
    pub fn worktree_add(&self, path: &Path, branch: &str) -> Result<(), GitError> {
        let path_str = path.to_string_lossy().into_owned();
        let args = ["worktree", "add", path_str.as_str(), branch];
        let output = Command::new("git")
            .current_dir(&self.root)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .map_err(map_spawn_err)?;

        if !output.status.success() {
            return Err(command_error(&args, &output.status, &output.stderr));
        }
        Ok(())
    }

    /// Removes a worktree at `path` (`git worktree remove <path>`, fixed
    /// argv, never `--force`). Fails with [`GitError::Command`] (git's own
    /// stderr — e.g. a dirty tree) rather than retrying with force; the
    /// caller decides how to surface it and must leave the worktree and any
    /// persisted review state untouched on failure.
    pub fn worktree_remove(&self, path: &Path) -> Result<(), GitError> {
        let path_str = path.to_string_lossy().into_owned();
        let args = ["worktree", "remove", path_str.as_str()];
        let output = Command::new("git")
            .current_dir(&self.root)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .map_err(map_spawn_err)?;

        if !output.status.success() {
            return Err(command_error(&args, &output.status, &output.stderr));
        }
        Ok(())
    }

    /// Prunes stale worktree administrative records (`git worktree prune`,
    /// fixed argv). Run only after a successful
    /// [`GitRunner::worktree_remove`], to clear now-stale admin entries.
    pub fn worktree_prune(&self) -> Result<(), GitError> {
        self.run_raw(&["worktree", "prune"])?;
        Ok(())
    }

    /// Switches the working tree to branch `name` (`git switch -- <name>`).
    /// Never forces: a dirty tree that would be overwritten, or a branch
    /// already checked out in another worktree, surfaces as
    /// [`GitError::Command`] with git's own stderr, and the tree is left
    /// untouched — the caller decides how to report the failure.
    pub fn switch_branch(&self, name: &str) -> Result<(), GitError> {
        self.run_raw(&["switch", "--", name])?;
        Ok(())
    }

    /// Returns a one-line summary of the tip commit (`HEAD`): its abbreviated
    /// hash and subject. `Ok(None)` when the repository has no commits yet —
    /// `git log` exits non-zero there, which is expected rather than an error,
    /// so the panel simply shows no last-commit line.
    pub fn last_commit(&self) -> Result<Option<CommitSummary>, GitError> {
        let format_arg = format!("--format={COMMIT_SUMMARY_FORMAT}");
        match self.run_utf8(&["log", "-1", &format_arg]) {
            Ok(out) => parse_commit_summary(&out),
            Err(GitError::Command { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Reads a file's content at a git revision spec (`git show <spec>`,
    /// e.g. `HEAD:src/main.rs`, `:0:src/main.rs`). Used to source whole-file
    /// content for syntax highlighting, since the diff itself only carries
    /// changed lines. Returns `None` on any failure — an unknown revision,
    /// a spec with no such path, or non-UTF8 (e.g. binary) content — so
    /// callers can degrade to unhighlighted content rather than erroring.
    pub fn show_file(&self, spec: &str) -> Option<String> {
        let bytes = self.run_raw(&["show", spec]).ok()?;
        String::from_utf8(bytes).ok()
    }

    /// Returns every tracked file path, repo-relative, via `git ls-files -z`
    /// (NUL-delimited, parsed by [`parse_ls_files_z`]). Half of the fuzzy
    /// file finder's candidate source; combined with
    /// [`GitRunner::ls_files_untracked`] for the full set. Chosen over the
    /// `ignore` crate's walker for exact fidelity to git's own tracked set.
    pub fn ls_files(&self) -> Result<Vec<String>, GitError> {
        let out = self.run_utf8(&["ls-files", "-z"])?;
        Ok(parse_ls_files_z(&out))
    }

    /// Returns every untracked-but-not-ignored file path, repo-relative, via
    /// `git ls-files -z --others --exclude-standard`. The other half of the
    /// fuzzy file finder's candidate source.
    pub fn ls_files_untracked(&self) -> Result<Vec<String>, GitError> {
        let out = self.run_utf8(&["ls-files", "-z", "--others", "--exclude-standard"])?;
        Ok(parse_ls_files_z(&out))
    }

    /// Returns `path`'s blob SHA on `branch` (`git rev-parse --verify -q
    /// <branch>:<path>`, full SHA), for capturing at accept time and
    /// comparing again at reconciliation time to detect staleness.
    /// `Ok(None)` — never an error — whenever the spec doesn't resolve to a
    /// blob: the path doesn't exist at `branch`, or `branch` itself doesn't
    /// resolve. Deletion vs. a missing branch is the caller's concern to
    /// distinguish, not this method's. A genuine failure to run `git` at
    /// all (not found, spawn error, non-UTF8 output) still surfaces as
    /// `Err`.
    pub fn blob_sha(&self, branch: &str, path: &str) -> Result<Option<String>, GitError> {
        let spec = format!("{branch}:{path}");
        let args = ["rev-parse", "--verify", "-q", spec.as_str()];
        let output = Command::new("git")
            .current_dir(&self.root)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .map_err(map_spawn_err)?;
        if !output.status.success() {
            return Ok(None);
        }
        let out = String::from_utf8(output.stdout).map_err(GitError::Utf8)?;
        Ok(Some(out.trim().to_string()))
    }
}
