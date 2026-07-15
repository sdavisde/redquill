//! Real-git integration tests for spec 06 Unit 1 (the `gp` fuzzy file finder
//! and the read-only file view), driven through the actual key-dispatch
//! pipeline against a throwaway repository built in a tempdir — never the
//! host repo. Mirrors `history_integration_tests.rs`'s conventions exactly
//! (same fixture/dispatch helpers), since this task builds directly on that
//! suspend/restore pattern.
//!
//! This suite is also this task's substitute for the "drive the real TUI"
//! acceptance journey: `enable_raw_mode` fails in this sandbox (no
//! controlling TTY), so a live terminal recording isn't possible — but every
//! step of the journey (`gp` → type a partial name → the ranked list narrows
//! → `Enter` opens an un-diffed file → scroll → `Esc` back with position
//! intact) is exercised here through the *real* dispatch_key/background-poll
//! pipeline against a *real* git repository, which is stronger evidence than
//! a fake-backed unit test even without a terminal recording.

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

/// A repo with a diffed file (`a.txt`, uncommitted edit) and an *un-diffed*
/// file (`docs/notes.md`) the finder should be able to jump straight to.
fn repo_with_a_diff_and_an_undiffed_file() -> tempfile::TempDir {
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
        "line one\nline two\nline three\nline four\nline five\n",
    );
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "initial"]);
    write(dir, "a.txt", "one\ntwo\n");
    tmp
}

fn app_for(dir: &Path) -> App {
    let runner = GitRunner::discover_in(dir).expect("discover repo");
    let snapshot = build_review(&runner, &DiffTarget::WorkingTree).expect("build review");
    let mut app = App::with_git(snapshot, DiffTarget::WorkingTree, Box::new(runner.clone()));
    app.set_repo_root(runner.root().to_path_buf());
    app
}

fn press(app: &mut App, keymap: &Keymap, pending: &mut Option<KeyEvent>, code: KeyCode) {
    dispatch_key(
        app,
        keymap,
        pending,
        &mut None,
        KeyEvent::new(code, KeyModifiers::NONE),
    );
}

fn type_str(app: &mut App, keymap: &Keymap, pending: &mut Option<KeyEvent>, text: &str) {
    for c in text.chars() {
        press(app, keymap, pending, KeyCode::Char(c));
    }
}

/// Drains the background poller until the finder's candidate load lands (a
/// real background thread, since `GitRunner` supports the async path).
/// Panics if nothing completes in time.
fn wait_for_finder_load(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.finder_in_flight.is_some() && Instant::now() < deadline {
        app.poll_finder();
        std::thread::sleep(Duration::from_millis(10));
    }
}

// -- gp opens the finder; candidates load from the real repo -----------------

#[test]
fn gp_opens_the_finder_and_loads_real_repo_candidates() {
    let tmp = repo_with_a_diff_and_an_undiffed_file();
    let mut app = app_for(tmp.path());
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    press(&mut app, &keymap, &mut pending, KeyCode::Char('g'));
    press(&mut app, &keymap, &mut pending, KeyCode::Char('p'));
    assert_eq!(app.mode, Mode::Finder, "gp must open the finder");

    wait_for_finder_load(&mut app);
    let candidates = &app.finder.as_ref().unwrap().candidates;
    let paths: Vec<&str> = candidates.iter().map(|c| c.path.as_str()).collect();
    assert!(paths.contains(&"a.txt"));
    assert!(paths.contains(&"docs/notes.md"));
}

// -- Typing narrows the ranked list; Enter opens an un-diffed file -----------

