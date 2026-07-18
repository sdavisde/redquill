//! Real-git integration tests for the Review launcher's Branches tab, driven
//! through the actual key-dispatch pipeline (`R` from anywhere, `j`/`k` to
//! move the cursor, `Enter` to confirm, `Esc` to close) against throwaway
//! repositories built in tempdirs, per this repo's testing convention —
//! never the host repo. Migrated from `review_branch_integration_tests.rs`
//! (retired alongside `Mode::ReviewBranch`): identical coverage — branch-list
//! contents, the confirm/reroot flow, the worktree-add-failure path — now
//! entered through the launcher's global `R` instead of a panel-only
//! binding, plus origin-restore coverage from both `Normal` and `Panel`.
//!
//! Lives beside `git_switch_integration_tests.rs`/`review_guard_integration_tests.rs`
//! for the identical reason those files document: `dispatch_key` and the
//! launcher's confirm machinery are crate-internal by design (the modal
//! bypasses the public `Action`/`Keymap` surface entirely, same as the
//! switcher), so a `tests/*.rs` binary — which only sees this crate's `pub`
//! surface — has no way to drive `Enter` inside it. Living here keeps the
//! coverage genuinely end-to-end (real `git worktree add` subprocesses, real
//! key dispatch, a real full-frame render) without widening the public API
//! for a test's sake.
//!
//! Every fixture is built with `tempfile`; every mutating git call is
//! preceded by `assert_inside_tempdir` (a local copy of the shared isolation
//! guard `tests/git_review_integration.rs`/`tests/cli_review_integration.rs`
//! use — duplicated here rather than shared, matching how every one of
//! those files already carries its own copy, per this repo's established
//! one-copy-per-file convention: this in-crate module can't share code
//! across the crate boundary with the `tests/*.rs` binaries). This file is
//! exactly the 2026-07-16 incident's risk shape (worktree creation driven
//! through a modal confirm gesture), so the guard runs before every mutating
//! call as a matter of discipline.

use std::path::{Path, PathBuf};
use std::process::Command;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tempfile::TempDir;

use super::app::{Mode, ModeOrigin, PanelTab};
use super::review_launcher::LauncherTab;
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
/// the fixture for a successful in-app review start with exactly one
/// candidate branch (cursor stays at 0, no `j` needed).
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

/// Same fixture, plus a second candidate branch `zulu` — `for-each-ref`
/// lists local branches alphabetically, so after excluding the current
/// `main` the Branches tab lists `alpha` then `zulu`; `j` moves the cursor
/// onto `zulu`, proving the journey works from a real multi-branch list
/// rather than a single-candidate list where `j` would be a no-op.
fn repo_with_two_feature_branches() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    assert_inside_tempdir(dir, &tmp);
    git(dir, &["init", "-q", "-b", "main"]);
    configure_identity(dir);
    write(dir, "a.rs", "fn a() {}\n");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "initial"]);
    git(dir, &["branch", "alpha"]);
    git(dir, &["branch", "zulu"]);
    git(dir, &["checkout", "-q", "zulu"]);
    write(dir, "a.rs", "fn a() { changed(); }\n");
    git(dir, &["commit", "-aq", "-m", "zulu tip"]);
    git(dir, &["checkout", "-q", "main"]);
    tmp
}

/// A repo on `feature` (checked out, clean), one commit ahead of `main` —
/// the Commits tab's ahead-of-base source (FR-11) reads the *current*
/// checkout, so this fixture puts the fresh commit exactly where the
/// Journey A transcript needs it: one keystroke (`Enter`) away from the
/// cursor's resting place (row 0, newest first).
fn repo_on_feature_with_a_fresh_commit_ahead_of_main() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    assert_inside_tempdir(dir, &tmp);
    git(dir, &["init", "-q", "-b", "main"]);
    configure_identity(dir);
    write(dir, "a.rs", "fn a() {}\n");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "initial"]);
    git(dir, &["checkout", "-qb", "feature"]);
    write(dir, "a.rs", "fn a() { changed(); }\n");
    git(dir, &["commit", "-aq", "-m", "agent: fix the thing"]);
    tmp
}

