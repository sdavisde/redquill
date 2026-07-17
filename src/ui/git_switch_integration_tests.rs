//! Real-git integration tests for branch switch + worktree
//! re-root, driven through the actual key-dispatch pipeline
//! (`` ` `` -> `b` -> `j`/`k`/`Tab` -> `Enter`) against throwaway repositories
//! built in tempdirs via `std::process::Command`, per this repo's testing
//! convention (see CLAUDE.md/`docs/rust-best-practices.md`) — never the host
//! repo.
//!
//! This lives beside the other real-git, real-dispatch tests in `src/ui`
//! (e.g. `capture_task_04_smoke_transcript` in `mod_tests.rs`) rather than in
//! the top-level `tests/` directory: `dispatch_key`, `draw`, and the
//! switcher's modal key handler are crate-internal by design (the modal
//! bypasses the public `Action`/`Keymap` surface entirely, same as
//! List/Staging/Peek), so a `tests/*.rs` binary — which only sees this
//! crate's `pub` surface — has no way to drive `Enter` inside the switcher
//! modal. Living here keeps the coverage genuinely end-to-end (real `git`
//! subprocesses, real key dispatch) without widening the public API just for
//! a test's sake.

use std::path::Path;
use std::process::Command;

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
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

fn configure_identity(dir: &Path) {
    git(dir, &["config", "user.email", "test@example.com"]);
    git(dir, &["config", "user.name", "Test"]);
}

/// A repo with two branches, `main` (checked out, clean) and `feature`, each
/// with its own committed tip so a switch is observable via the branch
/// header, the last-commit summary, and real `git` state.
fn repo_with_two_branches() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-q", "-b", "main"]);
    configure_identity(dir);
    write(dir, "a.txt", "shared\n");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "main tip"]);
    git(dir, &["branch", "feature"]);
    git(dir, &["checkout", "-q", "feature"]);
    write(dir, "b.txt", "feature only\n");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "feature tip"]);
    git(dir, &["checkout", "-q", "main"]);
    tmp
}

/// A repo where `main`'s working tree has an uncommitted edit that conflicts
/// with `feature`'s committed version of the same file — `git switch
/// feature` refuses ("local changes would be overwritten").
fn repo_with_conflicting_branches() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-q", "-b", "main"]);
    configure_identity(dir);
    write(dir, "a.txt", "from main\n");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "main tip"]);
    git(dir, &["checkout", "-q", "-b", "feature"]);
    write(dir, "a.txt", "from feature\n");
    git(dir, &["commit", "-aq", "-m", "feature tip"]);
    git(dir, &["checkout", "-q", "main"]);
    // Uncommitted, conflicting with feature's committed a.txt.
    write(dir, "a.txt", "local dirty\n");
    tmp
}

/// A main repo plus a linked worktree checked out on `feature`, with an
/// uncommitted change that exists only in the worktree's own checkout — the
/// signal a re-root must pick up that a plain refresh against the old root
/// never would.
fn repo_with_linked_worktree() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
    let base = TempDir::new().unwrap();
    let main_dir = base.path().join("main");
    std::fs::create_dir_all(&main_dir).unwrap();
    git(&main_dir, &["init", "-q", "-b", "main"]);
    configure_identity(&main_dir);
    write(&main_dir, "a.txt", "main content\n");
    git(&main_dir, &["add", "."]);
    git(&main_dir, &["commit", "-q", "-m", "main tip"]);
    git(&main_dir, &["branch", "feature"]);

    let wt_dir = base.path().join("wt");
    git(
        &main_dir,
        &["worktree", "add", "-q", wt_dir.to_str().unwrap(), "feature"],
    );
    // Uncommitted, worktree-only.
    write(&wt_dir, "b.txt", "worktree-only change\n");

    (base, main_dir, wt_dir)
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

/// Dispatches one key through the real `dispatch_key` pipeline — the same
/// handler the blocking event loop calls — so these tests exercise mode
/// routing and the switcher's modal handler exactly as the product does.
fn press(app: &mut App, keymap: &Keymap, pending: &mut Option<KeyEvent>, code: KeyCode) {
    dispatch_key(
        app,
        keymap,
        pending,
        &mut None,
        KeyEvent::new(code, KeyModifiers::NONE),
    );
}

/// Steps the switcher's active-tab cursor from `current` to `target` via
/// repeated `j`/`k` presses — direction-agnostic so these tests don't depend
/// on `git`'s branch/worktree listing order.
fn drive_cursor(
    app: &mut App,
    keymap: &Keymap,
    pending: &mut Option<KeyEvent>,
    current: usize,
    target: usize,
) {
    if target >= current {
        for _ in 0..(target - current) {
            press(app, keymap, pending, KeyCode::Char('j'));
        }
    } else {
        for _ in 0..(current - target) {
            press(app, keymap, pending, KeyCode::Char('k'));
        }
    }
}

// -- Scenarios ---------------------------------------------------------------

