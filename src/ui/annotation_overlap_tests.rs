use super::*;
use crate::annotate::{Annotation, Classification, Side, Source, Target};

/// Builds an annotation with an explicit store id and target — the two fields
/// the overlap rule reads; body/classification/source are filler.
fn ann(id: usize, target: Target) -> Annotation {
    Annotation {
        id,
        target,
        classification: Classification::Nit,
        body: "note".to_string(),
        source: Source::default(),
        published: false,
        draft_created: false,
    }
}

fn anchor_of(target: Target) -> CursorAnchor {
    CursorAnchor::from_target(&target)
}

// -- CursorAnchor::from_target -------------------------------------------------

#[test]
fn file_target_maps_to_the_file_header_anchor() {
    assert_eq!(
        CursorAnchor::from_target(&Target::file("a.rs")),
        CursorAnchor::FileHeader
    );
}

#[test]
fn hunk_target_maps_to_the_hunk_header_anchor() {
    assert_eq!(
        CursorAnchor::from_target(&Target::hunk("a.rs", 4, 9).unwrap()),
        CursorAnchor::HunkHeader { start: 4, end: 9 }
    );
}

#[test]
fn line_and_worktree_targets_map_to_line_anchors() {
    assert!(matches!(
        CursorAnchor::from_target(&Target::line("a.rs", 7, Side::New)),
        CursorAnchor::Line { line: 7, .. }
    ));
    assert!(matches!(
        CursorAnchor::from_target(&Target::worktree_line("a.rs", 7)),
        CursorAnchor::Line { line: 7, .. }
    ));
    // A range-shaped target reduces to a point at its start.
    assert!(matches!(
        CursorAnchor::from_target(&Target::range("a.rs", 5, 9, Side::Old).unwrap()),
        CursorAnchor::Line { line: 5, .. }
    ));
}

// -- Line targets --------------------------------------------------------------

#[test]
fn line_target_matches_the_same_line_and_side() {
    let anns = [ann(0, Target::line("a.rs", 12, Side::New))];
    let anchor = anchor_of(Target::line("a.rs", 12, Side::New));
    assert_eq!(overlapping_annotation(&anchor, &anns), Some(0));
}

#[test]
fn line_target_misses_a_different_line() {
    let anns = [ann(0, Target::line("a.rs", 12, Side::New))];
    let anchor = anchor_of(Target::line("a.rs", 13, Side::New));
    assert_eq!(overlapping_annotation(&anchor, &anns), None);
}

#[test]
fn line_target_misses_the_same_line_on_the_other_side() {
    let anns = [ann(0, Target::line("a.rs", 12, Side::Old))];
    let anchor = anchor_of(Target::line("a.rs", 12, Side::New));
    assert_eq!(overlapping_annotation(&anchor, &anns), None);
}

// -- Range targets -------------------------------------------------------------

#[test]
fn range_target_covers_every_line_in_its_span() {
    let anns = [ann(0, Target::range("a.rs", 10, 14, Side::New).unwrap())];
    for line in 10..=14 {
        let anchor = anchor_of(Target::line("a.rs", line, Side::New));
        assert_eq!(
            overlapping_annotation(&anchor, &anns),
            Some(0),
            "line {line} should be covered"
        );
    }
    let outside = anchor_of(Target::line("a.rs", 15, Side::New));
    assert_eq!(overlapping_annotation(&outside, &anns), None);
}

#[test]
fn range_target_respects_side() {
    let anns = [ann(0, Target::range("a.rs", 10, 14, Side::Old).unwrap())];
    let anchor = anchor_of(Target::line("a.rs", 12, Side::New));
    assert_eq!(overlapping_annotation(&anchor, &anns), None);
}

// -- Hunk targets --------------------------------------------------------------

#[test]
fn hunk_target_matches_the_hunk_header_of_its_span() {
    let anns = [ann(0, Target::hunk("a.rs", 4, 9).unwrap())];
    let anchor = anchor_of(Target::hunk("a.rs", 4, 9).unwrap());
    assert_eq!(overlapping_annotation(&anchor, &anns), Some(0));
    // A different hunk header (different span) does not match.
    let other = anchor_of(Target::hunk("a.rs", 20, 25).unwrap());
    assert_eq!(overlapping_annotation(&other, &anns), None);
}

#[test]
fn hunk_target_covers_new_side_lines_within_its_span() {
    let anns = [ann(0, Target::hunk("a.rs", 4, 9).unwrap())];
    let inside = anchor_of(Target::line("a.rs", 6, Side::New));
    assert_eq!(overlapping_annotation(&inside, &anns), Some(0));
    // Old-side lines are numbered independently, so a hunk (new-side) does
    // not cover an old-side cursor point.
    let old_side = anchor_of(Target::line("a.rs", 6, Side::Old));
    assert_eq!(overlapping_annotation(&old_side, &anns), None);
}

// -- File targets --------------------------------------------------------------

