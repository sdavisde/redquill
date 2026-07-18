//! State for the Review launcher modal ([`super::app::Mode::ReviewLauncher`]):
//! a tabbed overlay reachable from anywhere (`R`, `Scope::Global`) that hosts
//! branch review (Branches tab) and single-commit review (Commits tab)
//! behind one entry point, replacing the panel-only review-branch modal.
//! Modeled on [`super::switcher::SwitcherState`]'s tab/cursor shape and
//! [`super::app::ModeOrigin`]'s origin-restore pattern.
//!
//! This module ships the shell only: opening/closing, tab switching, cursor
//! movement, and tab memory. Each tab's real list and `Enter` behavior land
//! in later work (Branches, Commits) — until then `Enter` is a documented
//! no-op and the cursor never moves off zero (see
//! [`App::review_launcher_row_count`]).

use super::app::{App, Mode, ModeOrigin};

/// Which tab of the Review launcher is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LauncherTab {
    /// Local branches, for starting a worktree-backed review session (the
    /// default tab — the first launcher open of a process lands here).
    #[default]
    Branches,
    /// Commits ahead of the auto-resolved base (or the full log, once
    /// toggled), for opening a single read-only commit view.
    Commits,
}

impl LauncherTab {
    /// The other tab — there are only two, so switching always toggles
    /// rather than needing a direction.
    fn toggle(self) -> LauncherTab {
        match self {
            LauncherTab::Branches => LauncherTab::Commits,
            LauncherTab::Commits => LauncherTab::Branches,
        }
    }
}

impl App {
    /// Opens the Review launcher (`R`, `Scope::Global`): captures the exact
    /// mode `R` was pressed from (via [`ModeOrigin::capture`]) so `Esc`/
    /// [`App::close_review_launcher`] can restore it, and reopens on
    /// whichever tab was last active this process (`App::last_launcher_tab`;
    /// the first open of a session lands on Branches — see [`LauncherTab`]'s
    /// `Default`). Unlike the review-branch modal it replaces, this never
    /// rejects mid-review-session: an in-session guard belongs to the
    /// Branches tab's own `Enter` handler, not to opening the launcher
    /// itself.
    pub(super) fn open_review_launcher(&mut self) {
        let origin = ModeOrigin::capture(self.mode);
        self.mode = Mode::ReviewLauncher {
            tab: self.last_launcher_tab,
            cursor: 0,
            origin,
        };
    }

    /// Closes the Review launcher without acting, restoring the mode it was
    /// opened from exactly (panel cursor/tab included, via `ModeOrigin`). A
    /// no-op (falls back to `Mode::Normal`, never panicking) if called while
    /// the modal isn't open — defensive rather than relied upon.
    pub(super) fn close_review_launcher(&mut self) {
        self.mode = match self.mode {
            Mode::ReviewLauncher { origin, .. } => origin.restore(),
            other => other,
        };
    }

    /// Switches the launcher between its two tabs, resetting the cursor to
    /// the top (each tab's list is independent) and remembering the new tab
    /// in `last_launcher_tab` so the next open this process lands back here.
    /// A no-op unless the launcher is open.
    pub(super) fn review_launcher_switch_tab(&mut self) {
        let Mode::ReviewLauncher { tab, cursor, .. } = &mut self.mode else {
            return;
        };
        *tab = tab.toggle();
        *cursor = 0;
        self.last_launcher_tab = *tab;
    }

    /// Moves the launcher's cursor down one row, clamped at the last row of
    /// whichever list backs the active tab (or pinned at 0 on an empty
    /// list). A no-op unless the launcher is open.
    pub(super) fn review_launcher_move_down(&mut self) {
        let len = self.review_launcher_row_count();
        let Mode::ReviewLauncher { cursor, .. } = &mut self.mode else {
            return;
        };
        *cursor = if len == 0 {
            0
        } else {
            (*cursor + 1).min(len - 1)
        };
    }

    /// Moves the launcher's cursor up one row, clamped at the first. A no-op
    /// unless the launcher is open.
    pub(super) fn review_launcher_move_up(&mut self) {
        let Mode::ReviewLauncher { cursor, .. } = &mut self.mode else {
            return;
        };
        *cursor = cursor.saturating_sub(1);
    }

    /// The active tab's row count. Always `0` until the Branches/Commits
    /// tabs are wired up to real data — kept as its own method now so that
    /// work only needs to change this one place, mirroring how
    /// [`super::git_panel::App::panel_row_count`] centralizes the git
    /// panel's per-tab length.
    fn review_launcher_row_count(&self) -> usize {
        0
    }

