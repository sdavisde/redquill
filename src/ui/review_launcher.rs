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

use crate::git::{CommitLogEntry, CommitLogRange, DiffTarget, GitRunner};

use super::app::{App, Mode, ModeOrigin};
use super::background::TaskId;
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
        match tab {
            LauncherTab::Branches => self.load_launcher_branches(),
            LauncherTab::Commits => self.ensure_launcher_commits_source_loaded(),
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
        match new_tab {
            LauncherTab::Branches => self.load_launcher_branches(),
            LauncherTab::Commits => self.ensure_launcher_commits_source_loaded(),
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
    /// the active Commits source's length on Commits — kept as its own
    /// method so that work only needs to change one arm, mirroring how
    /// [`super::git_panel::App::panel_row_count`] centralizes the git
    /// panel's per-tab length.
    fn review_launcher_row_count(&self) -> usize {
        let Mode::ReviewLauncher { tab, .. } = self.mode else {
            return 0;
        };
        match tab {
            LauncherTab::Branches => self.launcher_branches.len(),
            LauncherTab::Commits => self.launcher_commits_rows().len(),
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

    /// The launcher's `Enter` gesture: dispatches on the active tab.
    pub(super) fn review_launcher_confirm(&mut self) {
        let Mode::ReviewLauncher { tab, cursor, .. } = self.mode else {
            return;
        };
        match tab {
            LauncherTab::Branches => self.confirm_launcher_branch_review(cursor),
            LauncherTab::Commits => self.confirm_launcher_commit(cursor),
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
}