/// A repo with two commits ahead of `main` on the checked-out `feature`
/// branch — for asserting newest-first ordering (a single-commit fixture
/// can't distinguish ordering from "the only commit").
fn repo_on_feature_with_two_commits_ahead_of_main() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    assert_inside_tempdir(dir, &tmp);
    git(dir, &["init", "-q", "-b", "main"]);
    configure_identity(dir);
    write(dir, "a.rs", "fn a() {}\n");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "base commit"]);
    git(dir, &["checkout", "-qb", "feature"]);
    write(dir, "a.rs", "fn a() { one(); }\n");
    git(dir, &["commit", "-aq", "-m", "feature commit one"]);
    write(dir, "a.rs", "fn a() { two(); }\n");
    git(dir, &["commit", "-aq", "-m", "feature commit two"]);
    tmp
}

/// A repo on `main` (checked out, clean) with a single commit and no other
/// branches — the current branch *is* the auto-resolved base, so the
/// Commits tab's ahead-of-base source is empty (Journey C's precondition).
fn repo_on_the_base_branch_with_no_other_history() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    assert_inside_tempdir(dir, &tmp);
    git(dir, &["init", "-q", "-b", "main"]);
    configure_identity(dir);
    write(dir, "a.rs", "fn a() {}\n");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "initial commit"]);
    tmp
}

