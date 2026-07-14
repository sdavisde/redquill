//! Splits combined `git diff` output into raw per-file patches.
//!
//! This is deliberately shallow: it locates `diff --git` boundaries, extracts
//! the path(s) for each file, and flags binary patches. It does NOT parse
//! hunks — see `crate::diff::parse_hunks` for that. Each file's patch text
//! is preserved verbatim in [`RawFilePatch::raw`].

/// One file's slice of a combined diff, kept as raw patch text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawFilePatch {
    /// The current (b-side) path of the file.
    pub path: String,
    /// The original (a-side) path, set only for renames and copies.
    pub old_path: Option<String>,
    /// The verbatim patch text for this file, including its `diff --git` header.
    pub raw: String,
    /// Whether git reported this file as binary (contents not parseable).
    pub is_binary: bool,
}

/// Which diff git should produce, mirroring the CLI's diff targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffTarget {
    /// Working tree vs. index (`git diff`).
    WorkingTree,
    /// Index vs. `HEAD` (`git diff --staged`).
    Staged,
    /// An explicit range or ref expression (`git diff <range>`).
    Range(String),
    /// A single commit's own changes: what that commit introduced relative
    /// to its first parent (`git diff <rev>^ <rev>`), or relative to git's
    /// empty tree for a root commit. The payload is any revision spec git
    /// accepts (full/short SHA, `HEAD`, etc.).
    Commit(String),
}

/// Which staging action (if any) a diff target supports — the direction
/// [`crate::ui`]'s staging gestures (`space`/`S`/the staging panel) apply, or
/// [`StagingMode::ReadOnly`] when the target has no index comparison to
/// stage/unstage against at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StagingMode {
    /// The target compares the working tree to the index: staging gestures
    /// move changes *into* the index (`git add` semantics).
    Stage,
    /// The target compares the index to `HEAD`: staging gestures move
    /// changes *out of* the index (`git restore --staged` semantics).
    Unstage,
    /// The target is a fixed historical comparison with no index on either
    /// side, so no staging gesture applies.
    ReadOnly,
}

impl DiffTarget {
    /// Whether this source's content can change out from under the app while
    /// it's running (independent of any git action the user takes here) —
    /// drives whether auto-refresh polls the target at all and whether
    /// untracked files get synthesized into the diff (only meaningful for a
    /// source that reflects the live working tree).
    pub fn is_live(&self) -> bool {
        match self {
            DiffTarget::WorkingTree => true,
            DiffTarget::Staged => false,
            DiffTarget::Range(_) => false,
            DiffTarget::Commit(_) => false,
        }
    }

    /// Which staging direction (if any) this target supports — see
    /// [`StagingMode`].
    pub fn staging_mode(&self) -> StagingMode {
        match self {
            DiffTarget::WorkingTree => StagingMode::Stage,
            DiffTarget::Staged => StagingMode::Unstage,
            DiffTarget::Range(_) => StagingMode::ReadOnly,
            DiffTarget::Commit(_) => StagingMode::ReadOnly,
        }
    }

    /// Whether LSP code-intelligence requests (`gd`/`gr`/`K`) are valid
    /// against this target: true only when the diff's new side is the live
    /// working tree on disk, since that's the only content an LSP server can
    /// be asked about with on-disk-accurate positions.
    pub fn supports_code_intel(&self) -> bool {
        match self {
            DiffTarget::WorkingTree => true,
            DiffTarget::Staged => false,
            DiffTarget::Range(_) => false,
            DiffTarget::Commit(_) => false,
        }
    }
}

/// Splits a combined unified diff into one [`RawFilePatch`] per file.
///
/// Boundaries are `diff --git` lines at the start of a line; the text between
/// one boundary and the next (or end of input) is that file's raw patch.
pub fn split_patches(input: &str) -> Vec<RawFilePatch> {
    // Record byte offsets of every line that begins a new file's patch.
    let mut starts = Vec::new();
    let mut offset = 0usize;
    for line in input.split_inclusive('\n') {
        if line.starts_with("diff --git ") {
            starts.push(offset);
        }
        offset += line.len();
    }

    let mut patches = Vec::with_capacity(starts.len());
    for (i, &start) in starts.iter().enumerate() {
        let end = starts.get(i + 1).copied().unwrap_or(input.len());
        patches.push(parse_patch(&input[start..end]));
    }
    patches
}

