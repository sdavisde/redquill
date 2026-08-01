//! The system clipboard: one thin seam around `arboard`, shared by the two
//! places redquill hands the reviewer their annotations — the in-app
//! `submit-forge-review` gesture on a non-PR target (see
//! `ui::forge_submit`'s copy path) and `main`'s on-quit presentation.
//!
//! # Why the handle is cached
//!
//! On X11 the clipboard is *served by the owning process*: the content lives
//! as long as some live `arboard::Clipboard` keeps announcing ownership, and
//! evaporates when the last one drops. `main`'s exit-time copy can get away
//! with a throwaway handle because the process is about to end either way
//! (the documented X11 caveat). A copy made from inside the running TUI
//! cannot — a handle created and dropped per keypress would leave the
//! reviewer with an empty clipboard while redquill is still on screen, which
//! is the exact case the in-app gesture exists to serve.
//!
//! So the handle is created lazily on first use and kept for the process
//! lifetime, in a `thread_local` rather than on `App`: `arboard::Clipboard`
//! is not `Sync`, both call sites run on the main thread, and keeping it out
//! of `App` avoids threading a non-`Sync` field through a struct whose other
//! members are all plain data. A failed creation is not cached — a clipboard
//! that was unavailable at one keypress (no display server yet, a
//! transiently busy selection owner) may be available at the next.
//!
//! # Error contract
//!
//! Failures surface as [`ClipboardError`] for the caller to report; nothing
//! here degrades silently, because a copy the reviewer asked for and did not
//! get is exactly the thing they must be told about. `main` falls back to
//! writing the markdown on stdout; the TUI reports the error in the footer
//! (it has no fallback — stdout belongs to the annotation format and writing
//! there mid-render would corrupt the screen).

use std::cell::RefCell;

/// A clipboard write that didn't land, carrying `arboard`'s own message —
/// the reason is always environmental (no display server, no clipboard
/// backend compiled in, a selection owner that refused), never something the
/// caller passed in, so there is nothing to distinguish beyond the text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ClipboardError(String);

thread_local! {
    /// The cached handle (see the module doc). `None` until the first
    /// successful creation.
    static HANDLE: RefCell<Option<arboard::Clipboard>> = const { RefCell::new(None) };
}

/// Copies `text` to the system clipboard, reusing this thread's cached
/// handle (creating it on first use) so the content survives for as long as
/// the process runs.
pub fn copy(text: &str) -> Result<(), ClipboardError> {
    HANDLE.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            // Not cached on failure: see the module doc.
            *slot = Some(arboard::Clipboard::new().map_err(|e| ClipboardError(e.to_string()))?);
        }
        let Some(clipboard) = slot.as_mut() else {
            // Unreachable in practice — the block above either filled the
            // slot or returned — but written as a fallback rather than an
            // `expect`, per this repo's no-panic rule.
            return Err(ClipboardError("clipboard unavailable".to_string()));
        };
        clipboard
            .set_text(text.to_owned())
            .map_err(|e| ClipboardError(e.to_string()))
    })
}
