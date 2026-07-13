//! State for the branch/worktree switcher modal ([`super::app::Mode::Switcher`],
//! spec 03 Unit 1): which tab is active, the branch/worktree lists read once
//! when the modal opened, a per-tab cursor, and the git panel's cursor row to
//! restore when the modal closes. Also carries the `App` handlers that open
//! and close the modal and drive its cursor, split out of `app.rs` alongside
//! this state so all switcher logic lives in one module (mirrors
//! [`super::git_panel`]'s panel-state-plus-handlers split).

use std::path::Path;

use crate::git::{GitError, GitRunner, LocalBranch, WorktreeEntry};

use super::app::{App, Mode};
use super::command_log::CommandLogEntry;
use super::stage_ops::build_review;

/// Which tab of the switcher modal is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitcherTab {
    /// Local branches (the default tab).
    Branches,
    /// Worktrees (`git worktree list --porcelain`).
    Worktrees,
}

/// The switcher modal's state: the branch/worktree lists read once when the
/// modal opened, which tab is active, a cursor per tab (so switching tabs
/// doesn't lose your place), and the git panel's cursor row to restore on
/// close.
#[derive(Debug, Clone)]
pub struct SwitcherState {
    /// The active tab.
    pub tab: SwitcherTab,
    /// Local branches, as read when the modal opened.
    pub branches: Vec<LocalBranch>,
    /// Worktrees, as read when the modal opened.
    pub worktrees: Vec<WorktreeEntry>,
    /// The Branches tab's cursor, independent of the Worktrees tab's.
    pub branch_cursor: usize,
    /// The Worktrees tab's cursor, independent of the Branches tab's.
    pub worktree_cursor: usize,
    /// The git panel's cursor row captured when the modal opened, restored
    /// by [`App::close_switcher`] so `Esc` lands the user back on the same
    /// panel row.
    pub panel_cursor: usize,
}

impl SwitcherState {
    /// Builds switcher state from freshly read branch/worktree lists,
    /// starting each tab's cursor on the current branch/worktree (falling
    /// back to the top row if none is marked current, or the list is
    /// empty). `panel_cursor` is the git panel's cursor to restore on close
    /// (see [`App::open_switcher`]).
    pub fn new(
        branches: Vec<LocalBranch>,
        worktrees: Vec<WorktreeEntry>,
        repo_root: Option<&Path>,
        panel_cursor: usize,
    ) -> SwitcherState {
        let branch_cursor = branches.iter().position(|b| b.is_current).unwrap_or(0);
        let worktree_cursor = worktrees
            .iter()
            .position(|w| is_current_worktree(repo_root, w))
            .unwrap_or(0);
        SwitcherState {
            tab: SwitcherTab::Branches,
            branches,
            worktrees,
            branch_cursor,
            worktree_cursor,
            panel_cursor,
        }
    }

    /// Switches between the Branches and Worktrees tabs (there are only
    /// two, so this always toggles rather than needing a direction).
    pub fn toggle_tab(&mut self) {
        self.tab = match self.tab {
            SwitcherTab::Branches => SwitcherTab::Worktrees,
            SwitcherTab::Worktrees => SwitcherTab::Branches,
        };
    }

    /// The active tab's row count.
    fn active_len(&self) -> usize {
        match self.tab {
            SwitcherTab::Branches => self.branches.len(),
            SwitcherTab::Worktrees => self.worktrees.len(),
        }
    }

    /// The active tab's cursor field.
    fn active_cursor_mut(&mut self) -> &mut usize {
        match self.tab {
            SwitcherTab::Branches => &mut self.branch_cursor,
            SwitcherTab::Worktrees => &mut self.worktree_cursor,
        }
    }

    /// Moves the active tab's cursor down one row, clamped at the last (or
    /// pinned at 0 on an empty list).
    pub fn move_down(&mut self) {
        let len = self.active_len();
        let cursor = self.active_cursor_mut();
        *cursor = if len == 0 {
            0
        } else {
            (*cursor + 1).min(len - 1)
        };
    }

    /// Moves the active tab's cursor up one row, clamped at the first.
    pub fn move_up(&mut self) {
        let cursor = self.active_cursor_mut();
        *cursor = cursor.saturating_sub(1);
    }
}

