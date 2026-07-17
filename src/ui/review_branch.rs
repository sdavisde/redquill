//! State for the review-branch modal ([`super::app::Mode::ReviewBranch`],
//! spec 08 Unit 1's in-app entry path / Unit 5 task 5.1-5.3): lists local
//! branches (excluding the one currently checked out) so the user can start
//! a review session in place without leaving the app, using the same
//! cursor/Enter/Esc gestures and visual styling as
//! [`super::switcher::SwitcherState`]'s Branches tab. Confirming here is its
//! own mode/state rather than a third switcher tab, since it does something
//! structurally different on confirm — resolves a base ref and ensures a
//! managed worktree exists (spec 08 Unit 1) rather than switching onto an
//! already-checked-out ref — and shares that "ensure a review session" core
//! with the CLI's `--review` flag via [`super::review_session`] (task 5.2:
//! one code path, two entry points), landing through the same generalized
//! [`super::App::reroot`] build-before-swap the worktree switcher already
//! established (spec 03 Unit 3).

use crate::git::{DiffTarget, GitRunner, LocalBranch};

use super::app::{App, Mode};
use super::review_session::{
    ensure_review_worktree, load_reconciled_review_state, resolve_review_base,
    resolve_review_state_path,
};

/// The review-branch modal's state: local branches read once when the modal
/// opened (excluding the one currently checked out — nothing to review
/// there), a cursor, and the git panel's cursor row to restore on `Esc`
/// (mirrors [`super::switcher::SwitcherState::panel_cursor`]).
#[derive(Debug, Clone)]
pub struct ReviewBranchState {
    /// Local branches, as read when the modal opened, excluding the one
    /// currently checked out.
    pub branches: Vec<LocalBranch>,
    /// The cursor into `branches`.
    pub cursor: usize,
    /// The git panel's cursor row captured when the modal opened, restored
    /// by [`App::close_review_branch_modal`].
    pub panel_cursor: usize,
}

impl ReviewBranchState {
    /// Builds review-branch modal state from a freshly read branch list
    /// (already filtered to exclude the current branch by the caller),
    /// starting the cursor at the top. `panel_cursor` is the git panel's
    /// cursor to restore on close.
    pub fn new(branches: Vec<LocalBranch>, panel_cursor: usize) -> ReviewBranchState {
        ReviewBranchState {
            branches,
            cursor: 0,
            panel_cursor,
        }
    }

    /// Moves the cursor down one row, clamped at the last (or pinned at 0 on
    /// an empty list).
    pub(super) fn move_down(&mut self) {
        let len = self.branches.len();
        self.cursor = if len == 0 {
            0
        } else {
            (self.cursor + 1).min(len - 1)
        };
    }

    /// Moves the cursor up one row, clamped at the first.
    pub(super) fn move_up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }
}

impl App {
    /// Opens the review-branch modal (new panel-scope binding, spec 08 Unit
    /// 5 task 5.1): reads local branches through `stage_ops`, excluding the
    /// branch currently checked out in the user's own worktree (nothing to
    /// review there), and switches to [`Mode::ReviewBranch`]. Degrades to a
    /// footer message — leaving `self.mode`/`self.review_branch_modal` untouched —
    /// without a git backend, on a read error, or while already mid-review
    /// (starting a nested review from inside one is out of scope for this
    /// spec: the modal's whole point is a normal session's entry point, and
    /// a second worktree-inside-a-worktree review would tangle the banner,
    /// annotation group-line, and `finish`'s origin-backend bookkeeping for
    /// no user-facing benefit — finish or pause the current review first).
    pub(super) fn open_review_branch_modal(&mut self) {
        if self.in_review_session() {
            self.set_status_message(format!(
                "already reviewing {} \u{2014} finish or pause first",
                self.review_branch().unwrap_or("this branch")
            ));
            return;
        }
        let Some(ops) = self.stage_ops.as_deref() else {
            self.set_status_message("review-branch unavailable (no git backend)");
            return;
        };
        match ops.branch_list() {
            Ok(branches) => {
                let branches: Vec<LocalBranch> =
                    branches.into_iter().filter(|b| !b.is_current).collect();
                let panel_cursor = self.panel_cursor();
                self.review_branch_modal = Some(ReviewBranchState::new(branches, panel_cursor));
                self.mode = Mode::ReviewBranch;
            }
            Err(e) => self.set_status_message(format!("review-branch: {e}")),
        }
    }

