//! Real-git integration tests for spec 05 Unit 3 (git panel History tab and
//! commit view), driven through the actual key-dispatch pipeline
//! (`` ` `` -> `Tab` -> `j`/`k` -> `Enter` -> `Esc`/`q`) against throwaway
//! repositories built in tempdirs, per this repo's testing convention (see
//! CLAUDE.md / `docs/rust-best-practices.md`) — never the host repo.
//!
//! Lives beside `commit_integration_tests.rs`/`git_switch_integration_tests.rs`
//! for the same reason those do: `dispatch_key` and the panel's key handling
//! are crate-internal by design, so a `tests/*.rs` binary could not drive keys
//! into the panel; living here keeps the coverage genuinely end-to-end (real
//! `git log`/`git diff` subprocesses through the real background poller, real
//! key dispatch) without widening the public API for a test's sake.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tempfile::TempDir;

use super::app::PanelTab;
use super::stage_ops::build_review;
use super::*;
use crate::git::{DiffTarget, GitRunner};

// -- Repo/dispatch fixtures (mirrors commit_integration_tests.rs) ----------

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

fn write(dir: &Path, rel: &str, contents: &str) {
    std::fs::write(dir.join(rel), contents).unwrap();
}

/// A repo with three commits touching `a.txt`, then a clean working tree —
/// the "agent already committed" scenario this whole spec targets. Identity
/// and hooks path are pinned locally so no host git config leaks in.
fn repo_with_history() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.email", "test@redquill.invalid"]);
    git(dir, &["config", "user.name", "redquill test"]);
    git(dir, &["config", "core.hooksPath", ".git/hooks"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
    write(dir, "a.txt", "one\n");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "first commit"]);
    write(dir, "a.txt", "one\ntwo\n");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "second commit"]);
    write(dir, "a.txt", "one\ntwo\nthree\n");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "third commit"]);
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

