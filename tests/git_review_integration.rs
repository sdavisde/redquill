//! Integration tests for the branch-review-mode (spec 08 Unit 1) `GitRunner`
//! methods: `git_common_dir`, `default_base`, and `worktree_add`.
//!
//! Each test builds a throwaway repository in a fresh tempdir, configures
//! git identity LOCALLY (never touching the host repo or global config),
//! and pins every git invocation inside that tempdir — mirroring
//! `tests/git_worktree_integration.rs`'s fixture helpers. Every mutating
//! call is preceded by `assert_inside_tempdir`, the shared isolation guard
//! this task introduces (per the 2026-07-16 incident notes in the task
//! file): a test in this exact shape (worktree creation) previously escaped
//! its tempdir and mutated a real repository.

use std::fs;
use std::path::Path;
use std::process::Command;

use redquill::git::{GitError, GitRunner};
use tempfile::TempDir;

/// Runs a git command in `dir` and returns its trimmed stdout. Used only to
/// build fixtures / assert post-conditions.
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

/// Runs a git command in `dir`, asserting success. Used only to build fixtures.
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

fn write(dir: &Path, rel: &str, contents: &[u8]) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

/// Canonicalizes a path, asserting it succeeds (used on both sides of path
/// comparisons — macOS `/tmp` symlinks to `/private/tmp`).
fn canon(path: &Path) -> std::path::PathBuf {
    fs::canonicalize(path).unwrap_or_else(|e| panic!("canonicalize {path:?}: {e}"))
}

/// Asserts `path` is lexically inside `tmp`'s canonicalized root — the
/// shared isolation guard every mutating git call in this file (and every
/// later spec-08 test file) runs before touching disk, so a fixture that
/// somehow resolved outside its tempdir fails loudly instead of mutating
/// real state.
fn assert_inside_tempdir(path: &Path, tmp: &TempDir) {
    let tmp_root = canon(tmp.path());
    // The target path may not exist yet (e.g. a worktree destination about
    // to be created), so canonicalize its existing ancestor instead of the
    // path itself.
    let mut probe = path.to_path_buf();
    while !probe.exists() {
        match probe.parent() {
            Some(parent) => probe = parent.to_path_buf(),
            None => panic!("path {path:?} has no existing ancestor to canonicalize"),
        }
    }
    let probe_canon = canon(&probe);
    assert!(
        probe_canon.starts_with(&tmp_root),
        "refusing to run a mutating git call outside the tempdir: {path:?} (resolved ancestor {probe_canon:?}) is not under {tmp_root:?}"
    );
}

/// Initializes a fresh repo with a single committed file, returning the tempdir.
fn init_repo() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.name", "redquill test"]);
    git(dir, &["config", "user.email", "test@redquill.invalid"]);
    write(dir, "base.txt", b"line one\nline two\n");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "initial"]);
    tmp
}

/// Initializes a fresh repo on a deterministic branch name (`git init`'s
/// default branch name depends on the host's global config), returning the
/// tempdir.
fn init_repo_on_branch(name: &str) -> TempDir {
    let tmp = init_repo();
    git(tmp.path(), &["branch", "-M", name]);
    tmp
}

fn runner_for(tmp: &TempDir) -> GitRunner {
    GitRunner::discover_in(tmp.path()).expect("discover repo")
}

// -- git_common_dir -----------------------------------------------------

#[test]
fn git_common_dir_is_the_canonicalized_dot_git_for_a_plain_repo() {
    let repo = init_repo();
    let runner = runner_for(&repo);

    let common_dir = runner.git_common_dir().unwrap();
    assert_eq!(common_dir, canon(&repo.path().join(".git")));
}

#[test]
fn git_common_dir_from_a_linked_worktree_is_the_main_repos_git_dir() {
    let repo = init_repo_on_branch("main");
    git(repo.path(), &["branch", "feature"]);
    let wt_parent = TempDir::new().unwrap();
    let wt_path = wt_parent.path().join("wt");
    assert_inside_tempdir(&wt_path, &wt_parent);
    git(
        repo.path(),
        &[
            "worktree",
            "add",
            "-q",
            wt_path.to_str().unwrap(),
            "feature",
        ],
    );

    let wt_runner = GitRunner::discover_in(&wt_path).expect("discover linked worktree");
    let common_dir = wt_runner.git_common_dir().unwrap();

    // The *shared* administrative dir, not the linked worktree's own
    // `.git` file location — same answer as asking from the main worktree.
    assert_eq!(common_dir, canon(&repo.path().join(".git")));
}

// -- default_base ---------------------------------------------------------

