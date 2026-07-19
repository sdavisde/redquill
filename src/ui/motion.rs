//! The shared motion layer: the vim-style motion set every buffer-like or
//! list-like context in the app supports (step down/up, half-page,
//! full-page, jump-to-top, jump-to-bottom), defined once as data, plus the
//! pure count-prefix arithmetic (`3j`, `2Ctrl-d`, ...) every context's digit
//! interception shares. Everything here is pure — no `App`/`Mode`/render
//! types — so it's unit-testable without constructing the app.
//!
//! **Why a trait *and* free functions**: [`Motionable`] is the shared
//! contract for a context where one type owns one coherent motion target —
//! the diff view (`DiffViewState`) is the only such case today. Every other
//! consuming context (the git panel, the annotation list, the staging panel,
//! the switcher, LSP peek) is a *mode* of one `App`, and `App` can't
//! implement the same trait once per mode — so those contexts share the
//! motion set through the free functions below ([`step`], [`half_page`],
//! [`full_page`], [`jump_top`], [`jump_bottom`]) instead, called directly
//! from their own thin methods. Either way, the actual arithmetic is defined
//! exactly once.
//!
//! (This module lands ahead of its callers, which arrive over the next few
//! commits as each consuming context is migrated — `allow(dead_code)` is
//! scoped to that staging window and comes off once everything here is
//! consumed.)
//!
//! **Jump-to-top in non-diff contexts**: the diff view's `gg` is a two-key
//! sequence, resolved by [`super::keymap::Keymap`]'s pending-prefix machine.
//! The git panel and the modal-list tables ([`super::modal_keys`]) have no
//! two-key-sequence support (a real, pre-existing limitation — modal tables
//! only ever match one physical key per row), so every non-diff context
//! binds jump-to-top to a single `g` (plus `Home`) instead, mirroring the
//! precedent the help overlay already established for its own scrolling
//! (see `modal_keys::HELP_KEYS`). `G`/`End` remains jump-to-bottom
//! everywhere. This is a deliberate, documented divergence from the diff
//! view's literal `gg`, not an oversight.

#![allow(dead_code)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// The largest numeric prefix any context's digit interception will
/// accumulate — a mistyped run of digits shouldn't be able to turn a single
/// keypress into a render-loop hitch on a large diff or list.
pub const MAX_COUNT: usize = 1000;

/// The shared motion set (FR-1): every context that consumes this layer
/// supports all eight, each with an optional count prefix. `StepDown`/`Up`
/// are the plain `j`/`k` line motions; the rest mirror the diff view's
/// existing half/full-page and buffer-extreme jumps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Motion {
    StepDown,
    StepUp,
    HalfPageDown,
    HalfPageUp,
    FullPageDown,
    FullPageUp,
    JumpToTop,
    JumpToBottom,
}

impl Motion {
    /// Every motion, in a stable order — the coverage drift test's
    /// enumeration source, so a ninth motion added here is automatically
    /// included in every context's coverage check.
    pub const ALL: [Motion; 8] = [
        Motion::StepDown,
        Motion::StepUp,
        Motion::HalfPageDown,
        Motion::HalfPageUp,
        Motion::FullPageDown,
        Motion::FullPageUp,
        Motion::JumpToTop,
        Motion::JumpToBottom,
    ];

    /// Whether a count prefix repeats this motion. Jumps have no natural
    /// "repeat" meaning (this layer does not reinterpret `3gg` as "go to
    /// line 3") so they always apply once; every other motion repeats.
    pub fn is_repeatable(self) -> bool {
        !matches!(self, Motion::JumpToTop | Motion::JumpToBottom)
    }

    /// How many times to apply this motion for an accumulated count prefix
    /// (`None` if the user typed no digits before the motion key).
    pub fn repeat_count(self, count: Option<usize>) -> usize {
        if self.is_repeatable() {
            clamp_count(count)
        } else {
            1
        }
    }
}

