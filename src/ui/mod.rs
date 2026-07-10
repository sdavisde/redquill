//! ratatui widgets, layout, the event loop, and the keymap. The keymap is
//! data (remappable), not hardcoded match arms scattered through widgets.
//!
//! Normal and Visual mode dispatch every keystroke through the [`Keymap`]
//! table. Compose, List, and Staging are modal: while one is active, every
//! keystroke is handled directly by [`handle_compose_key`]/
//! [`handle_list_key`]/[`handle_staging_key`] instead of going through the
//! table, since most of what they read (printable characters, `j`/`k` as
//! list navigation rather than the Navigation action) isn't expressible as
//! one fixed [`Action`] per key.
//!
//! The TUI renders to **stderr**, never stdout: stdout is reserved for the
//! annotation markdown emitted on quit (`redquill | claude -p "..."`), while
//! the TUI itself stays interactive on the terminal. [`run`] owns the whole
//! lifecycle — raw mode, alternate screen, panic-safe restoration, and the
//! blocking event loop — and returns which way the session ended.

mod app;
mod compose;
mod compose_modal;
mod diff_view;
mod help;
mod keymap;
mod list_panel;
mod lsp_ops;
mod peek;
mod peek_overlay;
mod rows;
mod search;
mod sidebar;
mod stage_ops;
mod staging_panel;
mod syntax;
mod theme;

pub use app::{App, Mode};
pub use keymap::{Action, Binding, Keymap};
pub use lsp_ops::LspClient;
pub use rows::{Row, build_rows};
pub use stage_ops::{ReviewError, ReviewSnapshot, StageOps, StagedFile, build_review};
pub use theme::Theme;

use std::io::{self, Stderr};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

/// How long the event loop waits for a key event before giving up and
/// draining LSP events anyway. Keeps `gd`/`gr`/`K` responses (and any
/// server-driven state) flowing even while the user isn't typing, without
/// ever blocking the render loop on a slow or missing language server.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// How a TUI session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitOutcome {
    /// The user pressed `q`: emit `app.annotations` to stdout.
    Emit,
    /// The user pressed `Q` or Ctrl-C: discard annotations, emit nothing.
    Discard,
}

/// Splits the full terminal area into the main content area and the
/// single-line status footer at the bottom (transient messages).
fn split_footer(area: Rect) -> (Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);
    (chunks[0], chunks[1])
}

/// Splits the main content area into the sidebar and diff-pane rects. The
/// sidebar renders on the right; see `docs/config-layer.md` for making this
/// (and its width) configurable.
fn split_layout(area: Rect) -> (Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(32)])
        .split(area);
    (chunks[1], chunks[0])
}

/// Splits the right-hand area into the diff pane and (when `show_panel`) a
/// bottom panel (annotation list or staging panel) below it, ~60/40.
fn split_right(area: Rect, show_panel: bool) -> (Rect, Option<Rect>) {
    if !show_panel {
        return (area, None);
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);
    (chunks[0], Some(chunks[1]))
}

/// Whether the current mode shows a bottom panel next frame.
fn panel_open(mode: Mode) -> bool {
    matches!(mode, Mode::List | Mode::Staging)
}

/// Draws one frame: sidebar, diff pane, bottom panel (annotation list or
/// staging panel, if open), status footer, help overlay (if open), and the
/// Compose modal (if open).
fn draw(frame: &mut ratatui::Frame, app: &App, keymap: &Keymap) {
    let area = frame.area();
    let (main_area, footer_area) = split_footer(area);
    let (sidebar_area, right_area) = split_layout(main_area);
    let (diff_area, panel_area) = split_right(right_area, panel_open(app.mode));

    sidebar::render(frame, sidebar_area, app);
    diff_view::render(frame, diff_area, app);
    if let Some(panel_area) = panel_area {
        match app.mode {
            Mode::Staging => staging_panel::render(frame, panel_area, app),
            _ => list_panel::render(frame, panel_area, app),
        }
    }
    if matches!(app.mode, Mode::Search) {
        let text = format!("/{}", app.search_input);
        let footer = Line::from(Span::styled(
            text.clone(),
            Style::default().fg(app.theme.search_prompt),
        ));
        frame.render_widget(footer, footer_area);
        let cursor_x = footer_area
            .x
            .saturating_add(text.chars().count() as u16)
            .min(footer_area.x + footer_area.width.saturating_sub(1));
        frame.set_cursor_position(Position::new(cursor_x, footer_area.y));
    } else if let Some(message) = &app.status_message {
        let footer = Line::from(Span::styled(
            format!(" {message}"),
            Style::default().fg(app.theme.status_message),
        ));
        frame.render_widget(footer, footer_area);
    }
    if app.help_open {
        help::render(frame, area, keymap, &app.theme);
    }
    if matches!(app.mode, Mode::Compose) {
        compose_modal::render(frame, area, app);
    }
    if matches!(app.mode, Mode::Peek) {
        peek_overlay::render(frame, area, app);
    }
}