#[test]
fn file_target_matches_only_the_file_header_row() {
    let anns = [ann(0, Target::file("a.rs"))];
    let header = anchor_of(Target::file("a.rs"));
    assert_eq!(overlapping_annotation(&header, &anns), Some(0));
    // Not from a content line.
    let line = anchor_of(Target::line("a.rs", 3, Side::New));
    assert_eq!(overlapping_annotation(&line, &anns), None);
}

#[test]
fn file_header_row_ignores_line_annotations() {
    let anns = [ann(0, Target::line("a.rs", 3, Side::New))];
    let header = anchor_of(Target::file("a.rs"));
    assert_eq!(overlapping_annotation(&header, &anns), None);
}

// -- Worktree variants ---------------------------------------------------------

#[test]
fn worktree_line_target_matches_a_worktree_cursor_line() {
    let anns = [ann(0, Target::worktree_line("a.rs", 8))];
    let anchor = anchor_of(Target::worktree_line("a.rs", 8));
    assert_eq!(overlapping_annotation(&anchor, &anns), Some(0));
}

#[test]
fn worktree_range_target_covers_its_span() {
    let anns = [ann(0, Target::worktree_range("a.rs", 8, 12).unwrap())];
    let inside = anchor_of(Target::worktree_line("a.rs", 10));
    assert_eq!(overlapping_annotation(&inside, &anns), Some(0));
    let outside = anchor_of(Target::worktree_line("a.rs", 13));
    assert_eq!(overlapping_annotation(&outside, &anns), None);
}

#[test]
fn diff_line_does_not_match_a_worktree_target() {
    let anns = [ann(0, Target::worktree_line("a.rs", 8))];
    let anchor = anchor_of(Target::line("a.rs", 8, Side::New));
    assert_eq!(overlapping_annotation(&anchor, &anns), None);
}

// -- Multi-overlap: nearest start above-or-at wins -----------------------------

#[test]
fn nested_ranges_the_innermost_nearest_start_wins() {
    // Two ranges both cover line 12; the inner one starts nearer the cursor.
    let anns = [
        ann(0, Target::range("a.rs", 8, 14, Side::New).unwrap()),
        ann(1, Target::range("a.rs", 11, 13, Side::New).unwrap()),
    ];
    let anchor = anchor_of(Target::line("a.rs", 12, Side::New));
    assert_eq!(overlapping_annotation(&anchor, &anns), Some(1));
}

#[test]
fn a_line_target_beats_an_enclosing_range_by_nearest_start() {
    // A range [8,14] and an exact line at 12 both cover line 12; the line
    // starts at 12 (nearer) so it wins over the range starting at 8.
    let anns = [
        ann(0, Target::range("a.rs", 8, 14, Side::New).unwrap()),
        ann(1, Target::line("a.rs", 12, Side::New)),
    ];
    let anchor = anchor_of(Target::line("a.rs", 12, Side::New));
    assert_eq!(overlapping_annotation(&anchor, &anns), Some(1));
}

#[test]
fn insertion_order_does_not_change_the_nearest_start_winner() {
    // Same as above with the store order reversed: the rule keys off start and
    // id, not iteration order.
    let anns = [
        ann(1, Target::line("a.rs", 12, Side::New)),
        ann(0, Target::range("a.rs", 8, 14, Side::New).unwrap()),
    ];
    let anchor = anchor_of(Target::line("a.rs", 12, Side::New));
    assert_eq!(overlapping_annotation(&anchor, &anns), Some(1));
}

// -- Ties: same start, oldest (lowest id) wins ---------------------------------

#[test]
fn ties_on_start_break_to_the_oldest_annotation() {
    // Two line targets on the same line; the older (lower id) wins even when
    // it appears later in the slice.
    let anns = [
        ann(5, Target::line("a.rs", 12, Side::New)),
        ann(2, Target::line("a.rs", 12, Side::New)),
    ];
    let anchor = anchor_of(Target::line("a.rs", 12, Side::New));
    assert_eq!(overlapping_annotation(&anchor, &anns), Some(2));
}

#[test]
fn same_start_ranges_break_to_the_oldest() {
    let anns = [
        ann(3, Target::range("a.rs", 10, 20, Side::New).unwrap()),
        ann(1, Target::range("a.rs", 10, 12, Side::New).unwrap()),
    ];
    // Cursor at 11 is inside both; both start at 10, so the tie-break (oldest)
    // decides — id 1.
    let anchor = anchor_of(Target::line("a.rs", 11, Side::New));
    assert_eq!(overlapping_annotation(&anchor, &anns), Some(1));
}

// -- Empty / no coverage -------------------------------------------------------

#[test]
fn no_annotations_yields_none() {
    let anns: [Annotation; 0] = [];
    let anchor = anchor_of(Target::line("a.rs", 12, Side::New));
    assert_eq!(overlapping_annotation(&anchor, &anns), None);
}
