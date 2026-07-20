//! Pure annotation-target derivation: mapping a cursor row or a Visual
//! selection over the flattened row model to the [`Target`] it addresses.
//! Kept free of [`super::App`] so the file/hunk/line/range mapping is
//! unit-testable in isolation.
//!
//! These functions consume UI row types ([`Row`]/[`LineRow`] from
//! [`super::rows`]), which is why they live in the `ui` layer rather than in
//! `diff`/`annotate`: pushing them lower would leak the row model downward
//! across the layer boundary. [`super::App`] keeps thin wrappers that gather
//! the selected file, cursor, and Visual anchor and delegate here (see
//! `App::target_for_cursor` / `App::target_for_visual`).

use crate::annotate::{Side, Target};
use crate::diff::{FileDiff, LineOrigin};

use super::rows::{LineRow, Row, hunk_span};

/// The annotation target for the cursor's current row in [`super::Mode::Normal`]:
/// a `Line` target for a diff line (side/number from the line's origin), a
/// `Hunk` target for a hunk header, or a `File` target for the file
/// header/binary placeholder. `None` on rows that carry no derivable target
/// (currently [`Row::Annotation`] and [`Row::AnnotationBorder`], which the
/// cursor never addresses) or when `cursor` is out of bounds.
pub(super) fn target_for_cursor(file: &FileDiff, rows: &[Row], cursor: usize) -> Option<Target> {
    match rows.get(cursor)? {
        Row::Line(line) => line_target(&file.path, line),
        Row::HunkHeader { hunk_index, .. } => hunk_target(file, *hunk_index),
        Row::FileHeader { .. } | Row::Binary => Some(Target::file(&file.path)),
        Row::Annotation { .. }
        | Row::AnnotationBorder { .. }
        | Row::Thread(_)
        | Row::ThreadBorder { .. } => None,
    }
}

/// The `Hunk` target for the `hunk_index`-th hunk of `file`, spanning the
/// same range [`Row::HunkHeader`] anchors on. When the span's end line is a
/// context line, its old-side number is recorded as the counterpart —
/// forge submission anchors the hunk on that end line, and a context anchor
/// needs both sides. `None` if the index is out of range or the span is not
/// a valid target.
fn hunk_target(file: &FileDiff, hunk_index: usize) -> Option<Target> {
    let hunk = file.hunks.get(hunk_index)?;
    let (start, end) = hunk_span(hunk);
    let other_end = hunk
        .lines
        .iter()
        .find(|l| l.origin == LineOrigin::Context && l.new_line == Some(end))
        .and_then(|l| l.old_line);
    Target::hunk_with_other_end(&file.path, start, end, other_end).ok()
}

/// The annotation target for a [`super::Mode::Visual`] selection between
/// `anchor` and `cursor` (inclusive, order-independent) over `rows`. Only
/// [`Row::Line`] rows in the span count; selections spanning hunk/file
/// headers clamp to the line rows within them. If every selected line is
/// `Removed`, the target uses the old side and old-side line numbers;
/// otherwise it uses the new side and the new-side line numbers of the
/// non-removed rows the selection spans. `None` if the selection covers no
/// line rows at all, or an endpoint is out of bounds.
pub(super) fn target_for_visual(
    file: &FileDiff,
    rows: &[Row],
    cursor: usize,
    anchor: usize,
) -> Option<Target> {
    let (lo, hi) = if anchor <= cursor {
        (anchor, cursor)
    } else {
        (cursor, anchor)
    };
    let span = rows.get(lo..=hi)?;
    let lines: Vec<&LineRow> = span
        .iter()
        .filter_map(|r| match r {
            Row::Line(l) => Some(l),
            _ => None,
        })
        .collect();
    if lines.is_empty() {
        return None;
    }

    if lines.iter().all(|l| l.origin == LineOrigin::Removed) {
        let nums: Vec<u32> = lines.iter().filter_map(|l| l.old_line).collect();
        let start = *nums.iter().min()?;
        let end = *nums.iter().max()?;
        Target::range(&file.path, start, end, Side::Old).ok()
    } else {
        let nums: Vec<u32> = lines
            .iter()
            .filter(|l| l.origin != LineOrigin::Removed)
            .filter_map(|l| l.new_line)
            .collect();
        let start = *nums.iter().min()?;
        let end = *nums.iter().max()?;
        // Forge submission anchors the span on its end line; when that line
        // is context, record its old-side number so a both-sided position
        // can be built.
        let other_end = lines
            .iter()
            .find(|l| l.origin == LineOrigin::Context && l.new_line == Some(end))
            .and_then(|l| l.old_line);
        Target::range_with_other_end(&file.path, start, end, Side::New, other_end).ok()
    }
}

