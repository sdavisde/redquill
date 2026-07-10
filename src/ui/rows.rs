//! The flattened row model for the diff pane: pure data built once per
//! selected file (not recomputed per frame), so scrolling and cursor motion
//! stay instant even on large diffs.
//!
//! Annotations attached to the file are spliced in as [`Row::Annotation`]
//! rows immediately after the row they anchor to (a line, a hunk header, or
//! the file header), and the anchor row itself is flagged `annotated` so the
//! gutter can show a marker even when the inline annotation text has
//! scrolled out of view (this matters for [`crate::annotate::Target::Range`]
//! annotations, which mark every covered line).

use std::collections::{HashMap, HashSet};

use crate::annotate::{Annotation, AnnotationStore, Classification, Side, Target};
use crate::diff::{
    DiffLine, FileChangeKind, FileDiff, Hunk, LineOrigin, WordSpan, pair_hunk_lines, word_diff,
};

/// One rendered line of the diff pane's content, carrying everything the
/// widget needs precomputed: both gutter numbers, origin, content, and (for
/// lines paired via [`pair_hunk_lines`]) word-diff spans against their
/// counterpart.
#[derive(Debug, Clone, PartialEq)]
pub struct LineRow {
    /// Index into the owning [`FileDiff::hunks`] this line belongs to.
    pub hunk_index: usize,
    /// 1-based old-side line number, if the line exists there.
    pub old_line: Option<u32>,
    /// 1-based new-side line number, if the line exists there.
    pub new_line: Option<u32>,
    /// Which side of the diff this line belongs to.
    pub origin: LineOrigin,
    /// The line's text.
    pub content: String,
    /// Word-level diff spans against this line's paired counterpart, if any
    /// (see [`pair_hunk_lines`]). Precomputed at row-build time.
    pub word_spans: Option<Vec<WordSpan>>,
    /// Whether the file has no trailing newline after this line.
    pub no_newline: bool,
    /// Whether this line is covered by at least one annotation (a `Line`
    /// target on this exact line, or a `Range` target spanning it).
    pub annotated: bool,
}

/// One row of the flattened diff-pane row model. The cursor addresses rows
/// by index into the `Vec<Row>` for the selected file; [`Row::Annotation`]
/// rows are display-only and never addressable (see [`Row::is_addressable`]).
#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    /// The selected file's summary line: path, old path (for renames), and
    /// change kind.
    FileHeader {
        /// The current (b-side) path.
        path: String,
        /// The original (a-side) path, for renames.
        old_path: Option<String>,
        /// The kind of change.
        kind: FileChangeKind,
        /// Whether this file has a `Target::File` annotation.
        annotated: bool,
    },
    /// A `@@ -a,b +c,d @@ section` hunk header.
    HunkHeader {
        /// Index into the owning [`FileDiff::hunks`] this header starts.
        hunk_index: usize,
        /// The formatted header text.
        text: String,
        /// Whether this hunk has a `Target::Hunk` annotation.
        annotated: bool,
    },
    /// One line of hunk content.
    Line(LineRow),
    /// A binary file's single placeholder row (no hunks are rendered).
    Binary,
    /// A display-only line of an annotation's body, spliced in after its
    /// anchor row. `classification` is `Some` on the first line of the body
    /// (rendered with the `●` marker and `[label]` tag) and `None` on
    /// continuation lines (rendered indented, no marker).
    Annotation {
        /// The id of the annotation this row belongs to.
        id: usize,
        /// One line of the annotation body.
        text: String,
        /// `Some` only for the first rendered line of the body.
        classification: Option<Classification>,
    },
}

impl Row {
    /// Whether the cursor can land on this row. [`Row::Annotation`] rows
    /// are display-only.
    pub fn is_addressable(&self) -> bool {
        !matches!(self, Row::Annotation { .. })
    }
}

/// Formats a hunk's `@@ -a,b +c,d @@[ section]` header line.
fn format_hunk_header(hunk: &Hunk) -> String {
    let mut text = format!(
        "@@ -{},{} +{},{} @@",
        hunk.old_start, hunk.old_count, hunk.new_start, hunk.new_count
    );
    if let Some(section) = &hunk.section {
        text.push(' ');
        text.push_str(section);
    }
    text
}

/// Precomputes word-diff spans for every paired line in a hunk, keyed by
/// index into `hunk.lines`.
fn word_spans_by_line(hunk: &Hunk) -> HashMap<usize, Vec<WordSpan>> {
    let mut spans = HashMap::new();
    for (removed_idx, added_idx) in pair_hunk_lines(hunk) {
        let (old_spans, new_spans) = word_diff(
            &hunk.lines[removed_idx].content,
            &hunk.lines[added_idx].content,
        );
        spans.insert(removed_idx, old_spans);
        spans.insert(added_idx, new_spans);
    }
    spans
}

