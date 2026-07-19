//! Integration tests for the remote operations (fetch / pull / push) against
//! a `file://` bare remote.
//!
//! Each test builds a throwaway bare "remote" plus one or more working clones
//! in fresh tempdirs, configuring git identity LOCALLY (never the host repo or
//! global config). The operations are driven through redquill's own
//! [`redquill::git::remote_command`] construction (fixed argv,
//! `GIT_TERMINAL_PROMPT=0`, never `--force`), run synchronously here — the
//! async spawn/poll plumbing is covered by the `ui` unit tests. `file://`
//! keeps everything on the local filesystem, so no network is ever touched.

use std::path::Path;
use std::process::Command;

use redquill::git::{ChangeKind, GitRunner, RemoteOp, remote_command};
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

/// Runs a git command in `dir` and returns its trimmed stdout. Fixture-only.
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
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

/// Configures a fresh working clone's identity locally, plus merge-style
/// pulls (`pull.rebase false`) so a divergent `git pull` integrates via a
/// merge — recent git otherwise refuses a divergent pull outright. redquill
/// runs a plain `git pull` and respects whatever the user configured here.
fn configure_identity(dir: &Path) {
    git(dir, &["config", "user.name", "redquill test"]);
    git(dir, &["config", "user.email", "test@redquill.invalid"]);
    git(dir, &["config", "pull.rebase", "false"]);
}

/// Runs one of redquill's remote operations (through its own fixed-argv
/// construction) against `root`, returning the process output for inspection.
fn run_remote(op: RemoteOp, root: &Path) -> std::process::Output {
    remote_command(op, root)
        .output()
        .expect("failed to spawn git remote op")
}

/// A `file://` URL for a bare remote directory.
fn file_url(bare: &Path) -> String {
    format!("file://{}", bare.display())
}

/// Builds a bare "remote" and a working clone on `main` pushed to it with an
/// upstream set. Returns `(bare, repo)`.
fn setup_remote() -> (TempDir, TempDir) {
    let bare = TempDir::new().unwrap();
    // Branch must be explicit: the host's init.defaultBranch config must not
    // leak into the fixture. A bare repo's HEAD (e.g. `master` on a host
    // without init.defaultBranch set) that never has a matching branch
    // pushed to it leaves clones with a dangling HEAD and no working tree.
    git(bare.path(), &["init", "-q", "--bare", "-b", "main"]);

    let repo = TempDir::new().unwrap();
    git(repo.path(), &["init", "-q"]);
    configure_identity(repo.path());
    git(repo.path(), &["branch", "-M", "main"]);
    write(repo.path(), "base.txt", b"line one\nline two\n");
    git(repo.path(), &["add", "."]);
    git(repo.path(), &["commit", "-q", "-m", "initial"]);
    git(
        repo.path(),
        &["remote", "add", "origin", &file_url(bare.path())],
    );
    git(repo.path(), &["push", "-q", "-u", "origin", "main"]);

    (bare, repo)
}

/// Makes a second working clone of the bare remote (able to push its own
/// commits) that advances the remote out from under `repo`.
fn advance_remote(bare: &Path, contents: &[u8], message: &str) {
    let parent = TempDir::new().unwrap();
    git(parent.path(), &["clone", "-q", &file_url(bare), "clone2"]);
    let clone2 = parent.path().join("clone2");
    configure_identity(&clone2);
    write(&clone2, "base.txt", contents);
    git(&clone2, &["commit", "-aqm", message]);
    git(&clone2, &["push", "-q", "origin", "main"]);
}

