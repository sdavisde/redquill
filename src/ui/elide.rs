//! Left-truncation for the path text in changed-file rows.
//!
//! Every changed-file surface ends in a fixed-width cluster — the `+A -R`
//! counts plus the staged/review/change-kind glyphs — that has to survive a
//! pane too narrow to hold the whole row. The path is what gives, and it
//! gives from the *left*: the basename identifies the file, while the leading
//! directories are the part a reader can usually infer. `…src/ui/rows.rs`
//! beats `src/ui/anno…` with the counts pushed off the edge.
//!
//! Widths are counted in `char`s, matching the rest of the row-building code
//! (`stat_display`, `diff_view::gutter_number`).

/// Marker standing in for the dropped leading text. One cell wide.
const ELLIPSIS: char = '\u{2026}';

/// Fits `text` into `max_width` cells by dropping characters from the left and
/// prefixing [`ELLIPSIS`]. Text that already fits is returned unchanged; a
/// `max_width` of zero yields an empty string.
pub(super) fn elide_left(text: &str, max_width: usize) -> String {
    let len = text.chars().count();
    if len <= max_width {
        return text.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    let mut out = String::with_capacity(max_width);
    out.push(ELLIPSIS);
    out.extend(text.chars().skip(len - (max_width - 1)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elides_from_the_left_within_the_given_width() {
        // Each case: (input, max_width, expected). The defect these catch is a
        // row that overruns its pane — anything wider than `max_width` shoves
        // the counts and status glyphs off the right edge.
        for (text, max, expected) in [
            ("src/ui/rows.rs", 20, "src/ui/rows.rs"),
            ("src/ui/rows.rs", 14, "src/ui/rows.rs"),
            ("src/ui/rows.rs", 13, "\u{2026}c/ui/rows.rs"),
            ("src/ui/rows.rs", 10, "\u{2026}i/rows.rs"),
            ("src/ui/rows.rs", 1, "\u{2026}"),
            ("src/ui/rows.rs", 0, ""),
        ] {
            let got = elide_left(text, max);
            assert_eq!(got, expected, "elide_left({text:?}, {max})");
            assert!(got.chars().count() <= max, "elide_left({text:?}, {max})");
        }
    }
}