/// Whether `wt` is the worktree redquill is currently rooted at: its path
/// canonicalizes to the same location as `repo_root`. Falls back to a raw
/// path comparison if either side fails to canonicalize (a synthetic path
/// in unit tests, or a worktree directory that has since vanished), so the
/// comparison degrades to "these two `PathBuf`s are literally equal" rather
/// than panicking or always reporting "not current".
pub fn is_current_worktree(repo_root: Option<&Path>, wt: &WorktreeEntry) -> bool {
    let Some(root) = repo_root else {
        return false;
    };
    let root_canon = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let wt_canon = wt.path.canonicalize().unwrap_or_else(|_| wt.path.clone());
    root_canon == wt_canon
}

impl App {
    /// Opens the branch/worktree switcher modal (`b`, panel scope): reads
    /// the local-branch and worktree lists through the attached
    /// [`super::stage_ops::StageOps`] backend and switches to
    /// [`Mode::Switcher`], capturing the panel's current cursor row so
    /// [`App::close_switcher`] can restore it. A read error — including no
    /// git backend attached — degrades to a footer message; `self.mode` and
    /// `self.switcher` are only touched together, on the success path, so a
    /// failure never leaves a half-open modal.
    pub(super) fn open_switcher(&mut self) {
        let Some(ops) = self.stage_ops.as_deref() else {
            self.set_status_message("switcher unavailable (no git backend)");
            return;
        };
        match (ops.branch_list(), ops.worktree_list()) {
            (Ok(branches), Ok(worktrees)) => {
                let panel_cursor = self.panel_cursor();
                self.switcher = Some(SwitcherState::new(
                    branches,
                    worktrees,
                    self.repo_root.as_deref(),
                    panel_cursor,
                ));
                self.mode = Mode::Switcher;
            }
            (Err(e), _) | (_, Err(e)) => self.set_status_message(format!("switcher: {e}")),
        }
    }

    /// Closes the switcher modal, returning to [`Mode::Panel`] at the
    /// cursor row it had before the modal opened — re-clamped against the
    /// panel's current row count in case it shrank while the modal was
    /// open (e.g. a background refresh completing).
    pub fn close_switcher(&mut self) {
        let cursor = self.switcher.take().map(|s| s.panel_cursor).unwrap_or(0);
        let len = super::git_panel::navigable_rows(self).len();
        self.mode = Mode::Panel {
            cursor: cursor.min(len.saturating_sub(1)),
        };
    }

    /// Switches tabs (`Tab`/`BackTab`/`h`/`l`/arrows); a no-op if the modal
    /// isn't open.
    pub(super) fn switcher_toggle_tab(&mut self) {
        if let Some(s) = self.switcher.as_mut() {
            s.toggle_tab();
        }
    }

    /// Moves the active tab's cursor down one row; a no-op if the modal
    /// isn't open.
    pub(super) fn switcher_move_down(&mut self) {
        if let Some(s) = self.switcher.as_mut() {
            s.move_down();
        }
    }

    /// Moves the active tab's cursor up one row; a no-op if the modal isn't
    /// open.
    pub(super) fn switcher_move_up(&mut self) {
        if let Some(s) = self.switcher.as_mut() {
            s.move_up();
        }
    }

    /// The `Enter` gesture inside the switcher modal (spec 03 Units 2/3):
    /// dispatches on the active tab to [`App::confirm_branch_switch`] or
    /// [`App::confirm_worktree_switch`]. Guarded up front by the same
    /// single-in-flight rule [`App::request_remote_op`] enforces: a running
    /// fetch/pull/push blocks a switch attempt (both mutate the working tree
    /// state the remote op is mid-flight against) — rejected with a footer
    /// message, modal left open, exactly like a second remote-op request.
    pub(super) fn switcher_confirm(&mut self) {
        if let Some(label) = self.running_op_label() {
            self.set_status_message(format!("{label} is running \u{2014} wait before switching"));
            return;
        }
        let Some(s) = self.switcher.as_ref() else {
            return;
        };
        match s.tab {
            SwitcherTab::Branches => self.confirm_branch_switch(),
            SwitcherTab::Worktrees => self.confirm_worktree_switch(),
        }
    }