/// The primary acceptance journey: `gp`, type a partial name, watch the
/// ranked list narrow to the target, `Enter` opens the *un-diffed* file (no
/// hunks in the working-tree diff at all) in the read-only whole-file view,
/// scroll, then `Esc` back to the exact diff position.
#[test]
fn gp_type_partial_name_open_undiffed_file_scroll_esc_back_to_position() {
    let tmp = repo_with_a_diff_and_an_undiffed_file();
    let mut app = app_for(tmp.path());
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    // Establish a distinctive prior position in the diff view.
    app.view.cursor = app.view.max_cursor();
    let prior_target = app.target.clone();
    let prior_cursor = app.view.cursor;

    press(&mut app, &keymap, &mut pending, KeyCode::Char('g'));
    press(&mut app, &keymap, &mut pending, KeyCode::Char('p'));
    wait_for_finder_load(&mut app);

    type_str(&mut app, &keymap, &mut pending, "notes");
    let finder = app.finder.as_ref().unwrap();
    assert_eq!(finder.query, "notes");
    assert_eq!(
        finder.matches.len(),
        1,
        "the query must narrow to exactly the target file"
    );
    assert_eq!(
        finder.candidates[finder.matches[0].index].path,
        "docs/notes.md"
    );

    press(&mut app, &keymap, &mut pending, KeyCode::Enter);

    assert_eq!(app.mode, Mode::Normal, "opening a file returns to Normal");
    assert!(app.finder.is_none(), "the finder must have closed");
    assert_eq!(app.target, DiffTarget::File("docs/notes.md".to_string()));
    assert!(app.viewing_file(), "a file view must be recorded as open");
    assert_eq!(app.view.files.len(), 1);
    assert_eq!(app.view.files[0].path, "docs/notes.md");

    // Scroll around inside the file view — must not corrupt the suspended
    // diff-view state.
    press(&mut app, &keymap, &mut pending, KeyCode::Char('G')); // jump to bottom
    press(&mut app, &keymap, &mut pending, KeyCode::Char('j'));
    press(&mut app, &keymap, &mut pending, KeyCode::Char('k'));

    press(&mut app, &keymap, &mut pending, KeyCode::Esc);

    assert_eq!(app.mode, Mode::Normal, "Esc returns to Normal");
    assert!(!app.viewing_file(), "the file view must be closed");
    assert_eq!(
        app.target, prior_target,
        "prior diff target must be restored"
    );
    assert_eq!(
        app.view.cursor, prior_cursor,
        "prior cursor position must be restored"
    );
}

// -- Esc from the finder without selecting closes losslessly -----------------

#[test]
fn esc_from_the_finder_without_selecting_returns_unchanged() {
    let tmp = repo_with_a_diff_and_an_undiffed_file();
    let mut app = app_for(tmp.path());
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    app.view.cursor = app.view.max_cursor();
    let prior_target = app.target.clone();
    let prior_cursor = app.view.cursor;

    press(&mut app, &keymap, &mut pending, KeyCode::Char('g'));
    press(&mut app, &keymap, &mut pending, KeyCode::Char('p'));
    wait_for_finder_load(&mut app);
    type_str(&mut app, &keymap, &mut pending, "notes");

    press(&mut app, &keymap, &mut pending, KeyCode::Esc);

    assert_eq!(app.mode, Mode::Normal);
    assert!(app.finder.is_none());
    assert_eq!(app.target, prior_target, "target must be unchanged");
    assert_eq!(app.view.cursor, prior_cursor, "cursor must be unchanged");
    assert!(!app.viewing_file(), "no file view was ever opened");
}

// -- Capability gating in the file view (mirrors the commit-view proof) -----

#[test]
fn file_view_hides_and_disarms_staging_and_code_intel_keys() {
    let tmp = repo_with_a_diff_and_an_undiffed_file();
    let mut app = app_for(tmp.path());
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    press(&mut app, &keymap, &mut pending, KeyCode::Char('g'));
    press(&mut app, &keymap, &mut pending, KeyCode::Char('p'));
    wait_for_finder_load(&mut app);
    type_str(&mut app, &keymap, &mut pending, "notes");
    press(&mut app, &keymap, &mut pending, KeyCode::Enter);
    assert!(matches!(app.target, DiffTarget::File(_)));

    let staging_allowed = app.target.staging_mode() != crate::git::StagingMode::ReadOnly;
    let code_intel_allowed = app.target.supports_code_intel();
    assert!(!staging_allowed);
    assert!(!code_intel_allowed);

    let entries = footer::build_hints(
        app.mode,
        footer::FooterFlags {
            staging_allowed,
            code_intel_allowed,
            push_publishes: app.push_publishes(),
            viewing_commit: app.viewing_commit(),
            help_open: app.help_open,
            project_search_focus: app.project_search_focus(),
        },
        None,
        &keymap,
    );
    assert!(
        !entries.iter().any(|e| e.label.contains("stage")),
        "no staging hint may appear in the file-view footer: {entries:?}"
    );
    for action in [
        Action::ToggleStage,
        Action::StageFile,
        Action::GotoDefinition,
        Action::GotoReferences,
        Action::Hover,
    ] {
        assert!(help::binding_hidden(
            action,
            staging_allowed,
            code_intel_allowed
        ));
    }

    // Inert: pressing space (ToggleStage) degrades to a footer message.
    press(&mut app, &keymap, &mut pending, KeyCode::Char(' '));
    assert_eq!(app.status_message.as_deref(), Some("read-only diff target"));
}

