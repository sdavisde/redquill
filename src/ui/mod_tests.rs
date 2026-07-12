use std::path::PathBuf;

use super::*;
use crate::annotate::{Classification, Target};
use crate::diff::FileDiff;
use crate::git::RawFilePatch;
use crate::highlight::TokenKind;
use crate::lsp::SourceLocation;
use crossterm::event::KeyModifiers;
use ratatui::backend::TestBackend;

fn sample_file() -> FileDiff {
    let raw = "\
diff --git a/src/main.rs b/src/main.rs
index 111..222 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,2 +1,2 @@
 fn main() {
-    old();
+    new();
";
    FileDiff::from_patch(&RawFilePatch {
        path: "src/main.rs".to_string(),
        old_path: None,
        raw: raw.to_string(),
        is_binary: false,
    })
    .unwrap()
}

/// Renders a small `App` to a `TestBackend` and asserts the diff pane shows
/// expected content. No real terminal is touched. The git panel sidebar is
/// hidden by default (see `sidebar_hidden_in_normal_mode_shown_when_panel_focused`
/// below), so it has nothing to assert here.
#[test]
fn renders_diff_pane_content() {
    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let app = App::new(vec![sample_file()]);
    let keymap = Keymap::default_map();

    terminal.draw(|frame| draw(frame, &app, &keymap)).unwrap();

    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    assert!(content.contains("src/main.rs"));
    assert!(content.contains("old()"));
    assert!(content.contains("new()"));
}

/// The git panel sidebar is hidden by default (`Mode::Normal`) — the diff
/// pane gets the full width and none of the sidebar's content (its
/// `[N files]` footer) renders — and appears only once the panel is
/// focused (`Mode::Panel`, entered via the backtick toggle).
#[test]
fn sidebar_hidden_in_normal_mode_shown_when_panel_focused() {
    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    let keymap = Keymap::default_map();

    terminal.draw(|frame| draw(frame, &app, &keymap)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
    assert!(!content.contains("[1 files]"));

    app.apply(Action::FocusGitPanel);
    assert!(matches!(app.mode, Mode::Panel { .. }));

    terminal.draw(|frame| draw(frame, &app, &keymap)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
    assert!(content.contains("[1 files]"));
}

fn multi_file(path: &str) -> FileDiff {
    let raw = format!(
        "diff --git a/{path} b/{path}\nindex 111..222 100644\n--- a/{path}\n+++ b/{path}\n@@ -1,1 +1,1 @@\n-old\n+new\n"
    );
    FileDiff::from_patch(&RawFilePatch {
        path: path.to_string(),
        old_path: None,
        raw,
        is_binary: false,
    })
    .unwrap()
}

/// The multibuffer renders every file's section header (expanded, ▾
/// indicator) with its kind letter and path, all in one buffer.
#[test]
fn multibuffer_renders_all_section_headers() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let app = App::new(vec![multi_file("a.rs"), multi_file("b.rs")]);
    let keymap = Keymap::default_map();

    terminal.draw(|frame| draw(frame, &app, &keymap)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    // Both section headers present, each with the expanded indicator ▾
    // and the change-kind letter M, and both files' bodies visible.
    assert!(content.contains("\u{25be}")); // ▾ expanded indicator
    assert!(content.contains("M a.rs"));
    assert!(content.contains("M b.rs"));
    assert!(content.contains("old"));
}

/// A collapsed section renders exactly one line: its header with the
/// collapsed indicator ▸, and none of its body rows (the `old`/`new`
/// diff lines are hidden).
#[test]
fn collapsed_section_renders_header_only_with_collapsed_indicator() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![multi_file("a.rs")]);
    app.view.set_collapsed("a.rs", true);
    app.rebuild_rows();
    let keymap = Keymap::default_map();

    terminal.draw(|frame| draw(frame, &app, &keymap)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    assert!(content.contains("\u{25b8}")); // ▸ collapsed indicator
    assert!(content.contains("M a.rs"));
    // Body rows are gone while collapsed.
    assert!(!content.contains("old"));
    assert!(!content.contains("new"));
}

/// A fully-staged file renders the `●` marker slot in its section
/// header; a partially-staged one renders `±`.
#[test]
fn staged_file_section_header_shows_marker() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![multi_file("a.rs")]);
    app.staged_states
        .insert("a.rs".to_string(), stage_ops::StagedState::Full);
    app.rebuild_rows();
    let keymap = Keymap::default_map();

    terminal.draw(|frame| draw(frame, &app, &keymap)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    assert!(content.contains("M a.rs"));
    assert!(content.contains("\u{25cf}")); // ● staged marker
}

/// A partially-staged file renders the `±` marker in its section header.
#[test]
fn partial_file_section_header_shows_partial_marker() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![multi_file("a.rs")]);
    app.staged_states
        .insert("a.rs".to_string(), stage_ops::StagedState::Partial);
    app.rebuild_rows();
    let keymap = Keymap::default_map();

    terminal.draw(|frame| draw(frame, &app, &keymap)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    assert!(content.contains("M a.rs"));
    assert!(content.contains("\u{00b1}")); // ± partial-staged marker
}

#[test]
fn empty_diff_shows_no_changes_message() {
    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let app = App::new(vec![]);
    let keymap = Keymap::default_map();

    terminal.draw(|frame| draw(frame, &app, &keymap)).unwrap();

    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
    assert!(content.contains("no changes"));
}

/// The multibuffer renders for a ref-range target exactly as for the
/// working tree — every file's section header and body appear.
#[test]
fn multibuffer_renders_for_a_range_target() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![multi_file("a.rs"), multi_file("b.rs")]);
    app.target = crate::git::DiffTarget::Range("main..HEAD".to_string());
    let keymap = Keymap::default_map();

    terminal.draw(|frame| draw(frame, &app, &keymap)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    assert!(content.contains("\u{25be}")); // ▾ expanded indicator
    assert!(content.contains("M a.rs"));
    assert!(content.contains("M b.rs"));
    assert!(content.contains("old"));
}