/// A hunk's annotation-anchoring span: its new-side `(start, end)`, or (for
/// a hunk whose new side is empty, e.g. a pure deletion) its old-side span
/// instead. This is the same span [`crate::annotate::Target::hunk`] targets
/// are constructed with, so hunk annotations can be matched back to the
/// header row that anchors them.
pub(crate) fn hunk_span(hunk: &Hunk) -> (u32, u32) {
    if hunk.new_count > 0 {
        (hunk.new_start, hunk.new_start + hunk.new_count - 1)
    } else {
        (
            hunk.old_start,
            hunk.old_start + hunk.old_count.saturating_sub(1),
        )
    }
}

/// The line number on `side`, if the line exists there.
fn diff_line_number(line: &DiffLine, side: Side) -> Option<u32> {
    match side {
        Side::Old => line.old_line,
        Side::New => line.new_line,
    }
}

fn line_row_number(line: &LineRow, side: Side) -> Option<u32> {
    match side {
        Side::Old => line.old_line,
        Side::New => line.new_line,
    }
}

/// Converts one annotation's body into its display rows: the first line
/// tagged with the marker and classification, continuation lines indented
/// and untagged.
fn annotation_rows(annotation: &Annotation) -> Vec<Row> {
    let mut lines = annotation.body.lines();
    let mut rows = Vec::new();
    if let Some(first) = lines.next() {
        rows.push(Row::Annotation {
            id: annotation.id,
            text: first.to_string(),
            classification: Some(annotation.classification),
        });
    }
    for line in lines {
        rows.push(Row::Annotation {
            id: annotation.id,
            text: line.to_string(),
            classification: None,
        });
    }
    rows
}

/// Builds the flattened row model for one file: a [`Row::FileHeader`],
/// followed by a [`Row::HunkHeader`] and its [`Row::Line`]s per hunk, or a
/// single [`Row::Binary`] placeholder for binary files. Annotations in
/// `annotations` targeting this file are spliced in as [`Row::Annotation`]
/// rows after their anchor, with covered rows flagged `annotated`.
pub fn build_rows(file: &FileDiff, annotations: &AnnotationStore) -> Vec<Row> {
    let file_annotations: Vec<&Annotation> = annotations.for_path(&file.path).collect();

    let mut rows = Vec::new();

    let file_targeted: Vec<&Annotation> = file_annotations
        .iter()
        .filter(|a| matches!(a.target, Target::File { .. }))
        .copied()
        .collect();
    rows.push(Row::FileHeader {
        path: file.path.clone(),
        old_path: file.old_path.clone(),
        kind: file.kind,
        annotated: !file_targeted.is_empty(),
    });
    for a in &file_targeted {
        rows.extend(annotation_rows(a));
    }

    if file.is_binary {
        rows.push(Row::Binary);
        return rows;
    }

    for (hunk_index, hunk) in file.hunks.iter().enumerate() {
        let (hstart, hend) = hunk_span(hunk);
        let hunk_targeted: Vec<&Annotation> = file_annotations
            .iter()
            .filter(|a| matches!(&a.target, Target::Hunk { start, end, .. } if *start == hstart && *end == hend))
            .copied()
            .collect();
        rows.push(Row::HunkHeader {
            hunk_index,
            text: format_hunk_header(hunk),
            annotated: !hunk_targeted.is_empty(),
        });
        for a in &hunk_targeted {
            rows.extend(annotation_rows(a));
        }

        // For each annotation targeting a Line/Range in this hunk, compute
        // the set of covered hunk-line indices (for the gutter dot) and the
        // last covered index (where its display splices in).
        let mut dotted: HashSet<usize> = HashSet::new();
        let mut splice_after: HashMap<usize, Vec<&Annotation>> = HashMap::new();
        for a in &file_annotations {
            match &a.target {
                Target::Line { line, side, .. } => {
                    if let Some(idx) = hunk
                        .lines
                        .iter()
                        .position(|l| diff_line_number(l, *side) == Some(*line))
                    {
                        dotted.insert(idx);
                        splice_after.entry(idx).or_default().push(a);
                    }
                }
                Target::Range {
                    start, end, side, ..
                } => {
                    let covered: Vec<usize> = hunk
                        .lines
                        .iter()
                        .enumerate()
                        .filter(|(_, l)| {
                            diff_line_number(l, *side).is_some_and(|n| n >= *start && n <= *end)
                        })
                        .map(|(i, _)| i)
                        .collect();
                    if let Some(&last) = covered.iter().max() {
                        dotted.extend(&covered);
                        splice_after.entry(last).or_default().push(a);
                    }
                }
                _ => {}
            }
        }

        let mut spans = word_spans_by_line(hunk);
        for (line_index, line) in hunk.lines.iter().enumerate() {
            rows.push(Row::Line(LineRow {
                hunk_index,
                old_line: line.old_line,
                new_line: line.new_line,
                origin: line.origin,
                content: line.content.clone(),
                word_spans: spans.remove(&line_index),
                no_newline: line.no_newline,
                annotated: dotted.contains(&line_index),
            }));
            if let Some(list) = splice_after.get(&line_index) {
                for a in list {
                    rows.extend(annotation_rows(a));
                }
            }
        }
    }

    rows
}

