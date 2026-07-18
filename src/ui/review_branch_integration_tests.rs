//! Real-git integration tests for the review-branch modal's in-app entry
//! path, driven through the actual key-dispatch pipeline for focus and
//! confirm (`` ` `` -> `Enter`) against throwaway repositories built in
//! tempdirs, per this repo's testing convention — never the host repo. The
//! modal itself opens via a direct `App::open_review_branch_modal()` call
//! rather than a keypress: its panel-scope `R` binding moved to the Review
//! launcher's global `R`, so this file now exercises the confirm/reroot
//! machinery the launcher's Branches tab will drive once migrated.
//!
//! Lives beside `git_switch_integration_tests.rs`/`review_guard_integration_tests.rs`
//! for the identical reason those files document: `dispatch_key`,
//! `open_review_branch_modal`, and `confirm_review_branch` are
//! crate-internal by design (the modal bypasses the public `Action`/
//! `Keymap` surface entirely, same as the switcher), so a `tests/*.rs`
//! binary — which only sees this crate's `pub` surface — has no way to
//! drive `Enter` inside it. Living here keeps the coverage genuinely
//! end-to-end (real `git worktree add` subprocesses, real key dispatch, a
//! real full-frame render) without widening the public API for a test's
//! sake.
//!
//! Every fixture is built with `tempfile`; every mutating git call is
//! preceded by `assert_inside_tempdir` (a local copy of the shared isolation
//! guard `tests/git_review_integration.rs`/`tests/cli_review_integration.rs`
//! use — duplicated here rather than shared, matching how every one of
//! those files already carries its own copy, per this repo's established
//! one-copy-per-file convention: this in-crate module can't share code
//! across the crate boundary with the `tests/*.rs` binaries).
//! This file is exactly the incident's risk shape (worktree creation driven
//! through a modal confirm gesture), so the guard runs before every
//! mutating call as a matter of discipline, per the task's explicit
//! requirement.

use std::path::{Path, PathBuf};
use std::process::Command;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tempfile::TempDir;

use super::stage_ops::build_review;
use super::*;
use crate::git::{DiffTarget, GitRunner};
use crate::review::{ReviewStatus, store};

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

fn configure_identity(dir: &Path) {
    git(dir, &["config", "user.email", "test@redquill.invalid"]);
    git(dir, &["config", "user.name", "redquill test"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
}

fn canon(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|e| panic!("canonicalize {path:?}: {e}"))
}

/// The shared isolation guard every mutating git call in this file runs
/// before touching disk (local copy per this repo's established
/// one-copy-per-file rule).
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

/// A repo on `main` (checked out, clean) with one commit, plus a `feature`
/// branch (not checked out anywhere) one commit ahead that changes `a.rs` —
/// the fixture for a successful in-app review start.
fn repo_with_feature_branch() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    assert_inside_tempdir(dir, &tmp);
    git(dir, &["init", "-q", "-b", "main"]);
    configure_identity(dir);
    write(dir, "a.rs", "fn a() {}\n");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "initial"]);
    git(dir, &["branch", "feature"]);
    git(dir, &["checkout", "-q", "feature"]);
    write(dir, "a.rs", "fn a() { changed(); }\n");
    git(dir, &["commit", "-aq", "-m", "feature tip"]);
    git(dir, &["checkout", "-q", "main"]);
    tmp
}

/// Same as [`repo_with_feature_branch`], except `feature` is already checked
/// out in a second, unmanaged worktree — the shape that makes `git worktree
/// add` (and therefore the review-branch modal's confirm gesture) fail.
fn repo_with_feature_checked_out_elsewhere() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    assert_inside_tempdir(dir, &tmp);
    git(dir, &["init", "-q", "-b", "main"]);
    configure_identity(dir);
    write(dir, "a.rs", "fn a() {}\n");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "initial"]);
    git(dir, &["branch", "feature"]);
    let elsewhere = dir.join("elsewhere-wt");
    assert_inside_tempdir(&elsewhere, &tmp);
    git(
        dir,
        &[
            "worktree",
            "add",
            "-q",
            elsewhere.to_str().unwrap(),
            "feature",
        ],
    );
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

