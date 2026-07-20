//! Persisted-annotation schema: the serde shape a review session's
//! [`super::Annotation`]s are saved in as part of
//! `crate::review::store::PersistedReview`, and the snapshot/restore pair
//! that moves annotations between an in-memory [`super::AnnotationStore`]
//! and that shape.
//!
//! Lives here so `review/` stays free of annotation types: `review/`
//! composes a `Vec<PersistedAnnotation>` field without ever constructing or
//! interpreting one itself.
//!
//! **Accepted limitation:** restore reattaches an annotation to the exact
//! `path`/`line`/`side` (or hunk/range) recorded in its target, verbatim.
//! Line anchors are not re-mapped when the file's content has changed since
//! the annotation was made, so a stale anchor is tolerated rather than
//! re-resolved or dropped.

use serde::{Deserialize, Serialize};

use super::model::{Annotation, Classification, Source, Target};
use super::store::AnnotationStore;

/// One annotation's persisted shape: everything [`Annotation`] carries
/// except its `id`, which [`AnnotationStore`] assigns fresh on every
/// `add`/`add_with_source` call and is never meaningful to replay verbatim
/// — [`restore_all`] rebuilds ids from scratch, in the same order the
/// annotations were saved, so a restored session's ids are simply
/// `0..restored.len()` exactly as if they'd been typed in that order this
/// session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedAnnotation {
    pub target: Target,
    pub classification: Classification,
    pub body: String,
    /// `#[serde(default)]` so a hand-trimmed or future-minimal record still
    /// parses — mirrors `crate::review::store::PersistedFile::blob_sha`'s
    /// same allowance. Defaults to [`Source::WorkingTree`], matching
    /// [`AnnotationStore::add`]'s own default.
    #[serde(default)]
    pub source: Source,
    /// Whether this annotation was already published to the forge. Defaults
    /// to `false` and is omitted from the JSON entirely when `false`
    /// (`skip_serializing_if`), so an unpublished annotation — the only kind
    /// a non-forge review ever has — is on-disk-identical to one from before
    /// this field existed. The `default` half absorbs a record with no
    /// `published` key.
    #[serde(default, skip_serializing_if = "is_false")]
    pub published: bool,
    /// Whether a private GitLab draft note for this annotation exists
    /// server-side but is not yet published (a stopped submit run). Same
    /// serde compatibility contract as `published`: absent when `false`,
    /// defaulted when missing.
    #[serde(default, skip_serializing_if = "is_false")]
    pub draft_created: bool,
}

/// `skip_serializing_if` predicate: omit a `bool` field when it is `false`
/// (serde hands the closure a `&bool`). Keeps an unpublished annotation's
/// JSON free of a `"published": false` key, preserving the byte-exact shape
/// the stdout/persistence contracts lock.
fn is_false(b: &bool) -> bool {
    !*b
}

impl PersistedAnnotation {
    /// Captures `annotation`'s persistable fields, dropping its store-owned
    /// `id`.
    fn from_annotation(annotation: &Annotation) -> PersistedAnnotation {
        PersistedAnnotation {
            target: annotation.target.clone(),
            classification: annotation.classification,
            body: annotation.body.clone(),
            source: annotation.source.clone(),
            published: annotation.published,
            draft_created: annotation.draft_created,
        }
    }
}

/// Snapshots every annotation currently in `store`, in insertion order —
/// the shape the UI layer's review save path writes into a review's
/// persisted entry on every annotation add/edit/delete, alongside the
/// existing accept/defer save.
pub fn snapshot(store: &AnnotationStore) -> Vec<PersistedAnnotation> {
    store
        .iter()
        .map(PersistedAnnotation::from_annotation)
        .collect()
}

