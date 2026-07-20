//! Integration tests for git-layer PR/MR head-ref fetch
//! (`GitRunner::fetch_pr_head`/`fetch_base_ref`/`managed_pr_branches`/
//! `delete_managed_pr_branch`) against a `file://` bare "origin" that
//! advertises PR-style special refs (`refs/pull/<n>/head`) the way GitHub
//! itself would on the real host — everything here is a plain local bare
//! repo under our own control, so no network and no real forge is ever
//! touched.
//!
//! Every repo is built fresh in a tempdir; git identity is configured
//! locally (never the host repo or global config), mirroring
//! `tests/git_remote_integration.rs` and `tests/git_review_integration.rs`.
//! Per this repo's guardrails (and the 2026-07-16 tempdir-leak incident),
//! [`assert_repo_root_inside_tempdir`] runs before the first redquill
//! mutating call in every test.

use std::path::{Path, PathBuf};
use std::process::Command;

use redquill::git::{GitError, GitRunner, PrRef, PrRefKind};
use tempfile::TempDir;

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("failed to spawn git");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_out(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("failed to spawn git");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn write(dir: &Path, rel: &str, contents: &[u8]) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

fn configure_identity(dir: &Path) {
    git(dir, &["config", "user.name", "redquill test"]);
    git(dir, &["config", "user.email", "test@redquill.invalid"]);
}

fn file_url(bare: &Path) -> String {
    format!("file://{}", bare.display())
}

/// Canonicalizes a path, asserting it succeeds (macOS `/tmp` symlinks to
/// `/private/tmp`).
fn canon(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|e| panic!("canonicalize {path:?}: {e}"))
}

/// The shared isolation guard (mirroring `tests/git_review_integration.rs`):
/// fails loudly if a discovered repo root ever resolved outside its own
/// tempdir, rather than silently letting a mutating call touch real state.
fn assert_repo_root_inside_tempdir(runner: &GitRunner, tmp: &TempDir) {
    let root = canon(runner.root());
    let tmp_root = canon(tmp.path());
    assert!(
        root.starts_with(&tmp_root),
        "refusing to run a mutating git call outside the tempdir: {root:?} is not under {tmp_root:?}"
    );
}

/// Builds a bare "origin" with a single committed file pushed to `main`,
/// returning the bare repo's tempdir.
fn setup_bare_origin() -> TempDir {
    let bare = TempDir::new().unwrap();
    git(bare.path(), &["init", "-q", "--bare", "-b", "main"]);

    let seed = TempDir::new().unwrap();
    git(seed.path(), &["init", "-q"]);
    configure_identity(seed.path());
    git(seed.path(), &["branch", "-M", "main"]);
    write(seed.path(), "base.txt", b"line one\n");
    git(seed.path(), &["add", "."]);
    git(seed.path(), &["commit", "-q", "-m", "initial"]);
    git(
        seed.path(),
        &["remote", "add", "origin", &file_url(bare.path())],
    );
    git(seed.path(), &["push", "-q", "-u", "origin", "main"]);

    bare
}

/// Clones `bare` into a fresh tempdir with a local identity configured,
/// returning the clone's own tempdir (the clone's working tree root).
fn clone_of(bare: &Path) -> TempDir {
    let dest = TempDir::new().unwrap();
    git(dest.path(), &["clone", "-q", &file_url(bare), "."]);
    configure_identity(dest.path());
    dest
}

/// From `contributor` (a clone of the bare origin), creates a branch off
/// `main` with a unique commit and pushes it to origin under
/// `refs/pull/<number>/head` — the GitHub-style special ref a real PR would
/// advertise. Leaves `contributor` back on `main`.
fn push_pr_special_ref(contributor: &Path, branch: &str, number: u64) -> String {
    git(contributor, &["checkout", "-qb", branch, "main"]);
    write(
        contributor,
        "base.txt",
        format!("line one\n{branch}\n").as_bytes(),
    );
    git(contributor, &["commit", "-aqm", branch]);
    let sha = git_out(contributor, &["rev-parse", "HEAD"]);
    git(
        contributor,
        &[
            "push",
            "-q",
            "origin",
            &format!("{branch}:refs/pull/{number}/head"),
        ],
    );
    git(contributor, &["checkout", "-q", "main"]);
    sha
}

