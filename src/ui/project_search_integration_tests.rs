//! Real-git integration tests for spec 06 Unit 2 (`g/` Project Search),
//! driven through the actual key-dispatch pipeline against a throwaway
//! repository built in a tempdir — never the host repo. Mirrors
//! `file_finder_integration_tests.rs`'s conventions exactly (same
//! fixture/dispatch helpers), since this task builds directly on that
//! suspend/restore pattern.
//!
//! This suite is also this task's substitute for the "drive the real TUI"
//! acceptance journey: `enable_raw_mode` fails in this sandbox (no
//! controlling TTY), so a live terminal recording isn't possible — but every
//! step of the primary journey (`g/` an identifier seen in a diff, watch
//! results stream, refine + toggle whole-word, open a hit in a file the diff
//! doesn't touch, `Esc` `Esc` back to the exact diff position) is exercised
//! here through the *real* `dispatch_key`/background-poll pipeline against a
//! *real* git repository.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::stage_ops::build_review;
use super::*;
use crate::git::{DiffTarget, GitRunner};

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
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

/// A repo with a diffed file (`a.txt`, uncommitted edit introducing the
/// identifier `needle_fn`) and an *untouched* file (`docs/notes.md`, also
/// containing `needle_fn`) that Project Search should be able to jump
/// straight to — the "grep a reference seen in the diff, open it in a file
/// the diff doesn't touch" journey.
fn repo_with_a_diff_and_an_undiffed_match() -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.email", "test@redquill.invalid"]);
    git(dir, &["config", "user.name", "redquill test"]);
    git(dir, &["config", "core.hooksPath", ".git/hooks"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
    write(dir, "a.txt", "one\n");
    write(
        dir,
        "docs/notes.md",
        "line one\ncall needle_fn(1) here\nline three\n",
    );
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "initial"]);
    write(dir, "a.txt", "one\nneedle_fn(2)\n");
    tmp
}

fn app_for(dir: &Path) -> App {
    let runner = GitRunner::discover_in(dir).expect("discover repo");
    let snapshot = build_review(&runner, &DiffTarget::WorkingTree).expect("build review");
    let mut app = App::with_git(snapshot, DiffTarget::WorkingTree, Box::new(runner.clone()));
    app.set_repo_root(runner.root().to_path_buf());
    app
}

fn press_key(app: &mut App, keymap: &Keymap, pending: &mut Option<KeyEvent>, key: KeyEvent) {
    dispatch_key(app, keymap, pending, &mut None, key);
}

fn press(app: &mut App, keymap: &Keymap, pending: &mut Option<KeyEvent>, code: KeyCode) {
    press_key(
        app,
        keymap,
        pending,
        KeyEvent::new(code, KeyModifiers::NONE),
    );
}

fn press_alt(app: &mut App, keymap: &Keymap, pending: &mut Option<KeyEvent>, c: char) {
    press_key(
        app,
        keymap,
        pending,
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT),
    );
}

fn type_str(app: &mut App, keymap: &Keymap, pending: &mut Option<KeyEvent>, text: &str) {
    for c in text.chars() {
        press(app, keymap, pending, KeyCode::Char(c));
    }
}

