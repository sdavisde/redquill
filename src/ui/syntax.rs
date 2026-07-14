//! Syntax-highlight glue: deriving where to source one side's whole-file
//! content from for a given [`DiffTarget`] ([`content_source`]/
//! [`fetch_content`]), and caching the resulting per-line highlight spans
//! so a `(path, side)` is only ever highlighted once between cache clears
//! ([`HighlightCache`]).
//!
//! The diff itself only carries changed lines, but tree-sitter needs whole
//! -file text to parse correctly, hence sourcing full content per side
//! separately from the diff/patch machinery in [`crate::git`]/[`crate::diff`].

use std::collections::HashMap;
use std::ops::Range;

use crate::annotate::Side;
use crate::diff::{FileDiff, LineOrigin};
use crate::git::DiffTarget;
use crate::highlight::{Highlighter, Lang, TokenKind};

use super::stage_ops::StageOps;

/// Where to read one side's whole-file content from for highlighting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ContentSource {
    /// The live working-tree file at this repo-relative path.
    Worktree(String),
    /// `git show <spec>`.
    Show(String),
}

/// Splits a range expression at its last `..`/`...` boundary into
/// `(left, right)`, with surrounding dots trimmed off each piece (so both
/// two-dot and three-dot range syntax resolve the same way). `None` if `r`
/// contains no `..` at all (a bare ref, e.g. `HEAD~2`).
fn split_range(r: &str) -> Option<(String, String)> {
    let idx = r.rfind("..")?;
    let left = r[..idx].trim_end_matches('.').to_string();
    let right = r[idx + 2..].trim_start_matches('.').to_string();
    Some((left, right))
}

/// Derives where to source `side`'s whole-file content for `path` under
/// `target`. `old_path` is used for the old side of a renamed file (the
/// content lived at the old path before the rename). Pure — no I/O, so
/// every target x side x rename combination is directly unit-testable.
///
/// - New side: `WorkingTree` -> the worktree file; `Staged` -> the index
///   blob (`:0:<path>`); `Range(r)` -> if `r` contains `..`, the blob at
///   the ref right of the last `..` (empty means the worktree file, e.g.
///   `main..`); otherwise (a bare ref) the worktree file; `Commit(rev)` ->
///   `<rev>:<path>`, the blob as that commit left it.
/// - Old side (for `Removed` lines): `WorkingTree` -> the index blob
///   (`:0:<path>`, i.e. what staging would currently produce); `Staged` ->
///   `HEAD:<path>`; `Range(r)` -> the blob at the ref left of the last
///   `..` if present, else `<r>:<path>` for a bare ref; `Commit(rev)` ->
///   `<rev>^:<path>`, the blob as the commit's parent left it. For a root
///   commit `<rev>^` doesn't resolve, so [`super::stage_ops::StageOps::show_file`]
///   returns `None` and highlighting degrades to no content — the same
///   graceful fallback any unresolvable spec gets, never a special case
///   here (this function stays pure and I/O-free).
pub(super) fn content_source(
    target: &DiffTarget,
    side: Side,
    path: &str,
    old_path: Option<&str>,
) -> ContentSource {
    match side {
        Side::New => match target {
            DiffTarget::WorkingTree => ContentSource::Worktree(path.to_string()),
            DiffTarget::Staged => ContentSource::Show(format!(":0:{path}")),
            DiffTarget::Range(r) => match split_range(r) {
                Some((_, right)) if !right.is_empty() => {
                    ContentSource::Show(format!("{right}:{path}"))
                }
                _ => ContentSource::Worktree(path.to_string()),
            },
            DiffTarget::Commit(rev) => ContentSource::Show(format!("{rev}:{path}")),
        },
        Side::Old => {
            let src = old_path.unwrap_or(path);
            match target {
                DiffTarget::WorkingTree => ContentSource::Show(format!(":0:{src}")),
                DiffTarget::Staged => ContentSource::Show(format!("HEAD:{src}")),
                DiffTarget::Range(r) => match split_range(r) {
                    Some((left, _)) => ContentSource::Show(format!("{left}:{src}")),
                    None => ContentSource::Show(format!("{r}:{src}")),
                },
                DiffTarget::Commit(rev) => ContentSource::Show(format!("{rev}^:{src}")),
            }
        }
    }
}

