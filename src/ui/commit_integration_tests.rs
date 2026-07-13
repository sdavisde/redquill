//! Real-git integration tests for spec 04 (commit staged changes from the
//! git panel), driven through the actual key-dispatch pipeline
//! (`` ` `` -> `c` -> typed message -> `Enter`/`Esc`) against throwaway
//! repositories built in tempdirs, per this repo's testing convention (see
//! CLAUDE.md / `docs/rust-best-practices.md`) — never the host repo.
//!
//! Lives beside `git_switch_integration_tests.rs` for the same reason that
//! file documents: `dispatch_key` and the commit modal's key handler are
//! crate-internal by design, so a `tests/*.rs` binary could not drive keys
//! into the modal; living here keeps the coverage genuinely end-to-end
//! (real `git` subprocesses through the real background poller, real key
//! dispatch) without widening the public API for a test's sake.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tempfile::TempDir;

use super::stage_ops::build_review;
use super::*;
use crate::git::{DiffTarget, GitRunner};

// -- Repo/dispatch fixtures -------------------------------------------------

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

/// A repo with one committed file whose working-tree edit is fully staged —
/// the precondition the `c` gesture needs. Identity and hooks path are
/// pinned locally so no host git config can leak in; the tempdir path is
/// canonicalized on use via `GitRunner::discover_in` (macOS `/var` symlink).
fn repo_with_staged_change() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.email", "test@redquill.invalid"]);
    git(dir, &["config", "user.name", "redquill test"]);
    git(dir, &["config", "core.hooksPath", ".git/hooks"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
    write(dir, "a.txt", "one\n");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "initial"]);
    write(dir, "a.txt", "one\ntwo\n");
    git(dir, &["add", "a.txt"]);
    tmp
}

/// An `App` with a real `GitRunner` rooted at `dir`, wired exactly like
/// `main.rs` wires the real thing (`with_git` + `set_repo_root`).
fn app_for(dir: &Path) -> App {
    let runner = GitRunner::discover_in(dir).expect("discover repo");
    let snapshot = build_review(&runner, &DiffTarget::WorkingTree).expect("build review");
    let mut app = App::with_git(snapshot, DiffTarget::WorkingTree, Box::new(runner.clone()));
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
        KeyEvent::new(code, KeyModifiers::NONE),
    );
}

/// Types `text` into the commit modal, character by character, through the
/// real dispatch pipeline.
fn type_text(app: &mut App, keymap: &Keymap, pending: &mut Option<KeyEvent>, text: &str) {
    for c in text.chars() {
        press(app, keymap, pending, KeyCode::Char(c));
    }
}

/// Opens the git panel and the commit modal via `` ` `` then `c`.
fn open_commit_modal(app: &mut App, keymap: &Keymap, pending: &mut Option<KeyEvent>) {
    press(app, keymap, pending, KeyCode::Char('`'));
    assert!(matches!(app.mode, Mode::Panel { .. }), "panel must focus");
    press(app, keymap, pending, KeyCode::Char('c'));
    assert_eq!(app.mode, Mode::CommitMessage, "c must open the modal");
}

/// Drains the background poller until the in-flight commit completes (the
/// event loop's tick, minus rendering). Panics if nothing completes in time.
fn wait_for_commit(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.git_op.is_some() && Instant::now() < deadline {
        app.poll_git_ops();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(app.git_op.is_none(), "commit did not complete in time");
}

// -- Scenarios ---------------------------------------------------------------

/// The happy path (spec success metric 1): stage -> `c` -> message ->
/// `Enter` produces a real commit, the panel refreshes (CHANGES empties of
/// the committed file, last-commit line updates), and annotations survive.
#[test]
fn commit_staged_creates_a_commit_and_refreshes_the_review() {
    let tmp = repo_with_staged_change();
    let dir = tmp.path();
    let mut app = app_for(dir);
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    app.annotations
        .add(
            crate::annotate::Target::file("a.txt"),
            crate::annotate::Classification::Nit,
            "survives the commit",
        )
        .unwrap();
    assert_eq!(app.staged.len(), 1, "fixture must have a staged file");

    open_commit_modal(&mut app, &keymap, &mut pending);
    type_text(&mut app, &keymap, &mut pending, "feat: from redquill");
    press(&mut app, &keymap, &mut pending, KeyCode::Enter);

    // Submit closed the modal back to the panel; the commit runs behind it.
    assert!(matches!(app.mode, Mode::Panel { .. }));
    wait_for_commit(&mut app);

    // A real commit exists with the exact subject.
    assert_eq!(
        git_out(dir, &["log", "-1", "--format=%s"]),
        "feat: from redquill"
    );
    // The refresh emptied the staged list and updated the last-commit line.
    assert!(
        app.staged.is_empty(),
        "CHANGES must empty of committed files"
    );
    assert_eq!(
        app.last_commit.as_ref().map(|c| c.subject.as_str()),
        Some("feat: from redquill")
    );
    // Transparency: the command log recorded the run, and the footer said so.
    let entry = app.command_log.entries().next().expect("a log entry");
    assert!(entry.success);
    assert!(entry.command_line.starts_with("git commit -m"));
    assert_eq!(app.status_message.as_deref(), Some("commit succeeded"));
    // Review state survives.
    assert_eq!(app.annotations.len(), 1);
}

/// `Ctrl-j` builds a body under the subject line, and the message reaches
/// git verbatim — newlines preserved, nothing shell-mangled.
#[test]
fn multiline_message_reaches_git_verbatim() {
    let tmp = repo_with_staged_change();
    let dir = tmp.path();
    let mut app = app_for(dir);
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    open_commit_modal(&mut app, &keymap, &mut pending);
    type_text(&mut app, &keymap, &mut pending, "feat: subject");
    let ctrl_j = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL);
    dispatch_key(&mut app, &keymap, &mut pending, ctrl_j);
    dispatch_key(&mut app, &keymap, &mut pending, ctrl_j);
    type_text(&mut app, &keymap, &mut pending, "body $(hostname) `pwd`");
    press(&mut app, &keymap, &mut pending, KeyCode::Enter);
    wait_for_commit(&mut app);

    // %B is the raw body; `git_out` trims the trailing newline git appends.
    assert_eq!(
        git_out(dir, &["log", "-1", "--format=%B"]),
        "feat: subject\n\nbody $(hostname) `pwd`",
        "newlines preserved, no shell ever expanded the message"
    );
}