    /// Branches-tab `Enter` (spec 03 Unit 2): switches to the selected
    /// branch via `git switch -- <name>`, records the attempt in the
    /// command log either way, and reports the outcome in the footer.
    ///
    /// The current branch is a no-op with a footer message, modal left open
    /// (nothing to switch to). On success the modal closes, a full refresh
    /// rebuilds the review (diff/panel/branch/annotation targets — annotations
    /// themselves are untouched, so they keep pointing at the same
    /// paths/lines) and the git-panel cursor re-follows the diff. On failure
    /// (dirty tree, branch checked out in another worktree, ...) the modal
    /// stays open per spec so the reviewer can see the failure and retry or
    /// pick something else; the footer points at the command log (`@`) for
    /// git's stderr.
    ///
    /// Race-safe for free: [`App::refresh`] bumps `refresh_generation`, so
    /// any working-tree poll spawned before this switch is discarded on
    /// drain rather than clobbering the post-switch state (see
    /// [`super::refresh`]).
    fn confirm_branch_switch(&mut self) {
        let Some(s) = self.switcher.as_ref() else {
            return;
        };
        let Some(branch) = s.branches.get(s.branch_cursor) else {
            return;
        };
        let name = branch.name.clone();
        if branch.is_current {
            self.set_status_message(format!("already on {name}"));
            return;
        }
        let Some(ops) = self.stage_ops.as_deref() else {
            self.set_status_message("switcher unavailable (no git backend)");
            return;
        };
        let result = ops.switch_branch(&name);
        self.command_log
            .push(branch_switch_log_entry(&name, &result));
        match result {
            Ok(()) => {
                self.close_switcher();
                self.refresh();
                self.after_panel_coherence();
                self.set_status_message(format!("switched to {name} (annotations kept)"));
            }
            Err(_) => {
                self.set_status_message("switch failed \u{2014} see command log (@)");
            }
        }
    }

    /// Worktrees-tab `Enter` (spec 03 Unit 3): re-roots the whole review
    /// session onto the selected worktree.
    ///
    /// Guards a bare worktree (nothing to review there) and the already-current
    /// worktree with a footer message, modal left open — mirrors the
    /// current-branch guard on the Branches tab. Otherwise discovers a fresh
    /// [`GitRunner`] at the worktree's path and hands off to
    /// [`App::reroot`], which does the actual build-before-swap.
    fn confirm_worktree_switch(&mut self) {
        let Some(s) = self.switcher.as_ref() else {
            return;
        };
        let Some(wt) = s.worktrees.get(s.worktree_cursor) else {
            return;
        };
        if wt.bare {
            self.set_status_message("cannot re-root onto a bare worktree");
            return;
        }
        if is_current_worktree(self.repo_root.as_deref(), wt) {
            self.set_status_message("already in this worktree");
            return;
        }
        let path = wt.path.clone();
        match GitRunner::discover_in(&path) {
            Ok(runner) => self.reroot(runner),
            Err(e) => {
                self.close_switcher();
                self.set_status_message(format!("re-root failed: {e}"));
            }
        }
    }

