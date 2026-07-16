//! Real-git integration tests for spec 08 Unit 5 (local-mode parity: the
//! accepted-files panel and the pull/push confirm guard), driven through the
//! actual key-dispatch pipeline against throwaway repositories built in
//! tempdirs — never the host repo.
//!
//! Lives beside `commit_integration_tests.rs`/`git_switch_integration_tests.rs`
//! for the identical reason those files document: `dispatch_key` and the
//! confirm-modal/accepted-panel handlers are crate-internal by design, so a
//! `tests/*.rs` binary could not drive keys into them; living here keeps the
//! coverage genuinely end-to-end (real `git` subprocesses through the real
//! background poller, real key dispatch) without widening the public API for
//! a test's sake.
//!
//! Every fixture is built with `tempfile`; every mutating git call is
//! preceded by `assert_inside_tempdir` (a local copy of the shared isolation
//! guard `tests/git_review_integration.rs`/`tests/cli_review_integration.rs`
//! introduced in task 1.5 — duplicated here rather than shared, matching how
//! every one of those files already carries its own copy, since `tests/*.rs`
//! and this in-crate module can't share code across the crate boundary).
//! Neither test in this file ever runs `git worktree add`/`remove` — the
//! per-file review-status/panel logic and the remote-op guard are both
//! provable against a plain two-ref diff (`base...branch`) in an ordinary
//! checkout, so the worktree-incident risk shape (2026-07-16 notes) never
//! applies here; the isolation guard is still run before every mutating call
//! as a matter of discipline, per the task's explicit requirement.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tempfile::TempDir;

use super::stage_ops::build_review;
use super::*;
use crate::git::{DiffTarget, GitRunner, RemoteOp};

// -- Repo/dispatch fixtures --------------------------------------------------

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("failed to spawn git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_out(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("failed to spawn git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn write(dir: &Path, rel: &str, contents: &str) {
    std::fs::write(dir.join(rel), contents).unwrap();
}

fn canon(path: &Path) -> std::path::PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|e| panic!("canonicalize {path:?}: {e}"))
}

/// Asserts `path` is lexically inside `tmp`'s canonicalized root — the same
/// isolation guard `tests/git_review_integration.rs`/
/// `tests/cli_review_integration.rs` run before every mutating git call (see
/// the module doc for why this is a local copy rather than a shared helper).
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

/// A repo on `main` with one commit (`a.rs`/`b.rs`), plus a `feature` branch
/// one commit ahead that changes both files — the "review two files, accept
/// them, un-accept one from the panel" fixture.
fn repo_with_feature_branch_two_files() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    assert_inside_tempdir(dir, &tmp);
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.email", "test@redquill.invalid"]);
    git(dir, &["config", "user.name", "redquill test"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
    write(dir, "a.rs", "fn a() {}\n");
    write(dir, "b.rs", "fn b() {}\n");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "initial"]);
    git(dir, &["checkout", "-qb", "feature"]);
    write(dir, "a.rs", "fn a() { changed(); }\n");
    write(dir, "b.rs", "fn b() { changed(); }\n");
    git(dir, &["commit", "-aqm", "feature change"]);
    tmp
}

/// A bare "origin" plus a "review" checkout whose `main` is pushed, then
/// advanced by one more local-only commit — origin starts one commit behind,
/// so a confirmed push has a real, observable effect (its ref advances to
/// match local) and a cancelled one has none.
fn origin_and_review_repo() -> (TempDir, TempDir) {
    let origin = TempDir::new().unwrap();
    assert_inside_tempdir(origin.path(), &origin);
    git(origin.path(), &["init", "-q", "--bare", "-b", "main"]);

    let review = TempDir::new().unwrap();
    assert_inside_tempdir(review.path(), &review);
    let dir = review.path();
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.email", "test@redquill.invalid"]);
    git(dir, &["config", "user.name", "redquill test"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
    write(dir, "a.txt", "one\n");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "initial"]);
    git(
        dir,
        &[
            "remote",
            "add",
            "origin",
            origin.path().to_str().expect("tempdir path is valid utf8"),
        ],
    );
    git(dir, &["push", "-q", "-u", "origin", "main"]);
    write(dir, "a.txt", "one\ntwo\n");
    git(dir, &["add", "a.txt"]);
    git(dir, &["commit", "-qm", "second (local only)"]);
    (origin, review)
}

