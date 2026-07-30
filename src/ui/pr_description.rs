//! The read-only PR/MR description overlay
//! ([`super::app::Mode::PrDescription`]): what the PR is *about*, without
//! leaving the terminal. Reachable two ways — `d` on the Review launcher's
//! Pull Requests tab (before committing to a review) and the `gi` chord
//! inside a PR review session (while reading the diff) — and read-only by
//! construction: nothing here can write to the forge.
//!
//! **Silent degradation is the contract.** The detail read
//! ([`super::stage_ops::AsyncPrDetailFetcher`]) runs off the render thread and
//! resolves to one of exactly two stored values: the parsed
//! [`crate::forge::PrDetail`], or [`PrDetailOutcome::Unavailable`]. Every
//! failure mode — a missing/unauthenticated CLI, an offline machine, a
//! non-zero exit, unparseable output, a backend with no fetcher at all, or a
//! panicking task — folds into `Unavailable`, which the overlay renders as a
//! dim "description unavailable" line. No error modal, no status-line
//! interruption, no blocked keystroke: the reviewer keeps reading the diff.
//!
//! **Caching is per PR number, for the process lifetime**
//! ([`super::app::App::pr_details`]): opening the overlay for a number already
//! in the map renders immediately with no second round trip, so flipping
//! between rows costs one fetch each. A description barely changes mid-review,
//! and a stale body is strictly better than a spinner; a genuine refresh is a
//! restart away. The overlay's own state carries the number it was opened for
//! and every render looks the body up *by that number*, so a landing fetch for
//! one PR can never paint another PR's body.
//!
//! **Stale results are dropped, never applied.** Fetches are single-flight
//! with a generation counter (mirroring
//! [`super::forge_threads::App::spawn_thread_fetch`]): a result whose task id
//! or generation no longer matches the in-flight request is discarded. Applying
//! a result only ever writes into the cache — it never touches `mode` — so a
//! fetch landing after the overlay closed cannot reopen it or disturb anything
//! else.

use std::cell::Cell;

use crate::forge::PrDetail;

use super::app::{App, Mode, ModeOrigin};
use super::background::TaskId;
use super::review_launcher::LauncherTab;

/// Where closing the description overlay returns to — captured at open time
/// so `q`/`Esc` restores exactly what was on screen. `Copy` (like every
/// [`Mode`] payload) so the mode enum stays `Copy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrDescriptionReturn {
    /// Opened from the Review launcher's Pull Requests tab: closing reopens
    /// the launcher on that tab with the same row highlighted (`cursor` is the
    /// launcher's own cursor, a filtered position while a filter is active),
    /// carrying the origin `R` was pressed from so a later `Esc` still gets
    /// back there.
    Launcher { cursor: usize, origin: ModeOrigin },
    /// Opened from inside a review session: closing returns to the diff.
    Session,
}

impl PrDescriptionReturn {
    /// The mode this return path restores to.
    fn restore(self) -> Mode {
        match self {
            PrDescriptionReturn::Launcher { cursor, origin } => Mode::ReviewLauncher {
                tab: LauncherTab::PullRequests,
                cursor,
                origin,
            },
            PrDescriptionReturn::Session => Mode::Normal,
        }
    }
}

/// The open overlay's own state: which PR it shows, and the scroll/viewport
/// pair the renderer clamps. The scroll model deliberately follows
/// [`super::help::HelpOverlayState`]'s rather than
/// [`super::forge_threads::ThreadViewState`]'s: the key handler advances a
/// `Cell` offset freely (including a `u16::MAX` "jump to end" sentinel) and
/// [`super::pr_description_modal::render`] clamps it against the real wrapped
/// content height each frame and writes the clamped value back, so the stored
/// offset can never run past the end of a short description.
pub(super) struct PrDescriptionState {
    /// The PR/MR number this overlay was opened for — the key every body
    /// lookup goes through, so one PR's description can never render under
    /// another's number.
    pub(super) number: u64,
    /// The vertical scroll offset, clamped at render time.
    pub(super) scroll: Cell<u16>,
    /// The scrollable region's height, recorded by `render` each frame.
    pub(super) viewport: Cell<u16>,
}

