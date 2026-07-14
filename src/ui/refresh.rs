//! The refresh subsystem: re-reading the diff/status for the current target
//! and folding the fresh [`ReviewSnapshot`] back into [`super::App`], both
//! synchronously (`R`, staging/remote follow-ups) and via the background
//! working-tree poll. Kept out of `app.rs` so the coordinator stays thin;
//! these methods own the generation-counter staleness guard, the single-flight
//! [`InFlightRefresh`] guard, and the targeted highlight-cache invalidation
//! that `apply_snapshot` performs.

use std::collections::{HashMap, HashSet};

use crate::diff::FileDiff;

use super::App;
use super::Mode;
use super::background::TaskId;
use super::stage_ops::{ReviewError, ReviewSnapshot, StagedState, build_review};

/// An async working-tree refresh awaiting completion. Its [`TaskId`]
/// correlates the background [`ReviewSnapshot`] back to the request; `generation`
/// is the [`App::refresh_generation`] at spawn time, and the drain discards the
/// result if a foreground refresh has since bumped it.
#[derive(Debug, Clone, Copy)]
pub(super) struct InFlightRefresh {
    /// The background task delivering this refresh's snapshot.
    pub(super) id: TaskId,
    /// The refresh generation captured when this read was spawned.
    pub(super) generation: u64,
}

impl App {
    /// Re-runs the diff and status for the current target, rebuilds
    /// files/patches/rows and the staged list, then restores position: the
    /// previously selected file is kept by path if it still exists (else
    /// the index is clamped to the nearest remaining file), and cursor,
    /// scroll, and the staging-panel cursor are clamped into range. On any
    /// git/parse error the state is left unchanged and a footer message is
    /// set. A no-op without a git backend.
    pub(super) fn refresh(&mut self) {
        if let Err(e) = self.rebuild_from_git() {
            self.set_status_message(format!("refresh failed: {e}"));
        }
    }

    /// Re-reads the diff/status for the current target and applies the fresh
    /// snapshot (see [`App::apply_snapshot`]). Surfaces a git/parse failure to
    /// the caller rather than swallowing it; a no-op (and `Ok`) without a git
    /// backend.
    fn rebuild_from_git(&mut self) -> Result<(), ReviewError> {
        // A foreground refresh authoritatively sets the displayed state, so
        // bump the generation: any async snapshot spawned before now is
        // discarded on drain rather than clobbering this newer state.
        self.refresh_generation = self.refresh_generation.wrapping_add(1);
        let snapshot = {
            let Some(ops) = self.stage_ops() else {
                return Ok(());
            };
            build_review(ops, &self.target)?
        };
        self.apply_snapshot(snapshot);
        Ok(())
    }

    /// The `R` action: an unconditional refresh with a footer acknowledgement,
    /// so a manual reload always confirms it ran even when nothing changed.
    pub(super) fn manual_refresh(&mut self) {
        match self.rebuild_from_git() {
            Ok(()) => self.set_status_message("refreshed"),
            Err(e) => self.set_status_message(format!("refresh failed: {e}")),
        }
    }

    /// The *synchronous* fallback for the working-tree poll, used only when the
    /// backend can't cross a thread boundary (test fakes, git-less contexts);
    /// the production path is async ([`App::spawn_auto_refresh`] +
    /// [`App::poll_refresh`]). Re-reads the tree and applies the fresh snapshot
    /// only when it actually changed, returning whether it did — the expensive
    /// row rebuild and cursor restoration are skipped whenever the review is
    /// byte-identical to what's displayed, so idle polling never disturbs
    /// scrolling. Silent on a transient git error, keeping the current view.
    pub(super) fn auto_refresh(&mut self) -> bool {
        let snapshot = {
            let Some(ops) = self.stage_ops() else {
                return false;
            };
            match build_review(ops, &self.target) {
                Ok(snapshot) => snapshot,
                Err(_) => return false,
            }
        };
        if snapshot.files == self.view.files
            && snapshot.staged == self.staged
            && snapshot.staged_states == self.staged_states
        {
            return false;
        }
        self.apply_snapshot(snapshot);
        true
    }

