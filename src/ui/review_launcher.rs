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
//! [`App::confirm_launcher_branch_review`]. The Commits tab opens a
//! read-only single-commit view on `Enter` via
//! [`App::confirm_launcher_commit`]; both tabs share the shared motion
//! layer (spec 12 FR-12) clamped against [`App::review_launcher_row_count`]
//! and the shared `/` filter component (spec 12 FR-12,
//! [`super::list_filter::ListFilter`]) via [`App::launcher_filter`] — see
//! that field's doc for the shared-field-cleared-on-toggle decision. A
//! filtered `Enter` still routes through [`App::confirm_launcher_branch_review`]'s
//! in-session guard unchanged: [`App::review_launcher_confirm`] only
//! translates the cursor to a real index before dispatch, never bypasses
//! the guard that dispatch itself performs first.

use crate::git::{CommitLogEntry, CommitLogRange, DiffTarget, GitRunner};

use super::app::{App, Mode, ModeOrigin};
use super::background::TaskId;
use super::list_filter::ListFilter;
use super::review_session::{
    ensure_review_worktree, load_reconciled_review_state, resolve_review_base,
    resolve_review_state_path,
};

/// A background ahead-of-base commit-log fetch awaiting completion. Mirrors
/// [`super::history::InFlightHistory`]'s shape exactly — the Commits tab's
/// ahead-of-base source reuses the identical single-flight +
/// generation-guard discipline the History tab pioneered, just against a
/// single-shot (never paginated) fetch rather than a page sequence.
#[derive(Debug, Clone, Copy)]
pub(super) struct InFlightLauncherCommits {
    /// The background task delivering the ahead-of-base commit list.
    pub(super) id: TaskId,
    /// The generation captured when this fetch was spawned.
    pub(super) generation: u64,
}

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
        // A count left mid-accumulation in whatever mode `R` was pressed
        // from (e.g. the panel's own `motion_count`) must not leak into the
        // launcher's first keystroke — mirrors every other motion-consuming
        // mode's entry point (`open_switcher`, `toggle_list`, ...).
        self.motion_count = None;
        match tab {
            LauncherTab::Branches => self.load_launcher_branches(),
            LauncherTab::Commits => self.ensure_launcher_commits_source_loaded(),
        }
    }

    /// Closes the Review launcher without acting, restoring the mode it was
    /// opened from exactly (panel cursor/tab included, via `ModeOrigin`) and
    /// dropping any active filter (transient per-open, see
    /// [`App::launcher_filter`]'s doc). A no-op (falls back to
    /// `Mode::Normal`, never panicking) if called while the modal isn't
    /// open — defensive rather than relied upon.
    pub(super) fn close_review_launcher(&mut self) {
        self.mode = match self.mode {
            Mode::ReviewLauncher { origin, .. } => origin.restore(),
            other => other,
        };
        self.launcher_filter = None;
    }

    /// Switches the launcher between its two tabs, resetting the cursor to
    /// the top (each tab's list is independent) and remembering the new tab
    /// in `last_launcher_tab` so the next open this process lands back here.
    /// Also drops any active filter (see [`App::launcher_filter`]'s doc on
    /// why toggling tabs doesn't try to carry a query over). A no-op unless
    /// the launcher is open.
    pub(super) fn review_launcher_switch_tab(&mut self) {
        let Mode::ReviewLauncher { tab, cursor, .. } = &mut self.mode else {
            return;
        };
        *tab = tab.toggle();
        *cursor = 0;
        let new_tab = *tab;
        self.launcher_filter = None;
        self.last_launcher_tab = new_tab;
        match new_tab {
            LauncherTab::Branches => self.load_launcher_branches(),
            LauncherTab::Commits => self.ensure_launcher_commits_source_loaded(),
        }
    }

    /// Moves the launcher's cursor down one row, clamped at the last row of
    /// whichever list backs the active tab (or pinned at 0 on an empty
    /// list). A no-op unless the launcher is open.
    pub(super) fn review_launcher_move_down(&mut self) {
        self.review_launcher_step(1, true);
    }

    /// Moves the launcher's cursor up one row, clamped at the first. A no-op
    /// unless the launcher is open.
    pub(super) fn review_launcher_move_up(&mut self) {
        self.review_launcher_step(1, false);
    }

    /// The active tab's raw (unfiltered) row count: `launcher_branches`'
    /// length on Branches, the active Commits source's length on Commits —
    /// kept as its own method so that work only needs to change one arm,
    /// mirroring how [`super::git_panel::App::panel_row_count`] centralizes
    /// the git panel's per-tab length.
    fn review_launcher_raw_row_count(&self) -> usize {
        let Mode::ReviewLauncher { tab, .. } = self.mode else {
            return 0;
        };
        match tab {
            LauncherTab::Branches => self.launcher_branches.len(),
            LauncherTab::Commits => self.launcher_commits_rows().len(),
        }
    }

    /// The active tab's effective row count: the active filter's filtered
    /// view when one is set, the full tab's row count otherwise (spec 12's
    /// filtered-view design constraint) — every motion clamps against this.
    fn review_launcher_row_count(&self) -> usize {
        self.launcher_filter
            .as_ref()
            .map_or_else(|| self.review_launcher_raw_row_count(), ListFilter::len)
    }

    /// Builds the active tab's `/`-filterable labels: branch names on the
    /// Branches tab, "`<short-sha> <subject>`" on the Commits tab — whichever
    /// source [`App::launcher_commits_rows`] currently selects.
    fn review_launcher_filter_labels(&self) -> Vec<String> {
        let Mode::ReviewLauncher { tab, .. } = self.mode else {
            return Vec::new();
        };
        match tab {
            LauncherTab::Branches => self
                .launcher_branches
                .iter()
                .map(|b| b.name.clone())
                .collect(),
            LauncherTab::Commits => self
                .launcher_commits_rows()
                .iter()
                .map(|c| format!("{} {}", c.short_sha, c.subject))
                .collect(),
        }
    }

    /// Translates the launcher's cursor (a filtered position while a filter
    /// is active, a raw index otherwise) into a real index into whichever
    /// list backs the active tab — the one point `Enter`
    /// ([`App::review_launcher_confirm`]) and the Commits-tab prefetch check
    /// route through.
    fn review_launcher_real_index(&self, cursor: usize) -> Option<usize> {
        match &self.launcher_filter {
            Some(f) => f.real_index(cursor),
            None => (cursor < self.review_launcher_raw_row_count()).then_some(cursor),
        }
    }

    /// The launcher's cursor translated to a real index (see
    /// [`App::review_launcher_real_index`]), exposed read-only for
    /// integration tests that need to know which real row `Enter` is about
    /// to act on before pressing it. A no-op (`None`) unless the launcher is
    /// open.
    #[cfg(test)]
    pub(super) fn review_launcher_selected_index(&self) -> Option<usize> {
        let Mode::ReviewLauncher { cursor, .. } = self.mode else {
            return None;
        };
        self.review_launcher_real_index(cursor)
    }

    /// Enters filter mode (`/`): a no-op if it's already active (`/` while
    /// locked resumes editing instead — see
    /// [`App::review_launcher_resume_filter_editing`]).
    pub(super) fn review_launcher_enter_filter(&mut self) {
        if self.launcher_filter.is_none() {
            let labels = self.review_launcher_filter_labels();
            self.launcher_filter = Some(ListFilter::open(&labels));
        }
    }

    /// Resumes editing a locked filter (`/` while locked).
    pub(super) fn review_launcher_resume_filter_editing(&mut self) {
        if let Some(f) = self.launcher_filter.as_mut() {
            f.resume_editing();
        }
    }

    /// Locks the active filter (`Enter` while editing), handing key
    /// handling back to the launcher's own verbs.
    pub(super) fn review_launcher_lock_filter(&mut self) {
        if let Some(f) = self.launcher_filter.as_mut() {
            f.lock();
        }
    }

    /// Clears the active filter entirely (`Esc`).
    pub(super) fn review_launcher_clear_filter(&mut self) {
        self.launcher_filter = None;
        self.review_launcher_clamp_cursor_to_len();
    }

    /// Appends `c` to the active filter's query and re-clamps the cursor
    /// into the freshly reranked view. A no-op if no filter is active.
    pub(super) fn review_launcher_filter_push_char(&mut self, c: char) {
        let labels = self.review_launcher_filter_labels();
        if let Some(f) = self.launcher_filter.as_mut() {
            f.push_char(c, &labels);
        }
        self.review_launcher_clamp_cursor_to_len();
    }

    /// Deletes the last character of the active filter's query. A no-op if
    /// no filter is active.
    pub(super) fn review_launcher_filter_backspace(&mut self) {
        let labels = self.review_launcher_filter_labels();
        if let Some(f) = self.launcher_filter.as_mut() {
            f.backspace(&labels);
        }
        self.review_launcher_clamp_cursor_to_len();
    }

    /// Re-clamps the launcher's cursor into `review_launcher_row_count()` —
    /// the effective (filtered or full) length — after the filter mutates.
    fn review_launcher_clamp_cursor_to_len(&mut self) {
        let len = self.review_launcher_row_count();
        if let Mode::ReviewLauncher { cursor, .. } = &mut self.mode {
            *cursor = (*cursor).min(len.saturating_sub(1));
        }
    }

    /// The launcher's page-size proxy for half/full-page motions: like the
    /// git panel and switcher, it has no render height of its own to track
    /// (see [`super::git_panel::App::panel_viewport_proxy`]'s identical
    /// rationale).
    fn review_launcher_viewport_proxy(&self) -> usize {
        self.view.viewport_height()
    }

    /// Steps the launcher's cursor by `step` rows in `down`'s direction,
    /// clamped against the active tab's row count, then re-runs the
    /// Commits tab's lazy-prefetch check so every layer-driven move
    /// (half/full-page, jumps, and the plain `j`/`k` step
    /// [`App::review_launcher_move_down`]/[`App::review_launcher_move_up`]
    /// delegate to) behaves identically (mirrors
    /// [`super::git_panel::App::panel_step`]). A no-op unless the launcher
    /// is open.
    fn review_launcher_step(&mut self, step: usize, down: bool) {
        let len = self.review_launcher_row_count();
        if let Mode::ReviewLauncher { cursor, .. } = &mut self.mode {
            *cursor = super::motion::step(*cursor, len, step, down);
        }
        self.review_launcher_maybe_prefetch_commits();
    }

    /// Jumps the launcher's cursor to `target`, clamped against the active
    /// tab's row count, with the same prefetch bookkeeping as
    /// [`App::review_launcher_step`]. A no-op unless the launcher is open.
    fn review_launcher_jump(&mut self, target: usize) {
        let len = self.review_launcher_row_count();
        if let Mode::ReviewLauncher { cursor, .. } = &mut self.mode {
            *cursor = target.min(len.saturating_sub(1));
        }
        self.review_launcher_maybe_prefetch_commits();
    }

    /// Moves the launcher's cursor down half a viewport (`Ctrl-d`, shared
    /// motion set — see `super::motion`).
    pub(super) fn review_launcher_half_page_down(&mut self) {
        let step = super::motion::half_page(self.review_launcher_viewport_proxy());
        self.review_launcher_step(step, true);
    }

    /// Moves the launcher's cursor up half a viewport (`Ctrl-u`).
    pub(super) fn review_launcher_half_page_up(&mut self) {
        let step = super::motion::half_page(self.review_launcher_viewport_proxy());
        self.review_launcher_step(step, false);
    }

    /// Moves the launcher's cursor down a full viewport (`Ctrl-f`).
    pub(super) fn review_launcher_full_page_down(&mut self) {
        let step = super::motion::full_page(self.review_launcher_viewport_proxy());
        self.review_launcher_step(step, true);
    }

    /// Moves the launcher's cursor up a full viewport (`Ctrl-b`).
    pub(super) fn review_launcher_full_page_up(&mut self) {
        let step = super::motion::full_page(self.review_launcher_viewport_proxy());
        self.review_launcher_step(step, false);
    }

    /// Jumps the launcher's cursor to the first row (`g`/`Home`).
    pub(super) fn review_launcher_jump_to_top(&mut self) {
        self.review_launcher_jump(super::motion::jump_top());
    }

    /// Jumps the launcher's cursor to the last row (`G`/`End`).
    pub(super) fn review_launcher_jump_to_bottom(&mut self) {
        let len = self.review_launcher_row_count();
        self.review_launcher_jump(super::motion::jump_bottom(len));
    }

    /// Re-runs the Commits tab's lazy-prefetch check
    /// ([`App::maybe_prefetch_history`]) after a cursor move — only
    /// meaningful once the tab's "all commits" source is active (the
    /// ahead-of-base source, `launcher_commits`, is a single-shot fetch with
    /// nothing to paginate); a no-op on the Branches tab or while the
    /// ahead-of-base source is active. Translates the cursor through
    /// [`App::review_launcher_real_index`] first, so a filtered position
    /// checks proximity to the end of the *real* (unfiltered) source rather
    /// than the filtered view's own (typically much smaller) length.
    /// Mirrors the git panel History tab's own scroll-triggered prefetch, so
    /// scrolling the launcher's Commits tab (in "all commits" mode) never
    /// has to wait on a visible "load more" action either.
    fn review_launcher_maybe_prefetch_commits(&mut self) {
        let Mode::ReviewLauncher {
            tab: LauncherTab::Commits,
            cursor,
            ..
        } = self.mode
        else {
            return;
        };
        if self.launcher_all_commits
            && let Some(real) = self.review_launcher_real_index(cursor)
        {
            self.maybe_prefetch_history(real);
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

    /// The launcher's `Enter` gesture: translates the cursor to a real index
    /// while a filter is active (a no-op if the filter matches nothing, same
    /// contract as [`super::switcher::SwitcherState`]'s own
    /// `active_real_index`) and dispatches on the active tab. Without a
    /// filter the raw cursor passes straight through unchanged — including
    /// when it's out of range (an empty list) — exactly as before spec 12:
    /// the callee's own guard
    /// ([`App::confirm_launcher_branch_review`]'s in-session check) must
    /// still run *before* any row-emptiness check, so this never resolves
    /// the index down to "nothing to do" ahead of that guard the way
    /// [`App::review_launcher_real_index`] would (that method is for the
    /// prefetch check, where skipping silently on an unresolved index is
    /// correct). A filtered `Enter` on the Branches tab still runs into the
    /// same in-session guard exactly as an unfiltered one does — this only
    /// changes *which* row's index gets passed in, never what the callee
    /// does with it.
    pub(super) fn review_launcher_confirm(&mut self) {
        let Mode::ReviewLauncher { tab, cursor, .. } = self.mode else {
            return;
        };
        let index = match &self.launcher_filter {
            Some(f) => match f.real_index(cursor) {
                Some(i) => i,
                None => return,
            },
            None => cursor,
        };
        match tab {
            LauncherTab::Branches => self.confirm_launcher_branch_review(index),
            LauncherTab::Commits => self.confirm_launcher_commit(index),
        }
    }

    /// Commits-tab `Enter`: opens the highlighted commit (from whichever
    /// source is active — ahead-of-base or the full log) into the existing
    /// read-only single-commit view ([`App::open_commit_view`]), unchanged
    /// from how the History tab's own `Enter` opens a commit — same
    /// suspend/restore semantics, same behavior whether or not a branch-
    /// review session is active (the commit view reads through whatever
    /// `stage_ops` is currently attached, review session or not). A no-op
    /// on an out-of-range cursor (an empty or still-loading list).
    fn confirm_launcher_commit(&mut self, cursor: usize) {
        let Some(sha) = self
            .launcher_commits_rows()
            .get(cursor)
            .map(|c| c.sha.clone())
        else {
            return;
        };
        self.open_commit_view(sha);
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

    // -- Commits tab: data loading (FR-11, FR-12) ----------------------------

    /// The Commits tab's currently-active row source: the ahead-of-base
    /// list (`launcher_commits`) by default, or the full recent-HEAD log
    /// (`history`, the same source the History tab loads) once toggled via
    /// [`App::review_launcher_toggle_all_commits`]. Both are newest-first,
    /// so the cursor starting at `0` always lands on the newest commit
    /// regardless of which source is active.
    pub(super) fn launcher_commits_rows(&self) -> &[CommitLogEntry] {
        if self.launcher_all_commits {
            &self.history
        } else {
            &self.launcher_commits
        }
    }

    /// Whether the active Commits source hasn't produced its first result
    /// yet — drives the tab's loading placeholder, mirroring
    /// [`App::history_loading`] for the all-commits source and the
    /// equivalent single-shot check for the ahead-of-base source.
    pub(super) fn launcher_commits_loading(&self) -> bool {
        if self.launcher_all_commits {
            self.history_loading()
        } else {
            !self.launcher_commits_loaded && self.launcher_commits_in_flight.is_some()
        }
    }

    /// Ensures whichever Commits source is currently active has a load
    /// requested: the all-commits source reuses [`App::ensure_history_loaded`]
    /// verbatim; the ahead-of-base source gets its own single-flight
    /// [`App::ensure_launcher_commits_loaded`]. Called on every path that
    /// makes the Commits tab (or a new source within it) visible: opening
    /// the launcher onto Commits, switching onto Commits, and the `a`
    /// toggle itself.
    pub(super) fn ensure_launcher_commits_source_loaded(&mut self) {
        if self.launcher_all_commits {
            self.ensure_history_loaded();
        } else {
            self.ensure_launcher_commits_loaded();
        }
    }

    /// Toggles the Commits tab between the ahead-of-base list and the full
    /// recent-HEAD log (`a`, remembered for the process lifetime like
    /// `last_launcher_tab`): the newly-active source's load is kicked off
    /// immediately (mirrors `review_launcher_switch_tab` eagerly loading
    /// Branches data) so displaying it never shows a stale, pre-toggle
    /// list, and the cursor resets to the top since the two sources are
    /// different lengths.
    pub(super) fn review_launcher_toggle_all_commits(&mut self) {
        self.launcher_all_commits = !self.launcher_all_commits;
        if let Mode::ReviewLauncher { cursor, .. } = &mut self.mode {
            *cursor = 0;
        }
        self.ensure_launcher_commits_source_loaded();
    }

    /// Kicks off the ahead-of-base fetch if nothing has loaded yet and
    /// nothing is already in flight — single-flight, mirroring
    /// [`App::ensure_history_loaded`]. A no-op on every subsequent call.
    pub(super) fn ensure_launcher_commits_loaded(&mut self) {
        if !self.launcher_commits_loaded && self.launcher_commits_in_flight.is_none() {
            self.request_launcher_commits();
        }
    }

    /// Requests the ahead-of-base commit list: resolves the base
    /// synchronously (a cheap `git symbolic-ref`/`rev-parse`, the same call
    /// the Branches tab's confirm flow already makes on the foreground
    /// thread) and hands the resolved range to the async fetcher, which
    /// runs the actual (potentially larger) `git log` off the render
    /// thread — [`App::poll_launcher_commits`] drains it once per tick.
    /// Falls back to a synchronous fetch for backends that can't cross a
    /// thread boundary (test fakes, git-less contexts), matching every
    /// other lazy-load path's fallback shape. An unresolvable base (no
    /// `origin/HEAD`, no local `main`/`master`) or a git-less context
    /// degrades to "loaded, empty" rather than surfacing an error — the
    /// same silent-degrade contract [`App::load_launcher_branches`]
    /// documents for the Branches tab.
    fn request_launcher_commits(&mut self) {
        if self.launcher_commits_in_flight.is_some() {
            return;
        }
        let Some(ops) = self.stage_ops.as_deref() else {
            self.launcher_commits_loaded = true;
            return;
        };
        let base = match resolve_review_base(ops, None) {
            Ok(base) => base,
            Err(_) => {
                self.launcher_commits_loaded = true;
                return;
            }
        };
        let range = CommitLogRange {
            base,
            head: "HEAD".to_string(),
        };
        if let Some(fetcher) = ops.async_commit_log_range_fetcher() {
            let generation = self.launcher_commits_generation;
            let id = self
                .launcher_commits_tasks
                .spawn(move || fetcher(&range).ok());
            self.launcher_commits_in_flight = Some(InFlightLauncherCommits { id, generation });
        } else {
            match ops.commit_log_range(&range) {
                Ok(commits) => self.apply_launcher_commits(commits),
                Err(_) => self.launcher_commits_loaded = true,
            }
        }
    }

    /// Drains a completed background ahead-of-base fetch (once per
    /// event-loop tick, alongside [`App::poll_history`]). Drops a stale
    /// result — spawned before `launcher_commits_generation` was last
    /// bumped — or a foreign/task-panic/git-error result silently (marking
    /// the load "loaded, empty" rather than leaving the placeholder stuck);
    /// applies a successful list otherwise.
    pub(super) fn poll_launcher_commits(&mut self) {
        for (id, result) in self.launcher_commits_tasks.poll() {
            let Some(in_flight) = self.launcher_commits_in_flight else {
                continue;
            };
            if in_flight.id != id {
                continue;
            }
            self.launcher_commits_in_flight = None;
            if in_flight.generation != self.launcher_commits_generation {
                continue;
            }
            match result {
                Ok(Some(commits)) => self.apply_launcher_commits(commits),
                _ => self.launcher_commits_loaded = true,
            }
        }
    }

    /// Folds a fetched ahead-of-base list into `launcher_commits`, marking
    /// the load complete.
    fn apply_launcher_commits(&mut self, commits: Vec<CommitLogEntry>) {
        self.launcher_commits = commits;
        self.launcher_commits_loaded = true;
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

    /// The same guard, proven to hold when `Enter` is filtered (spec 12
    /// FR-12): a locked filter narrows two real branches down to one
    /// (`"zulu-target"`, real index 1 — deliberately not the first row, so
    /// this can't pass by coincidence the way an unfiltered index-0 case
    /// could), yet the in-session guard still fires before any branch
    /// lookup or worktree call, exactly as the unfiltered case above.
    #[test]
    fn confirm_during_an_active_review_session_emits_the_hint_and_mutates_nothing_under_a_filter() {
        let mut app = app_with_branches(vec![branch("alpha"), branch("zulu-target")]);
        app.open_review_launcher();
        app.target = crate::git::DiffTarget::Review {
            base: "main".to_string(),
            branch: "feature".to_string(),
        };

        app.review_launcher_enter_filter();
        for c in "zulu".chars() {
            app.review_launcher_filter_push_char(c);
        }
        app.review_launcher_lock_filter();
        assert_eq!(
            app.review_launcher_selected_index(),
            Some(1),
            "sanity: the filter must resolve to the second real branch"
        );

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
        // them ran, even though the filter translated the cursor to a
        // non-zero real index before the guard ran.
    }

    // -- Commits tab: data loading (FR-11) -----------------------------------

    /// A minimal `StageOps` fake serving a fixed ahead-of-base commit list
    /// synchronously (no `async_commit_log_range_fetcher`, so
    /// `request_launcher_commits` takes the synchronous fallback path) plus
    /// a fixed `default_base` — mirrors `history_tests.rs`'s
    /// `SyncHistoryFake`.
    struct SyncCommitRangeOps {
        base: &'static str,
        commits: Vec<CommitLogEntry>,
    }

    impl super::super::stage_ops::StageOps for SyncCommitRangeOps {
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
            Ok(())
        }
        fn unstage_file(&self, _path: &str) -> Result<(), crate::git::GitError> {
            Ok(())
        }
        fn apply_cached(&self, _patch: &str) -> Result<(), crate::git::GitError> {
            Ok(())
        }
        fn unapply_cached(&self, _patch: &str) -> Result<(), crate::git::GitError> {
            Ok(())
        }
        fn read_worktree_file(&self, _path: &str) -> Option<Vec<u8>> {
            None
        }
        fn show_file(&self, _spec: &str) -> Option<String> {
            None
        }
        fn default_base(&self) -> Result<String, crate::git::GitError> {
            Ok(self.base.to_string())
        }
        fn commit_log_range(
            &self,
            _range: &CommitLogRange,
        ) -> Result<Vec<CommitLogEntry>, crate::git::GitError> {
            Ok(self.commits.clone())
        }
    }

    fn commit(sha: &str, subject: &str) -> CommitLogEntry {
        CommitLogEntry {
            sha: sha.to_string(),
            short_sha: sha.to_string(),
            subject: subject.to_string(),
            author_name: "Dev".to_string(),
            timestamp: 1_700_000_000,
        }
    }

    fn app_with_commit_range(commits: Vec<CommitLogEntry>) -> App {
        let mut app = app();
        app.stage_ops = Some(Box::new(SyncCommitRangeOps {
            base: "main",
            commits,
        }));
        app
    }

    #[test]
    fn launcher_commits_is_empty_and_not_loading_before_anything_is_requested() {
        let app = app_with_commit_range(vec![commit("a", "one")]);
        assert!(app.launcher_commits.is_empty());
        assert!(!app.launcher_commits_loading());
    }

    #[test]
    fn ensure_launcher_commits_loaded_applies_synchronously_when_no_async_fetcher() {
        let mut app = app_with_commit_range(vec![commit("a", "one"), commit("b", "two")]);
        app.ensure_launcher_commits_loaded();
        assert_eq!(app.launcher_commits.len(), 2);
        assert!(!app.launcher_commits_loading());
        assert!(app.launcher_commits_in_flight.is_none());
    }

    #[test]
    fn no_backend_degrades_to_loaded_and_empty_rather_than_a_stuck_placeholder() {
        let mut app = app();
        app.ensure_launcher_commits_loaded();
        assert!(app.launcher_commits.is_empty());
        assert!(!app.launcher_commits_loading());
    }

    #[test]
    fn an_unresolvable_base_degrades_to_loaded_and_empty() {
        struct NoBaseOps;
        impl super::super::stage_ops::StageOps for NoBaseOps {
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
                Ok(())
            }
            fn unstage_file(&self, _path: &str) -> Result<(), crate::git::GitError> {
                Ok(())
            }
            fn apply_cached(&self, _patch: &str) -> Result<(), crate::git::GitError> {
                Ok(())
            }
            fn unapply_cached(&self, _patch: &str) -> Result<(), crate::git::GitError> {
                Ok(())
            }
            fn read_worktree_file(&self, _path: &str) -> Option<Vec<u8>> {
                None
            }
            fn show_file(&self, _spec: &str) -> Option<String> {
                None
            }
            // `default_base` keeps the trait's own default: an error.
        }
        let mut app = app();
        app.stage_ops = Some(Box::new(NoBaseOps));
        app.ensure_launcher_commits_loaded();
        assert!(app.launcher_commits.is_empty());
        assert!(!app.launcher_commits_loading());
    }

    #[test]
    fn launcher_commits_loading_is_true_while_a_fetch_is_in_flight_and_false_after_it_lands() {
        let mut app = app();
        let id = app
            .launcher_commits_tasks
            .spawn(|| Some(vec![commit("a", "one")]));
        app.launcher_commits_in_flight = Some(InFlightLauncherCommits {
            id,
            generation: app.launcher_commits_generation,
        });
        assert!(
            app.launcher_commits_loading(),
            "placeholder must show while the fetch is in flight"
        );

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.launcher_commits_in_flight.is_some() && std::time::Instant::now() < deadline {
            app.poll_launcher_commits();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            app.launcher_commits_in_flight.is_none(),
            "fetch must have completed"
        );
        assert_eq!(app.launcher_commits.len(), 1);
        assert!(!app.launcher_commits_loading());
    }

    #[test]
    fn stale_generation_launcher_commits_result_is_dropped_not_applied() {
        let mut app = app();
        let stale = vec![commit("stale", "should never appear")];
        let id = app.launcher_commits_tasks.spawn(move || Some(stale));
        app.launcher_commits_in_flight = Some(InFlightLauncherCommits {
            id,
            generation: app.launcher_commits_generation,
        });

        // Something bumps the generation before this fetch lands.
        app.launcher_commits_generation = app.launcher_commits_generation.wrapping_add(1);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            app.poll_launcher_commits();
            if app.launcher_commits_in_flight.is_none() || std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        assert!(
            app.launcher_commits_in_flight.is_none(),
            "stale fetch was consumed"
        );
        assert!(
            app.launcher_commits.is_empty(),
            "a stale-generation result must never be applied"
        );
    }

    #[test]
    fn request_launcher_commits_is_single_flight() {
        let mut app = app();
        let id = app
            .launcher_commits_tasks
            .spawn(|| Some(vec![commit("a", "one")]));
        app.launcher_commits_in_flight = Some(InFlightLauncherCommits {
            id,
            generation: app.launcher_commits_generation,
        });
        app.stage_ops = Some(Box::new(SyncCommitRangeOps {
            base: "main",
            commits: vec![commit("b", "two")],
        }));

        app.ensure_launcher_commits_loaded();

        // Still the original in-flight task; the synchronous fake's list
        // was never applied (a second fetch never started).
        assert_eq!(app.launcher_commits_in_flight.map(|f| f.id), Some(id));
        assert!(app.launcher_commits.is_empty());
    }

    // -- Commits tab: the `a` all-commits toggle (FR-12) ---------------------

    #[test]
    fn toggle_all_commits_switches_between_ahead_of_base_and_the_full_log() {
        let mut app = app_with_commit_range(vec![commit("a", "ahead one")]);
        app.history = vec![commit("h1", "history one"), commit("h2", "history two")];
        app.history_exhausted = true; // pretend the History tab already loaded
        app.mode = Mode::ReviewLauncher {
            tab: LauncherTab::Commits,
            cursor: 0,
            origin: ModeOrigin::Normal,
        };
        app.ensure_launcher_commits_loaded();

        assert!(!app.launcher_all_commits);
        assert_eq!(
            app.launcher_commits_rows()
                .iter()
                .map(|c| c.subject.as_str())
                .collect::<Vec<_>>(),
            vec!["ahead one"]
        );

        app.review_launcher_toggle_all_commits();
        assert!(app.launcher_all_commits);
        assert_eq!(
            app.launcher_commits_rows()
                .iter()
                .map(|c| c.subject.as_str())
                .collect::<Vec<_>>(),
            vec!["history one", "history two"]
        );

        app.review_launcher_toggle_all_commits();
        assert!(!app.launcher_all_commits);
        assert_eq!(
            app.launcher_commits_rows()
                .iter()
                .map(|c| c.subject.as_str())
                .collect::<Vec<_>>(),
            vec!["ahead one"]
        );
    }

    #[test]
    fn toggle_all_commits_resets_the_cursor() {
        let mut app = app_with_commit_range(vec![commit("a", "one"), commit("b", "two")]);
        app.mode = Mode::ReviewLauncher {
            tab: LauncherTab::Commits,
            cursor: 1,
            origin: ModeOrigin::Normal,
        };
        app.review_launcher_toggle_all_commits();
        assert!(matches!(app.mode, Mode::ReviewLauncher { cursor: 0, .. }));
    }

    #[test]
    fn toggle_state_survives_close_and_reopen() {
        let mut app = app_with_commit_range(Vec::new());
        app.open_review_launcher();
        app.review_launcher_switch_tab(); // Branches -> Commits
        app.review_launcher_toggle_all_commits();
        assert!(app.launcher_all_commits);

        app.close_review_launcher();
        app.open_review_launcher();
        assert_eq!(
            app.mode,
            Mode::ReviewLauncher {
                tab: LauncherTab::Commits,
                cursor: 0,
                origin: ModeOrigin::Normal,
            }
        );
        assert!(
            app.launcher_all_commits,
            "the toggle must survive close/reopen for the process lifetime"
        );
    }

    // -- Commits tab: confirm opens the commit view (FR-14) ------------------

    #[test]
    fn confirm_on_commits_tab_opens_the_commit_view() {
        let mut app = app_with_commit_range(Vec::new());
        app.launcher_commits = vec![commit("deadbeef", "a commit")];
        app.mode = Mode::ReviewLauncher {
            tab: LauncherTab::Commits,
            cursor: 0,
            origin: ModeOrigin::Normal,
        };

        app.review_launcher_confirm();

        assert_eq!(app.mode, Mode::Normal);
        assert!(
            matches!(&app.target, crate::git::DiffTarget::Commit(sha) if sha == "deadbeef"),
            "got {:?}",
            app.target
        );
    }

    #[test]
    fn confirm_on_commits_tab_with_an_empty_list_is_a_no_op() {
        let mut app = app();
        app.mode = Mode::ReviewLauncher {
            tab: LauncherTab::Commits,
            cursor: 0,
            origin: ModeOrigin::Normal,
        };
        app.review_launcher_confirm();
        assert!(matches!(app.mode, Mode::ReviewLauncher { .. }));
    }

    // -- Motion layer adoption (spec 12 FR-12) -------------------------------

    #[test]
    fn half_and_full_page_motions_step_and_clamp_on_the_branches_tab() {
        let mut app = app_with_branches(vec![branch("alpha"), branch("beta"), branch("gamma")]);
        app.open_review_launcher();
        assert_eq!(
            app.launcher_branches.len(),
            3,
            "sanity: real branches loaded"
        );

        app.review_launcher_half_page_down();
        app.review_launcher_full_page_down();
        assert_eq!(
            app.mode,
            Mode::ReviewLauncher {
                tab: LauncherTab::Branches,
                cursor: 2,
                origin: ModeOrigin::Normal,
            },
            "half then full page down must clamp at the last of 3 branches"
        );

        app.review_launcher_half_page_up();
        assert_eq!(
            app.mode,
            Mode::ReviewLauncher {
                tab: LauncherTab::Branches,
                cursor: 0,
                origin: ModeOrigin::Normal,
            },
            "half page up must clamp at the first row"
        );

        app.review_launcher_jump_to_bottom();
        assert_eq!(
            app.mode,
            Mode::ReviewLauncher {
                tab: LauncherTab::Branches,
                cursor: 2,
                origin: ModeOrigin::Normal,
            }
        );
        app.review_launcher_jump_to_top();
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
    fn count_prefix_composes_with_a_motion_through_the_real_dispatch() {
        use crate::ui::modes::handle_review_launcher_key;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = app_with_branches(vec![branch("a"), branch("b"), branch("c"), branch("d")]);
        app.open_review_launcher();
        let key = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
        handle_review_launcher_key(&mut app, key('3'));
        handle_review_launcher_key(&mut app, key('j'));
        assert_eq!(
            app.mode,
            Mode::ReviewLauncher {
                tab: LauncherTab::Branches,
                cursor: 3,
                origin: ModeOrigin::Normal,
            },
            "3j must step three rows in one gesture"
        );
    }

    /// A minimal `StageOps` fake serving a fixed commit list synchronously
    /// for the History-tab source the Commits tab's "all commits" toggle
    /// reuses (mirrors `git_panel_tests.rs`'s `PanelHistoryFake`, private to
    /// its own module).
    struct LauncherHistoryFake {
        entries: Vec<CommitLogEntry>,
    }

    impl super::super::stage_ops::StageOps for LauncherHistoryFake {
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
            Ok(())
        }
        fn unstage_file(&self, _path: &str) -> Result<(), crate::git::GitError> {
            Ok(())
        }
        fn apply_cached(&self, _patch: &str) -> Result<(), crate::git::GitError> {
            Ok(())
        }
        fn unapply_cached(&self, _patch: &str) -> Result<(), crate::git::GitError> {
            Ok(())
        }
        fn read_worktree_file(&self, _path: &str) -> Option<Vec<u8>> {
            None
        }
        fn show_file(&self, _spec: &str) -> Option<String> {
            None
        }
        fn commit_log(
            &self,
            count: u32,
            skip: u32,
        ) -> Result<Vec<CommitLogEntry>, crate::git::GitError> {
            let start = (skip as usize).min(self.entries.len());
            let end = (start + count as usize).min(self.entries.len());
            Ok(self.entries[start..end].to_vec())
        }
    }

    /// A layer-driven full-page-down on the Commits tab's "all commits"
    /// source must trigger the same lazy prefetch the git panel's History
    /// tab gets from a plain `j` — mirrors
    /// `git_panel_tests.rs::panel_full_page_down_on_history_tab_triggers_prefetch_near_the_end`.
    #[test]
    fn full_page_down_on_the_all_commits_source_triggers_prefetch_near_the_end() {
        let entries: Vec<CommitLogEntry> = (0..100)
            .map(|i| commit(&format!("c{i}"), "subject"))
            .collect();
        let mut app = app();
        app.stage_ops = Some(Box::new(LauncherHistoryFake { entries }));
        app.ensure_history_loaded();
        assert_eq!(app.history.len(), 100);
        assert!(!app.history_exhausted);

        app.launcher_all_commits = true;
        app.mode = Mode::ReviewLauncher {
            tab: LauncherTab::Commits,
            cursor: 85,
            origin: ModeOrigin::Normal,
        };
        // viewport defaults to 20 -> full page steps 20 -> cursor 99
        // (clamped), within HISTORY_PREFETCH_MARGIN (10) of history.len()
        // (100).
        app.review_launcher_full_page_down();
        assert_eq!(
            app.mode,
            Mode::ReviewLauncher {
                tab: LauncherTab::Commits,
                cursor: 99,
                origin: ModeOrigin::Normal,
            }
        );
        assert!(
            app.history_exhausted,
            "landing within the prefetch margin must have requested (and exhausted) the next page"
        );
    }

    #[test]
    fn prefetch_does_not_fire_against_the_single_shot_ahead_of_base_source() {
        // The ahead-of-base source (`launcher_commits`) is a single-shot
        // fetch, not paginated — moving to its end must never touch
        // `history` (there is nothing to page there).
        let mut app =
            app_with_commit_range((0..5).map(|i| commit(&format!("c{i}"), "s")).collect());
        app.ensure_launcher_commits_loaded();
        assert_eq!(app.launcher_commits.len(), 5);
        app.mode = Mode::ReviewLauncher {
            tab: LauncherTab::Commits,
            cursor: 0,
            origin: ModeOrigin::Normal,
        };
        app.review_launcher_jump_to_bottom();
        assert_eq!(
            app.mode,
            Mode::ReviewLauncher {
                tab: LauncherTab::Commits,
                cursor: 4,
                origin: ModeOrigin::Normal,
            }
        );
        assert!(
            app.history.is_empty(),
            "history must stay untouched — nothing to prefetch on the ahead-of-base source"
        );
    }
}