/// Polls `app.poll_launcher_commits()` until the Commits tab's ahead-of-base
/// fetch lands (or a 5s deadline, generous for a background thread doing
/// nothing more than a local `git log`).
fn drain_launcher_commits(app: &mut App) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while app.launcher_commits_in_flight.is_some() && std::time::Instant::now() < deadline {
        app.poll_launcher_commits();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Polls `app.poll_history()` until the History tab's (and, via the `a`
/// toggle, the Commits tab's all-commits source's) first page lands.
fn drain_history(app: &mut App) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while app.history_in_flight.is_some() && std::time::Instant::now() < deadline {
        app.poll_history();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Same as [`repo_with_feature_branch`], except `feature` is already checked
/// out in a second, unmanaged worktree — the shape that makes `git worktree
/// add` (and therefore the launcher's confirm gesture) fail.
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
/// routing and the launcher's own key handler exactly as the product does.
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

// -- Scenarios: Branches tab data (FR-8 parity) ------------------------------

#[test]
fn launcher_branches_tab_lists_local_branches_excluding_the_current_one() {
    let tmp = repo_with_feature_branch();
    let dir = tmp.path();
    let mut app = app_for(dir);
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    press(&mut app, &keymap, &mut pending, KeyCode::Char('`')); // focus git panel
    assert!(matches!(app.mode, Mode::Panel { .. }));
    press(&mut app, &keymap, &mut pending, KeyCode::Char('R')); // open the launcher (global scope)
    assert!(
        matches!(
            app.mode,
            Mode::ReviewLauncher {
                tab: LauncherTab::Branches,
                ..
            }
        ),
        "R must open the launcher on the Branches tab, got {:?}",
        app.mode
    );

    assert!(
        app.launcher_branches.iter().all(|b| b.name != "main"),
        "the currently checked-out branch must be excluded: {:?}",
        app.launcher_branches
    );
    assert!(
        app.launcher_branches.iter().any(|b| b.name == "feature"),
        "feature must be listed: {:?}",
        app.launcher_branches
    );

    let content = render_frame(&app, &keymap);
    assert!(content.contains("Branches"));
    assert!(content.contains("feature"));
    dump_frame(
        "launcher open over the git panel, Branches tab",
        &app,
        &keymap,
    );

    // Esc closes without starting anything.
    press(&mut app, &keymap, &mut pending, KeyCode::Esc);
    assert!(matches!(app.mode, Mode::Panel { .. }));
    assert!(matches!(app.target, DiffTarget::WorkingTree));

    drop(tmp);
}

// -- Scenarios: reroot-into-review happy path (FR-8, FR-9) -------------------

#[test]
fn launcher_reroots_into_a_bannered_review_session_with_persisted_marks_restored() {
    let tmp = repo_with_feature_branch();
    let dir = tmp.path();
    assert_inside_tempdir(dir, &tmp);

    // Pre-seed persisted review progress for `feature` (as if a prior
    // paused CLI session had already accepted a.rs) — proves the launcher's
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

    // Journey B, git-panel origin: `` ` ``, `R`, `Enter` — one candidate
    // branch (`feature`), so no `j` is needed.
    press(&mut app, &keymap, &mut pending, KeyCode::Char('`'));
    press(&mut app, &keymap, &mut pending, KeyCode::Char('R'));
    assert!(matches!(
        app.mode,
        Mode::ReviewLauncher {
            tab: LauncherTab::Branches,
            ..
        }
    ));
    press(&mut app, &keymap, &mut pending, KeyCode::Enter); // confirm on `feature`

    assert!(
        matches!(app.mode, Mode::Panel { .. }),
        "modal closes into the panel after a successful review start, got {:?}",
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
        "landed in the review session via the launcher, first frame (a.rs pre-accepted)",
        &app,
        &keymap,
    );

    drop(tmp);
}

#[test]
fn launcher_from_the_diff_view_reroots_into_review_session_within_three_keystrokes() {
    let tmp = repo_with_two_feature_branches();
    let dir = tmp.path();
    let mut app = app_for(dir);
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    assert_eq!(app.mode, Mode::Normal, "starts in the diff view");

    // Journey B, diff-view origin, exactly three keystrokes: `R`, `j`, `Enter`.
    press(&mut app, &keymap, &mut pending, KeyCode::Char('R'));
    assert!(matches!(
        app.mode,
        Mode::ReviewLauncher {
            tab: LauncherTab::Branches,
            origin: ModeOrigin::Normal,
            ..
        }
    ));
    assert_eq!(
        app.launcher_branches
            .iter()
            .map(|b| b.name.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha", "zulu"],
        "for-each-ref lists local branches alphabetically"
    );
    press(&mut app, &keymap, &mut pending, KeyCode::Char('j')); // move onto `zulu`
    dump_frame(
        "launcher open over the diff view, cursor on zulu",
        &app,
        &keymap,
    );
    press(&mut app, &keymap, &mut pending, KeyCode::Enter); // confirm on `zulu`

    assert_eq!(
        app.mode,
        Mode::Normal,
        "a Normal-origin launcher restores to Normal after a successful start, got {:?}",
        app.mode
    );
    assert!(
        matches!(&app.target, DiffTarget::Review { branch, .. } if branch == "zulu"),
        "target must be Review{{branch: zulu}} (the `j`-selected branch), got {:?}",
        app.target
    );
    let content = render_frame(&app, &keymap);
    assert!(
        content.contains("REVIEWING zulu"),
        "banner must show the branch under review: {content}"
    );
    dump_frame(
        "landed in the review session from the diff view",
        &app,
        &keymap,
    );

    drop(tmp);
}

// -- Scenarios: failure path (FR-8) -------------------------------------------

#[test]
fn launcher_surfaces_a_worktree_add_failure_and_restores_the_origin_without_mutating_state() {
    let tmp = repo_with_feature_checked_out_elsewhere();
    let dir = tmp.path();
    let mut app = app_for(dir);
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    press(&mut app, &keymap, &mut pending, KeyCode::Char('`'));
    press(&mut app, &keymap, &mut pending, KeyCode::Char('R'));
    assert!(matches!(
        app.mode,
        Mode::ReviewLauncher {
            tab: LauncherTab::Branches,
            ..
        }
    ));

    let before_root = app.repo_root.clone();
    let before_target = app.target.clone();

    press(&mut app, &keymap, &mut pending, KeyCode::Enter); // confirm on `feature`, which fails

    assert!(
        matches!(app.mode, Mode::Panel { .. }),
        "a failed start restores the launcher's Panel origin, got {:?}",
        app.mode
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

// -- Scenarios: single-in-flight guard (FR-8 parity) -------------------------

#[test]
fn confirm_is_rejected_while_a_remote_op_is_in_flight() {
    let tmp = repo_with_feature_branch();
    let dir = tmp.path();
    let mut app = app_for(dir);
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    press(&mut app, &keymap, &mut pending, KeyCode::Char('`'));
    press(&mut app, &keymap, &mut pending, KeyCode::Char('R'));
    assert!(matches!(
        app.mode,
        Mode::ReviewLauncher {
            tab: LauncherTab::Branches,
            ..
        }
    ));

    // Mark a fetch as running, the same way `App::request_remote_op` would,
    // without actually spawning one — mirrors `app_tests.rs`'s
    // `in_flight_fetch` precedent for the switcher's identical guard.
    app.git_op = Some(super::app::InFlightGitOp {
        id: super::background::TaskId(1),
        kind: super::app::GitOpKind::Remote(crate::git::RemoteOp::Fetch),
        command_line: crate::git::RemoteOp::Fetch.command_line(),
    });

    press(&mut app, &keymap, &mut pending, KeyCode::Enter); // confirm on `feature`, blocked

    assert!(
        matches!(
            app.mode,
            Mode::ReviewLauncher {
                tab: LauncherTab::Branches,
                ..
            }
        ),
        "the launcher stays open while a remote op is running, got {:?}",
        app.mode
    );
    assert!(
        app.status_message
            .as_deref()
            .is_some_and(|m| m.contains("is running")),
        "got {:?}",
        app.status_message
    );

    // No managed worktree was created while blocked.
    let runner = GitRunner::discover_in(dir).unwrap();
    let common_dir = runner.git_common_dir().unwrap();
    let managed = common_dir.join("redquill").join("worktrees");
    assert!(
        !managed.exists() || std::fs::read_dir(&managed).unwrap().next().is_none(),
        "no worktree must have been created while a remote op is in flight"
    );

    drop(tmp);
}

// -- Scenarios: close-without-start origin restore (FR-5, FR-9) -------------

#[test]
fn esc_without_confirming_restores_the_normal_origin() {
    let tmp = repo_with_feature_branch();
    let dir = tmp.path();
    let mut app = app_for(dir);
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    press(&mut app, &keymap, &mut pending, KeyCode::Char('R'));
    assert!(matches!(
        app.mode,
        Mode::ReviewLauncher {
            origin: ModeOrigin::Normal,
            ..
        }
    ));
    press(&mut app, &keymap, &mut pending, KeyCode::Esc);
    assert_eq!(app.mode, Mode::Normal);
    assert!(matches!(app.target, DiffTarget::WorkingTree));

    drop(tmp);
}

#[test]
fn esc_without_confirming_restores_the_panel_origin_cursor_and_tab() {
    let tmp = repo_with_feature_branch();
    let dir = tmp.path();
    let mut app = app_for(dir);
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    press(&mut app, &keymap, &mut pending, KeyCode::Char('`')); // focus panel
    press(&mut app, &keymap, &mut pending, KeyCode::Tab); // switch to History
    assert_eq!(
        app.mode,
        Mode::Panel {
            cursor: 0,
            tab: PanelTab::History,
        }
    );
    press(&mut app, &keymap, &mut pending, KeyCode::Char('R'));
    assert!(matches!(
        app.mode,
        Mode::ReviewLauncher {
            origin: ModeOrigin::Panel {
                cursor: 0,
                tab: PanelTab::History,
            },
            ..
        }
    ));
    press(&mut app, &keymap, &mut pending, KeyCode::Esc);
    assert_eq!(
        app.mode,
        Mode::Panel {
            cursor: 0,
            tab: PanelTab::History,
        },
        "Esc without confirming restores the exact panel cursor/tab, got {:?}",
        app.mode
    );
    assert!(matches!(app.target, DiffTarget::WorkingTree));

    drop(tmp);
}

// -- Scenarios: Commits tab ahead-of-base data (FR-11) -----------------------

#[test]
fn launcher_commits_tab_lists_commits_ahead_of_base_newest_first() {
    let tmp = repo_on_feature_with_two_commits_ahead_of_main();
    let dir = tmp.path();
    let mut app = app_for(dir);
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    press(&mut app, &keymap, &mut pending, KeyCode::Char('R'));
    assert!(matches!(
        app.mode,
        Mode::ReviewLauncher {
            tab: LauncherTab::Branches,
            ..
        }
    ));
    press(&mut app, &keymap, &mut pending, KeyCode::Tab); // -> Commits
    assert!(matches!(
        app.mode,
        Mode::ReviewLauncher {
            tab: LauncherTab::Commits,
            ..
        }
    ));
    drain_launcher_commits(&mut app);

    assert_eq!(
        app.launcher_commits
            .iter()
            .map(|c| c.subject.as_str())
            .collect::<Vec<_>>(),
        vec!["feature commit two", "feature commit one"],
        "ahead-of-base, newest first"
    );

    let content = render_frame(&app, &keymap);
    assert!(content.contains("feature commit two"));
    dump_frame(
        "Commits tab populated with ahead-of-base commits",
        &app,
        &keymap,
    );

    drop(tmp);
}

// -- Scenarios: Journey A — R, Enter opens the newest commit, Esc returns ----

#[test]
fn launcher_commits_tab_enter_opens_the_newest_commit_and_esc_restores_the_prior_view() {
    let tmp = repo_on_feature_with_a_fresh_commit_ahead_of_main();
    let dir = tmp.path();
    let mut app = app_for(dir);
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    // The launcher's first-ever open of a process lands on Branches (FR-6);
    // Journey A's 2-keystroke claim is the steady state once the launcher
    // already remembers the Commits tab from earlier use this session — a
    // legitimate precondition under FR-6's process-lifetime tab memory, not
    // a claim about the very first `R` ever pressed.
    app.last_launcher_tab = LauncherTab::Commits;

    assert_eq!(app.mode, Mode::Normal, "starts in the diff view");

    // Keystroke 1: `R` opens the launcher straight onto Commits.
    press(&mut app, &keymap, &mut pending, KeyCode::Char('R'));
    assert!(matches!(
        app.mode,
        Mode::ReviewLauncher {
            tab: LauncherTab::Commits,
            origin: ModeOrigin::Normal,
            ..
        }
    ));
    drain_launcher_commits(&mut app);
    assert_eq!(
        app.launcher_commits
            .iter()
            .map(|c| c.subject.as_str())
            .collect::<Vec<_>>(),
        vec!["agent: fix the thing"]
    );
    dump_frame(
        "Journey A: R opens the launcher on Commits, fresh commit listed",
        &app,
        &keymap,
    );

    // Keystroke 2: `Enter` opens the newest (only) commit.
    press(&mut app, &keymap, &mut pending, KeyCode::Enter);
    assert_eq!(
        app.mode,
        Mode::Normal,
        "opening a commit view returns focus to the diff"
    );
    assert!(app.viewing_commit(), "a commit view must now be open");
    assert!(
        matches!(&app.target, DiffTarget::Commit(_)),
        "got {:?}",
        app.target
    );
    let opened = app
        .active_commit
        .clone()
        .expect("commit header must be set");
    assert_eq!(opened.subject, "agent: fix the thing");
    let commit_view = render_frame(&app, &keymap);
    assert!(commit_view.contains("a.rs"), "the commit's diff must show");
    dump_frame(
        "Journey A: after Enter, the fresh commit's diff",
        &app,
        &keymap,
    );

    // `Esc` returns to the exact prior view.
    press(&mut app, &keymap, &mut pending, KeyCode::Esc);
    assert!(!app.viewing_commit());
    assert_eq!(app.mode, Mode::Normal);
    assert!(matches!(app.target, DiffTarget::WorkingTree));
    dump_frame("Journey A: Esc restores the prior view", &app, &keymap);

    drop(tmp);
}

// -- Scenarios: Journey C — empty state, `a` expands -------------------------

#[test]
fn launcher_commits_tab_shows_the_empty_state_hint_and_a_expands_to_all_commits() {
    let tmp = repo_on_the_base_branch_with_no_other_history();
    let dir = tmp.path();
    let mut app = app_for(dir);
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    press(&mut app, &keymap, &mut pending, KeyCode::Char('R'));
    press(&mut app, &keymap, &mut pending, KeyCode::Tab); // -> Commits
    assert!(matches!(
        app.mode,
        Mode::ReviewLauncher {
            tab: LauncherTab::Commits,
            ..
        }
    ));
    drain_launcher_commits(&mut app);
    assert!(
        app.launcher_commits.is_empty(),
        "the current branch IS the base — nothing ahead of itself"
    );

    let content = render_frame(&app, &keymap);
    assert!(
        content.contains("no commits ahead of base"),
        "must show the empty-state hint:\n{content}"
    );
    assert!(
        content.contains("all commits"),
        "the hint (and footer) must name the toggle:\n{content}"
    );
    dump_frame(
        "Journey C: Commits tab empty-state hint on the base branch",
        &app,
        &keymap,
    );

    // `a` expands to the full recent-HEAD log.
    press(&mut app, &keymap, &mut pending, KeyCode::Char('a'));
    assert!(app.launcher_all_commits);
    drain_history(&mut app);

    let expanded = render_frame(&app, &keymap);
    assert!(
        expanded.contains("initial commit"),
        "the full log must show the base's own commit:\n{expanded}"
    );
    dump_frame("Journey C: a expands to the full log", &app, &keymap);

    drop(tmp);
}

// -- Scenarios: commit peek during an active review session (FR-14) ---------

#[test]
fn launcher_commits_tab_enter_opens_a_commit_during_an_active_review_session() {
    let tmp = repo_with_feature_branch();
    let dir = tmp.path();
    let mut app = app_for(dir);
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    // Start a branch review of `feature` first (Journey B's own path).
    press(&mut app, &keymap, &mut pending, KeyCode::Char('R'));
    press(&mut app, &keymap, &mut pending, KeyCode::Enter); // confirm on `feature`
    assert!(matches!(app.target, DiffTarget::Review { .. }));
    assert!(app.in_review_session());
    let review_target = app.target.clone();

    // Peek at a commit from the Commits tab while the session stays active.
    press(&mut app, &keymap, &mut pending, KeyCode::Char('R'));
    press(&mut app, &keymap, &mut pending, KeyCode::Tab); // -> Commits
    drain_launcher_commits(&mut app);
    assert_eq!(
        app.launcher_commits
            .iter()
            .map(|c| c.subject.as_str())
            .collect::<Vec<_>>(),
        vec!["feature tip"],
        "ahead-of-base from inside the reviewed worktree's own history"
    );

    press(&mut app, &keymap, &mut pending, KeyCode::Enter); // open the commit

    assert!(app.viewing_commit(), "a commit view must be open");
    assert!(matches!(&app.target, DiffTarget::Commit(_)));
    dump_frame(
        "peeking a commit mid-review-session via the Commits tab",
        &app,
        &keymap,
    );

    // `Esc` restores the review session's own suspended diff view exactly —
    // not just any `Mode::Normal`, the specific `DiffTarget::Review` the
    // session was rerooted onto.
    press(&mut app, &keymap, &mut pending, KeyCode::Esc);
    assert!(!app.viewing_commit());
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(
        app.target, review_target,
        "Esc must restore the review session, not just Normal mode"
    );
    assert!(
        app.in_review_session(),
        "the review session itself must be untouched by the peek"
    );

    drop(tmp);
}
