//! Per-file diff data: change-kind derivation and hunk aggregation, combining
//! the git module's raw patch metadata with parsed hunks.

use crate::git::RawFilePatch;

use super::error::DiffParseError;
use super::hunk::{Hunk, parse_hunks};
use super::line::{DiffLine, LineOrigin};

/// The kind of change a file underwent, mirroring git's own
/// `--name-status` classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileChangeKind {
    /// A new file.
    Added,
    /// A removed file.
    Deleted,
    /// An existing file's content changed, same path.
    Modified,
    /// The file was renamed (with or without content changes).
    Renamed,
    /// The file was copied from another path.
    Copied,
}

impl FileChangeKind {
    /// A single-letter label matching git's `--name-status` convention.
    pub fn letter(self) -> char {
        match self {
            FileChangeKind::Added => 'A',
            FileChangeKind::Deleted => 'D',
            FileChangeKind::Modified => 'M',
            FileChangeKind::Renamed => 'R',
            FileChangeKind::Copied => 'C',
        }
    }

    /// Derives the change kind from a raw patch's header text and metadata.
    fn from_raw(patch: &RawFilePatch) -> FileChangeKind {
        if patch.raw.contains("\nnew file mode ") {
            FileChangeKind::Added
        } else if patch.raw.contains("\ndeleted file mode ") {
            FileChangeKind::Deleted
        } else if patch.raw.contains("\ncopy from ") {
            FileChangeKind::Copied
        } else if patch.raw.contains("\nrename from ") || patch.old_path.is_some() {
            FileChangeKind::Renamed
        } else {
            FileChangeKind::Modified
        }
    }
}

/// A single file's diff: metadata plus its parsed hunks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    /// The current (b-side) path of the file.
    pub path: String,
    /// The original (a-side) path, set only for renames and copies.
    pub old_path: Option<String>,
    /// The kind of change this file underwent.
    pub kind: FileChangeKind,
    /// Whether this is a binary file (no hunks are parsed for these).
    pub is_binary: bool,
    /// The file's parsed hunks, in order.
    pub hunks: Vec<Hunk>,
}

impl FileDiff {
    /// Combines a [`RawFilePatch`] with its parsed hunks into a [`FileDiff`].
    pub fn from_patch(patch: &RawFilePatch) -> Result<FileDiff, DiffParseError> {
        let hunks = parse_hunks(&patch.raw)?;
        Ok(FileDiff {
            path: patch.path.clone(),
            old_path: patch.old_path.clone(),
            kind: FileChangeKind::from_raw(patch),
            is_binary: patch.is_binary,
            hunks,
        })
    }

    /// Counts changed lines across all hunks, as `(added, removed)`. Context
    /// lines aren't counted on either side.
    pub fn line_counts(&self) -> (usize, usize) {
        self.hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .fold((0, 0), |(added, removed), line| match line.origin {
                LineOrigin::Added => (added + 1, removed),
                LineOrigin::Removed => (added, removed + 1),
                LineOrigin::Context => (added, removed),
            })
    }

    /// Builds a synthetic [`FileDiff`] for an untracked file: `git diff` never
    /// surfaces untracked content, but a reviewer needs to see it as a
    /// single all-added hunk (old side `0,0`; new side `1,n`).
    ///
    /// `content` is the file's full text. A missing trailing newline on the
    /// last line is reflected via [`DiffLine::no_newline`], matching how
    /// [`parse_hunks`] handles the same case for real patches. An empty
    /// `content` yields a [`FileDiff`] with no hunks.
    pub fn synthetic_added(path: String, content: &str) -> FileDiff {
        let has_trailing_newline = content.is_empty() || content.ends_with('\n');
        let body = content.strip_suffix('\n').unwrap_or(content);
        let raw_lines: Vec<&str> = if content.is_empty() {
            Vec::new()
        } else {
            body.split('\n').collect()
        };

        let last_index = raw_lines.len().saturating_sub(1);
        let lines: Vec<DiffLine> = raw_lines
            .into_iter()
            .enumerate()
            .map(|(i, text)| DiffLine {
                origin: LineOrigin::Added,
                old_line: None,
                new_line: Some(i as u32 + 1),
                content: text.to_string(),
                no_newline: !has_trailing_newline && i == last_index,
            })
            .collect();

        let new_count = lines.len() as u32;
        let hunks = if lines.is_empty() {
            Vec::new()
        } else {
            vec![Hunk {
                old_start: 0,
                old_count: 0,
                new_start: 1,
                new_count,
                section: None,
                lines,
            }]
        };

        FileDiff {
            path,
            old_path: None,
            kind: FileChangeKind::Added,
            is_binary: false,
            hunks,
        }
    }

