//! The full-screen Project Search view ([`Mode::ProjectSearch`]): a live,
//! debounced project-wide grep over the worktree on disk, streaming results
//! from [`crate::search::spawn_scan`] into [`ProjectSearchState`], grouped by
//! file for [`super::project_search_view`] to render.
//!
//! Opening this view never touches `App::view`/`App::target` — it has its own
//! dedicated state here — so `Esc` back to the diff just restores
//! [`ProjectSearchState::return_mode`]. The read-only file view a hit's
//! `Enter` opens is a nested suspension landing back in `Mode::ProjectSearch`
//! rather than `Mode::Normal`, so the query/toggles/results/selection survive
//! that round trip; the in-flight scan keeps streaming in the background
//! regardless. Every query-affecting change aborts the in-flight scan and
//! (re)starts a [`DEBOUNCE`]-length window via
//! [`App::note_project_search_change`]; once it elapses,
//! [`App::fire_project_search`] spawns a new scan tagged with the current
//! `generation`, so stale results are dropped on arrival.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use crate::config::SearchConfig;
use crate::search::{
    CaseMode, ScanMessage, ScanOptions, ScanSummary, SearchHit, SearchQuery, spawn_scan,
};

use super::app::{App, Mode};

/// How long after the last query-affecting change to wait before firing a
/// new scan. Long enough to coalesce a fast typist's burst into one scan;
/// short enough to feel live.
pub(super) const DEBOUNCE: Duration = Duration::from_millis(140);

/// Minimum query length (in `char`s) that fires a scan. Shorter queries show
/// no results and no error — not "invalid", just "too short to search yet".
pub(super) const MIN_QUERY_LEN: usize = 2;

/// Which half of the Project Search view is receiving keystrokes. `Input`
/// types into the query buffer and live-searches as before; `Results` routes
/// `j`/`k`/`Up`/`Down`
/// to result navigation instead, freeing those letters up (the same reason
/// the finder/search tables only bind `Up`/`Down`, not `j`/`k`, still
/// applies while `Input` is active). `Esc` moves `Input` -> `Results`
/// (without closing the view) and `Results` -> closes the view; `/` and
/// `Tab` move `Results` -> `Input` (`Tab` toggles either direction). See
/// [`super::modes::handle_project_search_key`] for the exact dispatch.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(super) enum SearchFocus {
    /// Typing edits the query; Up/Down still navigate results (unaffected by
    /// the focus split, since they were never in the query's own alphabet).
    #[default]
    Input,
    /// `j`/`k`/Up/Down navigate results; letters no longer type into the
    /// query; `/` returns to `Input` with the query preserved.
    Results,
}

/// One background scan currently streaming results, tagged with the
/// generation it was spawned under so a consumer can recognize its own
/// staleness (see the module doc's generation discussion).
pub(super) struct InFlightScan {
    pub(super) generation: u64,
    pub(super) receiver: Receiver<ScanMessage>,
    pub(super) abort: Arc<AtomicBool>,
}

/// One file's hits, in the order they streamed in (ascending line number
/// within a file, since a single file is always searched sequentially by one
/// worker — see `crate::search::engine`'s module doc). Groups themselves
/// appear in first-batch-seen order, which is not globally sorted (files are
/// scanned concurrently across threads) — expected for a live streaming
/// search, and matches how a terminal grep tool's output arrives too.
pub(super) struct ResultGroup {
    pub(super) path: String,
    pub(super) hits: Vec<SearchHit>,
}

