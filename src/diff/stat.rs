//! Added/removed line counts for a hunk or file, and the shared display
//! decision (real counts, a `bin` placeholder, or omitted) every render
//! surface bases its "show counts or not" call on.

use super::file::FileDiff;
use super::hunk::Hunk;
use super::line::LineOrigin;

/// A diff's added/removed line counts. Distinct from a hunk header's
/// old/new counts ([`Hunk::old_count`]/[`Hunk::new_count`]), which include
/// context lines — this counts only lines that actually changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DiffStat {
    /// Number of added lines.
    pub added: usize,
    /// Number of removed lines.
    pub removed: usize,
}

impl std::ops::AddAssign for DiffStat {
    fn add_assign(&mut self, other: DiffStat) {
        self.added += other.added;
        self.removed += other.removed;
    }
}

impl std::iter::Sum for DiffStat {
    fn sum<I: Iterator<Item = DiffStat>>(iter: I) -> DiffStat {
        iter.fold(DiffStat::default(), |mut acc, s| {
            acc += s;
            acc
        })
    }
}

impl Hunk {
    /// This hunk's added/removed line counts; context lines don't count.
    pub fn stats(&self) -> DiffStat {
        self.lines
            .iter()
            .fold(DiffStat::default(), |mut acc, line| {
                match line.origin {
                    LineOrigin::Added => acc.added += 1,
                    LineOrigin::Removed => acc.removed += 1,
                    LineOrigin::Context => {}
                }
                acc
            })
    }
}

impl FileDiff {
    /// This file's added/removed line counts, summed across every hunk.
    /// `0`/`0` for a binary file (no hunks are parsed) or any other file
    /// with no hunks at all.
    pub fn stats(&self) -> DiffStat {
        self.hunks.iter().map(Hunk::stats).sum()
    }
}

/// How a file's change-count summary should render: real counts, a `bin`
/// placeholder (binary content makes a line count meaningless), or omitted
/// entirely.
///
/// `Omitted` covers both "nothing to count" (no hunks at all — a pure
/// rename, or a fully-staged section with no textual patch) and the
/// read-only whole-file view's synthetic all-context body (see
/// [`FileDiff::synthetic_context`]): a real hunk always carries at least one
/// added or removed line, so a hunk-having file whose `stat` still lands on
/// `0`/`0` only happens in that synthetic case — folded into the same
/// "nothing to show" bucket rather than rendering a misleading `+0 -0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatDisplay {
    /// Real added/removed counts to render.
    Counts(DiffStat),
    /// Binary content — render a `bin` placeholder instead of counts.
    Binary,
    /// Nothing to show — render no counts at all.
    Omitted,
}

/// Decides `file`'s [`StatDisplay`] from its own binary/hunk state plus its
/// already-computed `stat`. `stat` is a parameter (rather than computed
/// internally via [`FileDiff::stats`]) so a caller holding a precomputed,
/// snapshot-level stat never re-walks the file's hunks just to decide how to
/// render it.
pub fn stat_display(file: &FileDiff, stat: DiffStat) -> StatDisplay {
    if file.is_binary {
        StatDisplay::Binary
    } else if file.hunks.is_empty() || (stat.added == 0 && stat.removed == 0) {
        StatDisplay::Omitted
    } else {
        StatDisplay::Counts(stat)
    }
}

#[cfg(test)]
mod tests {
    use super::super::file::FileChangeKind;
    use super::super::line::DiffLine;
    use super::*;
    use crate::git::RawFilePatch;

    fn line(origin: LineOrigin) -> DiffLine {
        DiffLine {
            origin,
            old_line: None,
            new_line: None,
            content: String::new(),
            no_newline: false,
        }
    }

    fn hunk(lines: Vec<DiffLine>) -> Hunk {
        Hunk {
            old_start: 1,
            old_count: lines.len() as u32,
            new_start: 1,
            new_count: lines.len() as u32,
            section: None,
            lines,
        }
    }

    // -- Hunk::stats --

