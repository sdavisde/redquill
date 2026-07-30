//! Review-wide aggregation over a set of [`FileDiff`]s: how many files
//! changed, how they break down by change kind, the total added/removed
//! counts, and which single file and hunk carry the most churn.
//!
//! Pure data over the diff model — the counts a reviewer wants before
//! deciding where to start reading. Nothing here does I/O or knows about
//! rendering.

use super::file::FileDiff;
use super::hunk::Hunk;
use super::stat::DiffStat;

/// Where the largest single unit of churn sits, so a reviewer can jump
/// straight at the part of the review that most likely needs attention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hotspot {
    /// The file's current (b-side) path.
    pub path: String,
    /// The file's total added/removed counts.
    pub stat: DiffStat,
}

/// Review-wide counts over every file in a review.
///
/// Binary files contribute to `files` and `binary_files` but never to
/// `stat`: a line count over binary content is meaningless, and
/// [`super::stat_display`] already renders them as a placeholder rather
/// than as `+0 -0`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReviewSummary {
    /// Total number of files in the review, binary and rename-only included.
    pub files: usize,
    /// How many files are binary.
    pub binary_files: usize,
    /// How many files changed content (added, deleted, or modified) — a
    /// rename or copy with no hunks is excluded.
    pub content_files: usize,
    /// Added/removed lines summed across every file.
    pub stat: DiffStat,
    /// The file carrying the most changed lines, or `None` for an empty
    /// review or one with no counted lines at all. Ties go to the file that
    /// comes first in the input order, which callers keep path-sorted.
    pub largest_file: Option<Hotspot>,
    /// The largest single hunk's changed-line count, across all files.
    pub largest_hunk: usize,
}

impl ReviewSummary {
    /// Whether there is nothing at all to review.
    pub fn is_empty(&self) -> bool {
        self.files == 0
    }
}

/// Aggregates `files` into a [`ReviewSummary`].
///
/// One pass over the files and their hunks; the caller's own per-file stats
/// are recomputed here rather than threaded in, so this stays usable from
/// anywhere holding a slice of [`FileDiff`].
pub fn summarize(files: &[FileDiff]) -> ReviewSummary {
    let mut summary = ReviewSummary {
        files: files.len(),
        ..ReviewSummary::default()
    };

    for file in files {
        if file.is_binary {
            summary.binary_files += 1;
            continue;
        }

        let stat = file.stats();
        if stat.is_empty() {
            continue;
        }

        if file.kind.is_content_change() {
            summary.content_files += 1;
        }
        summary.stat += stat;

        let largest_here = file
            .hunks
            .iter()
            .map(Hunk::changed_lines)
            .max()
            .unwrap_or(0);
        summary.largest_hunk = summary.largest_hunk.max(largest_here);

        let bigger = match &summary.largest_file {
            Some(current) => stat.total() > current.stat.total(),
            None => true,
        };
        if bigger {
            summary.largest_file = Some(Hotspot {
                path: file.path.clone(),
                stat,
            });
        }
    }

    summary
}

#[cfg(test)]
mod tests {
    use super::super::file::FileChangeKind;
    use super::super::line::{DiffLine, LineOrigin};
    use super::*;

    fn line(origin: LineOrigin) -> DiffLine {
        DiffLine {
            origin,
            old_line: None,
            new_line: None,
            content: String::new(),
            no_newline: false,
        }
    }

    /// `added`/`removed`/`context` counts of same-shaped lines in one hunk.
    fn hunk(added: usize, removed: usize, context: usize) -> Hunk {
        let mut lines = Vec::new();
        lines.extend((0..removed).map(|_| line(LineOrigin::Removed)));
        lines.extend((0..added).map(|_| line(LineOrigin::Added)));
        lines.extend((0..context).map(|_| line(LineOrigin::Context)));
        Hunk {
            old_start: 1,
            old_count: (removed + context) as u32,
            new_start: 1,
            new_count: (added + context) as u32,
            section: None,
            lines,
        }
    }

    fn file(path: &str, kind: FileChangeKind, hunks: Vec<Hunk>) -> FileDiff {
        FileDiff {
            path: path.to_string(),
            old_path: None,
            kind,
            is_binary: false,
            hunks,
        }
    }

    fn binary(path: &str) -> FileDiff {
        FileDiff {
            path: path.to_string(),
            old_path: None,
            kind: FileChangeKind::Modified,
            is_binary: true,
            hunks: Vec::new(),
        }
    }

    #[test]
    fn empty_review_summarizes_to_nothing() {
        let summary = summarize(&[]);
        assert!(summary.is_empty());
        assert_eq!(summary.largest_file, None);
        assert_eq!(summary.largest_hunk, 0);
    }

    #[test]
    fn totals_sum_added_and_removed_across_files() {
        let files = vec![
            file(
                "a.rs",
                FileChangeKind::Modified,
                vec![hunk(3, 1, 2), hunk(1, 0, 4)],
            ),
            file("b.rs", FileChangeKind::Added, vec![hunk(5, 0, 0)]),
        ];
        let summary = summarize(&files);
        assert_eq!(
            summary.stat,
            DiffStat {
                added: 9,
                removed: 1
            }
        );
        assert_eq!(summary.files, 2);
        assert_eq!(summary.content_files, 2);
    }

    #[test]
    fn binary_files_are_counted_but_never_add_lines() {
        let files = vec![
            file("a.rs", FileChangeKind::Modified, vec![hunk(2, 2, 1)]),
            binary("logo.png"),
        ];
        let summary = summarize(&files);
        assert_eq!(summary.files, 2);
        assert_eq!(summary.binary_files, 1);
        assert_eq!(summary.content_files, 1);
        assert_eq!(
            summary.stat,
            DiffStat {
                added: 2,
                removed: 2
            }
        );
    }

    #[test]
    fn rename_with_no_hunks_counts_as_a_file_but_not_a_content_change() {
        let files = vec![file("new.rs", FileChangeKind::Renamed, Vec::new())];
        let summary = summarize(&files);
        assert_eq!(summary.files, 1);
        assert_eq!(summary.content_files, 0);
        assert_eq!(summary.stat, DiffStat::default());
        assert_eq!(summary.largest_file, None);
    }

    #[test]
    fn largest_file_is_the_one_with_the_most_changed_lines() {
        let files = vec![
            file("small.rs", FileChangeKind::Modified, vec![hunk(1, 1, 0)]),
            file("big.rs", FileChangeKind::Modified, vec![hunk(4, 3, 0)]),
            file("mid.rs", FileChangeKind::Modified, vec![hunk(2, 2, 0)]),
        ];
        let summary = summarize(&files);
        let hotspot = summary.largest_file.expect("a hotspot");
        assert_eq!(hotspot.path, "big.rs");
        assert_eq!(
            hotspot.stat,
            DiffStat {
                added: 4,
                removed: 3
            }
        );
    }

    #[test]
    fn largest_file_tie_keeps_the_earlier_file() {
        let files = vec![
            file("a.rs", FileChangeKind::Modified, vec![hunk(2, 2, 0)]),
            file("b.rs", FileChangeKind::Modified, vec![hunk(2, 2, 0)]),
        ];
        let summary = summarize(&files);
        assert_eq!(summary.largest_file.expect("a hotspot").path, "a.rs");
    }

    #[test]
    fn largest_hunk_ignores_context_lines() {
        let files = vec![file(
            "a.rs",
            FileChangeKind::Modified,
            vec![hunk(1, 1, 40), hunk(3, 2, 0)],
        )];
        assert_eq!(summarize(&files).largest_hunk, 5);
    }
}