    /// Closes the review-branch modal, returning to [`Mode::Panel`] at the
    /// cursor row it had before the modal opened — re-clamped against the
    /// panel's current row count, mirroring
    /// [`super::switcher::App::close_switcher`].
    pub(super) fn close_review_branch_modal(&mut self) {
        let cursor = self
            .review_branch_modal
            .take()
            .map(|s| s.panel_cursor)
            .unwrap_or(0);
        let len = self.panel_row_count();
        self.mode = Mode::Panel {
            cursor: cursor.min(len.saturating_sub(1)),
            tab: self.last_panel_tab,
        };
    }

    /// Moves the modal's cursor down one row; a no-op if it isn't open.
    pub(super) fn review_branch_move_down(&mut self) {
        if let Some(s) = self.review_branch_modal.as_mut() {
            s.move_down();
        }
    }

    /// Moves the modal's cursor up one row; a no-op if it isn't open.
    pub(super) fn review_branch_move_up(&mut self) {
        if let Some(s) = self.review_branch_modal.as_mut() {
            s.move_up();
        }
    }

    /// The `Enter` gesture (spec 08 Unit 5 task 5.2): resolves the base ref,
    /// ensures the highlighted branch's managed worktree exists (creating or
    /// reusing it, spec 08 Unit 1), then re-roots the whole session onto it
    /// via [`App::reroot`] (build-before-swap, LSP re-create, annotation
    /// preservation) — the exact same "ensure a review session" core
    /// `main.rs::resolve_session` runs for `--review` (see
    /// [`super::review_session`]). On success, attaches the origin-rooted
    /// backend `finish_review` needs (discovered fresh at the *pre-reroot*
    /// repo root, mirroring `main.rs::run_tui`'s own `discovered` handle) and
    /// loads + reconciles this branch's persisted progress (spec 08 Unit 4),
    /// restoring annotations before the modal closes — parity with the CLI
    /// path's bootstrap order.
    ///
    /// Guarded up front by the same single-in-flight rule
    /// [`App::request_remote_op`]/[`App::switcher_confirm`] enforce: a
    /// running fetch/pull/push blocks starting a review the same way it
    /// blocks a branch/worktree switch.
    ///
    /// Every failure path (`git_common_dir`/`default_base` unavailable,
    /// `worktree_add` refusing — unknown branch, branch checked out
    /// elsewhere, path collision — or a failed rebuild) surfaces git's
    /// message in the modal via `self.status_message` and leaves the modal
    /// open with all prior state untouched (spec 08 Unit 5 task 5.3): never
    /// crashes, never partially mutates `self.target`/`self.stage_ops`/
    /// `self.repo_root`, since [`App::reroot`] itself only swaps those on a
    /// successful rebuild.
    pub(super) fn confirm_review_branch(&mut self) {
        if let Some(label) = self.running_op_label() {
            self.set_status_message(format!(
                "{label} is running \u{2014} wait before starting a review"
            ));
            return;
        }
        let Some(s) = self.review_branch_modal.as_ref() else {
            return;
        };
        let Some(branch) = s.branches.get(s.cursor).map(|b| b.name.clone()) else {
            return;
        };
        let Some(ops) = self.stage_ops.as_deref() else {
            self.set_status_message("review-branch unavailable (no git backend)");
            return;
        };

        let base = match resolve_review_base(ops, None) {
            Ok(base) => base,
            Err(e) => {
                self.set_status_message(format!("review failed: {e}"));
                return;
            }
        };
        let worktree_path = match ensure_review_worktree(ops, &branch) {
            Ok(path) => path,
            Err(e) => {
                self.set_status_message(format!("review failed: {e}"));
                return;
            }
        };
        let session_runner = match GitRunner::discover_in(&worktree_path) {
            Ok(runner) => runner,
            Err(e) => {
                self.set_status_message(format!("review failed: {e}"));
                return;
            }
        };

        // Resolved (and reconciled) *before* `reroot` swaps `self.stage_ops`
        // out from under `ops` — `session_runner` reads the branch's current
        // blob SHAs exactly as truthfully as the pre-reroot backend would
        // (Unit 4's reconciliation needs the branch's tip, not any
        // particular worktree; `git_common_dir` resolves to the same shared
        // path from either).
        let state_path = resolve_review_state_path(ops).ok();
        let reconciled = state_path
            .as_ref()
            .map(|path| load_reconciled_review_state(&session_runner, path, &branch));
        // The backend `finish_review` later runs `worktree_remove`/`prune`
        // through (spec 08 Unit 2): discovered fresh at the *current*
        // (pre-reroot) repo root, mirroring `main.rs::run_tui`'s
        // `discovered` handle — must be rooted outside the worktree being
        // removed, which the worktree about to become `self.repo_root` is
        // not.
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
                self.close_review_branch_modal();
                self.after_panel_coherence();
                self.set_status_message(format!("reviewing {branch}"));
            }
            Err(e) => {
                self.set_status_message(format!("review failed: {e}"));
            }
        }
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

    fn branch(name: &str, is_current: bool) -> LocalBranch {
        LocalBranch {
            name: name.to_string(),
            is_current,
            worktree: None,
        }
    }

    // -- ReviewBranchState: cursor movement ----------------------------------

    #[test]
    fn move_down_and_up_clamp() {
        let mut state = ReviewBranchState::new(vec![branch("a", false), branch("b", false)], 0);
        state.move_down();
        assert_eq!(state.cursor, 1);
        state.move_down(); // clamps at the last
        assert_eq!(state.cursor, 1);
        state.move_up();
        assert_eq!(state.cursor, 0);
        state.move_up(); // clamps at the first
        assert_eq!(state.cursor, 0);
    }

    #[test]
    fn move_on_empty_list_stays_at_zero() {
        let mut state = ReviewBranchState::new(Vec::new(), 0);
        state.move_down();
        assert_eq!(state.cursor, 0);
        state.move_up();
        assert_eq!(state.cursor, 0);
    }

    #[test]
    fn panel_cursor_is_captured_verbatim() {
        let state = ReviewBranchState::new(Vec::new(), 7);
        assert_eq!(state.panel_cursor, 7);
    }

    // -- App::open_review_branch_modal / close_review_branch_modal ----------

    #[test]
    fn open_without_backend_sets_footer_message() {
        let mut app = App::new(vec![sample_file()]);
        app.mode = Mode::Panel {
            cursor: 0,
            tab: PanelTab::Changes,
        };
        app.open_review_branch_modal();
        assert!(app.review_branch_modal.is_none());
        assert_eq!(
            app.mode,
            Mode::Panel {
                cursor: 0,
                tab: PanelTab::Changes
            }
        );
        assert!(app.status_message.is_some());
    }

    #[test]
    fn open_while_already_reviewing_degrades_to_a_message_and_leaves_mode_untouched() {
        let mut app = App::new(vec![sample_file()]);
        app.target = DiffTarget::Review {
            base: "main".to_string(),
            branch: "feature".to_string(),
        };
        app.mode = Mode::Panel {
            cursor: 0,
            tab: PanelTab::Changes,
        };
        app.open_review_branch_modal();
        assert!(app.review_branch_modal.is_none());
        assert_eq!(
            app.mode,
            Mode::Panel {
                cursor: 0,
                tab: PanelTab::Changes
            }
        );
        assert!(
            app.status_message
                .as_deref()
                .is_some_and(|m| m.contains("feature")),
            "message must name the branch already under review: {:?}",
            app.status_message
        );
    }

    #[test]
    fn close_without_ever_opening_returns_to_panel_at_zero() {
        let mut app = App::new(vec![sample_file()]);
        app.mode = Mode::ReviewBranch;
        app.close_review_branch_modal();
        assert_eq!(
            app.mode,
            Mode::Panel {
                cursor: 0,
                tab: PanelTab::Changes
            }
        );
    }

    #[test]
    fn confirm_without_a_git_backend_degrades_to_a_message_and_leaves_the_modal_open() {
        let mut app = App::new(vec![sample_file()]);
        app.mode = Mode::ReviewBranch;
        app.review_branch_modal = Some(ReviewBranchState::new(vec![branch("feature", false)], 0));

        app.confirm_review_branch();

        assert_eq!(app.mode, Mode::ReviewBranch, "modal must stay open");
        assert!(app.status_message.is_some());
        assert!(!app.in_review_session());
    }

    #[test]
    fn confirm_on_an_empty_branch_list_is_a_no_op() {
        let mut app = App::new(vec![sample_file()]);
        app.mode = Mode::ReviewBranch;
        app.review_branch_modal = Some(ReviewBranchState::new(Vec::new(), 0));

        app.confirm_review_branch();

        assert_eq!(app.mode, Mode::ReviewBranch);
        assert!(app.status_message.is_none());
    }
}
