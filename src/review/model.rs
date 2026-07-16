//! The pure per-file review-status state machine (spec 08 Unit 3). No TUI
//! types and no I/O — every transition is a plain function over
//! [`ReviewStatus`], so the full transition table is unit-tested here
//! without constructing an `App` or a git backend. The presentation-side
//! wiring (`src/ui/review_ops.rs`) maps `Space`/`S`/`d` onto these functions
//! against an `App`-owned per-path map, mirroring how `staged_states` drives
//! the staging markers.

/// A file's review status within a branch-review session. `Unreviewed` is
/// the default: the presentation layer's per-path map stores no entry at
/// all for it, exactly mirroring how a missing `staged_states` entry means
/// "unstaged" in the pre-existing staging model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReviewStatus {
    /// Not yet reviewed — the default, and the map's "missing entry" state.
    #[default]
    Unreviewed,
    /// Set aside to revisit later (`d`). Collapses the file's section.
    Deferred,
    /// Reviewed and accepted (`Space`/`S`). Collapses the file's section.
    Accepted,
    /// Was `Accepted` in a previous review session, but the branch's blob
    /// SHA for this file has since changed (spec 08 Unit 4 reconciliation).
    /// Rendered un-collapsed with its own marker; a `Space` press re-accepts
    /// it at the fresh SHA (see [`accept`]).
    ChangedSinceAccepted,
}

/// The `Space` gesture: toggles the file between `Accepted` and everything
/// else. `Deferred` and `ChangedSinceAccepted` both accept on the first
/// press (neither round-trips through `Unreviewed` first); an already-
/// `Accepted` file un-accepts straight back to `Unreviewed`.
pub fn toggle_accept(status: ReviewStatus) -> ReviewStatus {
    match status {
        ReviewStatus::Accepted => ReviewStatus::Unreviewed,
        ReviewStatus::Unreviewed | ReviewStatus::Deferred | ReviewStatus::ChangedSinceAccepted => {
            ReviewStatus::Accepted
        }
    }
}

/// The `S` gesture: accepts unconditionally, regardless of the current
/// status — mirrors `StageFile`'s "works from anywhere in the file, not
/// just its header row" gesture, always landing on `Accepted`. This is also
/// the transition a re-accept applies to a `ChangedSinceAccepted` file (at
/// its fresh blob SHA, once spec 08 Unit 4 wires persistence).
pub fn accept(_status: ReviewStatus) -> ReviewStatus {
    ReviewStatus::Accepted
}

/// The `d` gesture: toggles the file between `Deferred` and everything
/// else, exactly mirroring [`toggle_accept`]'s shape for the opposite
/// status.
pub fn toggle_defer(status: ReviewStatus) -> ReviewStatus {
    match status {
        ReviewStatus::Deferred => ReviewStatus::Unreviewed,
        ReviewStatus::Unreviewed | ReviewStatus::Accepted | ReviewStatus::ChangedSinceAccepted => {
            ReviewStatus::Deferred
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_status_is_unreviewed() {
        assert_eq!(ReviewStatus::default(), ReviewStatus::Unreviewed);
    }

    // -- toggle_accept: exhaustive transition table --------------------------

    #[test]
    fn toggle_accept_from_unreviewed_accepts() {
        assert_eq!(
            toggle_accept(ReviewStatus::Unreviewed),
            ReviewStatus::Accepted
        );
    }

    #[test]
    fn toggle_accept_from_accepted_un_accepts() {
        assert_eq!(
            toggle_accept(ReviewStatus::Accepted),
            ReviewStatus::Unreviewed
        );
    }

    #[test]
    fn toggle_accept_from_deferred_accepts() {
        assert_eq!(
            toggle_accept(ReviewStatus::Deferred),
            ReviewStatus::Accepted
        );
    }

    #[test]
    fn toggle_accept_from_changed_since_accepted_re_accepts() {
        assert_eq!(
            toggle_accept(ReviewStatus::ChangedSinceAccepted),
            ReviewStatus::Accepted
        );
    }

    #[test]
    fn toggle_accept_is_a_true_toggle_round_trip() {
        let accepted = toggle_accept(ReviewStatus::Unreviewed);
        assert_eq!(accepted, ReviewStatus::Accepted);
        assert_eq!(toggle_accept(accepted), ReviewStatus::Unreviewed);
    }

    // -- accept: unconditional -----------------------------------------------

    #[test]
    fn accept_is_unconditional_from_every_status() {
        for status in [
            ReviewStatus::Unreviewed,
            ReviewStatus::Deferred,
            ReviewStatus::Accepted,
            ReviewStatus::ChangedSinceAccepted,
        ] {
            assert_eq!(accept(status), ReviewStatus::Accepted);
        }
    }

    // -- toggle_defer: exhaustive transition table ----------------------------

    #[test]
    fn toggle_defer_from_unreviewed_defers() {
        assert_eq!(
            toggle_defer(ReviewStatus::Unreviewed),
            ReviewStatus::Deferred
        );
    }

    #[test]
    fn toggle_defer_from_deferred_un_defers() {
        assert_eq!(
            toggle_defer(ReviewStatus::Deferred),
            ReviewStatus::Unreviewed
        );
    }

    #[test]
    fn toggle_defer_from_accepted_defers() {
        assert_eq!(toggle_defer(ReviewStatus::Accepted), ReviewStatus::Deferred);
    }

    #[test]
    fn toggle_defer_from_changed_since_accepted_defers() {
        assert_eq!(
            toggle_defer(ReviewStatus::ChangedSinceAccepted),
            ReviewStatus::Deferred
        );
    }

    #[test]
    fn toggle_defer_is_a_true_toggle_round_trip() {
        let deferred = toggle_defer(ReviewStatus::Unreviewed);
        assert_eq!(deferred, ReviewStatus::Deferred);
        assert_eq!(toggle_defer(deferred), ReviewStatus::Unreviewed);
    }

    // -- accept/defer are mutually exclusive ---------------------------------

    #[test]
    fn accepting_a_deferred_file_then_deferring_it_again_round_trips() {
        let deferred = toggle_defer(ReviewStatus::Unreviewed);
        let accepted = toggle_accept(deferred);
        assert_eq!(accepted, ReviewStatus::Accepted);
        let deferred_again = toggle_defer(accepted);
        assert_eq!(deferred_again, ReviewStatus::Deferred);
    }
}
