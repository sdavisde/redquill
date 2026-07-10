//! Navigation primitives over a parsed diff model.
//!
//! Pure lookups for the hunk/file moves `ui/` will bind in Tasks 4-5 (spec
//! DUW 3.3, FR-diff-nav-1/2/3). All four functions take `&[DiffFile]` plus a
//! `&DiffPosition` and return `Option<DiffPosition>` — no mutation, no I/O.

use super::model::{DiffFile, DiffPosition};

/// Returns the position of the next hunk after `pos` (crossing file
/// boundaries), or `None` at the end of the model.
///
/// Zero-hunk files (binary / pure rename / mode-only) are skipped: they have
/// no hunk to land on (spec §6).
// FR-diff-nav-1
pub fn next_hunk(files: &[DiffFile], pos: &DiffPosition) -> Option<DiffPosition> {
    let current = files.get(pos.file)?;
    if pos.hunk + 1 < current.hunks.len() {
        return Some(DiffPosition {
            file: pos.file,
            hunk: pos.hunk + 1,
            line: 0,
        });
    }
    // FR-diff-nav-1: scan forward across file boundaries, skipping any file
    // with zero hunks, until we find the next file that has one.
    for (idx, file) in files.iter().enumerate().skip(pos.file + 1) {
        if !file.hunks.is_empty() {
            return Some(DiffPosition {
                file: idx,
                hunk: 0,
                line: 0,
            });
        }
    }
    None
}

/// Returns the position of the previous hunk before `pos` (crossing file
/// boundaries), or `None` at the start of the model.
///
/// Zero-hunk files (binary / pure rename / mode-only) are skipped: they have
/// no hunk to land on (spec §6).
// FR-diff-nav-1
pub fn prev_hunk(files: &[DiffFile], pos: &DiffPosition) -> Option<DiffPosition> {
    files.get(pos.file)?;
    if pos.hunk > 0 {
        return Some(DiffPosition {
            file: pos.file,
            hunk: pos.hunk - 1,
            line: 0,
        });
    }
    // FR-diff-nav-1: scan backward across file boundaries, skipping any file
    // with zero hunks, landing on the last hunk of the first file that has one.
    for idx in (0..pos.file).rev() {
        if let Some(file) = files.get(idx)
            && !file.hunks.is_empty()
        {
            return Some(DiffPosition {
                file: idx,
                hunk: file.hunks.len() - 1,
                line: 0,
            });
        }
    }
    None
}

/// Returns the first position of the next file after `pos`'s file, or
/// `None` at the end of the model.
///
/// Unlike `next_hunk`, this lands on zero-hunk files too — a file's header
/// (`hunk: 0, line: 0`) is a valid position even with no hunk body (spec §6).
// FR-diff-nav-2
pub fn next_file(files: &[DiffFile], pos: &DiffPosition) -> Option<DiffPosition> {
    if pos.file + 1 >= files.len() {
        return None;
    }
    Some(DiffPosition {
        file: pos.file + 1,
        hunk: 0,
        line: 0,
    })
}

/// Returns the first position of the previous file before `pos`'s file, or
/// `None` at the start of the model.
///
/// Unlike `prev_hunk`, this lands on zero-hunk files too — a file's header
/// (`hunk: 0, line: 0`) is a valid position even with no hunk body (spec §6).
// FR-diff-nav-2
pub fn prev_file(files: &[DiffFile], pos: &DiffPosition) -> Option<DiffPosition> {
    if pos.file == 0 || pos.file >= files.len() {
        return None;
    }
    Some(DiffPosition {
        file: pos.file - 1,
        hunk: 0,
        line: 0,
    })
}