/// Replays `persisted` into `store`, in order, via
/// [`AnnotationStore::add_with_source`] — the session-start restore path:
/// annotations reattach to their recorded file/line anchors verbatim before
/// first render. A record whose body is empty after trimming (only
/// reachable from hand-edited or corrupt JSON — the compose modal already
/// rejects an empty body before anything reaches [`snapshot`]) is skipped
/// rather than failing the whole restore, so one bad record can't cost
/// every other annotation in the session. Returns the number of
/// annotations actually restored.
pub fn restore_all(store: &mut AnnotationStore, persisted: Vec<PersistedAnnotation>) -> usize {
    let mut restored = 0;
    for entry in persisted {
        let published = entry.published;
        let draft_created = entry.draft_created;
        if let Ok(id) =
            store.add_with_source(entry.target, entry.classification, entry.body, entry.source)
        {
            // `add_with_source` always creates an unpublished annotation, so
            // a restored already-published one is marked back afterward — the
            // dedupe against the forge's own copy depends on this surviving a
            // reopen. The draft-created flag survives the same way so a
            // resumed session's resubmit still skips existing drafts.
            if published {
                let _ = store.set_published(id, true);
            }
            if draft_created {
                let _ = store.set_draft_created(id, true);
            }
            restored += 1;
        }
    }
    restored
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotate::model::Side;

    #[test]
    fn snapshot_captures_every_annotation_in_insertion_order() {
        let mut store = AnnotationStore::new();
        store
            .add(Target::file("a.rs"), Classification::Nit, "first")
            .unwrap();
        store
            .add_with_source(
                Target::line("b.rs", 4, Side::New),
                Classification::Issue,
                "second",
                Source::Commit("abc1234".to_string()),
            )
            .unwrap();

        let snap = snapshot(&store);

        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].target, Target::file("a.rs"));
        assert_eq!(snap[0].classification, Classification::Nit);
        assert_eq!(snap[0].body, "first");
        assert_eq!(snap[0].source, Source::WorkingTree);
        assert_eq!(snap[1].target, Target::line("b.rs", 4, Side::New));
        assert_eq!(snap[1].source, Source::Commit("abc1234".to_string()));
    }

    #[test]
    fn restore_all_replays_into_a_fresh_store_with_sequential_ids() {
        let persisted = vec![
            PersistedAnnotation {
                target: Target::file("a.rs"),
                classification: Classification::Nit,
                body: "first".to_string(),
                source: Source::WorkingTree,
                published: false,
                draft_created: false,
            },
            PersistedAnnotation {
                target: Target::line("b.rs", 4, Side::New),
                classification: Classification::Issue,
                body: "second".to_string(),
                source: Source::Staged,
                published: false,
                draft_created: false,
            },
        ];
        let mut store = AnnotationStore::new();

        let restored = restore_all(&mut store, persisted);

        assert_eq!(restored, 2);
        let ids: Vec<usize> = store.iter().map(|a| a.id).collect();
        assert_eq!(ids, vec![0, 1]);
        let bodies: Vec<&str> = store.iter().map(|a| a.body.as_str()).collect();
        assert_eq!(bodies, vec!["first", "second"]);
    }

    #[test]
    fn snapshot_then_restore_round_trips_every_field() {
        let mut store = AnnotationStore::new();
        store
            .add_with_source(
                Target::range("src/lib.rs", 10, 20, Side::Old).unwrap(),
                Classification::Praise,
                "clean",
                Source::Range("main..feature".to_string()),
            )
            .unwrap();
        store
            .add(
                Target::worktree_range("docs/notes.md", 1, 2).unwrap(),
                Classification::Question,
                "stale?",
            )
            .unwrap();

        let snap = snapshot(&store);
        let mut restored_store = AnnotationStore::new();
        restore_all(&mut restored_store, snap);

        let original: Vec<_> = store
            .iter()
            .map(|a| {
                (
                    a.target.clone(),
                    a.classification,
                    a.body.clone(),
                    a.source.clone(),
                )
            })
            .collect();
        let restored: Vec<_> = restored_store
            .iter()
            .map(|a| {
                (
                    a.target.clone(),
                    a.classification,
                    a.body.clone(),
                    a.source.clone(),
                )
            })
            .collect();
        assert_eq!(original, restored);
    }

    #[test]
    fn restore_all_skips_a_record_whose_body_is_empty_after_trimming() {
        let persisted = vec![
            PersistedAnnotation {
                target: Target::file("a.rs"),
                classification: Classification::Nit,
                body: "   ".to_string(),
                source: Source::WorkingTree,
                published: false,
                draft_created: false,
            },
            PersistedAnnotation {
                target: Target::file("b.rs"),
                classification: Classification::Nit,
                body: "kept".to_string(),
                source: Source::WorkingTree,
                published: false,
                draft_created: false,
            },
        ];
        let mut store = AnnotationStore::new();

        let restored = restore_all(&mut store, persisted);

        assert_eq!(restored, 1);
        assert_eq!(store.len(), 1);
        assert_eq!(store.iter().next().unwrap().body, "kept");
    }

    // -- JSON shape (locks the schema this module writes to disk) -----------

    #[test]
    fn persisted_annotation_json_shape_is_stable() {
        let entry = PersistedAnnotation {
            target: Target::line("src/lib.rs", 10, Side::New),
            classification: Classification::Issue,
            body: "fix this".to_string(),
            source: Source::WorkingTree,
            published: false,
            draft_created: false,
        };
        let json = serde_json::to_string_pretty(&entry).unwrap();
        let expected = r#"{
  "target": {
    "kind": "line",
    "path": "src/lib.rs",
    "line": 10,
    "side": "new"
  },
  "classification": "issue",
  "body": "fix this",
  "source": "working_tree"
}"#;
        assert_eq!(json, expected);
        let round_tripped: PersistedAnnotation = serde_json::from_str(&json).unwrap();
        assert_eq!(round_tripped, entry);
    }

    #[test]
    fn persisted_annotation_missing_source_defaults_to_working_tree() {
        let json = r#"{
            "target": {"kind": "file", "path": "a.rs"},
            "classification": "nit",
            "body": "note"
        }"#;
        let entry: PersistedAnnotation = serde_json::from_str(json).unwrap();
        assert_eq!(entry.source, Source::WorkingTree);
    }

    // -- published flag ------------------------------------------------------

    #[test]
    fn persisted_annotation_missing_published_defaults_to_false() {
        let json = r#"{
            "target": {"kind": "file", "path": "a.rs"},
            "classification": "nit",
            "body": "note"
        }"#;
        let entry: PersistedAnnotation = serde_json::from_str(json).unwrap();
        assert!(
            !entry.published,
            "an annotation with no published key must default to unpublished"
        );
    }

    #[test]
    fn published_true_serializes_the_key_and_round_trips() {
        let entry = PersistedAnnotation {
            target: Target::line("src/lib.rs", 10, Side::New),
            classification: Classification::Issue,
            body: "fix this".to_string(),
            source: Source::WorkingTree,
            published: true,
            draft_created: false,
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(
            json.contains("\"published\":true"),
            "a published annotation must carry the flag on disk: {json}"
        );
        let round_tripped: PersistedAnnotation = serde_json::from_str(&json).unwrap();
        assert_eq!(round_tripped, entry);
    }

    #[test]
    fn draft_created_serializes_only_when_set_and_round_trips_through_restore() {
        let mut store = AnnotationStore::new();
        let id = store
            .add(Target::file("a.rs"), Classification::Nit, "pending draft")
            .unwrap();
        store.set_draft_created(id, true).unwrap();
        store
            .add(Target::file("b.rs"), Classification::Nit, "no draft")
            .unwrap();

        let snap = snapshot(&store);
        let json = serde_json::to_string(&snap[0]).unwrap();
        assert!(
            json.contains("\"draft_created\":true"),
            "flag on disk: {json}"
        );
        let json = serde_json::to_string(&snap[1]).unwrap();
        assert!(
            !json.contains("draft_created"),
            "unset flag must be omitted: {json}"
        );

        let mut restored = AnnotationStore::new();
        restore_all(&mut restored, snap);
        let flags: Vec<bool> = restored.iter().map(|a| a.draft_created).collect();
        assert_eq!(flags, vec![true, false]);
    }

    #[test]
    fn record_without_a_draft_created_key_defaults_to_false() {
        let json = r#"{
            "target": {"kind": "file", "path": "a.rs"},
            "classification": "nit",
            "body": "note"
        }"#;
        let entry: PersistedAnnotation = serde_json::from_str(json).unwrap();
        assert!(!entry.draft_created);
    }

    #[test]
    fn snapshot_and_restore_preserve_the_published_flag() {
        let mut store = AnnotationStore::new();
        let id = store
            .add(Target::file("a.rs"), Classification::Nit, "posted")
            .unwrap();
        store.set_published(id, true).unwrap();
        store
            .add(Target::file("b.rs"), Classification::Nit, "draft")
            .unwrap();

        let snap = snapshot(&store);
        assert!(
            snap[0].published,
            "the published annotation snapshots as such"
        );
        assert!(!snap[1].published);

        let mut restored = AnnotationStore::new();
        restore_all(&mut restored, snap);
        let flags: Vec<bool> = restored.iter().map(|a| a.published).collect();
        assert_eq!(
            flags,
            vec![true, false],
            "restore must replay the published flag verbatim"
        );
    }
}