/// Locates the row in `rows` (built via [`build_rows`] for `file`) that
/// anchors `target`: row `0` for a file target, the matching `HunkHeader`
/// for a hunk target, or the first `Line` row whose gutter number (on the
/// target's side) falls within the target's line/range. `None` if no
/// matching row exists (e.g. the annotation's file/hunk/line no longer
/// exists in `rows`).
pub(crate) fn anchor_row_index(file: &FileDiff, rows: &[Row], target: &Target) -> Option<usize> {
    match target {
        Target::File { .. } => Some(0),
        Target::Hunk { start, end, .. } => rows.iter().position(|r| match r {
            Row::HunkHeader { hunk_index, .. } => file
                .hunks
                .get(*hunk_index)
                .is_some_and(|h| hunk_span(h) == (*start, *end)),
            _ => false,
        }),
        Target::Line { line, side, .. } => rows.iter().position(|r| match r {
            Row::Line(l) => line_row_number(l, *side) == Some(*line),
            _ => false,
        }),
        Target::Range {
            start, end, side, ..
        } => rows.iter().position(|r| match r {
            Row::Line(l) => line_row_number(l, *side).is_some_and(|n| n >= *start && n <= *end),
            _ => false,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotate::Classification;
    use crate::git::RawFilePatch;

    fn file_diff(raw: &str, path: &str, is_binary: bool) -> FileDiff {
        FileDiff::from_patch(&RawFilePatch {
            path: path.to_string(),
            old_path: None,
            raw: raw.to_string(),
            is_binary,
        })
        .unwrap()
    }

    fn no_notes() -> AnnotationStore {
        AnnotationStore::new()
    }

    #[test]
    fn builds_file_header_first() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1 +1 @@
-a
+b
";
        let diff = file_diff(raw, "f.rs", false);
        let rows = build_rows(&diff, &no_notes());
        assert_eq!(
            rows[0],
            Row::FileHeader {
                path: "f.rs".to_string(),
                old_path: None,
                kind: FileChangeKind::Modified,
                annotated: false,
            }
        );
    }

    #[test]
    fn hunk_header_is_formatted_with_section() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -10,2 +10,3 @@ fn foo() {
 context
+added
 context2
";
        let diff = file_diff(raw, "f.rs", false);
        let rows = build_rows(&diff, &no_notes());
        assert_eq!(
            rows[1],
            Row::HunkHeader {
                hunk_index: 0,
                text: "@@ -10,2 +10,3 @@ fn foo() {".to_string(),
                annotated: false,
            }
        );
    }

    #[test]
    fn hunk_header_without_section_has_no_trailing_text() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1 +1 @@
-a
+b
";
        let diff = file_diff(raw, "f.rs", false);
        let rows = build_rows(&diff, &no_notes());
        let Row::HunkHeader { text, .. } = &rows[1] else {
            panic!("expected hunk header");
        };
        assert_eq!(text, "@@ -1,1 +1,1 @@");
    }

    #[test]
    fn line_rows_carry_gutter_numbers_and_content() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 111..222 100644
--- a/f.rs
+++ b/f.rs
@@ -1,3 +1,4 @@
 line1
-line2
+line2 mod
+line new
 line3
";
        let diff = file_diff(raw, "f.rs", false);
        let rows = build_rows(&diff, &no_notes());
        // FileHeader, HunkHeader, then 5 line rows.
        assert_eq!(rows.len(), 7);

        let Row::Line(context) = &rows[2] else {
            panic!("expected line row");
        };
        assert_eq!(context.old_line, Some(1));
        assert_eq!(context.new_line, Some(1));
        assert_eq!(context.origin, LineOrigin::Context);
        assert_eq!(context.content, "line1");
        assert_eq!(context.word_spans, None);
        assert!(!context.annotated);

        let Row::Line(removed) = &rows[3] else {
            panic!("expected line row");
        };
        assert_eq!(removed.old_line, Some(2));
        assert_eq!(removed.new_line, None);
        assert_eq!(removed.origin, LineOrigin::Removed);
    }

    #[test]
    fn paired_lines_get_word_diff_spans() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 111..222 100644
--- a/f.rs
+++ b/f.rs
@@ -1 +1 @@
-let x = foo;
+let x = bar;
";
        let diff = file_diff(raw, "f.rs", false);
        let rows = build_rows(&diff, &no_notes());

        let Row::Line(removed) = &rows[2] else {
            panic!("expected removed line row");
        };
        let removed_spans = removed.word_spans.as_ref().expect("removed spans");
        assert!(removed_spans.iter().any(|s| s.changed));

        let Row::Line(added) = &rows[3] else {
            panic!("expected added line row");
        };
        let added_spans = added.word_spans.as_ref().expect("added spans");
        assert!(added_spans.iter().any(|s| s.changed));
    }

    #[test]
    fn unpaired_lines_have_no_word_spans() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 111..222 100644