    /// Runs an [`App::auto_refresh`] unless a background reload would be
    /// unwelcome: a mutating git op (remote op or commit) is mid-flight (its
    /// completion refreshes anyway, and the intermediate tree is transient —
    /// mirrors lazygit pausing background refreshes during its own git ops);
    /// the target is a fixed range (nothing to pick up); the user has
    /// in-progress input (Compose, Search, or a commit message being typed)
    /// or a Visual selection anchored to positions a rebuild would move; or
    /// the branch/worktree switcher modal is open (rebuilding under an open
    /// modal is wasted work and could shift `panel_cursor`/`selected_file`
    /// mid-decision — the generation guard would keep it correct either
    /// way, but pausing avoids the churn).
    pub(super) fn maybe_auto_refresh(&mut self) {
        if self.git_op.is_some() || !self.target.is_live() {
            return;
        }
        if matches!(
            self.mode,
            Mode::Compose
                | Mode::Search
                | Mode::Visual { .. }
                | Mode::Switcher
                | Mode::CommitMessage
        ) {
            return;
        }
        // Prefer the async path (git I/O off the render thread); fall back to
        // a synchronous rebuild for backends that can't cross a thread
        // boundary (test fakes, git-less contexts).
        if !self.spawn_auto_refresh() {
            self.auto_refresh();
        }
    }

    /// Spawns a background working-tree read when the backend supports it,
    /// returning whether the async path is handling this poll. Single-flight:
    /// if a refresh is already in flight it reports handled (`true`) without
    /// stacking a second read. Returns `false` only when there is no async
    /// backend, telling [`App::maybe_auto_refresh`] to fall back to a
    /// synchronous refresh.
    fn spawn_auto_refresh(&mut self) -> bool {
        let Some(builder) = self.stage_ops().and_then(|ops| ops.async_review_builder()) else {
            return false;
        };
        if self.refresh_in_flight.is_some() {
            return true;
        }
        let target = self.target.clone();
        let id = self.refresh_tasks.spawn(move || builder(&target).ok());
        self.refresh_in_flight = Some(InFlightRefresh {
            id,
            generation: self.refresh_generation,
        });
        true
    }

    /// Drains a completed async working-tree refresh (once per event-loop tick,
    /// alongside [`App::poll_remote`]). Applies the fresh snapshot only when it
    /// isn't stale — its spawn-time generation still matches the current one —
    /// and only when the review actually changed (the same gate the synchronous
    /// path uses). Task panics, git errors, and no-op results drop silently.
    pub(super) fn poll_refresh(&mut self) {
        for (id, result) in self.refresh_tasks.poll() {
            let Some(in_flight) = self.refresh_in_flight else {
                continue;
            };
            if in_flight.id != id {
                continue;
            }
            self.refresh_in_flight = None;

            // A foreground refresh (e.g. a stage) landed after this read was
            // spawned: the snapshot may predate it, so drop it rather than
            // clobber the newer state. The next poll re-reads a fresh tree.
            if in_flight.generation != self.refresh_generation {
                continue;
            }
            // The spawn was gated on a safe mode, but the user may have entered
            // input/selection in the moment since — don't rebuild rows under an
            // active Compose/Search/Visual. Drop it; the next poll re-reads once
            // a safe mode returns (matches the `maybe_auto_refresh` guard).
            if matches!(
                self.mode,
                Mode::Compose
                    | Mode::Search
                    | Mode::Visual { .. }
                    | Mode::Switcher
                    | Mode::CommitMessage
            ) {
                continue;
            }
            let Ok(Some(snapshot)) = result else {
                continue;
            };
            if snapshot.files == self.view.files
                && snapshot.staged == self.staged
                && snapshot.staged_states == self.staged_states
            {
                continue;
            }
            self.apply_snapshot(snapshot);
        }
    }

