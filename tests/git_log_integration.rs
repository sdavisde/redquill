//! Integration tests for `GitRunner::commit_log` (the commit-log read
//! model's pagination), built against throwaway tempdir repos — never the
//! host repo or its config.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use redquill::git::{CommitLogRange, GitRunner};
use tempfile::TempDir;

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

fn runner_for(tmp: &TempDir) -> GitRunner {
    GitRunner::discover_in(tmp.path()).expect("discover repo")
}

/// Canonicalizes `path`, panicking rather than silently comparing raw
/// (potentially symlink-relative, e.g. macOS `/var`) paths.
fn canon(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|e| panic!("canonicalize {path:?}: {e}"))
}

/// The shared isolation guard every fixture in this file runs before
/// touching disk, per this repo's tempdir-isolation convention (local copy,
/// matching how every integration-test file already carries its own — see
/// the 2026-07-16 incident writeup this rule traces back to).
fn assert_inside_tempdir(path: &Path, tmp: &TempDir) {
    let tmp_root = canon(tmp.path());
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
        "refusing to run a git call outside the tempdir: {path:?} (resolved ancestor {probe_canon:?}) is not under {tmp_root:?}"
    );
}

/// Initializes a fresh repo with a local-only identity (nothing leaks to
/// the host's global git config) and no commits yet.
fn init_bare_repo() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    assert_inside_tempdir(dir, &tmp);
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.name", "redquill test"]);
    git(dir, &["config", "user.email", "test@redquill.invalid"]);
    tmp
}

/// A fresh repo with its initial branch explicitly named `branch` (never
/// relying on the ambient `init.defaultBranch` config) and no commits yet —
/// the fixture the range-log tests build a base/head history on top of.
fn init_repo_with_base(branch: &str) -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    assert_inside_tempdir(dir, &tmp);
    git(dir, &["init", "-q", "-b", branch]);
    git(dir, &["config", "user.name", "redquill test"]);
    git(dir, &["config", "user.email", "test@redquill.invalid"]);
    tmp
}

fn commit_file(dir: &Path, contents: &str, message: &str) {
    fs::write(dir.join("file.txt"), contents).unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", message]);
}

/// Builds `count` commits, each touching `file.txt`, with subjects
/// `"commit 0"`, `"commit 1"`, ... in creation order (so `"commit N-1"` is
/// the newest / `HEAD`).
fn commit_n_times(dir: &Path, count: usize) {
    for i in 0..count {
        fs::write(dir.join("file.txt"), format!("content {i}\n")).unwrap();
        git(dir, &["add", "."]);
        git(dir, &["commit", "-qm", &format!("commit {i}")]);
    }
}

#[test]
fn commit_log_on_an_empty_repo_yields_no_entries() {
    let tmp = init_bare_repo();
    let runner = runner_for(&tmp);
    assert!(runner.commit_log(50, 0).unwrap().is_empty());
}

#[test]
fn commit_log_first_page_is_newest_first() {
    let tmp = init_bare_repo();
    commit_n_times(tmp.path(), 3);

    let runner = runner_for(&tmp);
    let page = runner.commit_log(10, 0).unwrap();
    assert_eq!(page.len(), 3);
    // Newest first: "commit 2" was committed last.
    assert_eq!(page[0].subject, "commit 2");
    assert_eq!(page[1].subject, "commit 1");
    assert_eq!(page[2].subject, "commit 0");
    // Author name flows through.
    assert_eq!(page[0].author_name, "redquill test");
    // short_sha is a prefix of the full sha.
    assert!(page[0].sha.starts_with(&page[0].short_sha));
}

#[test]
fn commit_log_pagination_yields_two_non_overlapping_pages_in_stable_order() {
    let tmp = init_bare_repo();
    commit_n_times(tmp.path(), 5);

    let runner = runner_for(&tmp);
    let page1 = runner.commit_log(2, 0).unwrap();
    let page2 = runner.commit_log(2, 2).unwrap();

    assert_eq!(page1.len(), 2);
    assert_eq!(page2.len(), 2);
    assert_eq!(page1[0].subject, "commit 4");
    assert_eq!(page1[1].subject, "commit 3");
    assert_eq!(page2[0].subject, "commit 2");
    assert_eq!(page2[1].subject, "commit 1");

    // No overlap between pages.
    let page1_shas: Vec<&str> = page1.iter().map(|c| c.sha.as_str()).collect();
    assert!(page2.iter().all(|c| !page1_shas.contains(&c.sha.as_str())));

    // Requesting the same pages again is stable (no history changed).
    let page1_again = runner.commit_log(2, 0).unwrap();
    assert_eq!(page1, page1_again);
}

#[test]
fn commit_log_skip_past_the_end_yields_an_empty_final_page() {
    let tmp = init_bare_repo();
    commit_n_times(tmp.path(), 2);

    let runner = runner_for(&tmp);
    let last_page = runner.commit_log(10, 2).unwrap();
    assert!(last_page.is_empty());
}

// -- `GitRunner::commit_log_range` (the Review launcher Commits tab's
// ahead-of-base source) --------------------------------------------------

#[test]
fn commit_log_range_lists_commits_ahead_of_base_newest_first() {
    let tmp = init_repo_with_base("main");
    let dir = tmp.path();
    commit_file(dir, "base\n", "base commit");
    git(dir, &["checkout", "-qb", "feature"]);
    commit_file(dir, "feature one\n", "feature commit one");
    commit_file(dir, "feature two\n", "feature commit two");

    let runner = runner_for(&tmp);
    let range = CommitLogRange {
        base: "main".to_string(),
        head: "feature".to_string(),
    };
    let commits = runner.commit_log_range(&range).unwrap();

    assert_eq!(commits.len(), 2);
    assert_eq!(commits[0].subject, "feature commit two");
    assert_eq!(commits[1].subject, "feature commit one");
    // The base commit itself is excluded.
    assert!(commits.iter().all(|c| c.subject != "base commit"));
}

#[test]
fn commit_log_range_on_the_base_branch_itself_yields_no_entries() {
    let tmp = init_repo_with_base("main");
    let dir = tmp.path();
    commit_file(dir, "base\n", "base commit");

    let runner = runner_for(&tmp);
    let range = CommitLogRange {
        base: "main".to_string(),
        head: "main".to_string(),
    };
    let commits = runner.commit_log_range(&range).unwrap();
    assert!(
        commits.is_empty(),
        "a branch that IS the base has nothing ahead of itself"
    );
}

#[test]
fn commit_log_range_with_head_already_reachable_from_base_yields_no_entries() {
    // `head` (an older tag) contributes nothing `base` doesn't already
    // have — still an empty vec, not an error.
    let tmp = init_repo_with_base("main");
    let dir = tmp.path();
    commit_file(dir, "one\n", "first");
    git(dir, &["tag", "older"]);
    commit_file(dir, "two\n", "second");

    let runner = runner_for(&tmp);
    let range = CommitLogRange {
        base: "main".to_string(),
        head: "older".to_string(),
    };
    let commits = runner.commit_log_range(&range).unwrap();
    assert!(commits.is_empty());
}

#[test]
fn commit_log_range_with_an_unresolvable_base_is_an_error() {
    let tmp = init_repo_with_base("main");
    let dir = tmp.path();
    commit_file(dir, "one\n", "first");

    let runner = runner_for(&tmp);
    let range = CommitLogRange {
        base: "does-not-exist".to_string(),
        head: "main".to_string(),
    };
    assert!(runner.commit_log_range(&range).is_err());
}