/// The shared count-repeat clamp: an absent count applies once, otherwise
/// the accumulated digits, capped at [`MAX_COUNT`].
pub fn clamp_count(count: Option<usize>) -> usize {
    count.unwrap_or(1).clamp(1, MAX_COUNT)
}

/// The outcome of feeding one character into a context's in-progress count
/// prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigitOutcome {
    /// `c` was a digit that extends the count; the new accumulated value.
    Consumed(usize),
    /// `c` was a bare leading `0` with no count already pending — vim
    /// convention treats this as its own gesture (e.g. `CursorLineStart`),
    /// not the start of a count, so the caller should let it fall through
    /// unconsumed.
    LeadingZero,
    /// `c` isn't a digit at all; the caller should resolve it normally.
    NotADigit,
}

/// Folds one character into an in-progress count prefix (`pending`),
/// matching vim's accumulation rule: `1`-`9` always extend the count
/// (`saturating` arithmetic, capped at [`MAX_COUNT`]); a `0` extends it too,
/// but only once a count has already started (`10`, `20`, ...) — a bare `0`
/// with nothing pending is its own motion (line-start), not a count digit.
pub fn accumulate_digit(pending: Option<usize>, c: char) -> DigitOutcome {
    match c {
        '1'..='9' => {
            let digit = c.to_digit(10).unwrap_or(0) as usize;
            DigitOutcome::Consumed(
                pending
                    .unwrap_or(0)
                    .saturating_mul(10)
                    .saturating_add(digit)
                    .min(MAX_COUNT),
            )
        }
        '0' if pending.is_some() => {
            DigitOutcome::Consumed(pending.unwrap_or(0).saturating_mul(10).min(MAX_COUNT))
        }
        '0' => DigitOutcome::LeadingZero,
        _ => DigitOutcome::NotADigit,
    }
}

/// Whether an in-progress count should survive a key that just *started* a
/// two-key sequence (`3` then the `g` of `3gg`) rather than being abandoned.
/// Pure restatement of the rule `dispatch_key` applies: a count only
/// survives when nothing was pending before this key and something is
/// pending after it — any other outcome (a completed sequence, an unbound
/// key, a cancelled sequence) abandons the count, matching vim.
pub fn count_survives_sequence_start(had_pending_prefix: bool, now_pending_prefix: bool) -> bool {
    !had_pending_prefix && now_pending_prefix
}

/// The shared contract for a context where one type owns one coherent
/// motion target (see the module doc for why this isn't used everywhere).
/// Implementors carry their own clamping/scrolling semantics; only the
/// *set* of motions and the repeat-count convention ([`Motion::repeat_count`])
/// are shared.
pub trait Motionable {
    fn step_down(&mut self);
    fn step_up(&mut self);
    fn half_page_down(&mut self);
    fn half_page_up(&mut self);
    fn full_page_down(&mut self);
    fn full_page_up(&mut self);
    fn jump_to_top(&mut self);
    fn jump_to_bottom(&mut self);
}

/// Applies `motion` to `target`, repeating it [`Motion::repeat_count`] times
/// for `count`. The single call every [`Motionable`]-based context's
/// dispatch routes a resolved motion through.
pub fn dispatch<T: Motionable>(target: &mut T, motion: Motion, count: Option<usize>) {
    for _ in 0..motion.repeat_count(count) {
        match motion {
            Motion::StepDown => target.step_down(),
            Motion::StepUp => target.step_up(),
            Motion::HalfPageDown => target.half_page_down(),
            Motion::HalfPageUp => target.half_page_up(),
            Motion::FullPageDown => target.full_page_down(),
            Motion::FullPageUp => target.full_page_up(),
            Motion::JumpToTop => target.jump_to_top(),
            Motion::JumpToBottom => target.jump_to_bottom(),
        }
    }
}

