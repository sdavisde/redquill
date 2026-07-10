//! [`AnnotationStore`]: an in-memory, insertion-ordered collection of
//! [`Annotation`]s.

use super::model::{AnnotateError, Annotation, Classification, Target, validate_body};

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

    /// Adds a new annotation, validating the body, and returns its id.
    pub fn add(
        &mut self,
        target: Target,
        classification: Classification,
        body: impl Into<String>,
    ) -> Result<usize, AnnotateError> {
        let body = validate_body(&body.into())?;
        let id = self.next_id;
        self.next_id += 1;
        self.annotations.push(Annotation {
            id,
            target,
            classification,
            body,
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

    /// Iterates over annotations in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = &Annotation> {
        self.annotations.iter()
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