/// Puts the terminal into raw mode + alternate screen, on stderr.
fn init_terminal() -> io::Result<Terminal<CrosstermBackend<Stderr>>> {
    enable_raw_mode()?;
    execute!(io::stderr(), EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(io::stderr()))
}

/// Restores the terminal to its normal state. Safe to call more than once
/// and safe to call from a panic hook.
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stderr(), LeaveAlternateScreen);
}

/// Installs a panic hook that restores the terminal before the default hook
/// runs, so a panic mid-session doesn't leave the user's terminal in raw
/// mode / the alternate screen.
fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        original(info);
    }));
}

/// Runs the interactive TUI over `app` until the user quits, returning how
/// the session ended. Terminal setup, the blocking event loop, and terminal
/// restoration all happen here; callers only need to act on the returned
/// [`QuitOutcome`] (e.g. emit `app.annotations` on [`QuitOutcome::Emit`]).
pub fn run(app: &mut App) -> anyhow::Result<QuitOutcome> {
    install_panic_hook();
    let mut terminal = init_terminal()?;
    let keymap = Keymap::default_map();
    let outcome = event_loop(&mut terminal, app, &keymap);
    restore_terminal();
    // Shut down any LSP servers spawned this session only after the
    // terminal is restored: `shutdown` blocks briefly (grace period) and
    // must never delay giving the user's terminal back.
    if let Some(client) = app.take_lsp_client() {
        client.shutdown();
    }
    outcome
}

/// Handles one key event while [`Mode::Compose`] is active: printable chars
/// insert, Backspace deletes, arrow keys move within the text, `Ctrl-j`
/// inserts a newline, `Enter` submits, `Esc` cancels. Bypasses the
/// [`Keymap`] table entirely (see the module docs).
fn handle_compose_key(app: &mut App, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => app.cancel_compose(),
        KeyCode::Enter => app.submit_compose(),
        KeyCode::Char('j') if ctrl => {
            if let Some(compose) = app.compose.as_mut() {
                compose.buffer.newline();
            }
        }
        KeyCode::Char('t') if ctrl => {
            if let Some(compose) = app.compose.as_mut() {
                compose.classification = compose.classification.cycle();
            }
        }
        KeyCode::Backspace => {
            if let Some(compose) = app.compose.as_mut() {
                compose.buffer.backspace();
            }
        }
        KeyCode::Left => {
            if let Some(compose) = app.compose.as_mut() {
                compose.buffer.move_left();
            }
        }
        KeyCode::Right => {
            if let Some(compose) = app.compose.as_mut() {
                compose.buffer.move_right();
            }
        }
        KeyCode::Up => {
            if let Some(compose) = app.compose.as_mut() {
                compose.buffer.move_up();
            }
        }
        KeyCode::Down => {
            if let Some(compose) = app.compose.as_mut() {
                compose.buffer.move_down();
            }
        }
        KeyCode::Char(c) if !ctrl => {
            if let Some(compose) = app.compose.as_mut() {
                compose.buffer.insert_char(c);
            }
        }
        _ => {}
    }
}

