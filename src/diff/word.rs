//! Intra-line word diff seam.
//!
//! `word_diff_spans` is the ONE place the intra-line diff algorithm lives
//! (spec §5); `attach_word_spans` pairs removed/added lines within each
//! hunk and calls it exactly once per pair. This module is a compiling stub
//! for T1.0 — the real LCS-over-tokens algorithm and pairing logic are
//! filled in by T2.0 (Wave 2). Do not add other span-computing call sites.

use super::model::DiffFile;

/// The ONLY place the intra-line algorithm lives; swap the body freely.
/// Returns `(old_spans, new_spans)` as char ranges into `old` / `new`
/// respectively.
///
/// Stub (T1.0): always returns empty spans. Filled in by T2.0.
pub fn word_diff_spans(
    old: &str,
    new: &str,
) -> (Vec<std::ops::Range<usize>>, Vec<std::ops::Range<usize>>) {
    let _ = old;
    let _ = new;
    (Vec::new(), Vec::new())
}

/// Pairs removed/added lines positionally within each contiguous change run
/// and stores `word_diff_spans` results onto both lines of each pair.
///
/// Stub (T1.0): no-op. Filled in by T2.0.
pub fn attach_word_spans(file: &mut DiffFile) {
    let _ = file;
}