/// Dispatches one key through the real `dispatch_key` pipeline — the same
/// handler the blocking event loop calls — so these tests exercise mode
/// routing and the review-branch modal's own key handler exactly as the
/// product does.
fn press(app: &mut App, keymap: &Keymap, pending: &mut Option<KeyEvent>, code: KeyCode) {
    dispatch_key(
        app,
        keymap,
        pending,
        &mut None,
        KeyEvent::new(code, KeyModifiers::NONE),
    );
}

/// Renders a full frame (banner included) and flattens it to a plain string
/// for substring assertions — the same test-render approach
/// `mod_tests.rs`/`git_switch_integration_tests.rs` use.
fn render_frame(app: &App, keymap: &Keymap) -> String {
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| draw(frame, app, keymap, None))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect()
}

/// Prints the rendered frame row-by-row to stderr when `REDQUILL_PROOF_DUMP`
/// is set — the same proof-capture convention
/// `review_persistence_integration_tests.rs`/`mod_tests.rs` use, so a proof
/// artifact can `eprintln!`-capture a real frame rather than transcribing it
/// by hand.
fn dump_frame(label: &str, app: &App, keymap: &Keymap) {
    if std::env::var_os("REDQUILL_PROOF_DUMP").is_none() {
        return;
    }
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| draw(frame, app, keymap, None))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let w = buffer.area.width as usize;
    let symbols: Vec<&str> = buffer.content().iter().map(|c| c.symbol()).collect();
    eprintln!("-- {label} --");
    for row in symbols.chunks(w) {
        let line = row.concat();
        if !line.trim().is_empty() {
            eprintln!("{line}");
        }
    }
}

// -- Scenarios ---------------------------------------------------------------

#[test]
fn review_branch_modal_lists_local_branches_excluding_the_current_one() {
    let tmp = repo_with_feature_branch();
    let dir = tmp.path();
    let mut app = app_for(dir);
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    press(&mut app, &keymap, &mut pending, KeyCode::Char('`')); // focus git panel
    assert!(matches!(app.mode, Mode::Panel { .. }));
    app.open_review_branch_modal(); // panel-scope `R` moved to the Review launcher's global `R`
    assert_eq!(app.mode, Mode::ReviewBranch);

    let branches = &app.review_branch_modal.as_ref().unwrap().branches;
    assert!(
        branches.iter().all(|b| b.name != "main"),
        "the currently checked-out branch must be excluded: {branches:?}"
    );
    assert!(
        branches.iter().any(|b| b.name == "feature"),
        "feature must be listed: {branches:?}"
    );

    let content = render_frame(&app, &keymap);
    assert!(content.contains("Review branch"));
    assert!(content.contains("feature"));
    dump_frame("review-branch modal open over the git panel", &app, &keymap);

    // Esc closes without starting anything.
    press(&mut app, &keymap, &mut pending, KeyCode::Esc);
    assert!(matches!(app.mode, Mode::Panel { .. }));
    assert!(matches!(app.target, DiffTarget::WorkingTree));

    drop(tmp);
}

