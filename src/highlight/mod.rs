//! Pure tree-sitter syntax highlighting engine.
//!
//! No TUI types here — this module turns `(language, file contents)` into
//! per-line lists of byte-range spans tagged with a small semantic
//! [`TokenKind`] palette, for the diff renderer to map onto colors. See
//! [`Highlighter::highlight_lines`] for the entry point and its
//! degrade-never-panic-never-error contract.

mod engine;
mod lang;
mod token_kind;

pub use engine::Highlighter;
pub use lang::Lang;
pub use token_kind::{TokenKind, capture_name_to_kind};