// FR-diff-nav-3: all four functions above are pure — they take `&[DiffFile]`
// and `&DiffPosition` by shared reference and return a freshly constructed
// `Option<DiffPosition>`; none mutate the model or its inputs.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::model::{ChangeStatus, Hunk};

    fn hunk() -> Hunk {
        Hunk {
            old_start: 1,
            old_count: 1,
            new_start: 1,
            new_count: 1,
            section: None,
            lines: Vec::new(),
        }
    }

    fn file_with_hunks(path: &str, n: usize) -> DiffFile {
        DiffFile {
            path: path.to_string(),
            old_path: None,
            status: ChangeStatus::Modified,
            mode_change: None,
            is_binary: false,
            hunks: (0..n).map(|_| hunk()).collect(),
        }
    }

    /// A zero-hunk file: mode-only change (binary / pure rename are the other
    /// two spec §6 zero-hunk shapes; navigation treats all three identically).
    fn zero_hunk_file(path: &str) -> DiffFile {
        DiffFile {
            path: path.to_string(),
            old_path: None,
            status: ChangeStatus::Modified,
            mode_change: Some(("100644".to_string(), "100755".to_string())),
            is_binary: false,
            hunks: Vec::new(),
        }
    }

    #[test]
    fn next_hunk_within_file_advances_hunk_index() {
        let files = vec![file_with_hunks("a.rs", 2)];
        let pos = DiffPosition {
            file: 0,
            hunk: 0,
            line: 3,
        };
        assert_eq!(
            next_hunk(&files, &pos),
            Some(DiffPosition {
                file: 0,
                hunk: 1,
                line: 0
            })
        );
    }

    /// Proof artifact: from the last hunk of file 0 in a 2-file model,
    /// `next_hunk` lands on the first hunk of file 1 (FR-diff-nav-1).
    #[test]
    fn next_hunk_crosses_file_boundary() {
        let files = vec![file_with_hunks("a.rs", 2), file_with_hunks("b.rs", 1)];
        let pos = DiffPosition {
            file: 0,
            hunk: 1,
            line: 0,
        };
        assert_eq!(
            next_hunk(&files, &pos),
            Some(DiffPosition {
                file: 1,
                hunk: 0,
                line: 0
            })
        );
    }

    /// Proof artifact: `next_hunk` at the model end returns `None`
    /// (FR-diff-nav-1).
    #[test]
    fn next_hunk_returns_none_at_model_end() {
        let files = vec![file_with_hunks("a.rs", 1)];
        let pos = DiffPosition {
            file: 0,
            hunk: 0,
            line: 0,
        };
        assert_eq!(next_hunk(&files, &pos), None);
    }

    /// Proof artifact: `prev_hunk` at the model start returns `None`
    /// (FR-diff-nav-1).
    #[test]
    fn prev_hunk_returns_none_at_model_start() {
        let files = vec![file_with_hunks("a.rs", 1)];
        let pos = DiffPosition {
            file: 0,
            hunk: 0,
            line: 0,
        };
        assert_eq!(prev_hunk(&files, &pos), None);
    }

    #[test]
    fn prev_hunk_crosses_file_boundary() {
        let files = vec![file_with_hunks("a.rs", 2), file_with_hunks("b.rs", 1)];
        let pos = DiffPosition {
            file: 1,
            hunk: 0,
            line: 0,
        };
        assert_eq!(
            prev_hunk(&files, &pos),
            Some(DiffPosition {
                file: 0,
                hunk: 1,
                line: 0
            })
        );
    }

    /// Proof artifact: `prev_file` from file 0 returns `None`
    /// (FR-diff-nav-1/2).
    #[test]
    fn prev_file_from_first_file_returns_none() {
        let files = vec![file_with_hunks("a.rs", 1), file_with_hunks("b.rs", 1)];
        let pos = DiffPosition {
            file: 0,
            hunk: 0,
            line: 0,
        };
        assert_eq!(prev_file(&files, &pos), None);
    }

    #[test]
    fn next_file_from_last_file_returns_none() {
        let files = vec![file_with_hunks("a.rs", 1), file_with_hunks("b.rs", 1)];
        let pos = DiffPosition {
            file: 1,
            hunk: 0,
            line: 0,
        };
        assert_eq!(next_file(&files, &pos), None);
    }

    #[test]
    fn next_file_lands_on_adjacent_file_first_position() {
        let files = vec![file_with_hunks("a.rs", 2), file_with_hunks("b.rs", 3)];
        let pos = DiffPosition {
            file: 0,
            hunk: 1,
            line: 5,
        };
        assert_eq!(
            next_file(&files, &pos),
            Some(DiffPosition {
                file: 1,
                hunk: 0,
                line: 0
            })
        );
    }

    /// Proof artifact: zero-hunk files (binary / pure rename / mode-only) are
    /// skipped by `next_hunk` but landed on by `next_file` (spec §6).
    #[test]
    fn zero_hunk_file_skipped_by_next_hunk_but_landed_by_next_file() {
        let files = vec![
            file_with_hunks("a.rs", 1),
            zero_hunk_file("b.rs"),
            file_with_hunks("c.rs", 1),
        ];
        let pos = DiffPosition {
            file: 0,
            hunk: 0,
            line: 0,
        };
        assert_eq!(
            next_hunk(&files, &pos),
            Some(DiffPosition {
                file: 2,
                hunk: 0,
                line: 0
            })
        );
        assert_eq!(
            next_file(&files, &pos),
            Some(DiffPosition {
                file: 1,
                hunk: 0,
                line: 0
            })
        );
    }

    /// Proof artifact: zero-hunk files are skipped by `prev_hunk` but landed
    /// on by `prev_file` (spec §6).
    #[test]
    fn zero_hunk_file_skipped_by_prev_hunk_but_landed_by_prev_file() {
        let files = vec![
            file_with_hunks("a.rs", 1),
            zero_hunk_file("b.rs"),
            file_with_hunks("c.rs", 1),
        ];
        let pos = DiffPosition {
            file: 2,
            hunk: 0,
            line: 0,
        };
        assert_eq!(
            prev_hunk(&files, &pos),
            Some(DiffPosition {
                file: 0,
                hunk: 0,
                line: 0
            })
        );
        assert_eq!(
            prev_file(&files, &pos),
            Some(DiffPosition {
                file: 1,
                hunk: 0,
                line: 0
            })
        );
    }

    #[test]
    fn next_hunk_skips_multiple_consecutive_zero_hunk_files() {
        let files = vec![
            file_with_hunks("a.rs", 1),
            zero_hunk_file("b.rs"),
            zero_hunk_file("c.rs"),
            file_with_hunks("d.rs", 1),
        ];
        let pos = DiffPosition {
            file: 0,
            hunk: 0,
            line: 0,
        };
        assert_eq!(
            next_hunk(&files, &pos),
            Some(DiffPosition {
                file: 3,
                hunk: 0,
                line: 0
            })
        );
    }

    #[test]
    fn next_hunk_returns_none_if_all_remaining_files_are_zero_hunk() {
        let files = vec![file_with_hunks("a.rs", 1), zero_hunk_file("b.rs")];
        let pos = DiffPosition {
            file: 0,
            hunk: 0,
            line: 0,
        };
        assert_eq!(next_hunk(&files, &pos), None);
    }

    /// Spec §6 edge case: empty diff (no files) — all navigation is `None`.
    #[test]
    fn empty_model_returns_none_for_all_navigation() {
        let files: Vec<DiffFile> = Vec::new();
        let pos = DiffPosition {
            file: 0,
            hunk: 0,
            line: 0,
        };
        assert_eq!(next_hunk(&files, &pos), None);
        assert_eq!(prev_hunk(&files, &pos), None);
        assert_eq!(next_file(&files, &pos), None);
        assert_eq!(prev_file(&files, &pos), None);
    }
}
