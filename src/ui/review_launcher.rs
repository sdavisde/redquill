//! State for the Review launcher modal ([`super::app::Mode::ReviewLauncher`]):
//! a tabbed overlay reachable from anywhere (`R`, `Scope::Global`) that hosts
//! branch review (Branches tab) and single-commit review (Commits tab)
//! behind one entry point — the sole in-app entry point for starting a
//! branch review. Modeled on [`super::switcher::SwitcherState`]'s tab/cursor
//! shape and [`super::app::ModeOrigin`]'s origin-restore pattern.
//!
//! The Branches tab is wired to the real branch-review flow: `App`'s
//! [`ensure_review_worktree`]/[`resolve_review_base`]/
//! [`load_reconciled_review_state`] machinery — the same "ensure a review
//! session" path the CLI's `--review` flag runs through (see
//! [`super::review_session`]'s module doc) — via
//! [`App::confirm_launcher_branch_review`]. The Commits tab's real list and
//! `Enter` behavior land in follow-up work; until then its cursor never
//! moves off zero (see [`App::review_launcher_row_count`]).

use crate::git::{DiffTarget, GitRunner};

use super::app::{App, Mode, ModeOrigin};
use super::review_session::{
    ensure_review_worktree, load_reconciled_review_state, resolve_review_base,
    resolve_review_state_path,
};

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
        let tab = self.last_launcher_tab;
        self.mode = Mode::ReviewLauncher {
            tab,
            cursor: 0,
            origin,
        };
        if tab == LauncherTab::Branches {
            self.load_launcher_branches();
        }
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
        let new_tab = *tab;
        self.last_launcher_tab = new_tab;
        if new_tab == LauncherTab::Branches {
            self.load_launcher_branches();
        }
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

    /// The active tab's row count: `launcher_branches`' length on Branches,
    /// still `0` on Commits until its data lands — kept as its own method so
    /// that work only needs to change this one arm, mirroring how
    /// [`super::git_panel::App::panel_row_count`] centralizes the git
    /// panel's per-tab length.
    fn review_launcher_row_count(&self) -> usize {
        let Mode::ReviewLauncher { tab, .. } = self.mode else {
            return 0;
        };
        match tab {
            LauncherTab::Branches => self.launcher_branches.len(),
            LauncherTab::Commits => 0,
        }
    }

    /// Populates `launcher_branches` from the attached git backend: local
    /// branches excluding the one currently checked out (identical to the
    /// retired review-branch modal's filter — see [`super::review_session`]'s
    /// module doc for why this is the shared "ensure a review session" data
    /// path). Degrades to an empty list — never a status message — without a
    /// backend or on a read error: opening the launcher never refuses just
    /// because the backend can't answer; the Branches tab's own empty-state
    /// row already covers "nothing to show".
    fn load_launcher_branches(&mut self) {
        self.launcher_branches = self
            .stage_ops
            .as_deref()
            .and_then(|ops| ops.branch_list().ok())
            .map(|branches| branches.into_iter().filter(|b| !b.is_current).collect())
            .unwrap_or_default();
    }

    /// The launcher's `Enter` gesture: dispatches on the active tab. The
    /// Commits tab is still inert — its data lands in follow-up work.
    pub(super) fn review_launcher_confirm(&mut self) {
        let Mode::ReviewLauncher { tab, cursor, .. } = self.mode else {
            return;
        };
        match tab {
            LauncherTab::Branches => self.confirm_launcher_branch_review(cursor),
            LauncherTab::Commits => {}
        }
    }

    /// Branches-tab `Enter`: starts a worktree-backed review session on the
    /// highlighted branch, reusing the exact machinery the retired
    /// review-branch modal drove — [`resolve_review_base`]
    /// (`origin/HEAD` → `main` → `master`), [`ensure_review_worktree`],
    /// review-state reconciliation, and [`App::reroot`] onto
    /// [`DiffTarget::Review`] — see [`super::review_session`]'s module doc
    /// for why this is the one "ensure a review session" path shared with
    /// the CLI's `--review` flag.
    ///
    /// Guarded first by the in-session check: starting a second review from
    /// inside one is unsupported (nested worktrees would tangle the
    /// banner/finish bookkeeping), so this only ever sets a status message
    /// naming `q` — no branch lookup, no worktree call, no mode change.
    /// Guarded second by the same single-in-flight rule
    /// [`App::request_remote_op`]/[`App::switcher_confirm`] enforce: a
    /// running fetch/pull/push blocks starting a review.
    ///
    /// On success the launcher closes into the re-rooted review view (see
    /// [`App::close_review_launcher_after_start`]); `after_panel_coherence`
    /// re-follows the diff when the invocation origin was the panel. On
    /// failure the launcher also closes back to its origin, with git's
    /// message surfaced as a status line — the launcher is one keystroke
    /// away again for a retry.
    fn confirm_launcher_branch_review(&mut self, cursor: usize) {
        if self.in_review_session() {
            self.set_status_message(format!(
                "already reviewing {} \u{2014} press q to finish or pause",
                self.review_branch().unwrap_or("this branch")
            ));
            return;
        }
        if let Some(label) = self.running_op_label() {
            self.set_status_message(format!(
                "{label} is running \u{2014} wait before starting a review"
            ));
            return;
        }
        let Some(branch) = self.launcher_branches.get(cursor).map(|b| b.name.clone()) else {
            return;
        };
        let Some(ops) = self.stage_ops.as_deref() else {
            self.set_status_message("review unavailable (no git backend)");
            return;
        };

        let base = match resolve_review_base(ops, None) {
            Ok(base) => base,
            Err(e) => {
                self.set_status_message(format!("review failed: {e}"));
                self.close_review_launcher();
                return;
            }
        };
        let worktree_path = match ensure_review_worktree(ops, &branch) {
            Ok(path) => path,
            Err(e) => {
                self.set_status_message(format!("review failed: {e}"));
                self.close_review_launcher();
                return;
            }
        };
        let session_runner = match GitRunner::discover_in(&worktree_path) {
            Ok(runner) => runner,
            Err(e) => {
                self.set_status_message(format!("review failed: {e}"));
                self.close_review_launcher();
                return;
            }
        };

        // Resolved (and reconciled) *before* `reroot` swaps `self.stage_ops`
        // out from under `ops` — `session_runner` reads the branch's current
        // blob SHAs exactly as truthfully as the pre-reroot backend would.
        let state_path = resolve_review_state_path(ops).ok();
        let reconciled = state_path
            .as_ref()
            .map(|path| load_reconciled_review_state(&session_runner, path, &branch));
        // The backend `finish_review` later runs `worktree_remove`/`prune`
        // through: discovered fresh at the *current* (pre-reroot) repo root,
        // since it must be rooted outside the worktree being removed, which
        // the worktree about to become `self.repo_root` is not.
        let origin_runner = self
            .repo_root
            .as_deref()
            .and_then(|root| GitRunner::discover_in(root).ok());

        let target = DiffTarget::Review {
            base,
            branch: branch.clone(),
        };
        match self.reroot(session_runner, target) {
            Ok(()) => {
                if let Some(origin) = origin_runner {
                    self.set_review_origin_ops(Box::new(origin));
                }
                if let Some(path) = state_path {
                    self.set_review_state_path(path);
                }
                if let Some((states, blob_shas, annotations)) = reconciled {
                    self.set_review_states(states, blob_shas);
                    self.restore_review_annotations(annotations);
                }
                self.close_review_launcher_after_start();
                self.after_panel_coherence();
                self.set_status_message(format!("reviewing {branch}"));
            }
            Err(e) => {
                self.set_status_message(format!("review failed: {e}"));
                self.close_review_launcher();
            }
        }
    }

    /// Closes the launcher after a successful branch-review start: a
    /// `Panel` origin's cursor is re-clamped against the post-reroot row
    /// count (the reviewed branch's file list can be a different shape than
    /// what was on screen when `R` was pressed) — the same reclamp
    /// `App::close_switcher`/the retired review-branch modal's close
    /// applied; `Normal`/`Visual` origins restore verbatim, since a reroot
    /// doesn't change their shape.
    fn close_review_launcher_after_start(&mut self) {
        self.mode = match self.mode {
            Mode::ReviewLauncher {
                origin: ModeOrigin::Panel { cursor, tab },
                ..
            } => {
                let len = self.panel_row_count();
                Mode::Panel {
                    cursor: cursor.min(len.saturating_sub(1)),
                    tab,
                }
            }
            Mode::ReviewLauncher { origin, .. } => origin.restore(),
            other => other,
        };
    }
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

    // -- Cursor movement (no-op without a backend: the branch list is empty) -

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
    fn confirm_on_an_empty_branch_list_is_a_no_op() {
        let mut app = app();
        app.open_review_launcher();
        app.review_launcher_confirm();
        assert!(matches!(app.mode, Mode::ReviewLauncher { .. }));
        assert!(app.status_message.is_none());
    }

    #[test]
    fn confirm_without_a_git_backend_degrades_to_a_message_and_leaves_the_modal_open() {
        // A populated list with no backend attached is an edge case the
        // production code guards defensively (branch data normally implies
        // a backend); this exercises that guard directly rather than
        // relying on it being unreachable.
        let mut app = app();
        app.mode = Mode::ReviewLauncher {
            tab: LauncherTab::Branches,
            cursor: 0,
            origin: ModeOrigin::Normal,
        };
        app.launcher_branches = vec![crate::git::LocalBranch {
            name: "feature".to_string(),
            is_current: false,
            worktree: None,
        }];

        app.review_launcher_confirm();

        assert!(matches!(app.mode, Mode::ReviewLauncher { .. }));
        assert!(
            app.status_message
                .as_deref()
                .is_some_and(|m| m.contains("no git backend")),
            "got {:?}",
            app.status_message
        );
    }

    // -- Branches tab: data population (FR-8) --------------------------------

    /// A minimal `StageOps` fake exposing a fixed branch list — enough to
    /// drive `load_launcher_branches`/`confirm_launcher_branch_review`
    /// without a real repository. Every worktree-mutating method panics, so
    /// a test that reaches this fake and never panics has proven those
    /// calls never happened — the operations-seam proof the in-session
    /// guard test (FR-10) below relies on.
    struct BranchListOps {
        branches: Vec<crate::git::LocalBranch>,
    }

    impl super::super::stage_ops::StageOps for BranchListOps {
        fn diff(
            &self,
            _target: &crate::git::DiffTarget,
        ) -> Result<Vec<crate::git::RawFilePatch>, crate::git::GitError> {
            Ok(Vec::new())
        }
        fn status(&self) -> Result<Vec<crate::git::FileStatus>, crate::git::GitError> {
            Ok(Vec::new())
        }
        fn stage_file(&self, _path: &str) -> Result<(), crate::git::GitError> {
            panic!("stage_file must never be called from the launcher's Branches tab")
        }
        fn unstage_file(&self, _path: &str) -> Result<(), crate::git::GitError> {
            panic!("unstage_file must never be called from the launcher's Branches tab")
        }
        fn apply_cached(&self, _patch: &str) -> Result<(), crate::git::GitError> {
            panic!("apply_cached must never be called from the launcher's Branches tab")
        }
        fn unapply_cached(&self, _patch: &str) -> Result<(), crate::git::GitError> {
            panic!("unapply_cached must never be called from the launcher's Branches tab")
        }
        fn read_worktree_file(&self, _path: &str) -> Option<Vec<u8>> {
            None
        }
        fn show_file(&self, _spec: &str) -> Option<String> {
            None
        }
        fn branch_list(&self) -> Result<Vec<crate::git::LocalBranch>, crate::git::GitError> {
            Ok(self.branches.clone())
        }
        fn worktree_add(
            &self,
            _path: &std::path::Path,
            _branch: &str,
        ) -> Result<(), crate::git::GitError> {
            panic!("worktree_add must never be called while the in-session guard should block it")
        }
        fn git_common_dir(&self) -> Result<std::path::PathBuf, crate::git::GitError> {
            panic!("git_common_dir must never be called while the in-session guard should block it")
        }
        fn default_base(&self) -> Result<String, crate::git::GitError> {
            panic!("default_base must never be called while the in-session guard should block it")
        }
    }

    fn branch(name: &str) -> crate::git::LocalBranch {
        crate::git::LocalBranch {
            name: name.to_string(),
            is_current: false,
            worktree: None,
        }
    }

    fn app_with_branches(branches: Vec<crate::git::LocalBranch>) -> App {
        let mut app = app();
        app.stage_ops = Some(Box::new(BranchListOps { branches }));
        app
    }

    #[test]
    fn opening_the_launcher_populates_the_branches_tab() {
        let mut app = app_with_branches(vec![branch("alpha"), branch("zulu")]);
        app.open_review_launcher();
        assert_eq!(
            app.launcher_branches
                .iter()
                .map(|b| b.name.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "zulu"]
        );
    }

    #[test]
    fn switching_onto_the_branches_tab_reloads_the_list() {
        let mut app = app_with_branches(vec![branch("alpha")]);
        app.mode = Mode::ReviewLauncher {
            tab: LauncherTab::Commits,
            cursor: 0,
            origin: ModeOrigin::Normal,
        };
        app.last_launcher_tab = LauncherTab::Commits;
        assert!(app.launcher_branches.is_empty(), "not loaded yet");
        app.review_launcher_switch_tab();
        assert_eq!(
            app.launcher_branches
                .iter()
                .map(|b| b.name.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha"]
        );
    }

    #[test]
    fn row_count_reflects_the_branch_list_on_the_branches_tab() {
        let mut app = app_with_branches(vec![branch("alpha"), branch("zulu")]);
        app.open_review_launcher();
        app.review_launcher_move_down();
        assert_eq!(
            app.mode,
            Mode::ReviewLauncher {
                tab: LauncherTab::Branches,
                cursor: 1,
                origin: ModeOrigin::Normal,
            },
            "cursor must be able to reach the second real branch row"
        );
        app.review_launcher_move_down(); // clamps at the last
        assert_eq!(
            app.mode,
            Mode::ReviewLauncher {
                tab: LauncherTab::Branches,
                cursor: 1,
                origin: ModeOrigin::Normal,
            }
        );
    }

    // -- Branches tab: in-session guard (FR-10) ------------------------------

    #[test]
    fn confirm_during_an_active_review_session_emits_the_hint_and_mutates_nothing() {
        let mut app = app_with_branches(vec![branch("alpha")]);
        app.open_review_launcher();
        app.target = crate::git::DiffTarget::Review {
            base: "main".to_string(),
            branch: "feature".to_string(),
        };

        app.review_launcher_confirm();

        assert!(
            matches!(
                app.mode,
                Mode::ReviewLauncher {
                    tab: LauncherTab::Branches,
                    ..
                }
            ),
            "the launcher stays open and on the Branches tab, got {:?}",
            app.mode
        );
        assert!(
            app.status_message
                .as_deref()
                .is_some_and(|m| m.contains("feature") && m.contains('q')),
            "the hint must name the branch under review and point at q, got {:?}",
            app.status_message
        );
        // `BranchListOps::worktree_add`/`git_common_dir`/`default_base` all
        // panic — reaching this assertion without a panic proves none of
        // them ran.
    }
}
