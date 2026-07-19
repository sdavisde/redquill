//! Reusable `/` filter mode for list-like contexts (annotation list, staging
//! panel, accepted-files panel, switcher tabs): `/` enters filter mode,
//! printable characters build a fuzzy query narrowing a caller-supplied
//! label list, `Esc` clears the query and exits filter mode entirely,
//! `Enter` locks the filter so the list's own verbs (`e`, `d`, `Space`,
//! `Enter`, ...) act on the filtered view instead of the raw list.
//!
//! Matching/ranking delegates to [`crate::search::rank`] — the fuzzy file
//! finder's matcher — via synthetic [`FileCandidate`]s built from the
//! labels the caller passes: the `path` field is just whatever text
//! describes the row (an annotation's one-line summary, a staged file's
//! path, a branch/worktree name), not necessarily a real file path.
//!
//! Pure — no `App`/`Mode`/render types, so it's unit-testable without the
//! app. Each consuming context's integration (building labels from its own
//! data, translating a filtered position back to a real index for its
//! motions/verbs) lives beside that context: `annotation_list.rs`,
//! `staging.rs`, `switcher.rs`.
//!
//! **Deliberately diverges from the file finder's "empty query matches
//! nothing" convention**: here an empty query shows the *whole* list,
//! unfiltered (in original order) — the finder is a query-first free-text
//! overlay with nothing to show before you type anything (this component's
//! spec explicitly calls that a different, unrelated model); this filter
//! narrows a list that was already fully visible before `/` was pressed, so
//! going blank the instant filter mode opens would read as broken, not as
//! an empty state waiting to be filled in.

use crate::search::{FileCandidate, rank};

/// One list's `/`-filter session: the query buffer, whether it's still
/// being typed (`editing`) or has been locked in with `Enter`, and the
/// query's filtered/ranked view over the labels it was last reranked
/// against — a cache, like the file finder's own `matches` field
/// (recomputed on every keystroke via [`ListFilter::rerank`], not on every
/// motion or render).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ListFilter {
    query: String,
    editing: bool,
    indices: Vec<usize>,
}

impl ListFilter {
    /// Enters filter mode against `labels`: an empty query, editing, and
    /// the whole list visible in its original order (see the module doc on
    /// why an empty query isn't "no matches" here).
    pub(super) fn open(labels: &[String]) -> ListFilter {
        ListFilter {
            query: String::new(),
            editing: true,
            indices: (0..labels.len()).collect(),
        }
    }

    /// The current query text.
    pub(super) fn query(&self) -> &str {
        &self.query
    }

    /// Whether the query is still being typed (`true`) or has been locked
    /// in with `Enter` (`false`).
    pub(super) fn is_editing(&self) -> bool {
        self.editing
    }

    /// The number of rows in the filtered/ranked view.
    pub(super) fn len(&self) -> usize {
        self.indices.len()
    }

    /// Whether the filtered/ranked view has no rows (the empty-result
    /// state a caller renders as a hint line rather than a blank list).
    pub(super) fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// The filtered/ranked view: indices into the labels this filter was
    /// last reranked against, in match order (best first once a query is
    /// typed; original order when the query is empty).
    pub(super) fn indices(&self) -> &[usize] {
        &self.indices
    }

    /// Maps a position within the filtered view back to the real
    /// underlying index — the translation every motion/verb needs.
    pub(super) fn real_index(&self, filtered_pos: usize) -> Option<usize> {
        self.indices.get(filtered_pos).copied()
    }

    /// Appends `c` to the query and reranks against `labels`. A no-op once
    /// locked — `/` must resume editing first (see [`ListFilter::resume_editing`]),
    /// mirroring the help overlay's own filter convention.
    pub(super) fn push_char(&mut self, c: char, labels: &[String]) {
        if self.editing {
            self.query.push(c);
            self.rerank(labels);
        }
    }

    /// Removes the last character of the query and reranks against
    /// `labels`. A no-op once locked.
    pub(super) fn backspace(&mut self, labels: &[String]) {
        if self.editing {
            self.query.pop();
            self.rerank(labels);
        }
    }

