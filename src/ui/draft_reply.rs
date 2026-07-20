//! Locally drafted replies to imported PR threads: the reviewer's queued
//! responses to teammates' existing comments, held in an in-memory,
//! insertion-ordered store entirely separate from two neighbors it must not
//! be confused with:
//!
//! - `crate::annotate::AnnotationStore` (the reviewer's own line/file
//!   annotations) — replies are kept out of it so they never reach the
//!   stdout markdown stream, which serializes only annotations.
//! - `crate::forge::ThreadOverlayStore` (teammates' fetched comments) — that
//!   store is read-only live-fetch context; a draft reply is the reviewer's
//!   own unsent text.
//!
//! A reply targets a thread's root comment id and is persisted in schema v3
//! as a `(thread_id, body)` pair (see
//! [`crate::review::store::PersistedReply`]) so a resumed session keeps it,
//! then published post-submit alongside annotations (a later unit).

use crate::review::store::PersistedReply;

/// One drafted reply: a store-assigned ordinal, the thread it answers, and
/// the body text (guaranteed non-empty after trimming by [`DraftReplyStore::add`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftReply {
    /// Stable id assigned by the owning [`DraftReplyStore`], in insertion
    /// order; never reused, even after `remove` (mirrors
    /// `AnnotationStore`'s id discipline).
    pub id: usize,
    /// The root comment id of the thread this reply targets.
    pub thread_id: u64,
    /// The reply body, non-empty after trimming.
    pub body: String,
    /// Whether this reply has already been published to the forge. `false`
    /// for a freshly drafted reply; set once the review is submitted so it
    /// is excluded from future submits — mirrors
    /// [`crate::annotate::Annotation::published`].
    pub published: bool,
    /// Whether a private GitLab draft note for this reply exists
    /// server-side but is not yet published — mirrors
    /// [`crate::annotate::Annotation::draft_created`].
    pub draft_created: bool,
}

/// An in-memory, insertion-ordered collection of draft replies.
#[derive(Debug, Default, Clone)]
pub struct DraftReplyStore {
    replies: Vec<DraftReply>,
    next_id: usize,
}

impl DraftReplyStore {
    /// Creates an empty store.
    pub fn new() -> DraftReplyStore {
        DraftReplyStore::default()
    }

    /// Adds a reply to thread `thread_id`, trimming the body and rejecting an
    /// empty result (returns `None`, matching the compose flow's "an
    /// abandoned body just cancels" semantics rather than erroring). Returns
    /// the new reply's id on success.
    pub fn add(&mut self, thread_id: u64, body: impl Into<String>) -> Option<usize> {
        let body = body.into();
        let trimmed = body.trim();
        if trimmed.is_empty() {
            return None;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.replies.push(DraftReply {
            id,
            thread_id,
            body: trimmed.to_string(),
            published: false,
            draft_created: false,
        });
        Some(id)
    }

    /// Replaces the body of the reply with `id`, trimming and rejecting an
    /// empty result. Returns whether a reply was updated.
    pub fn edit(&mut self, id: usize, new_body: impl Into<String>) -> bool {
        let body = new_body.into();
        let trimmed = body.trim();
        if trimmed.is_empty() {
            return false;
        }
        match self.replies.iter_mut().find(|r| r.id == id) {
            Some(reply) => {
                reply.body = trimmed.to_string();
                true
            }
            None => false,
        }
    }

    /// Removes the reply with `id`. Returns whether one was removed.
    pub fn remove(&mut self, id: usize) -> bool {
        match self.replies.iter().position(|r| r.id == id) {
            Some(index) => {
                self.replies.remove(index);
                true
            }
            None => false,
        }
    }

    /// The reply with `id`, if present.
    pub fn get(&self, id: usize) -> Option<&DraftReply> {
        self.replies.iter().find(|r| r.id == id)
    }

    /// Sets the published flag of the reply with `id` — used both by the
    /// submit flow (marking a reply published on a successful post) and by
    /// the session-start restore path (replaying a persisted published
    /// state). Returns whether a reply was updated. Mirrors
    /// `AnnotationStore::set_published`.
    pub fn set_published(&mut self, id: usize, published: bool) -> bool {
        match self.replies.iter_mut().find(|r| r.id == id) {
            Some(reply) => {
                reply.published = published;
                true
            }
            None => false,
        }
    }

    /// Sets the draft-created flag of the reply with `id` — the submit
    /// flow's record that a private GitLab draft of this reply exists
    /// server-side, so a resubmit skips re-creating it. Mirrors
    /// [`DraftReplyStore::set_published`].
    pub fn set_draft_created(&mut self, id: usize, draft_created: bool) -> bool {
        match self.replies.iter_mut().find(|r| r.id == id) {
            Some(reply) => {
                reply.draft_created = draft_created;
                true
            }
            None => false,
        }
    }

    /// Iterates over replies in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = &DraftReply> {
        self.replies.iter()
    }