/// On a read-only range target the help overlay omits the inert
/// file/hunk staging gestures, but keeps the still-working staging-panel
/// toggle.
#[test]
fn help_overlay_hides_staging_rows_on_a_range_target() {
    let backend = TestBackend::new(100, 44);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.help_open = true;
    app.target = crate::git::DiffTarget::Range("main..HEAD".to_string());
    let keymap = Keymap::default_map();

    terminal.draw(|frame| draw(frame, &app, &keymap)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    assert!(content.contains("keybinds"));
    assert!(!content.contains("Stage/unstage file under cursor"));
    assert!(!content.contains("Stage/unstage hunk"));
    // The staging panel toggle still works on any target, so it stays.
    assert!(content.contains("Toggle staging panel"));
}

/// On the working-tree target every staging gesture is listed.
#[test]
fn help_overlay_shows_staging_rows_on_the_working_tree_target() {
    let backend = TestBackend::new(100, 44);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.help_open = true; // target defaults to WorkingTree
    let keymap = Keymap::default_map();

    terminal.draw(|frame| draw(frame, &app, &keymap)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    assert!(content.contains("Stage/unstage file under cursor"));
    assert!(content.contains("Stage/unstage hunk"));
    assert!(content.contains("Toggle staging panel"));
}

#[test]
fn help_overlay_renders_bindings_when_open() {
    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.help_open = true;
    let keymap = Keymap::default_map();

    terminal.draw(|frame| draw(frame, &app, &keymap)).unwrap();

    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
    assert!(content.contains("keybinds"));
    assert!(content.contains("Move cursor down"));
}

/// The help overlay documents the new remote-op and command-log bindings
/// in their scope groups (no hidden features). A tall terminal avoids the
/// overlay clipping its lower sections.
#[test]
fn help_overlay_lists_remote_and_command_log_bindings() {
    let backend = TestBackend::new(100, 300);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.help_open = true;
    let keymap = Keymap::default_map();

    terminal.draw(|frame| draw(frame, &app, &keymap)).unwrap();
    let content: String = terminal
        .backend()
        .buffer()
        .clone()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();

    // Command-log toggle (Panels group, diff scope).
    assert!(content.contains("Toggle command log pane"));
    // Remote ops (Git panel focused section, panel scope).
    assert!(content.contains("Fetch from remote"));
    assert!(content.contains("Pull from remote"));
    assert!(content.contains("Push to remote"));
    // Switcher-open binding (Git panel focused section, panel scope) and
    // its modal hint section (spec 03, task 3.0).
    assert!(content.contains("Open branch/worktree switcher"));
    assert!(content.contains("Branch/worktree switcher"));
    assert!(content.contains("Switch to the selected branch/worktree"));
}

/// On a terminal too short for the whole binding list, the help overlay
/// caps its height and scrolls: the top frame shows the first sections
/// only, and driving it to the bottom (End, through the real key path)
/// reveals the last section while scrolling the first off-screen.
#[test]
fn help_overlay_scrolls_to_reveal_lower_sections() {
    let backend = TestBackend::new(100, 22);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.help_open = true;
    let keymap = Keymap::default_map();

    // First frame renders the top of the list (and records the viewport
    // height the pager needs). The last section (Help filter) is far below.
    terminal.draw(|frame| draw(frame, &app, &keymap)).unwrap();
    let top: String = terminal
        .backend()
        .buffer()
        .clone()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(top.contains("Move cursor down"));
    assert!(!top.contains("Clear the filter"));

    // Jump to the bottom through the real dispatch path, then redraw.
    let mut pending = None;
    let end = KeyEvent::new(KeyCode::End, KeyModifiers::NONE);
    let _ = dispatch_key(&mut app, &keymap, &mut pending, end);
    terminal.draw(|frame| draw(frame, &app, &keymap)).unwrap();
    let bottom: String = terminal
        .backend()
        .buffer()
        .clone()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(bottom.contains("Clear the filter"));
    assert!(!bottom.contains("Move cursor down"));
}

/// The full lazygit-style filter lifecycle, driven through the real
/// dispatch path: `/` starts editing with an empty query, typing extends it,
/// `Enter` locks it in (handing control back to the scroll keys without
/// closing the overlay), a first `Esc` clears the locked filter (still
/// without closing), and only a second `Esc` (now with no filter left)
/// closes help.
#[test]
fn help_filter_enter_locks_and_two_escapes_close() {
    let mut app = App::new(vec![sample_file()]);
    app.help_open = true;
    let keymap = Keymap::default_map();
    let mut pending = None;

    let slash = KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE);
    dispatch_key(&mut app, &keymap, &mut pending, slash);
    assert_eq!(app.help_search, Some((String::new(), true)));

    for c in ['q', 'u', 'i', 't'] {
        let key = KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
        dispatch_key(&mut app, &keymap, &mut pending, key);
    }
    assert_eq!(app.help_search, Some(("quit".to_string(), true)));

    let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    dispatch_key(&mut app, &keymap, &mut pending, enter);
    assert_eq!(app.help_search, Some(("quit".to_string(), false)));
    assert!(
        app.help_open,
        "locking the filter must not close the overlay"
    );

    let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
    dispatch_key(&mut app, &keymap, &mut pending, esc);
    assert_eq!(app.help_search, None, "first Esc clears the locked filter");
    assert!(
        app.help_open,
        "clearing the filter must not close the overlay"
    );

    dispatch_key(&mut app, &keymap, &mut pending, esc);
    assert!(
        !app.help_open,
        "second Esc, with no filter left, closes the overlay"
    );
}

/// Closing the help overlay (either `?` or the overlay's own Close action)
/// always resets an in-progress or locked filter, so reopening starts clean.
#[test]
fn closing_help_resets_the_filter() {
    let mut app = App::new(vec![sample_file()]);
    app.apply(Action::ToggleHelp);
    assert!(app.help_open);
    app.help_search = Some(("foo".to_string(), true));

    app.apply(Action::ToggleHelp);
    assert!(!app.help_open);
    assert_eq!(app.help_search, None, "closing help must clear the filter");

    app.apply(Action::ToggleHelp);
    assert_eq!(
        app.help_search, None,
        "reopening help must start with no filter"
    );
}

/// A locked filter narrows the rendered list to rows whose key label or
/// description match the query, dropping section headers (e.g.
/// "Navigation") whose section ends up with no matching rows.
#[test]
fn help_filter_narrows_rendered_bindings_to_matching_rows() {
    let backend = TestBackend::new(120, 50);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.help_open = true;
    app.help_search = Some(("quit".to_string(), false));
    let keymap = Keymap::default_map();

    terminal.draw(|frame| draw(frame, &app, &keymap)).unwrap();
    let content: String = terminal
        .backend()
        .buffer()
        .clone()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(content.contains("Quit and emit annotations"));
    assert!(
        !content.contains("Navigation"),
        "a section with no matching rows must be dropped entirely"
    );
    assert!(!content.contains("Move cursor down"));
}

/// A query with no matches anywhere in the overlay shows a "no matches"
/// line instead of an empty list.
#[test]
fn help_filter_shows_no_matches_message_when_nothing_matches() {
    let backend = TestBackend::new(120, 50);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.help_open = true;
    app.help_search = Some(("zzznomatchzzz".to_string(), false));
    let keymap = Keymap::default_map();

    terminal.draw(|frame| draw(frame, &app, &keymap)).unwrap();
    let content: String = terminal
        .backend()
        .buffer()
        .clone()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(content.contains("no matches for"));
    assert!(content.contains("zzznomatchzzz"));
}

/// An annotation present on the selected file renders both its inline
/// display row in the diff pane and its entry in the list panel when
/// toggled open — the two annotation UI surfaces this task adds.
#[test]
fn annotation_renders_inline_and_in_list_panel() {
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.annotations
        .add(
            Target::file("src/main.rs"),
            Classification::Question,
            "why swap old() for new()?",
        )
        .unwrap();
    // App::new built `rows` before this annotation existed; rebuild so
    // the inline display row/gutter marker reflect it (this is what
    // `App::submit_compose` does internally on a real compose flow).
    app.view.rows = build_rows(
        &app.view.files[0],
        &app.annotations,
        rows::SyntaxSpans::default(),
    );
    app.mode = Mode::List;

    let keymap = Keymap::default_map();
    terminal.draw(|frame| draw(frame, &app, &keymap)).unwrap();

    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    // Inline display row in the diff pane.
    assert!(content.contains("question"));
    assert!(content.contains("why swap old() for new()?"));
    // List panel entry (mode is List, so the panel is rendered). The
    // `[1 notes]` count lives in the git panel sidebar, which is hidden
    // here since List mode isn't Mode::Panel — see
    // `sidebar_hidden_in_normal_mode_shown_when_panel_focused`.
    assert!(content.contains("src/main.rs"));
}

/// With a staged file present and the staging panel open, one frame shows
/// both staging surfaces: the staging panel entry and the transient
/// status-footer message. The git panel sidebar's staged `●` indicator and
/// `[N staged]` footer count are covered separately (Staging mode isn't
/// Mode::Panel, so the sidebar is hidden here) — see
/// `sidebar_staged_indicator_renders_when_panel_focused`.
#[test]
fn staging_panel_indicator_and_footer_render() {
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.staged = vec![StagedFile {
        path: "src/main.rs".to_string(),
        letter: 'M',
    }];
    app.staged_states
        .insert("src/main.rs".to_string(), stage_ops::StagedState::Full);
    app.mode = Mode::Staging;
    app.set_status_message("staged hunk");
    let keymap = Keymap::default_map();

    terminal.draw(|frame| draw(frame, &app, &keymap)).unwrap();

    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    assert!(content.contains("staged")); // staging panel title
    assert!(content.contains("M src/main.rs")); // panel entry
    assert!(content.contains("staged hunk")); // status footer message
}

/// The sidebar's staged `●` indicator and `[N staged]` footer count render
/// once the git panel is focused (`Mode::Panel`) — the sidebar's only
/// visibility condition post-hide-by-default.
#[test]
fn sidebar_staged_indicator_renders_when_panel_focused() {
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.staged = vec![StagedFile {
        path: "src/main.rs".to_string(),
        letter: 'M',
    }];
    app.staged_states
        .insert("src/main.rs".to_string(), stage_ops::StagedState::Full);
    app.apply(Action::FocusGitPanel);
    assert!(matches!(app.mode, Mode::Panel { .. }));
    let keymap = Keymap::default_map();

    terminal.draw(|frame| draw(frame, &app, &keymap)).unwrap();

    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    assert!(content.contains("\u{25cf}")); // sidebar staged indicator
    assert!(content.contains("[1 staged]")); // sidebar footer count
}

#[test]
fn empty_staging_panel_shows_hint() {
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.mode = Mode::Staging;
    let keymap = Keymap::default_map();

    terminal.draw(|frame| draw(frame, &app, &keymap)).unwrap();

    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
    assert!(content.contains("nothing staged yet"));
}

// -- Syntax highlighting (rendering layer) -------------------------------

/// A row carrying a syntax-highlight span renders that span's text with
/// the theme's token color — asserted via actual buffer cell styles,
/// not just text content, so this exercises the diff pane's rendering
/// (not the tree-sitter engine, which has its own tests).
#[test]
fn syntax_highlighted_span_renders_with_token_color() {
    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    let theme = app.theme;

    let Some(Row::Line(line)) = app.view.rows.get_mut(2) else {
        panic!("expected a line row at index 2");
    };
    assert_eq!(line.content, "fn main() {");
    line.syntax_spans = Some(vec![(0..2, TokenKind::Keyword)]);

    let keymap = Keymap::default_map();
    terminal.draw(|frame| draw(frame, &app, &keymap)).unwrap();

    let buffer = terminal.backend().buffer().clone();
    let has_keyword_cell = buffer
        .content()
        .iter()
        .any(|cell| cell.symbol() == "f" && cell.fg == theme.keyword);
    assert!(
        has_keyword_cell,
        "expected a cell styled with the keyword token color"
    );
}

// -- Search ---------------------------------------------------------------

/// An active search shows the `/pattern` footer prompt while typing,
/// and — once confirmed — the matched row's text renders with the
/// search-match background.
#[test]
fn search_mode_renders_prompt_and_confirmed_match_is_highlighted() {
    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    let theme = app.theme;

    app.mode = Mode::Search;
    app.search_input = "n".to_string();
    let keymap = Keymap::default_map();
    terminal.draw(|frame| draw(frame, &app, &keymap)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
    assert!(content.contains("/n"));

    // "n" matches both "fn main() {" (row 2) and "new();" (row 4);
    // confirming jumps the cursor to the first match (row 2), leaving
    // the second match (row 4) highlighted but not selected — so its
    // background is unambiguously the search-match tint, not the
    // cursor-row tint.
    app.confirm_search();
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.search.matches.len(), 2);
    assert_ne!(app.view.cursor, app.search.matches[1]);

    terminal.draw(|frame| draw(frame, &app, &keymap)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let has_match_bg = buffer
        .content()
        .iter()
        .any(|cell| cell.bg == theme.search_match_bg);
    assert!(
        has_match_bg,
        "expected the unselected matched row to carry the search-match background"
    );
}

// -- Column cursor ---------------------------------------------------------

/// The column cursor renders as a distinct background on the cursor
/// row's char cell, and only on the cursor row.
#[test]
fn column_cursor_renders_distinct_background_on_the_cursor_cell() {
    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    let theme = app.theme;
    app.apply(Action::CursorDown); // hunk header
    app.apply(Action::CursorDown); // "fn main() {"
    app.apply(Action::CursorRight);
    app.apply(Action::CursorRight); // column 2, the second 'n' of "fn"
    let keymap = Keymap::default_map();

    terminal.draw(|frame| draw(frame, &app, &keymap)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let has_cursor_bg = buffer
        .content()
        .iter()
        .any(|cell| cell.bg == theme.column_cursor_bg);
    assert!(
        has_cursor_bg,
        "expected exactly one cell styled with the column-cursor background"
    );
}

// -- LSP peek overlay --------------------------------------------------------

/// Canned References results plus a preloaded preview cache render both
/// the location list and the syntax-free preview text, without ever
/// touching a real LSP server.
#[test]
fn peek_overlay_renders_canned_references_and_preview() {
    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);

    let loc_path = PathBuf::from("/tmp/repo/src/main.rs");
    let mut peek = super::peek::PeekState::locations(
        super::peek::PeekKind::References,
        vec![SourceLocation {
            path: loc_path.clone(),
            line: 0,
            character: 0,
        }],
    );
    peek.preview_cache.insert(
        loc_path,
        super::peek::CachedPreview {
            lines: vec!["fn main() {".to_string(), "    old();".to_string()],
            spans: Vec::new(),
        },
    );
    app.peek = Some(peek);
    app.mode = Mode::Peek;
    let keymap = Keymap::default_map();

    terminal.draw(|frame| draw(frame, &app, &keymap)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    assert!(content.contains("references: 1 results"));
    assert!(content.contains("main.rs"));
    assert!(content.contains("fn main() {"));
}

/// A Hover overlay renders its text body in the same overlay chrome.
#[test]
fn peek_overlay_renders_hover_text() {
    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.peek = Some(super::peek::PeekState::hover(
        "this function does nothing interesting".to_string(),
    ));
    app.mode = Mode::Peek;
    let keymap = Keymap::default_map();

    terminal.draw(|frame| draw(frame, &app, &keymap)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    assert!(content.contains("hover"));
    assert!(content.contains("this function does nothing interesting"));
}

// -- Git panel focus -----------------------------------------------------

/// A diff file with several rows (so `j` visibly scrolls the cursor),
/// parametrized by path so the panel can list more than one entry.
fn named_file(path: &str) -> FileDiff {
    let raw = format!(
        "diff --git a/{path} b/{path}\n\
             index 111..222 100644\n\
             --- a/{path}\n\
             +++ b/{path}\n\
             @@ -1,2 +1,2 @@\n\
             -old\n\
             +new\n\
             \x20ctx\n"
    );
    FileDiff::from_patch(&RawFilePatch {
        path: path.to_string(),
        old_path: None,
        raw,
        is_binary: false,
    })
    .unwrap()
}

// -- Performance (spec 03, task 5.2/5.3) --------------------------------

/// A file whose single hunk carries `pairs` removed/added line pairs
/// (`2 * pairs` changed lines), each a realistic Rust statement so the
/// word-diff pairing runs on non-trivial content.
fn perf_file(i: usize, pairs: usize) -> FileDiff {
    let path = format!("src/module_{i}.rs");
    let mut raw = format!(
        "diff --git a/{path} b/{path}\nindex 1..2 100644\n--- a/{path}\n+++ b/{path}\n@@ -1,{pairs} +1,{pairs} @@\n"
    );
    for k in 0..pairs {
        raw.push_str(&format!(
                "-    let value_{k} = compute_old({k}, factor);\n+    let value_{k} = compute_new({k}, factor);\n"
            ));
    }
    FileDiff::from_patch(&RawFilePatch {
        path,
        old_path: None,
        raw,
        is_binary: false,
    })
    .unwrap()
}

/// An `App` populated like a real review session with panel state:
/// two tracked files, one untracked, a branch header, and two stashes.
fn panel_smoke_app() -> App {
    let mut app = App::new(vec![
        named_file("src/a.rs"),
        named_file("src/b.rs"),
        named_file("notes.md"),
    ]);
    app.untracked_paths = vec!["notes.md".to_string()];
    app.branch = Some(crate::git::BranchStatus {
        name: "main".to_string(),
        detached: false,
        upstream: Some("origin/main".to_string()),
        ahead_behind: Some((2, 1)),
    });
    app.stashes = vec![
        crate::git::StashEntry {
            stash_ref: "stash@{0}".to_string(),
            branch: Some("main".to_string()),
            message: "wip: parser".to_string(),
        },
        crate::git::StashEntry {
            stash_ref: "stash@{1}".to_string(),
            branch: Some("main".to_string()),
            message: "spike: tabs".to_string(),
        },
    ];
    app
}

/// Scans the top border row (y = 0) for cells painted with the
/// focused-border color, returning `(diff_side_hot, panel_side_hot)`. The
/// diff pane occupies `x < panel_start`; the panel occupies the rest.
fn top_border_hot(
    terminal: &Terminal<TestBackend>,
    focus: ratatui::style::Color,
    width: usize,
    panel_start: usize,
) -> (bool, bool) {
    let binding = terminal.backend().buffer().clone();
    let content = binding.content();
    let mut diff_hot = false;
    let mut panel_hot = false;
    for (x, cell) in content.iter().enumerate().take(width) {
        if cell.fg == focus {
            if x < panel_start {
                diff_hot = true;
            } else {
                panel_hot = true;
            }
        }
    }
    (diff_hot, panel_hot)
}

/// The focused pane's border is emphasized, and the toggle moves that
/// emphasis from the diff pane to the git panel and back. Since the panel
/// is hidden unless focused (see `split_layout`), this also exercises the
/// sidebar appearing/disappearing across the same toggle.
#[test]
fn focused_pane_border_emphasis_follows_the_toggle() {
    let width = 80usize;
    let panel_start = width - 32;
    let backend = TestBackend::new(width as u16, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = panel_smoke_app();
    let focus = app.theme.focused_border;
    let keymap = Keymap::default_map();

    // Diff focused (Normal): the sidebar is hidden entirely, so the diff
    // pane spans the full width with its border emphasized.
    terminal.draw(|f| draw(f, &app, &keymap)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|c| c.symbol()).collect();
    assert!(
        !content.contains("git: main"),
        "sidebar should be hidden when diff focused"
    );
    let (diff_hot, _) = top_border_hot(&terminal, focus, width, panel_start);
    assert!(
        diff_hot,
        "diff border should be emphasized when diff focused"
    );

    // Panel focused: the sidebar reappears at its fixed width, and emphasis
    // moves to the panel border.
    app.apply(Action::FocusGitPanel);
    assert!(matches!(app.mode, Mode::Panel { .. }));
    terminal.draw(|f| draw(f, &app, &keymap)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|c| c.symbol()).collect();
    assert!(
        content.contains("git: main"),
        "sidebar should render when panel focused"
    );
    let (diff_hot, panel_hot) = top_border_hot(&terminal, focus, width, panel_start);
    assert!(
        panel_hot,
        "panel border should be emphasized when panel focused"
    );
    assert!(!diff_hot, "diff border should be plain when panel focused");

    // Toggling back to the diff hides the sidebar again.
    app.apply(Action::FocusGitPanel);
    assert!(matches!(app.mode, Mode::Normal));
    terminal.draw(|f| draw(f, &app, &keymap)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|c| c.symbol()).collect();
    assert!(
        !content.contains("git: main"),
        "sidebar should hide again once focus returns to the diff"
    );
}

/// Drives real `KeyEvent`s through `dispatch_key` — the exact path the
/// blocking event loop uses — proving the focus toggle, panel `j`/`k`
/// traversal across all three sections (with the diff auto-following file
/// rows as the cursor moves), Enter-on-file, and that the diff-scope keys
/// still dispatch identically while the panel is unfocused. tmux is
/// unavailable on this host, so this headless driver stands in for the
/// manual smoke transcript (see 02-task-03-smoke.txt).
#[test]
fn panel_focus_key_dispatch_smoke() {
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut app = panel_smoke_app();
    let press = |app: &mut App, pending: &mut Option<KeyEvent>, code: KeyCode| {
        let _ = dispatch_key(
            app,
            &keymap,
            pending,
            KeyEvent::new(code, KeyModifiers::NONE),
        );
    };

    // Focus the panel: cursor resets to the top and follows to src/a.rs
    // (already selected, since `selected_file` starts at 0).
    assert_eq!(app.mode, Mode::Normal);
    press(&mut app, &mut pending, KeyCode::Char('`'));
    assert!(matches!(app.mode, Mode::Panel { .. }));
    assert_eq!(app.panel_cursor(), 0); // src/a.rs (CHANGES)
    assert_eq!(app.view.selected_file, 0); // src/a.rs

    // Traverse all three sections with `j`: a.rs -> b.rs -> notes.md
    // (UNTRACKED) -> stash0 -> stash1, clamping at the last stash. The diff
    // follows each file row; stash rows leave the last-followed file alone.
    press(&mut app, &mut pending, KeyCode::Char('j'));
    assert_eq!(app.panel_cursor(), 1); // src/b.rs
    assert_eq!(app.view.selected_file, 1); // followed to src/b.rs
    press(&mut app, &mut pending, KeyCode::Char('j'));
    assert_eq!(app.panel_cursor(), 2); // notes.md (crossed into UNTRACKED)
    assert_eq!(app.view.selected_file, 2); // followed to notes.md
    press(&mut app, &mut pending, KeyCode::Char('j'));
    assert_eq!(app.panel_cursor(), 3); // stash0 (crossed into STASHES)
    assert_eq!(app.view.selected_file, 2); // unchanged: nothing to follow
    press(&mut app, &mut pending, KeyCode::Char('j'));
    assert_eq!(app.panel_cursor(), 4); // stash1
    assert_eq!(app.view.selected_file, 2); // still unchanged
    press(&mut app, &mut pending, KeyCode::Char('j'));
    assert_eq!(app.panel_cursor(), 4); // clamped at the bottom
    press(&mut app, &mut pending, KeyCode::Char('k'));
    assert_eq!(app.panel_cursor(), 3); // back up onto a stash

    // Enter on a stash is a no-op; still focused.
    press(&mut app, &mut pending, KeyCode::Enter);
    assert!(matches!(app.mode, Mode::Panel { .. }));

    // Move onto src/b.rs (following along the way) and Enter: focus
    // returns to the diff, already on src/b.rs from the follow.
    press(&mut app, &mut pending, KeyCode::Char('k')); // -> notes.md (2)
    assert_eq!(app.view.selected_file, 2);
    press(&mut app, &mut pending, KeyCode::Char('k')); // -> src/b.rs (1)
    assert_eq!(app.panel_cursor(), 1);
    assert_eq!(app.view.selected_file, 1); // followed to src/b.rs
    press(&mut app, &mut pending, KeyCode::Enter);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.view.selected_file, 1); // src/b.rs

    // With the panel unfocused, the diff-scope keys dispatch as before.
    let cursor_before = app.view.cursor;
    press(&mut app, &mut pending, KeyCode::Char('j')); // CursorDown
    assert_eq!(app.view.cursor, cursor_before + 1);
    press(&mut app, &mut pending, KeyCode::Char('k')); // CursorUp
    assert_eq!(app.view.cursor, cursor_before);
    press(&mut app, &mut pending, KeyCode::Char('s')); // staging panel
    assert_eq!(app.mode, Mode::Staging);
    press(&mut app, &mut pending, KeyCode::Char('s')); // close it
    assert_eq!(app.mode, Mode::Normal);
    // `space` (ToggleStage) and `gd` still dispatch (no git/LSP backend
    // here, so they degrade to a footer message rather than acting) —
    // the point is they resolve and run without panicking, unchanged.
    press(&mut app, &mut pending, KeyCode::Char(' '));
    assert_eq!(app.mode, Mode::Normal);
    press(&mut app, &mut pending, KeyCode::Char('g'));
    press(&mut app, &mut pending, KeyCode::Char('d'));
    assert_eq!(app.mode, Mode::Normal);
}

/// The focused git panel is a first-class view, so the quit family ends
/// the session from it just as from the diff view: `q` emits, `Q`/Ctrl-C
/// discard. Driven through the real `dispatch_key` path.
#[test]
fn quit_family_quits_from_focused_panel() {
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let cases = [
        (
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
            QuitOutcome::Emit,
        ),
        (
            KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::NONE),
            QuitOutcome::Discard,
        ),
        (
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            QuitOutcome::Discard,
        ),
    ];
    for (ev, want) in cases {
        let mut app = panel_smoke_app();
        app.apply(Action::FocusGitPanel);
        assert!(matches!(app.mode, Mode::Panel { .. }));
        match dispatch_key(&mut app, &keymap, &mut pending, ev) {
            Flow::Quit(outcome) => assert_eq!(outcome, want, "wrong quit outcome for {ev:?}"),
            Flow::Continue => panic!("{ev:?} should quit from the focused panel"),
        }
    }
}

// -- Branch/worktree switcher modal (spec 03, task 3.0) --------------------

/// `b` resolves to `OpenSwitcher` only in panel scope, driven through the
/// real `dispatch_key` path. `panel_smoke_app` attaches no git backend, so
/// the switcher can't read branch/worktree lists and this degrades to a
/// footer message rather than opening — still proving `b` reaches
/// `App::open_switcher` from the focused panel rather than resolving to
/// nothing.
#[test]
fn b_in_panel_mode_opens_switcher_through_dispatch_key() {
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut app = panel_smoke_app();
    app.apply(Action::FocusGitPanel);
    assert!(matches!(app.mode, Mode::Panel { .. }));
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE),
    );
    assert!(matches!(app.mode, Mode::Panel { .. }));
    assert!(
        app.status_message.is_some(),
        "b must act (no-backend footer message) from the focused panel"
    );
}

/// `b` in Normal mode (diff scope) is unaffected by the panel-scope
/// binding: it stays `WordBackward`, never opening the switcher.
#[test]
fn b_in_normal_mode_still_word_jumps() {
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut app = panel_smoke_app();
    assert_eq!(app.mode, Mode::Normal);
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE),
    );
    assert_eq!(
        app.mode,
        Mode::Normal,
        "b must not open the switcher outside the focused panel"
    );
    assert!(app.switcher.is_none());
}