    /// Re-opens editing on the existing query (`/` while locked).
    pub(super) fn resume_editing(&mut self) {
        self.editing = true;
    }

    /// Re-ranks against `labels` without changing the query — used after the
    /// underlying list mutates (e.g. a delete/unstage/accept while
    /// filtered), so the filtered view reflects the new list instead of
    /// going stale.
    pub(super) fn refresh(&mut self, labels: &[String]) {
        self.rerank(labels);
    }

    /// Locks the filter (`Enter` while editing), handing key handling back
    /// to the list's own verbs.
    pub(super) fn lock(&mut self) {
        self.editing = false;
    }

    fn rerank(&mut self, labels: &[String]) {
        self.indices = filtered_indices(labels, &self.query);
    }
}

/// The active-filter indicator text every adopting context's chrome renders
/// (in `theme.search_prompt`'s color, per the spec's design note reusing the
/// help overlay's own filter-line styling): a live `/query` while editing,
/// or a locked reminder once `Enter` has confirmed it — the same two-shape
/// convention `help::render`'s subtitle uses for its own `/` filter.
pub(super) fn chrome_text(filter: &ListFilter) -> String {
    if filter.is_editing() {
        format!("/{}", filter.query())
    } else {
        format!(
            "filter: /{}  (/ to edit \u{00b7} esc to clear)",
            filter.query()
        )
    }
}

/// The empty-result hint line (FR-9): the query plus a reminder that `Esc`
/// clears it, shown in place of a blank list when a locked, non-empty
/// filter matches nothing.
pub(super) fn empty_hint(filter: &ListFilter) -> String {
    format!(
        "no matches for \"{}\" \u{2014} esc to clear",
        filter.query()
    )
}

