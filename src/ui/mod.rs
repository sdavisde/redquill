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
mod editor;
mod file_finder;
mod file_finder_modal;
mod file_view;
mod footer;
mod git_panel;
mod help;
mod history;
mod keymap;
mod keymap_config;
mod list_panel;
mod lsp_config;
mod lsp_ops;
mod modal_keys;
mod modal_keys_config;
mod modes;
mod peek;
mod peek_overlay;
mod project_search;
mod project_search_view;
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
mod textwrap;
mod theme;
mod time_format;
mod welcome;

pub use app::{App, Mode};
pub use diff_view_state::DiffViewState;
pub use editor::{EditorConfigTier, EditorLaunch, resolve_editor_config_tier};
pub use keymap::{Action, Binding, Keymap};
pub use lsp_ops::LspClient;
pub use rows::{Row, build_rows};
pub use stage_ops::{ReviewError, ReviewSnapshot, StageOps, StagedFile, build_review};
pub use theme::Theme;

use std::io::{self, Stderr};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    supports_keyboard_enhancement,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::config::SidebarSide;

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

/// Sidebar width when unconfigured (`[layout] sidebar_width` unset): 30% of
/// the containing area's width, clamped to `[40, 72]` columns. The floor
/// deliberately widens the sidebar beyond the historical fixed 32 on narrow
/// terminals (136 cols and below all get 40); the cap keeps a very wide
/// terminal from dedicating an unreasonable share to the sidebar. `total` is
/// widened to `u32` before the multiply so `total * 3` can never overflow
/// `u16` — the widest input, `u16::MAX`, scales to `19660`, itself well
/// within `u16`, so the final cast back is always in range.
///
/// `configured` (`[layout] sidebar_width`, already range-validated at
/// load time — see `crate::config::LayoutConfig`) overrides this formula
/// entirely when set, further clamped to `total` here so a width wider than
/// the terminal never overflows the split (the FR's render-time clamp to
/// "available space").
fn sidebar_width(total: u16, configured: Option<u16>) -> u16 {
    match configured {
        Some(width) => width.min(total),
        None => {
            let scaled = (u32::from(total) * 3 / 10) as u16;
            scaled.clamp(40, 72)
        }
    }
}

/// Splits the main content area into the sidebar and diff-pane rects. The
/// sidebar is hidden by default and only occupies its slot while the git
/// panel is focused (`Mode::Panel`) — visibility coincides exactly with
/// focus, no separate state field. Mirrors [`split_right`]'s
/// `(area, None)`-when-hidden pattern; when hidden the diff pane gets the
/// full width. `side`/`configured_width` come from `[layout]`
/// (`crate::config::LayoutConfig`); `side` picks which edge the sidebar
/// renders against, and `configured_width` feeds [`sidebar_width`] (`None`
/// preserves today's proportional-with-clamps formula exactly).
fn split_layout(
    area: Rect,
    show_sidebar: bool,
    side: SidebarSide,
    configured_width: Option<u16>,
) -> (Option<Rect>, Rect) {
    if !show_sidebar {
        return (None, area);
    }
    let width = sidebar_width(area.width, configured_width);
    let constraints = match side {
        SidebarSide::Left => [Constraint::Length(width), Constraint::Min(0)],
        SidebarSide::Right => [Constraint::Min(0), Constraint::Length(width)],
    };
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);
    match side {
        SidebarSide::Left => (Some(chunks[0]), chunks[1]),
        SidebarSide::Right => (Some(chunks[1]), chunks[0]),
    }
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
    /// Suspend the TUI and open the configured editor on `path` (repo-
    /// relative) at `line` (1-based), then resume (`g<Space>`). `path`/`line`
    /// have already been resolved and guard-checked by
    /// [`resolve_editor_target`] by the time this variant is produced.
    OpenEditor { path: PathBuf, line: u32 },
}

/// The largest numeric prefix [`dispatch_key`]'s digit interception will
/// accumulate — a mistyped run of digits shouldn't be able to turn a single
/// keypress into a render-loop hitch on a large diff.
const MAX_COUNT: usize = 1000;

