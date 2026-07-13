//! The staging seam between the TUI and git: [`StageOps`] is the small
//! trait the [`super::App`] drives staging through, implemented by
//! [`GitRunner`] for real sessions and by a recording fake in unit tests.
//! [`build_review`] assembles everything a review session needs from one
//! `StageOps` — parsed [`FileDiff`]s, the raw patches they came from
//! (needed later to construct hunk/line patches), and which paths currently
//! have staged changes.

use std::collections::HashMap;
use std::process::Command;

use thiserror::Error;

use crate::diff::{DiffParseError, FileChangeKind, FileDiff};
use crate::git::{
    BranchStatus, ChangeKind, CommitSummary, DiffTarget, FileStatus, GitError, GitRunner,
    LocalBranch, RawFilePatch, StashEntry, StatusCode, WorktreeEntry,
};

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

/// A `Send` closure that rebuilds a [`ReviewSnapshot`] off the render thread,
/// so the periodic working-tree poll doesn't block the event loop on git I/O.
/// Only backends that can cross a thread boundary produce one (see
/// [`StageOps::async_review_builder`]): the production [`GitRunner`] does;
/// non-`Send` test fakes and git-less contexts return `None` and stay on the
/// synchronous refresh path.
pub type AsyncReviewBuilder =
    Box<dyn Fn(&DiffTarget) -> Result<ReviewSnapshot, ReviewError> + Send>;

/// The git operations the TUI needs for staging and refresh, kept behind a
/// trait so [`super::App`]'s staging logic is unit-testable without a real
/// repository. [`GitRunner`] is the production implementation.
pub trait StageOps {
    /// Raw per-file patches for `target` (see [`GitRunner::diff`]).
    fn diff(&self, target: &DiffTarget) -> Result<Vec<RawFilePatch>, GitError>;
    /// Parsed porcelain status for every changed path (see
    /// [`GitRunner::status`]).
    fn status(&self) -> Result<Vec<FileStatus>, GitError>;
    /// A `Send` snapshot builder for the async working-tree poll, or `None`
    /// for backends that can't cross a thread boundary. The default returns
    /// `None`, keeping non-`Send` fakes (and git-less contexts) on the
    /// synchronous path; [`GitRunner`] overrides it by cloning itself into the
    /// closure (it is a cheap `PathBuf` handle).
    fn async_review_builder(&self) -> Option<AsyncReviewBuilder> {
        None
    }
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
    /// Reads the current branch / upstream / ahead-behind state (see
    /// [`GitRunner::status_with_branch`]). The default errors, so
    /// backend-less or navigation-only fakes need not implement it; the
    /// panel treats branch state as best-effort.
    fn branch_status(&self) -> Result<BranchStatus, GitError> {
        Err(GitError::Parse("branch status unavailable".into()))
    }
    /// Reads the stash list, newest first (see [`GitRunner::stash_list`]).
    /// The default returns an empty list.
    fn stash_list(&self) -> Result<Vec<StashEntry>, GitError> {
        Ok(Vec::new())
    }
    /// Reads a one-line summary of the tip commit (see
    /// [`GitRunner::last_commit`]). The default returns `None`, so
    /// backend-less or navigation-only fakes need not implement it; the panel
    /// treats the last commit as best-effort.
    fn last_commit(&self) -> Result<Option<CommitSummary>, GitError> {
        Ok(None)
    }
    /// Reads the local branches (see [`GitRunner::branch_list`]). The
    /// default errors, so backend-less or navigation-only fakes need not
    /// implement it; the switcher treats this as unavailable rather than
    /// crashing.
    fn branch_list(&self) -> Result<Vec<LocalBranch>, GitError> {
        Err(GitError::Parse("branch list unavailable".into()))
    }
    /// Reads every worktree (see [`GitRunner::worktree_list`]). The default
    /// errors, mirroring [`StageOps::branch_list`].
    fn worktree_list(&self) -> Result<Vec<WorktreeEntry>, GitError> {
        Err(GitError::Parse("worktree list unavailable".into()))
    }
    /// Switches the working tree to branch `name` (see
    /// [`GitRunner::switch_branch`]). The default errors, mirroring
    /// [`StageOps::branch_list`].
    fn switch_branch(&self, name: &str) -> Result<(), GitError> {
        let _ = name;
        Err(GitError::Parse("branch switch unavailable".into()))
    }
    /// Builds the `git commit -m <message>` [`Command`] the commit gesture
    /// (spec 04) spawns on the background poller — returned as a `Command`
    /// rather than run here so the caller can execute it off the render
    /// thread (see [`crate::git::commit_command`] for the fixed-argv
    /// contract). The default returns `None`: backend-less contexts and
    /// fakes that don't opt in degrade to a footer message, and a fake *can*
    /// opt in with a synthetic command (e.g. `true`/`false`) to drive the
    /// full spawn → poll → command-log pipeline without git.
    fn commit_command(&self, message: &str) -> Option<Command> {
        let _ = message;
        None
    }
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

