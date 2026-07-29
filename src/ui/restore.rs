//! The restore confirm modal's state transitions
//! ([`super::app::Mode::ConfirmRestore`]): opening it on the cursor file,
//! cancelling back to wherever `d` was pressed, and confirming — the one
//! place in the UI that runs an operation git cannot undo.
//!
//! The split of duties is deliberate. Opening resolves *which* paths, *what
//! kind* of discard, and *which copies* it will touch, and freezes all three
//! in [`App::restore_request`]; confirming only executes what was frozen. A
//! background refresh landing between the two can therefore change the diff
//! under the modal without ever changing what a confirm acts on.

use super::app::{App, Mode, ModeOrigin};
use crate::diff::FileChangeKind;
use crate::git::{DiffTarget, RestoreScope, StagingMode};

/// The file a pending [`Mode::ConfirmRestore`] will discard, resolved once at
/// open time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RestoreRequest {
    /// Repo-relative path of the file to restore.
    pub(super) path: String,
    /// The pre-rename path, for a renamed file only.
    ///
    /// A rename must be restored at *both* names in one invocation. Restoring
    /// just the new path deletes it (it isn't in `HEAD`) and leaves the
    /// original staged-deleted — a file at neither path. Not populated for
    /// copies, whose source path is unchanged and must not be touched.
    pub(super) old_path: Option<String>,
    /// Whether the path is untracked — git has no committed content to put
    /// back, so confirming deletes the file instead of restoring it. Drives
    /// both the modal's wording and which operation actually runs, so the
    /// question the reviewer answered and the op that runs can't disagree.
    pub(super) untracked: bool,
    /// Which copies of the path confirming will overwrite. The staged view
    /// gets [`RestoreScope::IndexOnly`] so it can never destroy working-tree
    /// edits it doesn't render — see [`RestoreScope`].
    pub(super) scope: RestoreScope,
}

impl RestoreRequest {
    /// The paths to hand [`crate::git::GitRunner::restore_paths`]: the file,
    /// plus its pre-rename name when there is one.
    fn paths(&self) -> Vec<&str> {
        let mut paths = vec![self.path.as_str()];
        if let Some(old) = &self.old_path {
            paths.push(old.as_str());
        }
        paths
    }
}

impl App {
    /// Opens the restore confirm modal for the file under the cursor.
    ///
    /// Refuses on a read-only diff target (a commit, a range, the file view,
    /// or a review session — none of which have a working tree this gesture
    /// could sensibly write) and on an empty diff, hinting in the footer
    /// rather than opening a modal that could only be cancelled.
    pub(super) fn open_confirm_restore(&mut self) {
        if self.target.staging_mode() == StagingMode::ReadOnly {
            self.set_status_message("read-only diff target");
            return;
        }
        let Some(index) = self
            .view
            .files
            .get(self.view.file_of_cursor())
            .map(|_| self.view.file_of_cursor())
        else {
            return;
        };
        if self.stage_ops.is_none() {
            self.set_status_message("restore unavailable (no git backend)");
            return;
        }
        let Some(file) = self.view.files.get(index) else {
            return;
        };
        let path = file.path.clone();
        // Only a rename carries its source path into the restore. A copy's
        // `old_path` names an untouched file that must stay untouched.
        let old_path = (file.kind == FileChangeKind::Renamed)
            .then(|| file.old_path.clone())
            .flatten();
        let untracked = self.untracked_paths.contains(&path);
        // The staged view renders the index only, so a restore launched from
        // it must stop at the index.
        let scope = if self.target == DiffTarget::Staged {
            RestoreScope::IndexOnly
        } else {
            RestoreScope::IndexAndWorktree
        };
        self.restore_request = Some(RestoreRequest {
            path,
            old_path,
            untracked,
            scope,
        });
        self.mode = Mode::ConfirmRestore {
            origin: ModeOrigin::capture(self.mode),
        };
    }

    /// Closes the modal without touching the repo, restoring the mode `d` was
    /// pressed from. A no-op (falls back to [`Mode::Normal`], never
    /// panicking) if called while the modal isn't open.
    ///
    /// Unlike [`App::confirm_confirm_restore`], this may return to
    /// [`Mode::Visual`] safely: cancelling runs no refresh, so the
    /// selection's anchor still indexes the rows it was taken against.
    pub(super) fn cancel_confirm_restore(&mut self) {
        self.restore_request = None;
        self.mode = match self.mode {
            Mode::ConfirmRestore { origin } => origin.restore(),
            other => other,
        };
    }

    /// Runs the frozen restore, then refreshes so the discarded file leaves
    /// the diff (or shrinks to whatever still differs from `HEAD`).
    ///
    /// Closes the modal *before* running, so a git failure surfaces in the
    /// footer of the restored view rather than under a modal that outlived
    /// its question. A failure leaves the working tree exactly as it was —
    /// `restore_paths` is a single git invocation, so there is no
    /// half-applied state to unwind. A no-op if called while the modal isn't
    /// open.
    ///
    /// A [`Mode::Visual`] origin collapses to [`Mode::Normal`] rather than
    /// being restored: the refresh below rebuilds `view.rows`, and a Visual
    /// anchor is a raw index into that list. Returning to Visual would leave
    /// the anchor pointing past the end of the new rows, where the next
    /// selection gesture slices out of range. [`App::maybe_auto_refresh`]
    /// declines to rebuild in Visual mode for exactly this reason; confirming
    /// *must* rebuild, so the selection is dropped instead.
    pub(super) fn confirm_confirm_restore(&mut self) {
        let Mode::ConfirmRestore { origin } = self.mode else {
            return;
        };
        let settled = match origin {
            ModeOrigin::Visual { .. } => Mode::Normal,
            other => other.restore(),
        };
        let Some(request) = self.restore_request.take() else {
            self.mode = settled;
            return;
        };
        self.mode = settled;

        let result = {
            let Some(ops) = self.stage_ops.as_deref() else {
                self.set_status_message("restore unavailable (no git backend)");
                return;
            };
            if request.untracked {
                ops.discard_untracked_file(&request.path)
            } else {
                ops.restore_paths(&request.paths(), request.scope)
            }
        };
        match result {
            Ok(()) => {
                let verb = if request.untracked {
                    "deleted"
                } else if request.scope == RestoreScope::IndexOnly {
                    "unstaged"
                } else {
                    "restored"
                };
                self.set_status_message(format!("{verb} {}", request.path));
                self.refresh();
            }
            Err(e) => self.set_status_message(e.to_string()),
        }
    }
}

#[cfg(test)]
#[path = "restore_tests.rs"]
mod tests;
