//! State for the branch/worktree switcher modal ([`super::app::Mode::Switcher`]):
//! which tab is active, the branch/worktree lists read once
//! when the modal opened, a per-tab cursor, and the git panel's cursor row to
//! restore when the modal closes. Also carries the `App` handlers that open
//! and close the modal and drive its cursor, split out of `app.rs` alongside
//! this state so all switcher logic lives in one module (mirrors
//! [`super::git_panel`]'s panel-state-plus-handlers split).

use std::path::Path;

use crate::git::{DiffTarget, GitError, GitRunner, LocalBranch, WorktreeEntry};

use super::app::{App, Mode};
use super::command_log::CommandLogEntry;
use super::list_filter::ListFilter;
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
    /// The active tab's `/` filter session (`None`: no filter active).
    /// Cleared on every [`SwitcherState::toggle_tab`] — spec 12 §2.4's
    /// decision on the simplest sane tab/filter interaction: a query typed
    /// against branch names doesn't carry any meaning over to worktree
    /// names (or vice versa), so switching tabs starts the other tab fresh
    /// rather than silently reapplying a stale query. Transient per-open
    /// either way (spec 12 Non-Goal 5). `pub(super)` (not `pub`, unlike this
    /// struct's other fields) since [`ListFilter`] itself is
    /// `pub(in crate::ui)` — nothing outside this module tree could name
    /// the field's type anyway.
    pub(super) filter: Option<ListFilter>,
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
            filter: None,
        }
    }

    /// Switches between the Branches and Worktrees tabs (there are only
    /// two, so this always toggles rather than needing a direction),
    /// clearing any active filter (see [`Self::filter`]'s doc on why).
    pub fn toggle_tab(&mut self) {
        self.tab = match self.tab {
            SwitcherTab::Branches => SwitcherTab::Worktrees,
            SwitcherTab::Worktrees => SwitcherTab::Branches,
        };
        self.filter = None;
    }

    /// The active tab's raw (unfiltered) label list, for the `/` filter's
    /// fuzzy matcher: branch names, or a worktree's path plus branch (the
    /// same fields [`super::switcher_modal`]'s rows show).
    fn active_labels(&self) -> Vec<String> {
        match self.tab {
            SwitcherTab::Branches => self.branches.iter().map(|b| b.name.clone()).collect(),
            SwitcherTab::Worktrees => self
                .worktrees
                .iter()
                .map(|wt| {
                    format!(
                        "{} {}",
                        wt.path.display(),
                        wt.branch.as_deref().unwrap_or("")
                    )
                })
                .collect(),
        }
    }

    /// The active tab's effective row count: the active filter's filtered
    /// view when one is set, the full tab's row count otherwise — every
    /// motion clamps against this (spec 12's filtered-view design
    /// constraint).
    fn active_len(&self) -> usize {
        if let Some(f) = &self.filter {
            return f.len();
        }
        match self.tab {
            SwitcherTab::Branches => self.branches.len(),
            SwitcherTab::Worktrees => self.worktrees.len(),
        }
    }

    /// The active tab's cursor field — a filtered position while
    /// [`Self::filter`] is active, a raw index otherwise.
    fn active_cursor_mut(&mut self) -> &mut usize {
        match self.tab {
            SwitcherTab::Branches => &mut self.branch_cursor,
            SwitcherTab::Worktrees => &mut self.worktree_cursor,
        }
    }

    /// Translates the active tab's cursor into a real index into
    /// `branches`/`worktrees` — the one point every confirm gesture routes
    /// through.
    fn active_real_index(&self) -> Option<usize> {
        let cursor = match self.tab {
            SwitcherTab::Branches => self.branch_cursor,
            SwitcherTab::Worktrees => self.worktree_cursor,
        };
        match &self.filter {
            Some(f) => f.real_index(cursor),
            None => {
                let len = match self.tab {
                    SwitcherTab::Branches => self.branches.len(),
                    SwitcherTab::Worktrees => self.worktrees.len(),
                };
                (cursor < len).then_some(cursor)
            }
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

    /// Steps the active tab's cursor by `delta` rows in `down`'s direction,
    /// clamped at both ends (the shared core of half/full-page paging and
    /// `move_down`/`move_up`'s single-row step).
    fn step(&mut self, delta: usize, down: bool) {
        let len = self.active_len();
        let cursor = self.active_cursor_mut();
        *cursor = super::motion::step(*cursor, len, delta, down);
    }

    /// Moves the active tab's cursor down half a viewport (`Ctrl-d`; shared
    /// motion set, see `super::motion`). `viewport_height` is the caller's
    /// page-size proxy (the switcher has no render height of its own to
    /// track).
    pub fn half_page_down(&mut self, viewport_height: usize) {
        self.step(super::motion::half_page(viewport_height), true);
    }

    /// Moves the active tab's cursor up half a viewport (`Ctrl-u`).
    pub fn half_page_up(&mut self, viewport_height: usize) {
        self.step(super::motion::half_page(viewport_height), false);
    }

    /// Moves the active tab's cursor down a full viewport (`Ctrl-f`).
    pub fn full_page_down(&mut self, viewport_height: usize) {
        self.step(super::motion::full_page(viewport_height), true);
    }

    /// Moves the active tab's cursor up a full viewport (`Ctrl-b`).
    pub fn full_page_up(&mut self, viewport_height: usize) {
        self.step(super::motion::full_page(viewport_height), false);
    }

    /// Jumps the active tab's cursor to its first row (`g`/`Home`).
    pub fn jump_to_top(&mut self) {
        *self.active_cursor_mut() = super::motion::jump_top();
    }

    /// Jumps the active tab's cursor to its last row (`G`/`End`).
    pub fn jump_to_bottom(&mut self) {
        let len = self.active_len();
        *self.active_cursor_mut() = super::motion::jump_bottom(len);
    }
}

/// Re-clamps `s`'s active-tab cursor into its filter's freshly reranked
/// view — the switcher-state counterpart of the App-level filter methods'
/// clamp, needed here since [`SwitcherState::filter`] and its cursor fields
/// both live on the state struct rather than directly on `App`.
fn switcher_clamp_cursor_to_filter(s: &mut SwitcherState) {
    if let Some(f) = s.filter.as_ref() {
        let len = f.len();
        let cursor = s.active_cursor_mut();
        *cursor = (*cursor).min(len.saturating_sub(1));
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
                self.motion_count = None;
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
        let len = self.panel_row_count();
        self.mode = Mode::Panel {
            cursor: cursor.min(len.saturating_sub(1)),
            tab: self.last_panel_tab,
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

    /// The switcher's page-size proxy for half/full-page motions (see
    /// `git_panel::App::panel_viewport_proxy`'s identical rationale).
    fn switcher_viewport_proxy(&self) -> usize {
        self.view.viewport_height()
    }

    /// Moves the active tab's cursor down half a viewport (`Ctrl-d`, shared
    /// motion set); a no-op if the modal isn't open.
    pub(super) fn switcher_half_page_down(&mut self) {
        let height = self.switcher_viewport_proxy();
        if let Some(s) = self.switcher.as_mut() {
            s.half_page_down(height);
        }
    }

    /// Moves the active tab's cursor up half a viewport (`Ctrl-u`); a no-op
    /// if the modal isn't open.
    pub(super) fn switcher_half_page_up(&mut self) {
        let height = self.switcher_viewport_proxy();
        if let Some(s) = self.switcher.as_mut() {
            s.half_page_up(height);
        }
    }

    /// Moves the active tab's cursor down a full viewport (`Ctrl-f`); a
    /// no-op if the modal isn't open.
    pub(super) fn switcher_full_page_down(&mut self) {
        let height = self.switcher_viewport_proxy();
        if let Some(s) = self.switcher.as_mut() {
            s.full_page_down(height);
        }
    }

    /// Moves the active tab's cursor up a full viewport (`Ctrl-b`); a no-op
    /// if the modal isn't open.
    pub(super) fn switcher_full_page_up(&mut self) {
        let height = self.switcher_viewport_proxy();
        if let Some(s) = self.switcher.as_mut() {
            s.full_page_up(height);
        }
    }

    /// Jumps the active tab's cursor to its first row (`g`/`Home`); a no-op
    /// if the modal isn't open.
    pub(super) fn switcher_jump_to_top(&mut self) {
        if let Some(s) = self.switcher.as_mut() {
            s.jump_to_top();
        }
    }

    /// Jumps the active tab's cursor to its last row (`G`/`End`); a no-op
    /// if the modal isn't open.
    pub(super) fn switcher_jump_to_bottom(&mut self) {
        if let Some(s) = self.switcher.as_mut() {
            s.jump_to_bottom();
        }
    }

    /// Enters filter mode against the active tab (`/`); a no-op if the
    /// modal isn't open or a filter is already active.
    pub(super) fn switcher_enter_filter(&mut self) {
        let Some(s) = self.switcher.as_mut() else {
            return;
        };
        if s.filter.is_none() {
            let labels = s.active_labels();
            s.filter = Some(ListFilter::open(&labels));
        }
    }

    /// Resumes editing a locked filter (`/` while locked); a no-op if the
    /// modal isn't open or no filter is active.
    pub(super) fn switcher_resume_filter_editing(&mut self) {
        if let Some(f) = self.switcher.as_mut().and_then(|s| s.filter.as_mut()) {
            f.resume_editing();
        }
    }

    /// Locks the active filter (`Enter` while editing); a no-op if the
    /// modal isn't open or no filter is active.
    pub(super) fn switcher_lock_filter(&mut self) {
        if let Some(f) = self.switcher.as_mut().and_then(|s| s.filter.as_mut()) {
            f.lock();
        }
    }

    /// Clears the active tab's filter entirely (`Esc`); a no-op if the
    /// modal isn't open.
    pub(super) fn switcher_clear_filter(&mut self) {
        let Some(s) = self.switcher.as_mut() else {
            return;
        };
        s.filter = None;
        let len = s.active_len();
        let cursor = s.active_cursor_mut();
        *cursor = (*cursor).min(len.saturating_sub(1));
    }

    /// Appends `c` to the active filter's query and re-clamps the cursor; a
    /// no-op if the modal isn't open or no filter is active.
    pub(super) fn switcher_filter_push_char(&mut self, c: char) {
        let Some(s) = self.switcher.as_mut() else {
            return;
        };
        let labels = s.active_labels();
        if let Some(f) = s.filter.as_mut() {
            f.push_char(c, &labels);
        }
        switcher_clamp_cursor_to_filter(s);
    }

    /// Deletes the last character of the active filter's query; a no-op if
    /// the modal isn't open or no filter is active.
    pub(super) fn switcher_filter_backspace(&mut self) {
        let Some(s) = self.switcher.as_mut() else {
            return;
        };
        let labels = s.active_labels();
        if let Some(f) = s.filter.as_mut() {
            f.backspace(&labels);
        }
        switcher_clamp_cursor_to_filter(s);
    }

    /// The `Enter` gesture inside the switcher modal: dispatches on the
    /// active tab to [`App::confirm_branch_switch`] or
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

    /// Branches-tab `Enter`: switches to the selected
    /// branch via `git switch -- <name>`, records the attempt in the
    /// command log either way, and reports the outcome in the footer.
    ///
    /// The current branch is a no-op with a footer message, modal left open
    /// (nothing to switch to). On success the modal closes, a full refresh
    /// rebuilds the review (diff/panel/branch/annotation targets — annotations
    /// themselves are untouched, so they keep pointing at the same
    /// paths/lines) and the git-panel cursor re-follows the diff. On failure
    /// (dirty tree, branch checked out in another worktree, ...) the modal
    /// stays open so the reviewer can see the failure and retry or pick
    /// something else; the footer points at the command log (`@`) for
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
        let Some(index) = s.active_real_index() else {
            return;
        };
        let Some(branch) = s.branches.get(index) else {
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

    /// Worktrees-tab `Enter`: re-roots the whole review
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
        let Some(index) = s.active_real_index() else {
            return;
        };
        let Some(wt) = s.worktrees.get(index) else {
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
        let runner = match GitRunner::discover_in(&path) {
            Ok(runner) => runner,
            Err(e) => {
                self.close_switcher();
                self.set_status_message(format!("re-root failed: {e}"));
                return;
            }
        };
        let target = self.target.clone();
        match self.reroot(runner, target) {
            Ok(()) => {
                self.close_switcher();
                self.after_panel_coherence();
                self.set_status_message("re-rooted (annotations kept)");
            }
            Err(e) => {
                self.close_switcher();
                self.set_status_message(format!("re-root failed: {e}"));
            }
        }
    }

    /// Re-roots the app onto `runner`'s repository, reviewing `target`.
    /// Builds the new review snapshot before swapping any state, so a
    /// failed rebuild leaves the current session fully intact; only on
    /// success does the backend, repo root, target, and LSP state actually
    /// swap. Bumps `refresh_generation` and clears `refresh_in_flight` on
    /// success so any working-tree poll still in flight against the old
    /// root is dropped on arrival rather than applied, and the next poll
    /// tick can spawn a fresh read against the new backend. Shared by
    /// [`App::confirm_worktree_switch`] (re-roots onto the same target) and
    /// the review-branch modal's confirm gesture (re-roots onto a new
    /// [`DiffTarget::Review`] target); each caller owns its own
    /// post-outcome UI wiring. Annotations are untouched by the swap —
    /// they're keyed by path/line, not by backend identity.
    pub(super) fn reroot(&mut self, runner: GitRunner, target: DiffTarget) -> Result<(), String> {
        let new_root = runner.root().to_path_buf();
        let snapshot = build_review(&runner, &target).map_err(|e| e.to_string())?;

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
        self.target = target;
        // Any prior PR/MR session identity belongs to the old root; the PR
        // checkout flow re-stamps forge metadata after this returns, and a
        // plain branch/worktree re-root has none. Clearing here keeps a
        // stale forge block or stale label from leaking across the swap.
        self.review_forge = None;
        self.review_stale = false;
        self.apply_snapshot(snapshot);
        Ok(())
    }

    /// After a branch switch or worktree re-root closes the modal back to
    /// [`Mode::Panel`], re-follows the diff to the panel cursor's row — the
    /// rebuilt review may have reordered or replaced files out from under
    /// it, so this keeps the diff pointed at whatever the panel cursor now
    /// rests on rather than a stale selection. A no-op when the panel isn't
    /// focused (e.g. the switcher was opened some other way in the future).
    pub(super) fn after_panel_coherence(&mut self) {
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
        app.mode = Mode::Panel {
            cursor: 0,
            tab: crate::ui::app::PanelTab::Changes,
        };
        app.open_switcher();
        assert!(app.switcher.is_none());
        assert_eq!(
            app.mode,
            Mode::Panel {
                cursor: 0,
                tab: crate::ui::app::PanelTab::Changes
            }
        );
        assert!(app.status_message.is_some());
    }

    #[test]
    fn close_switcher_without_ever_opening_returns_to_panel_at_zero() {
        let mut app = App::new(vec![sample_file()]);
        app.mode = Mode::Switcher;
        app.close_switcher();
        assert_eq!(
            app.mode,
            Mode::Panel {
                cursor: 0,
                tab: crate::ui::app::PanelTab::Changes
            }
        );
    }

    // -- Filter + motion + verb composition (spec 12 FR-8) -------------------

    /// A no-op `StageOps` fake recording every `switch_branch` call (via a
    /// shared `Rc<RefCell<_>>`, mirroring `staging.rs`'s identical
    /// `RecordingOps` pattern), so a test can prove *which* branch a
    /// filtered `Enter` actually switched to.
    struct RecordingSwitchOps {
        switched: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
    }

    impl crate::ui::stage_ops::StageOps for RecordingSwitchOps {
        fn diff(&self, _target: &DiffTarget) -> Result<Vec<RawFilePatch>, GitError> {
            Ok(Vec::new())
        }
        fn status(&self) -> Result<Vec<crate::git::FileStatus>, GitError> {
            Ok(Vec::new())
        }
        fn stage_file(&self, _path: &str) -> Result<(), GitError> {
            Ok(())
        }
        fn unstage_file(&self, _path: &str) -> Result<(), GitError> {
            Ok(())
        }
        fn apply_cached(&self, _patch: &str) -> Result<(), GitError> {
            Ok(())
        }
        fn unapply_cached(&self, _patch: &str) -> Result<(), GitError> {
            Ok(())
        }
        fn read_worktree_file(&self, _path: &str) -> Option<Vec<u8>> {
            None
        }
        fn show_file(&self, _spec: &str) -> Option<String> {
            None
        }
        fn switch_branch(&self, name: &str) -> Result<(), GitError> {
            self.switched.borrow_mut().push(name.to_string());
            Ok(())
        }
    }

    /// Three branches, two of which ("feature-apple"/"feature-apricot")
    /// share a `feature` prefix a `/feature` query narrows to, leaving
    /// "main" (also the current branch) out — so the filtered view is
    /// genuinely narrower, with two real rows to move between.
    fn switcher_app_with_three_branches(log: std::rc::Rc<std::cell::RefCell<Vec<String>>>) -> App {
        let mut app = App::new(vec![sample_file()]);
        // None marked current, so `SwitcherState::new` falls back to
        // starting the cursor at row 0 (rather than following whichever
        // branch happens to be checked out, which could start the cursor
        // outside the filtered view below).
        let branches = vec![
            branch("feature-apple", false, None),
            branch("feature-apricot", false, None),
            branch("main", false, None),
        ];
        app.switcher = Some(SwitcherState::new(branches, vec![], None, 0));
        app.mode = Mode::Switcher;
        app.stage_ops = Some(Box::new(RecordingSwitchOps { switched: log }));
        app
    }

    #[test]
    fn filter_narrows_branches_motion_moves_within_it_and_confirm_switches_to_it() {
        use crate::ui::modes::handle_switcher_key;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut app = switcher_app_with_three_branches(log.clone());
        let key = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
        let enter = || KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);

        handle_switcher_key(&mut app, key('/'));
        for c in "feature".chars() {
            handle_switcher_key(&mut app, key(c));
        }
        assert_eq!(
            app.switcher
                .as_ref()
                .unwrap()
                .filter
                .as_ref()
                .unwrap()
                .len(),
            2,
            "main must be excluded"
        );
        handle_switcher_key(&mut app, enter()); // locks the filter
        assert!(
            !app.switcher
                .as_ref()
                .unwrap()
                .filter
                .as_ref()
                .unwrap()
                .is_editing()
        );

        let first = app.switcher.as_ref().unwrap().active_real_index().unwrap();
        handle_switcher_key(&mut app, key('j'));
        let second = app.switcher.as_ref().unwrap().active_real_index().unwrap();
        assert_ne!(first, second, "`j` must move within the filtered view");
        let target_name = app.switcher.as_ref().unwrap().branches[second].name.clone();

        handle_switcher_key(&mut app, enter()); // confirms the filtered selection
        assert_eq!(
            *log.borrow(),
            vec![target_name],
            "Enter must switch to the filtered (not raw-list) selection"
        );
    }

    #[test]
    fn toggling_tabs_clears_the_active_filter() {
        use crate::ui::modes::handle_switcher_key;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut app = switcher_app_with_three_branches(log);
        let key = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);

        handle_switcher_key(&mut app, key('/'));
        for c in "feature".chars() {
            handle_switcher_key(&mut app, key(c));
        }
        handle_switcher_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)); // locks
        assert!(app.switcher.as_ref().unwrap().filter.is_some());

        handle_switcher_key(&mut app, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.switcher.as_ref().unwrap().tab, SwitcherTab::Worktrees);
        assert!(
            app.switcher.as_ref().unwrap().filter.is_none(),
            "switching tabs must drop the other tab's stale filter"
        );
    }
}
