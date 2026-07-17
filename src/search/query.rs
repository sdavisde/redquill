//! Search query model: [`SearchQuery`] plus `grep-regex`
//! matcher construction (regex by default, smartcase, whole-word, literal).
//! Pure — no I/O, no TUI types; [`crate::search::engine`] uses
//! [`build_matcher`] to compile the matcher it hands to `grep-searcher`.

use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use thiserror::Error;

/// Case-sensitivity behavior for a [`SearchQuery`] — the three states the
/// Project Search view's case toggle (`Alt-c`) cycles through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CaseMode {
    /// Case-insensitive unless the pattern contains an uppercase letter —
    /// vim's smartcase, matching `crate::ui::search::smartcase_contains`'s
    /// one-line rule (a pattern is case-sensitive iff
    /// `pattern.chars().any(char::is_uppercase)`) so Project Search and the
    /// diff-view's in-buffer search agree on what "smart" means. The rule is
    /// replicated in [`smart_case_is_insensitive`] rather than imported:
    /// `search/` may not depend on `ui/` (layering rule) — see
    /// `smart_case_matches_ui_convention` below for the correspondence.
    #[default]
    Smart,
    /// Always case-sensitive, regardless of the pattern's casing.
    Sensitive,
    /// Always case-insensitive, regardless of the pattern's casing.
    Insensitive,
}

/// A user-entered search query: pattern text plus the toggle states the
/// Project Search view exposes and cycles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchQuery {
    /// The pattern text — a regex unless `literal` is set.
    pub pattern: String,
    /// Case-sensitivity behavior.
    pub case: CaseMode,
    /// Whole-word matching (`grep-regex`'s `word` option): a match must not
    /// be adjacent to another word character.
    pub whole_word: bool,
    /// Treat `pattern` as a fixed string rather than a regex (metacharacters
    /// matched literally).
    pub literal: bool,
}

/// Errors constructing a [`SearchQuery`]'s matcher, surfaced to the caller
/// (e.g. the search view's inline error line) rather than panicking.
/// `grep-regex` is a finite-automata engine (no backreferences/lookaround),
/// so there's no catastrophic-backtracking class of error to guard against —
/// only pattern syntax/size errors reach here.
#[derive(Debug, Error)]
pub enum SearchError {
    /// `grep-regex` could not compile `pattern` (syntax error or a
    /// configured size limit exceeded).
    #[error("invalid search pattern: {0}")]
    InvalidPattern(String),
}

impl From<grep_regex::Error> for SearchError {
    fn from(err: grep_regex::Error) -> SearchError {
        SearchError::InvalidPattern(err.to_string())
    }
}

/// Whether a [`CaseMode::Smart`] query should match case-insensitively:
/// true unless `pattern` contains an uppercase letter. This is the one-line
/// rule `crate::ui::search::smartcase_contains` also implements — kept in
/// sync by the `smart_case_matches_ui_convention` test below rather than by
/// a shared dependency (search/ may not import ui/).
fn smart_case_is_insensitive(pattern: &str) -> bool {
    !pattern.chars().any(char::is_uppercase)
}