/// Strips an `a/` or `b/` prefix from a diff path side, mapping `/dev/null` to
/// `None`. Trailing whitespace (e.g. a tab-separated timestamp) is trimmed.
fn clean_side(side: &str) -> Option<String> {
    let side = side.trim_end();
    if side == "/dev/null" {
        return None;
    }
    let stripped = side
        .strip_prefix("a/")
        .or_else(|| side.strip_prefix("b/"))
        .unwrap_or(side);
    Some(stripped.to_string())
}

/// Extracts the two paths from a `Binary files a/X and b/Y differ` line.
fn parse_binary_line(line: &str) -> (Option<String>, Option<String>) {
    let inner = line
        .strip_prefix("Binary files ")
        .and_then(|s| s.strip_suffix(" differ"));
    match inner.and_then(|s| s.split_once(" and ")) {
        Some((a, b)) => (clean_side(a), clean_side(b)),
        None => (None, None),
    }
}

/// Extracts `(old, new)` paths from the tail of a `diff --git ` line.
///
/// Best-effort: splits `a/<old> b/<new>` at the ` b/` boundary. The unambiguous
/// per-side header lines (`---`, `+++`, `rename from/to`) are preferred over
/// this when present.
fn parse_diff_git(rest: &str) -> Option<(String, String)> {
    let idx = rest.find(" b/")?;
    let old = rest.get(..idx)?.strip_prefix("a/").unwrap_or(&rest[..idx]);
    let new = &rest[idx + 3..];
    Some((old.to_string(), new.to_string()))
}