impl PrDescriptionState {
    pub(super) fn new(number: u64) -> PrDescriptionState {
        PrDescriptionState {
            number,
            scroll: Cell::new(0),
            viewport: Cell::new(0),
        }
    }
}

/// What a resolved detail read left in the cache: the parsed detail, or the
/// single "we couldn't read it" value every failure mode folds into (see the
/// module doc's degradation contract).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PrDetailOutcome {
    Ready(PrDetail),
    Unavailable,
}

/// A background detail fetch awaiting completion: its [`TaskId`], the
/// generation captured at spawn (a straggler from before a bump is dropped),
/// and the PR number the result belongs to — so the poller writes it under the
/// right key without consulting the (possibly since-closed) overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct InFlightPrDetailFetch {
    pub(super) id: TaskId,
    pub(super) generation: u64,
    pub(super) number: u64,
}

impl App {
    /// `d` on the launcher's Pull Requests tab: opens the description overlay
    /// for the highlighted PR. A PRs-tab-only gesture — a no-op on the other
    /// tabs, on a filter that matches nothing, and on an out-of-range cursor
    /// (an empty, still-loading, or degraded listing), exactly like the tab's
    /// own `Enter`.
    pub(super) fn open_pr_description_from_launcher(&mut self) {
        let Mode::ReviewLauncher {
            tab: LauncherTab::PullRequests,
            cursor,
            origin,
        } = self.mode
        else {
            return;
        };
        let Some(index) = self.review_launcher_real_index(cursor) else {
            return;
        };
        let Some(number) = self.launcher_prs_rows().get(index).map(|pr| pr.number) else {
            return;
        };
        self.open_pr_description(number, PrDescriptionReturn::Launcher { cursor, origin });
    }

    /// `gi` in a review session: opens the description overlay for the PR/MR
    /// under review. A status hint (never an error) in a session with no forge
    /// PR behind it — a local-branch review has no description to read.
    pub(super) fn open_pr_description_in_session(&mut self) {
        let Some(number) = self.review_forge.as_ref().map(|f| f.number) else {
            self.set_status_message("no PR under review \u{2014} nothing to describe");
            return;
        };
        self.open_pr_description(number, PrDescriptionReturn::Session);
    }

    /// The shared open path: fresh scroll state on the named PR, then a
    /// cache-checked fetch. Clears any mid-accumulation motion count so it
    /// can't leak into the overlay's first keystroke (mirrors
    /// [`App::open_review_launcher`]).
    fn open_pr_description(&mut self, number: u64, ret: PrDescriptionReturn) {
        self.pr_description = Some(PrDescriptionState::new(number));
        self.mode = Mode::PrDescription { ret };
        self.motion_count = None;
        self.ensure_pr_detail(number);
    }

    /// Closes the overlay, restoring whatever it was opened over (see
    /// [`PrDescriptionReturn`]). Leaves the cache alone, so reopening the same
    /// PR paints immediately. A no-op outside the mode — defensive rather than
    /// relied upon.
    pub(super) fn close_pr_description(&mut self) {
        let Mode::PrDescription { ret } = self.mode else {
            return;
        };
        self.pr_description = None;
        self.mode = ret.restore();
    }

    /// `Enter` in the overlay: starts the review on the PR being described,
    /// but only when the overlay was opened from the launcher — reopening the
    /// launcher on the same row first, then running its unchanged `Enter`
    /// path, guards included. Inside a review session `Enter` does nothing
    /// (the PR is already open), which is why the session context omits the
    /// hint for it.
    pub(super) fn pr_description_confirm(&mut self) {
        let Mode::PrDescription {
            ret: PrDescriptionReturn::Launcher { cursor, origin },
        } = self.mode
        else {
            return;
        };
        self.pr_description = None;
        self.mode = Mode::ReviewLauncher {
            tab: LauncherTab::PullRequests,
            cursor,
            origin,
        };
        self.review_launcher_confirm();
    }