--- a/f.rs
+++ b/f.rs
@@ -1,2 +1,2 @@
-only removed
 context
";
        let diff = file_diff(raw, "f.rs", false);
        let rows = build_rows(&diff, &no_notes());
        let Row::Line(removed) = &rows[2] else {
            panic!("expected line row");
        };
        assert_eq!(removed.word_spans, None);
    }

    #[test]
    fn binary_file_yields_single_placeholder_row() {
        let raw = "\
diff --git a/img.png b/img.png
index 1..2 100644
Binary files a/img.png and b/img.png differ
";
        let diff = file_diff(raw, "img.png", true);
        let rows = build_rows(&diff, &no_notes());
        assert_eq!(rows.len(), 2);
        assert!(matches!(rows[0], Row::FileHeader { .. }));
        assert_eq!(rows[1], Row::Binary);
    }

    #[test]
    fn multiple_hunks_produce_independent_headers() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 111..222 100644
--- a/f.rs
+++ b/f.rs
@@ -1,1 +1,1 @@
-a
+A
@@ -10,1 +10,1 @@
-j
+J
";
        let diff = file_diff(raw, "f.rs", false);
        let rows = build_rows(&diff, &no_notes());
        let hunk_headers: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter_map(|(i, r)| matches!(r, Row::HunkHeader { .. }).then_some(i))
            .collect();
        assert_eq!(hunk_headers, vec![1, 4]);
        let Row::HunkHeader { hunk_index, .. } = &rows[4] else {
            panic!("expected hunk header");
        };
        assert_eq!(*hunk_index, 1);
    }

    fn raw_two_line_hunk() -> &'static str {
        "\
diff --git a/f.rs b/f.rs
index 111..222 100644
--- a/f.rs
+++ b/f.rs
@@ -1,2 +1,2 @@
-old1
+new1
 ctx
"
    }

    #[test]
    fn line_annotation_marks_row_and_splices_display_after_it() {
        let diff = file_diff(raw_two_line_hunk(), "f.rs", false);
        let mut store = AnnotationStore::new();
        store
            .add(
                Target::line("f.rs", 1, Side::New),
                Classification::Question,
                "why change this?",
            )
            .unwrap();
        let rows = build_rows(&diff, &store);

        // rows: FileHeader(0), HunkHeader(1), Line old1(2), Line new1(3),
        // Annotation(4), Line ctx(5)
        let Row::Line(new1) = &rows[3] else {
            panic!("expected added line row");
        };
        assert!(new1.annotated);
        let Row::Line(old1) = &rows[2] else {
            panic!("expected removed line row");
        };
        assert!(!old1.annotated);

        match &rows[4] {
            Row::Annotation {
                text,
                classification,
                ..
            } => {
                assert_eq!(text, "why change this?");
                assert_eq!(*classification, Some(Classification::Question));
            }
            other => panic!("expected annotation row, got {other:?}"),
        }
        assert!(matches!(rows[5], Row::Line(_)));
    }

    #[test]
    fn range_annotation_marks_every_covered_line_and_splices_after_last() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 111..222 100644
--- a/f.rs
+++ b/f.rs
@@ -1,3 +1,3 @@
 a