    #[test]
    fn hunk_stats_counts_added_and_removed_only() {
        let h = hunk(vec![
            line(LineOrigin::Context),
            line(LineOrigin::Added),
            line(LineOrigin::Added),
            line(LineOrigin::Removed),
        ]);
        assert_eq!(
            h.stats(),
            DiffStat {
                added: 2,
                removed: 1
            }
        );
    }

    #[test]
    fn hunk_stats_all_context_is_zero() {
        let h = hunk(vec![line(LineOrigin::Context), line(LineOrigin::Context)]);
        assert_eq!(h.stats(), DiffStat::default());
    }

    // -- FileDiff::stats --

    #[test]
    fn file_stats_sums_across_hunks() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 111..222 100644
--- a/f.rs
+++ b/f.rs
@@ -1,2 +1,2 @@
 a
-b
+B
@@ -10,2 +10,3 @@
 j
+k
 l
";
        let file = FileDiff::from_patch(&RawFilePatch {
            path: "f.rs".to_string(),
            old_path: None,
            raw: raw.to_string(),
            is_binary: false,
        })
        .unwrap();
        assert_eq!(
            file.stats(),
            DiffStat {
                added: 2,
                removed: 1
            }
        );
    }

    #[test]
    fn file_stats_is_zero_with_no_hunks() {
        let file = FileDiff {
            path: "f.rs".to_string(),
            old_path: Some("old.rs".to_string()),
            kind: FileChangeKind::Renamed,
            is_binary: false,
            hunks: Vec::new(),
        };
        assert_eq!(file.stats(), DiffStat::default());
    }

    #[test]
    fn file_stats_is_zero_for_binary() {
        let file = FileDiff {
            path: "img.png".to_string(),
            old_path: None,
            kind: FileChangeKind::Modified,
            is_binary: true,
            hunks: Vec::new(),
        };
        assert_eq!(file.stats(), DiffStat::default());
    }

    #[test]
    fn synthetic_added_file_counts_every_line_as_added() {
        let file = FileDiff::synthetic_added("new.rs".to_string(), "a\nb\nc\n");
        assert_eq!(
            file.stats(),
            DiffStat {
                added: 3,
                removed: 0
            }
        );
    }

    #[test]
    fn synthetic_context_file_counts_nothing() {
        let file = FileDiff::synthetic_context("f.rs".to_string(), "a\nb\n");
        assert_eq!(file.stats(), DiffStat::default());
    }

    // -- stat_display --

    #[test]
    fn stat_display_binary_wins_regardless_of_hunk_state() {
        let file = FileDiff {
            path: "img.png".to_string(),
            old_path: None,
            kind: FileChangeKind::Modified,
            is_binary: true,
            hunks: Vec::new(),
        };
        assert_eq!(
            stat_display(&file, DiffStat::default()),
            StatDisplay::Binary
        );
    }

    #[test]
    fn stat_display_omitted_for_empty_hunks_rename() {
        let file = FileDiff {
            path: "f.rs".to_string(),
            old_path: Some("old.rs".to_string()),
            kind: FileChangeKind::Renamed,
            is_binary: false,
            hunks: Vec::new(),
        };
        assert_eq!(
            stat_display(&file, DiffStat::default()),
            StatDisplay::Omitted
        );
    }

    #[test]
    fn stat_display_omitted_for_whole_file_read_only_view() {
        let file = FileDiff::synthetic_context("f.rs".to_string(), "a\nb\n");
        let stat = file.stats();
        assert_eq!(
            stat_display(&file, stat),
            StatDisplay::Omitted,
            "an all-context synthetic file must never render +0 -0"
        );
    }

    #[test]
    fn stat_display_shows_counts_for_a_real_change() {
        let file = FileDiff::synthetic_added("new.rs".to_string(), "a\n");
        let stat = file.stats();
        assert_eq!(
            stat_display(&file, stat),
            StatDisplay::Counts(DiffStat {
                added: 1,
                removed: 0
            })
        );
    }
}
