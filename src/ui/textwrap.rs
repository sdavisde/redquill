//! Pure soft word-wrap layout for the Compose and commit-message modals.
//!
//! Given a buffer's logical lines and a wrap width (in columns/char cells),
//! [`layout`] produces the ordered list of *visual* rows the modal renders,
//! and [`WrapLayout::cursor_position`] maps a `(logical_row, logical_col)`
//! cursor onto its `(visual_row, visual_col)`. Both the rendered rows and the
//! cursor share this one layout, so the terminal cursor always lands on the
//! character it edits — no edge-clamping approximation.
//!
//! Wrapping is word-first with a glyph fallback (`WordOrGlyph`): a row breaks
//! at the last whitespace that fits; a single word wider than the width is
//! hard-split at the width. The visual rows *partition* each logical line —
//! every char index belongs to exactly one row and rows are contiguous — which
//! is what keeps the cursor mapping lossless. This module is pure data in /
//! data out with no ratatui or terminal types, and is unit-tested directly.

/// One rendered row: a `[start_col, end_col)` char range within its
/// `logical_line`. Ranges are half-open and contiguous across the rows of a
/// logical line (row *n*'s `end_col` equals row *n+1*'s `start_col`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct VisualRow {
    /// Index of the logical (buffer) line this visual row came from.
    pub logical_line: usize,
    /// First char index (inclusive) of this row within the logical line.
    pub start_col: usize,
    /// One-past-the-last char index (exclusive) of this row.
    pub end_col: usize,
}

/// The full set of visual rows for a buffer at a given wrap width, in render
/// order. Always has at least one row (an empty buffer wraps to one empty row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WrapLayout {
    /// The visual rows, top to bottom.
    pub rows: Vec<VisualRow>,
}

impl WrapLayout {
    /// Maps a logical cursor `(cursor_row, cursor_col)` to its visual
    /// `(visual_row, visual_col)`. `cursor_col` may equal the logical line's
    /// length (cursor past the last char), which maps to the end of that
    /// line's last visual row.
    pub(super) fn cursor_position(&self, cursor_row: usize, cursor_col: usize) -> (usize, usize) {
        let mut last_of_line: Option<usize> = None;
        for (vi, r) in self.rows.iter().enumerate() {
            if r.logical_line != cursor_row {
                continue;
            }
            last_of_line = Some(vi);
            if cursor_col >= r.start_col && cursor_col < r.end_col {
                return (vi, cursor_col - r.start_col);
            }
        }
        // cursor_col == line length (or an out-of-range row): fall to the last
        // visual row of the logical line and sit just past its content.
        if let Some(vi) = last_of_line {
            let r = &self.rows[vi];
            return (vi, cursor_col.saturating_sub(r.start_col));
        }
        (0, 0)
    }
}

/// Builds the [`WrapLayout`] for `lines` wrapped at `width` columns. A `width`
/// of 0 is treated as 1 (never panics, never produces zero-progress rows).
pub(super) fn layout(lines: &[String], width: usize) -> WrapLayout {
    let mut rows = Vec::new();
    for (li, line) in lines.iter().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        for (start_col, end_col) in wrap_line(&chars, width) {
            rows.push(VisualRow {
                logical_line: li,
                start_col,
                end_col,
            });
        }
    }
    if rows.is_empty() {
        rows.push(VisualRow {
            logical_line: 0,
            start_col: 0,
            end_col: 0,
        });
    }
    WrapLayout { rows }
}

/// The substring of `line` a `row` covers, as an owned `String` (sliced by
/// char index, so it's safe on multi-byte content).
pub(super) fn row_str(line: &str, row: &VisualRow) -> String {
    line.chars()
        .skip(row.start_col)
        .take(row.end_col - row.start_col)
        .collect()
}

