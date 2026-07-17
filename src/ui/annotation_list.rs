//! The annotation-list panel's handlers: opening/closing the panel, moving
//! its focus, and acting on the focused annotation (jump / edit / delete).
//! Kept out of `app.rs` so the coordinator stays thin; these methods drive
//! the annotation store and the diff view purely through `App`'s own state.
//!
//! [`App::delete_focused_annotation`] additionally saves-on-change (spec 08
//! Unit 6, task 7.2) via `App::persist_review_state`, a no-op outside a
//! review session — see `super::review_ops`'s module doc.

use super::App;
use super::Mode;

impl App {
    /// Toggles the annotation list panel: opens it from Normal/Visual, closes
    /// it from List. A no-op while another modal owns the keyboard.
    pub(super) fn toggle_list(&mut self) {
        match self.mode {
            Mode::List => self.mode = Mode::Normal,
            Mode::Compose
            | Mode::Staging
            | Mode::Panel { .. }
            | Mode::Search
            | Mode::Peek
            | Mode::Switcher
            | Mode::CommitMessage
            | Mode::Finder
            | Mode::ProjectSearch
            | Mode::EndReview { .. }
            | Mode::ConfirmRemoteOp { .. } => {}
            Mode::Normal | Mode::Visual { .. } => {
                if !self.annotations.is_empty() {
                    self.list_cursor = self.list_cursor.min(self.annotations.len() - 1);
                }
                self.mode = Mode::List;
            }
        }
    }

    /// Closes the annotation list panel, returning to [`Mode::Normal`].
    pub fn close_list(&mut self) {
        self.mode = Mode::Normal;
    }

    /// Moves the list panel's focus down one annotation, clamped at the
    /// last.
    pub fn list_move_down(&mut self) {
        if !self.annotations.is_empty() {
            self.list_cursor = (self.list_cursor + 1).min(self.annotations.len() - 1);
        }
    }

    /// Moves the list panel's focus up one annotation, clamped at the
    /// first.
    pub fn list_move_up(&mut self) {
        self.list_cursor = self.list_cursor.saturating_sub(1);
    }

    /// Switches to the focused annotation's file, places the cursor on its
    /// anchor row, and closes the list panel. A no-op if the store is
    /// empty or the annotation's file/anchor can no longer be found.
    pub fn jump_to_focused_annotation(&mut self) {
        let Some(id) = self.annotations.iter().nth(self.list_cursor).map(|a| a.id) else {
            self.mode = Mode::Normal;
            return;
        };
        self.jump_to_annotation(id);
    }

    fn jump_to_annotation(&mut self, id: usize) {
        let Some(annotation) = self.annotations.iter().find(|a| a.id == id) else {
            self.mode = Mode::Normal;
            return;
        };
        let target = annotation.target.clone();
        let path = target.path().to_string();
        if let Some(index) = self.view.files.iter().position(|f| f.path == path) {
            // Expand the target section so line/hunk anchors are reachable,
            // then resolve the anchor against the whole buffer (a File target
            // lands on the section header; line/hunk/range targets resolve
            // within this file's row span). Fall back to the section header
            // if the specific anchor row can no longer be found.
            self.view.set_collapsed(&path, false);
            self.rebuild_rows();
            self.view.cursor = self
                .view
                .anchor_row_in_buffer(&target)
                .unwrap_or_else(|| self.view.header_row_of_file[index]);
            self.view.scroll = 0;
            self.view.ensure_visible();
            self.mode = Mode::Normal;
            return;
        }
        // Not in the currently-loaded buffer (the common case for a `(=)`
        // annotation: the file view that made it is rarely still open) --
        // if it's a worktree-file-content target (spec 06 Unit 3),
        // `open_file_view` re-opens (or replaces, if a file view is already
        // showing a different file) that file at its anchor line, exactly
        // the "navigate back to its file-view location" behavior the
        // annotation list panel owes these entries. `open_file_view` sets
        // `Mode::Normal` itself. Every other target shape whose path isn't
        // loaded (e.g. a commit-authored annotation while a different diff
        // target is active) has no reliable file-view equivalent -- opening
        // the *live* worktree file for a historical target's line numbers
        // would show misleading content -- so it degrades to the existing
        // no-op-but-close-the-list behavior.
        if let Some(line) = target.worktree_anchor_line() {
            self.open_file_view(path, Some(line));
            return;
        }
        self.mode = Mode::Normal;
    }

    /// Opens Compose pre-filled with the focused annotation for editing.
    pub fn edit_focused_annotation(&mut self) {
        let Some(id) = self.annotations.iter().nth(self.list_cursor).map(|a| a.id) else {
            return;
        };
        self.open_compose_for(id);
    }

    /// Deletes the focused annotation. No confirmation — deletion is cheap
    /// to redo.
    pub fn delete_focused_annotation(&mut self) {
        let Some(id) = self.annotations.iter().nth(self.list_cursor).map(|a| a.id) else {
            return;
        };
        let _ = self.annotations.remove(id);
        if self.annotations.is_empty() {
            self.list_cursor = 0;
        } else {
            self.list_cursor = self.list_cursor.min(self.annotations.len() - 1);
        }
        self.refresh_rows();
        // Save-on-change (spec 08 Unit 6, task 7.2) — see `review_ops`'s
        // module doc for why this is safe to call unconditionally outside a
        // review session.
        self.persist_review_state();
    }
}
