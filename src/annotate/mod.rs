//! The annotation model, its persistence, and stdout serialization.
//!
//! - [`model`] — [`Classification`], [`Side`], [`Target`], [`Source`],
//!   [`Annotation`], and [`AnnotateError`].
//! - [`store`] — [`AnnotationStore`], an in-memory, insertion-ordered
//!   collection of annotations with add/remove/edit/iter/for_path.
//! - [`markdown`] — [`render_markdown`], which emits the public-contract
//!   markdown format (`## path/to/file.rs:LINE (+)` header, comment body
//!   below) that the UI writes to stdout on quit.
//! - [`persist`] — [`PersistedAnnotation`] plus [`snapshot`]/[`restore_all`]
//!   (spec 08 Unit 6): the serde shape a review session's annotations are
//!   saved in, composed into `crate::review::store::PersistedReview`, and
//!   the pair of functions that move annotations between that shape and a
//!   live [`AnnotationStore`].

mod markdown;
mod model;
mod persist;
mod store;

pub use markdown::render_markdown;
pub use model::{AnnotateError, Annotation, Classification, Side, Source, Target};
pub use persist::{PersistedAnnotation, restore_all, snapshot};
pub use store::AnnotationStore;