/// Resolves [`content_source`] against a real backend. `None` on any
/// sourcing failure (unreadable worktree file, unknown revision, binary
/// content that fails UTF-8 decode, ...) — highlighting degrades silently
/// rather than erroring.
pub(super) fn fetch_content(
    ops: &dyn StageOps,
    target: &DiffTarget,
    path: &str,
    old_path: Option<&str>,
    side: Side,
) -> Option<String> {
    match content_source(target, side, path, old_path) {
        ContentSource::Worktree(p) => ops
            .read_worktree_file(&p)
            .and_then(|bytes| String::from_utf8(bytes).ok()),
        ContentSource::Show(spec) => ops.show_file(&spec),
    }
}

/// Whether `file` has at least one line on `side` (`Removed` lines live
/// only on the old side; `Added`/`Context` lines live on the new side) —
/// used to skip a wasted content fetch/highlight pass for a side no row
/// needs (e.g. the old side of a pure-addition diff).
pub(super) fn side_in_use(file: &FileDiff, side: Side) -> bool {
    file.hunks.iter().any(|h| {
        h.lines.iter().any(|l| match side {
            Side::Old => l.origin == LineOrigin::Removed,
            Side::New => matches!(l.origin, LineOrigin::Added | LineOrigin::Context),
        })
    })
}

/// Per-line highlighted spans for one whole-file side, indexed by 0-based
/// line number (index `n` is 1-based line `n + 1`), matching
/// [`Highlighter::highlight_lines`]'s output order.
pub(super) type LineSpans = Vec<Vec<(Range<usize>, TokenKind)>>;

/// Caches highlighted line spans per `(path, side)`, so a file/side is
/// highlighted at most once between [`HighlightCache::clear`] calls (the
/// [`super::App`] clears the cache on every refresh, since staging/refresh
/// can change file content).
#[derive(Default)]
pub(super) struct HighlightCache {
    entries: HashMap<(String, Side), LineSpans>,
}

impl HighlightCache {
    /// Drops every cached entry. Used when the whole diff context changes at
    /// once (e.g. [`super::App::with_git`] switching target/backend). A plain
    /// refresh instead invalidates per file (see [`HighlightCache::invalidate_path`]
    /// / [`HighlightCache::retain_paths`]) so unchanged files keep their spans.
    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }

    /// Drops both sides' cached spans for one `path` (a `no-op` if neither is
    /// cached). Called on refresh for a file whose diff content actually
    /// changed, so only that file is re-highlighted on the next rebuild while
    /// every other file's spans survive.
    pub(super) fn invalidate_path(&mut self, path: &str) {
        self.entries
            .retain(|(cached_path, _), _| cached_path != path);
    }

    /// Drops every cached entry whose path fails `keep`, so files that left
    /// the review on a refresh can't leave their spans behind (unbounded
    /// growth over a long session).
    pub(super) fn retain_paths(&mut self, keep: impl Fn(&str) -> bool) {
        self.entries.retain(|(path, _), _| keep(path));
    }

    /// The cached spans for `(path, side)`, or an empty slice if not (yet)
    /// populated.
    pub(super) fn get(&self, path: &str, side: Side) -> &[Vec<(Range<usize>, TokenKind)>] {
        self.entries
            .get(&(path.to_string(), side))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// The number of `(path, side)` entries currently cached (test hook).
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether `(path, side)` currently has a cached entry, regardless of
    /// whether its span list is empty (test hook — distinguishes "cached, no
    /// spans" from "not cached").
    #[cfg(test)]
    pub(super) fn contains(&self, path: &str, side: Side) -> bool {
        self.entries.contains_key(&(path.to_string(), side))
    }
}