    /// Iterates, in insertion order, over replies not yet published to the
    /// forge — the reply counterpart to `AnnotationStore::unpublished`, so a
    /// re-submit after a prior success sends only what's left. Exercised
    /// only by tests today; the submit-sequence driver (a later unit) is its
    /// first production caller, hence the non-test-only `allow`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn unpublished(&self) -> impl Iterator<Item = &DraftReply> {
        self.replies.iter().filter(|r| !r.published)
    }

    /// The number of drafted replies.
    pub fn len(&self) -> usize {
        self.replies.len()
    }

    /// Whether the store holds no replies.
    pub fn is_empty(&self) -> bool {
        self.replies.is_empty()
    }

    /// Snapshots every reply as a persisted `(thread_id, body)` pair, in
    /// insertion order — the shape the review save path writes into a
    /// review's persisted entry alongside the annotation snapshot.
    pub fn snapshot(&self) -> Vec<PersistedReply> {
        self.replies
            .iter()
            .map(|r| PersistedReply {
                thread_id: r.thread_id,
                body: r.body.clone(),
                published: r.published,
                draft_created: r.draft_created,
            })
            .collect()
    }

    /// Replays `persisted` into the store, in order, assigning fresh
    /// sequential ids — the session-start restore path. A record whose body
    /// is empty after trimming (only reachable from hand-edited JSON) is
    /// skipped rather than failing the whole restore. `add` always creates
    /// an unpublished reply, so a restored already-published one is marked
    /// back afterward — mirrors `crate::annotate::persist::restore_all`.
    /// Returns the number actually restored.
    pub fn restore(&mut self, persisted: Vec<PersistedReply>) -> usize {
        let mut restored = 0;
        for entry in persisted {
            let published = entry.published;
            let draft_created = entry.draft_created;
            if let Some(id) = self.add(entry.thread_id, entry.body) {
                if published {
                    self.set_published(id, true);
                }
                if draft_created {
                    self.set_draft_created(id, true);
                }
                restored += 1;
            }
        }
        restored
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_assigns_sequential_ids_and_trims() {
        let mut store = DraftReplyStore::new();
        let id0 = store.add(10, "  first  ").unwrap();
        let id1 = store.add(20, "second").unwrap();
        assert_eq!((id0, id1), (0, 1));
        let bodies: Vec<&str> = store.iter().map(|r| r.body.as_str()).collect();
        assert_eq!(bodies, vec!["first", "second"]);
        assert_eq!(store.get(id0).unwrap().thread_id, 10);
    }

    #[test]
    fn add_rejects_empty_body() {
        let mut store = DraftReplyStore::new();
        assert_eq!(store.add(1, "   \n\t"), None);
        assert!(store.is_empty());
    }

    #[test]
    fn edit_replaces_body_and_rejects_empty() {
        let mut store = DraftReplyStore::new();
        let id = store.add(1, "old").unwrap();
        assert!(store.edit(id, "new"));
        assert_eq!(store.get(id).unwrap().body, "new");
        assert!(!store.edit(id, "   "));
        assert_eq!(store.get(id).unwrap().body, "new");
        assert!(!store.edit(999, "x"));
    }

    #[test]
    fn remove_deletes_without_reusing_ids() {
        let mut store = DraftReplyStore::new();
        let id0 = store.add(1, "a").unwrap();
        assert!(store.remove(id0));
        let id1 = store.add(2, "b").unwrap();
        assert_ne!(id0, id1);
        assert_eq!(id1, 1);
        assert!(!store.remove(id0));
    }

    #[test]
    fn snapshot_then_restore_round_trips_thread_id_and_body() {
        let mut store = DraftReplyStore::new();
        store.add(100, "agreed").unwrap();
        store.add(200, "why?").unwrap();

        let snap = store.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].thread_id, 100);
        assert_eq!(snap[0].body, "agreed");

        let mut restored = DraftReplyStore::new();
        let n = restored.restore(snap);
        assert_eq!(n, 2);
        let pairs: Vec<(u64, &str)> = restored
            .iter()
            .map(|r| (r.thread_id, r.body.as_str()))
            .collect();
        assert_eq!(pairs, vec![(100, "agreed"), (200, "why?")]);
    }

    #[test]
    fn restore_skips_empty_bodies() {
        let mut store = DraftReplyStore::new();
        let n = store.restore(vec![
            PersistedReply {
                thread_id: 1,
                body: "  ".to_string(),
                published: false,
                draft_created: false,
            },
            PersistedReply {
                thread_id: 2,
                body: "kept".to_string(),
                published: false,
                draft_created: false,
            },
        ]);
        assert_eq!(n, 1);
        assert_eq!(store.len(), 1);
        assert_eq!(store.iter().next().unwrap().body, "kept");
    }

    // -- published flag -------------------------------------------------------

    #[test]
    fn add_creates_an_unpublished_reply() {
        let mut store = DraftReplyStore::new();
        let id = store.add(1, "note").unwrap();
        assert!(!store.get(id).unwrap().published);
    }

    #[test]
    fn set_published_flips_the_flag_and_reports_unknown_ids() {
        let mut store = DraftReplyStore::new();
        let id = store.add(1, "note").unwrap();
        assert!(store.set_published(id, true));
        assert!(store.get(id).unwrap().published);
        assert!(!store.set_published(999, true));
    }

    #[test]
    fn unpublished_excludes_replies_marked_published() {
        let mut store = DraftReplyStore::new();
        let id0 = store.add(1, "sent already").unwrap();
        let id1 = store.add(2, "still draft").unwrap();
        store.set_published(id0, true);

        let remaining: Vec<usize> = store.unpublished().map(|r| r.id).collect();
        assert_eq!(remaining, vec![id1]);
    }

    #[test]
    fn snapshot_and_restore_preserve_the_draft_created_flag() {
        let mut store = DraftReplyStore::new();
        let drafted = store.add(1, "draft exists").unwrap();
        store.set_draft_created(drafted, true);
        store.add(2, "no draft").unwrap();

        let snap = store.snapshot();
        assert!(snap[0].draft_created);
        assert!(!snap[1].draft_created);

        let mut restored = DraftReplyStore::new();
        restored.restore(snap);
        let flags: Vec<bool> = restored.iter().map(|r| r.draft_created).collect();
        assert_eq!(flags, vec![true, false]);
    }

    #[test]
    fn snapshot_and_restore_preserve_the_published_flag() {
        let mut store = DraftReplyStore::new();
        let id0 = store.add(1, "sent already").unwrap();
        store.add(2, "still draft").unwrap();
        store.set_published(id0, true);

        let snap = store.snapshot();
        assert!(snap[0].published);
        assert!(!snap[1].published);

        let mut restored = DraftReplyStore::new();
        restored.restore(snap);
        let flags: Vec<bool> = restored.iter().map(|r| r.published).collect();
        assert_eq!(flags, vec![true, false]);
    }

    /// Draft replies live entirely outside the annotation store, so the
    /// stdout markdown emitted on quit — which takes only an
    /// `AnnotationStore` — is provably unaffected by any number of drafted
    /// replies. The regression guard mirrors
    /// `crate::forge::threads`' `fetched_threads_never_change_annotation_markdown_output`.
    #[test]
    fn draft_replies_never_change_annotation_markdown_output() {
        use crate::annotate::{AnnotationStore, Classification, Target, render_markdown};

        let mut annotations = AnnotationStore::new();
        annotations
            .add(
                Target::file("src/a.rs"),
                Classification::Issue,
                "please fix",
            )
            .unwrap();
        let without_replies = render_markdown(&annotations);

        let mut store = DraftReplyStore::new();
        store
            .add(42, "a reply that must never reach stdout")
            .unwrap();
        store.add(43, "nor this one").unwrap();
        assert!(!store.is_empty());

        // `render_markdown` takes only the annotation store; there is no path
        // by which a drafted reply can reach the output.
        let with_replies = render_markdown(&annotations);
        assert_eq!(without_replies, with_replies);
    }
}