/// Builds a `grep-regex` [`RegexMatcher`] for `query`: resolves [`CaseMode`]
/// (smartcase via [`smart_case_is_insensitive`]), and applies whole-word and
/// literal (fixed-string) settings. Returns [`SearchError::InvalidPattern`]
/// for a pattern `grep-regex` can't compile — never panics.
pub fn build_matcher(query: &SearchQuery) -> Result<RegexMatcher, SearchError> {
    let case_insensitive = match query.case {
        CaseMode::Smart => smart_case_is_insensitive(&query.pattern),
        CaseMode::Sensitive => false,
        CaseMode::Insensitive => true,
    };
    let mut builder = RegexMatcherBuilder::new();
    builder
        .case_insensitive(case_insensitive)
        .word(query.whole_word)
        .fixed_strings(query.literal);
    builder.build(&query.pattern).map_err(SearchError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use grep_matcher::Matcher;

    fn query(pattern: &str) -> SearchQuery {
        SearchQuery {
            pattern: pattern.to_string(),
            case: CaseMode::Smart,
            whole_word: false,
            literal: false,
        }
    }

    fn matches(pattern_query: &SearchQuery, haystack: &str) -> bool {
        let matcher = build_matcher(pattern_query).expect("pattern compiles");
        matcher
            .is_match(haystack.as_bytes())
            .expect("matcher runs without error")
    }

    #[test]
    fn regex_metacharacters_are_interpreted_by_default() {
        let q = query(r"fn\s+main");
        assert!(matches(&q, "fn   main() {}"));
        assert!(!matches(&query(r"fn\s+main\("), "function main("));
    }

    #[test]
    fn smart_case_lowercase_pattern_is_case_insensitive() {
        let q = query("hello");
        assert!(matches(&q, "Hello World"));
        assert!(matches(&q, "HELLO"));
    }

    #[test]
    fn smart_case_uppercase_pattern_is_case_sensitive() {
        let q = query("Hello");
        assert!(matches(&q, "Hello World"));
        assert!(!matches(&q, "hello world"));
    }

    #[test]
    fn smart_case_matches_ui_convention() {
        // Same one-line rule as `crate::ui::search::smartcase_contains`:
        // insensitive iff no uppercase letter in the pattern.
        assert!(smart_case_is_insensitive("hello"));
        assert!(!smart_case_is_insensitive("Hello"));
        assert!(smart_case_is_insensitive("hello_world_123"));
    }

    #[test]
    fn case_sensitive_mode_ignores_pattern_casing() {
        let q = SearchQuery {
            pattern: "hello".to_string(),
            case: CaseMode::Sensitive,
            whole_word: false,
            literal: false,
        };
        assert!(matches(&q, "hello world"));
        assert!(!matches(&q, "HELLO WORLD"));
    }

    #[test]
    fn case_insensitive_mode_ignores_pattern_casing() {
        let q = SearchQuery {
            pattern: "Hello".to_string(),
            case: CaseMode::Insensitive,
            whole_word: false,
            literal: false,
        };
        assert!(matches(&q, "hello world"));
        assert!(matches(&q, "HELLO WORLD"));
    }

    #[test]
    fn whole_word_excludes_substring_matches() {
        let q = SearchQuery {
            pattern: "cat".to_string(),
            case: CaseMode::Smart,
            whole_word: true,
            literal: false,
        };
        assert!(matches(&q, "the cat sat"));
        assert!(!matches(&q, "concatenate"));
    }

    #[test]
    fn whole_word_off_matches_substrings() {
        let q = SearchQuery {
            pattern: "cat".to_string(),
            case: CaseMode::Smart,
            whole_word: false,
            literal: false,
        };
        assert!(matches(&q, "concatenate"));
    }

    #[test]
    fn literal_mode_treats_metacharacters_as_text() {
        let q = SearchQuery {
            pattern: "a.b".to_string(),
            case: CaseMode::Smart,
            whole_word: false,
            literal: true,
        };
        assert!(matches(&q, "x a.b y"));
        assert!(!matches(&q, "x axb y"));
    }

    #[test]
    fn non_literal_mode_treats_dot_as_wildcard() {
        let q = SearchQuery {
            pattern: "a.b".to_string(),
            case: CaseMode::Smart,
            whole_word: false,
            literal: false,
        };
        assert!(matches(&q, "x axb y"));
    }

    #[test]
    fn invalid_regex_is_a_typed_error_not_a_panic() {
        let q = query("(unclosed");
        let err = build_matcher(&q).expect_err("unbalanced parens must fail to compile");
        assert!(matches!(err, SearchError::InvalidPattern(_)));
        assert!(err.to_string().contains("invalid search pattern"));
    }
}