// -- fetch_pr_head ----------------------------------------------------------

#[test]
fn fetch_pr_head_creates_the_managed_branch_from_a_github_style_special_ref() {
    let bare = setup_bare_origin();
    let contributor = clone_of(bare.path());
    let feature_sha = push_pr_special_ref(contributor.path(), "feature", 1);

    let reviewer = clone_of(bare.path());
    let runner = GitRunner::discover_in(reviewer.path()).unwrap();
    assert_repo_root_inside_tempdir(&runner, &reviewer);

    let pr_ref = PrRef::new(PrRefKind::GitHub, 1);
    runner.fetch_pr_head(&pr_ref).unwrap();

    let managed_sha = git_out(reviewer.path(), &["rev-parse", "redquill/pr/1"]);
    assert_eq!(managed_sha, feature_sha);
}

#[test]
fn fetch_pr_head_forced_refetch_updates_the_managed_branch_after_origin_side_rewrite() {
    let bare = setup_bare_origin();
    let contributor = clone_of(bare.path());
    let first_sha = push_pr_special_ref(contributor.path(), "feature", 1);

    let reviewer = clone_of(bare.path());
    let runner = GitRunner::discover_in(reviewer.path()).unwrap();
    assert_repo_root_inside_tempdir(&runner, &reviewer);

    let pr_ref = PrRef::new(PrRefKind::GitHub, 1);
    runner.fetch_pr_head(&pr_ref).unwrap();
    assert_eq!(
        git_out(reviewer.path(), &["rev-parse", "redquill/pr/1"]),
        first_sha
    );

    // The PR author rewrites history (amend) and force-pushes the same
    // special ref — a real non-fast-forward update on origin's side.
    git(contributor.path(), &["checkout", "-q", "feature"]);
    write(
        contributor.path(),
        "base.txt",
        b"line one\nfeature rewritten\n",
    );
    git(
        contributor.path(),
        &["commit", "-a", "--amend", "-q", "-m", "feature rewritten"],
    );
    let second_sha = git_out(contributor.path(), &["rev-parse", "HEAD"]);
    assert_ne!(first_sha, second_sha, "amend must produce a new commit");
    git(
        contributor.path(),
        &["push", "-qf", "origin", "feature:refs/pull/1/head"],
    );

    // A plain (non-forced) fetch of this refspec would fail non-fast-forward
    // here; redquill's own fetch is always forced for the managed
    // namespace, so it must succeed and move the managed branch forward.
    runner.fetch_pr_head(&pr_ref).unwrap();
    let updated_sha = git_out(reviewer.path(), &["rev-parse", "redquill/pr/1"]);
    assert_eq!(updated_sha, second_sha);
    assert_ne!(updated_sha, first_sha);
}

#[test]
fn fetch_pr_head_of_an_unadvertised_pr_fails_and_creates_nothing() {
    let bare = setup_bare_origin();
    let reviewer = clone_of(bare.path());
    let runner = GitRunner::discover_in(reviewer.path()).unwrap();
    assert_repo_root_inside_tempdir(&runner, &reviewer);

    let err = runner
        .fetch_pr_head(&PrRef::new(PrRefKind::GitHub, 999))
        .unwrap_err();
    match err {
        GitError::Command { stderr, .. } => assert!(!stderr.is_empty()),
        other => panic!("expected GitError::Command, got {other:?}"),
    }

    let verify = Command::new("git")
        .current_dir(reviewer.path())
        .args(["rev-parse", "--verify", "redquill/pr/999"])
        .output()
        .expect("failed to spawn git");
    assert!(
        !verify.status.success(),
        "the managed branch must not have been created on a failed fetch"
    );
}

// -- fetch_base_ref -----------------------------------------------------------