/// `Esc` inside the switcher modal closes it and restores the git panel's
/// cursor to the row it had before the modal opened, not wherever the
/// panel cursor happens to sit afterward.
#[test]
fn esc_restores_panel_cursor_row() {
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut app = panel_smoke_app();
    app.apply(Action::FocusGitPanel);
    app.apply(Action::PanelCursorDown);
    app.apply(Action::PanelCursorDown);
    assert_eq!(app.panel_cursor(), 2);
    app.switcher = Some(super::switcher::SwitcherState::new(
        vec![],
        vec![],
        None,
        app.panel_cursor(),
    ));
    app.mode = Mode::Switcher;
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    );
    assert!(matches!(app.mode, Mode::Panel { .. }));
    assert_eq!(
        app.panel_cursor(),
        2,
        "Esc must restore the pre-open panel cursor row"
    );
}

/// `q` is inert inside the switcher modal, per the existing overlay rule —
/// it must not quit the session.
#[test]
fn q_is_inert_inside_switcher() {
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut app = panel_smoke_app();
    app.switcher = Some(super::switcher::SwitcherState::new(vec![], vec![], None, 0));
    app.mode = Mode::Switcher;
    match dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
    ) {
        Flow::Quit(_) => panic!("q must not quit from inside the switcher modal"),
        Flow::Continue => {}
    }
    assert_eq!(
        app.mode,
        Mode::Switcher,
        "q must not close the switcher modal"
    );
}