    /// Builds a synthetic [`FileDiff`] for the read-only whole-file view:
    /// every line is [`LineOrigin::Context`], with both the old and new line
    /// numbers set to the same 1-based line. Unlike
    /// [`FileDiff::synthetic_added`], this isn't a diff at all — it's the
    /// file's current content framed as one all-context hunk, so it renders
    /// through the same multibuffer/highlighting pipeline as a real diff.
    ///
    /// `content` is the file's full text; a missing trailing newline on the
    /// last line is reflected via [`DiffLine::no_newline`], matching
    /// [`FileDiff::synthetic_added`]. `kind` is [`FileChangeKind::Modified`]
    /// as a neutral placeholder — no `FileChangeKind` variant means "not a
    /// diff", and the read-only file view doesn't display this letter
    /// meaningfully today.
    pub fn synthetic_context(path: String, content: &str) -> FileDiff {
        let has_trailing_newline = content.is_empty() || content.ends_with('\n');
        let body = content.strip_suffix('\n').unwrap_or(content);
        let raw_lines: Vec<&str> = if content.is_empty() {
            Vec::new()
        } else {
            body.split('\n').collect()
        };

        let last_index = raw_lines.len().saturating_sub(1);
        let lines: Vec<DiffLine> = raw_lines
            .into_iter()
            .enumerate()
            .map(|(i, text)| DiffLine {
                origin: LineOrigin::Context,
                old_line: Some(i as u32 + 1),
                new_line: Some(i as u32 + 1),
                content: text.to_string(),
                no_newline: !has_trailing_newline && i == last_index,
            })
            .collect();

        let count = lines.len() as u32;
        let hunks = if lines.is_empty() {
            Vec::new()
        } else {
            vec![Hunk {
                old_start: 1,
                old_count: count,
                new_start: 1,
                new_count: count,
                section: None,
                lines,
            }]
        };

        FileDiff {
            path,
            old_path: None,
            kind: FileChangeKind::Modified,
            is_binary: false,
            hunks,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(raw: &str, path: &str, old_path: Option<&str>, is_binary: bool) -> RawFilePatch {
        RawFilePatch {
            path: path.to_string(),
            old_path: old_path.map(str::to_string),
            raw: raw.to_string(),
            is_binary,
        }
    }

    #[test]
    fn added_file_kind_and_letter() {
        let raw = "\
diff --git a/new.rs b/new.rs
new file mode 100644
index 000..111
--- /dev/null
+++ b/new.rs
@@ -0,0 +1,1 @@
+hi
";
        let p = patch(raw, "new.rs", None, false);
        let diff = FileDiff::from_patch(&p).unwrap();
        assert_eq!(diff.kind, FileChangeKind::Added);
        assert_eq!(diff.kind.letter(), 'A');
        assert_eq!(diff.hunks.len(), 1);
    }

    #[test]
    fn deleted_file_kind_and_letter() {
        let raw = "\
diff --git a/gone.rs b/gone.rs
deleted file mode 100644
index 111..000
--- a/gone.rs
+++ /dev/null
@@ -1 +0,0 @@
-bye
";
        let p = patch(raw, "gone.rs", None, false);
        let diff = FileDiff::from_patch(&p).unwrap();
        assert_eq!(diff.kind, FileChangeKind::Deleted);
        assert_eq!(diff.kind.letter(), 'D');
    }

    #[test]
    fn modified_file_kind_and_letter() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 111..222 100644
--- a/f.rs
+++ b/f.rs
@@ -1 +1 @@
-a
+b
";
        let p = patch(raw, "f.rs", None, false);
        let diff = FileDiff::from_patch(&p).unwrap();
        assert_eq!(diff.kind, FileChangeKind::Modified);
        assert_eq!(diff.kind.letter(), 'M');
    }

    #[test]
    fn renamed_via_rename_header() {
        let raw = "\
diff --git a/old.rs b/new.rs
similarity index 90%
rename from old.rs
rename to new.rs
index 1..2 100644
--- a/old.rs
+++ b/new.rs
@@ -1 +1 @@
-x
+y
";
        let p = patch(raw, "new.rs", Some("old.rs"), false);
        let diff = FileDiff::from_patch(&p).unwrap();
        assert_eq!(diff.kind, FileChangeKind::Renamed);
        assert_eq!(diff.kind.letter(), 'R');
    }

    #[test]
    fn renamed_via_old_path_without_rename_header() {
        // A differing a/b side with no explicit "rename from" header still
        // implies a move, per the git module's own old_path derivation.
        let raw = "\
diff --git a/old.rs b/new.rs
index 1..2 100644
--- a/old.rs
+++ b/new.rs
@@ -1 +1 @@
-x
+y
";
        let p = patch(raw, "new.rs", Some("old.rs"), false);
        let diff = FileDiff::from_patch(&p).unwrap();
        assert_eq!(diff.kind, FileChangeKind::Renamed);
    }

    #[test]
    fn copied_file_kind_and_letter() {
        let raw = "\
diff --git a/orig.rs b/copy.rs
similarity index 100%
copy from orig.rs
copy to copy.rs
index 1..1 100644
";
        let p = patch(raw, "copy.rs", Some("orig.rs"), false);
        let diff = FileDiff::from_patch(&p).unwrap();
        assert_eq!(diff.kind, FileChangeKind::Copied);
        assert_eq!(diff.kind.letter(), 'C');
    }

    #[test]
    fn binary_file_has_no_hunks() {
        let raw = "\
diff --git a/img.png b/img.png
index 1..2 100644
Binary files a/img.png and b/img.png differ
";
        let p = patch(raw, "img.png", None, true);
        let diff = FileDiff::from_patch(&p).unwrap();
        assert!(diff.is_binary);
        assert!(diff.hunks.is_empty());
    }

    #[test]
    fn synthetic_added_with_trailing_newline() {
        let diff = FileDiff::synthetic_added("new.rs".to_string(), "a\nb\nc\n");
        assert_eq!(diff.path, "new.rs");
        assert_eq!(diff.old_path, None);
        assert_eq!(diff.kind, FileChangeKind::Added);
        assert!(!diff.is_binary);
        assert_eq!(diff.hunks.len(), 1);
        let hunk = &diff.hunks[0];
        assert_eq!(hunk.old_start, 0);
        assert_eq!(hunk.old_count, 0);
        assert_eq!(hunk.new_start, 1);
        assert_eq!(hunk.new_count, 3);
        assert_eq!(hunk.lines.len(), 3);
        for (i, line) in hunk.lines.iter().enumerate() {
            assert_eq!(line.origin, LineOrigin::Added);
            assert_eq!(line.old_line, None);
            assert_eq!(line.new_line, Some(i as u32 + 1));
            assert!(!line.no_newline);
        }
        assert_eq!(hunk.lines[0].content, "a");
        assert_eq!(hunk.lines[2].content, "c");
    }

    #[test]
    fn synthetic_added_without_trailing_newline_marks_last_line() {
        let diff = FileDiff::synthetic_added("new.rs".to_string(), "a\nb");
        let hunk = &diff.hunks[0];
        assert_eq!(hunk.lines.len(), 2);
        assert!(!hunk.lines[0].no_newline);
        assert!(hunk.lines[1].no_newline);
        assert_eq!(hunk.lines[1].content, "b");
    }

    #[test]
    fn synthetic_added_single_line_no_trailing_newline() {
        let diff = FileDiff::synthetic_added("new.rs".to_string(), "only");
        let hunk = &diff.hunks[0];
        assert_eq!(hunk.lines.len(), 1);
        assert!(hunk.lines[0].no_newline);
        assert_eq!(hunk.lines[0].new_line, Some(1));
    }

    #[test]
    fn synthetic_added_empty_content_has_no_hunks() {
        let diff = FileDiff::synthetic_added("empty.rs".to_string(), "");
        assert!(diff.hunks.is_empty());
        assert_eq!(diff.kind, FileChangeKind::Added);
    }

    // -- FileDiff::synthetic_context (read-only file view) --

    #[test]
    fn synthetic_context_marks_every_line_as_context_on_both_sides() {
        let diff = FileDiff::synthetic_context("f.rs".to_string(), "a\nb\nc\n");
        assert_eq!(diff.path, "f.rs");
        assert_eq!(diff.old_path, None);
        assert!(!diff.is_binary);
        assert_eq!(diff.hunks.len(), 1);
        let hunk = &diff.hunks[0];
        assert_eq!(hunk.old_start, 1);
        assert_eq!(hunk.old_count, 3);
        assert_eq!(hunk.new_start, 1);
        assert_eq!(hunk.new_count, 3);
        assert_eq!(hunk.lines.len(), 3);
        for (i, line) in hunk.lines.iter().enumerate() {
            assert_eq!(line.origin, LineOrigin::Context);
            assert_eq!(line.old_line, Some(i as u32 + 1));
            assert_eq!(line.new_line, Some(i as u32 + 1));
        }
        assert_eq!(hunk.lines[0].content, "a");
        assert_eq!(hunk.lines[2].content, "c");
    }

    #[test]
    fn synthetic_context_without_trailing_newline_marks_last_line() {
        let diff = FileDiff::synthetic_context("f.rs".to_string(), "a\nb");
        let hunk = &diff.hunks[0];
        assert_eq!(hunk.lines.len(), 2);
        assert!(!hunk.lines[0].no_newline);
        assert!(hunk.lines[1].no_newline);
    }

    #[test]
    fn synthetic_context_empty_content_has_no_hunks() {
        let diff = FileDiff::synthetic_context("empty.rs".to_string(), "");
        assert!(diff.hunks.is_empty());
    }

    #[test]
    fn malformed_hunk_propagates_error() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,x +1 @@
+x
";
        let p = patch(raw, "f.rs", None, false);
        assert!(FileDiff::from_patch(&p).is_err());
    }

    #[test]
    fn line_counts_tallies_added_and_removed_across_hunks() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 111..222 100644
--- a/f.rs
+++ b/f.rs
@@ -1,3 +1,3 @@
 ctx
-old
+new
@@ -10,2 +10,3 @@
 ctx2
+extra1
+extra2
";
        let p = patch(raw, "f.rs", None, false);
        let diff = FileDiff::from_patch(&p).unwrap();
        assert_eq!(diff.line_counts(), (3, 1));
    }

    #[test]
    fn line_counts_ignores_context_only_hunks() {
        let diff = FileDiff::synthetic_context("f.rs".to_string(), "a\nb\n");
        assert_eq!(diff.line_counts(), (0, 0));
    }

    #[test]
    fn line_counts_on_file_with_no_hunks_is_zero() {
        let raw = "\
diff --git a/f.bin b/f.bin
index 111..222 100644
Binary files a/f.bin and b/f.bin differ
";
        let p = patch(raw, "f.bin", None, true);
        let diff = FileDiff::from_patch(&p).unwrap();
        assert_eq!(diff.line_counts(), (0, 0));
    }
}