/// The `Line` target for a diff line row: `Removed` lines anchor to the old
/// side/number, `Added`/`Context` lines to the new side/number. A `Context`
/// line exists on both sides, so its old-side number rides along as the
/// counterpart (some forge position formats need both). `None` only if the
/// row's own invariant (removed lines always carry `old_line`, non-removed
/// lines always carry `new_line`) is somehow violated.
fn line_target(path: &str, line: &LineRow) -> Option<Target> {
    match line.origin {
        LineOrigin::Removed => line.old_line.map(|n| Target::line(path, n, Side::Old)),
        LineOrigin::Added => line.new_line.map(|n| Target::line(path, n, Side::New)),
        LineOrigin::Context => line
            .new_line
            .map(|n| Target::line_with_other(path, n, Side::New, line.old_line)),
    }
}

/// The `(repo-relative path, 1-based line)` `g<Space>` opens in the
/// configured editor, for the cursor's current row. Unlike
/// [`target_for_cursor`]'s line targets — which use side-native numbering
/// (old-side numbers for `Removed` lines) and feed annotation storage — this
/// always resolves to a line number in the file **as it exists on disk**,
/// the only numbering an editor understands. A `Removed` line has no
/// corresponding line in that file, so it falls back to line 1 rather than
/// emitting a nonsensical `+0`; header rows ([`Row::FileHeader`]/
/// [`Row::HunkHeader`]) also open at line 1, since there is no more specific
/// line to jump to. `None` on [`Row::Binary`] (the caller special-cases this
/// with its own footer message rather than launching), on the
/// cursor-never-addresses-these display rows ([`Row::Annotation`]/
/// [`Row::AnnotationBorder`]), or when `cursor` is out of bounds.
pub(super) fn editor_target_for_cursor(
    file: &FileDiff,
    rows: &[Row],
    cursor: usize,
) -> Option<(String, u32)> {
    match rows.get(cursor)? {
        Row::Line(line) => {
            let ln = match line.origin {
                LineOrigin::Removed => 1,
                LineOrigin::Added | LineOrigin::Context => line.new_line.unwrap_or(1),
            };
            Some((file.path.clone(), ln))
        }
        Row::FileHeader { .. } | Row::HunkHeader { .. } => Some((file.path.clone(), 1)),
        Row::Binary
        | Row::Annotation { .. }
        | Row::AnnotationBorder { .. }
        | Row::Thread(_)
        | Row::ThreadBorder { .. } => None,
    }
}