#[test]
fn fetch_after_remote_movement_reveals_a_behind_count() {
    let (bare, repo) = setup_remote();
    // The remote gains a commit the local clone doesn't have yet.
    advance_remote(
        bare.path(),
        b"line one\nline two\nremote change\n",
        "remote commit",
    );

    // Before fetching, the local clone sees no divergence.
    let runner = GitRunner::discover_in(repo.path()).unwrap();
    let before = runner.status_with_branch().unwrap();
    assert_eq!(before.branch.ahead_behind, Some((0, 0)));

    // redquill's fetch updates the remote-tracking ref.
    let out = run_remote(RemoteOp::Fetch, repo.path());
    assert!(
        out.status.success(),
        "fetch failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Now the branch header shows behind 1.
    let after = runner.status_with_branch().unwrap();
    assert_eq!(after.branch.upstream.as_deref(), Some("origin/main"));
    assert_eq!(after.branch.ahead_behind, Some((0, 1)));
}

#[test]
fn push_advances_the_remote_ref_and_clears_the_ahead_count() {
    let (bare, repo) = setup_remote();
    // A local commit the remote doesn't have yet -> ahead 1.
    write(
        repo.path(),
        "base.txt",
        b"line one\nline two\nlocal change\n",
    );
    git(repo.path(), &["commit", "-aqm", "local commit"]);
    let local_head = git_out(repo.path(), &["rev-parse", "HEAD"]);

    let runner = GitRunner::discover_in(repo.path()).unwrap();
    assert_eq!(
        runner.status_with_branch().unwrap().branch.ahead_behind,
        Some((1, 0))
    );

    let out = run_remote(RemoteOp::Push, repo.path());
    assert!(
        out.status.success(),
        "push failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The bare remote's main now points at the pushed commit...
    let remote_head = git_out(bare.path(), &["rev-parse", "main"]);
    assert_eq!(remote_head, local_head);
    // ...and the ahead count is cleared (push moved origin/main forward too).
    assert_eq!(
        runner.status_with_branch().unwrap().branch.ahead_behind,
        Some((0, 0))
    );
}

#[test]
fn fast_forward_pull_integrates_the_remote_commit() {
    let (bare, repo) = setup_remote();
    advance_remote(
        bare.path(),
        b"line one\nline two\nremote change\n",
        "remote commit",
    );
    let remote_head = git_out(bare.path(), &["rev-parse", "main"]);

    let out = run_remote(RemoteOp::Pull, repo.path());
    assert!(
        out.status.success(),
        "pull failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The local HEAD fast-forwarded to the remote commit and the working
    // tree file now carries the remote content.
    let local_head = git_out(repo.path(), &["rev-parse", "HEAD"]);
    assert_eq!(local_head, remote_head);
    let contents = std::fs::read_to_string(repo.path().join("base.txt")).unwrap();
    assert!(contents.contains("remote change"));
}

#[test]
fn origin_url_returns_the_configured_origin_url() {
    let (bare, repo) = setup_remote();
    let runner = GitRunner::discover_in(repo.path()).unwrap();
    assert_eq!(runner.origin_url().unwrap(), Some(file_url(bare.path())));
}

#[test]
fn origin_url_is_none_without_an_origin_remote() {
    let repo = TempDir::new().unwrap();
    git(repo.path(), &["init", "-q"]);
    configure_identity(repo.path());

    let runner = GitRunner::discover_in(repo.path()).unwrap();
    assert_eq!(runner.origin_url().unwrap(), None);
}

#[test]
fn pull_with_divergent_edits_surfaces_conflicted_files_as_unmerged() {
    let (bare, repo) = setup_remote();
    // The remote edits line two one way...
    advance_remote(
        bare.path(),
        b"line one\nremote two\n",
        "remote edit of line two",
    );
    // ...while the local clone edits the same line another way and commits.
    write(repo.path(), "base.txt", b"line one\nlocal two\n");
    git(repo.path(), &["commit", "-aqm", "local edit of line two"]);

    // The pull tries to merge and hits a conflict: it exits non-zero, which
    // redquill surfaces (not a crash) — no conflict resolution is attempted.
    let out = run_remote(RemoteOp::Pull, repo.path());
    assert!(
        !out.status.success(),
        "expected the conflicting pull to fail"
    );

    // The conflicted file appears as an unmerged entry in the parsed status —
    // exactly the existing unmerged-status parsing, no new machinery.
    let runner = GitRunner::discover_in(repo.path()).unwrap();
    let status = runner.status().unwrap();
    let conflicted = status
        .iter()
        .find(|s| s.path == "base.txt")
        .expect("base.txt should appear in status");
    assert_eq!(conflicted.kind, ChangeKind::Unmerged);
}
