//! Syntax-highlight glue: deriving where to source one side's whole-file
//! content from for a given [`DiffTarget`] ([`content_source`]/[`source_for`]/
//! [`fetch_content`]), and caching the resulting per-line highlight spans
//! keyed by that source, so a given blob is only ever highlighted once
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
///
/// Doubles as [`HighlightCache`]'s key. That is the point: the cache is keyed
/// by *what bytes are being highlighted*, not by which view happens to be
/// showing them. Two views over the same blob (a commit reached from the
/// History tab and from the Review launcher) share one entry, and two views
/// over different blobs can never collide — so switching views needs no
/// wholesale cache clear, and a commit's spans survive an `Esc` and return.
///
/// `rev` and `path` stay separate rather than pre-joined into one `git show`
/// spec so per-path invalidation can match on the path alone.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum ContentSource {
    /// The live working-tree file at this repo-relative path.
    Worktree { path: String },
    /// The blob at `<rev>:<path>`, read via `git show`.
    Show { rev: String, path: String },
}

impl ContentSource {
    /// The repo-relative path this source reads, whichever variant it is —
    /// what [`HighlightCache::invalidate_path`] and
    /// [`HighlightCache::retain_paths`] match on.
    pub(super) fn path(&self) -> &str {
        match self {
            ContentSource::Worktree { path } | ContentSource::Show { path, .. } => path,
        }
    }
}

/// A [`ContentSource::Worktree`] for `path`.
fn worktree(path: &str) -> ContentSource {
    ContentSource::Worktree {
        path: path.to_string(),
    }
}

/// A [`ContentSource::Show`] for `<rev>:<path>`.
fn show(rev: &str, path: &str) -> ContentSource {
    ContentSource::Show {
        rev: rev.to_string(),
        path: path.to_string(),
    }
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
            DiffTarget::WorkingTree => worktree(path),
            DiffTarget::Staged => show(":0", path),
            DiffTarget::Range(r) => match split_range(r) {
                Some((_, right)) if !right.is_empty() => show(&right, path),
                _ => worktree(path),
            },
            DiffTarget::Commit(rev) => show(rev, path),
            // Same shape as `Range` with a non-empty right side: the
            // branch's blob at its tip. In practice this is byte-identical
            // to the review worktree's on-disk file (that's the whole point
            // of the dedicated worktree), but sourcing the git blob keeps
            // this function's contract uniform with every other historical
            // target rather than special-casing review to read off disk.
            DiffTarget::Review { branch, .. } => show(branch, path),
            // Defensive fallback; never expected in practice.
            DiffTarget::File(p) => worktree(p),
        },
        Side::Old => {
            let src = old_path.unwrap_or(path);
            match target {
                DiffTarget::WorkingTree => show(":0", src),
                DiffTarget::Staged => show("HEAD", src),
                DiffTarget::Range(r) => match split_range(r) {
                    Some((left, _)) => show(&left, src),
                    None => show(r, src),
                },
                DiffTarget::Commit(rev) => show(&format!("{rev}^"), src),
                // The base ref's blob — the three-dot diff's "old" side,
                // mirroring `Range`'s left-side handling.
                DiffTarget::Review { base, .. } => show(base, src),
                // A whole-file view has no old side at all (see the New-side
                // arm's doc above) — never reached, since the synthesized
                // file has zero `Removed` lines, so `side_in_use` never asks
                // for this side to begin with.
                DiffTarget::File(_) => worktree(src),
            }
        }
    }
}

/// The content source for one side of `path`, or `None` when that side has
/// no content at all.
///
/// `synthetic` marks an untracked file synthesized into the review: `git
/// diff` never surfaced it, so its new side is the worktree file whatever the
/// target is (a review worktree's branch blob wouldn't contain it at all),
/// and it has no old side to read.
pub(super) fn source_for(
    target: &DiffTarget,
    side: Side,
    path: &str,
    old_path: Option<&str>,
    synthetic: bool,
) -> Option<ContentSource> {
    match (synthetic, side) {
        (true, Side::New) => Some(worktree(path)),
        (true, Side::Old) => None,
        (false, _) => Some(content_source(target, side, path, old_path)),
    }
}

