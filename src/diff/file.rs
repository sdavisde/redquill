//! Per-file diff data: change-kind derivation and hunk aggregation, combining
//! the git module's raw patch metadata with parsed hunks.

use crate::git::RawFilePatch;

use super::error::DiffParseError;
use super::hunk::{Hunk, parse_hunks};

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
}
