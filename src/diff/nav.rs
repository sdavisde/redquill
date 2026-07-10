//! Navigation primitives over a parsed diff model.
//!
//! Pure lookups for the hunk/file moves `ui/` will bind in Tasks 4-5 (spec
//! DUW 3.3). This module is a compiling stub for T1.0 — the real
//! cross-file-boundary logic is filled in by T3.0 (Wave 2).

use super::model::{DiffFile, DiffPosition};

/// Returns the position of the next hunk after `pos` (crossing file
/// boundaries), or `None` at the end of the model.
///
/// Stub (T1.0): always returns `None`. Filled in by T3.0.
pub fn next_hunk(files: &[DiffFile], pos: &DiffPosition) -> Option<DiffPosition> {
    let _ = files;
    let _ = pos;
    None
}

/// Returns the position of the previous hunk before `pos` (crossing file
/// boundaries), or `None` at the start of the model.
///
/// Stub (T1.0): always returns `None`. Filled in by T3.0.
pub fn prev_hunk(files: &[DiffFile], pos: &DiffPosition) -> Option<DiffPosition> {
    let _ = files;
    let _ = pos;
    None
}

/// Returns the first position of the next file after `pos`'s file, or
/// `None` at the end of the model.
///
/// Stub (T1.0): always returns `None`. Filled in by T3.0.
pub fn next_file(files: &[DiffFile], pos: &DiffPosition) -> Option<DiffPosition> {
    let _ = files;
    let _ = pos;
    None
}

/// Returns the first position of the previous file before `pos`'s file, or
/// `None` at the start of the model.
///
/// Stub (T1.0): always returns `None`. Filled in by T3.0.
pub fn prev_file(files: &[DiffFile], pos: &DiffPosition) -> Option<DiffPosition> {
    let _ = files;
    let _ = pos;
    None
}
