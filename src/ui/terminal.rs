//! Panic-safe terminal enter/restore guard (spec §4.1 / DUW 4.1).
//!
//! T1.0 stubs the shapes below with a minimal, correctly-typed
//! implementation; T3.0 fills in the panic hook and real setup/teardown
//! ordering (see `04-tasks-first-render.md` §3.0).

use std::io::Stdout;
use std::sync::Once;

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::ui::UiError;

/// Ensures the panic hook is installed at most once across the process
/// lifetime, so repeated/nested `TerminalGuard::enter()` calls never stack
/// hooks that each re-take and re-wrap the previous one.
static PANIC_HOOK_INSTALLED: Once = Once::new();

/// Owns the ratatui [`Terminal`] for the lifetime of the TUI session. Its
/// [`Drop`] impl restores the terminal so early returns and `?` also restore
/// (FR-render-term-3).
pub struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    /// Enable raw mode, enter the alternate screen, build the backing
    /// [`Terminal`], and install the panic-restoring hook. If any step after
    /// raw mode is enabled fails, the terminal is restored before the error
    /// is returned (never `unwrap`/`expect`).
    pub fn enter() -> Result<Self, UiError> {
        // FR-render-term-1: enable raw mode and enter the alternate screen
        // on startup.
        crossterm::terminal::enable_raw_mode()?;
        if let Err(err) =
            crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen)
        {
            // Setup failed partway through; restore before surfacing the
            // error rather than leaving raw mode enabled.
            restore_terminal();
            return Err(err.into());
        }

        let backend = CrosstermBackend::new(std::io::stdout());
        let terminal = match Terminal::new(backend) {
            Ok(terminal) => terminal,
            Err(err) => {
                restore_terminal();
                return Err(err.into());
            }
        };

        // FR-render-term-2: install a panic hook — chaining the previous
        // hook — that restores the terminal before the default panic
        // message prints. Installed at most once (`Once`) so repeated or
        // nested `enter()` calls never stack hooks that each re-take and
        // re-wrap the previous one.
        PANIC_HOOK_INSTALLED.call_once(|| {
            let previous_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |panic_info| {
                restore_terminal();
                previous_hook(panic_info);
            }));
        });

        Ok(TerminalGuard { terminal })
    }

    /// Mutable access to the backing [`Terminal`] for the app loop's draw
    /// calls.
    pub fn terminal_mut(&mut self) -> &mut Terminal<CrosstermBackend<Stdout>> {
        &mut self.terminal
    }
}

impl Drop for TerminalGuard {
    // FR-render-term-3: restore the terminal on drop so early returns and
    // `?` also restore, without duplicating the teardown logic.
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// Idempotent terminal teardown: disable raw mode, leave the alternate
/// screen, and show the cursor. Callable without a live TTY (spec §4.1
/// proof) — errors (e.g. "already restored") are swallowed rather than
/// propagated, since `Drop` and the panic hook cannot return a `Result`.
// FR-render-term-1 / FR-render-term-3: this is the single teardown path
// both normal exit (via `Drop`) and the panic hook route through.
pub fn restore_terminal() {
    #[cfg(test)]
    RESTORE_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    let _ = crossterm::terminal::disable_raw_mode();
    let _ = crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::cursor::Show
    );
}

/// Test-only call counter used to observe that [`TerminalGuard`]'s `Drop`
/// impl routes through `restore_terminal()` rather than duplicating its
/// logic (FR-render-term-3), without needing a live TTY.
#[cfg(test)]
static RESTORE_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::*;

    /// Spec §4.1 proof / FR-render-term-1 / FR-render-term-3:
    /// `restore_terminal()` must be callable repeatedly, without a live TTY,
    /// and must never panic.
    #[test]
    fn restore_terminal_is_idempotent_without_a_tty() {
        // Call twice in a row; neither call should panic, both return `()`,
        // even though this test process has no controlling TTY / alternate
        // screen / raw mode active.
        restore_terminal();
        restore_terminal();
    }

    /// FR-render-term-3: `TerminalGuard`'s `Drop` impl must route teardown
    /// through `restore_terminal()` rather than duplicating the logic, so
    /// early returns and `?` restore correctly.
    #[test]
    fn terminal_guard_drop_routes_through_restore_terminal() {
        let before = RESTORE_CALLS.load(Ordering::SeqCst);

        // Constructed directly (bypassing `enter()`, which requires a live
        // TTY for raw-mode/alt-screen syscalls and is intentionally not
        // unit-tested here — spec §8 excludes interactive paths) so this
        // test exercises only the `Drop` teardown path.
        let backend = CrosstermBackend::new(std::io::stdout());
        let terminal =
            Terminal::new(backend).expect("terminal construction does not require a live TTY");
        let guard = TerminalGuard { terminal };
        drop(guard);

        let after = RESTORE_CALLS.load(Ordering::SeqCst);
        assert!(after > before, "Drop must call restore_terminal()");
    }
}