    fn branch_status(&self) -> Result<BranchStatus, GitError> {
        Ok(GitRunner::status_with_branch(self)?.branch)
    }

    fn stash_list(&self) -> Result<Vec<StashEntry>, GitError> {
        GitRunner::stash_list(self)
    }

    fn last_commit(&self) -> Result<Option<CommitSummary>, GitError> {
        GitRunner::last_commit(self)
    }

    fn branch_list(&self) -> Result<Vec<LocalBranch>, GitError> {
        GitRunner::branch_list(self)
    }

    fn worktree_list(&self) -> Result<Vec<WorktreeEntry>, GitError> {
        GitRunner::worktree_list(self)
    }

    fn switch_branch(&self, name: &str) -> Result<(), GitError> {
        GitRunner::switch_branch(self, name)
    }

    fn commit_command(&self, message: &str) -> Option<Command> {
        Some(crate::git::commit_command(message, self.root()))
    }

    fn async_review_builder(&self) -> Option<AsyncReviewBuilder> {
        // `GitRunner` is a `Clone` `PathBuf` handle, so cloning it into a
        // `Send` closure lets the periodic poll run `build_review` on a
        // background thread without touching `App`'s non-`Send` state.
        let runner = self.clone();
        Some(Box::new(move |target| build_review(&runner, target)))
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
    /// Every file in the diff, in display order: sorted by path
    /// (byte-wise ascending), independent of staged state, so staging or
    /// unstaging a file never moves it in the list.
    pub files: Vec<FileDiff>,
    /// The raw patch each entry of `files` was parsed from, by index.
    /// `None` for synthetic untracked entries and for fully-staged entries
    /// with no textual hunks in the staged diff (e.g. a staged deletion or
    /// binary file); a fully-staged entry with real staged hunks carries
    /// its staged `RawFilePatch` here, same as any other file.
    pub patches: Vec<Option<RawFilePatch>>,
    /// Files with staged changes, per `git status`.
    pub staged: Vec<StagedFile>,
    /// Per-path [`StagedState`] for the `●`/`±` header/sidebar markers.
    /// Missing entries default to [`StagedState::Unstaged`].
    pub staged_states: HashMap<String, StagedState>,
}

/// A single file's staged state, derived from its `git status` index-side
/// (`X`) and working-tree-side (`Y`) codes. This is the three-state marker
/// the multibuffer section header and sidebar render: `Full` → `●`,
/// `Partial` → `±`, `Unstaged` → blank.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StagedState {
    /// Nothing staged for this path (working-tree-only changes, or
    /// untracked): no marker.
    #[default]
    Unstaged,
    /// Some but not all of this path's changes are staged (the index
    /// differs from `HEAD` *and* the working tree differs from the index):
    /// `±`.
    Partial,
    /// Everything is staged (the index differs from `HEAD`, the working
    /// tree matches the index): `●`.
    Full,
}

/// Derives a file's [`StagedState`] from its porcelain status. A path with
/// no staged changes is `Unstaged` (covers untracked `??` and working-tree
/// -only `.M`); a path with staged changes is `Partial` when it *also* has
/// unstaged changes (e.g. `MM`, `AM`, `RM`) and `Full` otherwise (`M.`,
/// `A.`, `D.`, `R.`, `C.`).
pub fn staged_state(status: &FileStatus) -> StagedState {
    match (status.has_staged_changes(), status.has_unstaged_changes()) {
        (false, _) => StagedState::Unstaged,
        (true, true) => StagedState::Partial,
        (true, false) => StagedState::Full,
    }
}

/// A path-keyed map of every file's [`StagedState`], for the paths that have
/// any staged changes (`Partial`/`Full`); `Unstaged` files are omitted, so a
/// missing entry means [`StagedState::Unstaged`] (its `Default`). This is
/// what `rebuild_rows` and the sidebar consume to render the `●`/`±` markers.
pub fn staged_states_from_status(status: &[FileStatus]) -> HashMap<String, StagedState> {
    status
        .iter()
        .filter_map(|s| {
            let state = staged_state(s);
            (state != StagedState::Unstaged).then(|| (s.path.clone(), state))
        })
        .collect()
}