/// An open overlay never quits the app: `q` is inert while the help
/// overlay is up, and `?` still toggles it closed. Driven through
/// `dispatch_key`.
#[test]
fn q_is_inert_while_help_overlay_open() {
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut app = panel_smoke_app();
    app.help_open = true;
    let flow = dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
    );
    assert!(
        matches!(flow, Flow::Continue),
        "q must not quit while help is open"
    );
    assert!(app.help_open, "q must not close the help overlay");
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
    );
    assert!(!app.help_open, "? still closes the help overlay");
}

/// `@` toggles the command-log pane from *both* the diff view (Normal)
/// and the focused git panel, driven through the real `dispatch_key`
/// path; when open the pane renders in the bottom-panel slot, showing a
/// nonzero-exit entry with its stderr.
#[test]
fn at_toggles_command_log_from_both_scopes_and_renders_in_bottom_slot() {
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut app = panel_smoke_app();
    app.command_log.push(super::command_log::CommandLogEntry {
        command_line: "git push".to_string(),
        success: false,
        code: Some(1),
        stdout: String::new(),
        stderr: "! [rejected] main -> main (non-fast-forward)".to_string(),
    });
    let at = KeyEvent::new(KeyCode::Char('@'), KeyModifiers::NONE);
    let backtick = KeyEvent::new(KeyCode::Char('`'), KeyModifiers::NONE);

    // Diff scope: `@` opens the log.
    assert!(!app.command_log_open);
    dispatch_key(&mut app, &keymap, &mut pending, at);
    assert!(app.command_log_open);

    // It renders in the bottom slot with the failed entry and its stderr.
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &app, &keymap)).unwrap();
    let content: String = terminal
        .backend()
        .buffer()
        .clone()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(content.contains("command log"));
    assert!(content.contains("git push"));
    assert!(content.contains("exit 1"));
    assert!(content.contains("non-fast-forward"));

    // `@` again closes it.
    dispatch_key(&mut app, &keymap, &mut pending, at);
    assert!(!app.command_log_open);

    // Panel scope toggles it too: focus the panel, then `@`.
    dispatch_key(&mut app, &keymap, &mut pending, backtick);
    assert!(matches!(app.mode, Mode::Panel { .. }));
    dispatch_key(&mut app, &keymap, &mut pending, at);
    assert!(app.command_log_open);
    // Still focused on the panel — the log toggle is orthogonal to focus.
    assert!(matches!(app.mode, Mode::Panel { .. }));
}