/// The Project Search view's full state (see the module doc for the
/// suspend/restore and debounce contracts).
pub(super) struct ProjectSearchState {
    /// The free-text query buffer (a regex unless `literal` is set).
    pub(super) query: String,
    pub(super) case: CaseMode,
    pub(super) whole_word: bool,
    pub(super) literal: bool,
    /// Results grouped by file, accumulated from the current generation's
    /// scan batches; cleared whenever a new scan actually spawns (never on
    /// an invalid-regex failure — see the module doc).
    pub(super) groups: Vec<ResultGroup>,
    /// The selected result, as a flat index across `groups` in display
    /// order (see [`ProjectSearchState::total_hits`]).
    pub(super) cursor: usize,
    /// Bumped by every query-affecting change (see
    /// [`App::note_project_search_change`]); tags the scan spawned once the
    /// debounce settles, so stale batches/summaries from a superseded query
    /// are dropped on arrival.
    pub(super) generation: u64,
    /// When the current debounce window elapses and
    /// [`App::fire_project_search`] should run; `None` while nothing is
    /// pending.
    pub(super) debounce_deadline: Option<Instant>,
    /// The scan currently streaming results, if any.
    pub(super) scan: Option<InFlightScan>,
    /// The most recently completed scan's summary (counts, capped/aborted
    /// flags), for the summary line.
    pub(super) summary: Option<ScanSummary>,
    /// An inline regex-compile error from the most recent
    /// [`App::fire_project_search`] attempt, if the pattern didn't compile.
    /// Cleared the next time a query successfully spawns a scan; never
    /// clears `groups`/`summary` itself (see the module doc).
    pub(super) error: Option<String>,
    /// The mode to restore on the final `Esc` (the mode this view was
    /// opened from) — mirrors [`super::file_finder::FinderState::return_mode`].
    pub(super) return_mode: Mode,
    /// Which half of the view keystrokes route to (see [`SearchFocus`]).
    /// Always starts `Input` on open (`g/`), matching the pre-focus-split
    /// behavior of typing immediately.
    pub(super) focus: SearchFocus,
}

impl ProjectSearchState {
    /// Builds fresh state with the built-in defaults (`CaseMode::Smart`,
    /// both toggles off) — used directly by every test that doesn't care
    /// about `[search]` startup defaults; production
    /// code opens through [`ProjectSearchState::seeded`] instead, via
    /// [`super::App::open_project_search`], so this convenience is
    /// test-only.
    #[cfg(test)]
    pub(super) fn new(return_mode: Mode) -> ProjectSearchState {
        ProjectSearchState::seeded(return_mode, SearchConfig::default())
    }

    /// Builds fresh state seeded from `defaults` (`[search]`'s
    /// `case`/`whole_word`/`literal`): only the *startup*
    /// toggle state a fresh session opens with — an already-open session's
    /// in-session toggles (`Alt-c`/`Alt-w`/`Alt-r`) are never touched by a
    /// config reload, since there is none (config loads exactly once, at
    /// startup).
    pub(super) fn seeded(return_mode: Mode, defaults: SearchConfig) -> ProjectSearchState {
        ProjectSearchState {
            query: String::new(),
            case: defaults.case,
            whole_word: defaults.whole_word,
            literal: defaults.literal,
            groups: Vec::new(),
            cursor: 0,
            generation: 0,
            debounce_deadline: None,
            scan: None,
            summary: None,
            error: None,
            return_mode,
            focus: SearchFocus::Input,
        }
    }

    /// The total number of hits across every group — the flat navigable
    /// count [`App::project_search_move_down`]/[`App::project_search_move_up`]
    /// clamp `cursor` against.
    pub(super) fn total_hits(&self) -> usize {
        self.groups.iter().map(|g| g.hits.len()).sum()
    }
}

/// Cycles the case toggle (`Alt-c`): Smart -> Sensitive -> Insensitive ->
/// Smart. A pure function so its rotation order is independently testable
/// without constructing an `App`.
fn cycle_case(mode: CaseMode) -> CaseMode {
    match mode {
        CaseMode::Smart => CaseMode::Sensitive,
        CaseMode::Sensitive => CaseMode::Insensitive,
        CaseMode::Insensitive => CaseMode::Smart,
    }
}

/// Whether a pending debounce `deadline` has elapsed as of `now`. `None`
/// (nothing pending) never fires. A pure function over explicit `Instant`s
/// (rather than reading the system clock itself) so the generation/debounce
/// contract is testable without sleeping — see [`App::maybe_fire_project_search`],
/// which is the only caller and the one that supplies a real `Instant::now()`
/// in production.
fn debounce_elapsed(deadline: Option<Instant>, now: Instant) -> bool {
    deadline.is_some_and(|deadline| now >= deadline)
}

