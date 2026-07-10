//! The annotation model, its persistence, and stdout serialization.
//!
//! - [`model`] — [`Classification`], [`Side`], [`Target`], [`Annotation`],
//!   and [`AnnotateError`].
//! - [`store`] — [`AnnotationStore`], an in-memory, insertion-ordered
//!   collection of annotations with add/remove/edit/iter/for_path.
//! - [`markdown`] — [`render_markdown`], which emits the public-contract
//!   markdown format (`## path/to/file.rs:LINE (+)` header, comment body
//!   below) that the future UI writes to stdout on quit.

mod markdown;
mod model;
mod store;

pub use markdown::render_markdown;
pub use model::{AnnotateError, Annotation, Classification, Side, Target};
pub use store::AnnotationStore;
