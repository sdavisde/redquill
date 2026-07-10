//! The diff model: files, hunks, lines, and intra-line word diff. Pure data
//! and transforms with no I/O or TUI dependencies; heavily unit-tested.

pub mod model;
pub mod nav;
pub mod word;

pub use model::{
    ChangeStatus, DiffFile, DiffPosition, DiffSummary, Hunk, Line, LineKind, parse_patch,
    parse_patches, summarize,
};
pub use nav::{next_file, next_hunk, prev_file, prev_hunk};
pub use word::{attach_word_spans, word_diff_spans};
