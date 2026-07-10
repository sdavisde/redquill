//! Integration tests for index staging plumbing (`src/git/stage.rs`).
//!
//! Each test builds a throwaway repository in a fresh tempdir, configures
//! git identity LOCALLY (never touching the host repo or global config).
//! Every test that performs a staging operation also asserts the file's
//! on-disk working-tree content is unchanged afterward — these functions
//! must only ever touch the index.

use std::fs;
use std::path::Path;
use std::process::Command;

use redquill::git::{DiffTarget, GitRunner, build_hunk_patch, build_line_patch};
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

/// Runs a git command in `dir` and returns trimmed stdout.
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

/// Writes a file (creating parent dirs) inside the repo.
fn write(dir: &Path, rel: &str, contents: &[u8]) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

/// Reads a file's on-disk contents.
fn read(dir: &Path, rel: &str) -> Vec<u8> {
    fs::read(dir.join(rel)).unwrap()
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

/// Initializes a fresh repo with NO commits at all.
fn init_repo_no_commits() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.name", "redquill test"]);
    git(dir, &["config", "user.email", "test@redquill.invalid"]);
    tmp
}

fn runner_for(tmp: &TempDir) -> GitRunner {
    GitRunner::discover_in(tmp.path()).expect("discover repo")
}

fn staged_diff_is_empty(dir: &Path) -> bool {
    git_out(dir, &["diff", "--cached"]).is_empty()
}

fn unstaged_diff_is_empty(dir: &Path) -> bool {
    git_out(dir, &["diff"]).is_empty()
}

#[test]
fn stage_file_stages_a_modification_entirely() {
    let tmp = init_repo();
    let dir = tmp.path();
    let before = read(dir, "base.txt");
    write(dir, "base.txt", b"line one\nchanged two\n");
    let after_working_edit = read(dir, "base.txt");

    let runner = runner_for(&tmp);
    runner.stage_file("base.txt").unwrap();

    assert!(!staged_diff_is_empty(dir), "expected staged changes");
    assert!(
        unstaged_diff_is_empty(dir),
        "everything should now be staged, nothing left unstaged"
    );
    // Working tree content is untouched by staging.
    assert_eq!(read(dir, "base.txt"), after_working_edit);
    assert_ne!(before, after_working_edit);
}

#[test]
fn stage_file_stages_a_deletion() {
    let tmp = init_repo();
    let dir = tmp.path();
    fs::remove_file(dir.join("base.txt")).unwrap();

    let runner = runner_for(&tmp);
    runner.stage_file("base.txt").unwrap();

    let staged = git_out(dir, &["diff", "--cached", "--name-status"]);
    assert!(staged.starts_with('D'), "expected a staged deletion");
    assert!(!dir.join("base.txt").exists());
}

#[test]
fn stage_file_stages_an_untracked_file() {
    let tmp = init_repo();
    let dir = tmp.path();
    write(dir, "new.txt", b"brand new\n");

    let runner = runner_for(&tmp);
    runner.stage_file("new.txt").unwrap();

    let staged = git_out(dir, &["diff", "--cached", "--name-status"]);
    assert!(staged.contains("new.txt"));
    // Untracked files never show up in `git diff` (only `--cached` after add).
    assert_eq!(read(dir, "new.txt"), b"brand new\n");
}

#[test]
fn unstage_file_reverses_staging() {
    let tmp = init_repo();
    let dir = tmp.path();
    write(dir, "base.txt", b"line one\nchanged two\n");
    let working_content = read(dir, "base.txt");

    let runner = runner_for(&tmp);
    runner.stage_file("base.txt").unwrap();
    assert!(!staged_diff_is_empty(dir));

    runner.unstage_file("base.txt").unwrap();

    assert!(staged_diff_is_empty(dir), "index should match HEAD again");
    assert!(
        !unstaged_diff_is_empty(dir),
        "the edit is back in the working tree diff"
    );
    // Working tree content was never touched by either operation.
    assert_eq!(read(dir, "base.txt"), working_content);
}

#[test]
fn unstage_file_works_with_no_commits_yet() {
    let tmp = init_repo_no_commits();
    let dir = tmp.path();
    write(dir, "new.txt", b"hello\n");
    git(dir, &["add", "new.txt"]);
    let working_content = read(dir, "new.txt");

    let runner = runner_for(&tmp);
    runner.unstage_file("new.txt").unwrap();

    // No HEAD exists, so "unstaged" means dropped from the index entirely,
    // i.e. it shows back up as untracked.
    let status = git_out(dir, &["status", "--porcelain"]);
    assert!(status.contains("?? new.txt"), "status was: {status:?}");
    assert_eq!(read(dir, "new.txt"), working_content);
}

#[test]
fn unstage_file_is_a_noop_on_an_unstaged_untracked_path_with_no_commits() {
    let tmp = init_repo_no_commits();
    let dir = tmp.path();
    write(dir, "new.txt", b"hello\n");

    let runner = runner_for(&tmp);
    // Nothing staged yet; must not error.
    runner.unstage_file("new.txt").unwrap();
    let status = git_out(dir, &["status", "--porcelain"]);
    assert!(status.contains("?? new.txt"));
}

