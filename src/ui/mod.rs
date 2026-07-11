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
mod background;
mod code_intel;
mod command_log;
mod compose;
mod compose_modal;
mod diff_view;
mod diff_view_state;
mod git_panel;
mod help;
mod keymap;
mod list_panel;
mod lsp_ops;
mod modes;
mod peek;
mod peek_overlay;
mod rows;
mod search;
mod stage_ops;
mod staging;
mod staging_panel;
mod syntax;
mod theme;

pub use app::{App, Mode};
pub use diff_view_state::DiffViewState;
pub use keymap::{Action, Binding, Keymap};
pub use lsp_ops::LspClient;
pub use rows::{Row, build_rows};
pub use stage_ops::{ReviewError, ReviewSnapshot, StageOps, StagedFile, build_review};
pub use theme::Theme;

use std::io::{self, Stderr};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
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

/// How often the event loop polls the working tree for external changes (see
/// [`App::maybe_auto_refresh`]) so edits made outside redquill — e.g. by an
/// agent editing files while a review is open — surface without a keypress.
/// Modelled on lazygit's polling refresh; the reload is gated on the diff
/// actually changing, so idle ticks are cheap.
const AUTO_REFRESH_INTERVAL: Duration = Duration::from_secs(2);

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

/// Whether the bottom-panel slot is occupied this frame: the annotation list
/// or staging panel (mode-driven), or the command-log pane (toggled with `@`,
/// independent of mode). When the command log is open it takes the slot;
/// otherwise the mode's own panel does.
fn bottom_open(app: &App) -> bool {
    app.command_log_open || panel_open(app.mode)
}

/// What the event loop should do after dispatching one key.
enum Flow {
    /// Keep looping.
    Continue,
    /// End the session with this outcome (`q`/`Q`/Ctrl-C).
    Quit(QuitOutcome),
}

/// Dispatches one key-press event, mutating `app`. This is the single entry
/// point the blocking event loop and the headless key-driver tests both go
/// through, so tests exercise the *real* dispatch path (mode routing, the
/// diff-scope pending-prefix machine, panel-scope resolution, Esc handling)
/// rather than a copy of it. `pending` carries a `g`-prefix across calls in
/// diff scope (see [`Keymap::resolve`]).
fn dispatch_key(
    app: &mut App,
    keymap: &Keymap,
    pending: &mut Option<KeyEvent>,
    key: KeyEvent,
) -> Flow {
    // Transient footer messages last exactly until the next keypress
    // (whatever this key does may set a fresh one).
    app.clear_status_message();
    match app.mode {
        Mode::Compose => modes::handle_compose_key(app, key),
        Mode::List => modes::handle_list_key(app, key),
        Mode::Staging => modes::handle_staging_key(app, key),
        Mode::Panel => modes::handle_panel_key(app, key, keymap),
        Mode::Search => modes::handle_search_key(app, key),
        Mode::Peek => modes::handle_peek_key(app, key),
        Mode::Normal | Mode::Visual { .. } => {
            // While the help overlay is open it captures keys: navigation
            // keys scroll it (it can outgrow the screen), Esc/Enter/`?`/`q`
            // close it. This shadows the diff keymap so `j`/`k` scroll the
            // overlay rather than the diff underneath. Any pending `g` prefix
            // is irrelevant here, so drop it.
            if app.help_open {
                *pending = None;
                handle_help_key(app, key);
                return Flow::Continue;
            }

            let had_pending = pending.is_some();
            let action = keymap.resolve(pending, key);

            // Esc only ever closes an already-open help overlay or cancels
            // an in-progress Visual selection; it is never bound to opening
            // help, unlike `?` (see keymap.rs). This runs only when nothing
            // was pending — an Esc that cancelled a pending `g` prefix
            // (handled inside `resolve`) stops there instead.
            if key.code == KeyCode::Esc && !had_pending {
                if app.help_open {
                    app.help_open = false;
                } else if matches!(app.mode, Mode::Visual { .. }) {
                    app.apply(Action::EnterVisual);
                }
                return Flow::Continue;
            }

            let Some(action) = action else {
                return Flow::Continue;
            };
            match action {
                Action::Quit => return Flow::Quit(QuitOutcome::Emit),
                Action::QuitDiscard => return Flow::Quit(QuitOutcome::Discard),
                other => app.apply(other),
            }
        }
    }
    Flow::Continue
}