#[test]
fn review_branch_modal_reroots_into_a_bannered_review_session_with_persisted_marks_restored() {
    let tmp = repo_with_feature_branch();
    let dir = tmp.path();
    assert_inside_tempdir(dir, &tmp);

    // Pre-seed persisted review progress for `feature` (as if a prior
    // paused CLI session had already accepted a.rs) — proves the in-app
    // path restores it exactly like the CLI path does.
    let runner = GitRunner::discover_in(dir).unwrap();
    let common_dir = runner.git_common_dir().unwrap();
    let state_path = common_dir.join("redquill").join("review-state.json");
    assert_inside_tempdir(&state_path, &tmp);
    let a_sha = runner.blob_sha("feature", "a.rs").unwrap().unwrap();
    let mut files = std::collections::BTreeMap::new();
    files.insert(
        "a.rs".to_string(),
        store::PersistedFile {
            status: store::PersistedStatus::Accepted,
            blob_sha: Some(a_sha),
        },
    );
    store::save_review(
        &state_path,
        "feature",
        store::PersistedReview {
            base: "main".to_string(),
            worktree_path: dir.join("unused-in-this-fixture"),
            files,
            annotations: Vec::new(),
        },
    )
    .unwrap();

    let mut app = app_for(dir);
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    press(&mut app, &keymap, &mut pending, KeyCode::Char('`'));
    app.open_review_branch_modal(); // panel-scope `R` moved to the Review launcher's global `R`
    assert_eq!(app.mode, Mode::ReviewBranch);
    press(&mut app, &keymap, &mut pending, KeyCode::Enter); // confirm on `feature`

    assert!(
        matches!(app.mode, Mode::Panel { .. }),
        "modal closes after a successful review start, got {:?}",
        app.mode
    );
    assert!(
        matches!(&app.target, DiffTarget::Review { branch, .. } if branch == "feature"),
        "target must be Review{{branch: feature}}, got {:?}",
        app.target
    );

    // The managed worktree exists under the common dir, and the user's own
    // checkout is untouched.
    let new_root = canon(app.repo_root.as_ref().expect("repo root set"));
    let managed_root = canon(&common_dir.join("redquill").join("worktrees"));
    assert!(
        new_root.starts_with(&managed_root),
        "repo_root must point inside the managed worktrees dir: {new_root:?}"
    );
    let wt_list = git_out(dir, &["worktree", "list"]);
    assert!(wt_list.contains("feature"), "worktree list: {wt_list}");
    assert_eq!(
        git_out(dir, &["symbolic-ref", "--short", "HEAD"]),
        "main",
        "the user's own checkout must stay on main"
    );
    assert!(
        git_out(dir, &["status", "--porcelain"]).is_empty(),
        "the user's own checkout must stay clean"
    );

    // Persisted marks restored, no live gesture needed.
    assert_eq!(
        app.review_states.get("a.rs"),
        Some(&ReviewStatus::Accepted),
        "a.rs must start Accepted, restored from the persisted entry"
    );

    // Bannered: the review banner names the branch under review.
    let content = render_frame(&app, &keymap);
    assert!(
        content.contains("REVIEWING feature"),
        "banner must show the branch under review: {content}"
    );
    dump_frame(
        "landed in the review session, first frame (a.rs pre-accepted)",
        &app,
        &keymap,
    );

    drop(tmp);
}

#[test]
fn review_branch_modal_surfaces_a_worktree_add_failure_without_mutating_state() {
    let tmp = repo_with_feature_checked_out_elsewhere();
    let dir = tmp.path();
    let mut app = app_for(dir);
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    press(&mut app, &keymap, &mut pending, KeyCode::Char('`'));
    app.open_review_branch_modal(); // panel-scope `R` moved to the Review launcher's global `R`
    assert_eq!(app.mode, Mode::ReviewBranch);

    let before_root = app.repo_root.clone();
    let before_target = app.target.clone();

    press(&mut app, &keymap, &mut pending, KeyCode::Enter); // confirm on `feature`, which fails

    assert_eq!(
        app.mode,
        Mode::ReviewBranch,
        "the modal must stay open on a failed worktree add"
    );
    assert!(
        app.status_message.is_some(),
        "git's failure must surface as a status message"
    );
    assert_eq!(
        app.repo_root, before_root,
        "a failed review start must not touch repo_root"
    );
    assert_eq!(
        app.target, before_target,
        "a failed review start must not touch the diff target"
    );

    // No managed worktree was created.
    let runner = GitRunner::discover_in(dir).unwrap();
    let common_dir = runner.git_common_dir().unwrap();
    let managed = common_dir.join("redquill").join("worktrees");
    assert!(
        !managed.exists() || std::fs::read_dir(&managed).unwrap().next().is_none(),
        "no worktree must have been left behind by a failed attempt"
    );
    assert_eq!(
        git_out(dir, &["symbolic-ref", "--short", "HEAD"]),
        "main",
        "the user's own checkout must stay untouched"
    );

    drop(tmp);
}