/// How many times to apply `action` for an accumulated count prefix (`None`
/// if the user typed no digits before this key). Only pure motions honor a
/// count; everything else (toggles, staging, LSP requests, mode changes,
/// `JumpToTop`/`JumpToBottom`) always applies exactly once regardless of a
/// stray accumulated count — repeating e.g. `ToggleStage` N times would just
/// flip state back and forth, and `gg`/`G` have no natural "repeat" meaning
/// (v1 does not reinterpret `3gg` as "goto line 3").
fn repeat_count(action: Action, count: Option<usize>) -> usize {
    use Action::*;
    let repeatable = matches!(
        action,
        CursorDown
            | CursorUp
            | CursorLeft
            | CursorRight
            | WordForward
            | WordBackward
            | NextHunk
            | PrevHunk
            | NextFile
            | PrevFile
            | HalfPageDown
            | HalfPageUp
            | FullPageDown
            | FullPageUp
            | SearchNext
            | SearchPrev
    );
    if repeatable {
        count.unwrap_or(1).clamp(1, MAX_COUNT)
    } else {
        1
    }
}

/// Dispatches one key-press event, mutating `app`. This is the single entry
/// point the blocking event loop and the headless key-driver tests both go
/// through, so tests exercise the *real* dispatch path (mode routing, the
/// diff-scope pending-prefix machine, panel-scope resolution, Esc handling)
/// rather than a copy of it. `pending` carries a `g`-prefix across calls in
/// diff scope (see [`Keymap::resolve`]); `pending_count` carries an
/// accumulating numeric prefix (`3`, `1` then `0`, ...) the same way, applied
/// via [`repeat_count`] once a motion resolves.
fn dispatch_key(
    app: &mut App,
    keymap: &Keymap,
    pending: &mut Option<KeyEvent>,
    pending_count: &mut Option<usize>,
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
        Mode::Finder => modes::handle_finder_key(app, key),
        Mode::ProjectSearch => modes::handle_project_search_key(app, key),
        Mode::Normal | Mode::Visual { .. } => {
            // While an overlay is open it captures keys — here that overlay
            // can only be the help overlay, since Compose and Peek have their
            // own match arms above. Navigation keys scroll it (it can outgrow
            // the screen), Esc/Enter/`?` close it (`q` is inert — an open
            // overlay never quits the app). This shadows the diff keymap so
            // `j`/`k` scroll the overlay rather than the diff underneath. Any
            // pending `g` prefix (or numeric count) is irrelevant here, so
            // drop both.
            if app.overlay_active() {
                *pending = None;
                *pending_count = None;
                handle_help_key(app, key);
                return Flow::Continue;
            }

            // Digit interception, ahead of the keymap: only while no two-key
            // prefix is already pending (a count always precedes it, e.g.
            // `3gg`, never interleaves with it). `1'..='9'` always
            // accumulate; a bare `0` is the `CursorLineStart` motion (falls
            // through unconsumed to `keymap.resolve` below) but a `0` typed
            // *after* another digit continues the count, exactly like vim.
            if pending.is_none() && key.modifiers == KeyModifiers::NONE {
                if let KeyCode::Char(c @ '1'..='9') = key.code {
                    let digit = c.to_digit(10).unwrap_or(0) as usize;
                    *pending_count = Some(
                        pending_count
                            .unwrap_or(0)
                            .saturating_mul(10)
                            .saturating_add(digit)
                            .min(MAX_COUNT),
                    );
                    return Flow::Continue;
                }
                if key.code == KeyCode::Char('0') && pending_count.is_some() {
                    *pending_count =
                        Some(pending_count.unwrap_or(0).saturating_mul(10).min(MAX_COUNT));
                    return Flow::Continue;
                }
            }

            let had_pending = pending.is_some();
            let action = keymap.resolve(pending, key);
            let just_started_sequence = !had_pending && pending.is_some();

            // Esc only ever closes an already-open help overlay, cancels an
            // in-progress Visual selection, or returns from a commit view
            // opened via the git panel's History tab (spec 05 Unit 3); it is
            // never bound to opening help, unlike `?` (see keymap.rs). This
            // runs only when nothing was pending — an Esc that cancelled a
            // pending `g` prefix (handled inside `resolve`) stops there
            // instead. Esc always cancels an in-progress count too.
            if key.code == KeyCode::Esc && !had_pending {
                *pending_count = None;
                if app.help_open {
                    app.help_open = false;
                } else if matches!(app.mode, Mode::Visual { .. }) {
                    app.apply(Action::EnterVisual);
                } else if app.viewing_file() {
                    // Checked before `viewing_commit()`: a file view opened
                    // from within a commit view suspends the commit view,
                    // not the true original state (see `ui::file_view`'s
                    // module doc), so `Esc` unwinds one layer at a time.
                    app.return_from_file_view();
                } else if app.viewing_commit() {
                    app.return_from_commit_view();
                }
                return Flow::Continue;
            }

            let Some(action) = action else {
                // A two-key sequence just starting (e.g. the `g` of `3gg`)
                // isn't "nothing happened" — the count must survive to see
                // the sequence's second key. Anything else that resolved to
                // no action (an unbound key, or a cancelled/unknown second
                // key) abandons any in-progress count, matching vim.
                if !just_started_sequence {
                    *pending_count = None;
                }
                return Flow::Continue;
            };
            let count = pending_count.take();
            match action {
                Action::Quit => return Flow::Quit(QuitOutcome::Emit),
                Action::QuitDiscard => return Flow::Quit(QuitOutcome::Discard),
                Action::OpenEditor => {
                    return match resolve_editor_target(app) {
                        Some((path, line)) => Flow::OpenEditor { path, line },
                        None => Flow::Continue,
                    };
                }
                other => {
                    for _ in 0..repeat_count(other, count) {
                        app.apply(other);
                    }
                }
            }
        }
    }
    Flow::Continue
}

