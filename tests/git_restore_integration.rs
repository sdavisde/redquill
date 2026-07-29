//! Integration tests for the restore plumbing (`src/git/restore.rs`).
//!
//! Each test builds a throwaway repository in a fresh tempdir and configures
//! git identity LOCALLY (never touching the host repo or global config).
//! Every path these tests touch is asserted to live inside the tempdir before
//! anything destructive runs — these are the only operations in the codebase
//! that delete a user's work, so the blast-radius guard is part of the test,
//! not an assumption about the harness.

use std::fs;
use std::path::Path;
use std::process::Command;

use redquill::git::{GitRunner, RestoreScope};
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

fn write(dir: &Path, rel: &str, contents: &str) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn read(dir: &Path, rel: &str) -> String {
    fs::read_to_string(dir.join(rel)).unwrap()
}

/// Asserts the runner's repo root really is inside `tmp`, canonicalizing both
/// sides (macOS resolves `/var` to `/private/var`). Guards every destructive
/// test below against escaping into the host repo.
fn assert_runner_is_sandboxed(runner: &GitRunner, tmp: &TempDir) {
    let root = runner.root().canonicalize().unwrap();
    let sandbox = tmp.path().canonicalize().unwrap();
    assert!(
        root.starts_with(&sandbox),
        "runner root {root:?} escaped the tempdir {sandbox:?}"
    );
}

/// A repo with one committed file, `base.txt`.
fn init_repo() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.name", "redquill test"]);
    git(dir, &["config", "user.email", "test@redquill.invalid"]);
    write(dir, "base.txt", "line one\nline two\n");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "initial"]);
    tmp
}

fn runner_for(tmp: &TempDir) -> GitRunner {
    let runner = GitRunner::discover_in(tmp.path()).expect("discover repo");
    assert_runner_is_sandboxed(&runner, tmp);
    runner
}

fn is_clean(dir: &Path) -> bool {
    git_out(dir, &["status", "--porcelain"]).is_empty()
}

#[test]
fn restore_file_discards_an_unstaged_edit() {
    let tmp = init_repo();
    let dir = tmp.path();
    let runner = runner_for(&tmp);

    write(dir, "base.txt", "clobbered\n");
    assert!(!is_clean(dir));

    runner
        .restore_paths(&["base.txt"], RestoreScope::IndexAndWorktree)
        .unwrap();

    assert_eq!(read(dir, "base.txt"), "line one\nline two\n");
    assert!(is_clean(dir));
}

#[test]
fn restore_file_discards_a_fully_staged_edit() {
    let tmp = init_repo();
    let dir = tmp.path();
    let runner = runner_for(&tmp);

    write(dir, "base.txt", "staged change\n");
    git(dir, &["add", "base.txt"]);
    assert!(!git_out(dir, &["diff", "--cached"]).is_empty());

    runner
        .restore_paths(&["base.txt"], RestoreScope::IndexAndWorktree)
        .unwrap();

    assert_eq!(read(dir, "base.txt"), "line one\nline two\n");
    assert!(is_clean(dir));
}

#[test]
fn restore_file_discards_both_halves_of_a_partially_staged_file() {
    let tmp = init_repo();
    let dir = tmp.path();
    let runner = runner_for(&tmp);

    // Stage one edit, then make a second, different edit on top of it — the
    // file now differs from HEAD in the index *and* differs from the index in
    // the working tree.
    write(dir, "base.txt", "staged half\nline two\n");
    git(dir, &["add", "base.txt"]);
    write(dir, "base.txt", "staged half\nunstaged half\n");
    assert!(!git_out(dir, &["diff", "--cached"]).is_empty());
    assert!(!git_out(dir, &["diff"]).is_empty());

    runner
        .restore_paths(&["base.txt"], RestoreScope::IndexAndWorktree)
        .unwrap();

    assert_eq!(read(dir, "base.txt"), "line one\nline two\n");
    assert!(is_clean(dir));
}

#[test]
fn restore_file_brings_back_a_deleted_file() {
    let tmp = init_repo();
    let dir = tmp.path();
    let runner = runner_for(&tmp);

    fs::remove_file(dir.join("base.txt")).unwrap();
    git(dir, &["add", "-A", "base.txt"]);

    runner
        .restore_paths(&["base.txt"], RestoreScope::IndexAndWorktree)
        .unwrap();

    assert_eq!(read(dir, "base.txt"), "line one\nline two\n");
    assert!(is_clean(dir));
}

#[test]
fn restore_file_leaves_other_files_alone() {
    let tmp = init_repo();
    let dir = tmp.path();
    write(dir, "other.txt", "committed other\n");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "second"]);
    let runner = runner_for(&tmp);

    write(dir, "base.txt", "clobber base\n");
    write(dir, "other.txt", "clobber other\n");

    runner
        .restore_paths(&["base.txt"], RestoreScope::IndexAndWorktree)
        .unwrap();

    assert_eq!(read(dir, "base.txt"), "line one\nline two\n");
    // The untouched file keeps its dirty content.
    assert_eq!(read(dir, "other.txt"), "clobber other\n");
}

#[test]
fn restore_file_errors_on_an_untracked_path() {
    let tmp = init_repo();
    let dir = tmp.path();
    let runner = runner_for(&tmp);

    write(dir, "fresh.txt", "never committed\n");

    let err = runner
        .restore_paths(&["fresh.txt"], RestoreScope::IndexAndWorktree)
        .unwrap_err();
    assert!(matches!(err, redquill::git::GitError::Command { .. }));
    // The file survives a failed restore — errors never half-delete.
    assert_eq!(read(dir, "fresh.txt"), "never committed\n");
}