+b
+c
";
        let diff = file_diff(raw, "f.rs", false);
        let mut store = AnnotationStore::new();
        store
            .add(
                Target::range("f.rs", 2, 3, Side::New).unwrap(),
                Classification::Nit,
                "extract helper",
            )
            .unwrap();
        let rows = build_rows(&diff, &store);
        // rows: FileHeader(0) HunkHeader(1) Line a(2) Line b(3) Line c(4) Annotation(5)
        let Row::Line(b) = &rows[3] else {
            panic!("expected line b");
        };
        assert!(b.annotated);
        let Row::Line(c) = &rows[4] else {
            panic!("expected line c");
        };
        assert!(c.annotated);
        assert!(matches!(rows[5], Row::Annotation { .. }));
    }

    #[test]
    fn hunk_annotation_displays_under_hunk_header() {
        let diff = file_diff(raw_two_line_hunk(), "f.rs", false);
        let mut store = AnnotationStore::new();
        store
            .add(
                Target::hunk("f.rs", 1, 2).unwrap(),
                Classification::Praise,
                "clean",
            )
            .unwrap();
        let rows = build_rows(&diff, &store);
        // FileHeader(0) HunkHeader(1) Annotation(2) Line(3)...
        let Row::HunkHeader { annotated, .. } = &rows[1] else {
            panic!("expected hunk header");
        };
        assert!(annotated);
        assert!(matches!(rows[2], Row::Annotation { .. }));
    }

    #[test]
    fn file_annotation_displays_under_file_header() {
        let diff = file_diff(raw_two_line_hunk(), "f.rs", false);
        let mut store = AnnotationStore::new();
        store
            .add(Target::file("f.rs"), Classification::Praise, "nice module")
            .unwrap();
        let rows = build_rows(&diff, &store);
        let Row::FileHeader { annotated, .. } = &rows[0] else {
            panic!("expected file header");
        };
        assert!(annotated);
        assert!(matches!(rows[1], Row::Annotation { .. }));
    }

    #[test]
    fn multiline_body_produces_first_line_tagged_and_continuation_untagged() {
        let diff = file_diff(raw_two_line_hunk(), "f.rs", false);
        let mut store = AnnotationStore::new();
        store
            .add(Target::file("f.rs"), Classification::Issue, "first\nsecond")
            .unwrap();
        let rows = build_rows(&diff, &store);
        match &rows[1] {
            Row::Annotation {
                text,
                classification,
                ..
            } => {
                assert_eq!(text, "first");
                assert!(classification.is_some());
            }
            other => panic!("unexpected row {other:?}"),
        }
        match &rows[2] {
            Row::Annotation {
                text,
                classification,
                ..
            } => {
                assert_eq!(text, "second");
                assert!(classification.is_none());
            }
            other => panic!("unexpected row {other:?}"),
        }
    }

    #[test]
    fn annotation_rows_are_not_addressable_other_rows_are() {
        let diff = file_diff(raw_two_line_hunk(), "f.rs", false);
        let mut store = AnnotationStore::new();
        store
            .add(Target::file("f.rs"), Classification::Issue, "note")
            .unwrap();
        let rows = build_rows(&diff, &store);
        assert!(rows[0].is_addressable());
        assert!(!rows[1].is_addressable());
        assert!(rows[2].is_addressable());
    }

    #[test]
    fn hunk_span_uses_old_side_when_new_count_is_zero() {
        let raw = "\
diff --git a/gone.rs b/gone.rs
deleted file mode 100644
index 111..000
--- a/gone.rs
+++ /dev/null
@@ -1,3 +0,0 @@
-a
-b
-c
";
        let diff = file_diff(raw, "gone.rs", false);
        assert_eq!(hunk_span(&diff.hunks[0]), (1, 3));
    }

    #[test]
    fn anchor_row_index_finds_file_hunk_and_line_targets() {
        let diff = file_diff(raw_two_line_hunk(), "f.rs", false);
        let rows = build_rows(&diff, &no_notes());

        assert_eq!(
            anchor_row_index(&diff, &rows, &Target::file("f.rs")),
            Some(0)
        );
        assert_eq!(
            anchor_row_index(&diff, &rows, &Target::hunk("f.rs", 1, 2).unwrap()),
            Some(1)
        );
        assert_eq!(
            anchor_row_index(&diff, &rows, &Target::line("f.rs", 1, Side::New)),
            Some(3)
        );
        assert_eq!(
            anchor_row_index(
                &diff,
                &rows,
                &Target::range("f.rs", 2, 2, Side::New).unwrap()
            ),
            Some(4)
        );
    }

    #[test]
    fn anchor_row_index_returns_none_when_missing() {
        let diff = file_diff(raw_two_line_hunk(), "f.rs", false);
        let rows = build_rows(&diff, &no_notes());
        assert_eq!(
            anchor_row_index(&diff, &rows, &Target::line("f.rs", 99, Side::New)),
            None
        );
    }
}
