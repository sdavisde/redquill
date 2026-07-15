//! `nucleo-matcher` ranking glue for the fuzzy file finder (spec 06 Unit 1).
//! Pure — no I/O, no TUI types; [`rank`] is a plain function over
//! [`crate::search::files::FileCandidate`]s and a query string, unit-tested
//! directly.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use super::files::FileCandidate;

/// One ranked fuzzy match against the candidate at `index` (into the slice
/// [`rank`] was called with): a higher `score` is a better match, and
/// `positions` are the 0-based char indices within that candidate's path
/// that matched the query — for highlighting the match in the finder list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzyMatch {
    /// Index into the candidate slice `rank` ranked.
    pub index: usize,
    /// The match score; higher ranks better. Comparable only within one
    /// `rank` call (not a stable cross-query metric).
    pub score: u32,
    /// Deduplicated, ascending 0-based char positions within the path that
    /// matched the query.
    pub positions: Vec<u32>,
}

/// Smartcase, matching `crate::ui::search::smartcase_contains`'s convention
/// (vim's smartcase): a query containing any uppercase letter matches
/// case-sensitively; an all-lowercase query matches case-insensitively.
fn case_matching(query: &str) -> CaseMatching {
    if query.chars().any(char::is_uppercase) {
        CaseMatching::Respect
    } else {
        CaseMatching::Ignore
    }
}

/// Ranks `candidates` against `query` using a path-aware `nucleo-matcher`
/// configuration ([`Config::match_paths`]), returning matches sorted by
/// descending score, ties broken by ascending path for a deterministic order
/// (`nucleo-matcher` documents no tie-break of its own). An empty query
/// matches nothing — mirrors `crate::ui::search`'s "empty pattern never
/// matches" rule, so the finder shows an empty list rather than every
/// candidate unranked until the user types.
pub fn rank(candidates: &[FileCandidate], query: &str) -> Vec<FuzzyMatch> {
    if query.is_empty() {
        return Vec::new();
    }
    let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
    let pattern = Pattern::parse(query, case_matching(query), Normalization::Smart);

    let mut matches: Vec<FuzzyMatch> = candidates
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| {
            let mut buf = Vec::new();
            let haystack = Utf32Str::new(&candidate.path, &mut buf);
            let mut positions = Vec::new();
            let score = pattern.indices(haystack, &mut matcher, &mut positions)?;
            positions.sort_unstable();
            positions.dedup();
            Some(FuzzyMatch {
                index,
                score,
                positions,
            })
        })
        .collect();

    matches.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| candidates[a.index].path.cmp(&candidates[b.index].path))
    });
    matches
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidates(paths: &[&str]) -> Vec<FileCandidate> {
        paths
            .iter()
            .map(|p| FileCandidate {
                path: (*p).to_string(),
            })
            .collect()
    }

    #[test]
    fn empty_query_matches_nothing() {
        let c = candidates(&["src/main.rs"]);
        assert!(rank(&c, "").is_empty());
    }

    #[test]
    fn subsequence_query_matches_a_path() {
        let c = candidates(&["src/ui/file_finder.rs", "README.md"]);
        let matches = rank(&c, "finder");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].index, 0);
        assert!(!matches[0].positions.is_empty());
    }

    #[test]
    fn non_matching_query_yields_no_matches() {
        let c = candidates(&["src/main.rs"]);
        assert!(rank(&c, "zzzzz").is_empty());
    }

    #[test]
    fn lowercase_query_is_case_insensitive() {
        let c = candidates(&["src/Main.rs"]);
        let matches = rank(&c, "main");
        assert_eq!(matches.len(), 1);
    }

    #[test]
    fn uppercase_query_is_case_sensitive() {
        let c = candidates(&["src/main.rs", "src/Main.rs"]);
        let matches = rank(&c, "Main");
        let matched_paths: Vec<&str> = matches.iter().map(|m| c[m.index].path.as_str()).collect();
        assert_eq!(matched_paths, vec!["src/Main.rs"]);
    }

    #[test]
    fn positions_index_into_the_matched_path() {
        let c = candidates(&["abc"]);
        let matches = rank(&c, "abc");
        assert_eq!(matches.len(), 1);
        for &pos in &matches[0].positions {
            assert!((pos as usize) < c[0].path.chars().count());
        }
    }

    #[test]
    fn exact_match_ranks_above_a_looser_fuzzy_match() {
        let c = candidates(&["b/a/n/a/n/a.rs", "banana.rs"]);
        let matches = rank(&c, "banana");
        assert_eq!(matches.len(), 2);
        assert_eq!(
            c[matches[0].index].path, "banana.rs",
            "the contiguous match must outrank the scattered one"
        );
    }

    #[test]
    fn ties_break_by_ascending_path_for_determinism() {
        let c = candidates(&["b.rs", "a.rs"]);
        let matches = rank(&c, "rs");
        assert_eq!(matches.len(), 2);
        assert_eq!(c[matches[0].index].path, "a.rs");
        assert_eq!(c[matches[1].index].path, "b.rs");
    }

    #[test]
    fn rerank_on_a_narrower_query_can_drop_prior_matches() {
        let c = candidates(&["src/main.rs", "src/ui/mod.rs"]);
        let broad = rank(&c, "rs");
        assert_eq!(broad.len(), 2);
        let narrow = rank(&c, "main");
        assert_eq!(narrow.len(), 1);
        assert_eq!(c[narrow[0].index].path, "src/main.rs");
    }
}