#[test]
fn build_hunk_patch_and_apply_cached_stages_exactly_one_hunk() {
    let tmp = init_repo();
    let dir = tmp.path();
    // Ten separated lines so two edits land in distinct hunks.
    let base: String = (1..=10).map(|n| format!("line {n}\n")).collect();
    write(dir, "multi.txt", base.as_bytes());
    git(dir, &["add", "multi.txt"]);
    git(dir, &["commit", "-q", "-m", "multi base"]);

    let mut lines: Vec<String> = (1..=10).map(|n| format!("line {n}")).collect();
    lines[0] = "line 1 CHANGED".to_string();
    lines[9] = "line 10 CHANGED".to_string();
    let modified = lines.join("\n") + "\n";
    let working_content = modified.clone().into_bytes();
    write(dir, "multi.txt", &working_content);

    let runner = runner_for(&tmp);
    let patches = runner.diff(&DiffTarget::WorkingTree).unwrap();
    let file_patch = patches.iter().find(|p| p.path == "multi.txt").unwrap();

    // Confirm the fixture actually produced two hunks (default 3-line context
    // on a 10-line file with edits at both ends keeps them separate).
    let hunk_count = file_patch.raw.matches("@@ -").count();
    assert_eq!(hunk_count, 2, "fixture should produce two hunks");

    let hunk0_patch = build_hunk_patch(file_patch, 0).unwrap();
    runner.apply_cached(&hunk0_patch).unwrap();

    let staged = git_out(dir, &["diff", "--cached"]);
    assert!(staged.contains("line 1 CHANGED"));
    assert!(!staged.contains("line 10 CHANGED"));

    // The second hunk's change remains unstaged.
    let unstaged = git_out(dir, &["diff"]);
    assert!(unstaged.contains("line 10 CHANGED"));
    assert!(!unstaged.contains("line 1 CHANGED"));

    // Working tree is untouched.
    assert_eq!(read(dir, "multi.txt"), working_content);
}

#[test]
fn unapply_cached_reverses_a_staged_hunk() {
    let tmp = init_repo();
    let dir = tmp.path();
    let base: String = (1..=10).map(|n| format!("line {n}\n")).collect();
    write(dir, "multi.txt", base.as_bytes());
    git(dir, &["add", "multi.txt"]);
    git(dir, &["commit", "-q", "-m", "multi base"]);

    let mut lines: Vec<String> = (1..=10).map(|n| format!("line {n}")).collect();
    lines[0] = "line 1 CHANGED".to_string();
    let modified = lines.join("\n") + "\n";
    let working_content = modified.into_bytes();
    write(dir, "multi.txt", &working_content);

    let runner = runner_for(&tmp);
    let patches = runner.diff(&DiffTarget::WorkingTree).unwrap();
    let file_patch = patches.iter().find(|p| p.path == "multi.txt").unwrap();
    let hunk_patch = build_hunk_patch(file_patch, 0).unwrap();

    runner.apply_cached(&hunk_patch).unwrap();
    assert!(!staged_diff_is_empty(dir));

    runner.unapply_cached(&hunk_patch).unwrap();
    assert!(
        staged_diff_is_empty(dir),
        "hunk staging should be fully reversed"
    );
    // The change is still present in the working tree, now fully unstaged.
    assert!(!unstaged_diff_is_empty(dir));
    assert_eq!(read(dir, "multi.txt"), working_content);
}

#[test]
fn build_line_patch_stages_a_single_added_line_out_of_a_multi_line_hunk() {
    let tmp = init_repo();
    let dir = tmp.path();
    write(dir, "lines.txt", b"a\nb\nc\n");
    git(dir, &["add", "lines.txt"]);
    git(dir, &["commit", "-q", "-m", "lines base"]);

    // Insert two new lines adjacent to context, in one hunk.
    let working_content = b"a\nnew1\nnew2\nb\nc\n".to_vec();
    write(dir, "lines.txt", &working_content);

    let runner = runner_for(&tmp);
    let patches = runner.diff(&DiffTarget::WorkingTree).unwrap();
    let file_patch = patches.iter().find(|p| p.path == "lines.txt").unwrap();
    assert_eq!(file_patch.raw.matches("@@ -").count(), 1);

    // Body lines: 0=" a" context, 1="+new1", 2="+new2", 3=" b" context, 4=" c" context.
    let line_patch = build_line_patch(file_patch, 0, &[1]).unwrap();
    runner.apply_cached(&line_patch).unwrap();

    let staged = git_out(dir, &["diff", "--cached"]);
    assert!(staged.contains("+new1"));
    assert!(!staged.contains("+new2"));

    let unstaged = git_out(dir, &["diff"]);
    assert!(unstaged.contains("+new2"));
    assert!(!unstaged.contains("+new1"));

    // Working tree still has both inserted lines; only the index changed.
    assert_eq!(read(dir, "lines.txt"), working_content);
}

#[test]
fn staging_operations_never_modify_the_working_tree() {
    let tmp = init_repo();
    let dir = tmp.path();
    write(dir, "base.txt", b"line one\nchanged two\n");
    let working_content = read(dir, "base.txt");

    let runner = runner_for(&tmp);
    runner.stage_file("base.txt").unwrap();
    assert_eq!(read(dir, "base.txt"), working_content);
    runner.unstage_file("base.txt").unwrap();
    assert_eq!(read(dir, "base.txt"), working_content);

    let patches = runner.diff(&DiffTarget::WorkingTree).unwrap();
    let file_patch = patches.iter().find(|p| p.path == "base.txt").unwrap();
    let hunk_patch = build_hunk_patch(file_patch, 0).unwrap();
    runner.apply_cached(&hunk_patch).unwrap();
    assert_eq!(read(dir, "base.txt"), working_content);
    runner.unapply_cached(&hunk_patch).unwrap();
    assert_eq!(read(dir, "base.txt"), working_content);
}