/// Appends `hit` into the group matching its path (creating one, in
/// first-seen order, if none exists yet). A linear scan over `groups` rather
/// than a `HashMap` index: simple, and cheap enough at the engine's own hit
/// cap (10,000) against realistic file counts — see `crate::search::engine`'s
/// perf tripwire for the scan side's own budget, which this doesn't compete
/// with (grouping happens on the render/poll thread, off the scan's own
/// worker threads).
fn push_hit(groups: &mut Vec<ResultGroup>, hit: SearchHit) {
    if let Some(group) = groups.iter_mut().find(|g| g.path == hit.path) {
        group.hits.push(hit);
    } else {
        groups.push(ResultGroup {
            path: hit.path.clone(),
            hits: vec![hit],
        });
    }
}

impl App {
    /// Opens the full-screen Project Search view (`g/`): captures the
    /// current mode as the close-restore point and switches to
    /// [`Mode::ProjectSearch`]. Deliberately does *not* touch `view`/
    /// `target` — see the module doc.
    pub(super) fn open_project_search(&mut self) {
        let return_mode = self.mode;
        self.project_search = Some(ProjectSearchState::seeded(return_mode, self.config.search));
        self.mode = Mode::ProjectSearch;
    }

    /// Closes the Project Search view (`Esc`, the final "leave the feature"
    /// gesture, as opposed to a nested file-view return): aborts any
    /// in-flight scan promptly and restores the mode captured on open. A
    /// no-op if the view isn't open.
    pub(super) fn close_project_search(&mut self) {
        let Some(state) = self.project_search.take() else {
            return;
        };
        if let Some(scan) = state.scan {
            scan.abort.store(true, Ordering::Relaxed);
        }
        self.mode = state.return_mode;
    }

    /// The Project Search view's `Esc` gesture: a two-step unwind rather
    /// than an immediate exit. From [`SearchFocus::Input`]
    /// it only moves focus to [`SearchFocus::Results`] (the view stays open,
    /// vim motions become live) — from [`SearchFocus::Results`] it's the
    /// final "leave the feature" gesture, delegating to
    /// [`App::close_project_search`] for the existing lossless unwind. A
    /// no-op if the view isn't open.
    pub(super) fn project_search_esc(&mut self) {
        let Some(state) = self.project_search.as_mut() else {
            return;
        };
        match state.focus {
            SearchFocus::Input => state.focus = SearchFocus::Results,
            SearchFocus::Results => self.close_project_search(),
        }
    }

    /// Switches focus back to the query input (`/`, from
    /// [`SearchFocus::Results`]): the query buffer and cursor position are
    /// untouched — there's no separate text-cursor offset to restore, since
    /// typing/backspace always act at the end of the buffer, so "cursor at
    /// end" falls out for free. A no-op if the view isn't open.
    pub(super) fn project_search_focus_input(&mut self) {
        if let Some(state) = self.project_search.as_mut() {
            state.focus = SearchFocus::Input;
        }
    }

    /// Toggles focus between the query input and the results list (`Tab`,
    /// either direction). A no-op if the view isn't open.
    pub(super) fn project_search_toggle_focus(&mut self) {
        if let Some(state) = self.project_search.as_mut() {
            state.focus = match state.focus {
                SearchFocus::Input => SearchFocus::Results,
                SearchFocus::Results => SearchFocus::Input,
            };
        }
    }

    /// The view's current focus (see [`SearchFocus`]), or
    /// [`SearchFocus::Input`] if the view isn't open — a single named helper
    /// so "which hint table applies" is answered consistently everywhere it's
    /// asked (`super::footer`'s strip, `super::help`'s overlay), per the
    /// repo's convention for predicates asked in more than one place.
    pub(super) fn project_search_focus(&self) -> SearchFocus {
        self.project_search
            .as_ref()
            .map(|state| state.focus)
            .unwrap_or_default()
    }

    /// Appends `c` to the query buffer and notes the change (see
    /// [`App::note_project_search_change`]). A no-op if the view isn't open.
    pub(super) fn project_search_input_char(&mut self, c: char) {
        if let Some(state) = self.project_search.as_mut() {
            state.query.push(c);
        }
        self.note_project_search_change(Instant::now());
    }

