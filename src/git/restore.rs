//! Discarding a file's uncommitted changes.
//!
//! The counterpart to [`super::stage`], and the opposite contract: staging
//! only ever writes the index, while everything here destroys work. Once a
//! path is restored, its prior content is not recoverable from git — there is
//! no reflog entry for an uncommitted edit. Callers are expected to gate
//! these behind an explicit user confirmation; nothing in this module prompts.

use std::process::Command;

use super::error::{GitError, command_error, map_spawn_err};
use super::runner::GitRunner;

/// Which copies of a path a restore puts back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreScope {
    /// The index and the working tree both — the working-tree view's
    /// "put this file back", which discards staged and unstaged work alike.
    IndexAndWorktree,
    /// The index only, leaving the working tree untouched.
    ///
    /// This is what the staged view (`--staged`) must use: that view renders
    /// only what is in the index, so a restore launched from it may not
    /// destroy working-tree edits the reviewer was never shown.
    IndexOnly,
}

impl GitRunner {
    /// Discards uncommitted changes to every path in `paths`, returning each
    /// to its `HEAD` content, across the copies named by `scope`.
    ///
    /// `--source=HEAD` is passed explicitly rather than leaning on the
    /// implicit default `--staged` selects, so the restore source cannot
    /// shift with the flag combination.
    ///
    /// `paths` is a slice rather than a single path so a rename can be undone
    /// atomically: passing both the new and the original path in one
    /// invocation restores the original from `HEAD` and drops the new one
    /// (which `HEAD` doesn't have), leaving the file back where it started.
    /// Restoring only the new path would delete it *and* leave the original
    /// staged-deleted — a file at neither path.
    ///
    /// Errors on a path git doesn't track — an untracked file has no `HEAD`
    /// content to restore to, and goes through
    /// [`GitRunner::discard_untracked_file`] instead. An empty `paths` is a
    /// no-op rather than an unbounded restore: `git restore` with no pathspec
    /// would be an error, but relying on that would put "restore everything"
    /// one argv bug away.
    pub fn restore_paths(&self, paths: &[&str], scope: RestoreScope) -> Result<(), GitError> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut args = vec!["restore", "--source=HEAD", "--staged"];
        if scope == RestoreScope::IndexAndWorktree {
            args.push("--worktree");
        }
        args.push("--");
        args.extend_from_slice(paths);
        self.run_restore(&args)
    }

    /// Deletes an untracked `path` from the working tree.
    ///
    /// For a file git has never recorded, "discard its changes" can only mean
    /// removing it — there is no committed content to fall back to. Done
    /// through `std::fs` rather than `git clean` so exactly the one named file
    /// is removed with no pathspec expansion, and so the operation cannot
    /// widen to a directory.
    ///
    /// Errors when the path is already gone. Callers reach this only for a
    /// path the diff just listed as untracked, so a missing file means the
    /// caller's picture of the tree is wrong — reporting that beats a silent
    /// success that tells the reviewer their file was deleted when nothing
    /// happened.
    pub fn discard_untracked_file(&self, path: &str) -> Result<(), GitError> {
        std::fs::remove_file(self.root().join(path)).map_err(GitError::Io)
    }

    /// Runs a restore subcommand at the repo root, discarding stdout and
    /// erroring on a non-zero exit. Mirrors [`super::stage`]'s `run_index`,
    /// plus the `GIT_TERMINAL_PROMPT=0` every spawned git inherits here.
    fn run_restore(&self, args: &[&str]) -> Result<(), GitError> {
        let output = Command::new("git")
            .current_dir(self.root())
            .env("GIT_TERMINAL_PROMPT", "0")
            .args(args)
            .output()
            .map_err(map_spawn_err)?;
        if output.status.success() {
            return Ok(());
        }
        Err(command_error(args, &output.status, &output.stderr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// A throwaway repo with no commits. `--source=HEAD` cannot resolve
    /// there, so every restore below fails before touching anything — and a
    /// failed restore reports the argv it ran, which is what these tests
    /// read back.
    fn commitless_repo() -> TempDir {
        let tmp = TempDir::new().unwrap();
        let out = Command::new("git")
            .current_dir(tmp.path())
            .args(["init", "-q"])
            .output()
            .expect("failed to spawn git");
        assert!(out.status.success(), "git init failed");
        tmp
    }

    /// The exact argv `restore_paths` handed to git, recovered from the
    /// `GitError::Command` a failing restore carries. Callers must pass
    /// space-free paths so the joined command string splits back cleanly.
    fn argv_of(paths: &[&str], scope: RestoreScope) -> Vec<String> {
        let tmp = commitless_repo();
        let runner = GitRunner::discover_in(tmp.path()).unwrap();
        assert!(
            runner
                .root()
                .canonicalize()
                .unwrap()
                .starts_with(tmp.path().canonicalize().unwrap()),
            "runner root escaped the tempdir"
        );
        match runner.restore_paths(paths, scope) {
            Err(GitError::Command { command, .. }) => {
                command.split(' ').map(str::to_string).collect()
            }
            other => panic!("expected a failing restore carrying its argv, got {other:?}"),
        }
    }

    #[test]
    fn every_restore_starts_with_the_fixed_restore_source_head_staged_prefix() {
        for scope in [RestoreScope::IndexAndWorktree, RestoreScope::IndexOnly] {
            let argv = argv_of(&["a.rs"], scope);
            assert_eq!(
                &argv[..3],
                &["restore", "--source=HEAD", "--staged"],
                "{scope:?} must not shift the restore source or drop --staged"
            );
        }
    }

    #[test]
    fn the_worktree_flag_appears_only_for_the_index_and_worktree_scope() {
        // The index-only scope is what the staged view uses; a `--worktree`
        // leaking into it would destroy unshown working-tree edits.
        for (scope, expected) in [
            (RestoreScope::IndexAndWorktree, true),
            (RestoreScope::IndexOnly, false),
        ] {
            for paths in [&["a.rs"][..], &["a.rs", "b.rs"][..]] {
                let argv = argv_of(paths, scope);
                assert_eq!(
                    argv.iter().any(|a| a == "--worktree"),
                    expected,
                    "{scope:?} with {paths:?} produced {argv:?}"
                );
            }
        }
    }

    #[test]
    fn the_pathspec_always_sits_after_a_double_dash_separator() {
        // A path that looks like a flag must never be readable as one: the
        // separator is what guarantees that, whatever the path's content.
        const FIXED: [&str; 4] = ["restore", "--source=HEAD", "--staged", "--worktree"];
        for scope in [RestoreScope::IndexAndWorktree, RestoreScope::IndexOnly] {
            let paths = ["--force", "-f", "a.rs"];
            let argv = argv_of(&paths, scope);
            let sep = argv
                .iter()
                .position(|a| a == "--")
                .expect("the -- separator must be present");
            assert_eq!(
                &argv[sep + 1..],
                &paths[..],
                "{scope:?}: the pathspec must be exactly the caller's paths, in order"
            );
            assert!(
                argv[..sep].iter().all(|a| FIXED.contains(&a.as_str())),
                "{scope:?}: only fixed flags may precede the separator: {argv:?}"
            );
        }
    }
}