/// Resolves a [`ContentSource`] against a real backend. `None` on any
/// sourcing failure (unreadable worktree file, unknown revision, binary
/// content that fails UTF-8 decode, ...) — highlighting degrades silently
/// rather than erroring.
pub(super) fn fetch_content(ops: &dyn StageOps, source: &ContentSource) -> Option<String> {
    match source {
        ContentSource::Worktree { path } => ops
            .read_worktree_file(path)
            .and_then(|bytes| String::from_utf8(bytes).ok()),
        ContentSource::Show { rev, path } => ops.show_file(&format!("{rev}:{path}")),
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

/// How many cached lines the cache holds before evicting.
///
/// Entries keyed by an immutable blob (a commit's `<sha>:<path>`) are never
/// invalidated by anything, which is what makes returning to a commit view
/// free — but it also means nothing would ever drop them. This ceiling is the
/// backstop: a session paging through history stays bounded. It is several
/// times the largest review anyone scrolls end-to-end in one sitting, and
/// re-highlighting an evicted file costs a couple of milliseconds, so the
/// eviction path is cheap when it does fire.
const MAX_CACHED_LINES: usize = 150_000;

/// One cached side's spans plus the recency stamp [`HighlightCache`] evicts by.
struct Entry {
    spans: LineSpans,
    used: u64,
}

/// Caches highlighted line spans per [`ContentSource`] — per *blob*, not per
/// view — so a file/side is highlighted at most once however many views show
/// it, and is re-highlighted only when its bytes could actually have changed
/// (see [`HighlightCache::invalidate_path`]). Bounded by [`MAX_CACHED_LINES`],
/// evicting least-recently-used first.
#[derive(Default)]
pub(super) struct HighlightCache {
    entries: HashMap<ContentSource, Entry>,
    /// Total cached lines across every entry, kept in step with `entries` so
    /// the eviction check is O(1) rather than a re-sum per insert.
    lines: usize,
    /// Monotonic recency counter stamped onto an entry on insert and on hit.
    clock: u64,
}

impl HighlightCache {
    /// Drops every cached entry. Used when the whole diff context changes at
    /// once (e.g. [`super::App::with_git`] switching target/backend). Neither
    /// switching views nor refreshing needs this: view switches can't collide
    /// (entries are keyed by blob) and a refresh invalidates per file (see
    /// [`HighlightCache::invalidate_path`] / [`HighlightCache::retain_paths`]).
    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.lines = 0;
    }

    /// Drops every cached side sourced from `path` (a no-op if none are
    /// cached). Called on refresh for a file whose diff content actually
    /// changed, so only that file is re-highlighted while every other file's
    /// spans survive.
    ///
    /// This drops the path's immutable-blob entries too, which it needn't:
    /// only the live sources (worktree file, index blob) can have changed.
    /// Over-invalidating one path is a couple of milliseconds to redo and
    /// leaves no way for a stale span to survive, which is the tradeoff worth
    /// taking here.
    pub(super) fn invalidate_path(&mut self, path: &str) {
        self.retain(|source| source.path() != path);
    }

    /// Drops every cached entry whose path fails `keep`, so files that left
    /// the review on a refresh can't leave their spans behind.
    pub(super) fn retain_paths(&mut self, keep: impl Fn(&str) -> bool) {
        self.retain(|source| keep(source.path()));
    }

    /// Shared retain, keeping `lines` in step with what survives.
    fn retain(&mut self, keep: impl Fn(&ContentSource) -> bool) {
        let mut lines = 0;
        self.entries.retain(|source, entry| {
            let keeping = keep(source);
            if keeping {
                lines += entry.spans.len();
            }
            keeping
        });
        self.lines = lines;
    }

    /// The cached spans for `source`, or an empty slice if not (yet)
    /// populated. Takes `&self` so a row build can hold this borrow alongside
    /// the files it is building from; recency is stamped by
    /// [`HighlightCache::touch`] on the populate path instead.
    pub(super) fn get(&self, source: &ContentSource) -> &[Vec<(Range<usize>, TokenKind)>] {
        self.entries
            .get(source)
            .map(|entry| entry.spans.as_slice())
            .unwrap_or(&[])
    }

    /// Marks `source` as just-used and reports whether it was cached at all.
    fn touch(&mut self, source: &ContentSource) -> bool {
        self.clock += 1;
        let now = self.clock;
        match self.entries.get_mut(source) {
            Some(entry) => {
                entry.used = now;
                true
            }
            None => false,
        }
    }

    /// Inserts `spans` for `source`, then evicts down to [`MAX_CACHED_LINES`].
    fn insert(&mut self, source: ContentSource, spans: LineSpans) {
        self.clock += 1;
        self.lines += spans.len();
        let entry = Entry {
            spans,
            used: self.clock,
        };
        if let Some(replaced) = self.entries.insert(source, entry) {
            self.lines -= replaced.spans.len();
        }
        // The just-inserted entry holds the highest `used`, so it is the last
        // thing this loop would pick — it can only be evicted by being the
        // sole entry left, which the length guard rules out.
        while self.lines > MAX_CACHED_LINES && self.entries.len() > 1 {
            let victim = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(source, _)| source.clone());
            let Some(victim) = victim else { break };
            if let Some(evicted) = self.entries.remove(&victim) {
                self.lines -= evicted.spans.len();
            }
        }
    }

    /// The number of entries currently cached (test hook).
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Total cached lines across every entry (test hook).
    #[cfg(test)]
    pub(super) fn cached_lines(&self) -> usize {
        self.lines
    }

    /// Whether any cached entry is sourced from `path` (test hook). Asked by
    /// path rather than by key so it can still answer for a file that has
    /// left the review, which is what the invalidation tests need.
    #[cfg(test)]
    pub(super) fn has_path(&self, path: &str) -> bool {
        self.entries.keys().any(|source| source.path() == path)
    }
}