/// A rejecting pre-commit hook (spec Unit 2): the failure lands in the
/// command log with git's stderr, the footer points at the log, nothing
/// crashes, and the staged changes stay staged for a retry.
#[test]
fn rejected_pre_commit_hook_lands_in_the_command_log() {
    let tmp = repo_with_staged_change();
    let dir = tmp.path();
    let hook = dir.join(".git/hooks/pre-commit");
    std::fs::write(
        &hook,
        "#!/bin/sh\necho 'redquill hook says no' >&2\nexit 1\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let mut app = app_for(dir);
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    open_commit_modal(&mut app, &keymap, &mut pending);
    type_text(&mut app, &keymap, &mut pending, "feat: rejected");
    press(&mut app, &keymap, &mut pending, KeyCode::Enter);
    wait_for_commit(&mut app);

    // No commit was created; the tip is still the fixture's initial commit.
    assert_eq!(git_out(dir, &["log", "-1", "--format=%s"]), "initial");
    let entry = app.command_log.entries().next().expect("a log entry");
    assert!(!entry.success);
    assert!(
        entry.stderr.contains("redquill hook says no"),
        "the hook's stderr must be visible in the log, got {:?}",
        entry.stderr
    );
    assert_eq!(
        app.status_message.as_deref(),
        Some("commit failed \u{2014} see command log (@)")
    );
    // The staged changes survived the rejection, ready for a retry.
    assert_eq!(app.staged.len(), 1);
}

/// With nothing staged, `c` never opens the modal — a footer message and
/// the panel keeps focus (spec Unit 1).
#[test]
fn c_with_nothing_staged_is_a_footer_message() {
    let tmp = repo_with_staged_change();
    let dir = tmp.path();
    git(dir, &["reset", "-q", "a.txt"]); // unstage; edit stays in the tree
    let mut app = app_for(dir);
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    assert!(app.staged.is_empty());

    press(&mut app, &keymap, &mut pending, KeyCode::Char('`'));
    press(&mut app, &keymap, &mut pending, KeyCode::Char('c'));

    assert!(matches!(app.mode, Mode::Panel { .. }), "panel keeps focus");
    assert!(app.commit_message.is_none(), "no modal opened");
    assert_eq!(
        app.status_message.as_deref(),
        Some("nothing staged to commit")
    );
    assert_eq!(git_out(dir, &["log", "-1", "--format=%s"]), "initial");
}

/// `Enter` on an empty (or whitespace-only) message is rejected with a
/// footer message and the modal stays open; `Esc` then cancels back to the
/// panel at its prior cursor row with no commit made (spec Unit 1).
#[test]
fn empty_message_is_rejected_and_esc_cancels_back_to_the_panel() {
    let tmp = repo_with_staged_change();
    let dir = tmp.path();
    let mut app = app_for(dir);
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    open_commit_modal(&mut app, &keymap, &mut pending);
    press(&mut app, &keymap, &mut pending, KeyCode::Enter);
    assert_eq!(app.mode, Mode::CommitMessage, "modal must stay open");
    assert_eq!(
        app.status_message.as_deref(),
        Some("commit message is empty")
    );

    // Whitespace-only is equally blank.
    type_text(&mut app, &keymap, &mut pending, "   ");
    press(&mut app, &keymap, &mut pending, KeyCode::Enter);
    assert_eq!(app.mode, Mode::CommitMessage, "modal must stay open");

    press(&mut app, &keymap, &mut pending, KeyCode::Esc);
    assert!(
        matches!(app.mode, Mode::Panel { .. }),
        "Esc cancels to panel"
    );
    assert!(app.git_op.is_none(), "nothing was ever spawned");
    assert_eq!(git_out(dir, &["log", "-1", "--format=%s"]), "initial");
}

/// `q` is inert while the commit modal is open, through the *real* dispatch
/// pipeline (the existing overlay rule): it types into the message instead
/// of quitting.
#[test]
fn q_through_dispatch_types_into_the_message_rather_than_quitting() {
    let tmp = repo_with_staged_change();
    let mut app = app_for(tmp.path());
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    open_commit_modal(&mut app, &keymap, &mut pending);
    let flow = dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
    );
    assert!(
        matches!(flow, Flow::Continue),
        "q must not end the session while the modal is open"
    );
    assert_eq!(app.mode, Mode::CommitMessage);
    assert_eq!(app.commit_message.as_ref().unwrap().buffer.text(), "q");
}
