//! Word-level intra-line diff: tokenizes a pair of lines, computes a
//! longest-common-subsequence alignment over the tokens, and reports which
//! byte ranges changed. Also pairs up removed/added lines within a hunk so
//! the UI knows which lines to run word-level highlighting on.

use std::ops::Range;

use super::hunk::Hunk;
use super::line::LineOrigin;

/// Above this many tokens on either side, word-level diffing is skipped in
/// favor of marking the whole line changed — an O(n*m) LCS over two 5k-token
/// lines would blow the "instant feel" budget for no real UI benefit.
const TOKEN_LIMIT: usize = 300;

/// One contiguous run of a line marked either changed or unchanged, given as
/// a byte range into the original `&str` it was computed from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordSpan {
    /// Byte range into the input string this span covers.
    pub text_range: Range<usize>,
    /// Whether this span differs from the other side.
    pub changed: bool,
}

/// Which class of token a character belongs to, for tokenization purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharClass {
    Word,
    Space,
    Other,
}

fn classify(c: char) -> CharClass {
    if c == '_' || c.is_alphanumeric() {
        CharClass::Word
    } else if c.is_whitespace() {
        CharClass::Space
    } else {
        CharClass::Other
    }
}

/// Tokenizes `s` into byte ranges: runs of alphanumeric+underscore, runs of
/// whitespace, and single other characters.
fn tokenize(s: &str) -> Vec<Range<usize>> {
    let mut tokens = Vec::new();
    let mut chars = s.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        let class = classify(c);
        let mut end = start + c.len_utf8();
        if class != CharClass::Other {
            while let Some(&(idx, next_c)) = chars.peek() {
                if classify(next_c) == class {
                    end = idx + next_c.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
        }
        tokens.push(start..end);
    }
    tokens
}

/// Merges adjacent tokens sharing the same `changed` flag into [`WordSpan`]s.
fn spans_from_flags(tokens: &[Range<usize>], flags: &[bool]) -> Vec<WordSpan> {
    let mut spans = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let changed = flags[i];
        let start = tokens[i].start;
        let mut end = tokens[i].end;
        let mut j = i + 1;
        while j < tokens.len() && flags[j] == changed {
            end = tokens[j].end;
            j += 1;
        }
        spans.push(WordSpan {
            text_range: start..end,
            changed,
        });
        i = j;
    }
    spans
}

