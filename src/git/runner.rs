//! [`GitRunner`]: shells out to `git` on `PATH` against a discovered repo root.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::branch::{BRANCH_LIST_FORMAT, LocalBranch, parse_branch_list};
use super::commit::{COMMIT_SUMMARY_FORMAT, CommitSummary, parse_commit_summary};
use super::diff::{DiffTarget, RawFilePatch, split_patches};
use super::error::{GitError, command_error, map_spawn_err};
use super::stash::{STASH_LIST_FORMAT, StashEntry, parse_stash_list};
use super::status::{FileStatus, StatusSnapshot, parse_porcelain_v2, parse_porcelain_v2_full};
use super::worktree::{WorktreeEntry, parse_worktree_list};

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
    /// disabled so the output shape is stable regardless of user config.
    pub fn diff(&self, target: &DiffTarget) -> Result<Vec<RawFilePatch>, GitError> {
        let mut args = vec!["diff", "--no-color", "--no-ext-diff", "-M"];
        match target {
            DiffTarget::WorkingTree => {}
            DiffTarget::Staged => args.push("--staged"),
            DiffTarget::Range(range) => args.push(range.as_str()),
        }
        let out = self.run_utf8(&args)?;
        Ok(split_patches(&out))
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
}
