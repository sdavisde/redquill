//! Shared "ensure a review session" orchestration: resolving the base ref
//! and ensuring a managed worktree exists for a branch under review, plus
//! loading and reconciling that branch's persisted progress. Both entry
//! points — the CLI's `--review` flag (`main.rs::resolve_session`) and the
//! Review launcher's Branches tab ([`super::review_launcher`]) — call
//! through these same functions, so there is exactly one "ensure a review
//! session" code path, not two copies that could drift.
//!
//! Operates over `&dyn StageOps`, exactly like
//! [`super::stage_ops::build_review`]: the CLI's concrete `GitRunner`
//! coerces to the trait object at the call site (`GitRunner: StageOps`), so
//! `main.rs` can call these without any of `redquill`'s TUI types leaking
//! into its own layering, and the launcher calls them through `App`'s
//! existing `stage_ops` handle.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::annotate::PersistedAnnotation;
use crate::git::{GitError, sanitize_branch_dir_name};
use crate::review::{ReviewStatus, reconcile, store};

use super::stage_ops::StageOps;

/// Resolves the base ref for a review: `base_override` (`--base`, the CLI's
/// own flag — the in-app modal never passes one, always `None`) if given,
/// else [`StageOps::default_base`]'s fallback chain (`origin/HEAD` → `main`
/// → `master`).
pub fn resolve_review_base(
    ops: &dyn StageOps,
    base_override: Option<&str>,
) -> Result<String, GitError> {
    match base_override {
        Some(base) => Ok(base.to_string()),
        None => ops.default_base(),
    }
}

/// Ensures a managed worktree exists for `branch`, at
/// `<git-common-dir>/redquill/worktrees/<sanitized-branch>`, and returns its
/// path. Reuses an existing worktree (a paused review) rather than creating
/// a new one when `git worktree list` already knows about the path;
/// otherwise creates it with [`StageOps::worktree_add`], which surfaces
/// git's own error message (unknown branch, branch checked out elsewhere,
/// path collision, ...) without side effects on failure.
pub fn ensure_review_worktree(ops: &dyn StageOps, branch: &str) -> Result<PathBuf, GitError> {
    let worktree_path = review_worktree_path(ops, branch)?;

    if !worktree_registered(ops, &worktree_path)? {
        ops.worktree_add(&worktree_path, branch)?;
    }

    Ok(worktree_path)
}

/// The managed worktree path a review of `branch` resolves to
/// (`<git-common-dir>/redquill/worktrees/<sanitized-branch>`) — the single
/// place that layout is computed, shared by [`ensure_review_worktree`] and
/// the PR-checkout flow (which must know the path to test for an existing
/// worktree before deciding whether to recreate it).
pub fn review_worktree_path(ops: &dyn StageOps, branch: &str) -> Result<PathBuf, GitError> {
    let common_dir = ops.git_common_dir()?;
    let dir_name = sanitize_branch_dir_name(branch);
    Ok(common_dir.join("redquill").join("worktrees").join(dir_name))
}

/// Whether `worktree_path` is a live, git-registered worktree (both present
/// on disk and known to `git worktree list`) — a paused review to reuse
/// rather than recreate.
pub fn worktree_registered(ops: &dyn StageOps, worktree_path: &Path) -> Result<bool, GitError> {
    Ok(worktree_path.exists()
        && ops
            .worktree_list()?
            .iter()
            .any(|entry| paths_match(&entry.path, worktree_path)))
}

/// Resolves `<git-common-dir>/redquill/review-state.json`'s path through
/// `ops` — the same location `main.rs::gc_review_state` resolves at launch,
/// recomputed here so the in-app modal (which never sees that CLI-only GC
/// pass; it runs once per process at startup, before any modal could open)
/// can still attach the right persistence path to a session it starts
/// mid-run.
pub fn resolve_review_state_path(ops: &dyn StageOps) -> Result<PathBuf, GitError> {
    Ok(ops
        .git_common_dir()?
        .join("redquill")
        .join("review-state.json"))
}

