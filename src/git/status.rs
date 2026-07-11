//! Model and parser for `git status --porcelain=v2 -z`.
//!
//! Pure text-in / structs-out: [`parse_porcelain_v2`] takes the raw porcelain
//! payload and returns typed [`FileStatus`] records. No process spawning lives
//! here, which keeps it directly unit-testable with fixture strings.

use super::branch::{BranchStatus, parse_branch_headers};
use super::error::GitError;

/// A single porcelain status code, as it appears in the `XY` field.
///
/// For an ordinary entry, `X` is the state of the change staged in the index
/// and `Y` is the state of the change in the working tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusCode {
    /// No change on this side (`.`).
    Unmodified,
    /// Content modified (`M`).
    Modified,
    /// Type changed, e.g. file to symlink (`T`).
    TypeChange,
    /// Newly added (`A`).
    Added,
    /// Deleted (`D`).
    Deleted,
    /// Renamed (`R`).
    Renamed,
    /// Copied (`C`).
    Copied,
    /// Unmerged / conflicted (`U`).
    Unmerged,
    /// Untracked (`?`).
    Untracked,
    /// Ignored (`!`).
    Ignored,
}

impl StatusCode {
    /// Maps a single porcelain code character to a [`StatusCode`].
    fn from_char(c: char) -> StatusCode {
        match c {
            'M' => StatusCode::Modified,
            'T' => StatusCode::TypeChange,
            'A' => StatusCode::Added,
            'D' => StatusCode::Deleted,
            'R' => StatusCode::Renamed,
            'C' => StatusCode::Copied,
            'U' => StatusCode::Unmerged,
            '?' => StatusCode::Untracked,
            '!' => StatusCode::Ignored,
            // '.' and anything unexpected collapse to "no change on this side".
            _ => StatusCode::Unmodified,
        }
    }

    /// A single-letter label suitable for a summary line.
    pub fn letter(self) -> char {
        match self {
            StatusCode::Unmodified => '.',
            StatusCode::Modified => 'M',
            StatusCode::TypeChange => 'T',
            StatusCode::Added => 'A',
            StatusCode::Deleted => 'D',
            StatusCode::Renamed => 'R',
            StatusCode::Copied => 'C',
            StatusCode::Unmerged => 'U',
            StatusCode::Untracked => '?',
            StatusCode::Ignored => '!',
        }
    }
}

/// The category of a status entry, mirroring the porcelain v2 record types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// An ordinary changed tracked file (`1` record).
    Ordinary,
    /// A renamed or copied tracked file (`2` record).
    RenamedOrCopied,
    /// An unmerged / conflicted file (`u` record).
    Unmerged,
    /// An untracked file (`?` record).
    Untracked,
    /// An ignored file (`!` record).
    Ignored,
}

/// The status of a single path, parsed from one porcelain v2 record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStatus {
    /// Which porcelain record type produced this entry.
    pub kind: ChangeKind,
    /// Index-side status (`X`).
    pub staged: StatusCode,
    /// Working-tree-side status (`Y`).
    pub unstaged: StatusCode,
    /// The current path.
    pub path: String,
    /// For renames and copies, the original path; `None` otherwise.
    pub orig_path: Option<String>,
}

impl FileStatus {
    /// Whether the index differs from `HEAD` for this path.
    pub fn has_staged_changes(&self) -> bool {
        !matches!(
            self.staged,
            StatusCode::Unmodified | StatusCode::Untracked | StatusCode::Ignored
        )
    }

    /// Whether the working tree differs from the index for this path.
    ///
    /// Untracked files count as unstaged working-tree changes.
    pub fn has_unstaged_changes(&self) -> bool {
        matches!(self.kind, ChangeKind::Untracked)
            || !matches!(
                self.unstaged,
                StatusCode::Unmodified | StatusCode::Untracked | StatusCode::Ignored
            )
    }

    /// Whether this path is in an unmerged / conflicted state.
    pub fn is_conflicted(&self) -> bool {
        matches!(self.kind, ChangeKind::Unmerged)
    }

    /// Whether this path is untracked.
    pub fn is_untracked(&self) -> bool {
        matches!(self.kind, ChangeKind::Untracked)
    }
}

/// Reads the two-character `XY` field into staged/unstaged codes.
fn parse_xy(xy: &str) -> Result<(StatusCode, StatusCode), GitError> {
    let mut chars = xy.chars();
    let (Some(x), Some(y)) = (chars.next(), chars.next()) else {
        return Err(GitError::Parse(format!("malformed XY field: {xy:?}")));
    };
    Ok((StatusCode::from_char(x), StatusCode::from_char(y)))
}

