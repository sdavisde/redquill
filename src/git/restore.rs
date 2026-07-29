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