/// Drains the background poller until the History tab's first page lands
/// (a real background thread this time, unlike the fake-backed unit tests in
/// `history_tests.rs`). Panics if nothing completes in time.
fn wait_for_history(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.history.is_empty() && app.history_in_flight.is_some() && Instant::now() < deadline {
        app.poll_history();
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Presses `j` until the cursor lands on a `Row::Line`, bounded by the row
/// count so a view with no line rows (or a cursor that stops advancing)
/// can't spin the test forever — mirrors the bounded pattern
/// `commit_view_annotations_are_fully_functional` already uses inline.
fn advance_to_line_row(app: &mut App, keymap: &Keymap, pending: &mut Option<KeyEvent>) {
    for _ in 0..=app.view.rows.len() {
        if matches!(app.view.rows.get(app.view.cursor), Some(Row::Line(_))) {
            return;
        }
        let before = app.view.cursor;
        press(app, keymap, pending, KeyCode::Char('j'));
        if app.view.cursor == before {
            return;
        }
    }
}

/// Opens the git panel and switches to the History tab, waiting for the
/// first page to land.
fn open_history_tab(app: &mut App, keymap: &Keymap, pending: &mut Option<KeyEvent>) {
    press(app, keymap, pending, KeyCode::Char('`'));
    assert!(matches!(app.mode, Mode::Panel { .. }), "panel must focus");
    press(app, keymap, pending, KeyCode::Tab);
    assert_eq!(app.panel_tab(), PanelTab::History, "Tab must switch tabs");
    wait_for_history(app);
}

// -- History tab loads real commits -----------------------------------------

#[test]
fn history_tab_loads_the_repos_real_commits_newest_first() {
    let tmp = repo_with_history();
    let mut app = app_for(tmp.path());
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    open_history_tab(&mut app, &keymap, &mut pending);

    assert_eq!(app.history.len(), 3, "all three commits must load");
    assert_eq!(app.history[0].subject, "third commit", "newest first");
    assert_eq!(app.history[1].subject, "second commit");
    assert_eq!(app.history[2].subject, "first commit");
}

// -- Open-commit / return round trip (task 3.5) -----------------------------

/// The core navigation-correctness proof: opening a historical commit and
/// returning restores the exact prior target, cursor, and collapse state —
/// not just "some working-tree view".
#[test]
fn open_commit_then_return_restores_prior_target_cursor_and_collapse() {
    let tmp = repo_with_history();
    let mut app = app_for(tmp.path());
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    // Establish a distinctive prior state: cursor moved, section collapsed.
    app.view.set_collapsed("a.txt", true);
    app.rebuild_rows();
    let prior_target = app.target.clone();
    let prior_cursor = app.view.cursor;
    assert!(
        app.view.is_collapsed("a.txt"),
        "fixture must start collapsed"
    );

    open_history_tab(&mut app, &keymap, &mut pending);
    // Cursor starts on row 0 ("third commit"); open it.
    press(&mut app, &keymap, &mut pending, KeyCode::Enter);

    assert_eq!(app.mode, Mode::Normal, "opening a commit returns to Normal");
    assert!(
        matches!(app.target, DiffTarget::Commit(_)),
        "target must switch to the opened commit"
    );
    assert!(app.active_commit.is_some(), "header metadata must be set");
    assert!(
        app.viewing_commit(),
        "a commit view must be recorded as open"
    );

    // Navigate around inside the commit view — this must not corrupt the
    // suspended state.
    press(&mut app, &keymap, &mut pending, KeyCode::Char('j'));

    press(&mut app, &keymap, &mut pending, KeyCode::Esc);

    assert_eq!(app.mode, Mode::Normal, "Esc returns to Normal");
    assert!(!app.viewing_commit(), "the commit view must be closed");
    assert_eq!(app.active_commit, None, "header metadata must clear");
    assert_eq!(app.target, prior_target, "prior target must be restored");
    assert_eq!(
        app.view.cursor, prior_cursor,
        "prior cursor position must be restored"
    );
    assert!(
        app.view.is_collapsed("a.txt"),
        "prior collapse state must be restored"
    );
}

/// Opening a second, different commit from the History tab without
/// returning in between still lets `Esc` restore the *original* working-tree
/// state — the suspension is captured once, not nested.
#[test]
fn opening_a_second_commit_without_returning_still_restores_the_original_state() {
    let tmp = repo_with_history();
    let mut app = app_for(tmp.path());
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let prior_target = app.target.clone();

    open_history_tab(&mut app, &keymap, &mut pending);
    press(&mut app, &keymap, &mut pending, KeyCode::Enter); // opens "third commit"
    let first_opened = app.target.clone();

    // Back to the panel's History tab, pick a different commit.
    press(&mut app, &keymap, &mut pending, KeyCode::Char('`'));
    assert_eq!(app.panel_tab(), PanelTab::History, "still on History");
    press(&mut app, &keymap, &mut pending, KeyCode::Char('j')); // -> second commit
    press(&mut app, &keymap, &mut pending, KeyCode::Enter);
    let second_opened = app.target.clone();
    assert_ne!(
        first_opened, second_opened,
        "the second Enter must have switched to a different commit"
    );

    press(&mut app, &keymap, &mut pending, KeyCode::Esc);
    assert_eq!(
        app.target, prior_target,
        "Esc must restore the true original state, not the first commit"
    );
}

// -- Capability gating in the commit view (task 3.6) -------------------------

/// Staging is inert (a footer message, no git call) and its keys are absent
/// from both the footer strip and the `?` overlay while a commit view is
/// open — driven by the existing `staging_mode() == ReadOnly` gate (task
/// 1.0), inherited automatically by `DiffTarget::Commit` (task 2.0).
#[test]
fn commit_view_hides_and_disarms_staging_keys() {
    let tmp = repo_with_history();
    let mut app = app_for(tmp.path());
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    open_history_tab(&mut app, &keymap, &mut pending);
    press(&mut app, &keymap, &mut pending, KeyCode::Enter);
    assert!(matches!(app.target, DiffTarget::Commit(_)));

    // Absent from the footer strip.
    let staging_allowed = app.target.staging_mode() != crate::git::StagingMode::ReadOnly;
    let code_intel_allowed = app.target.supports_code_intel();
    assert!(!staging_allowed, "commit target must be read-only");
    let entries = footer::build_hints(
        app.mode,
        footer::FooterFlags {
            staging_allowed,
            code_intel_allowed,
            push_publishes: app.push_publishes(),
            viewing_commit: app.viewing_commit(),
            help_open: app.help_open,
        },
        None,
        &keymap,
    );
    assert!(
        !entries.iter().any(|e| e.label.contains("stage")),
        "no staging hint may appear in the commit-view footer: {entries:?}"
    );
    // Absent from the `?` overlay.
    assert!(help::binding_hidden(
        Action::ToggleStage,
        staging_allowed,
        code_intel_allowed
    ));
    assert!(help::binding_hidden(
        Action::StageFile,
        staging_allowed,
        code_intel_allowed
    ));

    // Inert: pressing space (ToggleStage) does nothing observable to git
    // (degrades to a footer message via the existing read-only guard).
    press(&mut app, &keymap, &mut pending, KeyCode::Char(' '));
    assert_eq!(app.status_message.as_deref(), Some("read-only diff target"));
}

/// No LSP code-intel keys work or show while a commit view is open — the
/// `supports_code_intel() == false` gate (task 1.0/2.0), automatic for a
/// `Commit` target.
#[test]
fn commit_view_hides_and_disarms_code_intel_keys() {
    let tmp = repo_with_history();
    let mut app = app_for(tmp.path());
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    open_history_tab(&mut app, &keymap, &mut pending);
    press(&mut app, &keymap, &mut pending, KeyCode::Enter);

    let code_intel_allowed = app.target.supports_code_intel();
    assert!(
        !code_intel_allowed,
        "commit target must disallow code-intel"
    );
    for action in [
        Action::GotoDefinition,
        Action::GotoReferences,
        Action::Hover,
    ] {
        assert!(help::binding_hidden(action, true, code_intel_allowed));
    }
    // `K` (Hover) must not open the peek overlay.
    press(&mut app, &keymap, &mut pending, KeyCode::Char('K'));
    assert_ne!(app.mode, Mode::Peek, "Hover must be inert in a commit view");
}

/// A commit target never auto-refreshes: `maybe_auto_refresh` bails on
/// `!target.is_live()` before ever touching the background poller.
#[test]
fn commit_view_never_auto_refreshes() {
    let tmp = repo_with_history();
    let mut app = app_for(tmp.path());
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    open_history_tab(&mut app, &keymap, &mut pending);
    press(&mut app, &keymap, &mut pending, KeyCode::Enter);
    assert!(!app.target.is_live(), "commit target must not be live");

    app.maybe_auto_refresh();
    assert!(
        app.refresh_in_flight.is_none(),
        "a commit target must never spawn a background refresh"
    );
}

/// Annotations (line/hunk/file) remain fully functional against a commit
/// view — the Compose/target-derivation path never inspects the diff
/// target, only the cursor row.
#[test]
fn commit_view_annotations_are_fully_functional() {
    let tmp = repo_with_history();
    let mut app = app_for(tmp.path());
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    open_history_tab(&mut app, &keymap, &mut pending);
    press(&mut app, &keymap, &mut pending, KeyCode::Enter);

    // Land on a line row, comment on it.
    while !matches!(app.view.rows.get(app.view.cursor), Some(Row::Line(_))) {
        let before = app.view.cursor;
        press(&mut app, &keymap, &mut pending, KeyCode::Char('j'));
        assert!(app.view.cursor != before || app.view.cursor == app.view.rows.len() - 1);
        if app.view.cursor == app.view.rows.len().saturating_sub(1) {
            break;
        }
    }
    assert!(matches!(app.view.rows[app.view.cursor], Row::Line(_)));

    press(&mut app, &keymap, &mut pending, KeyCode::Char('c'));
    assert_eq!(
        app.mode,
        Mode::Compose,
        "c must open Compose in a commit view"
    );
    for ch in "reviewed against the commit".chars() {
        press(&mut app, &keymap, &mut pending, KeyCode::Char(ch));
    }
    press(&mut app, &keymap, &mut pending, KeyCode::Enter);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(
        app.annotations.len(),
        1,
        "the annotation must have been recorded"
    );
}

/// Spec 05 Unit 4's scripted CLI proof, run end-to-end against a real
/// throwaway repo instead of an interactive terminal (this sandbox has no
/// controlling TTY — see the task's proof artifact for the equivalent
/// manual steps): annotating a line in a commit view and rendering the
/// store produces a `Reviewing: <short-sha>` metadata line naming exactly
/// the opened commit. `repo_with_history()`'s working tree is clean (its own
/// doc: "then a clean working tree" — the dead-end scenario this spec
/// targets), so there is no working-tree diff to additionally annotate here;
/// the working-tree-first / mixed-grouping ordering itself is covered
/// byte-exactly by `annotate::markdown`'s own tests, which don't need a real
/// repo.
#[test]
fn annotating_a_commit_view_records_a_reviewing_line_with_the_short_sha() {
    let tmp = repo_with_history();
    let mut app = app_for(tmp.path());
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    open_history_tab(&mut app, &keymap, &mut pending);
    press(&mut app, &keymap, &mut pending, KeyCode::Enter);
    let opened_short_sha = app
        .active_commit
        .clone()
        .expect("opening a commit sets active_commit")
        .short_sha;

    advance_to_line_row(&mut app, &keymap, &mut pending);
    assert!(
        matches!(app.view.rows.get(app.view.cursor), Some(Row::Line(_))),
        "commit view must have a line row to annotate"
    );
    press(&mut app, &keymap, &mut pending, KeyCode::Char('c'));
    for ch in "reviewed against the commit".chars() {
        press(&mut app, &keymap, &mut pending, KeyCode::Char(ch));
    }
    press(&mut app, &keymap, &mut pending, KeyCode::Enter);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.annotations.len(), 1);

    let rendered = crate::annotate::render_markdown(&app.annotations);
    let expected_reviewing_line = format!("Reviewing: {opened_short_sha}");
    assert!(
        rendered.contains(&expected_reviewing_line),
        "expected {expected_reviewing_line:?} in:\n{rendered}"
    );
    assert_eq!(
        rendered.matches("Reviewing:").count(),
        1,
        "exactly one Reviewing: line for the single non-worktree group:\n{rendered}"
    );
    assert!(
        rendered.starts_with(&expected_reviewing_line),
        "no working-tree group precedes the only (commit) group here:\n{rendered}"
    );
}

/// `q` from a commit view behaves exactly as it does everywhere else: quit,
/// emitting annotations.
#[test]
fn q_from_a_commit_view_quits_and_would_emit_annotations() {
    let tmp = repo_with_history();
    let mut app = app_for(tmp.path());
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    open_history_tab(&mut app, &keymap, &mut pending);
    press(&mut app, &keymap, &mut pending, KeyCode::Enter);

    let flow = dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
    );
    assert!(matches!(flow, Flow::Quit(QuitOutcome::Emit)));
}