    /// Scrolls the description down one line. The offset is clamped against
    /// the real content height at render time, so this can't run away.
    pub(super) fn pr_description_scroll_down(&mut self) {
        if let Some(state) = self.pr_description.as_ref() {
            state.scroll.set(state.scroll.get().saturating_add(1));
        }
    }

    /// Scrolls the description up one line.
    pub(super) fn pr_description_scroll_up(&mut self) {
        if let Some(state) = self.pr_description.as_ref() {
            state.scroll.set(state.scroll.get().saturating_sub(1));
        }
    }

    /// The cached outcome for `number`, or `None` while its first fetch is
    /// still in flight (the overlay's "loading…" state — see the module doc's
    /// caching note).
    pub(super) fn pr_detail_for(&self, number: u64) -> Option<&PrDetailOutcome> {
        self.pr_details.get(&number)
    }

    /// Requests `number`'s detail unless it's already cached. Single-flight
    /// with a generation bump, so a straggler from a previous request is
    /// dropped on arrival rather than overwriting a newer one. A backend that
    /// can't produce a `Send` fetcher (test fakes, git-less contexts) caches
    /// `Unavailable` immediately, so the overlay says so instead of waiting on
    /// a load that will never land.
    fn ensure_pr_detail(&mut self, number: u64) {
        if self.pr_details.contains_key(&number) {
            return;
        }
        // A PR session names its own provider; from the launcher, follow
        // whatever a prior PR listing resolved for this host, falling back to
        // GitHub — the same peek-never-re-resolve fallback the checkout and
        // `gx` use, so this read never spawns a credential check of its own.
        let provider = self
            .review_forge
            .as_ref()
            .map(|f| f.provider)
            .or_else(|| {
                self.stage_ops()
                    .and_then(|ops| ops.resolved_pr_provider())
                    .map(super::review_launcher::store_provider)
            })
            .unwrap_or(crate::review::store::ForgeProviderKind::GitHub);
        let fetcher = self
            .stage_ops()
            .and_then(|ops| ops.async_pr_detail_fetcher(provider));
        let Some(fetcher) = fetcher else {
            self.pr_details.insert(number, PrDetailOutcome::Unavailable);
            return;
        };
        self.pr_detail_generation = self.pr_detail_generation.wrapping_add(1);
        let generation = self.pr_detail_generation;
        let id = self.pr_detail_tasks.spawn(move || fetcher(number));
        self.pr_detail_in_flight = Some(InFlightPrDetailFetch {
            id,
            generation,
            number,
        });
    }

    /// Drains a completed background detail fetch (once per event-loop tick,
    /// alongside the other pollers). Drops a foreign result (one whose id
    /// isn't the single in-flight request's) and a stale one (spawned before
    /// `pr_detail_generation` was last bumped); otherwise caches the outcome,
    /// folding a failed read and a panicking task alike into `Unavailable`.
    /// Never touches `mode` — a result landing after the overlay closed
    /// updates the cache and nothing else.
    pub(super) fn poll_pr_detail(&mut self) {
        for (id, result) in self.pr_detail_tasks.poll() {
            let Some(in_flight) = self.pr_detail_in_flight else {
                continue;
            };
            if in_flight.id != id {
                continue;
            }
            self.pr_detail_in_flight = None;
            if in_flight.generation != self.pr_detail_generation {
                continue;
            }
            let outcome = match result {
                Ok(Ok(detail)) => PrDetailOutcome::Ready(detail),
                Ok(Err(_)) | Err(_) => PrDetailOutcome::Unavailable,
            };
            self.pr_details.insert(in_flight.number, outcome);
        }
    }
}

#[cfg(test)]
#[path = "pr_description_tests.rs"]
mod tests;
