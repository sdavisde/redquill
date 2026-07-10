//! Intra-line word diff seam.
//!
//! `word_diff_spans` is the ONE place the intra-line diff algorithm lives
//! (spec §5); `attach_word_spans` pairs removed/added lines within each
//! hunk's contiguous change runs and calls it exactly once per pair. Do not
//! add other span-computing call sites.
//!
//! Algorithm (spec §9 Open-Question default): tokenize on whitespace and
//! punctuation runs — identifiers (alnum + `_`) stay whole, `foo.bar()`
//! breaks at `.`, `(`, `)` individually — then run an LCS over the token
//! sequences. Unmatched (changed) tokens become `Range<usize>` spans, in
//! **char** offsets into `Line.content` (spec §9: lets `ui/` slice without
//! UTF-8 boundary math).

use super::model::{DiffFile, Line, LineKind};

/// Above this many tokens on either side, the O(n*m) LCS table would be
/// pathological for a single line. Degrade to "whole line changed" rather
/// than risk quadratic blowup on a hostile/huge line (perf target: instant
/// feel on a 5k-line diff — see spec Repo-specific execution context).
const MAX_LINE_TOKENS: usize = 400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenClass {
    Word,
    Whitespace,
    Punct,
}

fn classify(c: char) -> TokenClass {
    if c.is_whitespace() {
        TokenClass::Whitespace
    } else if c.is_alphanumeric() || c == '_' {
        TokenClass::Word
    } else {
        TokenClass::Punct
    }
}

/// Splits `chars` into maximal token runs as char-index ranges. Word and
/// whitespace runs merge across any chars of their class; a punctuation run
/// only merges while the *same* character repeats, so `foo.bar()` yields
/// `foo`, `.`, `bar`, `(`, `)` as five separate tokens.
fn tokenize(chars: &[char]) -> Vec<std::ops::Range<usize>> {
    let mut spans = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let class = classify(chars[i]);
        let start = i;
        let mut j = i + 1;
        while j < chars.len() {
            let same_run = match class {
                TokenClass::Word => classify(chars[j]) == TokenClass::Word,
                TokenClass::Whitespace => classify(chars[j]) == TokenClass::Whitespace,
                TokenClass::Punct => chars[j] == chars[start],
            };
            if !same_run {
                break;
            }
            j += 1;
        }
        spans.push(start..j);
        i = j;
    }
    spans
}