/// Converts a target derived by [`target_for_cursor`]/[`target_for_visual`]
/// against the read-only file view's synthesized all-context body into the
/// "current worktree file content, not a diff side" target
/// forms: `Line` -> [`Target::WorktreeLine`], `Range` ->
/// [`Target::WorktreeRange`]. A `Hunk` target -- reachable if the cursor
/// lands on the file view's single synthetic hunk header, which spans the
/// whole file -- redirects to a `WorktreeRange` over the same span, since
/// "hunk" is a diff concept the file view has no other use for and a
/// same-shaped `(=)` range is the least-surprising equivalent. `File`
/// targets pass through unchanged: a whole-file comment already carries no
/// side marker at all, diffed or not, so there is nothing to convert.
///
/// Callers (`App::target_for_cursor`/`App::target_for_visual`) apply this
/// only when the active `DiffTarget` is `DiffTarget::File`, so an ordinary
/// diff-view target is never routed through here.
pub(super) fn as_worktree_target(target: Target) -> Target {
    match target {
        Target::Line { path, line, .. } => Target::worktree_line(path, line),
        Target::Range {
            path, start, end, ..
        } => {
            // `start <= end` was already validated when this `Range` was
            // first built, so re-validating here can only ever succeed --
            // but going through the fallible constructor (rather than
            // building the variant literal) keeps this one call site, not
            // two, aware of `Target::WorktreeRange`'s invariant.
            Target::worktree_range(&path, start, end).unwrap_or(Target::Range {
                path,
                start,
                end,
                side: Side::New,
                other_end: None,
            })
        }
        Target::Hunk {
            path, start, end, ..
        } => Target::worktree_range(&path, start, end).unwrap_or(Target::Hunk {
            path,
            start,
            end,
            other_end: None,
        }),
        file @ Target::File { .. } => file,
        // Already a worktree target (should not happen -- callers only
        // route diff-view-shaped targets through this function -- but a
        // defensive identity fallback costs one line and keeps this
        // function total without a panic path).
        worktree @ (Target::WorktreeLine { .. } | Target::WorktreeRange { .. }) => worktree,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotate::AnnotationStore;
    use crate::git::RawFilePatch;
    use crate::ui::rows::{SyntaxSpans, build_rows};

    fn file_diff(raw: &str, path: &str) -> FileDiff {
        FileDiff::from_patch(&RawFilePatch {
            path: path.to_string(),
            old_path: None,
            raw: raw.to_string(),
            is_binary: false,
        })
        .unwrap()
    }

    /// Builds the single-file row model the same way the diff pane does, so
    /// the targeting functions see the exact rows the cursor addresses.
    fn rows_for(file: &FileDiff) -> Vec<Row> {
        build_rows(file, &AnnotationStore::new(), SyntaxSpans::default())
    }

    fn sample() -> &'static str {
        "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,3 +1,3 @@
 ctx1
-old2
+new2
 ctx3
"
    }

    #[test]
    fn cursor_on_file_header_yields_file_target() {
        let file = file_diff(sample(), "f.rs");
        let rows = rows_for(&file);
        assert!(matches!(rows[0], Row::FileHeader { .. }));
        assert_eq!(
            target_for_cursor(&file, &rows, 0),
            Some(Target::file("f.rs"))
        );
    }

    #[test]
    fn cursor_on_hunk_header_yields_hunk_target() {
        let file = file_diff(sample(), "f.rs");
        let rows = rows_for(&file);
        assert!(matches!(rows[1], Row::HunkHeader { .. }));
        let (start, end) = hunk_span(&file.hunks[0]);
        // The span ends on ctx3 (new 3 / old 3), so the old-side counterpart
        // rides along.
        assert_eq!(
            target_for_cursor(&file, &rows, 1),
            Target::hunk_with_other_end("f.rs", start, end, Some(3)).ok()
        );
    }

    #[test]
    fn hunk_ending_on_an_added_line_carries_no_counterpart() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,1 +1,3 @@
 a
+b
+c
";
        let file = file_diff(raw, "f.rs");
        let rows = rows_for(&file);
        assert!(matches!(rows[1], Row::HunkHeader { .. }));
        let (start, end) = hunk_span(&file.hunks[0]);
        assert_eq!(
            target_for_cursor(&file, &rows, 1),
            Target::hunk_with_other_end("f.rs", start, end, None).ok()
        );
    }

    #[test]
    fn cursor_on_context_line_targets_new_side() {
        let file = file_diff(sample(), "f.rs");
        let rows = rows_for(&file);
        // rows: FileHeader(0) HunkHeader(1) ctx1(2) old2(3) new2(4) ctx3(5)
        let Row::Line(ctx1) = &rows[2] else {
            panic!("expected line row");
        };
        assert_eq!(ctx1.new_line, Some(1));
        // A context line exists on both sides; its old-side number rides
        // along as the counterpart.
        assert_eq!(
            target_for_cursor(&file, &rows, 2),
            Some(Target::line_with_other("f.rs", 1, Side::New, Some(1)))
        );
    }

    #[test]
    fn cursor_on_context_line_after_edits_records_the_shifted_old_counterpart() {
        // ctx3 sits at new 3 / old 3 here, but an insertion above would shift
        // them apart — the counterpart must come from the row, not the anchor.
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,2 +1,3 @@
 a
+b
 c
";
        let file = file_diff(raw, "f.rs");
        let rows = rows_for(&file);
        // rows: FileHeader(0) HunkHeader(1) a(2) b(3) c(4); c is new 3, old 2.
        assert_eq!(
            target_for_cursor(&file, &rows, 4),
            Some(Target::line_with_other("f.rs", 3, Side::New, Some(2)))
        );
    }

    #[test]
    fn cursor_on_removed_line_targets_old_side() {
        let file = file_diff(sample(), "f.rs");
        let rows = rows_for(&file);
        let Row::Line(old2) = &rows[3] else {
            panic!("expected removed line row");
        };
        assert_eq!(old2.origin, LineOrigin::Removed);
        assert_eq!(old2.old_line, Some(2));
        assert_eq!(
            target_for_cursor(&file, &rows, 3),
            Some(Target::line("f.rs", 2, Side::Old))
        );
    }

    #[test]
    fn cursor_on_added_line_targets_new_side() {
        let file = file_diff(sample(), "f.rs");
        let rows = rows_for(&file);
        let Row::Line(new2) = &rows[4] else {
            panic!("expected added line row");
        };
        assert_eq!(new2.origin, LineOrigin::Added);
        assert_eq!(new2.new_line, Some(2));
        assert_eq!(
            target_for_cursor(&file, &rows, 4),
            Some(Target::line("f.rs", 2, Side::New))
        );
    }

    #[test]
    fn cursor_out_of_bounds_yields_none() {
        let file = file_diff(sample(), "f.rs");
        let rows = rows_for(&file);
        assert_eq!(target_for_cursor(&file, &rows, rows.len()), None);
    }

    #[test]
    fn visual_range_over_added_lines_targets_new_side_range() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,1 +1,3 @@
 a
