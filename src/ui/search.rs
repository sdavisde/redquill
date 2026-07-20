//! Search: smartcase substring matching over the diff pane's searchable
//! text (line content and hunk-header section text), and the
//! [`SearchState`] the App drives `/`/`n`/`N` navigation through.

use super::rows::Row;

/// Whether `pattern` smartcase-matches somewhere in `haystack`: a pattern
/// that is all-lowercase matches case-insensitively; a pattern containing
/// any uppercase letter matches case-sensitively (vim's `smartcase`). An
/// empty pattern never matches anything.
pub fn smartcase_contains(haystack: &str, pattern: &str) -> bool {
    if pattern.is_empty() {
        return false;
    }
    if pattern.chars().any(char::is_uppercase) {
        haystack.contains(pattern)
    } else {
        haystack.to_lowercase().contains(&pattern.to_lowercase())
    }
}

/// Row indices in `rows` whose searchable text smartcase-matches `pattern`:
/// a [`Row::Line`]'s content, or a [`Row::HunkHeader`]'s section text.
/// Other row kinds (file header, binary placeholder, annotation display and
/// border rows) are never matched.
pub fn find_matches(rows: &[Row], pattern: &str) -> Vec<usize> {
    rows.iter()
        .enumerate()
        .filter(|(_, row)| match row {
            Row::Line(l) => smartcase_contains(&l.content, pattern),
            Row::HunkHeader { text, .. } => smartcase_contains(text, pattern),
            _ => false,
        })
        .map(|(i, _)| i)
        .collect()
}

/// One active (or inactive) search session: the confirmed pattern and the
/// match row indices it currently produces against the selected file's
/// rows. `matches` is recomputed whenever rows rebuild (file switch,
/// annotation edit, refresh, ...), so the pattern survives all of those —
/// only a new confirmed search or an empty-buffer `Esc` replaces it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchState {
    /// The confirmed search pattern, if a search is active.
    pub pattern: Option<String>,
    /// Row indices matching `pattern` in the current file's rows, in
    /// ascending order.
    pub matches: Vec<usize>,
}

impl SearchState {
    /// Recomputes `matches` against `rows` for the current `pattern`
    /// (clearing `matches` if there is none).
    pub fn recompute(&mut self, rows: &[Row]) {
        self.matches = match &self.pattern {
            Some(pattern) => find_matches(rows, pattern),
            None => Vec::new(),
        };
    }

    /// The next match row at or after `cursor`, wrapping to the first match
    /// if none is found forward. `None` if there are no matches at all.
    pub fn next_from(&self, cursor: usize) -> Option<usize> {
        self.matches
            .iter()
            .find(|&&r| r >= cursor)
            .or_else(|| self.matches.first())
            .copied()
    }

    /// The next match strictly after `cursor`, wrapping to the first match.
    /// `None` if there are no matches at all.
    pub fn advance_from(&self, cursor: usize) -> Option<usize> {
        self.matches
            .iter()
            .find(|&&r| r > cursor)
            .or_else(|| self.matches.first())
            .copied()
    }

    /// The previous match strictly before `cursor`, wrapping to the last
    /// match. `None` if there are no matches at all.
    pub fn retreat_from(&self, cursor: usize) -> Option<usize> {
        self.matches
            .iter()
            .rev()
            .find(|&&r| r < cursor)
            .or_else(|| self.matches.last())
            .copied()
    }

    /// The 1-based position of `row` within `matches`, for the `match k/N`
    /// footer echo. `None` if `row` isn't a match.
    pub fn position_of(&self, row: usize) -> Option<usize> {
        self.matches.iter().position(|&r| r == row).map(|i| i + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::super::rows::LineRow;
    use super::*;
    use crate::diff::LineOrigin;

    #[test]
    fn find_matches_skips_annotation_and_border_rows() {
        let rows = vec![
            Row::Line(LineRow {
                hunk_index: 0,
                old_line: None,
                new_line: Some(1),
                origin: LineOrigin::Context,
                content: "hello".to_string(),
                word_spans: None,
                no_newline: false,
                annotated: false,
                thread: false,
                syntax_spans: None,
            }),
            Row::AnnotationBorder { top: true },
            Row::Annotation {
                id: 0,
                text: "hello".to_string(),
                classification: None,
            },
            Row::AnnotationBorder { top: false },
        ];
        // Both the annotation body (which literally contains "hello") and
        // its border rows must be excluded — only the Line row matches.
        assert_eq!(find_matches(&rows, "hello"), vec![0]);
    }

    #[test]
    fn smartcase_lowercase_pattern_is_case_insensitive() {
        assert!(smartcase_contains("Hello World", "hello"));
        assert!(smartcase_contains("HELLO", "hello"));
    }

    #[test]
    fn smartcase_uppercase_pattern_is_case_sensitive() {
        assert!(smartcase_contains("Hello World", "Hello"));
        assert!(!smartcase_contains("hello world", "Hello"));
    }

    #[test]
    fn empty_pattern_never_matches() {
        assert!(!smartcase_contains("anything", ""));
    }

    #[test]
    fn next_from_wraps_to_first_when_no_match_forward() {
        let mut state = SearchState {
            pattern: Some("x".to_string()),
            matches: vec![2, 5, 9],
        };
        assert_eq!(state.next_from(6), Some(9));
        assert_eq!(state.next_from(10), Some(2)); // wraps
        assert_eq!(state.next_from(0), Some(2));
        state.matches.clear();
        assert_eq!(state.next_from(0), None);
    }

    #[test]
    fn advance_from_is_strictly_after_and_wraps() {
        let state = SearchState {
            pattern: Some("x".to_string()),
            matches: vec![2, 5, 9],
        };
        assert_eq!(state.advance_from(2), Some(5));
        assert_eq!(state.advance_from(9), Some(2)); // wraps forward
        assert_eq!(state.advance_from(0), Some(2));
    }

    #[test]
    fn retreat_from_is_strictly_before_and_wraps() {
        let state = SearchState {
            pattern: Some("x".to_string()),
            matches: vec![2, 5, 9],
        };
        assert_eq!(state.retreat_from(9), Some(5));
        assert_eq!(state.retreat_from(2), Some(9)); // wraps backward
        assert_eq!(state.retreat_from(100), Some(9));
    }

    #[test]
    fn position_of_is_one_based() {
        let state = SearchState {
            pattern: Some("x".to_string()),
            matches: vec![2, 5, 9],
        };
        assert_eq!(state.position_of(2), Some(1));
        assert_eq!(state.position_of(9), Some(3));
        assert_eq!(state.position_of(3), None);
    }
}