/// The ONLY place the intra-line algorithm lives; swap the body freely.
/// Returns `(old_spans, new_spans)` as char ranges into `old` / `new`
/// respectively, covering the tokens that differ (shared prefix/suffix
/// tokens are excluded via the LCS match).
// FR-diff-word-1
// FR-diff-word-2
pub fn word_diff_spans(
    old: &str,
    new: &str,
) -> (Vec<std::ops::Range<usize>>, Vec<std::ops::Range<usize>>) {
    let old_chars: Vec<char> = old.chars().collect();
    let new_chars: Vec<char> = new.chars().collect();

    let old_tokens = tokenize(&old_chars);
    let new_tokens = tokenize(&new_chars);

    // Guard against pathological quadratic blowup on huge/hostile lines:
    // degrade to "whole line changed" instead of building an O(n*m) table.
    if old_tokens.len() > MAX_LINE_TOKENS || new_tokens.len() > MAX_LINE_TOKENS {
        let mut old_span = Vec::new();
        if !old_chars.is_empty() {
            old_span.push(0..old_chars.len());
        }
        let mut new_span = Vec::new();
        if !new_chars.is_empty() {
            new_span.push(0..new_chars.len());
        }
        return (old_span, new_span);
    }

    let token_eq = |a: &std::ops::Range<usize>, b: &std::ops::Range<usize>| -> bool {
        old_chars[a.clone()] == new_chars[b.clone()]
    };

    // Classic LCS DP table over token indices.
    let n = old_tokens.len();
    let m = new_tokens.len();
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if token_eq(&old_tokens[i], &new_tokens[j]) {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    // Backtrack to find which old/new token indices are part of the LCS
    // (i.e. unchanged); everything else is a changed token.
    let mut old_matched = vec![false; n];
    let mut new_matched = vec![false; m];
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if token_eq(&old_tokens[i], &new_tokens[j]) && dp[i][j] == dp[i + 1][j + 1] + 1 {
            old_matched[i] = true;
            new_matched[j] = true;
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }

    (
        merged_changed_spans(&old_tokens, &old_matched),
        merged_changed_spans(&new_tokens, &new_matched),
    )
}

/// Collects the char ranges of unmatched tokens, merging adjacent unmatched
/// tokens (no gap between them) into a single contiguous span so a run of
/// several changed tokens reads as one span rather than many.
fn merged_changed_spans(
    tokens: &[std::ops::Range<usize>],
    matched: &[bool],
) -> Vec<std::ops::Range<usize>> {
    let mut spans: Vec<std::ops::Range<usize>> = Vec::new();
    for (tok, &is_matched) in tokens.iter().zip(matched.iter()) {
        if is_matched {
            continue;
        }
        match spans.last_mut() {
            Some(last) if last.end == tok.start => last.end = tok.end,
            _ => spans.push(tok.clone()),
        }
    }
    spans
}

/// Pairs removed/added lines positionally within each contiguous change run
/// and stores `word_diff_spans` results onto both lines of each pair.
///
/// A "contiguous change run" is a maximal slice of non-`Context` lines
/// (git always emits a run's removed lines before its added lines). Within
/// a run, the i-th removed line pairs with the i-th added line; any excess
/// `|N-M|` lines are left unpaired. Context lines and unpaired lines keep
/// their default-empty `changed_spans`.
// FR-diff-word-3
pub fn attach_word_spans(file: &mut DiffFile) {
    for hunk in &mut file.hunks {
        let lines = &mut hunk.lines;
        let mut i = 0;
        while i < lines.len() {
            if lines[i].kind == LineKind::Context {
                i += 1;
                continue;
            }
            let start = i;
            while i < lines.len() && lines[i].kind != LineKind::Context {
                i += 1;
            }
            let end = i;
            pair_change_run(lines, start, end);
        }
    }
}

/// Positionally pairs removed/added lines within `lines[start..end]` (a
/// single contiguous change run) and attaches word-diff spans to each pair.
fn pair_change_run(lines: &mut [Line], start: usize, end: usize) {
    let removed_idxs: Vec<usize> = (start..end)
        .filter(|&idx| lines[idx].kind == LineKind::Removed)
        .collect();
    let added_idxs: Vec<usize> = (start..end)
        .filter(|&idx| lines[idx].kind == LineKind::Added)
        .collect();

    let pair_count = removed_idxs.len().min(added_idxs.len());
    for k in 0..pair_count {
        let r_idx = removed_idxs[k];
        let a_idx = added_idxs[k];
        let (old_spans, new_spans) = word_diff_spans(&lines[r_idx].content, &lines[a_idx].content);
        lines[r_idx].changed_spans = old_spans;
        lines[a_idx].changed_spans = new_spans;
    }
    // Excess unpaired lines (|N-M|) simply keep their default-empty
    // `changed_spans` — nothing to do here.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::model::{ChangeStatus, DiffFile, Hunk};

    fn line(kind: LineKind, content: &str) -> Line {
        Line {
            kind,
            old_lineno: None,
            new_lineno: None,
            content: content.to_string(),
            no_newline: false,
            changed_spans: Vec::new(),
        }
    }

    fn file_with_lines(lines: Vec<Line>) -> DiffFile {
        DiffFile {
            path: "f.rs".to_string(),
            old_path: None,
            status: ChangeStatus::Modified,
            mode_change: None,
            is_binary: false,
            hunks: vec![Hunk {
                old_start: 1,
                old_count: lines.len() as u32,
                new_start: 1,
                new_count: lines.len() as u32,
                section: None,
                lines,
            }],
        }
    }

    // --- 2.1: word_diff_spans single-span case (FR-diff-word-1/2) --------

    #[test]
    fn foo_to_bar_single_span_excludes_shared_prefix_suffix() {
        let (old_spans, new_spans) = word_diff_spans("let key = foo;", "let key = bar;");
        // "let key = " is 10 chars; "foo"/"bar" occupy 10..13; ";" is shared
        // suffix and excluded.
        assert_eq!(old_spans, vec![10..13]);
        assert_eq!(new_spans, vec![10..13]);
    }

    #[test]
    fn identical_strings_yield_no_spans() {
        let (old_spans, new_spans) = word_diff_spans("same line", "same line");
        assert!(old_spans.is_empty());
        assert!(new_spans.is_empty());
    }

    #[test]
    fn punctuation_breaks_tokens_individually() {
        // "foo.bar()" -> "foo.baz()": only "bar"->"baz" should differ; the
        // "." "(" ")" punctuation tokens are shared and excluded.
        let (old_spans, new_spans) = word_diff_spans("foo.bar()", "foo.baz()");
        assert_eq!(old_spans, vec![4..7]); // "bar"
        assert_eq!(new_spans, vec![4..7]); // "baz"
    }

    // --- 2.1: pairing rule / unpaired & identical stay empty (FR-diff-word-3) ---

    #[test]
    fn lone_added_line_has_empty_spans() {
        let mut file = file_with_lines(vec![line(LineKind::Added, "new stuff")]);
        attach_word_spans(&mut file);
        assert!(file.hunks[0].lines[0].changed_spans.is_empty());
    }

    #[test]
    fn lone_removed_line_has_empty_spans() {
        let mut file = file_with_lines(vec![line(LineKind::Removed, "old stuff")]);
        attach_word_spans(&mut file);
        assert!(file.hunks[0].lines[0].changed_spans.is_empty());
    }

    #[test]
    fn context_line_has_empty_spans() {
        let mut file = file_with_lines(vec![line(LineKind::Context, "unchanged")]);
        attach_word_spans(&mut file);
        assert!(file.hunks[0].lines[0].changed_spans.is_empty());
    }

    #[test]
    fn identical_removed_added_pair_has_empty_spans() {
        let mut file = file_with_lines(vec![
            line(LineKind::Removed, "same"),
            line(LineKind::Added, "same"),
        ]);
        attach_word_spans(&mut file);
        assert!(file.hunks[0].lines[0].changed_spans.is_empty());
        assert!(file.hunks[0].lines[1].changed_spans.is_empty());
    }

    #[test]
    fn excess_unpaired_lines_in_change_run_stay_empty() {
        // git emits N=2 removed then M=1 added: 0th removed pairs with the
        // lone added; the 1st removed is excess and stays unpaired (empty).
        let mut file = file_with_lines(vec![
            line(LineKind::Removed, "let key = foo;"),
            line(LineKind::Removed, "extra removed line"),
            line(LineKind::Added, "let key = bar;"),
        ]);
        attach_word_spans(&mut file);
        let lines = &file.hunks[0].lines;
        assert_eq!(lines[0].changed_spans, vec![10..13]); // foo
        assert!(
            lines[1].changed_spans.is_empty(),
            "excess removed line must stay unpaired"
        );
        assert_eq!(lines[2].changed_spans, vec![10..13]); // bar
    }

    #[test]
    fn excess_unpaired_added_lines_stay_empty() {
        // N=1 removed then M=2 added: 0th added pairs with the lone removed;
        // the 1st added is excess and stays unpaired (empty).
        let mut file = file_with_lines(vec![
            line(LineKind::Removed, "let key = foo;"),
            line(LineKind::Added, "let key = bar;"),
            line(LineKind::Added, "extra added line"),
        ]);
        attach_word_spans(&mut file);
        let lines = &file.hunks[0].lines;
        assert_eq!(lines[0].changed_spans, vec![10..13]); // foo
        assert_eq!(lines[1].changed_spans, vec![10..13]); // bar
        assert!(
            lines[2].changed_spans.is_empty(),
            "excess added line must stay unpaired"
        );
    }

    // --- 2.1/proof: word_diff_spans is exercised only through the single
    // seam, attach_word_spans (FR-diff-word-1 proof) -----------------------

    #[test]
    fn seam_exercised_only_through_attach_word_spans() {
        // This test never calls `word_diff_spans` directly — it only drives
        // the public seam `attach_word_spans`, proving callers only need
        // that one entry point. If the algorithm body were swapped for a
        // different implementation, this test's expectations (paired lines
        // get non-empty spans, context/unpaired stay empty) would still
        // hold as long as the seam's contract holds.
        let mut file = file_with_lines(vec![
            line(LineKind::Context, "unrelated context"),
            line(LineKind::Removed, "let key = foo;"),
            line(LineKind::Added, "let key = bar;"),
            line(LineKind::Context, "trailing context"),
        ]);
        attach_word_spans(&mut file);
        let lines = &file.hunks[0].lines;
        assert!(lines[0].changed_spans.is_empty());
        assert!(!lines[1].changed_spans.is_empty());
        assert!(!lines[2].changed_spans.is_empty());
        assert!(lines[3].changed_spans.is_empty());
    }

    // --- perf guard: pathological long line degrades instead of blowing up ---

    #[test]
    fn very_long_line_degrades_to_whole_line_span_without_hanging() {
        let old = "a ".repeat(MAX_LINE_TOKENS); // MAX_LINE_TOKENS+ tokens
        let new = "b ".repeat(MAX_LINE_TOKENS);
        let (old_spans, new_spans) = word_diff_spans(&old, &new);
        assert_eq!(old_spans, vec![0..old.chars().count()]);
        assert_eq!(new_spans, vec![0..new.chars().count()]);
    }
}
