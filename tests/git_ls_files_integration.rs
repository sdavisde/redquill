//! Integration test for `GitRunner::ls_files`/`ls_files_untracked` (spec 06
//! Unit 1's candidate source), against a real throwaway repository — never
//! the host repo. Mirrors `tests/git_integration.rs`'s tempdir/fixture
//! conventions.

use std::fs;
use std::path::Path;
use std::process::Command;

use redquill::git::GitRunner;
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

fn write(dir: &Path, rel: &str, contents: &[u8]) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

/// A repo with a tracked file, an untracked-but-unignored file, and an
/// ignored file (via `.gitignore`) — enough to distinguish all three
/// `git ls-files` outcomes.
fn repo_with_tracked_untracked_and_ignored() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.name", "redquill test"]);
    git(dir, &["config", "user.email", "test@redquill.invalid"]);
    write(dir, ".gitignore", b"ignored.txt\n");
    write(dir, "tracked.rs", b"fn main() {}\n");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "initial"]);
    write(dir, "untracked.rs", b"// scratch\n");
    write(dir, "ignored.txt", b"should never appear\n");
    tmp
}

#[test]
fn ls_files_lists_only_tracked_paths() {
    let tmp = repo_with_tracked_untracked_and_ignored();
    let runner = GitRunner::discover_in(tmp.path()).expect("discover repo");
    let mut tracked = runner.ls_files().expect("ls_files");
    tracked.sort();
    assert_eq!(
        tracked,
        vec![".gitignore".to_string(), "tracked.rs".to_string()]
    );
}

#[test]
fn ls_files_untracked_excludes_ignored_paths() {
    let tmp = repo_with_tracked_untracked_and_ignored();
    let runner = GitRunner::discover_in(tmp.path()).expect("discover repo");
    let untracked = runner.ls_files_untracked().expect("ls_files_untracked");
    assert_eq!(untracked, vec!["untracked.rs".to_string()]);
}

#[test]
fn combined_tracked_and_untracked_omit_ignored_files() {
    let tmp = repo_with_tracked_untracked_and_ignored();
    let runner = GitRunner::discover_in(tmp.path()).expect("discover repo");
    let tracked = runner.ls_files().expect("ls_files");
    let untracked = runner.ls_files_untracked().expect("ls_files_untracked");
    let mut all: Vec<String> = tracked.into_iter().chain(untracked).collect();
    all.sort();
    assert_eq!(
        all,
        vec![
            ".gitignore".to_string(),
            "tracked.rs".to_string(),
            "untracked.rs".to_string(),
        ]
    );
    assert!(!all.iter().any(|p| p == "ignored.txt"));
}