/// Ensures `source` is populated in `cache`, sourcing content and running
/// `highlighter` over it only on a cache miss. `lang_path` is the file's
/// current path, which decides the language (the old side of a rename reads
/// its content from the old path but highlights as the new path's language).
///
/// A free function (rather than a method) so callers can pass disjoint
/// borrows of an owning struct's fields (cache, highlighter, stage ops)
/// without the borrow checker treating them as one aggregate borrow.
pub(super) fn populate_cache(
    cache: &mut HighlightCache,
    highlighter: &mut Highlighter,
    ops: Option<&dyn StageOps>,
    source: &ContentSource,
    lang_path: &str,
) {
    if cache.touch(source) {
        return;
    }
    let content = ops.and_then(|ops| fetch_content(ops, source));
    let spans = match (content, Lang::from_path(lang_path)) {
        (Some(content), Some(lang)) => highlighter.highlight_lines(lang, &content),
        _ => Vec::new(),
    };
    cache.insert(source.clone(), spans);
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
            worktree("a.rs")
        );
    }

    #[test]
    fn new_side_staged_is_index_blob() {
        assert_eq!(
            content_source(&DiffTarget::Staged, Side::New, "a.rs", None),
            show(":0", "a.rs")
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
            show("HEAD", "a.rs")
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
            worktree("a.rs")
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
            show("HEAD", "a.rs")
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
            worktree("a.rs")
        );
    }

    #[test]
    fn old_side_working_tree_is_index_blob() {
        assert_eq!(
            content_source(&DiffTarget::WorkingTree, Side::Old, "a.rs", None),
            show(":0", "a.rs")
        );
    }

    #[test]
    fn old_side_staged_is_head_blob() {
        assert_eq!(
            content_source(&DiffTarget::Staged, Side::Old, "a.rs", None),
            show("HEAD", "a.rs")
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
            show("main", "a.rs")
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
            show("HEAD~2", "a.rs")
        );
    }

    #[test]
    fn old_side_prefers_old_path_for_renames() {
        assert_eq!(
            content_source(&DiffTarget::Staged, Side::Old, "new.rs", Some("old.rs")),
            show("HEAD", "old.rs")
        );
        assert_eq!(
            content_source(
                &DiffTarget::WorkingTree,
                Side::Old,
                "new.rs",
                Some("old.rs")
            ),
            show(":0", "old.rs")
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
            show("abc123", "a.rs")
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
            show("abc123^", "a.rs")
        );
    }

    #[test]
    fn new_side_ignores_old_path_even_for_renames() {
        // The new side always reads the current path; old_path only
        // matters on the old side.
        assert_eq!(
            content_source(&DiffTarget::Staged, Side::New, "new.rs", Some("old.rs")),
            show(":0", "new.rs")
        );
    }

    #[test]
    fn review_new_side_is_branch_blob() {
        let target = DiffTarget::Review {
            base: "main".to_string(),
            branch: "feature".to_string(),
        };
        assert_eq!(
            content_source(&target, Side::New, "a.rs", None),
            show("feature", "a.rs")
        );
    }

    #[test]
    fn review_old_side_is_base_blob_and_prefers_old_path_for_renames() {
        let target = DiffTarget::Review {
            base: "main".to_string(),
            branch: "feature".to_string(),
        };
        assert_eq!(
            content_source(&target, Side::Old, "a.rs", None),
            show("main", "a.rs")
        );
        assert_eq!(
            content_source(&target, Side::Old, "new.rs", Some("old.rs")),
            show("main", "old.rs")
        );
    }

    #[test]
    fn file_target_sources_the_worktree_on_both_sides() {
        let target = DiffTarget::File("docs/notes.md".to_string());
        // New side reads the target's own path (defensive fallback arm).
        assert_eq!(
            content_source(&target, Side::New, "a.rs", None),
            worktree("docs/notes.md")
        );
        // The old side is never asked for in practice (a synthesized
        // all-context file has no Removed lines) but stays total.
        assert_eq!(
            content_source(&target, Side::Old, "a.rs", None),
            worktree("a.rs")
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
        assert_eq!(
            fetch_content(&ops, &content_source(&target, Side::Old, "a.rs", None)),
            None
        );
        assert_eq!(
            fetch_content(&ops, &content_source(&target, Side::New, "a.rs", None)),
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

    /// Populates `cache` for `path`'s `side` under `target`, the way
    /// `rebuild_rows` does.
    fn populate(
        cache: &mut HighlightCache,
        highlighter: &mut Highlighter,
        ops: &dyn StageOps,
        target: &DiffTarget,
        path: &str,
        side: Side,
    ) {
        let source = content_source(target, side, path, None);
        populate_cache(cache, highlighter, Some(ops), &source, path);
    }

    #[test]
    fn populate_cache_only_fetches_once_per_source() {
        let ops = CountingOps {
            show_calls: std::cell::RefCell::new(0),
        };
        let mut cache = HighlightCache::default();
        let mut highlighter = Highlighter::new();
        let target = DiffTarget::Staged;

        for _ in 0..3 {
            populate(
                &mut cache,
                &mut highlighter,
                &ops,
                &target,
                "a.rs",
                Side::New,
            );
        }

        assert_eq!(*ops.show_calls.borrow(), 1);
        assert_eq!(cache.len(), 1);
        assert!(
            !cache
                .get(&content_source(&target, Side::New, "a.rs", None))
                .is_empty()
        );
    }

    #[test]
    fn one_blob_is_highlighted_once_however_many_targets_reach_it() {
        // The defect this catches: keying the cache by the *view* rather than
        // the blob, so opening the same commit from two places (or leaving a
        // commit view and coming back) re-fetches and re-highlights content
        // that is byte-identical and already cached.
        let ops = CountingOps {
            show_calls: std::cell::RefCell::new(0),
        };
        let mut cache = HighlightCache::default();
        let mut highlighter = Highlighter::new();
        let commit = DiffTarget::Commit("abc123".to_string());
        // The same blob `abc123:a.rs` is what a Range ending at that rev
        // reads on its new side, too.
        let range = DiffTarget::Range("main..abc123".to_string());

        populate(
            &mut cache,
            &mut highlighter,
            &ops,
            &commit,
            "a.rs",
            Side::New,
        );
        populate(
            &mut cache,
            &mut highlighter,
            &ops,
            &range,
            "a.rs",
            Side::New,
        );

        assert_eq!(*ops.show_calls.borrow(), 1);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn invalidate_path_drops_every_side_but_keeps_other_files() {
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
            populate(&mut cache, &mut highlighter, &ops, &target, path, side);
        }
        assert_eq!(cache.len(), 3);

        cache.invalidate_path("a.rs");
        // Both a.rs sides gone; b.rs untouched.
        assert!(!cache.has_path("a.rs"));
        assert!(cache.has_path("b.rs"));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn cache_evicts_least_recently_used_once_over_the_line_ceiling() {
        // The defect this catches: immutable-blob entries are never
        // invalidated, so without a ceiling a session paging through history
        // grows the cache without bound.
        struct BigOps;
        impl StageOps for BigOps {
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
            fn read_worktree_file(&self, _p: &str) -> Option<Vec<u8>> {
                None
            }
            fn show_file(&self, _spec: &str) -> Option<String> {
                // Two fifths of the ceiling per file: two entries coexist,
                // a third forces exactly one eviction — which is what makes
                // *which* one gets evicted observable.
                Some("let x = 1;\n".repeat(MAX_CACHED_LINES * 2 / 5))
            }
        }

        let ops = BigOps;
        let mut cache = HighlightCache::default();
        let mut highlighter = Highlighter::new();
        let target = DiffTarget::Staged;

        populate(
            &mut cache,
            &mut highlighter,
            &ops,
            &target,
            "a.rs",
            Side::New,
        );
        populate(
            &mut cache,
            &mut highlighter,
            &ops,
            &target,
            "b.rs",
            Side::New,
        );
        // Re-touch a.rs so b.rs is now the least recently used.
        populate(
            &mut cache,
            &mut highlighter,
            &ops,
            &target,
            "a.rs",
            Side::New,
        );
        populate(
            &mut cache,
            &mut highlighter,
            &ops,
            &target,
            "c.rs",
            Side::New,
        );

        assert!(cache.cached_lines() <= MAX_CACHED_LINES);
        assert!(cache.has_path("c.rs"), "the newest entry must survive");
        assert!(cache.has_path("a.rs"), "the re-touched entry must survive");
        assert!(!cache.has_path("b.rs"), "the stalest entry is evicted");
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
        let source = source_for(&DiffTarget::WorkingTree, Side::New, "new.rs", None, true)
            .expect("a synthetic file's new side always has a source");
        populate_cache(&mut cache, &mut highlighter, Some(&ops), &source, "new.rs");
        assert!(!cache.get(&source).is_empty());
    }

    #[test]
    fn synthetic_old_side_has_no_source_at_all() {
        assert_eq!(
            source_for(&DiffTarget::WorkingTree, Side::Old, "new.rs", None, true),
            None
        );
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