// -- Annotations remain fully functional in the file view --------------------

#[test]
fn file_view_annotations_are_fully_functional() {
    let tmp = repo_with_a_diff_and_an_undiffed_file();
    let mut app = app_for(tmp.path());
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    press(&mut app, &keymap, &mut pending, KeyCode::Char('g'));
    press(&mut app, &keymap, &mut pending, KeyCode::Char('p'));
    wait_for_finder_load(&mut app);
    type_str(&mut app, &keymap, &mut pending, "notes");
    press(&mut app, &keymap, &mut pending, KeyCode::Enter);

    assert!(matches!(
        app.view.rows.first(),
        Some(Row::FileHeader { .. })
    ));
    // Land on a line row (row 2: header, hunk header, then line rows).
    press(&mut app, &keymap, &mut pending, KeyCode::Char('j'));
    press(&mut app, &keymap, &mut pending, KeyCode::Char('j'));
    assert!(matches!(app.view.rows[app.view.cursor], Row::Line(_)));

    press(&mut app, &keymap, &mut pending, KeyCode::Char('c'));
    assert_eq!(
        app.mode,
        Mode::Compose,
        "c must open Compose in a file view"
    );
    type_str(&mut app, &keymap, &mut pending, "worth a second look");
    press(&mut app, &keymap, &mut pending, KeyCode::Enter);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.annotations.len(), 1, "the annotation must be recorded");
}

// -- File-view annotations serialize with `(=)` and navigate back from the
// annotation list panel (spec 06 Unit 3) -------------------------------------

#[test]
fn file_view_annotation_serializes_with_equals_marker_and_navigates_back_from_the_list() {
    let tmp = repo_with_a_diff_and_an_undiffed_file();
    let mut app = app_for(tmp.path());
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    press(&mut app, &keymap, &mut pending, KeyCode::Char('g'));
    press(&mut app, &keymap, &mut pending, KeyCode::Char('p'));
    wait_for_finder_load(&mut app);
    type_str(&mut app, &keymap, &mut pending, "notes");
    press(&mut app, &keymap, &mut pending, KeyCode::Enter);

    // Land on the file's first line row (row 2: header, hunk header, then
    // line rows) and annotate it.
    press(&mut app, &keymap, &mut pending, KeyCode::Char('j'));
    press(&mut app, &keymap, &mut pending, KeyCode::Char('j'));
    let Row::Line(line) = &app.view.rows[app.view.cursor] else {
        panic!("expected a line row");
    };
    assert_eq!(line.new_line, Some(1));

    press(&mut app, &keymap, &mut pending, KeyCode::Char('c'));
    type_str(&mut app, &keymap, &mut pending, "worth a second look");
    press(&mut app, &keymap, &mut pending, KeyCode::Enter);
    assert_eq!(app.annotations.len(), 1);

    let annotation = app.annotations.iter().next().unwrap();
    assert_eq!(
        annotation.target,
        crate::annotate::Target::worktree_line("docs/notes.md", 1),
        "a file-view annotation must target the (=) worktree-line form"
    );
    assert_eq!(
        annotation.source,
        crate::annotate::Source::WorkingTree,
        "a file-view annotation groups with the working-tree Reviewing: group"
    );
    let rendered = crate::annotate::render_markdown(&app.annotations);
    assert_eq!(
        rendered,
        "## docs/notes.md:1 (=)\n\n[issue] worth a second look\n"
    );

    // Esc back to the diff, open the annotation list panel, and jump to the
    // `(=)` entry: it must reopen the file view at the annotated line rather
    // than silently no-op (the path isn't in the diff view's loaded files).
    press(&mut app, &keymap, &mut pending, KeyCode::Esc);
    assert!(!app.viewing_file(), "back in the diff view");

    press(&mut app, &keymap, &mut pending, KeyCode::Char('a'));
    assert_eq!(app.mode, Mode::List);
    press(&mut app, &keymap, &mut pending, KeyCode::Enter);

    assert_eq!(app.mode, Mode::Normal);
    assert!(
        app.viewing_file(),
        "jumping to a (=) annotation must reopen the file view"
    );
    assert_eq!(app.target, DiffTarget::File("docs/notes.md".to_string()));
    let Row::Line(line) = &app.view.rows[app.view.cursor] else {
        panic!("expected cursor on a line row after the jump");
    };
    assert_eq!(line.new_line, Some(1));
}