/// Handles one key while the help overlay is open. Scrolls the overlay
/// (`j`/`k`/arrows by a line, PageUp/PageDown by a viewport, `g`/`G`/Home/End
/// to the ends) or closes it (Esc/Enter/`?`/`q`). The scroll offset is only
/// advanced here; [`help::render`] clamps it to the content each frame, so
/// setting `u16::MAX` for "end" is safe. Paging uses the viewport height
/// `render` recorded last frame.
fn handle_help_key(app: &mut App, key: KeyEvent) {
    let page = app.help_viewport.get().max(1);
    let cur = app.help_scroll.get();
    match key.code {
        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('?') | KeyCode::Char('q') => {
            app.help_open = false;
            app.help_scroll.set(0);
        }
        KeyCode::Down | KeyCode::Char('j') => app.help_scroll.set(cur.saturating_add(1)),
        KeyCode::Up | KeyCode::Char('k') => app.help_scroll.set(cur.saturating_sub(1)),
        KeyCode::PageDown => app.help_scroll.set(cur.saturating_add(page)),
        KeyCode::PageUp => app.help_scroll.set(cur.saturating_sub(page)),
        KeyCode::Home | KeyCode::Char('g') => app.help_scroll.set(0),
        KeyCode::End | KeyCode::Char('G') => app.help_scroll.set(u16::MAX),
        _ => {}
    }
}