/// Whether `a` and `b` name the same filesystem location, canonicalizing
/// both when possible (falling back to a direct comparison for a path that
/// doesn't exist, e.g. before the first `worktree add`) — macOS tempdirs
/// live under a symlinked root, so a raw `PathBuf` comparison can spuriously
/// disagree with what `git worktree list` reports.
pub fn paths_match(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// [`load_reconciled_review_state`]'s return shape: reconciled file statuses,
/// their 1:1 blob-SHA companion map, this branch's persisted annotations
/// verbatim, and its persisted draft replies verbatim — named so clippy's
/// `type_complexity` lint (and any reader) doesn't have to parse a
/// four-deep nested tuple inline.
pub type ReconciledReviewState = (
    HashMap<String, ReviewStatus>,
    HashMap<String, Option<String>>,
    Vec<PersistedAnnotation>,
    Vec<crate::review::store::PersistedReply>,
);

/// Loads and reconciles `branch`'s persisted review state against its
/// *current* blob SHAs, resolved through `ops` (the session's own backend,
/// rooted inside the review worktree, so `blob_sha` reads the branch's real
/// current tip). Returns empty maps when nothing was ever persisted for
/// this branch — an entirely ordinary first review, not an error. The
/// second map mirrors `review_states`' 1:1 blob-SHA companion
/// `App::review_blob_shas` expects, holding the *persisted* SHA rather than
/// `ops`'s freshly-read current one: for a stale (`ChangedSinceAccepted`)
/// result, keeping the persisted SHA is what lets the next session
/// re-derive `ChangedSinceAccepted` again instead of the reconciliation
/// silently losing track of the staleness.
///
/// The third element is this branch's persisted annotations, verbatim and
/// in their original order — annotations have no reconciliation step in v1
/// (see `crate::annotate::persist`'s module doc on the accepted
/// anchor-drift limitation): they're simply carried through for the caller
/// to replay into `app.annotations` before the first render.
pub fn load_reconciled_review_state(
    ops: &dyn StageOps,
    state_path: &Path,
    branch: &str,
) -> ReconciledReviewState {
    let state = store::load(state_path);
    let Some(review) = state.reviews.get(branch) else {
        return (HashMap::new(), HashMap::new(), Vec::new(), Vec::new());
    };
    let mut current_shas = HashMap::new();
    for path in review.files.keys() {
        let sha = ops.blob_sha(branch, path).unwrap_or(None);
        current_shas.insert(path.clone(), sha);
    }
    let statuses = reconcile(review, &current_shas);
    let mut blob_shas = HashMap::new();
    for (path, status) in &statuses {
        if matches!(
            status,
            ReviewStatus::Accepted | ReviewStatus::ChangedSinceAccepted
        ) && let Some(entry) = review.files.get(path)
        {
            blob_shas.insert(path.clone(), entry.blob_sha.clone());
        }
    }
    (
        statuses,
        blob_shas,
        review.annotations.clone(),
        review.replies.clone(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::GitRunner;
    use std::process::Command;
    use tempfile::TempDir;

    // -- paths_match ------------------------------------------------------

    #[test]
    fn paths_match_identical_nonexistent_paths() {
        // Neither side exists, so this exercises the direct-comparison
        // fallback rather than canonicalize.
        let p = PathBuf::from("/no/such/path/anywhere");
        assert!(paths_match(&p, &p));
    }

    #[test]
    fn paths_match_distinguishes_different_nonexistent_paths() {
        assert!(!paths_match(
            Path::new("/no/such/path/a"),
            Path::new("/no/such/path/b")
        ));
    }

    // -- load_reconciled_review_state ----------------------------------------
    //
    // Real-git tempdir tests exercising reconciliation end to end: every
    // fixture is built with `tempfile`, every mutating call is preceded by
    // `assert_inside_tempdir` (a local copy of the shared guard — this
    // in-crate module can't share code with the `tests/*.rs` binaries, per
    // this repo's established one-copy-per-file convention).

    fn git(dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .expect("failed to spawn git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn write(dir: &Path, rel: &str, contents: &str) {
        std::fs::write(dir.join(rel), contents).unwrap();
    }

    fn canon(path: &Path) -> PathBuf {
        std::fs::canonicalize(path).unwrap_or_else(|e| panic!("canonicalize {path:?}: {e}"))
    }

    fn assert_inside_tempdir(path: &Path, tmp: &TempDir) {
        let tmp_root = canon(tmp.path());
        let mut probe = path.to_path_buf();
        while !probe.exists() {
            match probe.parent() {
                Some(parent) => probe = parent.to_path_buf(),
                None => panic!("path {path:?} has no existing ancestor to canonicalize"),
            }
        }
        assert!(
            canon(&probe).starts_with(&tmp_root),
            "refusing to run a mutating git call outside the tempdir: {path:?}"
        );
    }

    fn repo_with_branch(name: &str) -> TempDir {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        assert_inside_tempdir(dir, &tmp);
        git(dir, &["init", "-q", "-b", name]);
        git(dir, &["config", "user.email", "test@redquill.invalid"]);
        git(dir, &["config", "user.name", "redquill test"]);
        git(dir, &["config", "commit.gpgsign", "false"]);
        write(dir, "base.txt", "line one\n");
        git(dir, &["add", "."]);
        git(dir, &["commit", "-qm", "initial"]);
        tmp
    }

    #[test]
    fn demotes_a_changed_file_and_carries_over_the_rest() {
        let repo = repo_with_branch("main");
        write(repo.path(), "a.rs", "fn a() {}\n");
        write(repo.path(), "b.rs", "fn b() {}\n");
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-qm", "add a.rs and b.rs"]);
        let runner = GitRunner::discover_in(repo.path()).unwrap();
        let a_sha_at_accept = runner.blob_sha("main", "a.rs").unwrap().unwrap();
        let b_sha_at_accept = runner.blob_sha("main", "b.rs").unwrap().unwrap();

        let common_dir = runner.git_common_dir().unwrap();
        let state_path = common_dir.join("redquill").join("review-state.json");
        assert_inside_tempdir(&state_path, &repo);
        let mut files = std::collections::BTreeMap::new();
        files.insert(
            "a.rs".to_string(),
            store::PersistedFile {
                status: store::PersistedStatus::Accepted,
                blob_sha: Some(a_sha_at_accept),
            },
        );
        files.insert(
            "b.rs".to_string(),
            store::PersistedFile {
                status: store::PersistedStatus::Accepted,
                blob_sha: Some(b_sha_at_accept.clone()),
            },
        );
        files.insert(
            "c.rs".to_string(),
            store::PersistedFile {
                status: store::PersistedStatus::Deferred,
                blob_sha: None,
            },
        );
        store::save_review(
            &state_path,
            "main",
            store::PersistedReview {
                base: "main".to_string(),
                worktree_path: repo.path().to_path_buf(),
                files,
                annotations: Vec::new(),
                replies: Vec::new(),
                forge: None,
            },
        )
        .unwrap();

        // Change a.rs on the branch after the "accept" above.
        write(repo.path(), "a.rs", "fn a() { changed(); }\n");
        git(repo.path(), &["commit", "-aqm", "change a.rs"]);

        let (states, blob_shas, annotations, _replies) =
            load_reconciled_review_state(&runner, &state_path, "main");
        assert!(
            annotations.is_empty(),
            "this fixture never persisted any annotations"
        );
        assert_eq!(
            states.get("a.rs"),
            Some(&ReviewStatus::ChangedSinceAccepted)
        );
        assert_eq!(states.get("b.rs"), Some(&ReviewStatus::Accepted));
        assert_eq!(states.get("c.rs"), Some(&ReviewStatus::Deferred));
        // The stale SHA is preserved (not overwritten with the new one) —
        // `App::persist_review_state`'s contract for `ChangedSinceAccepted`.
        assert_ne!(blob_shas.get("a.rs").cloned().flatten().unwrap(), {
            runner.blob_sha("main", "a.rs").unwrap().unwrap()
        });
        assert_eq!(
            blob_shas.get("b.rs").cloned().flatten(),
            Some(b_sha_at_accept)
        );
    }

    #[test]
    fn empty_for_a_branch_with_no_persisted_entry() {
        let repo = repo_with_branch("main");
        let runner = GitRunner::discover_in(repo.path()).unwrap();
        let common_dir = runner.git_common_dir().unwrap();
        let state_path = common_dir.join("redquill").join("review-state.json");

        let (states, blob_shas, annotations, _replies) =
            load_reconciled_review_state(&runner, &state_path, "main");

        assert!(states.is_empty());
        assert!(blob_shas.is_empty());
        assert!(annotations.is_empty());
    }

    #[test]
    fn returns_persisted_annotations_verbatim() {
        use crate::annotate::{Classification, Side, Source, Target};

        let repo = repo_with_branch("main");
        let runner = GitRunner::discover_in(repo.path()).unwrap();
        let common_dir = runner.git_common_dir().unwrap();
        let state_path = common_dir.join("redquill").join("review-state.json");
        assert_inside_tempdir(&state_path, &repo);

        store::save_review(
            &state_path,
            "main",
            store::PersistedReview {
                base: "main".to_string(),
                worktree_path: repo.path().to_path_buf(),
                files: Default::default(),
                annotations: vec![PersistedAnnotation {
                    target: Target::line("a.rs", 3, Side::New),
                    classification: Classification::Nit,
                    body: "note".to_string(),
                    source: Source::WorkingTree,
                }],
                replies: Vec::new(),
                forge: None,
            },
        )
        .unwrap();

        let (_, _, annotations, _) = load_reconciled_review_state(&runner, &state_path, "main");

        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].body, "note");
        assert_eq!(annotations[0].target, Target::line("a.rs", 3, Side::New));
    }
}