    /// Re-roots the app onto `runner`'s repository: builds the new review
    /// snapshot *before* touching any state (spec 03 Unit 3's build-first
    /// requirement), so a failed rebuild leaves the current worktree's
    /// session fully intact — only on success does the backend, repo root,
    /// and LSP state actually swap.
    ///
    /// Once the swap commits, bumps `refresh_generation` and clears
    /// `refresh_in_flight`: any working-tree poll still in flight against the
    /// *old* root was captured (by [`super::stage_ops::StageOps::async_review_builder`])
    /// as a clone of the old [`GitRunner`], so it keeps running to completion
    /// against the old repo — but its result is now orphaned twice over:
    /// [`super::App::poll_refresh`] drops anything whose spawn-time
    /// generation no longer matches (the bump), and even if that raced back
    /// to matching, `refresh_in_flight` being `None` means there's no
    /// tracked task for its id to match at all. Clearing it also frees
    /// `spawn_auto_refresh`'s single-flight gate immediately, rather than
    /// waiting for the orphaned old-root read to naturally drain, so the very
    /// next poll tick can spawn a fresh read against the *new* backend.
    ///
    /// Annotations are untouched by this swap (spec 03 Unit 4): they're
    /// keyed by path/line, not by backend identity, so they keep applying to
    /// the newly-rooted review as-is.
    fn reroot(&mut self, runner: GitRunner) {
        let new_root = runner.root().to_path_buf();
        let snapshot = match build_review(&runner, &self.target) {
            Ok(snapshot) => snapshot,
            Err(e) => {
                self.close_switcher();
                self.set_status_message(format!("re-root failed: {e}"));
                return;
            }
        };

        self.refresh_generation = self.refresh_generation.wrapping_add(1);
        self.refresh_in_flight = None;

        if let Some(client) = self.take_lsp_client() {
            std::thread::spawn(move || client.shutdown());
        }
        // A peek request in flight against the old root must not resolve
        // into the newly-rooted session once it lands.
        self.pending_lsp = None;

        self.repo_root = Some(new_root);
        self.stage_ops = Some(Box::new(runner));
        self.apply_snapshot(snapshot);
        self.close_switcher();
        self.after_panel_coherence();
        self.set_status_message("re-rooted (annotations kept)");
    }

    /// After a branch switch or worktree re-root closes the modal back to
    /// [`Mode::Panel`], re-follows the diff to the panel cursor's row — the
    /// rebuilt review may have reordered or replaced files out from under
    /// it, so this keeps the diff pointed at whatever the panel cursor now
    /// rests on rather than a stale selection. A no-op when the panel isn't
    /// focused (e.g. the switcher was opened some other way in the future).
    fn after_panel_coherence(&mut self) {
        if matches!(self.mode, Mode::Panel { .. }) {
            self.panel_follow();
        }
    }
}

