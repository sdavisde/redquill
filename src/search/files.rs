//! File-candidate model for the fuzzy file finder (spec 06 Unit 1): merges
//! git's tracked (`git ls-files`) and untracked-but-unignored
//! (`git ls-files --others --exclude-standard`) path lists into one
//! deduplicated, deterministically-ordered candidate list.
//!
//! Pure — no I/O, no TUI types. The git layer supplies the raw path lists
//! (see [`crate::git::GitRunner::ls_files`]/
//! [`crate::git::GitRunner::ls_files_untracked`]); `crate::ui` wires this
//! into the finder's background loader.

use std::collections::HashSet;

/// One file the fuzzy finder can jump to: a repo-relative path, tracked or
/// untracked-but-unignored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCandidate {
    /// The file's path, relative to the repo root.
    pub path: String,
}

/// Merges `tracked` and `untracked` path lists into one deduplicated
/// candidate list, sorted by path (byte-wise ascending) for a deterministic
/// result independent of which list a path came from. The two sets are
/// disjoint in practice — a path can't be both index-tracked and
/// untracked-but-unignored at once — but the dedup is defensive (one
/// `HashSet`, cheap) rather than assumed.
pub fn merge_candidates(tracked: Vec<String>, untracked: Vec<String>) -> Vec<FileCandidate> {
    let mut seen = HashSet::with_capacity(tracked.len() + untracked.len());
    let mut candidates: Vec<FileCandidate> = tracked
        .into_iter()
        .chain(untracked)
        .filter(|path| seen.insert(path.clone()))
        .map(|path| FileCandidate { path })
        .collect();
    candidates.sort_by(|a, b| a.path.cmp(&b.path));
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(candidates: &[FileCandidate]) -> Vec<&str> {
        candidates.iter().map(|c| c.path.as_str()).collect()
    }

    #[test]
    fn merges_and_sorts_tracked_and_untracked() {
        let tracked = vec!["src/main.rs".to_string(), "Cargo.toml".to_string()];
        let untracked = vec!["scratch.txt".to_string()];
        let merged = merge_candidates(tracked, untracked);
        assert_eq!(
            paths(&merged),
            vec!["Cargo.toml", "scratch.txt", "src/main.rs"]
        );
    }

    #[test]
    fn dedupes_a_path_present_in_both_lists() {
        let tracked = vec!["a.rs".to_string()];
        let untracked = vec!["a.rs".to_string(), "b.rs".to_string()];
        let merged = merge_candidates(tracked, untracked);
        assert_eq!(paths(&merged), vec!["a.rs", "b.rs"]);
    }

    #[test]
    fn empty_lists_yield_no_candidates() {
        assert!(merge_candidates(Vec::new(), Vec::new()).is_empty());
    }

    #[test]
    fn order_is_deterministic_regardless_of_input_order() {
        let tracked = vec!["z.rs".to_string(), "a.rs".to_string()];
        let untracked = vec!["m.rs".to_string()];
        let merged = merge_candidates(tracked, untracked);
        assert_eq!(paths(&merged), vec!["a.rs", "m.rs", "z.rs"]);
    }
}
