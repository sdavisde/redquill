//! History-tab background loading (spec 05 Unit 3): fetches commit-log pages
//! off the render thread via the same single-flight + generation-counter
//! pattern [`super::refresh`] uses for the working-tree poll, so scrolling
//! the git panel's History tab never blocks on `git log`.
//!
//! Pagination is simple and forward-only: [`super::App::history`]
//! accumulates pages in order and never discards one; [`App::request_history_page`]
//! asks for the next page (single-flight: a no-op while one is already in
//! flight or history is known exhausted); [`App::poll_history`] drains a
//! completed fetch once per event-loop tick (alongside
//! [`super::App::poll_git_ops`]/[`super::App::poll_refresh`]), dropping a
//! result whose generation predates the current one exactly as
//! [`super::refresh::InFlightRefresh`] does for the working-tree read.

use crate::git::CommitLogEntry;

use super::App;
use super::background::TaskId;

/// Commits requested per background page fetch. A tuning choice (spec 05
/// Open Question 3), not a contract.
pub(super) const HISTORY_PAGE_SIZE: u32 = 100;

/// How close to the end of what's loaded the panel cursor must get (rows
/// remaining) before the next page is requested — so scrolling the History
/// tab never has to wait on a visible "load more" action.
pub(super) const HISTORY_PREFETCH_MARGIN: usize = 10;

/// A background commit-log page fetch awaiting completion. Mirrors
/// [`super::refresh::InFlightRefresh`]'s shape exactly.
#[derive(Debug, Clone, Copy)]
pub(super) struct InFlightHistory {
    /// The background task delivering this fetch's page.
    pub(super) id: TaskId,
    /// The history generation captured when this fetch was spawned.
    pub(super) generation: u64,
}

impl App {
    /// Whether the History tab's first page hasn't arrived yet — drives the
    /// tab's loading placeholder. `false` once at least one page (possibly
    /// empty, for a repository with no commits) has landed, or once nothing
    /// is in flight (a git-less context, where nothing will ever load).
    pub(super) fn history_loading(&self) -> bool {
        self.history.is_empty() && !self.history_exhausted && self.history_in_flight.is_some()
    }

    /// Kicks off the History tab's first page fetch if nothing has loaded
    /// yet and nothing is already in flight. A no-op on every subsequent
    /// call — pages accumulate in `self.history` and are never discarded, so
    /// re-entering the tab never re-fetches what's already there.
    pub(super) fn ensure_history_loaded(&mut self) {
        if self.history.is_empty() && self.history_in_flight.is_none() && !self.history_exhausted {
            self.request_history_page();
        }
    }

    /// Requests the next commit-log page (`skip = self.history.len()`),
    /// single-flight: a no-op while a fetch is already in flight or history
    /// is known exhausted. Prefers the async path (off the render thread
    /// via [`crate::ui::stage_ops::StageOps::async_commit_log_fetcher`]);
    /// falls back to a synchronous fetch for backends that can't cross a
    /// thread boundary (test fakes, git-less contexts) — the same fallback
    /// shape [`super::refresh::App::spawn_auto_refresh`] uses.
    pub(super) fn request_history_page(&mut self) {
        if self.history_in_flight.is_some() || self.history_exhausted {
            return;
        }
        let skip = self.history.len() as u32;
        let Some(ops) = self.stage_ops() else {
            return;
        };
        if let Some(fetcher) = ops.async_commit_log_fetcher() {
            let generation = self.history_generation;
            let id = self
                .history_tasks
                .spawn(move || fetcher(HISTORY_PAGE_SIZE, skip).ok());
            self.history_in_flight = Some(InFlightHistory { id, generation });
        } else if let Ok(page) = ops.commit_log(HISTORY_PAGE_SIZE, skip) {
            self.apply_history_page(page);
        }
    }

    /// Drains a completed background commit-log fetch (once per event-loop
    /// tick). Drops a stale result — spawned before `history_generation` was
    /// last bumped — or a foreign/task-panic result silently; applies a
    /// successful page otherwise.
    pub(super) fn poll_history(&mut self) {
        for (id, result) in self.history_tasks.poll() {
            let Some(in_flight) = self.history_in_flight else {
                continue;
            };
            if in_flight.id != id {
                continue;
            }
            self.history_in_flight = None;
            if in_flight.generation != self.history_generation {
                continue;
            }
            let Ok(Some(page)) = result else { continue };
            self.apply_history_page(page);
        }
    }

    /// Folds one fetched page into `self.history`: a short page (fewer
    /// commits than requested) means history is exhausted.
    fn apply_history_page(&mut self, page: Vec<CommitLogEntry>) {
        if (page.len() as u32) < HISTORY_PAGE_SIZE {
            self.history_exhausted = true;
        }
        self.history.extend(page);
    }

    /// Called after the History tab's cursor moves to `cursor`: requests the
    /// next page once the cursor is within [`HISTORY_PREFETCH_MARGIN`] rows
    /// of the end of what's loaded.
    pub(super) fn maybe_prefetch_history(&mut self, cursor: usize) {
        if self.history_exhausted || self.history.is_empty() {
            return;
        }
        if cursor + HISTORY_PREFETCH_MARGIN >= self.history.len() {
            self.request_history_page();
        }
    }
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