/// Parses a single file's raw patch slice into a [`RawFilePatch`].
fn parse_patch(raw: &str) -> RawFilePatch {
    let mut a_path: Option<String> = None;
    let mut b_path: Option<String> = None;
    let mut rename_from: Option<String> = None;
    let mut rename_to: Option<String> = None;
    let mut copy_from: Option<String> = None;
    let mut copy_to: Option<String> = None;
    let mut header: Option<(String, String)> = None;
    let mut is_binary = false;

    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            header = parse_diff_git(rest);
        } else if let Some(rest) = line.strip_prefix("--- ") {
            a_path = clean_side(rest);
        } else if let Some(rest) = line.strip_prefix("+++ ") {
            b_path = clean_side(rest);
        } else if let Some(rest) = line.strip_prefix("rename from ") {
            rename_from = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("rename to ") {
            rename_to = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("copy from ") {
            copy_from = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("copy to ") {
            copy_to = Some(rest.to_string());
        } else if line.starts_with("Binary files ") && line.ends_with(" differ") {
            is_binary = true;
            let (a, b) = parse_binary_line(line);
            a_path = a_path.or(a);
            b_path = b_path.or(b);
        } else if line == "GIT binary patch" {
            is_binary = true;
        }
    }

    let path = rename_to
        .clone()
        .or_else(|| copy_to.clone())
        .or_else(|| b_path.clone())
        .or_else(|| a_path.clone())
        .or_else(|| header.as_ref().map(|(_, n)| n.clone()))
        .unwrap_or_default();

    let old_path = rename_from
        .or(copy_from)
        .or_else(|| match (&a_path, &b_path) {
            // A differing a/b side (with no rename header) still implies a move.
            (Some(a), Some(b)) if a != b => Some(a.clone()),
            _ => None,
        });

    RawFilePatch {
        path,
        old_path,
        raw: raw.to_string(),
        is_binary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Capability triple: is_live / staging_mode / supports_code_intel ---
    //
    // One test per variant asserting the full triple in a single place, so
    // the exhaustive-match methods above and this table can't silently drift
    // apart (rust-best-practices: data-driven invariants).

    #[test]
    fn working_tree_capability_triple() {
        let target = DiffTarget::WorkingTree;
        assert!(target.is_live());
        assert_eq!(target.staging_mode(), StagingMode::Stage);
        assert!(target.supports_code_intel());
    }

    #[test]
    fn staged_capability_triple() {
        let target = DiffTarget::Staged;
        assert!(!target.is_live());
        assert_eq!(target.staging_mode(), StagingMode::Unstage);
        assert!(!target.supports_code_intel());
    }

    #[test]
    fn range_capability_triple() {
        let target = DiffTarget::Range("main..HEAD".to_string());
        assert!(!target.is_live());
        assert_eq!(target.staging_mode(), StagingMode::ReadOnly);
        assert!(!target.supports_code_intel());
    }

    #[test]
    fn commit_capability_triple() {
        let target = DiffTarget::Commit("HEAD".to_string());
        assert!(!target.is_live());
        assert_eq!(target.staging_mode(), StagingMode::ReadOnly);
        assert!(!target.supports_code_intel());
    }

    #[test]
    fn splits_single_modified_file() {
        let diff = "\
diff --git a/src/main.rs b/src/main.rs
index 111..222 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,1 +1,1 @@
-old
+new
";
        let patches = split_patches(diff);
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].path, "src/main.rs");
        assert_eq!(patches[0].old_path, None);
        assert!(!patches[0].is_binary);
        assert!(patches[0].raw.starts_with("diff --git"));
        assert!(patches[0].raw.contains("+new"));
    }

    #[test]
    fn splits_multiple_files() {
        let diff = "\
diff --git a/a.rs b/a.rs
index 1..2 100644
--- a/a.rs
+++ b/a.rs
@@ -1 +1 @@
-a
+A
diff --git a/b.rs b/b.rs
index 3..4 100644
--- a/b.rs
+++ b/b.rs
@@ -1 +1 @@
-b
+B
";
        let patches = split_patches(diff);
        assert_eq!(patches.len(), 2);
        assert_eq!(patches[0].path, "a.rs");
        assert_eq!(patches[1].path, "b.rs");
        // Each patch keeps only its own text.
        assert!(patches[0].raw.contains("+A"));
        assert!(!patches[0].raw.contains("+B"));
        assert!(patches[1].raw.contains("+B"));
    }

    #[test]
    fn detects_added_and_deleted_paths() {
        let diff = "\
diff --git a/new.rs b/new.rs
new file mode 100644
index 0..1
--- /dev/null
+++ b/new.rs
@@ -0,0 +1 @@
+hello
diff --git a/gone.rs b/gone.rs
deleted file mode 100644
index 1..0
--- a/gone.rs
+++ /dev/null
@@ -1 +0,0 @@
-bye
";
        let patches = split_patches(diff);
        assert_eq!(patches.len(), 2);
        assert_eq!(patches[0].path, "new.rs");
        assert_eq!(patches[0].old_path, None);
        assert_eq!(patches[1].path, "gone.rs");
        assert_eq!(patches[1].old_path, None);
    }

    #[test]
    fn detects_rename() {
        let diff = "\
diff --git a/old/name.rs b/new/name.rs
similarity index 90%
rename from old/name.rs
rename to new/name.rs
index 1..2 100644
--- a/old/name.rs
+++ b/new/name.rs
@@ -1 +1 @@
-x
+y
";
        let patches = split_patches(diff);
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].path, "new/name.rs");
        assert_eq!(patches[0].old_path.as_deref(), Some("old/name.rs"));
        assert!(!patches[0].is_binary);
    }

    #[test]
    fn detects_binary_file() {
        let diff = "\
diff --git a/img.png b/img.png
new file mode 100644
index 0..1
Binary files /dev/null and b/img.png differ
";
        let patches = split_patches(diff);
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].path, "img.png");
        assert!(patches[0].is_binary);
    }

    #[test]
    fn detects_modified_binary_file() {
        let diff = "\
diff --git a/img.png b/img.png
index 1..2 100644
Binary files a/img.png and b/img.png differ
";
        let patches = split_patches(diff);
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].path, "img.png");
        assert!(patches[0].is_binary);
    }

    #[test]
    fn mixed_text_and_binary() {
        let diff = "\
diff --git a/text.rs b/text.rs
index 1..2 100644
--- a/text.rs
+++ b/text.rs
@@ -1 +1 @@
-a
+b
diff --git a/logo.png b/logo.png
index 3..4 100644
Binary files a/logo.png and b/logo.png differ
";
        let patches = split_patches(diff);
        assert_eq!(patches.len(), 2);
        assert!(!patches[0].is_binary);
        assert!(patches[1].is_binary);
        assert_eq!(patches[1].path, "logo.png");
    }

    #[test]
    fn empty_diff_yields_no_patches() {
        assert!(split_patches("").is_empty());
    }
}