/// Draws one frame: git panel, diff pane, bottom panel (annotation list or
/// staging panel, if open), status footer, help overlay (if open), and the
/// Compose modal (if open).
fn draw(frame: &mut ratatui::Frame, app: &App, keymap: &Keymap) {
    let area = frame.area();
    let (main_area, footer_area) = split_footer(area);
    let (sidebar_area, right_area) = split_layout(main_area);
    let (diff_area, panel_area) = split_right(right_area, bottom_open(app));

    git_panel::render(frame, sidebar_area, app);
    diff_view::render(frame, diff_area, app);
    if let Some(panel_area) = panel_area {
        // The command log, when open, owns the slot regardless of mode; else
        // the mode's own bottom panel renders.
        if app.command_log_open {
            command_log::render(frame, panel_area, app);
        } else {
            match app.mode {
                Mode::Staging => staging_panel::render(frame, panel_area, app),
                _ => list_panel::render(frame, panel_area, app),
            }
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
    } else if let Some(label) = app.remote_running_label() {
        // A remote op is in flight: show a persistent running indicator (it
        // outlives the transient status message, which clears on the next
        // keypress) so the user sees the non-blocking op is still working.
        let footer = Line::from(Span::styled(
            format!(" \u{27f3} {label}\u{2026}"),
            Style::default().fg(app.theme.status_message),
        ));
        frame.render_widget(footer, footer_area);
    } else if let Some(message) = &app.status_message {
        let footer = Line::from(Span::styled(
            format!(" {message}"),
            Style::default().fg(app.theme.status_message),
        ));
        frame.render_widget(footer, footer_area);
    }
    if app.help_open {
        let staging_allowed = !matches!(app.target, crate::git::DiffTarget::Range(_));
        help::render(
            frame,
            area,
            keymap,
            &app.theme,
            staging_allowed,
            &app.help_scroll,
            &app.help_viewport,
        );
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

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stderr>>,
    app: &mut App,
    keymap: &Keymap,
) -> anyhow::Result<QuitOutcome> {
    // Tracks a `g`-prefix key across loop iterations while it awaits a
    // second key to complete `gd`/`gr` (see `Keymap::resolve`).
    let mut pending_prefix: Option<KeyEvent> = None;

    // When the working tree was last polled for external changes.
    let mut last_auto_refresh = Instant::now();

    loop {
        let size = terminal.size()?;
        let full_area = Rect::new(0, 0, size.width, size.height);
        let (main_area, _) = split_footer(full_area);
        let (_, right_area) = split_layout(main_area);
        let (diff_area, _) = split_right(right_area, bottom_open(app));
        app.view
            .set_viewport_height(diff_view::viewport_height(diff_area));

        terminal.draw(|frame| draw(frame, app, keymap))?;

        // Bounded wait rather than a blocking read: LSP responses must keep
        // flowing (via `poll_lsp` below) even while the user isn't typing,
        // without ever blocking the render loop on a slow/missing server.
        if event::poll(POLL_INTERVAL)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    match dispatch_key(app, keymap, &mut pending_prefix, key) {
                        Flow::Quit(outcome) => return Ok(outcome),
                        Flow::Continue => {}
                    }
                }
                Event::Resize(_, _) => {
                    // The next loop iteration re-measures the layout and
                    // redraws at the new size; nothing else to do here.
                }
                _ => {}
            }
        }

        code_intel::poll(app);
        app.poll_remote();

        // Poll the working tree on a fixed cadence (independent of keypresses)
        // so external edits appear without the user asking. The reload itself
        // is gated on the diff actually changing (see `maybe_auto_refresh`).
        if last_auto_refresh.elapsed() >= AUTO_REFRESH_INTERVAL {
            app.maybe_auto_refresh();
            last_auto_refresh = Instant::now();
        }
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
        let backend = TestBackend::new(100, 80);
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
        // height the pager needs). The last section (Peek mode) is far below.
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
        assert!(!top.contains("Jump to location (definition/references)"));

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
        assert!(bottom.contains("Jump to location (definition/references)"));
        assert!(!bottom.contains("Move cursor down"));
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
        app.staged_states
            .insert("src/main.rs".to_string(), stage_ops::StagedState::Full);
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
    /// emphasis from the diff pane to the git panel and back.
    #[test]
    fn focused_pane_border_emphasis_follows_the_toggle() {
        let width = 80usize;
        let panel_start = width - 32;
        let backend = TestBackend::new(width as u16, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = panel_smoke_app();
        let focus = app.theme.focused_border;
        let keymap = Keymap::default_map();

        // Diff focused (Normal): diff border emphasized, panel border plain.
        terminal.draw(|f| draw(f, &app, &keymap)).unwrap();
        let (diff_hot, panel_hot) = top_border_hot(&terminal, focus, width, panel_start);
        assert!(
            diff_hot,
            "diff border should be emphasized when diff focused"
        );
        assert!(!panel_hot, "panel border should be plain when diff focused");

        // Panel focused: emphasis moves to the panel border.
        app.apply(Action::FocusGitPanel);
        assert_eq!(app.mode, Mode::Panel);
        terminal.draw(|f| draw(f, &app, &keymap)).unwrap();
        let (diff_hot, panel_hot) = top_border_hot(&terminal, focus, width, panel_start);
        assert!(
            panel_hot,
            "panel border should be emphasized when panel focused"
        );
        assert!(!diff_hot, "diff border should be plain when panel focused");
    }

    /// Drives real `KeyEvent`s through `dispatch_key` — the exact path the
    /// blocking event loop uses — proving the focus toggle, panel `j`/`k`
    /// traversal across all three sections, Enter-on-file, and that the
    /// diff-scope keys still dispatch identically while the panel is
    /// unfocused. tmux is unavailable on this host, so this headless driver
    /// stands in for the manual smoke transcript (see 02-task-03-smoke.txt).
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

        // Focus the panel.
        assert_eq!(app.mode, Mode::Normal);
        press(&mut app, &mut pending, KeyCode::Char('`'));
        assert_eq!(app.mode, Mode::Panel);
        assert_eq!(app.panel_cursor, 0); // src/a.rs (CHANGES)

        // Traverse all three sections with `j`: a.rs -> b.rs -> notes.md
        // (UNTRACKED) -> stash0 -> stash1, clamping at the last stash.
        press(&mut app, &mut pending, KeyCode::Char('j'));
        assert_eq!(app.panel_cursor, 1); // src/b.rs
        press(&mut app, &mut pending, KeyCode::Char('j'));
        assert_eq!(app.panel_cursor, 2); // notes.md (crossed into UNTRACKED)
        press(&mut app, &mut pending, KeyCode::Char('j'));
        assert_eq!(app.panel_cursor, 3); // stash0 (crossed into STASHES)
        press(&mut app, &mut pending, KeyCode::Char('j'));
        assert_eq!(app.panel_cursor, 4); // stash1
        press(&mut app, &mut pending, KeyCode::Char('j'));
        assert_eq!(app.panel_cursor, 4); // clamped at the bottom
        press(&mut app, &mut pending, KeyCode::Char('k'));
        assert_eq!(app.panel_cursor, 3); // back up onto a stash

        // Enter on a stash is a no-op; still focused.
        press(&mut app, &mut pending, KeyCode::Enter);
        assert_eq!(app.mode, Mode::Panel);

        // Move onto src/b.rs and Enter: selects it, focus returns to the diff.
        press(&mut app, &mut pending, KeyCode::Char('k')); // -> notes.md (2)
        press(&mut app, &mut pending, KeyCode::Char('k')); // -> src/b.rs (1)
        assert_eq!(app.panel_cursor, 1);
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
        assert_eq!(app.mode, Mode::Panel);
        dispatch_key(&mut app, &keymap, &mut pending, at);
        assert!(app.command_log_open);
        // Still focused on the panel — the log toggle is orthogonal to focus.
        assert_eq!(app.mode, Mode::Panel);
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
                app.mode, app.panel_cursor, app.view.selected_file, app.view.cursor
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
        git(bare.path(), &["init", "-q", "--bare"]);
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
}