    /// Deletes the last character of the query buffer and notes the change.
    /// A no-op if the view isn't open (or the buffer is already empty).
    pub(super) fn project_search_backspace(&mut self) {
        if let Some(state) = self.project_search.as_mut() {
            state.query.pop();
        }
        self.note_project_search_change(Instant::now());
    }

    /// Cycles the case toggle (`Alt-c`; see [`cycle_case`]) and notes the
    /// change.
    pub(super) fn project_search_toggle_case(&mut self) {
        if let Some(state) = self.project_search.as_mut() {
            state.case = cycle_case(state.case);
        }
        self.note_project_search_change(Instant::now());
    }

    /// Toggles whole-word matching (`Alt-w`) and notes the change.
    pub(super) fn project_search_toggle_whole_word(&mut self) {
        if let Some(state) = self.project_search.as_mut() {
            state.whole_word = !state.whole_word;
        }
        self.note_project_search_change(Instant::now());
    }

    /// Toggles regex-vs-literal (`Alt-r`) and notes the change.
    pub(super) fn project_search_toggle_literal(&mut self) {
        if let Some(state) = self.project_search.as_mut() {
            state.literal = !state.literal;
        }
        self.note_project_search_change(Instant::now());
    }

    /// Moves the result selection down one hit, clamped at the last (or
    /// pinned at 0 with no results). A no-op if the view isn't open.
    pub(super) fn project_search_move_down(&mut self) {
        if let Some(state) = self.project_search.as_mut() {
            let total = state.total_hits();
            if total > 0 {
                state.cursor = (state.cursor + 1).min(total - 1);
            }
        }
    }

    /// Moves the result selection up one hit, clamped at the first. A no-op
    /// if the view isn't open.
    pub(super) fn project_search_move_up(&mut self) {
        if let Some(state) = self.project_search.as_mut() {
            state.cursor = state.cursor.saturating_sub(1);
        }
    }

    /// The currently selected hit (`cursor`, walked across `groups` in
    /// display order), if any.
    pub(super) fn selected_project_search_hit(&self) -> Option<&SearchHit> {
        let state = self.project_search.as_ref()?;
        let mut remaining = state.cursor;
        for group in &state.groups {
            if remaining < group.hits.len() {
                return group.hits.get(remaining);
            }
            remaining -= group.hits.len();
        }
        None
    }

    /// The Project Search view's `Enter` gesture: opens the selected hit's
    /// file in the read-only file view, cursor on the hit line, landing
    /// back in `Mode::ProjectSearch` (not `Mode::Normal`) on `Esc` —
    /// see [`App::open_file_view_with_return_mode`]. Leaves
    /// `self.project_search` untouched (query/toggles/results/selection all
    /// survive the round trip). A no-op (view stays open) if nothing is
    /// selected.
    pub(super) fn project_search_confirm(&mut self) {
        let Some(hit) = self.selected_project_search_hit() else {
            return;
        };
        let path = hit.path.clone();
        let line = u32::try_from(hit.line_number).ok();
        self.open_file_view_with_return_mode(path, line, Mode::ProjectSearch);
    }

    /// Called on every query-affecting change (a typed char, backspace, or
    /// toggle): aborts any in-flight scan immediately (its results are about
    /// to become stale), bumps `generation` (so any batches already in the
    /// channel from that scan are dropped on arrival — see
    /// [`App::drain_project_search_scan`]), and (re)starts the debounce
    /// window from `now`. A no-op if the view isn't open.
    fn note_project_search_change(&mut self, now: Instant) {
        let Some(state) = self.project_search.as_mut() else {
            return;
        };
        if let Some(scan) = state.scan.take() {
            scan.abort.store(true, Ordering::Relaxed);
        }
        state.generation = state.generation.wrapping_add(1);
        state.debounce_deadline = Some(now + DEBOUNCE);
    }