/// Parses the payload of `git status --porcelain=v2 -z` into [`FileStatus`]es.
///
/// Records are NUL-separated. Rename/copy (`2`) records are followed by an
/// additional NUL-separated field carrying the original path, which is
/// consumed here transparently.
pub fn parse_porcelain_v2(input: &str) -> Result<Vec<FileStatus>, GitError> {
    let mut out = Vec::new();
    let mut fields = input.split('\0').filter(|s| !s.is_empty());

    while let Some(entry) = fields.next() {
        if let Some(rest) = entry.strip_prefix("1 ") {
            out.push(parse_ordinary(rest)?);
        } else if let Some(rest) = entry.strip_prefix("2 ") {
            let orig = fields
                .next()
                .ok_or_else(|| GitError::Parse("rename record missing original path".into()))?;
            out.push(parse_rename(rest, orig)?);
        } else if let Some(rest) = entry.strip_prefix("u ") {
            out.push(parse_unmerged(rest)?);
        } else if let Some(path) = entry.strip_prefix("? ") {
            out.push(FileStatus {
                kind: ChangeKind::Untracked,
                staged: StatusCode::Unmodified,
                unstaged: StatusCode::Untracked,
                path: path.to_string(),
                orig_path: None,
            });
        } else if let Some(path) = entry.strip_prefix("! ") {
            out.push(FileStatus {
                kind: ChangeKind::Ignored,
                staged: StatusCode::Ignored,
                unstaged: StatusCode::Ignored,
                path: path.to_string(),
                orig_path: None,
            });
        } else if entry.starts_with("# ") {
            // `--branch` header field (`# branch.head`, `# branch.ab`, ...);
            // not a file record, see `branch::parse_branch_headers`.
        } else {
            return Err(GitError::Parse(format!(
                "unrecognized status record: {entry:?}"
            )));
        }
    }

    Ok(out)
}

/// A status snapshot combining per-path statuses with branch sync state,
/// both parsed from a single `git status --porcelain=v2 --branch -z`
/// payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusSnapshot {
    /// Per-path statuses, as returned by [`parse_porcelain_v2`].
    pub files: Vec<FileStatus>,
    /// Branch name, upstream, and ahead/behind state.
    pub branch: BranchStatus,
}

/// Parses the full payload (file records plus `# branch.*` headers) of
/// `git status --porcelain=v2 --branch -z` into a [`StatusSnapshot`].
pub fn parse_porcelain_v2_full(input: &str) -> Result<StatusSnapshot, GitError> {
    Ok(StatusSnapshot {
        files: parse_porcelain_v2(input)?,
        branch: parse_branch_headers(input)?,
    })
}

/// Parses an ordinary `1` record (prefix already stripped).
///
/// Layout: `<XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>`.
fn parse_ordinary(rest: &str) -> Result<FileStatus, GitError> {
    let parts: Vec<&str> = rest.splitn(8, ' ').collect();
    if parts.len() < 8 {
        return Err(GitError::Parse(format!("short ordinary record: {rest:?}")));
    }
    let (staged, unstaged) = parse_xy(parts[0])?;
    Ok(FileStatus {
        kind: ChangeKind::Ordinary,
        staged,
        unstaged,
        path: parts[7].to_string(),
        orig_path: None,
    })
}

/// Parses a rename/copy `2` record (prefix already stripped) plus its original path.
///
/// Layout: `<XY> <sub> <mH> <mI> <mW> <hH> <hI> <Xscore> <path>`.
fn parse_rename(rest: &str, orig: &str) -> Result<FileStatus, GitError> {
    let parts: Vec<&str> = rest.splitn(9, ' ').collect();
    if parts.len() < 9 {
        return Err(GitError::Parse(format!("short rename record: {rest:?}")));
    }
    let (staged, unstaged) = parse_xy(parts[0])?;
    Ok(FileStatus {
        kind: ChangeKind::RenamedOrCopied,
        staged,
        unstaged,
        path: parts[8].to_string(),
        orig_path: Some(orig.to_string()),
    })
}