/// Resolves `g<Space>`'s cursor target into a repo-relative path and
/// 1-based line to hand to [`launch_editor`], applying the same-shaped
/// guard chain [`code_intel::request`] uses for `gd`/`gr`/`K` (no repo root,
/// missing row/target, file absent from the working tree) plus its own:
/// [`Row::Binary`] is a distinct guard (a binary file has no meaningful line
/// to jump to at all, unlike a header row, which at least opens the file).
/// Every guard failure sets a footer status message via
/// [`App::set_status_message`] and returns `None`, telling the caller not to
/// launch.
fn resolve_editor_target(app: &mut App) -> Option<(PathBuf, u32)> {
    let Some(root) = app.repo_root.clone() else {
        app.set_status_message("no repo root — can't open editor");
        return None;
    };
    let Some(file) = app.view.files.get(app.view.file_of_cursor()) else {
        app.set_status_message("no file under cursor");
        return None;
    };
    if matches!(app.view.rows.get(app.view.cursor), Some(Row::Binary)) {
        app.set_status_message("can't open binary file in editor");
        return None;
    }
    let Some((path, line)) =
        targeting::editor_target_for_cursor(file, &app.view.rows, app.view.cursor)
    else {
        app.set_status_message("no target under cursor");
        return None;
    };
    if !root.join(&path).is_file() {
        app.set_status_message("file not found in working tree");
        return None;
    }
    Some((PathBuf::from(path), line))
}

