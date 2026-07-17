//! Pure per-row wrap layout for the diff pane's soft line wrapping.
//!
//! Maps each logical [`Row`] to a visual height — 1 for every row except a
//! [`Row::Line`] whose content is wider than the available content width,
//! which wraps onto `n > 1` continuation rows — and provides the prefix-sum
//! lookups the scroll math and render walk need to convert between a
//! logical row index and a visual row offset. Pure data in / data out, no
//! ratatui or terminal types; see [`super::diff_view_state::DiffViewState`]
//! for how it's wired into scrolling (identity fallback when unbuilt) and
//! [`super::diff_view`] for how it drives the render walk.

use std::collections::HashMap;

use super::diff_view::content_col_offset;
use super::rows::Row;
use super::textwrap;

/// The visual layout of a row buffer at a given content width: each logical
/// row's height in terminal rows, a prefix-sum table over those heights (for
/// visual<->logical conversion), and the char-range partition for wrapped
/// (height > 1) `Row::Line` rows only — unwrapped rows need no entry, so a
/// buffer with no long lines carries an empty map.
///
/// `Default` produces the empty/"unbuilt" layout: `heights` and `ranges` are
/// empty and `prefix` is empty too, so [`WrapLayout::is_built_for`] is false
/// for any non-empty row buffer — the identity fallback in
/// [`super::diff_view_state::DiffViewState`] gates every accessor on that
/// check rather than trusting this type's own fallback values, which exist
/// only as a defensive backstop against a stale/partial layout.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct WrapLayout {
    /// One entry per logical row: its visual height (>= 1).
    heights: Vec<u32>,
    /// Prefix sums over `heights`; `prefix.len() == heights.len() + 1` when
    /// built, `prefix[0] == 0`, `prefix[heights.len()]` is the total visual
    /// row count.
    prefix: Vec<usize>,
    /// Char-range partitions for wrapped lines only, keyed by logical row
    /// index. Absent for every row with height 1 (including all non-`Line`
    /// rows and unwrapped `Line` rows).
    ranges: HashMap<usize, Vec<(usize, usize)>>,
}

impl WrapLayout {
    /// Builds the layout for `rows` at `inner_width` content columns, given
    /// the buffer's shared `gutter_width` (see
    /// [`super::rows::build_multibuffer`]). The content wrap width is
    /// `inner_width` minus the fixed gutter/marker prefix
    /// ([`content_col_offset`]), floored at 1 so a pathologically narrow
    /// pane still makes progress. `inner_width == 0` (no width fed yet, e.g.
    /// a test or code path that never calls
    /// [`super::diff_view_state::DiffViewState::set_content_width`]) returns
    /// the empty/unbuilt layout, so callers fall back to the identity
    /// (height-1) render path rather than wrapping at a nonsensical width.
    pub(super) fn build(rows: &[Row], gutter_width: usize, inner_width: usize) -> WrapLayout {
        if inner_width == 0 {
            return WrapLayout::default();
        }
        let wrap_width = inner_width
            .saturating_sub(content_col_offset(gutter_width))
            .max(1);
        let mut heights = Vec::with_capacity(rows.len());
        let mut ranges = HashMap::new();
        for (i, row) in rows.iter().enumerate() {
            if let Row::Line(line) = row {
                let r = textwrap::wrap_ranges(&line.content, wrap_width);
                let height = r.len().max(1) as u32;
                if r.len() > 1 {
                    ranges.insert(i, r);
                }
                heights.push(height);
            } else {
                heights.push(1);
            }
        }
        let mut prefix = Vec::with_capacity(heights.len() + 1);
        prefix.push(0);
        let mut acc = 0usize;
        for h in &heights {
            acc += *h as usize;
            prefix.push(acc);
        }
        WrapLayout {
            heights,
            prefix,
            ranges,
        }
    }

    /// Whether this layout was built for a row buffer of exactly
    /// `rows_len` rows — the gate every other accessor's caller should check
    /// before trusting this layout over the identity fallback (a rebuild
    /// that changes `rows.len()` without also calling
    /// [`WrapLayout::build`] again would otherwise silently misreport).
    pub(super) fn is_built_for(&self, rows_len: usize) -> bool {
        self.prefix.len() == rows_len + 1
    }

    /// The visual height (in terminal rows) of logical row `logical`. `1`
    /// for any row this layout doesn't know about.
    pub(super) fn row_height(&self, logical: usize) -> usize {
        self.heights.get(logical).copied().unwrap_or(1) as usize
    }

    /// The visual row offset (from the top of the buffer) where logical row
    /// `logical` begins. Falls back to `logical` itself (the identity
    /// mapping) when out of range.
    pub(super) fn visual_start(&self, logical: usize) -> usize {
        self.prefix.get(logical).copied().unwrap_or(logical)
    }

    /// The total visual row count across the whole buffer.
    pub(super) fn total_visual(&self) -> usize {
        self.prefix.last().copied().unwrap_or(0)
    }

