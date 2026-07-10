//! Integration tests for the `git` module.
//!
//! Each test builds a throwaway repository in a fresh tempdir, configures
//! git identity LOCALLY (never touching the host repo or global config), and
//! then asserts against the module's parsed output. All git invocations run
//! with the tempdir as their working directory.

use std::fs;
use std::path::Path;
use std::process::Command;

use redquill::git::{ChangeKind, DiffTarget, GitRunner, StatusCode};
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

fn runner_for(tmp: &TempDir) -> GitRunner {
    GitRunner::discover_in(tmp.path()).expect("discover repo")
}

#[test]
fn root_is_the_repo_toplevel() {
    let tmp = init_repo();
    let runner = runner_for(&tmp);
    // Canonicalize both sides: macOS tempdirs live under a symlinked /var.
    let expected = fs::canonicalize(tmp.path()).unwrap();
    let actual = fs::canonicalize(runner.root()).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn not_a_repo_errors() {
    let tmp = TempDir::new().unwrap();
    assert!(GitRunner::discover_in(tmp.path()).is_err());
}

#[test]
fn working_tree_modification_is_a_patch() {
    let tmp = init_repo();
    write(tmp.path(), "base.txt", b"line one\nchanged two\n");
    let runner = runner_for(&tmp);

    let patches = runner.diff(&DiffTarget::WorkingTree).unwrap();
    assert_eq!(patches.len(), 1);
    assert_eq!(patches[0].path, "base.txt");
    assert!(!patches[0].is_binary);
    assert!(patches[0].raw.contains("+changed two"));

    // The status parser sees an unstaged modification.
    let status = runner.status().unwrap();
    let entry = status.iter().find(|s| s.path == "base.txt").unwrap();
    assert_eq!(entry.unstaged, StatusCode::Modified);
    assert!(entry.has_unstaged_changes());
    assert!(!entry.has_staged_changes());
}

#[test]
fn staged_vs_unstaged_are_distinguished() {
    let tmp = init_repo();
    let dir = tmp.path();
    // One file staged, another modified but unstaged.
    write(dir, "staged.txt", b"new content\n");
    git(dir, &["add", "staged.txt"]);
    write(dir, "base.txt", b"line one\nunstaged edit\n");

    let runner = runner_for(&tmp);

    let staged = runner.diff(&DiffTarget::Staged).unwrap();
    assert_eq!(staged.len(), 1);
    assert_eq!(staged[0].path, "staged.txt");

    let working = runner.diff(&DiffTarget::WorkingTree).unwrap();
    assert_eq!(working.len(), 1);
    assert_eq!(working[0].path, "base.txt");

    let status = runner.status().unwrap();
    let staged_entry = status.iter().find(|s| s.path == "staged.txt").unwrap();
    assert!(staged_entry.has_staged_changes());
    let base_entry = status.iter().find(|s| s.path == "base.txt").unwrap();
    assert!(base_entry.has_unstaged_changes());
}

#[test]
fn added_file_shows_up_when_staged() {
    let tmp = init_repo();
    let dir = tmp.path();
    write(dir, "added.txt", b"brand new\n");
    git(dir, &["add", "added.txt"]);

    let runner = runner_for(&tmp);
    let patches = runner.diff(&DiffTarget::Staged).unwrap();
    let added = patches.iter().find(|p| p.path == "added.txt").unwrap();
    assert!(added.raw.contains("new file mode"));
    assert_eq!(added.old_path, None);

    let status = runner.status().unwrap();
    let entry = status.iter().find(|s| s.path == "added.txt").unwrap();
    assert_eq!(entry.staged, StatusCode::Added);
}

#[test]
fn deleted_file_is_reported() {
    let tmp = init_repo();
    let dir = tmp.path();
    fs::remove_file(dir.join("base.txt")).unwrap();

    let runner = runner_for(&tmp);
    let patches = runner.diff(&DiffTarget::WorkingTree).unwrap();
    let del = patches.iter().find(|p| p.path == "base.txt").unwrap();
    assert!(del.raw.contains("deleted file mode"));
    assert_eq!(del.old_path, None);

    let status = runner.status().unwrap();
    let entry = status.iter().find(|s| s.path == "base.txt").unwrap();
    assert_eq!(entry.unstaged, StatusCode::Deleted);
}

#[test]
fn untracked_file_is_status_only() {
    let tmp = init_repo();
    write(tmp.path(), "untracked.txt", b"nobody added me\n");

    let runner = runner_for(&tmp);
    // `git diff` never surfaces untracked content.
    let patches = runner.diff(&DiffTarget::WorkingTree).unwrap();
    assert!(patches.iter().all(|p| p.path != "untracked.txt"));

    let status = runner.status().unwrap();
    let entry = status.iter().find(|s| s.path == "untracked.txt").unwrap();
    assert_eq!(entry.kind, ChangeKind::Untracked);
    assert!(entry.is_untracked());
}

#[test]
fn renamed_file_carries_old_and_new_path() {
    let tmp = init_repo();
    let dir = tmp.path();
    // Stage a rename so it is detected as a rename (needs -M, which diff uses).
    git(dir, &["mv", "base.txt", "renamed.txt"]);

    let runner = runner_for(&tmp);
    let patches = runner.diff(&DiffTarget::Staged).unwrap();
    let rename = patches.iter().find(|p| p.path == "renamed.txt").unwrap();
    assert_eq!(rename.old_path.as_deref(), Some("base.txt"));

    let status = runner.status().unwrap();
    let entry = status.iter().find(|s| s.path == "renamed.txt").unwrap();
    assert_eq!(entry.kind, ChangeKind::RenamedOrCopied);
    assert_eq!(entry.orig_path.as_deref(), Some("base.txt"));
}

#[test]
fn binary_file_is_flagged_not_parsed() {
    let tmp = init_repo();
    let dir = tmp.path();
    // A NUL byte makes git treat the file as binary.
    write(dir, "blob.bin", &[0u8, 1, 2, 3, 0, 255, 10, 7]);
    git(dir, &["add", "blob.bin"]);

    let runner = runner_for(&tmp);
    let patches = runner.diff(&DiffTarget::Staged).unwrap();
    let bin = patches.iter().find(|p| p.path == "blob.bin").unwrap();
    assert!(bin.is_binary);
}

#[test]
fn empty_diff_yields_no_patches() {
    let tmp = init_repo();
    let runner = runner_for(&tmp);
    // Clean working tree: nothing changed since the initial commit.
    assert!(runner.diff(&DiffTarget::WorkingTree).unwrap().is_empty());
    assert!(runner.status().unwrap().is_empty());
}

#[test]
fn range_diff_between_commits() {
    let tmp = init_repo();
    let dir = tmp.path();
    write(dir, "base.txt", b"line one\nsecond commit\n");
    git(dir, &["commit", "-aqm", "second"]);

    let runner = runner_for(&tmp);
    let patches = runner
        .diff(&DiffTarget::Range("HEAD~1..HEAD".to_string()))
        .unwrap();
    assert_eq!(patches.len(), 1);
    assert_eq!(patches[0].path, "base.txt");
    assert!(patches[0].raw.contains("+second commit"));
}