    /// Fires a new scan if the debounce window has elapsed as of `now` (see
    /// [`debounce_elapsed`]). Split out from [`App::poll_project_search`] (the
    /// per-tick production caller, which supplies `Instant::now()`) so the
    /// debounce contract is testable with synthetic `Instant`s, no sleeping.
    pub(super) fn maybe_fire_project_search(&mut self, now: Instant) {
        let should_fire = self
            .project_search
            .as_ref()
            .is_some_and(|state| debounce_elapsed(state.debounce_deadline, now));
        if should_fire {
            self.fire_project_search();
        }
    }

    /// Actually spawns the scan once the debounce settles. Below
    /// [`MIN_QUERY_LEN`], clears any results/summary/error instead (no scan
    /// — nothing to show for a too-short query, and any prior scan was
    /// already aborted by [`App::note_project_search_change`]). On an
    /// invalid pattern, only `error` is set — `groups`/`summary` are left
    /// exactly as they were, so a bad keystroke never wipes good results. On
    /// success, clears `groups`/`summary`/`error` and starts accumulating
    /// the fresh scan's batches.
    fn fire_project_search(&mut self) {
        let Some(state) = self.project_search.as_mut() else {
            return;
        };
        state.debounce_deadline = None;
        if state.query.chars().count() < MIN_QUERY_LEN {
            state.groups.clear();
            state.cursor = 0;
            state.summary = None;
            state.error = None;
            return;
        }
        let generation = state.generation;
        let query = SearchQuery {
            pattern: state.query.clone(),
            case: state.case,
            whole_word: state.whole_word,
            literal: state.literal,
        };
        let Some(root) = self.repo_root.clone() else {
            if let Some(state) = self.project_search.as_mut() {
                state.error = Some("search unavailable (no repo root)".to_string());
            }
            return;
        };
        match spawn_scan(root, query, generation, ScanOptions::default()) {
            Ok((receiver, abort)) => {
                if let Some(state) = self.project_search.as_mut() {
                    state.error = None;
                    state.groups.clear();
                    state.cursor = 0;
                    state.summary = None;
                    state.scan = Some(InFlightScan {
                        generation,
                        receiver,
                        abort,
                    });
                }
            }
            Err(e) => {
                if let Some(state) = self.project_search.as_mut() {
                    state.error = Some(e.to_string());
                }
            }
        }
    }

    /// Drains the current scan's channel (non-blocking, once per tick — see
    /// [`App::poll_project_search`]): appends batches whose generation still
    /// matches (dropping stragglers from a superseded query), and records the
    /// terminal summary the same way. Clears `scan` once `Done` arrives, the
    /// channel disconnects, or the in-flight scan is found stale (defensive —
    /// `note_project_search_change` already clears it on every query change,
    /// so this shouldn't normally trigger).
    fn drain_project_search_scan(&mut self) {
        let Some(state) = self.project_search.as_mut() else {
            return;
        };
        let generation = state.generation;
        let mut clear_scan = false;
        if let Some(scan) = state.scan.as_ref() {
            if scan.generation != generation {
                clear_scan = true;
            } else {
                loop {
                    match scan.receiver.try_recv() {
                        Ok(ScanMessage::Batch(hits)) => {
                            for hit in hits {
                                if hit.generation == generation {
                                    push_hit(&mut state.groups, hit);
                                }
                            }
                        }
                        Ok(ScanMessage::Done(summary)) => {
                            if summary.generation == generation {
                                state.summary = Some(summary);
                            }
                            clear_scan = true;
                            break;
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            clear_scan = true;
                            break;
                        }
                    }
                }
            }
        }
        if clear_scan {
            state.scan = None;
        }
    }

    /// The per-tick Project Search poll (see `super::mod`'s event loop,
    /// alongside `poll_finder`/`poll_history`/etc.): drains any streaming
    /// scan results, then fires a fresh scan if the debounce has elapsed.
    /// Runs regardless of the current mode — kept alive while a hit's file
    /// view is showing on top (see the module doc) — so results keep
    /// streaming in behind it.
    pub(super) fn poll_project_search(&mut self) {
        self.drain_project_search_scan();
        self.maybe_fire_project_search(Instant::now());
    }
}

#[cfg(test)]
#[path = "project_search_tests.rs"]
mod tests;