#[test]
fn discard_untracked_file_removes_it() {
    let tmp = init_repo();
    let dir = tmp.path();
    let runner = runner_for(&tmp);

    write(dir, "fresh.txt", "never committed\n");
    assert!(dir.join("fresh.txt").exists());

    runner.discard_untracked_file("fresh.txt").unwrap();

    assert!(!dir.join("fresh.txt").exists());
    assert!(is_clean(dir));
}

#[test]
fn discard_untracked_file_removes_a_nested_path_without_its_directory() {
    let tmp = init_repo();
    let dir = tmp.path();
    let runner = runner_for(&tmp);

    write(dir, "nested/deep/fresh.txt", "never committed\n");

    runner
        .discard_untracked_file("nested/deep/fresh.txt")
        .unwrap();

    assert!(!dir.join("nested/deep/fresh.txt").exists());
    // Only the file goes; the directory is not the caller's target.
    assert!(dir.join("nested/deep").exists());
}

#[test]
fn discard_untracked_file_errors_when_the_path_is_already_gone() {
    let tmp = init_repo();
    let runner = runner_for(&tmp);

    // Reaching here means the caller's picture of the tree was wrong. A
    // silent success would tell the reviewer a file was deleted when nothing
    // happened — the staged-deletion misclassification did exactly that.
    assert!(runner.discard_untracked_file("never-existed.txt").is_err());
}

#[test]
fn discard_untracked_file_refuses_a_directory() {
    let tmp = init_repo();
    let dir = tmp.path();
    let runner = runner_for(&tmp);

    write(dir, "subdir/file.txt", "content\n");

    // `remove_file` on a directory fails rather than recursing — the guard
    // that keeps a mis-resolved path from taking a whole tree with it.
    assert!(runner.discard_untracked_file("subdir").is_err());
    assert!(dir.join("subdir/file.txt").exists());
}

// -- Renames ---------------------------------------------------------------

/// A staged rename restored by its new path *alone* leaves a file at neither
/// path: `HEAD` has no entry for the new name, so git drops it, while the
/// original stays staged-deleted. This pins the failure so the two-path fix
/// below can't silently regress to it.
#[test]
fn restoring_only_the_new_path_of_a_rename_loses_the_file_entirely() {
    let tmp = init_repo();
    let dir = tmp.path();
    let runner = runner_for(&tmp);

    git(dir, &["mv", "base.txt", "renamed.txt"]);

    runner
        .restore_paths(&["renamed.txt"], RestoreScope::IndexAndWorktree)
        .unwrap();

    assert!(!dir.join("renamed.txt").exists());
    assert!(!dir.join("base.txt").exists(), "the original is gone too");
}

#[test]
fn restoring_both_paths_of_a_rename_undoes_it_completely() {
    let tmp = init_repo();
    let dir = tmp.path();
    let runner = runner_for(&tmp);

    git(dir, &["mv", "base.txt", "renamed.txt"]);

    runner
        .restore_paths(&["renamed.txt", "base.txt"], RestoreScope::IndexAndWorktree)
        .unwrap();

    assert_eq!(read(dir, "base.txt"), "line one\nline two\n");
    assert!(!dir.join("renamed.txt").exists());
    assert!(is_clean(dir), "the rename is fully undone");
}

#[test]
fn restoring_a_rename_with_content_edits_reverts_the_content_too() {
    let tmp = init_repo();
    let dir = tmp.path();
    let runner = runner_for(&tmp);

    git(dir, &["mv", "base.txt", "renamed.txt"]);
    write(dir, "renamed.txt", "rewritten entirely\n");

    runner
        .restore_paths(&["renamed.txt", "base.txt"], RestoreScope::IndexAndWorktree)
        .unwrap();

    assert_eq!(read(dir, "base.txt"), "line one\nline two\n");
    assert!(is_clean(dir));
}

// -- Index-only scope (the staged view) -------------------------------------

#[test]
fn index_only_unstages_without_touching_the_working_tree() {
    let tmp = init_repo();
    let dir = tmp.path();
    let runner = runner_for(&tmp);

    write(dir, "base.txt", "staged edit\n");
    git(dir, &["add", "base.txt"]);
    write(dir, "base.txt", "staged edit\nplus an unstaged one\n");

    runner
        .restore_paths(&["base.txt"], RestoreScope::IndexOnly)
        .unwrap();

    // The index is back to HEAD...
    assert!(git_out(dir, &["diff", "--cached"]).is_empty());
    // ...and the working tree kept every byte, including the staged half,
    // which is now simply unstaged.
    assert_eq!(read(dir, "base.txt"), "staged edit\nplus an unstaged one\n");
}

#[test]
fn index_only_never_deletes_a_newly_added_file_from_disk() {
    let tmp = init_repo();
    let dir = tmp.path();
    let runner = runner_for(&tmp);

    write(dir, "added.txt", "brand new\n");
    git(dir, &["add", "added.txt"]);

    runner
        .restore_paths(&["added.txt"], RestoreScope::IndexOnly)
        .unwrap();

    // Unstaged back to untracked, but the content survives on disk.
    assert!(dir.join("added.txt").exists());
    assert_eq!(read(dir, "added.txt"), "brand new\n");
}

#[test]
fn empty_paths_is_a_no_op_rather_than_an_unbounded_restore() {
    let tmp = init_repo();
    let dir = tmp.path();
    let runner = runner_for(&tmp);

    write(dir, "base.txt", "dirty\n");

    runner
        .restore_paths(&[], RestoreScope::IndexAndWorktree)
        .unwrap();

    // Nothing was restored — an empty pathspec must never mean "everything".
    assert_eq!(read(dir, "base.txt"), "dirty\n");
}
