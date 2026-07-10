//! ratatui widgets, layout, the event loop, and the keymap. The keymap is
//! data (remappable), not hardcoded match arms scattered through widgets.
//!
//! `ui/` renders the already-parsed [`crate::diff::DiffFile`] model handed to
//! it by `main.rs`; it never calls into `git/` (FR-render-wire-1).

pub mod app;
pub mod keymap;
pub mod terminal;

pub use app::{App, run};
pub use keymap::{Action, KeyChord, Keymap};
pub use terminal::{TerminalGuard, restore_terminal};

/// Errors surfaced by the `ui/` layer. `ui/` is library code (under
/// `src/lib.rs`), so it returns errors via `thiserror`; `anyhow` stays at the
/// binary edge (`main.rs`), which converts `UiError` via `?`.
#[derive(Debug, thiserror::Error)]
pub enum UiError {
    #[error("terminal I/O error: {0}")]
    Io(#[from] std::io::Error),
}