/// An `App` with a real `GitRunner` rooted at `dir`, reviewing `base...branch`
/// — wired the same way `main.rs` wires a review session (`with_git` +
/// `set_repo_root`).
fn app_for_review(dir: &Path, base: &str, branch: &str) -> App {
    let runner = GitRunner::discover_in(dir).expect("discover repo");
    let target = DiffTarget::Review {
        base: base.to_string(),
        branch: branch.to_string(),
    };
    let snapshot = build_review(&runner, &target).expect("build review");
    let mut app = App::with_git(snapshot, target, Box::new(runner.clone()));
    app.set_repo_root(runner.root().to_path_buf());
    app
}

/// Dispatches one plain key through the real `dispatch_key` pipeline — the
/// same handler the blocking event loop calls.
fn press(app: &mut App, keymap: &Keymap, pending: &mut Option<KeyEvent>, code: KeyCode) {
    dispatch_key(
        app,
        keymap,
        pending,
        &mut None,
        KeyEvent::new(code, KeyModifiers::NONE),
    );
}

/// Drains the background poller until the in-flight mutating git op
/// completes (the event loop's tick, minus rendering). Panics if nothing
/// completes in time.
fn wait_for_git_op(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.git_op.is_some() && Instant::now() < deadline {
        app.poll_git_ops();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(app.git_op.is_none(), "git op did not complete in time");
}

// -- Accepted-files panel (spec 08 Unit 5, task 6.2/6.4) ---------------------

/// The full round trip against a real review session: accept both files,
/// open the accepted-files panel (`s`), un-accept one (`Space`) — its diff
/// section re-expands and the banner's live count drops, all through the
/// real key-dispatch pipeline and a real `git diff base...branch`.
#[test]
fn accepted_files_panel_un_accept_round_trips_against_a_real_review_session() {
    let tmp = repo_with_feature_branch_two_files();
    let mut app = app_for_review(tmp.path(), "main", "feature");
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    assert_eq!(app.view.files.len(), 2, "fixture must diff exactly 2 files");

    // Accept a.rs (cursor starts on its header), then move to b.rs's header
    // and accept it too.
    press(&mut app, &keymap, &mut pending, KeyCode::Char(' '));
    press(&mut app, &keymap, &mut pending, KeyCode::Tab);
    press(&mut app, &keymap, &mut pending, KeyCode::Char(' '));
    assert_eq!(app.review_progress(), (2, 2));
    assert!(app.view.is_collapsed("a.rs"));
    assert!(app.view.is_collapsed("b.rs"));

    // `s` opens the accepted-files panel (not the local staging panel — this
    // review session's `git status` is clean) listing both.
    press(&mut app, &keymap, &mut pending, KeyCode::Char('s'));
    assert_eq!(app.mode, Mode::Staging);
    assert_eq!(
        app.staged
            .iter()
            .map(|f| f.path.as_str())
            .collect::<Vec<_>>(),
        vec!["a.rs", "b.rs"]
    );

    // `Space` un-accepts the focused (first) entry.
    press(&mut app, &keymap, &mut pending, KeyCode::Char(' '));
    assert_eq!(
        app.staged
            .iter()
            .map(|f| f.path.as_str())
            .collect::<Vec<_>>(),
        vec!["b.rs"],
        "the un-accepted file drops off the panel list"
    );
    assert_eq!(app.review_progress(), (1, 2));

    press(&mut app, &keymap, &mut pending, KeyCode::Char('s')); // close panel
    assert_eq!(app.mode, Mode::Normal);
    assert!(
        !app.view.is_collapsed("a.rs"),
        "un-accepting must re-expand the section"
    );
    assert!(
        app.view.is_collapsed("b.rs"),
        "the still-accepted file stays collapsed"
    );
}

// -- Guarded panel writes (spec 08 Unit 5, task 6.3/6.4) ---------------------

/// `P` in a review session opens the confirm modal; confirming (`Enter`)
/// spawns the real `git push` and — after the background poller drains it —
/// the scratch remote's ref has genuinely advanced to match local. Proves
/// the confirm gate hands off to the unchanged, already-tested
/// `request_remote_op` path rather than intercepting or altering it.
#[test]
fn confirmed_push_in_a_review_session_actually_updates_the_scratch_remote() {
    let (origin, review) = origin_and_review_repo();
    let dir = review.path();

    let local_head = git_out(dir, &["rev-parse", "main"]);
    let origin_head_before = git_out(origin.path(), &["rev-parse", "main"]);
    assert_ne!(
        origin_head_before, local_head,
        "fixture must start with origin behind local"
    );

    let mut app = app_for_review(dir, "HEAD~1", "main");
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    press(&mut app, &keymap, &mut pending, KeyCode::Char('`')); // focus panel
    assert!(matches!(app.mode, Mode::Panel { .. }));
    press(&mut app, &keymap, &mut pending, KeyCode::Char('P'));
    assert!(
        matches!(app.mode, Mode::ConfirmRemoteOp { op, .. } if op == RemoteOp::Push),
        "P in a review session must open the push confirm modal, got {:?}",
        app.mode
    );

    press(&mut app, &keymap, &mut pending, KeyCode::Enter); // confirm
    assert!(
        matches!(app.mode, Mode::Panel { .. }),
        "confirming must return to the panel immediately, the op runs behind it"
    );
    wait_for_git_op(&mut app);

    let origin_head_after = git_out(origin.path(), &["rev-parse", "main"]);
    assert_eq!(
        origin_head_after, local_head,
        "a confirmed push must genuinely update the scratch remote"
    );
}

/// The mirror image: `Esc` cancels the confirm modal and the scratch remote
/// is never touched — proving the guard actually gates the write, not just
/// the modal's own text.
#[test]
fn cancelled_push_in_a_review_session_never_touches_the_scratch_remote() {
    let (origin, review) = origin_and_review_repo();
    let dir = review.path();
    let origin_head_before = git_out(origin.path(), &["rev-parse", "main"]);

    let mut app = app_for_review(dir, "HEAD~1", "main");
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    press(&mut app, &keymap, &mut pending, KeyCode::Char('`'));
    press(&mut app, &keymap, &mut pending, KeyCode::Char('P'));
    assert!(matches!(app.mode, Mode::ConfirmRemoteOp { .. }));
    press(&mut app, &keymap, &mut pending, KeyCode::Esc);
    assert!(matches!(app.mode, Mode::Panel { .. }));
    assert!(app.git_op.is_none(), "cancel must spawn nothing");

    let origin_head_after = git_out(origin.path(), &["rev-parse", "main"]);
    assert_eq!(
        origin_head_before, origin_head_after,
        "a cancelled push must never touch the scratch remote"
    );
}

/// `f` (fetch) stays unprompted in a review session and runs immediately —
/// the confirm guard applies only to `p`/`P` (spec 08 Unit 5's explicit
/// scoping, "reviewers are expected to fetch").
#[test]
fn fetch_stays_unprompted_in_a_review_session_against_a_real_remote() {
    let (_origin, review) = origin_and_review_repo();
    let dir = review.path();

    let mut app = app_for_review(dir, "HEAD~1", "main");
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    press(&mut app, &keymap, &mut pending, KeyCode::Char('`'));
    press(&mut app, &keymap, &mut pending, KeyCode::Char('f'));
    assert!(
        matches!(app.mode, Mode::Panel { .. }),
        "f must never open a confirm modal, got {:?}",
        app.mode
    );
    wait_for_git_op(&mut app);
    assert_eq!(
        app.running_op_label(),
        None,
        "fetch must have run to completion, not be left pending"
    );
}