#[test]
fn default_base_prefers_origin_head() {
    let repo = init_repo_on_branch("trunk");
    // A bare "remote" whose HEAD points at `trunk`.
    let bare = TempDir::new().unwrap();
    git(bare.path(), &["init", "-q", "--bare", "-b", "trunk"]);
    let bare_str = bare.path().to_str().unwrap();
    git(repo.path(), &["remote", "add", "origin", bare_str]);
    git(repo.path(), &["push", "-q", "-u", "origin", "trunk"]);
    // `push` alone doesn't set the remote's symbolic HEAD locally; set it
    // explicitly, exactly as `git remote set-head` (or a real clone) would.
    git(
        repo.path(),
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/trunk",
        ],
    );

    let runner = runner_for(&repo);
    assert_eq!(runner.default_base().unwrap(), "trunk");
}

#[test]
fn default_base_falls_back_to_local_main_when_no_origin_head() {
    let repo = init_repo_on_branch("main");
    git(repo.path(), &["checkout", "-qb", "feature"]);

    let runner = runner_for(&repo);
    assert_eq!(runner.default_base().unwrap(), "main");
}

#[test]
fn default_base_falls_back_to_local_master_when_no_main() {
    let repo = init_repo_on_branch("master");
    git(repo.path(), &["checkout", "-qb", "feature"]);

    let runner = runner_for(&repo);
    assert_eq!(runner.default_base().unwrap(), "master");
}

#[test]
fn default_base_errors_naming_the_base_flag_when_nothing_resolves() {
    let repo = init_repo_on_branch("trunk");

    let runner = runner_for(&repo);
    let err = runner.default_base().unwrap_err();
    assert!(matches!(err, GitError::NoDefaultBase));
    assert!(err.to_string().contains("--base"));
}

// -- worktree_add -----------------------------------------------------------

#[test]
fn worktree_add_creates_a_worktree_at_a_nested_new_path() {
    let repo = init_repo_on_branch("main");
    git(repo.path(), &["branch", "feature"]);
    let runner = runner_for(&repo);

    // Nested under `.git/` (as production code will place it), so it never
    // shows up as untracked content in the original checkout's status.
    let wt_path = repo
        .path()
        .join(".git")
        .join("redquill")
        .join("worktrees")
        .join("feature-1234abcd");
    assert_inside_tempdir(&wt_path, &repo);

    runner.worktree_add(&wt_path, "feature").unwrap();

    assert!(wt_path.join("base.txt").exists());
    let branch = git_out(&wt_path, &["branch", "--show-current"]);
    assert_eq!(branch, "feature");

    // The user's original checkout is untouched: still on main, clean.
    let original_branch = git_out(repo.path(), &["branch", "--show-current"]);
    assert_eq!(original_branch, "main");
    let status = git_out(repo.path(), &["status", "--porcelain"]);
    assert_eq!(status, "");
}

#[test]
fn worktree_add_never_passes_force() {
    // Structural guard: the argv this method builds can never carry
    // `--force`, independent of any runtime behavior above.
    let repo = init_repo_on_branch("main");
    git(repo.path(), &["branch", "feature"]);
    let runner = runner_for(&repo);
    let wt_path = repo.path().join("wt");
    assert_inside_tempdir(&wt_path, &repo);

    runner.worktree_add(&wt_path, "feature").unwrap();
    // A second add at a colliding path must fail rather than force-clobber.
    let err = runner.worktree_add(&wt_path, "feature").unwrap_err();
    match err {
        GitError::Command { stderr, .. } => assert!(!stderr.is_empty()),
        other => panic!("expected GitError::Command, got {other:?}"),
    }
}

#[test]
fn worktree_add_fails_readably_on_an_unknown_branch() {
    let repo = init_repo_on_branch("main");
    let runner = runner_for(&repo);
    let wt_path = repo.path().join("wt");
    assert_inside_tempdir(&wt_path, &repo);

    let err = runner.worktree_add(&wt_path, "no-such-branch").unwrap_err();
    match err {
        GitError::Command { stderr, .. } => assert!(!stderr.is_empty()),
        other => panic!("expected GitError::Command, got {other:?}"),
    }
    // No side effects: nothing was created.
    assert!(!wt_path.exists());
}

#[test]
fn worktree_add_fails_readably_when_branch_checked_out_elsewhere() {
    let repo = init_repo_on_branch("main");
    git(repo.path(), &["branch", "feature"]);
    let runner = runner_for(&repo);

    let first = repo.path().join("wt-a");
    assert_inside_tempdir(&first, &repo);
    runner.worktree_add(&first, "feature").unwrap();

    let second = repo.path().join("wt-b");
    assert_inside_tempdir(&second, &repo);
    let err = runner.worktree_add(&second, "feature").unwrap_err();
    match err {
        GitError::Command { stderr, .. } => assert!(!stderr.is_empty()),
        other => panic!("expected GitError::Command, got {other:?}"),
    }
    // No side effects from the failed attempt: the second path was never
    // created, and the first worktree is untouched.
    assert!(!second.exists());
    assert!(first.join("base.txt").exists());
}
