//! The staging seam between the TUI and git: [`StageOps`] is the small
//! trait the [`super::App`] drives staging through, implemented by
//! [`GitRunner`] for real sessions and by a recording fake in unit tests.
//! [`build_review`] assembles everything a review session needs from one
//! `StageOps` — parsed [`FileDiff`]s, the raw patches they came from
//! (needed later to construct hunk/line patches), and which paths currently
//! have staged changes.

use thiserror::Error;

use crate::diff::{DiffParseError, FileChangeKind, FileDiff};
use crate::git::{ChangeKind, DiffTarget, FileStatus, GitError, GitRunner, RawFilePatch};

/// Errors produced while building a [`ReviewSnapshot`].
#[derive(Debug, Error)]
pub enum ReviewError {
    /// Running or parsing git failed.
    #[error(transparent)]
    Git(#[from] GitError),
    /// A raw patch's hunks could not be parsed.
    #[error(transparent)]
    Parse(#[from] DiffParseError),
}

/// The git operations the TUI needs for staging and refresh, kept behind a
/// trait so [`super::App`]'s staging logic is unit-testable without a real
/// repository. [`GitRunner`] is the production implementation.
pub trait StageOps {
    /// Raw per-file patches for `target` (see [`GitRunner::diff`]).
    fn diff(&self, target: &DiffTarget) -> Result<Vec<RawFilePatch>, GitError>;
    /// Parsed porcelain status for every changed path (see
    /// [`GitRunner::status`]).
    fn status(&self) -> Result<Vec<FileStatus>, GitError>;
    /// Stages `path` in its entirety (see [`GitRunner::stage_file`]).
    fn stage_file(&self, path: &str) -> Result<(), GitError>;
    /// Unstages `path` (see [`GitRunner::unstage_file`]).
    fn unstage_file(&self, path: &str) -> Result<(), GitError>;
    /// Applies `patch` to the index only (see [`GitRunner::apply_cached`]).
    fn apply_cached(&self, patch: &str) -> Result<(), GitError>;
    /// Reverses `patch` against the index only (see
    /// [`GitRunner::unapply_cached`]).
    fn unapply_cached(&self, patch: &str) -> Result<(), GitError>;
    /// Reads an untracked file's working-tree content (repo-relative
    /// `path`), for synthesizing its all-added diff. `None` if unreadable.
    fn read_worktree_file(&self, path: &str) -> Option<Vec<u8>>;
    /// Reads a file's content at a git revision spec (see
    /// [`GitRunner::show_file`]), for sourcing whole-file content the diff
    /// pane highlights syntactically. `None` on any failure.
    fn show_file(&self, spec: &str) -> Option<String>;
}

impl StageOps for GitRunner {
    fn diff(&self, target: &DiffTarget) -> Result<Vec<RawFilePatch>, GitError> {
        GitRunner::diff(self, target)
    }

    fn status(&self) -> Result<Vec<FileStatus>, GitError> {
        GitRunner::status(self)
    }

    fn stage_file(&self, path: &str) -> Result<(), GitError> {
        GitRunner::stage_file(self, path)
    }

    fn unstage_file(&self, path: &str) -> Result<(), GitError> {
        GitRunner::unstage_file(self, path)
    }

    fn apply_cached(&self, patch: &str) -> Result<(), GitError> {
        GitRunner::apply_cached(self, patch)
    }

    fn unapply_cached(&self, patch: &str) -> Result<(), GitError> {
        GitRunner::unapply_cached(self, patch)
    }

    fn read_worktree_file(&self, path: &str) -> Option<Vec<u8>> {
        std::fs::read(self.root().join(path)).ok()
    }

    fn show_file(&self, spec: &str) -> Option<String> {
        GitRunner::show_file(self, spec)
    }
}

/// One file with staged changes, as shown in the staging panel and marked
/// in the sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedFile {
    /// The file's current path, relative to the repo root.
    pub path: String,
    /// The index-side porcelain status letter (`M`, `A`, `D`, ...).
    pub letter: char,
}

/// Everything one review pass over a diff target needs: parsed file diffs,
/// the raw patches they were built from (index-aligned with `files`; `None`
/// for synthetic untracked entries, which have no real patch), and the
/// paths that currently have staged changes.
#[derive(Debug, Clone)]
pub struct ReviewSnapshot {
    /// Every file in the diff, in display order.
    pub files: Vec<FileDiff>,
    /// The raw patch each entry of `files` was parsed from, by index.
    /// `None` for synthetic untracked entries.
    pub patches: Vec<Option<RawFilePatch>>,
    /// Files with staged changes, per `git status`.
    pub staged: Vec<StagedFile>,
}

/// The staged-file list derived from parsed porcelain status.
pub fn staged_from_status(status: &[FileStatus]) -> Vec<StagedFile> {
    status
        .iter()
        .filter(|s| s.has_staged_changes())
        .map(|s| StagedFile {
            path: s.path.clone(),
            letter: s.staged.letter(),
        })
        .collect()
}

/// Builds a [`ReviewSnapshot`] for `target`: the diff's parsed files plus,
/// for the working tree, synthetic all-added entries for untracked files
/// (`git diff` never surfaces those), and the staged-file list from status.
pub fn build_review(
    ops: &dyn StageOps,
    target: &DiffTarget,
) -> Result<ReviewSnapshot, ReviewError> {
    let raw_patches = ops.diff(target)?;
    let status = ops.status()?;

    let mut files = Vec::with_capacity(raw_patches.len());
    let mut patches = Vec::with_capacity(raw_patches.len());
    for patch in raw_patches {
        files.push(FileDiff::from_patch(&patch)?);
        patches.push(Some(patch));
    }

    if matches!(target, DiffTarget::WorkingTree) {
        for entry in &status {
            if entry.kind != ChangeKind::Untracked {
                continue;
            }
            // Unreadable (permissions, race with deletion, ...): skip
            // rather than fail the whole review session.
            let Some(bytes) = ops.read_worktree_file(&entry.path) else {
                continue;
            };
            let file = match String::from_utf8(bytes) {
                Ok(content) => FileDiff::synthetic_added(entry.path.clone(), &content),
                Err(_) => FileDiff {
                    path: entry.path.clone(),
                    old_path: None,
                    kind: FileChangeKind::Added,
                    is_binary: true,
                    hunks: Vec::new(),
                },
            };
            files.push(file);
            patches.push(None);
        }
    }

    Ok(ReviewSnapshot {
        files,
        patches,
        staged: staged_from_status(&status),
    })
}