+b
+c
";
        let file = file_diff(raw, "f.rs");
        let rows = rows_for(&file);
        // rows: FileHeader(0) HunkHeader(1) a(2) b(3) c(4)
        // Select from context line a (new 1) through added c (new 3).
        assert_eq!(
            target_for_visual(&file, &rows, 4, 2),
            Target::range("f.rs", 1, 3, Side::New).ok()
        );
        // Order-independence: anchor after cursor gives the same range.
        assert_eq!(
            target_for_visual(&file, &rows, 2, 4),
            Target::range("f.rs", 1, 3, Side::New).ok()
        );
    }

    #[test]
    fn visual_range_over_only_removed_lines_targets_old_side() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,3 +1,1 @@
-x
-y
 z
";
        let file = file_diff(raw, "f.rs");
        let rows = rows_for(&file);
        // rows: FileHeader(0) HunkHeader(1) x(2, old 1) y(3, old 2) z(4)
        assert_eq!(
            target_for_visual(&file, &rows, 3, 2),
            Target::range("f.rs", 1, 2, Side::Old).ok()
        );
    }

    #[test]
    fn visual_range_ending_on_a_context_line_records_the_counterpart() {
        let file = file_diff(sample(), "f.rs");
        let rows = rows_for(&file);
        // Select ctx1 (new 1) through ctx3 (new 3, old 3): the end line is
        // context, so its old-side number rides along.
        assert_eq!(
            target_for_visual(&file, &rows, 5, 2),
            Target::range_with_other_end("f.rs", 1, 3, Side::New, Some(3)).ok()
        );
    }

    #[test]
    fn visual_selection_with_no_line_rows_yields_none() {
        let file = file_diff(sample(), "f.rs");
        let rows = rows_for(&file);
        // rows 0..=1 are FileHeader and HunkHeader — no line rows.
        assert_eq!(target_for_visual(&file, &rows, 0, 1), None);
    }

    // -- editor_target_for_cursor (g<Space>) ---------------------------------

    #[test]
    fn editor_target_on_file_header_is_line_one() {
        let file = file_diff(sample(), "f.rs");
        let rows = rows_for(&file);
        assert!(matches!(rows[0], Row::FileHeader { .. }));
        assert_eq!(
            editor_target_for_cursor(&file, &rows, 0),
            Some(("f.rs".to_string(), 1))
        );
    }

    #[test]
    fn editor_target_on_hunk_header_is_line_one() {
        let file = file_diff(sample(), "f.rs");
        let rows = rows_for(&file);
        assert!(matches!(rows[1], Row::HunkHeader { .. }));
        assert_eq!(
            editor_target_for_cursor(&file, &rows, 1),
            Some(("f.rs".to_string(), 1))
        );
    }

    #[test]
    fn editor_target_on_context_line_uses_new_line() {
        let file = file_diff(sample(), "f.rs");
        let rows = rows_for(&file);
        // rows: FileHeader(0) HunkHeader(1) ctx1(2, new 1) old2(3) new2(4) ctx3(5)
        assert_eq!(
            editor_target_for_cursor(&file, &rows, 2),
            Some(("f.rs".to_string(), 1))
        );
    }

    #[test]
    fn editor_target_on_added_line_uses_new_line() {
        let file = file_diff(sample(), "f.rs");
        let rows = rows_for(&file);
        // new2(4) is the Added row, new_line 2.
        assert_eq!(
            editor_target_for_cursor(&file, &rows, 4),
            Some(("f.rs".to_string(), 2))
        );
    }

    #[test]
    fn editor_target_on_removed_line_falls_back_to_line_one() {
        let file = file_diff(sample(), "f.rs");
        let rows = rows_for(&file);
        let Row::Line(old2) = &rows[3] else {
            panic!("expected removed line row");
        };
        assert_eq!(old2.origin, LineOrigin::Removed);
        assert_eq!(
            editor_target_for_cursor(&file, &rows, 3),
            Some(("f.rs".to_string(), 1))
        );
    }

    #[test]
    fn editor_target_out_of_bounds_yields_none() {
        let file = file_diff(sample(), "f.rs");
        let rows = rows_for(&file);
        assert_eq!(editor_target_for_cursor(&file, &rows, rows.len()), None);
    }

    // -- as_worktree_target ---------------------------------------------------

    #[test]
    fn as_worktree_target_converts_line_to_worktree_line() {
        assert_eq!(
            as_worktree_target(Target::line("docs/notes.md", 44, Side::New)),
            Target::worktree_line("docs/notes.md", 44)
        );
        // The side is irrelevant to the conversion -- a file view line is
        // never anchored to the old side, but the function stays total
        // rather than assuming its input's shape.
        assert_eq!(
            as_worktree_target(Target::line("docs/notes.md", 44, Side::Old)),
            Target::worktree_line("docs/notes.md", 44)
        );
    }

    #[test]
    fn as_worktree_target_converts_range_to_worktree_range() {
        assert_eq!(
            as_worktree_target(Target::range("docs/notes.md", 10, 20, Side::New).unwrap()),
            Target::worktree_range("docs/notes.md", 10, 20).unwrap()
        );
    }

    #[test]
    fn as_worktree_target_converts_hunk_to_worktree_range_over_the_same_span() {
        assert_eq!(
            as_worktree_target(Target::hunk("docs/notes.md", 1, 5).unwrap()),
            Target::worktree_range("docs/notes.md", 1, 5).unwrap()
        );
    }

    #[test]
    fn as_worktree_target_leaves_file_target_unchanged() {
        assert_eq!(
            as_worktree_target(Target::file("docs/notes.md")),
            Target::file("docs/notes.md")
        );
    }

    #[test]
    fn as_worktree_target_is_idempotent_on_worktree_targets() {
        assert_eq!(
            as_worktree_target(Target::worktree_line("docs/notes.md", 3)),
            Target::worktree_line("docs/notes.md", 3)
        );
        assert_eq!(
            as_worktree_target(Target::worktree_range("docs/notes.md", 3, 4).unwrap()),
            Target::worktree_range("docs/notes.md", 3, 4).unwrap()
        );
    }
}