/// Splits one logical line (as chars) into contiguous `[start, end)` char
/// ranges no wider than `width`, breaking at the last whitespace that fits and
/// hard-splitting words wider than `width`. An empty line yields one empty
/// range so it still occupies a visual row.
fn wrap_line(chars: &[char], width: usize) -> Vec<(usize, usize)> {
    let width = width.max(1);
    if chars.is_empty() {
        return vec![(0, 0)];
    }
    let mut rows = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        if chars.len() - start <= width {
            rows.push((start, chars.len()));
            break;
        }
        let hard_end = start + width; // exclusive cap: at most `width` chars
        // Prefer breaking after the last whitespace within the window so the
        // next row starts on a word; otherwise hard-split at the width.
        let mut break_end = hard_end;
        for i in (start..hard_end).rev() {
            if chars[i].is_whitespace() {
                break_end = i + 1;
                break;
            }
        }
        rows.push((start, break_end));
        start = break_end;
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.split('\n').map(str::to_string).collect()
    }

    #[test]
    fn short_line_is_one_row() {
        let l = lines("hello");
        let layout = layout(&l, 20);
        assert_eq!(layout.rows.len(), 1);
        assert_eq!(row_str("hello", &layout.rows[0]), "hello");
    }

    #[test]
    fn empty_buffer_yields_one_empty_row() {
        let l = lines("");
        let layout = layout(&l, 20);
        assert_eq!(layout.rows.len(), 1);
        assert_eq!(
            layout.rows[0],
            VisualRow {
                logical_line: 0,
                start_col: 0,
                end_col: 0
            }
        );
    }

    #[test]
    fn wraps_at_word_boundary() {
        // width 10: "hello " (6) fits, "world" would overflow → break after
        // the space.
        let l = lines("hello world");
        let layout = layout(&l, 10);
        assert_eq!(layout.rows.len(), 2);
        assert_eq!(row_str("hello world", &layout.rows[0]), "hello ");
        assert_eq!(row_str("hello world", &layout.rows[1]), "world");
    }

    #[test]
    fn rows_partition_the_line_contiguously() {
        let l = lines("the quick brown fox jumps");
        let layout = layout(&l, 8);
        // Every row belongs to line 0, and ranges chain end→start with no gap
        // or overlap, covering [0, len).
        let len = "the quick brown fox jumps".chars().count();
        let mut expected_start = 0;
        for r in &layout.rows {
            assert_eq!(r.logical_line, 0);
            assert_eq!(r.start_col, expected_start);
            expected_start = r.end_col;
        }
        assert_eq!(expected_start, len);
    }

    #[test]
    fn word_wider_than_width_hard_splits() {
        let l = lines("supercalifragilistic");
        let layout = layout(&l, 5);
        // 20 chars / width 5 = 4 rows, each exactly 5 wide.
        assert_eq!(layout.rows.len(), 4);
        for r in &layout.rows {
            assert_eq!(r.end_col - r.start_col, 5);
        }
        assert_eq!(row_str("supercalifragilistic", &layout.rows[0]), "super");
    }

    #[test]
    fn width_zero_and_one_do_not_panic() {
        let l = lines("abc");
        let z = layout(&l, 0);
        assert_eq!(z.rows.len(), 3); // each char its own row (treated as w=1)
        let one = layout(&l, 1);
        assert_eq!(one.rows.len(), 3);
    }

    #[test]
    fn multiple_logical_lines_keep_their_indices() {
        let l = lines("aaa\nbbbbbbbb");
        let layout = layout(&l, 4);
        assert_eq!(layout.rows[0].logical_line, 0);
        // "bbbbbbbb" (8) at width 4 → two rows, both logical line 1.
        assert_eq!(layout.rows[1].logical_line, 1);
        assert_eq!(layout.rows[2].logical_line, 1);
    }

    #[test]
    fn cursor_maps_within_a_wrapped_row() {
        let l = lines("hello world");
        let layout = layout(&l, 10);
        // col 2 is on row 0 at visual col 2.
        assert_eq!(layout.cursor_position(0, 2), (0, 2));
        // col 6 is the start of "world" on row 1 at visual col 0.
        assert_eq!(layout.cursor_position(0, 6), (1, 0));
        // col 8 is "r" of "world": row 1, visual col 2.
        assert_eq!(layout.cursor_position(0, 8), (1, 2));
    }

    #[test]
    fn cursor_at_line_end_maps_to_last_rows_end() {
        let l = lines("hello world");
        let layout = layout(&l, 10);
        // col 11 == line length: last row ("world") end, visual col 5.
        assert_eq!(layout.cursor_position(0, 11), (1, 5));
    }

    #[test]
    fn cursor_on_second_logical_line() {
        let l = lines("aa\nbbbb");
        let layout = layout(&l, 20);
        assert_eq!(layout.cursor_position(1, 3), (1, 3));
    }

    #[test]
    fn cursor_on_empty_line() {
        let l = lines("a\n\nb");
        let layout = layout(&l, 20);
        // Line 1 is empty; cursor col 0 maps to its (single) visual row col 0.
        let (vrow, vcol) = layout.cursor_position(1, 0);
        assert_eq!(layout.rows[vrow].logical_line, 1);
        assert_eq!(vcol, 0);
    }

    #[test]
    fn multibyte_row_str_slices_by_char() {
        let l = lines("héllo wörld");
        let layout = layout(&l, 7);
        // "héllo " (6 chars) fits at width 7? 11 chars total, width 7:
        // remaining 11 > 7, last whitespace within [0,7) is index 5 → break
        // after it. Row 0 = "héllo ".
        assert_eq!(row_str("héllo wörld", &layout.rows[0]), "héllo ");
        assert_eq!(row_str("héllo wörld", &layout.rows[1]), "wörld");
    }
}