    /// Applies a freshly built [`ReviewSnapshot`]: swaps in the new
    /// files/patches/staged state, maintains the collapse map, invalidates
    /// only the highlight-cache entries whose file content changed, rebuilds
    /// rows, and restores the cursor/scroll/staging-panel position.
    ///
    /// `pub(super)` (rather than private) only so the performance tripwire
    /// (`perf_tests.rs`) can time this hot path in isolation without routing
    /// through a full `build_review`; nothing outside the refresh subsystem
    /// calls it in production.
    pub(super) fn apply_snapshot(&mut self, snapshot: ReviewSnapshot) {
        // Remember the cursor's file by path and its offset within that
        // file's section, so the same spot is restored even if files
        // reorder or the section shrinks.
        let cursor_file = self.view.file_of_cursor();
        let previous_path = self.view.files.get(cursor_file).map(|f| f.path.clone());
        let local_offset = self.view.cursor.saturating_sub(
            self.view
                .header_row_of_file
                .get(cursor_file)
                .copied()
                .unwrap_or(0),
        );

        // Take the previous files out (rather than clone) so their content
        // can be compared per path against the incoming snapshot for targeted
        // highlight-cache invalidation below.
        let old_by_path: HashMap<String, FileDiff> = std::mem::take(&mut self.view.files)
            .into_iter()
            .map(|f| (f.path.clone(), f))
            .collect();

        self.view.files = snapshot.files;
        self.patches = snapshot.patches;
        self.staged = snapshot.staged;
        self.staged_states = snapshot.staged_states;
        self.recompute_untracked();
        self.refresh_repo_state();

        // Collapse-map maintenance (spec Unit 2, "nothing hides"):
        // - drop entries for files that left the review, then
        // - auto-expand any collapsed file that is now *partially* staged
        //   (staged, then edited again — its fresh unstaged work must not
        //   stay hidden behind a collapsed header, and it renders `±`).
        // Fully-staged collapsed files stay collapsed (nothing to review),
        // and every other file keeps whatever collapse state it had.
        let present: HashSet<String> = self.view.files.iter().map(|f| f.path.clone()).collect();
        self.view.retain_collapsed(|path| present.contains(path));
        let reexpand: Vec<String> = self
            .view
            .files
            .iter()
            .map(|f| f.path.clone())
            .filter(|path| self.view.is_collapsed(path))
            .filter(|path| {
                self.staged_states.get(path).copied().unwrap_or_default() == StagedState::Partial
            })
            .collect();
        for path in reexpand {
            self.view.set_collapsed(&path, false);
        }

        // Per-file highlight-cache invalidation (spec 03, task 5.1): keep the
        // cached spans for files whose diff content is byte-identical across
        // the refresh, invalidate only files whose `FileDiff` changed (or are
        // newly present), and drop entries for files that left the review so
        // the cache can't grow without bound. `FileDiff` equality is a sound
        // and complete proxy for "the highlighted content could have changed":
        // the diff is a pure function of both sides' whole-file source, so an
        // unchanged `FileDiff` means unchanged content and still-valid spans
        // (renames included — `old_path` is part of the compared value). The
        // cache is keyed by each file's current path, matching `rebuild_rows`.
        for file in &self.view.files {
            if old_by_path.get(&file.path) != Some(file) {
                self.highlight_cache.invalidate_path(&file.path);
            }
        }
        self.highlight_cache
            .retain_paths(|path| present.contains(path));
        self.rebuild_rows();
        if self.view.rows.is_empty() {
            self.view.cursor = 0;
            self.view.scroll = 0;
        } else {
            let restored = previous_path
                .as_deref()
                .and_then(|path| self.view.files.iter().position(|f| f.path == path));
            let target = match restored {
                Some(j) => {
                    let (start, end) = self.view.section_span(j);
                    (start + local_offset).min(end.saturating_sub(1))
                }
                None => self.view.cursor.min(self.view.max_cursor()),
            };
            self.view.cursor = self
                .view
                .nearest_addressable(target.min(self.view.max_cursor()), false);
            self.view.scroll = self.view.scroll.min(self.view.cursor);
            self.view.ensure_visible();
        }
        self.staging_cursor = self.staging_cursor.min(self.staged.len().saturating_sub(1));
        // The git panel's navigable rows (files + stashes) can shrink on a
        // refresh; re-clamp the focused panel cursor here — the single place
        // it needs clamping outside its own motion helpers — so the panel
        // renderer can trust it. Inactive (unfocused) panels carry no cursor.
        if matches!(self.mode, Mode::Panel { .. }) {
            let len = super::git_panel::navigable_rows(self).len();
            if let Mode::Panel { cursor } = &mut self.mode {
                *cursor = (*cursor).min(len.saturating_sub(1));
            }
        }
    }
}