/// Drains Project Search's background scan (debounce + real scan thread)
/// until a summary lands, via the real per-tick poll — bounded real-time
/// wait, mirroring `file_finder_integration_tests.rs`'s `wait_for_finder_load`.
/// Panics if nothing completes in time.
fn wait_for_project_search_summary(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.poll_project_search();
        let has_summary = app
            .project_search
            .as_ref()
            .is_some_and(|s| s.summary.is_some());
        if has_summary || Instant::now() >= deadline {
            assert!(has_summary, "scan did not complete within the timeout");
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

// -- g/ opens Project Search; live query streams real results ---------------

#[test]
fn g_slash_opens_project_search_and_streams_real_scan_results() {
    let tmp = repo_with_a_diff_and_an_undiffed_match();
    let mut app = app_for(tmp.path());
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    press(&mut app, &keymap, &mut pending, KeyCode::Char('g'));
    press(&mut app, &keymap, &mut pending, KeyCode::Char('/'));
    assert_eq!(app.mode, Mode::ProjectSearch, "g/ must open Project Search");

    type_str(&mut app, &keymap, &mut pending, "needle_fn");
    wait_for_project_search_summary(&mut app);

    let state = app.project_search.as_ref().unwrap();
    assert_eq!(state.query, "needle_fn");
    let paths: Vec<&str> = state.groups.iter().map(|g| g.path.as_str()).collect();
    assert!(
        paths.contains(&"a.txt"),
        "must find the hit in the diffed file"
    );
    assert!(
        paths.contains(&"docs/notes.md"),
        "must find the hit in the untouched file too — search always scans the worktree on disk"
    );
}

// -- Primary journey: diff -> g/ -> refine + toggle -> open a hit -> Esc Esc back

#[test]
fn primary_journey_diff_search_refine_toggle_open_hit_esc_esc_back_to_position() {
    let tmp = repo_with_a_diff_and_an_undiffed_match();
    let mut app = app_for(tmp.path());
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    // Establish a distinctive prior position in the diff view.
    app.view.cursor = app.view.max_cursor();
    let prior_target = app.target.clone();
    let prior_cursor = app.view.cursor;

    // Review a diff, then `g/` an identifier seen in it.
    press(&mut app, &keymap, &mut pending, KeyCode::Char('g'));
    press(&mut app, &keymap, &mut pending, KeyCode::Char('/'));
    type_str(&mut app, &keymap, &mut pending, "needle_fn");
    wait_for_project_search_summary(&mut app);
    assert_eq!(
        app.project_search.as_ref().unwrap().groups.len(),
        2,
        "results must stream in from both files"
    );

    // Refine the query (still matches both) and toggle whole-word — a
    // toggle change re-triggers the debounced scan the same way typing does.
    press_alt(&mut app, &keymap, &mut pending, 'w');
    assert!(app.project_search.as_ref().unwrap().whole_word);
    wait_for_project_search_summary(&mut app);
    assert_eq!(
        app.project_search.as_ref().unwrap().groups.len(),
        2,
        "needle_fn is a whole word in both files, so toggling whole-word keeps both matches"
    );

    // Move the selection to the hit in the file the diff doesn't touch, and
    // open it.
    let notes_index = app
        .project_search
        .as_ref()
        .unwrap()
        .groups
        .iter()
        .position(|g| g.path == "docs/notes.md")
        .expect("docs/notes.md must be among the groups");
    // Cursor starts at 0 (first group's first hit); move down to the first
    // hit of whichever group comes second if notes.md isn't first.
    if notes_index != 0 {
        press(&mut app, &keymap, &mut pending, KeyCode::Down);
    }
    let selected = app.selected_project_search_hit().unwrap();
    assert_eq!(selected.path, "docs/notes.md");

    press(&mut app, &keymap, &mut pending, KeyCode::Enter);

    assert_eq!(app.mode, Mode::Normal, "opening a hit lands in Normal");
    assert_eq!(app.target, DiffTarget::File("docs/notes.md".to_string()));
    assert!(app.viewing_file());
    let Row::Line(line) = &app.view.rows[app.view.cursor] else {
        panic!("cursor must land on a Line row");
    };
    assert!(
        line.content.contains("needle_fn"),
        "cursor must land on the matched line"
    );

    // First Esc: back to Project Search, state fully intact.
    press(&mut app, &keymap, &mut pending, KeyCode::Esc);
    assert_eq!(
        app.mode,
        Mode::ProjectSearch,
        "Esc from the file view returns to Project Search, not the diff"
    );
    assert!(app.project_search.is_some());
    assert_eq!(app.project_search.as_ref().unwrap().query, "needle_fn");
    assert!(app.project_search.as_ref().unwrap().whole_word);
    assert_eq!(app.project_search.as_ref().unwrap().groups.len(), 2);

    // Second Esc: back to the exact prior diff position.
    press(&mut app, &keymap, &mut pending, KeyCode::Esc);
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.project_search.is_none());
    assert_eq!(
        app.target, prior_target,
        "prior diff target must be restored"
    );
    assert_eq!(
        app.view.cursor, prior_cursor,
        "prior cursor position must be restored"
    );
}

// -- Esc from Project Search without ever opening a hit ----------------------

#[test]
fn esc_from_project_search_without_opening_a_hit_returns_to_the_exact_prior_position() {
    let tmp = repo_with_a_diff_and_an_undiffed_match();
    let mut app = app_for(tmp.path());
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    app.view.cursor = app.view.max_cursor();
    let prior_target = app.target.clone();
    let prior_cursor = app.view.cursor;

    press(&mut app, &keymap, &mut pending, KeyCode::Char('g'));
    press(&mut app, &keymap, &mut pending, KeyCode::Char('/'));
    type_str(&mut app, &keymap, &mut pending, "needle_fn");
    wait_for_project_search_summary(&mut app);

    press(&mut app, &keymap, &mut pending, KeyCode::Esc);

    assert_eq!(app.mode, Mode::Normal);
    assert!(app.project_search.is_none());
    assert_eq!(app.target, prior_target);
    assert_eq!(app.view.cursor, prior_cursor);
}

// -- Invalid regex shows an inline error without wiping prior results -------

#[test]
fn invalid_regex_mid_session_shows_an_error_and_keeps_prior_results_on_screen() {
    let tmp = repo_with_a_diff_and_an_undiffed_match();
    let mut app = app_for(tmp.path());
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    press(&mut app, &keymap, &mut pending, KeyCode::Char('g'));
    press(&mut app, &keymap, &mut pending, KeyCode::Char('/'));
    type_str(&mut app, &keymap, &mut pending, "needle_fn");
    wait_for_project_search_summary(&mut app);
    assert!(app.project_search.as_ref().unwrap().error.is_none());
    assert_eq!(app.project_search.as_ref().unwrap().groups.len(), 2);

    // Turn the query into an unbalanced-paren regex — grep-regex must reject
    // it without touching the prior good results.
    type_str(&mut app, &keymap, &mut pending, "(");
    // Below-length/invalid firing still runs through the same debounce; poll
    // until an error appears (bounded).
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        app.poll_project_search();
        if app.project_search.as_ref().unwrap().error.is_some() || Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    let state = app.project_search.as_ref().unwrap();
    assert!(
        state.error.is_some(),
        "an unbalanced paren must be reported"
    );
    assert_eq!(
        state.groups.len(),
        2,
        "the prior good results must still be showing"
    );
}
