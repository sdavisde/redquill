//! Panic-safe terminal enter/restore guard (spec §4.1 / DUW 4.1).
//!
//! T1.0 stubs the shapes below with a minimal, correctly-typed
//! implementation; T3.0 fills in the panic hook and real setup/teardown
//! ordering (see `04-tasks-first-render.md` §3.0).

use std::io::Stdout;

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::ui::UiError;

/// Owns the ratatui [`Terminal`] for the lifetime of the TUI session. Its
/// [`Drop`] impl restores the terminal so early returns and `?` also restore
/// (FR-render-term-3).
pub struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    /// Enable raw mode, enter the alternate screen, and build the backing
    /// [`Terminal`]. T3.0 additionally installs the panic hook
    /// (FR-render-term-2) here; T1.0 stubs a minimal, always-fallible-through
    /// construction so the type compiles and returns errors (never
    /// `unwrap`/`expect`).
    pub fn enter() -> Result<Self, UiError> {
        let backend = CrosstermBackend::new(std::io::stdout());
        let terminal = Terminal::new(backend)?;
        Ok(TerminalGuard { terminal })
    }

    /// Mutable access to the backing [`Terminal`] for the app loop's draw
    /// calls.
    pub fn terminal_mut(&mut self) -> &mut Terminal<CrosstermBackend<Stdout>> {
        &mut self.terminal
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// Idempotent terminal teardown: disable raw mode and leave the alternate
/// screen. Callable without a live TTY (spec §4.1 proof) — errors (e.g.
/// "already restored") are swallowed rather than propagated, since `Drop`
/// and the panic hook (T3.0) cannot return a `Result`.
pub fn restore_terminal() {
    let _ = crossterm::terminal::disable_raw_mode();
    let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen);
}

// T3.0 (Wave 2) adds the `#[cfg(test)] mod tests` idempotent-teardown test
// here (test-first, per `04-tasks-first-render.md` §3.1) and installs the
// panic hook in `enter()` (FR-render-term-2).