// -- Keymap/help drift: the new keys are present (task 3.7) ------------------

/// The `?` overlay's panel-scope section documents `Tab` (switch tab); a
/// regression here would mean a hidden feature (CLAUDE.md: no user-visible
/// action without a `?` entry).
#[test]
fn panel_scope_keymap_documents_the_tab_toggle() {
    let km = Keymap::default_map();
    let row = km
        .bindings()
        .iter()
        .find(|b| b.scope == keymap::Scope::Panel && b.action == Action::TogglePanelTab)
        .expect("TogglePanelTab must be a registered panel-scope binding");
    assert_eq!(row.key_label(), "Tab");
}

// -- Empty-diff welcome state, commit-view wording (spec 05 Unit 5) ---------

/// A repo like `repo_with_history()` plus one more commit on top that
/// introduces no changes (`git commit --allow-empty`) — opening this one
/// from History yields a `DiffTarget::Commit` whose own diff has zero files,
/// the case task 5.3 asks for explicitly ("History-opened commit with an
/// empty diff → target-appropriate wording").
fn repo_with_history_and_a_trailing_empty_commit() -> TempDir {
    let tmp = repo_with_history();
    git(
        tmp.path(),
        &["commit", "-qm", "empty commit", "--allow-empty"],
    );
    tmp
}

/// Opening a commit whose own diff introduces no changes shows the
/// commit-appropriate welcome wording (naming that commit's short SHA), not
/// the working-tree phrase and not a blank buffer.
#[test]
fn opening_a_commit_with_an_empty_diff_shows_commit_appropriate_welcome_wording() {
    let tmp = repo_with_history_and_a_trailing_empty_commit();
    let mut app = app_for(tmp.path());
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    open_history_tab(&mut app, &keymap, &mut pending);
    assert_eq!(app.history[0].subject, "empty commit", "newest first");
    press(&mut app, &keymap, &mut pending, KeyCode::Enter);

    assert!(
        app.view.files.is_empty(),
        "the opened commit must have introduced no changes"
    );
    let short_sha = app
        .active_commit
        .clone()
        .expect("opening a commit sets active_commit")
        .short_sha;

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    let expected = format!("Empty commit diff for {short_sha}");
    assert!(
        content.contains(&expected),
        "expected {expected:?} in:\n{content}"
    );
    assert!(
        !content.contains("No uncommitted changes"),
        "must not reuse the working-tree wording for a commit target"
    );
}