// -- Linear-cursor helpers, for contexts that share the arithmetic without
// -- implementing `Motionable` (see the module doc) ------------------------

/// Steps a 0-based cursor by `delta` rows within a `len`-row list, clamping
/// at both ends. An empty list pins the cursor at 0. The shared core of
/// every plain (non-addressable-row-skipping, non-scrolloff) list's
/// step/half-page/full-page motions.
pub fn step(cursor: usize, len: usize, delta: usize, down: bool) -> usize {
    if len == 0 {
        return 0;
    }
    let max = len - 1;
    if down {
        cursor.saturating_add(delta).min(max)
    } else {
        cursor.saturating_sub(delta)
    }
}

/// The first row of a `len`-row list (jump-to-top's target).
pub fn jump_top() -> usize {
    0
}

/// The last row of a `len`-row list (jump-to-bottom's target); `0` when
/// empty.
pub fn jump_bottom(len: usize) -> usize {
    len.saturating_sub(1)
}

/// The half-page step size for a `viewport_height`-row view: at least 1, so
/// a not-yet-measured (zero) viewport still moves.
pub fn half_page(viewport_height: usize) -> usize {
    (viewport_height / 2).max(1)
}

/// The full-page step size for a `viewport_height`-row view: at least 1.
pub fn full_page(viewport_height: usize) -> usize {
    viewport_height.max(1)
}

// -- Non-diff jump-to-top: a single `g`/`Home` key, not the diff view's
// -- two-key `gg` (see the module doc) --------------------------------------

/// Whether `key` is the bare, unmodified `g` or `Home` every non-diff
/// context binds to jump-to-top (see the module doc).
pub fn is_top_jump_key(key: KeyEvent) -> bool {
    key.modifiers == KeyModifiers::NONE && matches!(key.code, KeyCode::Char('g') | KeyCode::Home)
}

// -- Coverage checking -------------------------------------------------------