    /// The logical row that owns visual row `v`: the last logical row whose
    /// `visual_start` is `<= v`. Exact inverse of [`WrapLayout::visual_start`]
    /// at row boundaries, and maps every interior wrapped-row offset to its
    /// owning logical row.
    pub(super) fn logical_of_visual(&self, v: usize) -> usize {
        self.prefix.partition_point(|&p| p <= v).saturating_sub(1)
    }

    /// The char-range partition for a wrapped `Row::Line` at `logical`.
    /// `None` for any row with height 1 (unwrapped, or not a `Row::Line`).
    pub(super) fn ranges_of(&self, logical: usize) -> Option<&[(usize, usize)]> {
        self.ranges.get(&logical).map(Vec::as_slice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotate::AnnotationStore;
    use crate::git::RawFilePatch;
    use crate::ui::rows::{ReviewMarker, StagedMarker, SyntaxSpans, build_multibuffer};

    fn rows_for(raw: &str) -> Vec<Row> {
        let file = crate::diff::FileDiff::from_patch(&RawFilePatch {
            path: "f.rs".to_string(),
            old_path: None,
            raw: raw.to_string(),
            is_binary: false,
        })
        .expect("patch parses");
        let mb = build_multibuffer(
            &[file],
            &[false],
            &[StagedMarker::None],
            &[ReviewMarker::None],
            &AnnotationStore::new(),
            &[SyntaxSpans::default()],
        );
        mb.rows
    }

    fn short_raw() -> &'static str {
        "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,2 +1,2 @@
 short one
 short two
"
    }

    fn long_line_raw(len: usize) -> String {
        let content = "x".repeat(len);
        format!(
            "diff --git a/f.rs b/f.rs\nindex 1..2 100644\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n+{content}\n"
        )
    }

    #[test]
    fn short_lines_get_height_one_and_identity_prefix() {
        let rows = rows_for(short_raw());
        let layout = WrapLayout::build(&rows, 3, 200);
        assert!(layout.is_built_for(rows.len()));
        for i in 0..rows.len() {
            assert_eq!(layout.row_height(i), 1);
            assert_eq!(layout.visual_start(i), i);
        }
        assert_eq!(layout.total_visual(), rows.len());
    }

    #[test]
    fn long_line_wraps_to_more_than_one_visual_row() {
        let rows = rows_for(&long_line_raw(200));
        // Narrow width: content_col_offset(3) = 2 + 3*2 + 3 = 11, so
        // wrap width = 30 - 11 = 19 chars for a 200-char line -> multiple rows.
        let layout = WrapLayout::build(&rows, 3, 30);
        let line_idx = rows
            .iter()
            .position(|r| matches!(r, Row::Line(_)))
            .expect("a line row exists");
        assert!(layout.row_height(line_idx) > 1);
        assert!(layout.total_visual() > rows.len());
        // Every row after the wrapped one shifts by the extra height.
        let extra = layout.row_height(line_idx) - 1;
        if line_idx + 1 < rows.len() {
            assert_eq!(
                layout.visual_start(line_idx + 1),
                layout.visual_start(line_idx) + layout.row_height(line_idx)
            );
            assert_eq!(layout.visual_start(line_idx + 1), (line_idx + 1) + extra);
        }
    }

    #[test]
    fn logical_of_visual_inverts_visual_start_and_maps_interior_rows() {
        let rows = rows_for(&long_line_raw(200));
        let layout = WrapLayout::build(&rows, 3, 30);
        let line_idx = rows
            .iter()
            .position(|r| matches!(r, Row::Line(_)))
            .expect("a line row exists");
        let height = layout.row_height(line_idx);
        assert!(height > 1);
        let start = layout.visual_start(line_idx);
        // Every visual row spanned by the wrapped logical row maps back to it.
        for v in start..start + height {
            assert_eq!(layout.logical_of_visual(v), line_idx);
        }
        // The boundary at row boundaries is exact: the row right after this
        // one's span maps to the next logical row.
        if line_idx + 1 < rows.len() {
            assert_eq!(layout.logical_of_visual(start + height), line_idx + 1);
        }
    }

    #[test]
    fn zero_inner_width_yields_an_unbuilt_identity_layout() {
        let rows = rows_for(&long_line_raw(200));
        let layout = WrapLayout::build(&rows, 3, 0);
        assert!(!layout.is_built_for(rows.len()));
        assert_eq!(layout, WrapLayout::default());
    }

    #[test]
    fn ranges_of_is_none_for_unwrapped_rows_and_some_for_wrapped() {
        let rows = rows_for(short_raw());
        let layout = WrapLayout::build(&rows, 3, 200);
        for i in 0..rows.len() {
            assert_eq!(layout.ranges_of(i), None);
        }

        let long_rows = rows_for(&long_line_raw(200));
        let layout = WrapLayout::build(&long_rows, 3, 30);
        let line_idx = long_rows
            .iter()
            .position(|r| matches!(r, Row::Line(_)))
            .unwrap();
        let ranges = layout.ranges_of(line_idx).expect("wrapped line has ranges");
        assert!(ranges.len() > 1);
        // Ranges partition the content contiguously.
        let mut expected_start = 0;
        for (s, e) in ranges {
            assert_eq!(*s, expected_start);
            expected_start = *e;
        }
        assert_eq!(expected_start, 200);
    }
}