    /// The launcher's `Enter` gesture. Deliberately inert for now: starting
    /// a branch review (Branches tab) and opening a commit (Commits tab)
    /// both land in later work. Kept as a named method (rather than leaving
    /// `Enter` unbound) so the modal key table's `Confirm` row has something
    /// real to dispatch to, and so wiring up either tab later is a
    /// one-function change.
    pub(super) fn review_launcher_confirm(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::FileDiff;
    use crate::git::RawFilePatch;
    use crate::ui::app::PanelTab;

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

    fn app() -> App {
        App::new(vec![sample_file()])
    }

    // -- LauncherTab::toggle -------------------------------------------------

    #[test]
    fn toggle_switches_between_branches_and_commits() {
        assert_eq!(LauncherTab::Branches.toggle(), LauncherTab::Commits);
        assert_eq!(LauncherTab::Commits.toggle(), LauncherTab::Branches);
    }

    #[test]
    fn default_tab_is_branches() {
        assert_eq!(LauncherTab::default(), LauncherTab::Branches);
    }

    // -- App::open_review_launcher / close_review_launcher: origin restore --

    #[test]
    fn open_from_normal_lands_on_branches_and_close_restores_normal() {
        let mut app = app();
        assert_eq!(app.mode, Mode::Normal);
        app.open_review_launcher();
        assert_eq!(
            app.mode,
            Mode::ReviewLauncher {
                tab: LauncherTab::Branches,
                cursor: 0,
                origin: ModeOrigin::Normal,
            }
        );
        app.close_review_launcher();
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn open_from_visual_and_close_restores_the_anchor() {
        let mut app = app();
        app.mode = Mode::Visual { anchor: 3 };
        app.open_review_launcher();
        assert_eq!(
            app.mode,
            Mode::ReviewLauncher {
                tab: LauncherTab::Branches,
                cursor: 0,
                origin: ModeOrigin::Visual { anchor: 3 },
            }
        );
        app.close_review_launcher();
        assert_eq!(app.mode, Mode::Visual { anchor: 3 });
    }

    #[test]
    fn open_from_panel_and_close_restores_the_cursor_and_tab() {
        let mut app = app();
        app.mode = Mode::Panel {
            cursor: 2,
            tab: PanelTab::History,
        };
        app.open_review_launcher();
        assert_eq!(
            app.mode,
            Mode::ReviewLauncher {
                tab: LauncherTab::Branches,
                cursor: 0,
                origin: ModeOrigin::Panel {
                    cursor: 2,
                    tab: PanelTab::History,
                },
            }
        );
        app.close_review_launcher();
        assert_eq!(
            app.mode,
            Mode::Panel {
                cursor: 2,
                tab: PanelTab::History,
            }
        );
    }

    #[test]
    fn close_while_not_open_is_a_no_op() {
        // `Mode::ReviewLauncher` always carries its own origin, so "never
        // opened" means some other mode entirely — the defensive `other =>
        // other` fallback, mirroring `close_switcher`'s identical guard.
        let mut app = app();
        assert_eq!(app.mode, Mode::Normal);
        app.close_review_launcher();
        assert_eq!(app.mode, Mode::Normal);
    }

    // -- Tab switching / tab memory ------------------------------------------

    #[test]
    fn switch_tab_toggles_and_resets_cursor() {
        let mut app = app();
        app.mode = Mode::ReviewLauncher {
            tab: LauncherTab::Branches,
            cursor: 0,
            origin: ModeOrigin::Normal,
        };
        app.review_launcher_switch_tab();
        assert_eq!(
            app.mode,
            Mode::ReviewLauncher {
                tab: LauncherTab::Commits,
                cursor: 0,
                origin: ModeOrigin::Normal,
            }
        );
    }

    #[test]
    fn tab_memory_survives_close_and_reopen() {
        let mut app = app();
        assert_eq!(app.last_launcher_tab, LauncherTab::Branches);
        app.open_review_launcher();
        app.review_launcher_switch_tab();
        assert_eq!(app.last_launcher_tab, LauncherTab::Commits);
        app.close_review_launcher();
        app.open_review_launcher();
        assert_eq!(
            app.mode,
            Mode::ReviewLauncher {
                tab: LauncherTab::Commits,
                cursor: 0,
                origin: ModeOrigin::Normal,
            },
            "reopening this process must land back on the last-used tab"
        );
    }

    #[test]
    fn switch_tab_is_a_no_op_outside_the_launcher() {
        let mut app = app();
        assert_eq!(app.mode, Mode::Normal);
        app.review_launcher_switch_tab();
        assert_eq!(app.mode, Mode::Normal);
    }

    // -- Cursor movement (no-op today: the placeholder list is always empty) -

    #[test]
    fn move_down_and_up_stay_at_zero_with_no_list_data_yet() {
        let mut app = app();
        app.open_review_launcher();
        app.review_launcher_move_down();
        assert_eq!(
            app.mode,
            Mode::ReviewLauncher {
                tab: LauncherTab::Branches,
                cursor: 0,
                origin: ModeOrigin::Normal,
            }
        );
        app.review_launcher_move_up();
        assert_eq!(
            app.mode,
            Mode::ReviewLauncher {
                tab: LauncherTab::Branches,
                cursor: 0,
                origin: ModeOrigin::Normal,
            }
        );
    }

    #[test]
    fn confirm_is_inert_and_keeps_the_modal_open() {
        let mut app = app();
        app.open_review_launcher();
        app.review_launcher_confirm();
        assert!(matches!(app.mode, Mode::ReviewLauncher { .. }));
    }
}
