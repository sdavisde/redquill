//! Integration tests proving the diff model against real `git diff` output.
//!
//! Per task T4.0 / spec §8 Testing Strategy: builds a throwaway repository in
//! a fresh tempdir (mirroring `tests/git_integration.rs`'s pattern) — never
//! touching the host repo or global git config — parses the resulting
//! `RawFilePatch`es through `diff::parse_patches`, and asserts no panic plus
//! sane aggregate counts via `diff::summarize`.
//!
//! One additional test exercises the explicitly-permitted host-repo
//! exception (spec §8): a READ-ONLY diff of this repo's own committed
//! history (`git diff <sha>^ <sha>`). It never writes to the host repo.

use std::fs;
use std::path::Path;
use std::process::Command;

use redquill::diff::{self, LineKind};
use redquill::git::{DiffTarget, GitRunner};
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
fn write(dir: &Path, rel: &str, contents: &str) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

/// Initializes a fresh repo with a single committed file, returning the
/// tempdir. Identity is set LOCALLY so nothing leaks to the host's global
/// config and commits succeed even in a CI environment with no git identity
/// configured.
fn init_repo() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.name", "redquill test"]);
    git(dir, &["config", "user.email", "test@redquill.invalid"]);
    write(dir, "base.rs", "fn main() {\n    let x = 1;\n}\n");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "initial"]);
    tmp
}

/// FR-diff-wire-2: a real diff produced by a throwaway tempdir repo parses
/// via `diff::parse_patches` without panicking, and `summarize` reports
/// nonzero files and hunks.
#[test]
fn real_diff_from_tempdir_repo_parses_without_panicking() {
    let tmp = init_repo();
    let dir = tmp.path();
    write(
        dir,
        "base.rs",
        "fn main() {\n    let x = 2;\n    let y = 3;\n}\n",
    );
    write(dir, "added.rs", "pub fn helper() {}\n");
    git(dir, &["add", "added.rs"]);

    let runner = GitRunner::discover_in(dir).expect("discover repo");
    let patches = runner
        .diff(&DiffTarget::WorkingTree)
        .expect("git diff working tree");

    let files = diff::parse_patches(&patches);
    let summary = diff::summarize(&files);

    assert!(summary.files > 0, "expected at least one parsed file");
    assert!(summary.hunks > 0, "expected at least one parsed hunk");
    // The modification to base.rs must contribute at least one added line.
    assert!(summary.added > 0, "expected at least one added line");

    // Sanity: every file's hunks actually contain lines with the expected
    // line-kind vocabulary (no silent empty/garbage parse).
    for file in &files {
        for hunk in &file.hunks {
            for line in &hunk.lines {
                assert!(matches!(
                    line.kind,
                    LineKind::Context | LineKind::Added | LineKind::Removed
                ));
            }
        }
    }
}

/// FR-diff-wire-2: after a committed edit, a further working-tree edit still
/// parses cleanly and reports the expected file/hunk/added counts (guards
/// against state leaking between successive commits/parses).
#[test]
fn subsequent_edit_after_a_commit_still_parses_and_updates_counts() {
    let tmp = init_repo();
    let dir = tmp.path();
    // A committed edit, then a further uncommitted edit on top of it.
    write(dir, "base.rs", "fn main() {\n    let x = 1;\n}\n// v2\n");
    git(dir, &["commit", "-aqm", "second commit"]);
    write(
        dir,
        "base.rs",
        "fn main() {\n    let x = 1;\n}\n// v2\n\nfn extra() {}\n",
    );

    let runner = GitRunner::discover_in(dir).expect("discover repo");
    let patches = runner
        .diff(&DiffTarget::WorkingTree)
        .expect("git diff working tree");
    let files = diff::parse_patches(&patches);
    let summary = diff::summarize(&files);

    assert_eq!(summary.files, 1);
    assert!(summary.hunks > 0);
    assert!(summary.added > 0);
}

/// The permitted host-repo exception (spec §8): a READ-ONLY diff over this
/// repo's own committed history (`git diff <sha>^ <sha>`) for an early,
/// known-nontrivial commit. Never mutates the host repo.
#[test]
fn real_diff_from_this_repos_own_history_parses_without_panicking() {
    let runner = GitRunner::discover().expect("discover host repo");
    // 0faf0d5: "feat: scaffold module layout and CLI parsing" — the repo's
    // second commit (the first, f29a9f7, is a parentless root commit and
    // has no `^` to diff against). Guaranteed to exist and add file content.
    let patches = runner
        .diff(&DiffTarget::Range("0faf0d5^..0faf0d5".to_string()))
        .expect("git diff over known commit range");

    let files = diff::parse_patches(&patches);
    let summary = diff::summarize(&files);

    assert!(summary.files > 0, "expected at least one parsed file");
    assert!(summary.hunks > 0, "expected at least one parsed hunk");
}
