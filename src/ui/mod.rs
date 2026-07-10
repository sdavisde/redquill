//! ratatui widgets, layout, the event loop, and the keymap. The keymap is
//! data (remappable), not hardcoded match arms scattered through widgets.
//!
//! The TUI renders to **stderr**, never stdout: stdout is reserved for the
//! annotation markdown emitted on quit (`redquill | claude -p "..."`), while
//! the TUI itself stays interactive on the terminal. [`run`] owns the whole
//! lifecycle — raw mode, alternate screen, panic-safe restoration, and the
//! blocking event loop — and returns which way the session ended.

mod app;
mod diff_view;
mod help;
mod keymap;
mod rows;
mod sidebar;

pub use app::App;
pub use keymap::{Action, Binding, Keymap};
pub use rows::{Row, build_rows};

use std::io::{self, Stderr};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// How a TUI session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitOutcome {
    /// The user pressed `q`: emit `app.annotations` to stdout.
    Emit,
    /// The user pressed `Q` or Ctrl-C: discard annotations, emit nothing.
    Discard,
}

/// Splits the full terminal area into the sidebar and diff-pane rects.
fn split_layout(area: Rect) -> (Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(32), Constraint::Min(0)])
        .split(area);
    (chunks[0], chunks[1])
}

/// Draws one frame: sidebar, diff pane, and (if open) the help overlay.
fn draw(frame: &mut ratatui::Frame, app: &App, keymap: &Keymap) {
    let area = frame.area();
    let (sidebar_area, diff_area) = split_layout(area);
    sidebar::render(frame, sidebar_area, app);
    diff_view::render(frame, diff_area, app);
    if app.help_open {
        help::render(frame, area, keymap);
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
    outcome
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stderr>>,
    app: &mut App,
    keymap: &Keymap,
) -> anyhow::Result<QuitOutcome> {
    loop {
        let size = terminal.size()?;
        let full_area = Rect::new(0, 0, size.width, size.height);
        let (_, diff_area) = split_layout(full_area);
        app.set_viewport_height(diff_view::viewport_height(diff_area));

        terminal.draw(|frame| draw(frame, app, keymap))?;

        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                // Esc only ever closes an already-open help overlay; it is
                // never bound to opening it, unlike `?` (see keymap.rs).
                if key.code == KeyCode::Esc {
                    if app.help_open {
                        app.help_open = false;
                    }
                    continue;
                }

                let Some(action) = keymap.lookup(key) else {
                    continue;
                };
                match action {
                    Action::Quit => return Ok(QuitOutcome::Emit),
                    Action::QuitDiscard => return Ok(QuitOutcome::Discard),
                    other => app.apply(other),
                }
            }
            Event::Resize(_, _) => {
                // The next loop iteration re-measures the layout and
                // redraws at the new size; nothing else to do here.
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::FileDiff;
    use crate::git::RawFilePatch;
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
}
