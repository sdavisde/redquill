//! Integration tests for `GitRunner::commit_log` (the commit-log read
//! model's pagination), built against throwaway tempdir repos — never the
//! host repo or its config.

use std::fs;
use std::path::Path;
use std::process::Command;

use redquill::git::GitRunner;
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

/// Initializes a fresh repo with a local-only identity (nothing leaks to
/// the host's global git config) and no commits yet.
fn init_bare_repo() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.name", "redquill test"]);
    git(dir, &["config", "user.email", "test@redquill.invalid"]);
    tmp
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
