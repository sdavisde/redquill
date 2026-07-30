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
//! - [`FileDiff::stats`]/[`Hunk::stats`] count added/removed lines, and
//!   [`stat_display`] decides how a file's counts should render (real
//!   counts, binary, or omitted).
//! - [`summarize`] rolls a whole review's files up into a [`ReviewSummary`]:
//!   file/binary counts, total churn, and the largest file and hunk.

mod error;
mod file;
mod hunk;
mod line;
mod stat;
mod summary;
mod word;

pub use error::DiffParseError;
pub use file::{FileChangeKind, FileDiff};
pub use hunk::{Hunk, parse_hunks};
pub use line::{DiffLine, LineOrigin};
pub use stat::{DiffStat, StatDisplay, stat_display};
pub use summary::{Hotspot, ReviewSummary, summarize};
pub use word::{WordSpan, pair_hunk_lines, word_diff};