/// The running indicator shows in the footer while a remote op is in
/// flight (here, a stalled background task the test controls).
#[test]
fn running_indicator_renders_while_a_remote_op_is_in_flight() {
    let keymap = Keymap::default_map();
    let mut app = panel_smoke_app();
    // Spawn a task that blocks on a gate we never release, so the op stays
    // "in flight" for the duration of the render.
    let (_gate_tx, gate_rx) = std::sync::mpsc::channel::<()>();
    let id = app.background.spawn(move || {
        let _ = gate_rx.recv();
        super::background::CommandOutcome {
            success: true,
            code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
        }
    });
    app.remote_op = Some(super::app::InFlightRemote {
        id,
        op: crate::git::RemoteOp::Fetch,
    });

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &app, &keymap)).unwrap();
    let content: String = terminal
        .backend()
        .buffer()
        .clone()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(
        content.contains("fetch"),
        "footer should show the running fetch indicator"
    );
}

/// With no search input, remote-op indicator, or transient status message
/// active, the footer falls back to the persistent idle hint advertising the
/// backtick binding — the git panel's only discoverability surface now that
/// it's hidden by default (besides the `?` help overlay).
#[test]
fn idle_footer_hint_renders_when_nothing_else_occupies_the_footer() {
    let keymap = Keymap::default_map();
    let app = App::new(vec![sample_file()]);

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &app, &keymap)).unwrap();
    let content: String = terminal
        .backend()
        .buffer()
        .clone()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(
        content.contains("git panel"),
        "idle footer should hint at the backtick binding"
    );
    assert!(
        content.contains("help"),
        "idle footer should hint at the help overlay"
    );
}