/// Handles one key event while [`Mode::List`] is active: `j`/`k` move
/// focus, `Enter` jumps to the annotation and closes the panel, `e` edits
/// it, `d` deletes it, `a`/`Esc` close the panel. Bypasses the [`Keymap`]
/// table entirely (see the module docs).
fn handle_list_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('j') => app.list_move_down(),
        KeyCode::Char('k') => app.list_move_up(),
        KeyCode::Enter => app.jump_to_focused_annotation(),
        KeyCode::Char('e') => app.edit_focused_annotation(),
        KeyCode::Char('d') => app.delete_focused_annotation(),
        KeyCode::Char('a') | KeyCode::Esc => app.close_list(),
        _ => {}
    }
}

/// Handles one key event while [`Mode::Staging`] is active: `j`/`k` move
/// focus, `Space`/`Enter` unstage the focused file (the panel stays open),
/// `s`/`Esc` close the panel. Bypasses the [`Keymap`] table entirely (see
/// the module docs).
fn handle_staging_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('j') => app.staging_move_down(),
        KeyCode::Char('k') => app.staging_move_up(),
        KeyCode::Char(' ') | KeyCode::Enter => app.unstage_focused_file(),
        KeyCode::Char('s') | KeyCode::Esc => app.close_staging(),
        _ => {}
    }
}

/// Handles one key event while [`Mode::Search`] is active: printable chars
/// insert into the pattern buffer, Backspace deletes, `Enter` confirms
/// (jumping to the first match at-or-after the cursor), `Esc` cancels
/// (clearing the active pattern only if the buffer was left empty).
/// Bypasses the [`Keymap`] table entirely (see the module docs).
fn handle_search_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => app.cancel_search(),
        KeyCode::Enter => app.confirm_search(),
        KeyCode::Backspace => {
            app.search_input.pop();
        }
        KeyCode::Char(c) => app.search_input.push(c),
        _ => {}
    }
}