/// Parses an unmerged `u` record (prefix already stripped).
///
/// Layout: `<XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>`.
fn parse_unmerged(rest: &str) -> Result<FileStatus, GitError> {
    let parts: Vec<&str> = rest.splitn(10, ' ').collect();
    if parts.len() < 10 {
        return Err(GitError::Parse(format!("short unmerged record: {rest:?}")));
    }
    let (staged, unstaged) = parse_xy(parts[0])?;
    Ok(FileStatus {
        kind: ChangeKind::Unmerged,
        staged,
        unstaged,
        path: parts[9].to_string(),
        orig_path: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modified_ordinary_entry() {
        // Modified in the working tree only (` M`).
        let input = "1 .M N... 100644 100644 100644 aaa bbb src/main.rs\0";
        let entries = parse_porcelain_v2(input).unwrap();
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.kind, ChangeKind::Ordinary);
        assert_eq!(e.staged, StatusCode::Unmodified);
        assert_eq!(e.unstaged, StatusCode::Modified);
        assert_eq!(e.path, "src/main.rs");
        assert!(e.has_unstaged_changes());
        assert!(!e.has_staged_changes());
    }

    #[test]
    fn parses_staged_added_entry() {
        let input = "1 A. N... 000000 100644 100644 000000 ccc new.rs\0";
        let e = &parse_porcelain_v2(input).unwrap()[0];
        assert_eq!(e.staged, StatusCode::Added);
        assert_eq!(e.unstaged, StatusCode::Unmodified);
        assert!(e.has_staged_changes());
        assert!(!e.has_unstaged_changes());
    }

    #[test]
    fn parses_deleted_entry() {
        let input = "1 D. N... 100644 000000 000000 ddd 000000 gone.rs\0";
        let e = &parse_porcelain_v2(input).unwrap()[0];
        assert_eq!(e.staged, StatusCode::Deleted);
        assert_eq!(e.path, "gone.rs");
    }

    #[test]
    fn parses_rename_with_orig_path() {
        // `2` record; orig path follows as its own NUL field.
        let input = "2 R. N... 100644 100644 100644 eee fff R100 new/name.rs\0old/name.rs\0";
        let entries = parse_porcelain_v2(input).unwrap();
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.kind, ChangeKind::RenamedOrCopied);
        assert_eq!(e.staged, StatusCode::Renamed);
        assert_eq!(e.path, "new/name.rs");
        assert_eq!(e.orig_path.as_deref(), Some("old/name.rs"));
    }

    #[test]
    fn parses_untracked_entry() {
        let input = "? untracked file.txt\0";
        let e = &parse_porcelain_v2(input).unwrap()[0];
        assert_eq!(e.kind, ChangeKind::Untracked);
        assert_eq!(e.path, "untracked file.txt");
        assert!(e.is_untracked());
        assert!(e.has_unstaged_changes());
        assert!(!e.has_staged_changes());
    }

    #[test]
    fn parses_conflicted_entry() {
        let input = "u UU N... 100644 100644 100644 100644 h1 h2 h3 conflict.rs\0";
        let e = &parse_porcelain_v2(input).unwrap()[0];
        assert_eq!(e.kind, ChangeKind::Unmerged);
        assert!(e.is_conflicted());
        assert_eq!(e.path, "conflict.rs");
    }

    #[test]
    fn parses_path_containing_spaces() {
        let input = "1 .M N... 100644 100644 100644 aaa bbb dir with spaces/a b.rs\0";
        let e = &parse_porcelain_v2(input).unwrap()[0];
        assert_eq!(e.path, "dir with spaces/a b.rs");
    }

    #[test]
    fn parses_multiple_mixed_records() {
        let input = concat!(
            "1 M. N... 100644 100644 100644 a b staged.rs\0",
            "2 R. N... 100644 100644 100644 c d R100 to.rs\0from.rs\0",
            "? new.rs\0",
        );
        let entries = parse_porcelain_v2(input).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].path, "staged.rs");
        assert_eq!(entries[1].orig_path.as_deref(), Some("from.rs"));
        assert_eq!(entries[2].kind, ChangeKind::Untracked);
    }

    #[test]
    fn empty_input_yields_no_entries() {
        assert!(parse_porcelain_v2("").unwrap().is_empty());
    }

    #[test]
    fn rename_missing_orig_path_errors() {
        let input = "2 R. N... 100644 100644 100644 c d R100 to.rs\0";
        assert!(matches!(parse_porcelain_v2(input), Err(GitError::Parse(_))));
    }

    #[test]
    fn branch_headers_are_skipped_not_errored() {
        // `git status --porcelain=v2 --branch -z` prepends `# branch.*`
        // header fields before the ordinary record fields.
        let input = concat!(
            "# branch.oid 85d7cc5dd1cf49f6abe1c81439fbb5deae4124ab\0",
            "# branch.head main\0",
            "# branch.upstream origin/main\0",
            "# branch.ab +2 -1\0",
            "1 .M N... 100644 100644 100644 aaa bbb src/main.rs\0",
        );
        let entries = parse_porcelain_v2(input).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "src/main.rs");
    }

    #[test]
    fn parse_porcelain_v2_full_combines_files_and_branch() {
        let input = concat!(
            "# branch.oid 85d7cc5dd1cf49f6abe1c81439fbb5deae4124ab\0",
            "# branch.head main\0",
            "# branch.upstream origin/main\0",
            "# branch.ab +2 -1\0",
            "1 .M N... 100644 100644 100644 aaa bbb src/main.rs\0",
            "? untracked.txt\0",
        );
        let snapshot = parse_porcelain_v2_full(input).unwrap();
        assert_eq!(snapshot.files.len(), 2);
        assert_eq!(snapshot.branch.name, "main");
        assert_eq!(snapshot.branch.ahead_behind, Some((2, 1)));
    }
}