#[test]
fn switch_branch_rebuilds_review_snapshot() {
    let tmp = repo_with_two_branches();
    let dir = tmp.path();
    let mut app = app_for(dir);
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    assert_eq!(app.branch.as_ref().unwrap().name, "main");

    press(&mut app, &keymap, &mut pending, KeyCode::Char('`')); // focus git panel
    assert!(matches!(app.mode, Mode::Panel { .. }));
    press(&mut app, &keymap, &mut pending, KeyCode::Char('b')); // open switcher
    assert_eq!(app.mode, Mode::Switcher);

    let branches = app.switcher.as_ref().unwrap().branches.clone();
    let current = app.switcher.as_ref().unwrap().branch_cursor;
    let target = branches
        .iter()
        .position(|b| b.name == "feature")
        .expect("feature listed");
    drive_cursor(&mut app, &keymap, &mut pending, current, target);
    press(&mut app, &keymap, &mut pending, KeyCode::Enter); // confirm switch

    assert!(
        matches!(app.mode, Mode::Panel { .. }),
        "modal closes on a successful switch"
    );
    assert_eq!(
        git_out(dir, &["symbolic-ref", "--short", "HEAD"]),
        "feature",
        "the real repo actually switched branches"
    );
    assert_eq!(app.branch.as_ref().unwrap().name, "feature");
    assert_eq!(
        app.last_commit.as_ref().unwrap().subject,
        "feature tip",
        "refresh() rebuilt the review state against the new HEAD"
    );
    let entry = app.command_log.entries().next().expect("logged entry");
    assert_eq!(entry.command_line, "git switch -- feature");
    assert!(entry.success);
    assert_eq!(
        app.status_message.as_deref(),
        Some("switched to feature (annotations kept)")
    );
}

#[test]
fn dirty_conflicting_switch_surfaces_in_command_log() {
    let tmp = repo_with_conflicting_branches();
    let dir = tmp.path();
    let mut app = app_for(dir);
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    press(&mut app, &keymap, &mut pending, KeyCode::Char('`'));
    press(&mut app, &keymap, &mut pending, KeyCode::Char('b'));

    let branches = app.switcher.as_ref().unwrap().branches.clone();
    let current = app.switcher.as_ref().unwrap().branch_cursor;
    let target = branches
        .iter()
        .position(|b| b.name == "feature")
        .expect("feature listed");
    drive_cursor(&mut app, &keymap, &mut pending, current, target);
    press(&mut app, &keymap, &mut pending, KeyCode::Enter);

    // No panic getting here is itself part of what this test asserts.
    assert_eq!(
        app.mode,
        Mode::Switcher,
        "the modal stays open on a failed switch (spec 03 Unit 2)"
    );
    assert_eq!(
        git_out(dir, &["symbolic-ref", "--short", "HEAD"]),
        "main",
        "a refused switch must leave the real branch unchanged"
    );
    let entry = app.command_log.entries().next().expect("logged entry");
    assert!(!entry.success);
    assert!(
        !entry.stderr.is_empty(),
        "git's rejection reason must be captured in the log"
    );
    assert_eq!(
        app.status_message.as_deref(),
        Some("switch failed \u{2014} see command log (@)")
    );
}

#[test]
fn worktree_reroot_swaps_root_backend_and_snapshot() {
    let (base, main_dir, wt_dir) = repo_with_linked_worktree();
    let mut app = app_for(&main_dir);
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    press(&mut app, &keymap, &mut pending, KeyCode::Char('`'));
    press(&mut app, &keymap, &mut pending, KeyCode::Char('b'));
    press(&mut app, &keymap, &mut pending, KeyCode::Tab); // -> Worktrees tab

    let worktrees = app.switcher.as_ref().unwrap().worktrees.clone();
    let current = app.switcher.as_ref().unwrap().worktree_cursor;
    let wt_canon = std::fs::canonicalize(&wt_dir).unwrap();
    let target = worktrees
        .iter()
        .position(|w| {
            w.path
                .canonicalize()
                .map(|p| p == wt_canon)
                .unwrap_or(false)
        })
        .expect("linked worktree listed");
    drive_cursor(&mut app, &keymap, &mut pending, current, target);
    press(&mut app, &keymap, &mut pending, KeyCode::Enter); // confirm re-root

    assert!(
        matches!(app.mode, Mode::Panel { .. }),
        "modal closes after a successful re-root"
    );
    let new_root = std::fs::canonicalize(app.repo_root.as_ref().expect("repo root set")).unwrap();
    assert_eq!(new_root, wt_canon, "repo_root now points at the worktree");
    assert_eq!(
        app.status_message.as_deref(),
        Some("re-rooted (annotations kept)")
    );

    // The rebuilt snapshot reflects the worktree's own working tree, not
    // main's — b.txt only exists as an uncommitted change over there.
    assert!(
        app.view.files.iter().any(|f| f.path == "b.txt"),
        "the review snapshot must be rebuilt against the new root"
    );

    // A subsequent stage now hits the WORKTREE's index, leaving main's
    // index untouched.
    assert!(app.select_file_by_path("b.txt"));
    app.apply(Action::StageFile);
    assert!(
        git_out(&wt_dir, &["diff", "--cached", "--name-only"]).contains("b.txt"),
        "stage_file must have staged into the worktree's own index"
    );
    assert!(
        git_out(&main_dir, &["diff", "--cached", "--name-only"]).is_empty(),
        "the old root's index must be untouched by a post-reroot stage"
    );

    drop(base);
}

#[test]
fn reroot_onto_current_worktree_is_noop() {
    let (base, main_dir, _wt_dir) = repo_with_linked_worktree();
    let mut app = app_for(&main_dir);
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    press(&mut app, &keymap, &mut pending, KeyCode::Char('`'));
    press(&mut app, &keymap, &mut pending, KeyCode::Char('b'));
    press(&mut app, &keymap, &mut pending, KeyCode::Tab); // -> Worktrees tab
    // The cursor already starts on the current worktree (main_dir).
    press(&mut app, &keymap, &mut pending, KeyCode::Enter);

    assert_eq!(app.mode, Mode::Switcher, "modal stays open on the no-op");
    assert_eq!(
        app.status_message.as_deref(),
        Some("already in this worktree")
    );
    let root = std::fs::canonicalize(app.repo_root.as_ref().unwrap()).unwrap();
    let main_canon = std::fs::canonicalize(&main_dir).unwrap();
    assert_eq!(root, main_canon, "repo_root must not have changed");

    drop(base);
}
