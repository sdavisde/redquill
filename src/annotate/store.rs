//! [`AnnotationStore`]: an in-memory, insertion-ordered collection of
//! [`Annotation`]s.

use super::model::{AnnotateError, Annotation, Classification, Source, Target, validate_body};

/// An in-memory, insertion-ordered collection of annotations.
///
/// Ids are stable ordinals assigned by `add`, starting at 0 and never
/// reused, even after `remove`.
#[derive(Debug, Default, Clone)]
pub struct AnnotationStore {
    annotations: Vec<Annotation>,
    next_id: usize,
}

impl AnnotationStore {
    /// Creates an empty store.
    pub fn new() -> AnnotationStore {
        AnnotationStore::default()
    }

    /// Adds a new annotation authored against the default working-tree
    /// source, validating the body, and returns its id. Equivalent to
    /// [`AnnotationStore::add_with_source`] with `Source::WorkingTree` — the
    /// vast majority of call sites (every existing test fixture, and any
    /// future working-tree-only session) never need to think about sources
    /// at all.
    pub fn add(
        &mut self,
        target: Target,
        classification: Classification,
        body: impl Into<String>,
    ) -> Result<usize, AnnotateError> {
        self.add_with_source(target, classification, body, Source::WorkingTree)
    }

    /// Adds a new annotation authored against `source`, validating the body,
    /// and returns its id. Callers outside the working-tree flow (e.g. the
    /// compose modal when a commit/range/staged view is active) use this so
    /// the emitted markdown can group and label non-worktree annotations —
    /// see `crate::annotate::markdown`.
    pub fn add_with_source(
        &mut self,
        target: Target,
        classification: Classification,
        body: impl Into<String>,
        source: Source,
    ) -> Result<usize, AnnotateError> {
        let body = validate_body(&body.into())?;
        let id = self.next_id;
        self.next_id += 1;
        self.annotations.push(Annotation {
            id,
            target,
            classification,
            body,
            source,
            published: false,
        });
        Ok(id)
    }

    /// Removes the annotation with the given id.
    pub fn remove(&mut self, id: usize) -> Result<(), AnnotateError> {
        let index = self
            .annotations
            .iter()
            .position(|a| a.id == id)
            .ok_or(AnnotateError::NotFound(id))?;
        self.annotations.remove(index);
        Ok(())
    }

    /// Replaces the body of the annotation with the given id.
    pub fn edit(&mut self, id: usize, new_body: impl Into<String>) -> Result<(), AnnotateError> {
        let body = validate_body(&new_body.into())?;
        let annotation = self
            .annotations
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or(AnnotateError::NotFound(id))?;
        annotation.body = body;
        Ok(())
    }

    /// Replaces the classification of the annotation with the given id.
    ///
    /// Additive alongside [`AnnotationStore::edit`] (which only ever
    /// changes the body) so the compose modal can re-classify an existing
    /// annotation without touching the locked `edit` contract.
    pub fn set_classification(
        &mut self,
        id: usize,
        classification: Classification,
    ) -> Result<(), AnnotateError> {
        let annotation = self
            .annotations
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or(AnnotateError::NotFound(id))?;
        annotation.classification = classification;
        Ok(())
    }

    /// Sets the published flag of the annotation with the given id — used
    /// both by the submit flow (marking an annotation published on a
    /// successful post) and by the session-start restore path (replaying a
    /// persisted published state). Additive alongside [`AnnotationStore::edit`]
    /// so neither the body-edit nor the classification-edit contract is
    /// touched.
    pub fn set_published(&mut self, id: usize, published: bool) -> Result<(), AnnotateError> {
        let annotation = self
            .annotations
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or(AnnotateError::NotFound(id))?;
        annotation.published = published;
        Ok(())
    }