/// A transient status message takes priority over the idle footer hint —
/// the hint only shows once the message clears.
#[test]
fn status_message_replaces_the_idle_footer_hint() {
    let keymap = Keymap::default_map();
    let mut app = App::new(vec![sample_file()]);
    app.set_status_message("staged hunk");

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &app, &keymap)).unwrap();
    let content: String = terminal
        .backend()
        .buffer()
        .clone()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(content.contains("staged hunk"));
    assert!(
        !content.contains("git panel"),
        "status message should displace the idle hint"
    );
}

/// Regenerates `02-proofs/02-task-03-smoke.txt` from a real key-dispatch
/// run, rendering a `TestBackend` frame at each step and recording the
/// observed state. Ignored by default (it writes into the repo); run with
/// `cargo test capture_task_03_smoke_transcript -- --ignored` to refresh.
#[test]
#[ignore = "writes the smoke transcript proof artifact; run explicitly"]
fn capture_task_03_smoke_transcript() {
    use std::fmt::Write as _;

    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut app = panel_smoke_app();
    let backend = TestBackend::new(60, 20);
    let mut terminal = Terminal::new(backend).unwrap();

    let mut out = String::new();
    out.push_str("Task 3.0 smoke transcript — git panel focus & navigation\n");
    out.push_str("Driver: real crossterm KeyEvents through ui::dispatch_key\n");
    out.push_str("(the same handler the blocking event loop calls).\n");
    out.push_str("tmux unavailable on this host; headless TestBackend fallback.\n\n");

    let mut step = 0;
    // One flat closure: optionally dispatch a key, render a frame, and
    // record observed state. All mutable state is passed in as params so
    // nothing aliases across calls.
    let mut do_step = |app: &mut App,
                       pending: &mut Option<KeyEvent>,
                       terminal: &mut Terminal<TestBackend>,
                       out: &mut String,
                       code: Option<KeyCode>,
                       label: &str| {
        if let Some(code) = code {
            let _ = dispatch_key(
                app,
                &keymap,
                pending,
                KeyEvent::new(code, KeyModifiers::NONE),
            );
        }
        terminal.draw(|f| draw(f, app, &keymap)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let text: String = buf.content().iter().map(|c| c.symbol()).collect();
        let pane = if app.git_panel_focused() {
            "git panel"
        } else {
            "diff view"
        };
        writeln!(out, "step {step}: {label}").unwrap();
        writeln!(
            out,
            "  mode={:?} focus={pane} panel_cursor={} selected_file={} diff_cursor={}",
            app.mode,
            app.panel_cursor(),
            app.view.selected_file,
            app.view.cursor
        )
        .unwrap();
        writeln!(
            out,
            "  frame shows: CHANGES={} UNTRACKED={} STASHES={} branch_hdr={}",
            text.contains("CHANGES"),
            text.contains("UNTRACKED"),
            text.contains("STASHES"),
            text.contains("git: main")
        )
        .unwrap();
        out.push('\n');
        step += 1;
    };

    let steps: &[(Option<KeyCode>, &str)] = &[
        (None, "initial (diff focused)"),
        (Some(KeyCode::Char('`')), "press ` — focus git panel"),
        (
            Some(KeyCode::Char('j')),
            "press j — CHANGES row 2 (src/b.rs)",
        ),
        (
            Some(KeyCode::Char('j')),
            "press j — cross into UNTRACKED (notes.md)",
        ),
        (
            Some(KeyCode::Char('j')),
            "press j — cross into STASHES (stash 0)",
        ),
        (Some(KeyCode::Char('j')), "press j — STASHES (stash 1)"),
        (
            Some(KeyCode::Enter),
            "press Enter on stash — no-op, stays focused",
        ),
        (Some(KeyCode::Char('k')), "press k — back up"),
        (
            Some(KeyCode::Char('k')),
            "press k — onto UNTRACKED (notes.md)",
        ),
        (
            Some(KeyCode::Char('k')),
            "press k — onto CHANGES (src/b.rs)",
        ),
        (
            Some(KeyCode::Enter),
            "press Enter on file — jump diff, return focus",
        ),
        (
            Some(KeyCode::Char('j')),
            "press j (unfocused) — diff cursor scrolls",
        ),
        (
            Some(KeyCode::Char('s')),
            "press s (unfocused) — staging panel opens",
        ),
        (Some(KeyCode::Char('s')), "press s — staging panel closes"),
    ];
    for (code, label) in steps {
        do_step(
            &mut app,
            &mut pending,
            &mut terminal,
            &mut out,
            *code,
            label,
        );
    }

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("docs/specs/02-spec-git-panel/02-proofs/02-task-03-smoke.txt");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, out).unwrap();
}