#[test]
fn fetch_base_ref_makes_origin_base_resolve_to_the_latest_remote_commit() {
    let bare = setup_bare_origin();
    let reviewer = clone_of(bare.path());
    let runner = GitRunner::discover_in(reviewer.path()).unwrap();
    assert_repo_root_inside_tempdir(&runner, &reviewer);

    // Advance origin's main from a second, independent clone, out from
    // under the reviewer's clone.
    let advancer = clone_of(bare.path());
    write(advancer.path(), "base.txt", b"line one\nremote advance\n");
    git(advancer.path(), &["commit", "-aqm", "remote advance"]);
    git(advancer.path(), &["push", "-q", "origin", "main"]);
    let remote_head = git_out(advancer.path(), &["rev-parse", "HEAD"]);

    let before = git_out(reviewer.path(), &["rev-parse", "origin/main"]);
    assert_ne!(
        before, remote_head,
        "sanity: the reviewer clone must be stale before fetching"
    );

    runner.fetch_base_ref("main").unwrap();

    let after = git_out(reviewer.path(), &["rev-parse", "origin/main"]);
    assert_eq!(after, remote_head);
}

// -- managed_pr_branches / delete_managed_pr_branch ----------------------------

#[test]
fn managed_pr_branches_lists_only_the_redquill_pr_prefix() {
    let bare = setup_bare_origin();
    let contributor = clone_of(bare.path());
    push_pr_special_ref(contributor.path(), "pr-1-branch", 1);
    push_pr_special_ref(contributor.path(), "pr-2-branch", 2);

    let reviewer = clone_of(bare.path());
    let runner = GitRunner::discover_in(reviewer.path()).unwrap();
    assert_repo_root_inside_tempdir(&runner, &reviewer);

    runner
        .fetch_pr_head(&PrRef::new(PrRefKind::GitHub, 1))
        .unwrap();
    runner
        .fetch_pr_head(&PrRef::new(PrRefKind::GitHub, 2))
        .unwrap();
    // An ordinary local branch, never redquill-managed, must not be swept
    // up by the prefix-scoped listing.
    git(reviewer.path(), &["branch", "some-other-branch"]);

    let names: Vec<String> = runner
        .managed_pr_branches()
        .unwrap()
        .into_iter()
        .map(|b| b.name)
        .collect();
    assert_eq!(names, vec!["redquill/pr/1", "redquill/pr/2"]);
}

#[test]
fn delete_managed_pr_branch_removes_only_the_named_branch() {
    let bare = setup_bare_origin();
    let contributor = clone_of(bare.path());
    push_pr_special_ref(contributor.path(), "pr-1-branch", 1);
    push_pr_special_ref(contributor.path(), "pr-2-branch", 2);

    let reviewer = clone_of(bare.path());
    let runner = GitRunner::discover_in(reviewer.path()).unwrap();
    assert_repo_root_inside_tempdir(&runner, &reviewer);

    runner
        .fetch_pr_head(&PrRef::new(PrRefKind::GitHub, 1))
        .unwrap();
    runner
        .fetch_pr_head(&PrRef::new(PrRefKind::GitHub, 2))
        .unwrap();

    runner.delete_managed_pr_branch(1).unwrap();

    let branches = git_out(reviewer.path(), &["branch", "--list"]);
    assert!(!branches.contains("redquill/pr/1"));
    assert!(branches.contains("redquill/pr/2"));
    assert!(branches.contains("main"));
}

#[test]
fn delete_managed_pr_branch_fails_readably_on_an_unknown_branch() {
    let bare = setup_bare_origin();
    let reviewer = clone_of(bare.path());
    let runner = GitRunner::discover_in(reviewer.path()).unwrap();
    assert_repo_root_inside_tempdir(&runner, &reviewer);

    let err = runner.delete_managed_pr_branch(999).unwrap_err();
    match err {
        GitError::Command { stderr, .. } => assert!(!stderr.is_empty()),
        other => panic!("expected GitError::Command, got {other:?}"),
    }
}