    /// Iterates over annotations in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = &Annotation> {
        self.annotations.iter()
    }

    /// A clone of this store with every annotation whose id is in
    /// `suppressed` dropped, preserving all surviving annotations' ids (and
    /// the store's `next_id`) verbatim. The presentation layer uses this to
    /// hand the row builder a view that omits published annotations already
    /// shown by the forge's own copy, without disturbing the real store the
    /// list panel, editing, and stdout serialization all read from. Cheap
    /// and allocation-light; the common (empty-`suppressed`) case is a plain
    /// clone.
    pub fn without_ids(&self, suppressed: &std::collections::HashSet<usize>) -> AnnotationStore {
        AnnotationStore {
            annotations: self
                .annotations
                .iter()
                .filter(|a| !suppressed.contains(&a.id))
                .cloned()
                .collect(),
            next_id: self.next_id,
        }
    }

    /// The number of annotations currently in the store.
    pub fn len(&self) -> usize {
        self.annotations.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.annotations.is_empty()
    }

    /// Iterates, in insertion order, over annotations whose target path
    /// equals `path`.
    pub fn for_path<'a>(&'a self, path: &'a str) -> impl Iterator<Item = &'a Annotation> {
        self.annotations
            .iter()
            .filter(move |a| a.target.path() == path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotate::model::Side;

    #[test]
    fn add_records_working_tree_as_the_default_source() {
        let mut store = AnnotationStore::new();
        store
            .add(Target::file("a.rs"), Classification::Nit, "note")
            .unwrap();
        assert_eq!(store.iter().next().unwrap().source, Source::WorkingTree);
    }

    #[test]
    fn add_with_source_records_the_given_source() {
        let mut store = AnnotationStore::new();
        store
            .add_with_source(
                Target::file("a.rs"),
                Classification::Nit,
                "note",
                Source::Commit("abc1234".to_string()),
            )
            .unwrap();
        assert_eq!(
            store.iter().next().unwrap().source,
            Source::Commit("abc1234".to_string())
        );
    }

    #[test]
    fn add_assigns_sequential_ids_starting_at_zero() {
        let mut store = AnnotationStore::new();
        let id0 = store
            .add(Target::file("a.rs"), Classification::Nit, "first")
            .unwrap();
        let id1 = store
            .add(Target::file("b.rs"), Classification::Issue, "second")
            .unwrap();
        assert_eq!(id0, 0);
        assert_eq!(id1, 1);
    }

    #[test]
    fn add_rejects_empty_body() {
        let mut store = AnnotationStore::new();
        let err = store
            .add(Target::file("a.rs"), Classification::Nit, "   ")
            .unwrap_err();
        assert_eq!(err, AnnotateError::EmptyBody);
        assert!(store.is_empty());
    }

    #[test]
    fn len_and_is_empty_track_contents() {
        let mut store = AnnotationStore::new();
        assert_eq!(store.len(), 0);
        assert!(store.is_empty());
        store
            .add(Target::file("a.rs"), Classification::Nit, "note")
            .unwrap();
        assert_eq!(store.len(), 1);
        assert!(!store.is_empty());
    }

    #[test]
    fn iter_preserves_insertion_order() {
        let mut store = AnnotationStore::new();
        store
            .add(Target::file("a.rs"), Classification::Nit, "one")
            .unwrap();
        store
            .add(Target::file("b.rs"), Classification::Issue, "two")
            .unwrap();
        store
            .add(Target::file("c.rs"), Classification::Praise, "three")
            .unwrap();
        let bodies: Vec<&str> = store.iter().map(|a| a.body.as_str()).collect();
        assert_eq!(bodies, vec!["one", "two", "three"]);
    }

    #[test]
    fn remove_deletes_and_preserves_remaining_order() {
        let mut store = AnnotationStore::new();
        let id0 = store
            .add(Target::file("a.rs"), Classification::Nit, "one")
            .unwrap();
        let id1 = store
            .add(Target::file("b.rs"), Classification::Issue, "two")
            .unwrap();
        store.remove(id0).unwrap();
        assert_eq!(store.len(), 1);
        let remaining: Vec<usize> = store.iter().map(|a| a.id).collect();
        assert_eq!(remaining, vec![id1]);
    }

    #[test]
    fn remove_unknown_id_errors() {
        let mut store = AnnotationStore::new();
        assert_eq!(store.remove(42), Err(AnnotateError::NotFound(42)));
    }

    #[test]
    fn remove_does_not_reuse_ids() {
        let mut store = AnnotationStore::new();
        let id0 = store
            .add(Target::file("a.rs"), Classification::Nit, "one")
            .unwrap();
        store.remove(id0).unwrap();
        let id1 = store
            .add(Target::file("b.rs"), Classification::Nit, "two")
            .unwrap();
        assert_ne!(id0, id1);
        assert_eq!(id1, 1);
    }

    #[test]
    fn edit_replaces_body() {
        let mut store = AnnotationStore::new();
        let id = store
            .add(Target::file("a.rs"), Classification::Nit, "old")
            .unwrap();
        store.edit(id, "new").unwrap();
        assert_eq!(store.iter().next().unwrap().body, "new");
    }

    #[test]
    fn edit_rejects_empty_body() {
        let mut store = AnnotationStore::new();
        let id = store
            .add(Target::file("a.rs"), Classification::Nit, "old")
            .unwrap();
        assert_eq!(store.edit(id, "  "), Err(AnnotateError::EmptyBody));
        assert_eq!(store.iter().next().unwrap().body, "old");
    }

    #[test]
    fn edit_unknown_id_errors() {
        let mut store = AnnotationStore::new();
        assert_eq!(store.edit(7, "x"), Err(AnnotateError::NotFound(7)));
    }

    #[test]
    fn set_classification_replaces_classification() {
        let mut store = AnnotationStore::new();
        let id = store
            .add(Target::file("a.rs"), Classification::Nit, "body")
            .unwrap();
        store
            .set_classification(id, Classification::Praise)
            .unwrap();
        assert_eq!(
            store.iter().next().unwrap().classification,
            Classification::Praise
        );
    }

    #[test]
    fn set_classification_unknown_id_errors() {
        let mut store = AnnotationStore::new();
        assert_eq!(
            store.set_classification(9, Classification::Issue),
            Err(AnnotateError::NotFound(9))
        );
    }

    #[test]
    fn for_path_filters_by_target_path() {
        let mut store = AnnotationStore::new();
        store
            .add(Target::file("a.rs"), Classification::Nit, "a-note")
            .unwrap();
        store
            .add(
                Target::line("b.rs", 3, Side::New),
                Classification::Issue,
                "b-note",
            )
            .unwrap();
        store
            .add(Target::file("a.rs"), Classification::Praise, "a-note-2")
            .unwrap();
        let bodies: Vec<&str> = store.for_path("a.rs").map(|a| a.body.as_str()).collect();
        assert_eq!(bodies, vec!["a-note", "a-note-2"]);
    }

    #[test]
    fn for_path_with_no_matches_is_empty() {
        let mut store = AnnotationStore::new();
        store
            .add(Target::file("a.rs"), Classification::Nit, "note")
            .unwrap();
        assert_eq!(store.for_path("missing.rs").count(), 0);
    }
}