/// Whether every motion in `required` is resolvable in some context, per
/// `resolves`. Generic over both the motion list and the resolver so a test
/// can prove the check has teeth (see the coverage-drift test module): a
/// hardcoded check that never varies its input could never fail.
pub fn covers_all<M: Copy>(required: &[M], resolves: impl Fn(M) -> bool) -> bool {
    required.iter().all(|&m| resolves(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Digit accumulation ---------------------------------------------------

    #[test]
    fn single_digits_one_through_nine_accumulate_from_none() {
        for c in '1'..='9' {
            let expected = c.to_digit(10).unwrap() as usize;
            assert_eq!(accumulate_digit(None, c), DigitOutcome::Consumed(expected));
        }
    }

    #[test]
    fn digits_extend_an_in_progress_count() {
        assert_eq!(accumulate_digit(Some(1), '0'), DigitOutcome::Consumed(10));
        assert_eq!(accumulate_digit(Some(10), '5'), DigitOutcome::Consumed(105));
    }

    #[test]
    fn bare_leading_zero_is_not_a_count_digit() {
        assert_eq!(accumulate_digit(None, '0'), DigitOutcome::LeadingZero);
    }

    #[test]
    fn non_digit_chars_are_not_count_digits() {
        assert_eq!(accumulate_digit(None, 'g'), DigitOutcome::NotADigit);
        assert_eq!(accumulate_digit(Some(3), 'j'), DigitOutcome::NotADigit);
    }

    #[test]
    fn accumulation_clamps_at_max_count() {
        // 999 -> add digit 9 -> would be 9999, clamped to MAX_COUNT.
        assert_eq!(
            accumulate_digit(Some(999), '9'),
            DigitOutcome::Consumed(MAX_COUNT)
        );
        // A run of nines from empty also clamps rather than overflowing.
        let mut pending = None;
        for c in "99999999".chars() {
            match accumulate_digit(pending, c) {
                DigitOutcome::Consumed(n) => pending = Some(n),
                other => panic!("expected a digit to accumulate, got {other:?}"),
            }
        }
        assert_eq!(pending, Some(MAX_COUNT));
    }

    // -- Motion::repeat_count / is_repeatable ---------------------------------

    #[test]
    fn step_and_page_motions_repeat_by_count() {
        for m in [
            Motion::StepDown,
            Motion::StepUp,
            Motion::HalfPageDown,
            Motion::HalfPageUp,
            Motion::FullPageDown,
            Motion::FullPageUp,
        ] {
            assert!(m.is_repeatable(), "{m:?} must be repeatable");
            assert_eq!(m.repeat_count(Some(5)), 5);
            assert_eq!(m.repeat_count(None), 1);
        }
    }

    #[test]
    fn jump_motions_always_apply_once() {
        for m in [Motion::JumpToTop, Motion::JumpToBottom] {
            assert!(!m.is_repeatable(), "{m:?} must not be repeatable");
            assert_eq!(m.repeat_count(Some(5)), 1);
            assert_eq!(m.repeat_count(None), 1);
        }
    }

    #[test]
    fn repeat_count_clamps_at_max_count() {
        assert_eq!(Motion::StepDown.repeat_count(Some(50_000)), MAX_COUNT);
    }

    #[test]
    fn motion_all_lists_every_variant_exactly_once() {
        let mut seen = std::collections::HashSet::new();
        for m in Motion::ALL {
            assert!(seen.insert(m), "{m:?} listed more than once in Motion::ALL");
        }
        assert_eq!(Motion::ALL.len(), 8);
    }

    // -- count_survives_sequence_start ----------------------------------------

    #[test]
    fn count_survives_only_when_a_sequence_just_started() {
        assert!(count_survives_sequence_start(false, true));
        assert!(!count_survives_sequence_start(false, false));
        assert!(!count_survives_sequence_start(true, false));
        assert!(!count_survives_sequence_start(true, true));
    }

    // -- Linear-cursor helpers -------------------------------------------------

    #[test]
    fn step_clamps_at_both_ends_of_a_list() {
        assert_eq!(step(0, 5, 1, true), 1);
        assert_eq!(step(4, 5, 1, true), 4, "clamps at the last row");
        assert_eq!(step(0, 5, 1, false), 0, "clamps at the first row");
        assert_eq!(step(3, 5, 10, true), 4, "a large delta still clamps");
        assert_eq!(step(3, 5, 10, false), 0);
    }

    #[test]
    fn step_on_an_empty_list_stays_at_zero() {
        assert_eq!(step(0, 0, 1, true), 0);
        assert_eq!(step(0, 0, 1, false), 0);
    }

    #[test]
    fn jump_top_and_bottom_target_the_list_extremes() {
        assert_eq!(jump_top(), 0);
        assert_eq!(jump_bottom(10), 9);
        assert_eq!(jump_bottom(0), 0, "an empty list's bottom is row 0");
    }

    #[test]
    fn half_and_full_page_sizes_are_at_least_one() {
        assert_eq!(half_page(0), 1);
        assert_eq!(half_page(1), 1);
        assert_eq!(half_page(20), 10);
        assert_eq!(full_page(0), 1);
        assert_eq!(full_page(20), 20);
    }

    // -- is_top_jump_key ---------------------------------------------------

    #[test]
    fn top_jump_key_accepts_bare_g_and_home_only() {
        assert!(is_top_jump_key(KeyEvent::new(
            KeyCode::Char('g'),
            KeyModifiers::NONE
        )));
        assert!(is_top_jump_key(KeyEvent::new(
            KeyCode::Home,
            KeyModifiers::NONE
        )));
        assert!(!is_top_jump_key(KeyEvent::new(
            KeyCode::Char('g'),
            KeyModifiers::CONTROL
        )));
        assert!(!is_top_jump_key(KeyEvent::new(
            KeyCode::Char('G'),
            KeyModifiers::NONE
        )));
    }

    // -- Motionable / dispatch --------------------------------------------------

    /// A minimal `Motionable` over a plain linear list, for exercising
    /// `dispatch` without pulling in `DiffViewState`.
    struct FakeList {
        cursor: usize,
        len: usize,
        viewport: usize,
    }

    impl Motionable for FakeList {
        fn step_down(&mut self) {
            self.cursor = step(self.cursor, self.len, 1, true);
        }
        fn step_up(&mut self) {
            self.cursor = step(self.cursor, self.len, 1, false);
        }
        fn half_page_down(&mut self) {
            self.cursor = step(self.cursor, self.len, half_page(self.viewport), true);
        }
        fn half_page_up(&mut self) {
            self.cursor = step(self.cursor, self.len, half_page(self.viewport), false);
        }
        fn full_page_down(&mut self) {
            self.cursor = step(self.cursor, self.len, full_page(self.viewport), true);
        }
        fn full_page_up(&mut self) {
            self.cursor = step(self.cursor, self.len, full_page(self.viewport), false);
        }
        fn jump_to_top(&mut self) {
            self.cursor = jump_top();
        }
        fn jump_to_bottom(&mut self) {
            self.cursor = jump_bottom(self.len);
        }
    }

    #[test]
    fn dispatch_applies_a_repeatable_motion_count_times() {
        let mut list = FakeList {
            cursor: 0,
            len: 100,
            viewport: 10,
        };
        dispatch(&mut list, Motion::StepDown, Some(5));
        assert_eq!(list.cursor, 5);
    }

    #[test]
    fn dispatch_applies_a_jump_once_regardless_of_count() {
        let mut list = FakeList {
            cursor: 0,
            len: 100,
            viewport: 10,
        };
        dispatch(&mut list, Motion::JumpToBottom, Some(5));
        assert_eq!(list.cursor, 99);
    }

    #[test]
    fn dispatch_covers_half_and_full_page_in_both_directions() {
        let mut list = FakeList {
            cursor: 50,
            len: 100,
            viewport: 10,
        };
        dispatch(&mut list, Motion::HalfPageDown, None);
        assert_eq!(list.cursor, 55);
        dispatch(&mut list, Motion::HalfPageUp, None);
        assert_eq!(list.cursor, 50);
        dispatch(&mut list, Motion::FullPageDown, None);
        assert_eq!(list.cursor, 60);
        dispatch(&mut list, Motion::FullPageUp, None);
        assert_eq!(list.cursor, 50);
    }

    // -- covers_all --------------------------------------------------------

    #[test]
    fn covers_all_is_true_when_every_motion_resolves() {
        assert!(covers_all(&Motion::ALL, |_| true));
    }

    #[test]
    fn covers_all_is_false_when_any_motion_is_missing() {
        assert!(!covers_all(&Motion::ALL, |m| m != Motion::JumpToBottom));
    }

    /// Proves `covers_all` isn't a tautology: it's generic over the required
    /// list, so a caller can hand it a list containing a motion no real
    /// context supports (`resolves` here is a stand-in for "every currently
    /// wired-up context's actual resolver", none of which can ever say yes
    /// to a value outside the real `Motion` enum) and the check correctly
    /// reports incomplete coverage rather than vacuously passing.
    #[test]
    fn covers_all_rejects_a_requirement_no_resolver_can_satisfy() {
        #[derive(Clone, Copy)]
        enum WithExtra {
            Real(Motion),
            NeverSupported,
        }
        let required: Vec<WithExtra> = Motion::ALL
            .into_iter()
            .map(WithExtra::Real)
            .chain(std::iter::once(WithExtra::NeverSupported))
            .collect();
        let resolves = |m: WithExtra| matches!(m, WithExtra::Real(_));
        assert!(
            !covers_all(&required, resolves),
            "a required motion with no resolver must fail coverage"
        );
    }
}