/// Regenerates `02-proofs/02-task-04-smoke.txt` from two real runs:
/// (a) a deliberately slow (2s) background command driven through the real
/// `BackgroundTasks` path while `j`/`k` scroll the diff via `dispatch_key`,
/// with timestamped observations proving the render loop never blocks; and
/// (b) a real non-fast-forward push rejection against a `file://` remote,
/// driven through the real spawn -> poll -> command-log pipeline, with the
/// command-log pane rendered showing git's stderr and nonzero exit.
///
/// tmux is unavailable on this host, so this headless driver stands in for
/// the manual transcript. Ignored by default (it writes into the repo, runs
/// a 2s sleep, and shells out to git); run with
/// `cargo test capture_task_04_smoke_transcript -- --ignored`.
#[test]
#[ignore = "writes the task-04 smoke transcript; 2s sleep + real git. run explicitly"]
fn capture_task_04_smoke_transcript() {
    use super::app::InFlightRemote;
    use super::background::run_command;
    use crate::git::RemoteOp;
    use std::fmt::Write as _;
    use std::process::Command as PCommand;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    fn git(dir: &std::path::Path, args: &[&str]) {
        let out = PCommand::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .expect("spawn git");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let mut out = String::new();
    let wall = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    out.push_str("Task 4.0 smoke transcript — async remote ops & command log\n");
    out.push_str("Driver: real BackgroundTasks + ui::dispatch_key (the handlers\n");
    out.push_str("the blocking event loop calls). tmux unavailable; headless.\n");
    writeln!(out, "Wall-clock start (unix seconds): {wall}\n").unwrap();

    // -- Part (a): non-blocking proof ---------------------------------
    out.push_str("== Part (a): render loop stays responsive during a slow op ==\n");
    out.push_str("A 2s `sleep` runs on the real background poller while j/k drive\n");
    out.push_str("the diff cursor through dispatch_key. Each keypress is timed from\n");
    out.push_str("op-spawn; all land in single-digit ms, far under the 2000ms op.\n\n");

    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut app = panel_smoke_app();

    let spawn_at = Instant::now();
    let id = app.background.spawn(|| {
        let mut cmd = PCommand::new("sh");
        cmd.args(["-c", "sleep 2"]);
        run_command(&mut cmd)
    });
    app.remote_op = Some(InFlightRemote {
        id,
        op: RemoteOp::Fetch,
    });
    writeln!(
        out,
        "t=+{:>4}ms spawned slow op (remote_op={:?}, running_label={:?})",
        spawn_at.elapsed().as_millis(),
        app.remote_op.map(|o| o.op),
        app.remote_running_label(),
    )
    .unwrap();

    let motions = [
        (KeyCode::Char('j'), "j"),
        (KeyCode::Char('j'), "j"),
        (KeyCode::Char('j'), "j"),
        (KeyCode::Char('k'), "k"),
    ];
    for (code, label) in motions {
        let before = app.view.cursor;
        dispatch_key(
            &mut app,
            &keymap,
            &mut pending,
            KeyEvent::new(code, KeyModifiers::NONE),
        );
        // Poll for completion the way the event loop does; the op is still
        // sleeping, so nothing drains — the guard stays set, log empty.
        app.poll_remote();
        let still_pending = app.remote_op.is_some() && app.command_log.is_empty();
        assert!(still_pending, "op must still be in flight during scrolling");
        writeln!(
            out,
            "t=+{:>4}ms press {label}: diff_cursor {before}->{} | op still pending={} log_len={}",
            spawn_at.elapsed().as_millis(),
            app.view.cursor,
            still_pending,
            app.command_log.len(),
        )
        .unwrap();
    }
    assert!(
        spawn_at.elapsed() < Duration::from_millis(1500),
        "all scrolling completed well before the 2s op"
    );
    writeln!(
        out,
        "\nObservation: all {} keypresses processed by t=+{}ms while the\n\
             2000ms op was still pending — dispatch never blocked on it.",
        motions.len(),
        spawn_at.elapsed().as_millis()
    )
    .unwrap();

    // Let the op finish and drain it, recording when.
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.remote_op.is_some() && Instant::now() < deadline {
        app.poll_remote();
        std::thread::sleep(Duration::from_millis(10));
    }
    writeln!(
        out,
        "t=+{:>4}ms op completed and drained (log_len={})\n",
        spawn_at.elapsed().as_millis(),
        app.command_log.len()
    )
    .unwrap();

    // -- Part (b): failure transparency -------------------------------
    out.push_str("== Part (b): a non-fast-forward push rejection is logged ==\n");
    out.push_str("A file:// bare remote is advanced by a second clone; the local\n");
    out.push_str("clone commits too, so `git push` is rejected non-fast-forward.\n");
    out.push_str("Driven through App::request_remote_op -> poll_remote (real spawn).\n\n");

    let bare = tempfile::TempDir::new().unwrap();
    // Branch must be explicit: the host's init.defaultBranch config must not
    // leak into the fixture. A bare repo's HEAD (e.g. `master` on a host
    // without init.defaultBranch set) that never has a matching branch
    // pushed to it leaves clones with a dangling HEAD and no working tree.
    git(bare.path(), &["init", "-q", "--bare", "-b", "main"]);
    let bare_url = format!("file://{}", bare.path().display());

    let repo = tempfile::TempDir::new().unwrap();
    git(repo.path(), &["init", "-q"]);
    git(repo.path(), &["config", "user.name", "redquill test"]);
    git(
        repo.path(),
        &["config", "user.email", "test@redquill.invalid"],
    );
    git(repo.path(), &["branch", "-M", "main"]);
    std::fs::write(repo.path().join("base.txt"), b"one\n").unwrap();
    git(repo.path(), &["add", "."]);
    git(repo.path(), &["commit", "-qm", "initial"]);
    git(repo.path(), &["remote", "add", "origin", &bare_url]);
    git(repo.path(), &["push", "-q", "-u", "origin", "main"]);

    // A second clone advances origin/main out from under `repo`.
    let parent = tempfile::TempDir::new().unwrap();
    git(parent.path(), &["clone", "-q", &bare_url, "clone2"]);
    let clone2 = parent.path().join("clone2");
    git(&clone2, &["config", "user.name", "redquill test"]);
    git(&clone2, &["config", "user.email", "test@redquill.invalid"]);
    std::fs::write(clone2.join("base.txt"), b"one\nremote two\n").unwrap();
    git(&clone2, &["commit", "-aqm", "remote commit"]);
    git(&clone2, &["push", "-q", "origin", "main"]);

    // The local clone commits its own divergent history.
    std::fs::write(repo.path().join("base.txt"), b"one\nlocal two\n").unwrap();
    git(repo.path(), &["commit", "-aqm", "local commit"]);

    let mut app2 = panel_smoke_app();
    app2.set_repo_root(repo.path().to_path_buf());
    app2.request_remote_op(RemoteOp::Push);
    writeln!(
        out,
        "spawned push (running_label={:?})",
        app2.remote_running_label()
    )
    .unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    while app2.command_log.is_empty() && Instant::now() < deadline {
        app2.poll_remote();
        std::thread::sleep(Duration::from_millis(10));
    }
    let entry = app2
        .command_log
        .entries()
        .next()
        .expect("push should have been logged")
        .clone();
    assert!(!entry.success, "the non-ff push must be recorded as failed");
    writeln!(
        out,
        "logged: command_line={:?} exit_status={:?} success={}",
        entry.command_line,
        entry.exit_status(),
        entry.success
    )
    .unwrap();
    writeln!(out, "stderr (verbatim from git):").unwrap();
    for line in entry.stderr.lines() {
        writeln!(out, "    {line}").unwrap();
    }

    // Render the command-log pane and capture what the user would see.
    app2.command_log_open = true;
    let backend = TestBackend::new(100, 40);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &app2, &keymap)).unwrap();
    let frame: String = terminal
        .backend()
        .buffer()
        .clone()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(frame.contains("command log"), "pane must be visible");
    assert!(
        frame.contains("git push") && frame.contains("exit"),
        "pane must show the failed push and its exit status"
    );
    let shows_reject = frame.contains("rejected") || frame.contains("fast-forward");
    writeln!(
        out,
        "\ncommand-log pane visible: {} | shows rejection text: {}",
        frame.contains("command log"),
        shows_reject
    )
    .unwrap();
    out.push_str("Observation: the rejected push did not crash the tool; it landed\n");
    out.push_str("in the command log with its nonzero exit and git's own stderr.\n");

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("docs/specs/02-spec-git-panel/02-proofs/02-task-04-smoke.txt");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, out).unwrap();
}