/// Builds the [`CommandLogEntry`] for a `git switch -- <name>` attempt from
/// its result: success records a clean `exit 0`; a [`GitError::Command`]
/// copies git's own exit code and stderr; any other [`GitError`] variant
/// (git missing, spawn failure, ...) never reached a process, so it has no
/// code/stderr to copy — those fields are recorded empty and the error's
/// `Display` becomes the stderr line instead, so the command log still shows
/// *something* actionable.
fn branch_switch_log_entry(name: &str, result: &Result<(), GitError>) -> CommandLogEntry {
    let command_line = format!("git switch -- {name}");
    match result {
        Ok(()) => CommandLogEntry {
            command_line,
            success: true,
            code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
        },
        Err(GitError::Command { code, stderr, .. }) => CommandLogEntry {
            command_line,
            success: false,
            code: code.parse().ok(),
            stdout: String::new(),
            stderr: stderr.clone(),
        },
        Err(e) => CommandLogEntry {
            command_line,
            success: false,
            code: None,
            stdout: String::new(),
            stderr: e.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::FileDiff;
    use crate::git::RawFilePatch;
    use std::path::PathBuf;

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

    fn branch(name: &str, is_current: bool, worktree: Option<&str>) -> LocalBranch {
        LocalBranch {
            name: name.to_string(),
            is_current,
            worktree: worktree.map(PathBuf::from),
        }
    }

    fn worktree(path: &str) -> WorktreeEntry {
        WorktreeEntry {
            path: PathBuf::from(path),
            head: Some("deadbeef".to_string()),
            branch: Some("main".to_string()),
            bare: false,
            detached: false,
            locked: None,
            prunable: None,
        }
    }

    // -- SwitcherState::new: initial cursor placement -----------------------

    #[test]
    fn cursor_starts_on_current_branch() {
        let branches = vec![
            branch("feature", false, None),
            branch("main", true, None),
            branch("other", false, None),
        ];
        let state = SwitcherState::new(branches, vec![], None, 0);
        assert_eq!(state.branch_cursor, 1);
    }

    #[test]
    fn cursor_falls_back_to_top_when_no_branch_is_current() {
        let branches = vec![branch("feature", false, None), branch("other", false, None)];
        let state = SwitcherState::new(branches, vec![], None, 0);
        assert_eq!(state.branch_cursor, 0);
    }

    #[test]
    fn worktree_cursor_starts_on_current_worktree_by_path() {
        let worktrees = vec![worktree("/repo/a"), worktree("/repo/b")];
        let state = SwitcherState::new(vec![], worktrees, Some(Path::new("/repo/b")), 0);
        assert_eq!(state.worktree_cursor, 1);
    }

    #[test]
    fn worktree_cursor_falls_back_to_top_without_a_repo_root() {
        let worktrees = vec![worktree("/repo/a"), worktree("/repo/b")];
        let state = SwitcherState::new(vec![], worktrees, None, 0);
        assert_eq!(state.worktree_cursor, 0);
    }

    #[test]
    fn panel_cursor_is_captured_verbatim() {
        let state = SwitcherState::new(vec![], vec![], None, 7);
        assert_eq!(state.panel_cursor, 7);
    }

    // -- toggle_tab / move_down / move_up: per-tab clamping -----------------

    #[test]
    fn toggle_tab_switches_between_branches_and_worktrees() {
        let mut state = SwitcherState::new(vec![], vec![], None, 0);
        assert_eq!(state.tab, SwitcherTab::Branches);
        state.toggle_tab();
        assert_eq!(state.tab, SwitcherTab::Worktrees);
        state.toggle_tab();
        assert_eq!(state.tab, SwitcherTab::Branches);
    }

    #[test]
    fn move_down_and_up_clamp_within_the_active_tab() {
        let branches = vec![branch("a", false, None), branch("b", false, None)];
        let mut state = SwitcherState::new(branches, vec![], None, 0);
        state.branch_cursor = 0;
        state.move_down();
        assert_eq!(state.branch_cursor, 1);
        state.move_down(); // clamps at the last
        assert_eq!(state.branch_cursor, 1);
        state.move_up();
        assert_eq!(state.branch_cursor, 0);
        state.move_up(); // clamps at the first
        assert_eq!(state.branch_cursor, 0);
    }

    #[test]
    fn move_on_empty_active_tab_stays_at_zero() {
        let mut state = SwitcherState::new(vec![], vec![], None, 0);
        state.move_down();
        assert_eq!(state.branch_cursor, 0);
        state.move_up();
        assert_eq!(state.branch_cursor, 0);
    }

    #[test]
    fn each_tab_keeps_its_own_cursor_across_a_toggle() {
        let branches = vec![branch("a", false, None), branch("b", false, None)];
        let worktrees = vec![worktree("/repo/a"), worktree("/repo/b")];
        let mut state = SwitcherState::new(branches, worktrees, None, 0);
        state.move_down(); // branch_cursor -> 1
        state.toggle_tab();
        assert_eq!(state.worktree_cursor, 0);
        state.move_down(); // worktree_cursor -> 1
        state.toggle_tab();
        assert_eq!(
            state.branch_cursor, 1,
            "branch tab's cursor survived the round trip"
        );
    }

    // -- is_current_worktree --------------------------------------------------

    #[test]
    fn is_current_worktree_true_for_matching_raw_paths() {
        let wt = worktree("/repo/a");
        assert!(is_current_worktree(Some(Path::new("/repo/a")), &wt));
    }

    #[test]
    fn is_current_worktree_false_for_different_paths() {
        let wt = worktree("/repo/a");
        assert!(!is_current_worktree(Some(Path::new("/repo/b")), &wt));
    }

    #[test]
    fn is_current_worktree_false_without_a_repo_root() {
        let wt = worktree("/repo/a");
        assert!(!is_current_worktree(None, &wt));
    }

    // -- App::open_switcher / close_switcher ---------------------------------

    #[test]
    fn open_switcher_without_backend_sets_footer_message() {
        let mut app = App::new(vec![sample_file()]);
        app.mode = Mode::Panel { cursor: 0 };
        app.open_switcher();
        assert!(app.switcher.is_none());
        assert_eq!(app.mode, Mode::Panel { cursor: 0 });
        assert!(app.status_message.is_some());
    }

    #[test]
    fn close_switcher_without_ever_opening_returns_to_panel_at_zero() {
        let mut app = App::new(vec![sample_file()]);
        app.mode = Mode::Switcher;
        app.close_switcher();
        assert_eq!(app.mode, Mode::Panel { cursor: 0 });
    }
}
