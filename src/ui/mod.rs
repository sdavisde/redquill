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

mod annotation_list;
mod app;
mod background;
mod code_intel;
mod command_log;
mod commit_message;
mod commit_modal;
mod compose;
mod compose_modal;
mod diff_view;
mod diff_view_state;
mod footer;
mod git_panel;
mod help;
mod keymap;
mod list_panel;
mod lsp_ops;
mod modal_keys;
mod modes;
mod peek;
mod peek_overlay;
mod refresh;
mod render_glue;
mod rows;
mod search;
mod stage_ops;
mod staging;
mod staging_panel;
mod switcher;
mod switcher_modal;
mod syntax;
mod targeting;
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

/// Splits the full terminal area into the main content area and the status
/// footer at the bottom. `footer_height` is 1 row whenever the search input,
/// a remote-op spinner, or a transient status message occupies it, or 1-2
/// rows for the context-sensitive hint strip (see [`footer::footer_height`]
/// — the single place that height is computed, so this and the event loop's
/// viewport-measurement mirror never disagree).
fn split_footer(area: Rect, footer_height: u16) -> (Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(footer_height)])
        .split(area);
    (chunks[0], chunks[1])
}

/// Splits the main content area into the sidebar and diff-pane rects. The
/// sidebar is hidden by default and only occupies its slot while the git
/// panel is focused (`Mode::Panel`) — visibility coincides exactly with
/// focus, no separate state field. Mirrors [`split_right`]'s
/// `(area, None)`-when-hidden pattern; when hidden the diff pane gets the
/// full width. The sidebar renders on the right when shown; see
/// `docs/config-layer.md` for making this (and its width) configurable.
fn split_layout(area: Rect, show_sidebar: bool) -> (Option<Rect>, Rect) {
    if !show_sidebar {
        return (None, area);
    }
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(32)])
        .split(area);
    (Some(chunks[1]), chunks[0])
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
        Mode::Panel { .. } => {
            // `?` opens help from the focused git panel too (see the
            // panel-scope `ToggleHelp` row in keymap.rs); once open it
            // shadows panel dispatch exactly like the Normal/Visual overlay
            // case above, so `j`/`k`/Esc scroll/close the overlay rather than
            // moving the panel cursor underneath it.
            if app.help_open {
                handle_help_key(app, key);
                return Flow::Continue;
            }
            return modes::handle_panel_key(app, key, keymap);
        }
        Mode::Search => modes::handle_search_key(app, key),
        Mode::Peek => modes::handle_peek_key(app, key),
        Mode::Switcher => modes::handle_switcher_key(app, key),
        Mode::CommitMessage => modes::handle_commit_message_key(app, key),
        Mode::Normal | Mode::Visual { .. } => {
            // While an overlay is open it captures keys — here that overlay
            // can only be the help overlay, since Compose and Peek have their
            // own match arms above. Navigation keys scroll it (it can outgrow
            // the screen), Esc/Enter/`?` close it (`q` is inert — an open
            // overlay never quits the app). This shadows the diff keymap so
            // `j`/`k` scroll the overlay rather than the diff underneath. Any
            // pending `g` prefix is irrelevant here, so drop it.
            if app.overlay_active() {
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

/// Handles one key while the help overlay is open.
///
/// Behavior depends on [`App::help_search`], a lazygit-style keybind filter:
/// - `None` (no filter): keys resolve through [`modal_keys::HELP_KEYS`] —
///   `j`/`k`/arrows scroll by a line, PageUp/PageDown by a viewport,
///   `g`/`G`/Home/End jump to the ends, Esc/Enter/`?` close the overlay, and
///   `/` starts filter-editing (`Some((String::new(), true))`, scroll reset
///   to 0). The scroll offset is only advanced here; [`help::render`] clamps
///   it to the (possibly filtered) content each frame, so setting `u16::MAX`
///   for "end" is safe. Paging uses the viewport height `render` recorded
///   last frame.
/// - `Some((_, true))` (filter-editing): free-text input, like
///   [`modes::handle_search_key`] — printable chars extend the query,
///   Backspace shortens it, scroll keys are inert. `Enter` locks the filter
///   in (`Some((query, false))`, handing control back to the scroll keys);
///   `Esc` clears it entirely (`None`).
/// - `Some((query, false))` (a locked filter): scroll keys resolve exactly as
///   in the `None` case (including Enter/`?` closing the overlay), except
///   `/` re-opens editing on the existing `query`, and `Esc` clears the
///   filter (`None`) instead of closing — so a second `Esc` (now with no
///   filter) is what closes help.
fn handle_help_key(app: &mut App, key: KeyEvent) {
    use modal_keys::HelpAction;

    if let Some((mut query, editing)) = app.help_search.clone() {
        if editing {
            match key.code {
                KeyCode::Esc => app.help_search = None,
                KeyCode::Enter => app.help_search = Some((query, false)),
                KeyCode::Backspace => {
                    query.pop();
                    app.help_search = Some((query, true));
                }
                KeyCode::Char(c) => {
                    query.push(c);
                    app.help_search = Some((query, true));
                }
                _ => {}
            }
            return;
        }
        // Locked filter: `/` resumes editing it, `Esc` clears it before it
        // can reach the `Close` action below — everything else (including
        // Enter/`?`) falls through to the ordinary scroll-key resolution.
        match key.code {
            KeyCode::Char('/') => {
                app.help_search = Some((query, true));
                return;
            }
            KeyCode::Esc => {
                app.help_search = None;
                return;
            }
            _ => {}
        }
    }

    let Some(action) = modal_keys::resolve(modal_keys::HELP_KEYS, key) else {
        return;
    };
    let page = app.help_viewport.get().max(1);
    let cur = app.help_scroll.get();
    match action {
        HelpAction::Close => {
            app.help_open = false;
            app.help_scroll.set(0);
            app.help_search = None;
        }
        HelpAction::ScrollDown => app.help_scroll.set(cur.saturating_add(1)),
        HelpAction::ScrollUp => app.help_scroll.set(cur.saturating_sub(1)),
        HelpAction::PageDown => app.help_scroll.set(cur.saturating_add(page)),
        HelpAction::PageUp => app.help_scroll.set(cur.saturating_sub(page)),
        HelpAction::Top => app.help_scroll.set(0),
        HelpAction::Bottom => app.help_scroll.set(u16::MAX),
        HelpAction::Search => {
            app.help_search = Some((String::new(), true));
            app.help_scroll.set(0);
        }
    }
}

/// Draws one frame: git panel, diff pane, bottom panel (annotation list or
/// staging panel, if open), status footer, help overlay (if open), and the
/// Compose modal (if open). `pending` mirrors the event loop's pending
/// two-key prefix (see [`event_loop`]), so the footer's pending-completion
/// strip (`za`, `gd`/`gr`/...) matches what the next keystroke will actually
/// resolve.
fn draw(frame: &mut ratatui::Frame, app: &App, keymap: &Keymap, pending: Option<KeyEvent>) {
    let area = frame.area();
    let footer_h = footer::footer_height(area.width, app, keymap, pending);
    let (main_area, footer_area) = split_footer(area, footer_h);
    let (sidebar_area, right_area) = split_layout(main_area, app.git_panel_focused());
    let (diff_area, panel_area) = split_right(right_area, bottom_open(app));

    if let Some(sidebar_area) = sidebar_area {
        git_panel::render(frame, sidebar_area, app);
    }
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
    } else if let Some(label) = app.running_op_label() {
        // A mutating background git op is in flight: show a persistent
        // running indicator (it outlives the transient status message, which
        // clears on the next keypress) so the user sees the non-blocking op
        // is still working.
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
    } else {
        // Lowest priority: no search/remote-op/status message is active, so
        // show the context-sensitive hint strip (see `footer`).
        let staging_allowed = !matches!(app.target, crate::git::DiffTarget::Range(_));
        let entries = footer::build_hints(
            app.mode,
            staging_allowed,
            app.push_publishes(),
            app.help_open,
            pending,
            keymap,
        );
        let lines = footer::render_hint_strip(&entries, footer_area.width, &app.theme);
        frame.render_widget(ratatui::widgets::Paragraph::new(lines), footer_area);
    }
    if app.help_open {
        let staging_allowed = !matches!(app.target, crate::git::DiffTarget::Range(_));
        let search = app
            .help_search
            .as_ref()
            .map(|(q, editing)| (q.as_str(), *editing));
        let state = help::HelpViewState {
            scroll: &app.help_scroll,
            viewport: &app.help_viewport,
            search,
        };
        help::render(frame, area, keymap, &app.theme, staging_allowed, &state);
    }
    if matches!(app.mode, Mode::Compose) {
        compose_modal::render(frame, area, app);
    }
    if matches!(app.mode, Mode::Peek) {
        peek_overlay::render(frame, area, app);
    }
    if matches!(app.mode, Mode::Switcher) {
        switcher_modal::render(frame, area, app);
    }
    if matches!(app.mode, Mode::CommitMessage) {
        commit_modal::render(frame, area, app);
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
        // Mirrors `draw`'s own `footer::footer_height` call so the viewport
        // height measured here for the diff pane matches what actually
        // renders this frame (same `app`/`pending` state, same computation —
        // see `footer::footer_height`'s doc comment).
        let footer_h = footer::footer_height(full_area.width, app, keymap, pending_prefix);
        let (main_area, _) = split_footer(full_area, footer_h);
        let (_, right_area) = split_layout(main_area, app.git_panel_focused());
        let (diff_area, _) = split_right(right_area, bottom_open(app));
        app.view
            .set_viewport_height(diff_view::viewport_height(diff_area));

        terminal.draw(|frame| draw(frame, app, keymap, pending_prefix))?;

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
        app.poll_git_ops();
        // Drain any completed background working-tree read every tick (cheap,
        // non-blocking) so its result lands promptly once the worker finishes.
        app.poll_refresh();

        // Spawn a working-tree read on a fixed cadence (independent of
        // keypresses) so external edits appear without the user asking. The
        // read runs off the render thread and is gated on the diff actually
        // changing (see `maybe_auto_refresh` / `poll_refresh`).
        if last_auto_refresh.elapsed() >= AUTO_REFRESH_INTERVAL {
            app.maybe_auto_refresh();
            last_auto_refresh = Instant::now();
        }
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "git_switch_integration_tests.rs"]
mod git_switch_integration_tests;

#[cfg(test)]
#[path = "commit_integration_tests.rs"]
mod commit_integration_tests;
