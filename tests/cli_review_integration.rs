//! End-to-end CLI integration tests for `redquill --review <branch>`: the
//! actual compiled binary is spawned via
//! `env!("CARGO_BIN_EXE_redquill")` against throwaway repositories built in
//! fresh tempdirs, exercising real CLI parsing, real `git` subprocesses, and
//! the headless plain-text summary path (`main.rs` falls back to it
//! whenever stderr isn't a terminal — exactly the case here, since
//! `Command::output()` pipes both streams).
//!
//! Every fixture is built with `tempfile`, paths are canonicalized (macOS
//! `/tmp` symlinks to `/private/tmp`), and every mutating git call this file
//! makes is preceded by `assert_inside_tempdir` — the shared isolation
//! guard this task introduces (also used by `tests/git_review_integration.rs`)
//! per the 2026-07-16 incident notes in the task file: a worktree-shaped
//! test previously escaped its tempdir and mutated a real repository. This
//! file never touches the host repo.

use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

// -- Fixture helpers (mirrors tests/git_review_integration.rs) -------------

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

fn canon(path: &Path) -> std::path::PathBuf {
    fs::canonicalize(path).unwrap_or_else(|e| panic!("canonicalize {path:?}: {e}"))
}

/// Asserts `path` is lexically inside `tmp`'s canonicalized root, walking up
/// to the nearest existing ancestor first (the path itself may not exist
/// yet, e.g. a worktree destination about to be created). The shared
/// isolation guard every mutating git call in this file runs before
/// touching disk.
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
        "refusing to run a mutating git call outside the tempdir: {path:?} (resolved ancestor {probe_canon:?}) is not under {tmp_root:?}"
    );
}

/// A repo on `main` with one commit, plus a `feature` branch one commit
/// ahead that changes `base.txt` — main stays checked out (the "user's own
/// worktree"), so every test starts from a clean, on-`main` checkout.
fn repo_with_feature_branch() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    assert_inside_tempdir(dir, &tmp);
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.name", "redquill test"]);
    git(dir, &["config", "user.email", "test@redquill.invalid"]);
    write(dir, "base.txt", b"line one\n");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "initial"]);

    git(dir, &["branch", "feature"]);
    git(dir, &["checkout", "-q", "feature"]);
    write(dir, "base.txt", b"line one\nfeature line\n");
    git(dir, &["commit", "-aqm", "feature change"]);
    git(dir, &["checkout", "-q", "main"]);

    tmp
}

/// Runs the actual compiled `redquill` binary with `args` at `dir`,
/// returning its output. Stdout/stderr are piped (not a tty), so `main.rs`
/// takes the headless plain-text summary path rather than launching the TUI.
fn run_redquill(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_redquill"))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("failed to spawn the redquill binary")
}

fn stdout_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

// -- Happy path -------------------------------------------------------------

#[test]
fn review_opens_the_three_dot_diff_and_leaves_the_original_checkout_untouched() {
    let repo = repo_with_feature_branch();

    let output = run_redquill(repo.path(), &["--review", "feature"]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&output),
        stderr_of(&output)
    );
    assert!(
        stdout_of(&output).contains("base.txt"),
        "expected the changed file in the summary, got: {}",
        stdout_of(&output)
    );

    // `git worktree list` (run in the user's own checkout) shows the new
    // managed worktree, checked out to `feature`.
    let worktrees = git_out(repo.path(), &["worktree", "list", "--porcelain"]);
    assert!(worktrees.contains("branch refs/heads/feature"));
    assert!(
        worktrees.contains(".git/redquill/worktrees/") || worktrees.contains("redquill/worktrees")
    );

    // The user's own checkout is byte-for-byte untouched: still on `main`,
    // clean.
    assert_eq!(git_out(repo.path(), &["branch", "--show-current"]), "main");
    assert_eq!(git_out(repo.path(), &["status", "--porcelain"]), "");
}

#[test]
fn relaunching_review_reuses_the_existing_worktree_rather_than_duplicating_it() {
    let repo = repo_with_feature_branch();

    let first = run_redquill(repo.path(), &["--review", "feature"]);
    assert!(
        first.status.success(),
        "first launch: {}",
        stderr_of(&first)
    );
    let worktrees_after_first = git_out(repo.path(), &["worktree", "list", "--porcelain"]);
    let feature_entries_first = worktrees_after_first
        .matches("branch refs/heads/feature")
        .count();
    assert_eq!(feature_entries_first, 1);

    let second = run_redquill(repo.path(), &["--review", "feature"]);
    assert!(
        second.status.success(),
        "second launch: {}",
        stderr_of(&second)
    );
    let worktrees_after_second = git_out(repo.path(), &["worktree", "list", "--porcelain"]);
    let feature_entries_second = worktrees_after_second
        .matches("branch refs/heads/feature")
        .count();
    assert_eq!(
        feature_entries_second, 1,
        "relaunch must reuse the worktree, not create a second one"
    );

    // Still untouched.
    assert_eq!(git_out(repo.path(), &["branch", "--show-current"]), "main");
    assert_eq!(git_out(repo.path(), &["status", "--porcelain"]), "");
}

// -- Failure paths: readable error, zero side effects ------------------------

#[test]
fn unknown_branch_fails_readably_with_no_side_effects() {
    let repo = repo_with_feature_branch();
    let worktrees_before = git_out(repo.path(), &["worktree", "list", "--porcelain"]);

    let output = run_redquill(repo.path(), &["--review", "no-such-branch"]);
    assert!(!output.status.success(), "expected a nonzero exit");
    assert!(
        !stderr_of(&output).trim().is_empty(),
        "expected a readable error message on stderr"
    );

    let worktrees_after = git_out(repo.path(), &["worktree", "list", "--porcelain"]);
    assert_eq!(
        worktrees_before, worktrees_after,
        "a failed review must create no worktree"
    );
    assert_eq!(git_out(repo.path(), &["branch", "--show-current"]), "main");
    assert_eq!(git_out(repo.path(), &["status", "--porcelain"]), "");
}

#[test]
fn branch_already_checked_out_fails_readably_with_no_side_effects() {
    let repo = repo_with_feature_branch();
    // `feature` is checked out right here, in the user's own (primary)
    // worktree — git refuses `worktree add` for a branch already in use by
    // any worktree, including the primary one.
    git(repo.path(), &["checkout", "-q", "feature"]);
    let worktrees_before = git_out(repo.path(), &["worktree", "list", "--porcelain"]);

    let output = run_redquill(repo.path(), &["--review", "feature"]);
    assert!(!output.status.success(), "expected a nonzero exit");
    assert!(
        !stderr_of(&output).trim().is_empty(),
        "expected a readable error message on stderr"
    );

    let worktrees_after = git_out(repo.path(), &["worktree", "list", "--porcelain"]);
    assert_eq!(
        worktrees_before, worktrees_after,
        "a failed review must create no worktree"
    );
    assert_eq!(
        git_out(repo.path(), &["branch", "--show-current"]),
        "feature"
    );
    assert_eq!(git_out(repo.path(), &["status", "--porcelain"]), "");
}