/// Computes word-level diff spans between two lines.
///
/// Tokenizes each side into runs of word characters, runs of whitespace, and
/// single other characters, then aligns tokens via an LCS over token text.
/// Tokens not part of the common subsequence are marked `changed`; adjacent
/// spans sharing a `changed` flag are merged. If either side has more than
/// ~300 tokens, the whole line is returned as a single changed span on each
/// side rather than paying for an O(n*m) LCS.
pub fn word_diff(old: &str, new: &str) -> (Vec<WordSpan>, Vec<WordSpan>) {
    let old_tokens = tokenize(old);
    let new_tokens = tokenize(new);

    if old_tokens.len() > TOKEN_LIMIT || new_tokens.len() > TOKEN_LIMIT {
        let whole = |s: &str| {
            if s.is_empty() {
                Vec::new()
            } else {
                vec![WordSpan {
                    text_range: 0..s.len(),
                    changed: true,
                }]
            }
        };
        return (whole(old), whole(new));
    }

    let n = old_tokens.len();
    let m = new_tokens.len();
    let text_eq = |i: usize, j: usize| old[old_tokens[i].clone()] == new[new_tokens[j].clone()];

    // dp[i][j] = length of the LCS of old_tokens[i..] and new_tokens[j..].
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if text_eq(i, j) {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    let mut old_flags = vec![true; n];
    let mut new_flags = vec![true; m];
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if text_eq(i, j) {
            old_flags[i] = false;
            new_flags[j] = false;
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }

    (
        spans_from_flags(&old_tokens, &old_flags),
        spans_from_flags(&new_tokens, &new_flags),
    )
}

/// Within a hunk, pairs the i-th [`LineOrigin::Removed`] line of a
/// contiguous removed-run with the i-th [`LineOrigin::Added`] line of the
/// immediately following added-run (a classic "these lines were probably
/// edited in place" heuristic). Returns `(removed_index, added_index)` pairs
/// as indices into `hunk.lines`. Lines with no counterpart (an unequal-length
/// run, or a removed-run not immediately followed by an added-run) are left
/// unpaired.
pub fn pair_hunk_lines(hunk: &Hunk) -> Vec<(usize, usize)> {
    let lines = &hunk.lines;
    let mut pairs = Vec::new();
    let mut idx = 0;
    while idx < lines.len() {
        if lines[idx].origin != LineOrigin::Removed {
            idx += 1;
            continue;
        }

        let removed_start = idx;
        let mut removed_end = idx;
        while removed_end + 1 < lines.len() && lines[removed_end + 1].origin == LineOrigin::Removed
        {
            removed_end += 1;
        }

        let added_start = removed_end + 1;
        if added_start < lines.len() && lines[added_start].origin == LineOrigin::Added {
            let mut added_end = added_start;
            while added_end + 1 < lines.len() && lines[added_end + 1].origin == LineOrigin::Added {
                added_end += 1;
            }

            let removed_count = removed_end - removed_start + 1;
            let added_count = added_end - added_start + 1;
            let pair_count = removed_count.min(added_count);
            for k in 0..pair_count {
                pairs.push((removed_start + k, added_start + k));
            }
            idx = added_end + 1;
        } else {
            idx = removed_end + 1;
        }
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{DiffLine, LineOrigin};

    fn text<'a>(s: &'a str, span: &WordSpan) -> &'a str {
        &s[span.text_range.clone()]
    }

    #[test]
    fn identical_lines_produce_one_unchanged_span_each() {
        let (old, new) = word_diff("foo bar", "foo bar");
        assert_eq!(old.len(), 1);
        assert_eq!(new.len(), 1);
        assert!(!old[0].changed);
        assert!(!new[0].changed);
        assert_eq!(text("foo bar", &old[0]), "foo bar");
        assert_eq!(text("foo bar", &new[0]), "foo bar");
    }

    #[test]
    fn fully_different_lines_are_one_changed_span_each() {
        let (old, new) = word_diff("abc", "xyz");
        assert_eq!(old.len(), 1);
        assert_eq!(new.len(), 1);
        assert!(old[0].changed);
        assert!(new[0].changed);
        assert_eq!(text("abc", &old[0]), "abc");
        assert_eq!(text("xyz", &new[0]), "xyz");
    }

    #[test]
    fn single_word_change_isolates_the_changed_token() {
        let old_line = "let x = foo;";
        let new_line = "let x = bar;";
        let (old, new) = word_diff(old_line, new_line);

        assert_eq!(old.len(), 3);
        assert!(!old[0].changed);
        assert_eq!(text(old_line, &old[0]), "let x = ");
        assert!(old[1].changed);
        assert_eq!(text(old_line, &old[1]), "foo");
        assert!(!old[2].changed);
        assert_eq!(text(old_line, &old[2]), ";");

        assert_eq!(new.len(), 3);
        assert!(!new[0].changed);
        assert_eq!(text(new_line, &new[0]), "let x = ");
        assert!(new[1].changed);
        assert_eq!(text(new_line, &new[1]), "bar");
        assert!(!new[2].changed);
        assert_eq!(text(new_line, &new[2]), ";");
    }

    #[test]
    fn whitespace_only_change_is_detected() {
        let old_line = "a  b"; // two spaces
        let new_line = "a b"; // one space
        let (old, new) = word_diff(old_line, new_line);

        assert_eq!(old.len(), 3);
        assert!(!old[0].changed);
        assert_eq!(text(old_line, &old[0]), "a");
        assert!(old[1].changed);
        assert_eq!(text(old_line, &old[1]), "  ");
        assert!(!old[2].changed);
        assert_eq!(text(old_line, &old[2]), "b");

        assert_eq!(new.len(), 3);
        assert!(!new[0].changed);
        assert!(new[1].changed);
        assert_eq!(text(new_line, &new[1]), " ");
        assert!(!new[2].changed);
    }

    #[test]
    fn token_limit_guardrail_marks_whole_line_changed() {
        let long_old = (0..400)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        let short_new = "short";
        let (old, new) = word_diff(&long_old, short_new);

        assert_eq!(old.len(), 1);
        assert!(old[0].changed);
        assert_eq!(old[0].text_range, 0..long_old.len());

        assert_eq!(new.len(), 1);
        assert!(new[0].changed);
        assert_eq!(text(short_new, &new[0]), "short");
    }

    #[test]
    fn token_limit_guardrail_handles_empty_side() {
        let long_old = (0..400)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        let (old, new) = word_diff(&long_old, "");
        assert_eq!(old.len(), 1);
        assert!(new.is_empty());
    }

    #[test]
    fn adjacent_same_flag_tokens_merge_into_one_span() {
        // "foo bar baz" vs "foo bar baz" but with "bar" swapped for "qux":
        // the two unchanged runs on either side of the change should each
        // merge into a single span rather than one-per-token.
        let old_line = "foo bar baz";
        let new_line = "foo qux baz";
        let (old, new) = word_diff(old_line, new_line);
        // "foo", " ", then changed "bar", then " ", "baz" merge to 3 spans.
        assert_eq!(old.len(), 3);
        assert_eq!(text(old_line, &old[0]), "foo ");
        assert_eq!(text(old_line, &old[1]), "bar");
        assert_eq!(text(old_line, &old[2]), " baz");
        assert_eq!(new.len(), 3);
        assert_eq!(text(new_line, &new[1]), "qux");
    }

    fn diff_line(origin: LineOrigin, content: &str) -> DiffLine {
        DiffLine {
            origin,
            old_line: None,
            new_line: None,
            content: content.to_string(),
            no_newline: false,
        }
    }

    fn hunk_with(lines: Vec<DiffLine>) -> Hunk {
        Hunk {
            old_start: 1,
            old_count: lines.len() as u32,
            new_start: 1,
            new_count: lines.len() as u32,
            section: None,
            lines,
        }
    }

    #[test]
    fn pairs_equal_length_removed_and_added_runs() {
        let hunk = hunk_with(vec![
            diff_line(LineOrigin::Context, "ctx"),
            diff_line(LineOrigin::Removed, "r0"),
            diff_line(LineOrigin::Removed, "r1"),
            diff_line(LineOrigin::Added, "a0"),
            diff_line(LineOrigin::Added, "a1"),
            diff_line(LineOrigin::Context, "ctx2"),
        ]);
        let pairs = pair_hunk_lines(&hunk);
        assert_eq!(pairs, vec![(1, 3), (2, 4)]);
    }

    #[test]
    fn pairs_only_up_to_shorter_run_length() {
        let hunk = hunk_with(vec![
            diff_line(LineOrigin::Removed, "r0"),
            diff_line(LineOrigin::Removed, "r1"),
            diff_line(LineOrigin::Removed, "r2"),
            diff_line(LineOrigin::Added, "a0"),
        ]);
        let pairs = pair_hunk_lines(&hunk);
        assert_eq!(pairs, vec![(0, 3)]);
    }

    #[test]
    fn removed_run_not_followed_by_added_is_unpaired() {
        let hunk = hunk_with(vec![
            diff_line(LineOrigin::Removed, "r0"),
            diff_line(LineOrigin::Context, "ctx"),
            diff_line(LineOrigin::Added, "a0"),
        ]);
        let pairs = pair_hunk_lines(&hunk);
        assert!(pairs.is_empty());
    }

    #[test]
    fn context_only_hunk_has_no_pairs() {
        let hunk = hunk_with(vec![
            diff_line(LineOrigin::Context, "a"),
            diff_line(LineOrigin::Context, "b"),
        ]);
        assert!(pair_hunk_lines(&hunk).is_empty());
    }
}
