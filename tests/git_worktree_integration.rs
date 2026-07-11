//! Integration tests for worktree/branch listing and `git switch`.
//!
//! Each test builds throwaway repositories in fresh tempdirs, mirroring
//! `tests/git_integration.rs`'s fixture helpers; nothing here touches the
//! host repo or global git config.

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

/// Writes a file (creating parent dirs) inside the repo.
fn write(dir: &Path, rel: &str, contents: &[u8]) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

/// Initializes a fresh repo with a single committed file, returning the tempdir.
fn init_repo() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-q"]);
    // Identity is set LOCALLY so nothing leaks to the host's global config.
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

/// Canonicalizes a path, asserting it succeeds (used on both sides of path
/// comparisons — macOS `/tmp` symlinks to `/private/tmp`).
fn canon(path: &Path) -> std::path::PathBuf {
    fs::canonicalize(path).unwrap_or_else(|e| panic!("canonicalize {path:?}: {e}"))
}

#[test]
fn worktree_list_reports_main_and_linked_worktree() {
    let repo = init_repo_on_branch("main");
    let wt_parent = TempDir::new().unwrap();
    let wt_path = wt_parent.path().join("wt");
    git(
        repo.path(),
        &[
            "worktree",
            "add",
            "-q",
            wt_path.to_str().unwrap(),
            "-b",
            "feature",
        ],
    );

    let runner = runner_for(&repo);
    let worktrees = runner.worktree_list().unwrap();
    assert_eq!(worktrees.len(), 2);

    let main_canon = canon(repo.path());
    let wt_canon = canon(&wt_path);

    let main_entry = worktrees
        .iter()
        .find(|w| canon(&w.path) == main_canon)
        .expect("main worktree entry present");
    assert_eq!(main_entry.branch.as_deref(), Some("main"));
    assert!(!main_entry.bare);
    assert!(!main_entry.detached);

    let linked_entry = worktrees
        .iter()
        .find(|w| canon(&w.path) == wt_canon)
        .expect("linked worktree entry present");
    assert_eq!(linked_entry.branch.as_deref(), Some("feature"));
    assert!(!linked_entry.bare);
    assert!(!linked_entry.detached);
}

#[test]
fn worktree_list_reports_detached_worktree() {
    let repo = init_repo_on_branch("main");
    let wt_parent = TempDir::new().unwrap();
    let wt_path = wt_parent.path().join("detached-wt");
    git(
        repo.path(),
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            wt_path.to_str().unwrap(),
        ],
    );

    let runner = runner_for(&repo);
    let worktrees = runner.worktree_list().unwrap();

    let wt_canon = canon(&wt_path);
    let entry = worktrees
        .iter()
        .find(|w| canon(&w.path) == wt_canon)
        .expect("detached worktree entry present");
    assert!(entry.detached);
    assert_eq!(entry.branch, None);
    assert!(entry.head.is_some());
}

#[test]
fn branch_list_marks_current_and_worktree_branches() {
    let repo = init_repo_on_branch("main");
    let wt_parent = TempDir::new().unwrap();
    let wt_path = wt_parent.path().join("wt");
    git(
        repo.path(),
        &[
            "worktree",
            "add",
            "-q",
            wt_path.to_str().unwrap(),
            "-b",
            "feature",
        ],
    );

    let runner = runner_for(&repo);
    let branches = runner.branch_list().unwrap();

    let main_branch = branches
        .iter()
        .find(|b| b.name == "main")
        .expect("main branch present");
    assert!(main_branch.is_current);

    let feature_branch = branches
        .iter()
        .find(|b| b.name == "feature")
        .expect("feature branch present");
    assert!(!feature_branch.is_current);
    let feature_wt = feature_branch
        .worktree
        .as_ref()
        .expect("feature branch reports its worktree");
    assert_eq!(canon(feature_wt), canon(&wt_path));
}

#[test]
fn switch_branch_switches_clean_tree() {
    let repo = init_repo_on_branch("main");
    git(repo.path(), &["branch", "feature"]);

    let runner = runner_for(&repo);
    runner.switch_branch("feature").unwrap();

    let branch = git_out(repo.path(), &["branch", "--show-current"]);
    assert_eq!(branch, "feature");
}

#[test]
fn switch_branch_fails_on_conflicting_dirty_tree_with_stderr() {
    let repo = init_repo_on_branch("main");
    git(repo.path(), &["branch", "feature"]);
    git(repo.path(), &["checkout", "-q", "feature"]);
    write(
        repo.path(),
        "base.txt",
        b"line one\nline two\nfeature change\n",
    );
    git(repo.path(), &["commit", "-aqm", "feature change"]);
    git(repo.path(), &["checkout", "-q", "main"]);
    // Dirty change on main that conflicts with feature's committed version
    // of the same file, so `git switch` refuses without --force.
    write(
        repo.path(),
        "base.txt",
        b"line one\nline two\ndirty uncommitted change\n",
    );

    let runner = runner_for(&repo);
    let result = runner.switch_branch("feature");

    match result {
        Err(GitError::Command { stderr, .. }) => assert!(!stderr.is_empty()),
        other => panic!("expected GitError::Command, got {other:?}"),
    }

    // Tree is left untouched: still on main, dirty content intact.
    let branch = git_out(repo.path(), &["branch", "--show-current"]);
    assert_eq!(branch, "main");
    let content = fs::read_to_string(repo.path().join("base.txt")).unwrap();
    assert!(content.contains("dirty uncommitted change"));
}

#[test]
fn switch_branch_fails_when_branch_checked_out_in_another_worktree() {
    let repo = init_repo_on_branch("main");
    let wt_parent = TempDir::new().unwrap();
    let wt_path = wt_parent.path().join("wt");
    git(
        repo.path(),
        &[
            "worktree",
            "add",
            "-q",
            wt_path.to_str().unwrap(),
            "-b",
            "feature",
        ],
    );

    let runner = runner_for(&repo);
    let result = runner.switch_branch("feature");

    match result {
        Err(GitError::Command { stderr, .. }) => assert!(!stderr.is_empty()),
        other => panic!("expected GitError::Command, got {other:?}"),
    }

    let branch = git_out(repo.path(), &["branch", "--show-current"]);
    assert_eq!(branch, "main");
}
