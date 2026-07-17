//! The pull/push confirm modal's state transitions
//! ([`super::app::Mode::ConfirmRemoteOp`]): opening it (capturing the panel
//! cursor/tab `p`/`P` were pressed from), cancelling back to the panel, and
//! confirming (running the pending [`crate::git::RemoteOp`] through the
//! existing [`App::request_remote_op`], unchanged).

use crate::git::RemoteOp;

use super::app::{App, Mode};

impl App {
    /// Opens the pull/push confirm modal for `op`, capturing the panel
    /// cursor/tab so [`App::cancel_confirm_remote_op`]/
    /// [`App::confirm_remote_op`] can restore [`Mode::Panel`] exactly. Only
    /// ever called from the focused git panel (`p`/`P` are panel-scope
    /// bindings — see [`super::modes::handle_panel_key`]); a no-op from any
    /// other mode, since there is no panel cursor/tab to capture.
    pub(super) fn open_confirm_remote_op_modal(&mut self, op: RemoteOp) {
        if let Mode::Panel { cursor, tab } = self.mode {
            self.mode = Mode::ConfirmRemoteOp { op, cursor, tab };
        }
    }

    /// Closes the confirm modal without running anything, restoring the
    /// panel it was opened from. A no-op (falls back to `Mode::Normal`,
    /// never panicking) if called while the modal isn't open — defensive
    /// rather than relied upon, mirroring [`App::cancel_end_review`].
    pub(super) fn cancel_confirm_remote_op(&mut self) {
        self.mode = match self.mode {
            Mode::ConfirmRemoteOp { cursor, tab, .. } => Mode::Panel { cursor, tab },
            other => other,
        };
    }

    /// Runs the confirmed op: restores [`Mode::Panel`] first (so
    /// [`App::request_remote_op`]'s footer/status-message and single-flight
    /// guard behave exactly as they do for the unprompted `f` fetch — the
    /// modal never lingers on top of a spawned op), then requests it through
    /// the unchanged existing path. A no-op if called while the modal isn't
    /// open.
    pub(super) fn confirm_remote_op(&mut self) {
        if let Mode::ConfirmRemoteOp { op, cursor, tab } = self.mode {
            self.mode = Mode::Panel { cursor, tab };
            self.request_remote_op(op);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::FileDiff;
    use crate::git::{DiffTarget, RawFilePatch};

    use super::super::app::PanelTab;

    fn sample_file() -> FileDiff {
        let raw = "\
diff --git a/src/main.rs b/src/main.rs
index 111..222 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,2 +1,2 @@
 fn main() {
-    old();
+    new();
";
        FileDiff::from_patch(&RawFilePatch {
            path: "src/main.rs".to_string(),
            old_path: None,
            raw: raw.to_string(),
            is_binary: false,
        })
        .unwrap()
    }

    fn panel_app() -> App {
        let mut app = App::new(vec![sample_file()]);
        app.target = DiffTarget::Review {
            base: "main".to_string(),
            branch: "feature".to_string(),
        };
        app.mode = Mode::Panel {
            cursor: 2,
            tab: PanelTab::Changes,
        };
        app
    }

    #[test]
    fn open_captures_the_panel_cursor_and_tab() {
        let mut app = panel_app();
        app.open_confirm_remote_op_modal(RemoteOp::Push);
        assert_eq!(
            app.mode,
            Mode::ConfirmRemoteOp {
                op: RemoteOp::Push,
                cursor: 2,
                tab: PanelTab::Changes,
            }
        );
    }

    #[test]
    fn open_from_a_non_panel_mode_is_a_no_op() {
        let mut app = panel_app();
        app.mode = Mode::Normal;
        app.open_confirm_remote_op_modal(RemoteOp::Pull);
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn cancel_restores_the_panel_cursor_and_tab() {
        let mut app = panel_app();
        app.open_confirm_remote_op_modal(RemoteOp::Pull);
        app.cancel_confirm_remote_op();
        assert_eq!(
            app.mode,
            Mode::Panel {
                cursor: 2,
                tab: PanelTab::Changes,
            }
        );
    }

    #[test]
    fn cancel_outside_the_modal_is_a_no_op() {
        let mut app = panel_app();
        app.cancel_confirm_remote_op();
        assert_eq!(
            app.mode,
            Mode::Panel {
                cursor: 2,
                tab: PanelTab::Changes,
            }
        );
    }

    #[test]
    fn confirm_restores_the_panel_and_spawns_the_pending_op() {
        let mut app = panel_app();
        app.set_repo_root(std::path::PathBuf::from("/tmp/review-worktree"));
        app.open_confirm_remote_op_modal(RemoteOp::Pull);
        app.confirm_remote_op();
        assert_eq!(
            app.mode,
            Mode::Panel {
                cursor: 2,
                tab: PanelTab::Changes,
            }
        );
        assert_eq!(app.running_op_label(), Some("pull"));
    }

    #[test]
    fn confirm_outside_the_modal_is_a_no_op() {
        let mut app = panel_app();
        app.confirm_remote_op();
        assert_eq!(app.running_op_label(), None);
    }
}