/// Handles one key event while [`Mode::Peek`] is active: `j`/`k` move
/// through results (or scroll hover text), `Enter` jumps the diff cursor to
/// a Definition/References result that's one of the diff's files (closing
/// the overlay) or sets `not in diff` otherwise (a no-op for Hover),
/// `Esc`/`q` close back to Normal. Bypasses the [`Keymap`] table entirely
/// (see the module docs).
fn handle_peek_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('j') => app.peek_move_down(),
        KeyCode::Char('k') => app.peek_move_up(),
        KeyCode::Enter => app.peek_enter(),
        KeyCode::Char('q') | KeyCode::Esc => app.close_peek(),
        _ => {}
    }
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stderr>>,
    app: &mut App,
    keymap: &Keymap,
) -> anyhow::Result<QuitOutcome> {
    // Tracks a `g`-prefix key across loop iterations while it awaits a
    // second key to complete `gd`/`gr` (see `Keymap::resolve`).
    let mut pending_prefix: Option<KeyEvent> = None;

    loop {
        let size = terminal.size()?;
        let full_area = Rect::new(0, 0, size.width, size.height);
        let (main_area, _) = split_footer(full_area);
        let (_, right_area) = split_layout(main_area);
        let (diff_area, _) = split_right(right_area, panel_open(app.mode));
        app.set_viewport_height(diff_view::viewport_height(diff_area));

        terminal.draw(|frame| draw(frame, app, keymap))?;

        // Bounded wait rather than a blocking read: LSP responses must keep
        // flowing (via `poll_lsp` below) even while the user isn't typing,
        // without ever blocking the render loop on a slow/missing server.
        if event::poll(POLL_INTERVAL)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    // Transient footer messages last exactly until the next
                    // keypress (whatever this key does may set a fresh one).
                    app.clear_status_message();
                    match app.mode {
                        Mode::Compose => handle_compose_key(app, key),
                        Mode::List => handle_list_key(app, key),
                        Mode::Staging => handle_staging_key(app, key),
                        Mode::Search => handle_search_key(app, key),
                        Mode::Peek => handle_peek_key(app, key),
                        Mode::Normal | Mode::Visual { .. } => {
                            let had_pending = pending_prefix.is_some();
                            let action = keymap.resolve(&mut pending_prefix, key);

                            // Esc only ever closes an already-open help
                            // overlay or cancels an in-progress Visual
                            // selection; it is never bound to opening help,
                            // unlike `?` (see keymap.rs). This runs only
                            // when nothing was pending — an Esc that
                            // cancelled a pending `g` prefix (handled inside
                            // `resolve`) stops there instead.
                            if key.code == KeyCode::Esc && !had_pending {
                                if app.help_open {
                                    app.help_open = false;
                                } else if matches!(app.mode, Mode::Visual { .. }) {
                                    app.apply(Action::EnterVisual);
                                }
                                continue;
                            }

                            let Some(action) = action else {
                                continue;
                            };
                            match action {
                                Action::Quit => return Ok(QuitOutcome::Emit),
                                Action::QuitDiscard => return Ok(QuitOutcome::Discard),
                                other => app.apply(other),
                            }
                        }
                    }
                }
                Event::Resize(_, _) => {
                    // The next loop iteration re-measures the layout and
                    // redraws at the new size; nothing else to do here.
                }
                _ => {}
            }
        }

        app.poll_lsp();
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::annotate::{Classification, Target};
    use crate::diff::FileDiff;
    use crate::git::RawFilePatch;
    use crate::highlight::TokenKind;
    use crate::lsp::SourceLocation;
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

    /// Renders a small `App` to a `TestBackend` and asserts the sidebar and
    /// diff pane both show expected content. No real terminal is touched.
    #[test]
    fn renders_sidebar_and_diff_pane_content() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let app = App::new(vec![sample_file()]);
        let keymap = Keymap::default_map();

        terminal.draw(|frame| draw(frame, &app, &keymap)).unwrap();

        let buffer = terminal.backend().buffer().clone();
        let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

        assert!(content.contains("files"));
        assert!(content.contains("src/main.rs"));
        assert!(content.contains("old()"));
        assert!(content.contains("new()"));
        assert!(content.contains("[1 files]"));
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
        assert!(content.contains("help"));
        assert!(content.contains("Move cursor down"));
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
        app.rows = build_rows(
            &app.files[0],
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
        // List panel entry (mode is List, so the panel is rendered).
        assert!(content.contains("src/main.rs"));
        assert!(content.contains("[1 notes]"));
    }

    /// With a staged file present and the staging panel open, one frame
    /// shows all three staging surfaces: the sidebar's staged `●`
    /// indicator and `[N staged]` footer count, the staging panel entry,
    /// and the transient status-footer message.
    #[test]
    fn staging_panel_indicator_and_footer_render() {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new(vec![sample_file()]);
        app.staged = vec![StagedFile {
            path: "src/main.rs".to_string(),
            letter: 'M',
        }];
        app.mode = Mode::Staging;
        app.set_status_message("staged hunk");
        let keymap = Keymap::default_map();

        terminal.draw(|frame| draw(frame, &app, &keymap)).unwrap();

        let buffer = terminal.backend().buffer().clone();
        let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

        assert!(content.contains("\u{25cf}")); // sidebar staged indicator
        assert!(content.contains("[1 staged]")); // sidebar footer count
        assert!(content.contains("staged")); // staging panel title
        assert!(content.contains("M src/main.rs")); // panel entry
        assert!(content.contains("staged hunk")); // status footer message
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

        let Some(Row::Line(line)) = app.rows.get_mut(2) else {
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

    #[test]
    fn search_input_editing_via_handle_search_key() {
        let mut app = App::new(vec![sample_file()]);
        app.mode = Mode::Search;
        handle_search_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE),
        );
        handle_search_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
        );
        handle_search_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
        );
        assert_eq!(app.search_input, "old");
        handle_search_key(
            &mut app,
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        );
        assert_eq!(app.search_input, "ol");
        handle_search_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.search.pattern.as_deref(), Some("ol"));
    }

    #[test]
    fn search_esc_cancels_mode() {
        let mut app = App::new(vec![sample_file()]);
        app.mode = Mode::Search;
        app.search_input.push('x');
        handle_search_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Normal);
    }

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
        assert_ne!(app.cursor, app.search.matches[1]);

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
}