/// Ensures `(path, side)` is populated in `cache`, sourcing content and
/// running `highlighter` over it only on a cache miss. A free function
/// (rather than a method) so callers can pass disjoint borrows of an
/// owning struct's fields (cache, highlighter, stage ops) without the
/// borrow checker treating them as one aggregate borrow.
#[allow(clippy::too_many_arguments)]
pub(super) fn populate_cache(
    cache: &mut HighlightCache,
    highlighter: &mut Highlighter,
    ops: Option<&dyn StageOps>,
    target: &DiffTarget,
    path: &str,
    old_path: Option<&str>,
    side: Side,
    synthetic: bool,
) {
    let key = (path.to_string(), side);
    if cache.entries.contains_key(&key) {
        return;
    }
    let content = match (synthetic, side) {
        // A synthetic untracked file has no diff target/old side to speak
        // of; its "new" content is just the worktree file itself.
        (true, Side::New) => ops
            .and_then(|ops| ops.read_worktree_file(path))
            .and_then(|bytes| String::from_utf8(bytes).ok()),
        (true, Side::Old) => None,
        (false, _) => ops.and_then(|ops| fetch_content(ops, target, path, old_path, side)),
    };
    let spans = match (content, Lang::from_path(path)) {
        (Some(content), Some(lang)) => highlighter.highlight_lines(lang, &content),
        _ => Vec::new(),
    };
    cache.entries.insert(key, spans);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotate::Side;

    // -- content_source: target x side x rename ------------------------

    #[test]
    fn new_side_working_tree_is_worktree_file() {
        assert_eq!(
            content_source(&DiffTarget::WorkingTree, Side::New, "a.rs", None),
            ContentSource::Worktree("a.rs".to_string())
        );
    }

    #[test]
    fn new_side_staged_is_index_blob() {
        assert_eq!(
            content_source(&DiffTarget::Staged, Side::New, "a.rs", None),
            ContentSource::Show(":0:a.rs".to_string())
        );
    }

    #[test]
    fn new_side_range_with_right_ref_uses_right_blob() {
        assert_eq!(
            content_source(
                &DiffTarget::Range("main..HEAD".to_string()),
                Side::New,
                "a.rs",
                None
            ),
            ContentSource::Show("HEAD:a.rs".to_string())
        );
    }

    #[test]
    fn new_side_range_with_empty_right_is_worktree_file() {
        assert_eq!(
            content_source(
                &DiffTarget::Range("main..".to_string()),
                Side::New,
                "a.rs",
                None
            ),
            ContentSource::Worktree("a.rs".to_string())
        );
    }

    #[test]
    fn new_side_range_three_dot_trims_dots() {
        assert_eq!(
            content_source(
                &DiffTarget::Range("main...HEAD".to_string()),
                Side::New,
                "a.rs",
                None
            ),
            ContentSource::Show("HEAD:a.rs".to_string())
        );
    }

    #[test]
    fn new_side_bare_ref_is_worktree_file() {
        assert_eq!(
            content_source(
                &DiffTarget::Range("HEAD~2".to_string()),
                Side::New,
                "a.rs",
                None
            ),
            ContentSource::Worktree("a.rs".to_string())
        );
    }

    #[test]
    fn old_side_working_tree_is_index_blob() {
        assert_eq!(
            content_source(&DiffTarget::WorkingTree, Side::Old, "a.rs", None),
            ContentSource::Show(":0:a.rs".to_string())
        );
    }

    #[test]
    fn old_side_staged_is_head_blob() {
        assert_eq!(
            content_source(&DiffTarget::Staged, Side::Old, "a.rs", None),
            ContentSource::Show("HEAD:a.rs".to_string())
        );
    }

    #[test]
    fn old_side_range_with_dots_uses_left_blob() {
        assert_eq!(
            content_source(
                &DiffTarget::Range("main..HEAD".to_string()),
                Side::Old,
                "a.rs",
                None
            ),
            ContentSource::Show("main:a.rs".to_string())
        );
    }

    #[test]
    fn old_side_bare_ref_uses_ref_blob() {
        assert_eq!(
            content_source(
                &DiffTarget::Range("HEAD~2".to_string()),
                Side::Old,
                "a.rs",
                None
            ),
            ContentSource::Show("HEAD~2:a.rs".to_string())
        );
    }

    #[test]
    fn old_side_prefers_old_path_for_renames() {
        assert_eq!(
            content_source(&DiffTarget::Staged, Side::Old, "new.rs", Some("old.rs")),
            ContentSource::Show("HEAD:old.rs".to_string())
        );
        assert_eq!(
            content_source(
                &DiffTarget::WorkingTree,
                Side::Old,
                "new.rs",
                Some("old.rs")
            ),
            ContentSource::Show(":0:old.rs".to_string())
        );
    }

    #[test]
    fn new_side_commit_is_rev_colon_path() {
        assert_eq!(
            content_source(
                &DiffTarget::Commit("abc123".to_string()),
                Side::New,
                "a.rs",
                None
            ),
            ContentSource::Show("abc123:a.rs".to_string())
        );
    }

    #[test]
    fn old_side_commit_is_rev_caret_colon_path() {
        assert_eq!(
            content_source(
                &DiffTarget::Commit("abc123".to_string()),
                Side::Old,
                "a.rs",
                None
            ),
            ContentSource::Show("abc123^:a.rs".to_string())
        );
    }

    #[test]
    fn old_side_commit_prefers_old_path_for_renames() {
        assert_eq!(
            content_source(
                &DiffTarget::Commit("abc123".to_string()),
                Side::Old,
                "new.rs",
                Some("old.rs")
            ),
            ContentSource::Show("abc123^:old.rs".to_string())
        );
    }

    #[test]
    fn new_side_commit_ignores_old_path_even_for_renames() {
        assert_eq!(
            content_source(
                &DiffTarget::Commit("abc123".to_string()),
                Side::New,
                "new.rs",
                Some("old.rs")
            ),
            ContentSource::Show("abc123:new.rs".to_string())
        );
    }

    #[test]
    fn new_side_ignores_old_path_even_for_renames() {
        // The new side always reads the current path; old_path only
        // matters on the old side.
        assert_eq!(
            content_source(&DiffTarget::Staged, Side::New, "new.rs", Some("old.rs")),
            ContentSource::Show(":0:new.rs".to_string())
        );
    }

    #[test]
    fn root_commit_old_side_degrades_to_no_content_not_a_panic() {
        // A root commit has no parent, so `<rev>^:<path>` never resolves.
        // fetch_content must degrade to `None` (fall through to
        // `show_file`'s own "unresolvable spec" contract) rather than the
        // git layer needing any root-commit special case.
        struct RootCommitOps;
        impl StageOps for RootCommitOps {
            fn diff(
                &self,
                _target: &DiffTarget,
            ) -> Result<Vec<crate::git::RawFilePatch>, crate::git::GitError> {
                Ok(Vec::new())
            }
            fn status(&self) -> Result<Vec<crate::git::FileStatus>, crate::git::GitError> {
                Ok(Vec::new())
            }
            fn stage_file(&self, _path: &str) -> Result<(), crate::git::GitError> {
                Ok(())
            }
            fn unstage_file(&self, _path: &str) -> Result<(), crate::git::GitError> {
                Ok(())
            }
            fn apply_cached(&self, _patch: &str) -> Result<(), crate::git::GitError> {
                Ok(())
            }
            fn unapply_cached(&self, _patch: &str) -> Result<(), crate::git::GitError> {
                Ok(())
            }
            fn read_worktree_file(&self, _path: &str) -> Option<Vec<u8>> {
                None
            }
            fn show_file(&self, spec: &str) -> Option<String> {
                // `<rev>^:<path>` never resolves for a root commit; every
                // other spec would.
                if spec.contains('^') {
                    None
                } else {
                    Some("fn main() {}\n".to_string())
                }
            }
        }

        let ops = RootCommitOps;
        let target = DiffTarget::Commit("root".to_string());
        assert_eq!(fetch_content(&ops, &target, "a.rs", None, Side::Old), None);
        assert_eq!(
            fetch_content(&ops, &target, "a.rs", None, Side::New),
            Some("fn main() {}\n".to_string())
        );
    }

    // -- HighlightCache ---------------------------------------------------

    struct CountingOps {
        show_calls: std::cell::RefCell<usize>,
    }

    impl StageOps for CountingOps {
        fn diff(
            &self,
            _target: &DiffTarget,
        ) -> Result<Vec<crate::git::RawFilePatch>, crate::git::GitError> {
            Ok(Vec::new())
        }
        fn status(&self) -> Result<Vec<crate::git::FileStatus>, crate::git::GitError> {
            Ok(Vec::new())
        }
        fn stage_file(&self, _path: &str) -> Result<(), crate::git::GitError> {
            Ok(())
        }
        fn unstage_file(&self, _path: &str) -> Result<(), crate::git::GitError> {
            Ok(())
        }
        fn apply_cached(&self, _patch: &str) -> Result<(), crate::git::GitError> {
            Ok(())
        }
        fn unapply_cached(&self, _patch: &str) -> Result<(), crate::git::GitError> {
            Ok(())
        }
        fn read_worktree_file(&self, _path: &str) -> Option<Vec<u8>> {
            None
        }
        fn show_file(&self, _spec: &str) -> Option<String> {
            *self.show_calls.borrow_mut() += 1;
            Some("fn main() {}\n".to_string())
        }
    }

    #[test]
    fn populate_cache_only_fetches_once_per_path_and_side() {
        let ops = CountingOps {
            show_calls: std::cell::RefCell::new(0),
        };
        let mut cache = HighlightCache::default();
        let mut highlighter = Highlighter::new();
        let target = DiffTarget::Staged;

        populate_cache(
            &mut cache,
            &mut highlighter,
            Some(&ops),
            &target,
            "a.rs",
            None,
            Side::New,
            false,
        );
        populate_cache(
            &mut cache,
            &mut highlighter,
            Some(&ops),
            &target,
            "a.rs",
            None,
            Side::New,
            false,
        );
        populate_cache(
            &mut cache,
            &mut highlighter,
            Some(&ops),
            &target,
            "a.rs",
            None,
            Side::New,
            false,
        );

        assert_eq!(*ops.show_calls.borrow(), 1);
        assert_eq!(cache.len(), 1);
        assert!(!cache.get("a.rs", Side::New).is_empty());
    }

    #[test]
    fn populate_cache_treats_each_side_independently() {
        let ops = CountingOps {
            show_calls: std::cell::RefCell::new(0),
        };
        let mut cache = HighlightCache::default();
        let mut highlighter = Highlighter::new();
        let target = DiffTarget::Staged;

        populate_cache(
            &mut cache,
            &mut highlighter,
            Some(&ops),
            &target,
            "a.rs",
            None,
            Side::New,
            false,
        );
        populate_cache(
            &mut cache,
            &mut highlighter,
            Some(&ops),
            &target,
            "a.rs",
            None,
            Side::Old,
            false,
        );

        assert_eq!(*ops.show_calls.borrow(), 2);
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn clear_forces_a_fresh_fetch() {
        let ops = CountingOps {
            show_calls: std::cell::RefCell::new(0),
        };
        let mut cache = HighlightCache::default();
        let mut highlighter = Highlighter::new();
        let target = DiffTarget::Staged;

        populate_cache(
            &mut cache,
            &mut highlighter,
            Some(&ops),
            &target,
            "a.rs",
            None,
            Side::New,
            false,
        );
        cache.clear();
        populate_cache(
            &mut cache,
            &mut highlighter,
            Some(&ops),
            &target,
            "a.rs",
            None,
            Side::New,
            false,
        );

        assert_eq!(*ops.show_calls.borrow(), 2);
    }

    #[test]
    fn invalidate_path_drops_both_sides_but_keeps_other_files() {
        let ops = CountingOps {
            show_calls: std::cell::RefCell::new(0),
        };
        let mut cache = HighlightCache::default();
        let mut highlighter = Highlighter::new();
        let target = DiffTarget::Staged;
        for (path, side) in [
            ("a.rs", Side::New),
            ("a.rs", Side::Old),
            ("b.rs", Side::New),
        ] {
            populate_cache(
                &mut cache,
                &mut highlighter,
                Some(&ops),
                &target,
                path,
                None,
                side,
                false,
            );
        }
        assert_eq!(cache.len(), 3);

        cache.invalidate_path("a.rs");
        // Both a.rs sides gone; b.rs untouched.
        assert!(!cache.contains("a.rs", Side::New));
        assert!(!cache.contains("a.rs", Side::Old));
        assert!(cache.contains("b.rs", Side::New));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn retain_paths_drops_entries_for_absent_paths() {
        let ops = CountingOps {
            show_calls: std::cell::RefCell::new(0),
        };
        let mut cache = HighlightCache::default();
        let mut highlighter = Highlighter::new();
        let target = DiffTarget::Staged;
        for path in ["a.rs", "b.rs", "c.rs"] {
            populate_cache(
                &mut cache,
                &mut highlighter,
                Some(&ops),
                &target,
                path,
                None,
                Side::New,
                false,
            );
        }
        assert_eq!(cache.len(), 3);

        // Keep only a.rs and c.rs (b.rs left the review).
        cache.retain_paths(|path| path == "a.rs" || path == "c.rs");
        assert!(cache.contains("a.rs", Side::New));
        assert!(!cache.contains("b.rs", Side::New));
        assert!(cache.contains("c.rs", Side::New));
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn synthetic_new_side_reads_worktree_not_show() {
        struct WorktreeOps;
        impl StageOps for WorktreeOps {
            fn diff(
                &self,
                _t: &DiffTarget,
            ) -> Result<Vec<crate::git::RawFilePatch>, crate::git::GitError> {
                Ok(Vec::new())
            }
            fn status(&self) -> Result<Vec<crate::git::FileStatus>, crate::git::GitError> {
                Ok(Vec::new())
            }
            fn stage_file(&self, _p: &str) -> Result<(), crate::git::GitError> {
                Ok(())
            }
            fn unstage_file(&self, _p: &str) -> Result<(), crate::git::GitError> {
                Ok(())
            }
            fn apply_cached(&self, _p: &str) -> Result<(), crate::git::GitError> {
                Ok(())
            }
            fn unapply_cached(&self, _p: &str) -> Result<(), crate::git::GitError> {
                Ok(())
            }
            fn read_worktree_file(&self, _path: &str) -> Option<Vec<u8>> {
                Some(b"let x = 1;\n".to_vec())
            }
            fn show_file(&self, _spec: &str) -> Option<String> {
                panic!("synthetic new-side content must not call show_file");
            }
        }

        let mut cache = HighlightCache::default();
        let mut highlighter = Highlighter::new();
        let ops = WorktreeOps;
        populate_cache(
            &mut cache,
            &mut highlighter,
            Some(&ops),
            &DiffTarget::WorkingTree,
            "new.rs",
            None,
            Side::New,
            true,
        );
        assert!(!cache.get("new.rs", Side::New).is_empty());
    }

    #[test]
    fn side_in_use_detects_removed_and_added_lines() {
        use crate::diff::{FileChangeKind, FileDiff};
        use crate::git::RawFilePatch;

        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,2 +1,2 @@
-old
+new
 ctx
";
        let file = FileDiff::from_patch(&RawFilePatch {
            path: "f.rs".to_string(),
            old_path: None,
            raw: raw.to_string(),
            is_binary: false,
        })
        .unwrap();
        assert!(side_in_use(&file, Side::Old));
        assert!(side_in_use(&file, Side::New));

        let no_hunks = FileDiff {
            path: "empty.rs".to_string(),
            old_path: None,
            kind: FileChangeKind::Modified,
            is_binary: false,
            hunks: Vec::new(),
        };
        assert!(!side_in_use(&no_hunks, Side::Old));
        assert!(!side_in_use(&no_hunks, Side::New));
    }
}