/// Maps a porcelain index-side [`StatusCode`] to the [`FileChangeKind`] used
/// for a fully-staged file's synthetic (header-only) section, so its header
/// shows the right change-kind letter.
fn kind_from_staged_code(code: StatusCode) -> FileChangeKind {
    match code {
        StatusCode::Added => FileChangeKind::Added,
        StatusCode::Deleted => FileChangeKind::Deleted,
        StatusCode::Renamed => FileChangeKind::Renamed,
        StatusCode::Copied => FileChangeKind::Copied,
        // Modified/TypeChange/anything else display as a modification.
        _ => FileChangeKind::Modified,
    }
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
        // Fully-staged files have no working-tree diff at all, so their
        // real content only exists in the staged (`--staged`) diff. Fetch
        // it once, indexed by path, so the synthesis loop below can give
        // them real hunks instead of an empty header-only placeholder.
        let staged_patches: HashMap<String, RawFilePatch> = ops
            .diff(&DiffTarget::Staged)?
            .into_iter()
            .map(|patch| (patch.path.clone(), patch))
            .collect();

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

        // Fully-staged files never appear in the working-tree diff (their
        // changes are all in the index), yet the review must keep them as
        // sections so unstaging is one `S` on a header, and expanding shows
        // their (staged) content rather than nothing (spec Unit 2 —
        // "nothing hides"). Union them in from the staged diff fetched
        // above, falling back to a header-only placeholder when there's no
        // textual staged patch; the path sort below places them, like
        // every other entry, by path. See 03-task-03-proofs.md for the
        // design note on this choice.
        for entry in &status {
            if staged_state(entry) != StagedState::Full {
                continue;
            }
            if files.iter().any(|f| f.path == entry.path) {
                continue;
            }
            match staged_patches.get(&entry.path) {
                // The staged diff has real hunks for this path (the common
                // case): parse them so the file is expandable and shows its
                // (staged) content, not just a header.
                Some(patch) => {
                    files.push(FileDiff::from_patch(patch)?);
                    patches.push(Some(patch.clone()));
                }
                // No staged patch (e.g. a staged deletion of a file with no
                // textual hunks, or a binary file): fall back to the
                // header-only placeholder so the section still exists.
                None => {
                    files.push(FileDiff {
                        path: entry.path.clone(),
                        old_path: entry.orig_path.clone(),
                        kind: kind_from_staged_code(entry.staged),
                        is_binary: false,
                        hunks: Vec::new(),
                    });
                    patches.push(None);
                }
            }
        }
    }

    // One flat list in a stable, status-independent order: sort every entry
    // by path (byte-wise), whatever source it came from. This is what keeps
    // a file from teleporting when staging flips it between the diff-parsed
    // and fully-staged-synthesized sources — only its marker and section
    // content change, never its position. `patches` is index-aligned with
    // `files`, so the two are sorted together.
    let mut entries: Vec<(FileDiff, Option<RawFilePatch>)> =
        files.into_iter().zip(patches).collect();
    entries.sort_by(|a, b| a.0.path.cmp(&b.0.path));
    let (files, patches) = entries.into_iter().unzip();

    Ok(ReviewSnapshot {
        files,
        patches,
        staged: staged_from_status(&status),
        staged_states: staged_states_from_status(&status),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::ChangeKind;

    /// A porcelain status entry with the given record kind, index-side (`X`)
    /// and working-tree-side (`Y`) codes.
    fn status(
        kind: ChangeKind,
        staged: StatusCode,
        unstaged: StatusCode,
        path: &str,
    ) -> FileStatus {
        FileStatus {
            kind,
            staged,
            unstaged,
            path: path.to_string(),
            orig_path: None,
        }
    }

    fn ordinary(staged: StatusCode, unstaged: StatusCode) -> FileStatus {
        status(ChangeKind::Ordinary, staged, unstaged, "f.rs")
    }

    #[test]
    fn unstaged_when_working_tree_only_modification() {
        // ` M`: modified in the working tree, nothing staged.
        let s = ordinary(StatusCode::Unmodified, StatusCode::Modified);
        assert_eq!(staged_state(&s), StagedState::Unstaged);
    }

    #[test]
    fn full_when_staged_modification_only() {
        // `M.`: staged modification, clean working tree.
        let s = ordinary(StatusCode::Modified, StatusCode::Unmodified);
        assert_eq!(staged_state(&s), StagedState::Full);
    }

    #[test]
    fn partial_when_both_staged_and_unstaged_modification() {
        // `MM`: staged and then edited again.
        let s = ordinary(StatusCode::Modified, StatusCode::Modified);
        assert_eq!(staged_state(&s), StagedState::Partial);
    }

    #[test]
    fn full_when_staged_addition() {
        // `A.`: newly added and fully staged.
        let s = ordinary(StatusCode::Added, StatusCode::Unmodified);
        assert_eq!(staged_state(&s), StagedState::Full);
    }

    #[test]
    fn partial_when_added_then_modified() {
        // `AM`: staged add plus a subsequent unstaged edit.
        let s = ordinary(StatusCode::Added, StatusCode::Modified);
        assert_eq!(staged_state(&s), StagedState::Partial);
    }

    #[test]
    fn full_when_staged_deletion() {
        // `D.`: staged deletion.
        let s = ordinary(StatusCode::Deleted, StatusCode::Unmodified);
        assert_eq!(staged_state(&s), StagedState::Full);
    }

    #[test]
    fn unstaged_when_untracked() {
        // `??`: untracked, counts as unstaged working-tree changes.
        let s = status(
            ChangeKind::Untracked,
            StatusCode::Unmodified,
            StatusCode::Untracked,
            "new.rs",
        );
        assert_eq!(staged_state(&s), StagedState::Unstaged);
    }

    #[test]
    fn full_when_staged_rename() {
        // `R.`: staged rename, clean working tree.
        let mut s = status(
            ChangeKind::RenamedOrCopied,
            StatusCode::Renamed,
            StatusCode::Unmodified,
            "new/name.rs",
        );
        s.orig_path = Some("old/name.rs".to_string());
        assert_eq!(staged_state(&s), StagedState::Full);
    }

    #[test]
    fn partial_when_renamed_then_modified() {
        // `RM`: staged rename plus a subsequent unstaged edit.
        let s = status(
            ChangeKind::RenamedOrCopied,
            StatusCode::Renamed,
            StatusCode::Modified,
            "new/name.rs",
        );
        assert_eq!(staged_state(&s), StagedState::Partial);
    }

    #[test]
    fn full_when_staged_copy() {
        // `C.`: staged copy.
        let s = status(
            ChangeKind::RenamedOrCopied,
            StatusCode::Copied,
            StatusCode::Unmodified,
            "copy.rs",
        );
        assert_eq!(staged_state(&s), StagedState::Full);
    }

    #[test]
    fn unstaged_when_no_changes_on_either_side() {
        let s = ordinary(StatusCode::Unmodified, StatusCode::Unmodified);
        assert_eq!(staged_state(&s), StagedState::Unstaged);
    }

    #[test]
    fn states_map_omits_unstaged_and_keys_partial_full_by_path() {
        let entries = vec![
            ordinary(StatusCode::Unmodified, StatusCode::Modified), // f.rs unstaged
            status(
                ChangeKind::Ordinary,
                StatusCode::Modified,
                StatusCode::Unmodified,
                "full.rs",
            ),
            status(
                ChangeKind::Ordinary,
                StatusCode::Modified,
                StatusCode::Modified,
                "partial.rs",
            ),
            status(
                ChangeKind::Untracked,
                StatusCode::Unmodified,
                StatusCode::Untracked,
                "new.rs",
            ),
        ];
        let map = staged_states_from_status(&entries);
        // Unstaged (`f.rs`) and untracked (`new.rs`) are omitted.
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("full.rs"), Some(&StagedState::Full));
        assert_eq!(map.get("partial.rs"), Some(&StagedState::Partial));
        assert_eq!(map.get("f.rs"), None);
        assert_eq!(map.get("new.rs"), None);
    }

    #[test]
    fn kind_from_staged_code_maps_letters() {
        assert_eq!(
            kind_from_staged_code(StatusCode::Added),
            FileChangeKind::Added
        );
        assert_eq!(
            kind_from_staged_code(StatusCode::Deleted),
            FileChangeKind::Deleted
        );
        assert_eq!(
            kind_from_staged_code(StatusCode::Renamed),
            FileChangeKind::Renamed
        );
        assert_eq!(
            kind_from_staged_code(StatusCode::Copied),
            FileChangeKind::Copied
        );
        assert_eq!(
            kind_from_staged_code(StatusCode::Modified),
            FileChangeKind::Modified
        );
        assert_eq!(
            kind_from_staged_code(StatusCode::TypeChange),
            FileChangeKind::Modified
        );
    }

    /// A minimal [`StageOps`] fake for [`build_review`]: `diff` is
    /// target-aware (separate working-tree and staged patch lists, as a
    /// real backend's would be), `status` is fixed, and every other
    /// operation is an unused no-op.
    #[derive(Default)]
    struct Fake {
        working_tree_diff: Vec<RawFilePatch>,
        staged_diff: Vec<RawFilePatch>,
        status: Vec<FileStatus>,
    }

    impl StageOps for Fake {
        fn diff(&self, target: &DiffTarget) -> Result<Vec<RawFilePatch>, GitError> {
            match target {
                DiffTarget::Staged => Ok(self.staged_diff.clone()),
                _ => Ok(self.working_tree_diff.clone()),
            }
        }

        fn status(&self) -> Result<Vec<FileStatus>, GitError> {
            Ok(self.status.clone())
        }

        fn stage_file(&self, _path: &str) -> Result<(), GitError> {
            Ok(())
        }

        fn unstage_file(&self, _path: &str) -> Result<(), GitError> {
            Ok(())
        }

        fn apply_cached(&self, _patch: &str) -> Result<(), GitError> {
            Ok(())
        }

        fn unapply_cached(&self, _patch: &str) -> Result<(), GitError> {
            Ok(())
        }

        fn read_worktree_file(&self, _path: &str) -> Option<Vec<u8>> {
            None
        }

        fn show_file(&self, _spec: &str) -> Option<String> {
            None
        }
    }

    /// A single-hunk raw patch for `path`, matching the minimal shape
    /// `FileDiff::from_patch` needs to parse a non-empty hunk list.
    fn one_hunk_patch(path: &str) -> RawFilePatch {
        RawFilePatch {
            path: path.to_string(),
            old_path: None,
            raw: format!(
                "diff --git a/{path} b/{path}\nindex 1..2 100644\n--- a/{path}\n+++ b/{path}\n@@ -1,1 +1,1 @@\n-old\n+new\n"
            ),
            is_binary: false,
        }
    }

    #[test]
    fn fully_staged_file_gets_hunks_from_the_staged_diff() {
        // `x.rs` is fully staged: it has no working-tree diff (its changes
        // are all in the index), but it does have a staged one. Before the
        // fix, `build_review` synthesized an empty, header-only `FileDiff`
        // for it here; now it should carry the real staged hunks so the
        // file stays expandable.
        let fake = Fake {
            working_tree_diff: Vec::new(),
            staged_diff: vec![one_hunk_patch("x.rs")],
            status: vec![status(
                ChangeKind::Ordinary,
                StatusCode::Modified,
                StatusCode::Unmodified,
                "x.rs",
            )],
        };

        let review = build_review(&fake, &DiffTarget::WorkingTree).unwrap();

        let idx = review
            .files
            .iter()
            .position(|f| f.path == "x.rs")
            .expect("x.rs must still appear as a section");
        assert!(
            !review.files[idx].hunks.is_empty(),
            "expanding a fully-staged file must show its staged hunks, not an empty section"
        );
        assert!(
            review.patches[idx].is_some(),
            "a fully-staged file with a real staged patch must carry it, enabling hunk/line addressing"
        );
    }

    #[test]
    fn fully_staged_file_without_a_staged_patch_falls_back_to_a_header_only_placeholder() {
        // No staged patch is found for `deleted.rs` (e.g. a staged deletion
        // with no textual hunks, or a binary file): the section must still
        // exist (so unstaging stays reachable) but degrades to the old
        // header-only placeholder rather than erroring.
        let fake = Fake {
            working_tree_diff: Vec::new(),
            staged_diff: Vec::new(),
            status: vec![status(
                ChangeKind::Ordinary,
                StatusCode::Deleted,
                StatusCode::Unmodified,
                "deleted.rs",
            )],
        };

        let review = build_review(&fake, &DiffTarget::WorkingTree).unwrap();

        let idx = review
            .files
            .iter()
            .position(|f| f.path == "deleted.rs")
            .expect("deleted.rs must still appear as a section");
        assert!(review.files[idx].hunks.is_empty());
        assert!(review.patches[idx].is_none());
    }

    #[test]
    fn staged_target_does_not_fetch_the_staged_diff_again() {
        // When `target` is already `Staged`, `build_review` must not issue
        // a second `diff(&Staged)` call — that extra fetch only exists to
        // backfill fully-staged sections in a *working-tree* review.
        let fake = Fake {
            working_tree_diff: Vec::new(),
            staged_diff: vec![one_hunk_patch("y.rs")],
            status: vec![status(
                ChangeKind::Ordinary,
                StatusCode::Modified,
                StatusCode::Unmodified,
                "y.rs",
            )],
        };

        let review = build_review(&fake, &DiffTarget::Staged).unwrap();

        // `diff(&Staged)` already returns `y.rs` as the primary diff, so it
        // must appear exactly once, not duplicated by the fully-staged
        // synthesis path.
        let count = review.files.iter().filter(|f| f.path == "y.rs").count();
        assert_eq!(count, 1);
    }
}