/// Spawns the resolved `editor` (see [`editor::build_editor_command`] /
/// [`editor::build_from_template`]) on `rel_path` (repo-relative) at `line`,
/// with the repo root as the child's working directory so the editor opens
/// exactly the path the user sees, and inherited stdio so it takes over the
/// terminal directly. Blocks synchronously on `.status()` — the sanctioned
/// exception to "never block the render loop": the caller has already
/// suspended the TUI (`restore_terminal`) by the time this runs, so there is
/// no render loop to block. Never goes through the background git-op
/// runner; this isn't a git operation and must not be single-flighted or
/// generation-guarded like one.
fn launch_editor(
    editor: &EditorLaunch,
    repo_root: &Path,
    rel_path: &Path,
    line: u32,
) -> io::Result<()> {
    let (program, args) = match editor {
        // A config template was already validated to contain `{{filename}}`
        // (by `crate::config::EditorConfig::from_value` for `edit_at_line`,
        // or by construction for a built-in preset), so `None` here would
        // mean a defensive-fallback bug rather than a user config error —
        // still reported as an error, never a panic.
        EditorLaunch::Template(template) => editor::build_from_template(template, rel_path, line)
            .ok_or_else(|| {
            io::Error::other(format!(
                "editor template {template:?} has no {{{{filename}}}} placeholder"
            ))
        })?,
        EditorLaunch::Command(command) => editor::build_editor_command(command, rel_path, line),
    };
    Command::new(program)
        .args(args)
        .current_dir(repo_root)
        .status()?;
    Ok(())
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
    use modal_keys::{HelpAction, HelpSearchAction};

    if let Some((mut query, editing)) = app.help_search.clone() {
        if editing {
            if let Some(action) = modal_keys::resolve(&app.modal_keys.help_search, key) {
                match action {
                    HelpSearchAction::Lock => app.help_search = Some((query, false)),
                    HelpSearchAction::Clear => app.help_search = None,
                    HelpSearchAction::DeleteChar => {
                        query.pop();
                        app.help_search = Some((query, true));
                    }
                }
                return;
            }
            // Bare, unmodified `Char` extends the filter query — never
            // remappable, per the free-text-mode contract.
            if let KeyCode::Char(c) = key.code {
                query.push(c);
                app.help_search = Some((query, true));
            }
            return;
        }
        // Locked filter: `/` resumes editing it, `Esc` clears it before it
        // can reach the `Close` action below — everything else (including
        // Enter/`?`) falls through to the ordinary scroll-key resolution.
        // Not part of `HELP_SEARCH_HINTS` (that table documents only the
        // editing sub-state's keys) — this micro-state stays fixed, like the
        // help overlay's own `?`/`/` toggle chrome.
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

    let Some(action) = modal_keys::resolve(&app.modal_keys.help, key) else {
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
    let (sidebar_area, right_area) = split_layout(
        main_area,
        app.git_panel_focused(),
        app.config.layout.sidebar_side,
        app.config.layout.sidebar_width,
    );
    let (diff_area, panel_area) = split_right(right_area, bottom_open(app));

    if let Some(sidebar_area) = sidebar_area {
        git_panel::render(frame, sidebar_area, app, keymap);
    }
    // Project Search is a full-screen view (Zed-like), replacing the diff
    // pane's content rather than overlaying it — sidebar/bottom-panel areas
    // are already `None` here in this mode (`git_panel_focused()`/
    // `bottom_open()` are both false for `Mode::ProjectSearch`).
    if matches!(app.mode, Mode::ProjectSearch) {
        project_search_view::render(frame, diff_area, app);
    } else {
        diff_view::render(frame, diff_area, app, keymap);
    }
    if let Some(panel_area) = panel_area {
        // The command log, when open, owns the slot regardless of mode; else
        // the mode's own bottom panel renders.
        if app.command_log_open {
            command_log::render(frame, panel_area, app);
        } else {
            match app.mode {
                Mode::Staging => staging_panel::render(frame, panel_area, app, keymap),
                _ => list_panel::render(frame, panel_area, app, keymap),
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
    } else if let Some(notice) = app.config_warning_notice() {
        // A config-load problem (spec 07 Unit 1): dismissible (`!`) and
        // non-blocking — it never covers the diff/panel content above, only
        // this footer row — and, like every other footer message, never
        // written to stdout (stdout is reserved for the annotation
        // markdown). Outranks a transient status message so it survives the
        // ordinary idle gaps between them; `!` clears it for the session.
        let footer = Line::from(Span::styled(
            format!(" {notice} (! to dismiss)"),
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
        let staging_allowed = app.target.staging_mode() != crate::git::StagingMode::ReadOnly;
        let code_intel_allowed = app.target.supports_code_intel();
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
            pending,
            keymap,
            &app.modal_keys,
        );
        let lines = footer::render_hint_strip(&entries, footer_area.width, &app.theme);
        frame.render_widget(ratatui::widgets::Paragraph::new(lines), footer_area);
    }
    if app.help_open {
        let staging_allowed = app.target.staging_mode() != crate::git::StagingMode::ReadOnly;
        let code_intel_allowed = app.target.supports_code_intel();
        let search = app
            .help_search
            .as_ref()
            .map(|(q, editing)| (q.as_str(), *editing));
        let state = help::HelpViewState {
            scroll: &app.help_scroll,
            viewport: &app.help_viewport,
            search,
        };
        help::render(
            frame,
            area,
            &help::HelpTables {
                keymap,
                modal_keys: &app.modal_keys,
            },
            &app.theme,
            staging_allowed,
            code_intel_allowed,
            &state,
        );
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
    if matches!(app.mode, Mode::Finder) {
        file_finder_modal::render(frame, area, app);
    }
}

/// Whether the kitty keyboard-enhancement flags were successfully pushed by
/// [`init_terminal`] and therefore need popping by [`restore_terminal`]. A
/// process-global (rather than a field) because the panic hook restores
/// through the argument-less [`restore_terminal`] and must know whether a pop
/// is owed. Set at most once per session; [`restore_terminal`] swaps it back
/// to `false` so a second restore (panic + normal exit) can't double-pop.
static KEYBOARD_ENHANCED: AtomicBool = AtomicBool::new(false);

/// Puts the terminal into raw mode + alternate screen, on stderr.
///
/// When the terminal advertises keyboard-enhancement support (kitty keyboard
/// protocol), this also pushes [`KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES`]
/// so modified keys the legacy encoding can't distinguish — most importantly
/// `Shift+Enter` (newline in the modals) vs. plain `Enter` (submit) — arrive
/// as separate events. This is an **optional enhancement that degrades
/// silently**: if support is absent or the push fails, the session runs on the
/// legacy encoding (where `Shift+Enter` is indistinguishable from `Enter` and
/// therefore submits — `Ctrl-j` remains the universal newline fallback). The
/// push is on the same stream the TUI renders to (stderr) and is paired with a
/// pop in [`restore_terminal`], performed before leaving the alternate screen.
fn init_terminal() -> io::Result<Terminal<CrosstermBackend<Stderr>>> {
    enable_raw_mode()?;
    execute!(io::stderr(), EnterAlternateScreen)?;
    // Optional enhancement; any failure leaves the session on the legacy
    // encoding (see the doc comment above). Never surfaced to the user.
    if matches!(supports_keyboard_enhancement(), Ok(true))
        && execute!(
            io::stderr(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )
        .is_ok()
    {
        KEYBOARD_ENHANCED.store(true, Ordering::SeqCst);
    }
    Terminal::new(CrosstermBackend::new(io::stderr()))
}

/// Restores the terminal to its normal state. Safe to call more than once
/// and safe to call from a panic hook.
fn restore_terminal() {
    // Pop the keyboard-enhancement flags before leaving the alternate screen,
    // and only if we actually pushed them. `swap` makes this idempotent: a
    // second restore (e.g. panic hook then normal exit) sees `false` and skips.
    if KEYBOARD_ENHANCED.swap(false, Ordering::SeqCst) {
        let _ = execute!(io::stderr(), PopKeyboardEnhancementFlags);
    }
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
    // Effective-keymap construction happens exactly once, here, before the
    // event loop starts: `default_map()` plus `[keys.diff]`/`[keys.panel]`
    // config overrides (spec 07 Unit 4). Merge-time warnings (unknown
    // action names, same-scope collisions) join the config-load warnings
    // `main` already collected via `App::set_config`, so both surface
    // through the same dismissible status-line notice.
    let (keymap, keymap_warnings) = keymap_config::effective_keymap(&app.config.keys);
    app.config_warnings.extend(keymap_warnings);
    // Same one-shot construction for every modal mode's table (spec 07 Unit
    // 4 task 5.3/5.4): `modal_keys::ModalKeymaps::default()` (the compiled-in
    // defaults) with each `[keys.<mode>]` override applied, stored on `app`
    // so every modal handler and render call reads a plain owned table.
    let (modal_keymaps, modal_warnings) = modal_keys_config::effective_modal_keys(&app.config.keys);
    app.modal_keys = modal_keymaps;
    app.config_warnings.extend(modal_warnings);
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

    // Tracks an accumulating numeric count (`3`, `1`+`0`, ...) across loop
    // iterations while it awaits the motion key it will repeat (see
    // `dispatch_key`'s digit interception).
    let mut pending_count: Option<usize> = None;

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
        let (_, right_area) = split_layout(
            main_area,
            app.git_panel_focused(),
            app.config.layout.sidebar_side,
            app.config.layout.sidebar_width,
        );
        let (diff_area, _) = split_right(right_area, bottom_open(app));
        app.view.set_viewport_height(diff_view::viewport_height(
            diff_area,
            app.active_commit.is_some(),
        ));

        terminal.draw(|frame| draw(frame, app, keymap, pending_prefix))?;

        // Bounded wait rather than a blocking read: LSP responses must keep
        // flowing (via `poll_lsp` below) even while the user isn't typing,
        // without ever blocking the render loop on a slow/missing server.
        if event::poll(POLL_INTERVAL)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    match dispatch_key(app, keymap, &mut pending_prefix, &mut pending_count, key) {
                        Flow::Quit(outcome) => return Ok(outcome),
                        Flow::Continue => {}
                        Flow::OpenEditor { path, line } => {
                            // Suspend the TUI, run the editor to completion,
                            // then resume exactly where the user was: `app`'s
                            // cursor/scroll/mode are untouched throughout, so
                            // nothing here needs to restore them.
                            restore_terminal();
                            let launch_result = match app.repo_root.clone() {
                                Some(root) => launch_editor(&app.editor, &root, &path, line),
                                None => Err(io::Error::other("no repo root — can't open editor")),
                            };
                            *terminal = init_terminal()?;
                            terminal.clear()?;
                            // A completed sequence, not a mode change — same
                            // reset the overlay-active branch above performs.
                            pending_prefix = None;
                            pending_count = None;
                            match launch_result {
                                // Re-read the working tree immediately so
                                // edits made in the editor show up now,
                                // rather than waiting for the next
                                // `AUTO_REFRESH_INTERVAL` tick.
                                Ok(()) => app.refresh(),
                                Err(e) => app.set_status_message(format!("editor failed: {e}")),
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

        code_intel::poll(app);
        app.poll_git_ops();
        // Drain any completed background working-tree read every tick (cheap,
        // non-blocking) so its result lands promptly once the worker finishes.
        app.poll_refresh();
        // Drain any completed History-tab commit-log page fetch (spec 05
        // Unit 3), same cadence as the other pollers.
        app.poll_history();
        // Drain any completed fuzzy-finder candidate-list load (spec 06
        // Unit 1), same cadence as the other pollers.
        app.poll_finder();
        // Drain Project Search's streaming scan results and fire a fresh
        // scan once its debounce elapses (spec 06 Unit 2). Runs regardless
        // of mode — kept alive while a hit's file view is showing on top
        // (see `project_search`'s module doc) — so results keep streaming
        // in behind it.
        app.poll_project_search();

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

#[cfg(test)]
#[path = "history_integration_tests.rs"]
mod history_integration_tests;

#[cfg(test)]
#[path = "file_finder_integration_tests.rs"]
mod file_finder_integration_tests;

#[cfg(test)]
#[path = "project_search_integration_tests.rs"]
mod project_search_integration_tests;