/// Ranks `labels` against `query`, returning indices into `labels` in
/// filtered/ranked order. An empty query is the whole list, in original
/// index order (see the module doc's divergence note); a non-empty query
/// delegates to [`crate::search::rank`] via synthetic [`FileCandidate`]s
/// built from `labels`.
pub(super) fn filtered_indices(labels: &[String], query: &str) -> Vec<usize> {
    if query.is_empty() {
        return (0..labels.len()).collect();
    }
    let candidates: Vec<FileCandidate> = labels
        .iter()
        .map(|label| FileCandidate {
            path: label.clone(),
        })
        .collect();
    rank(&candidates, query)
        .into_iter()
        .map(|m| m.index)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    // -- filtered_indices ----------------------------------------------------

    #[test]
    fn empty_query_returns_the_whole_list_in_original_order() {
        let l = labels(&["zebra", "apple", "mango"]);
        assert_eq!(filtered_indices(&l, ""), vec![0, 1, 2]);
    }

    #[test]
    fn non_empty_query_ranks_via_the_shared_fuzzy_matcher() {
        let l = labels(&["src/main.rs", "README.md"]);
        assert_eq!(filtered_indices(&l, "main"), vec![0]);
    }

    #[test]
    fn a_query_with_no_matches_yields_an_empty_list() {
        let l = labels(&["one", "two"]);
        assert!(filtered_indices(&l, "zzzzz").is_empty());
    }

    #[test]
    fn narrower_query_reranks_to_fewer_results() {
        let l = labels(&["annotate.rs", "annotation_list.rs", "app.rs"]);
        assert_eq!(filtered_indices(&l, "ann").len(), 2);
        assert_eq!(filtered_indices(&l, "annotation_list").len(), 1);
    }

    // -- ListFilter::open ------------------------------------------------------

    #[test]
    fn open_starts_editing_with_an_empty_query_and_the_whole_list() {
        let l = labels(&["a", "b", "c"]);
        let f = ListFilter::open(&l);
        assert!(f.is_editing());
        assert_eq!(f.query(), "");
        assert_eq!(f.indices(), &[0, 1, 2]);
        assert_eq!(f.len(), 3);
        assert!(!f.is_empty());
    }

    #[test]
    fn open_on_an_empty_list_is_the_empty_state() {
        let f = ListFilter::open(&[]);
        assert!(f.is_empty());
        assert_eq!(f.len(), 0);
    }

    // -- push_char / backspace: rerank on every keystroke -----------------------

    #[test]
    fn push_char_narrows_and_reranks() {
        let l = labels(&["src/main.rs", "README.md"]);
        let mut f = ListFilter::open(&l);
        for c in "main".chars() {
            f.push_char(c, &l);
        }
        assert_eq!(f.query(), "main");
        assert_eq!(f.indices(), &[0]);
    }

    #[test]
    fn backspace_widens_the_query_and_reranks() {
        let l = labels(&["src/main.rs", "README.md"]);
        let mut f = ListFilter::open(&l);
        for c in "mainx".chars() {
            f.push_char(c, &l);
        }
        assert!(f.is_empty(), "no path matches \"mainx\"");
        f.backspace(&l);
        assert_eq!(f.query(), "main");
        assert_eq!(f.indices(), &[0]);
    }

    #[test]
    fn push_char_and_backspace_are_no_ops_once_locked() {
        let l = labels(&["one", "two"]);
        let mut f = ListFilter::open(&l);
        f.push_char('o', &l);
        f.lock();
        f.push_char('n', &l);
        assert_eq!(f.query(), "o", "locked filter must not keep typing");
        f.backspace(&l);
        assert_eq!(f.query(), "o", "locked filter must not delete either");
    }

    // -- lock / resume_editing ---------------------------------------------------

    #[test]
    fn lock_stops_editing_without_changing_the_query_or_indices() {
        let l = labels(&["one", "two"]);
        let mut f = ListFilter::open(&l);
        f.push_char('o', &l);
        let before = f.indices().to_vec();
        f.lock();
        assert!(!f.is_editing());
        assert_eq!(f.query(), "o");
        assert_eq!(f.indices(), before.as_slice());
    }

    #[test]
    fn resume_editing_re_enables_typing_on_a_locked_filter() {
        let l = labels(&["one", "two"]);
        let mut f = ListFilter::open(&l);
        f.lock();
        f.resume_editing();
        assert!(f.is_editing());
        f.push_char('o', &l);
        assert_eq!(f.query(), "o");
    }

    // -- real_index: filtered position -> underlying index -----------------------

    #[test]
    fn real_index_maps_a_filtered_position_back_to_the_underlying_list() {
        let l = labels(&["zzz", "banana", "zzzbanana"]);
        let mut f = ListFilter::open(&l);
        for c in "banana".chars() {
            f.push_char(c, &l);
        }
        // The exact match ("banana", index 1) ranks above the looser one.
        assert_eq!(f.real_index(0), Some(1));
        assert_eq!(f.real_index(2), None);
    }

    #[test]
    fn real_index_is_identity_at_an_empty_query() {
        let l = labels(&["a", "b", "c"]);
        let f = ListFilter::open(&l);
        assert_eq!(f.real_index(2), Some(2));
    }

    // -- chrome_text / empty_hint ------------------------------------------------

    #[test]
    fn chrome_text_shows_a_live_query_while_editing() {
        let l = labels(&["a"]);
        let mut f = ListFilter::open(&l);
        f.push_char('x', &l);
        assert_eq!(chrome_text(&f), "/x");
    }

    #[test]
    fn chrome_text_shows_the_locked_reminder_once_locked() {
        let l = labels(&["a"]);
        let mut f = ListFilter::open(&l);
        f.push_char('x', &l);
        f.lock();
        assert_eq!(
            chrome_text(&f),
            "filter: /x  (/ to edit \u{00b7} esc to clear)"
        );
    }

    #[test]
    fn empty_hint_names_the_query() {
        let l = labels(&["a"]);
        let mut f = ListFilter::open(&l);
        f.push_char('z', &l);
        assert_eq!(empty_hint(&f), "no matches for \"z\" \u{2014} esc to clear");
    }
}
