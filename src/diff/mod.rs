//! The diff model: files, hunks, lines, and intra-line word diff. Pure data
//! and transforms with no I/O or TUI dependencies; heavily unit-tested.
//!
//! - [`parse_hunks`] turns one file's raw unified-diff patch text (as
//!   produced by [`crate::git::split_patches`]) into structured [`Hunk`]s
//!   with per-side line numbers.
//! - [`FileDiff::from_patch`] combines the git module's
//!   [`crate::git::RawFilePatch`] metadata with parsed hunks, deriving a
//!   [`FileChangeKind`].
//! - [`word_diff`] and [`pair_hunk_lines`] compute word-level intra-line
//!   highlights for paired removed/added lines.

mod error;
mod file;
mod hunk;
mod line;
mod word;

pub use error::DiffParseError;
pub use file::{FileChangeKind, FileDiff};
pub use hunk::{Hunk, parse_hunks};
pub use line::{DiffLine, LineOrigin};
pub use word::{WordSpan, pair_hunk_lines, word_diff};
