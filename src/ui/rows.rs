//! The flattened row model for the diff pane: pure data built once per
//! selected file (not recomputed per frame), so scrolling and cursor motion
//! stay instant even on large diffs.

use std::collections::HashMap;

use crate::diff::{
    FileChangeKind, FileDiff, Hunk, LineOrigin, WordSpan, pair_hunk_lines, word_diff,
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
}

/// One row of the flattened diff-pane row model. The cursor addresses rows
/// by index into the `Vec<Row>` for the selected file.
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
    },
    /// A `@@ -a,b +c,d @@ section` hunk header.
    HunkHeader {
        /// Index into the owning [`FileDiff::hunks`] this header starts.
        hunk_index: usize,
        /// The formatted header text.
        text: String,
    },
    /// One line of hunk content.
    Line(LineRow),
    /// A binary file's single placeholder row (no hunks are rendered).
    Binary,
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

/// Builds the flattened row model for one file: a [`Row::FileHeader`],
/// followed by a [`Row::HunkHeader`] and its [`Row::Line`]s per hunk, or a
/// single [`Row::Binary`] placeholder for binary files.
pub fn build_rows(file: &FileDiff) -> Vec<Row> {
    let mut rows = Vec::new();
    rows.push(Row::FileHeader {
        path: file.path.clone(),
        old_path: file.old_path.clone(),
        kind: file.kind,
    });

    if file.is_binary {
        rows.push(Row::Binary);
        return rows;
    }

    for (hunk_index, hunk) in file.hunks.iter().enumerate() {
        rows.push(Row::HunkHeader {
            hunk_index,
            text: format_hunk_header(hunk),
        });

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
            }));
        }
    }

    rows
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let rows = build_rows(&diff);
        assert_eq!(
            rows[0],
            Row::FileHeader {
                path: "f.rs".to_string(),
                old_path: None,
                kind: FileChangeKind::Modified,
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
        let rows = build_rows(&diff);
        assert_eq!(
            rows[1],
            Row::HunkHeader {
                hunk_index: 0,
                text: "@@ -10,2 +10,3 @@ fn foo() {".to_string(),
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
        let rows = build_rows(&diff);
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
        let rows = build_rows(&diff);
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
        let rows = build_rows(&diff);

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
        let rows = build_rows(&diff);
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
        let rows = build_rows(&diff);
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
        let rows = build_rows(&diff);
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
}