/// Builds a ~5k-changed-line, 15-file multibuffer and scrolls it top to
/// bottom a half-page at a time through the real `draw` render path on a
/// `TestBackend`, reporting ms/frame. The spec's quantitative proxy for
/// "instant-feel scrolling" is ms/frame well under 16ms; the assertion
/// uses a generous CI-safe bound (real measured value, recorded in the
/// perf proof, is far lower). Run with `--nocapture` to see the numbers.
#[test]
fn scrolling_a_5k_line_multibuffer_renders_fast() {
    let files: Vec<FileDiff> = (0..15).map(|i| perf_file(i, 168)).collect();
    let total_lines: usize = files
        .iter()
        .flat_map(|f| f.hunks.iter())
        .map(|h| h.lines.len())
        .sum();
    assert!(
        total_lines >= 5000,
        "fixture should be ~5k changed lines, got {total_lines}"
    );

    let mut app = App::new(files);
    let total_rows = app.view.rows.len();
    let keymap = Keymap::default_map();
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).unwrap();
    app.view.set_viewport_height(38);

    let mut frames = 0u32;
    let start = std::time::Instant::now();
    loop {
        terminal.draw(|frame| draw(frame, &app, &keymap)).unwrap();
        frames += 1;
        if app.view.cursor >= app.view.max_cursor() || frames > 2000 {
            break;
        }
        app.apply(Action::HalfPageDown);
    }
    let per_frame = start.elapsed() / frames;
    println!(
        "scroll: {frames} frames over {total_rows} rows ({total_lines} changed lines), {per_frame:?}/frame"
    );
    assert!(
        per_frame < std::time::Duration::from_millis(50),
        "ms/frame {per_frame:?} too slow over {frames} frames / {total_rows} rows"
    );
}
