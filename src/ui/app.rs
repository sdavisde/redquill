//! `App` owns the scroll offset and the parsed diff model; it is the
//! render/event driver (spec §5). T1.0 stubs the shapes and pure layout
//! helpers below; T4.0 fills in the real flattened-row model, styling, and
//! event loop (see `04-tasks-first-render.md` §4.0).

use crate::diff::{DiffFile, LineKind};
use crate::ui::UiError;

/// Owns scroll offset and the parsed model; the render/event driver.
pub struct App {
    files: Vec<DiffFile>,
    /// Line offset into the flattened diff. `usize`, not `u16`: the render
    /// path slices the visible range itself, so nothing forces ratatui's
    /// `u16` scroll bound on the model (spec §5).
    scroll: usize,
    should_quit: bool,
}

/// The `ui/` entry point: T4.0 fills in `TerminalGuard` + `Keymap` wiring and
/// the draw/event loop (FR-render-view-*, FR-render-wire-*). T1.0 stubs a
/// no-op body: construct `App` (so its private fields are genuinely read,
/// keeping the crate `dead_code`-clean without `#[allow(dead_code)]`) and
/// return immediately.
pub fn run(files: Vec<DiffFile>) -> Result<(), UiError> {
    let app = App {
        files,
        scroll: 0,
        should_quit: false,
    };
    let _ = &app.files;
    let _ = app.scroll;
    let _ = app.should_quit;
    Ok(())
}

/// Clamp a scroll offset to `[0, total_rows.saturating_sub(viewport)]`.
/// T1.0 provides a minimal, correctly-typed clamp; T4.0 exercises it against
/// the real flattened-row count (FR-render-view-5).
pub fn clamp_scroll(offset: usize, total_rows: usize, viewport: usize) -> usize {
    let max_scroll = total_rows.saturating_sub(viewport);
    offset.min(max_scroll)
}

/// The clamped `[scroll, scroll + viewport)` window into the flattened rows.
/// T1.0 provides a minimal, correctly-typed computation; T4.0 wires it to
/// the real row model (FR-render-view-5).
pub fn visible_range(scroll: usize, total_rows: usize, viewport: usize) -> std::ops::Range<usize> {
    let start = clamp_scroll(scroll, total_rows, viewport);
    let end = (start + viewport).min(total_rows);
    start..end
}

/// The gutter marker for a line's kind: `'+'` / `'-'` / `' '`
/// (FR-render-view-2).
pub fn gutter_marker(kind: LineKind) -> char {
    match kind {
        LineKind::Added => '+',
        LineKind::Removed => '-',
        LineKind::Context => ' ',
    }
}

/// The digit width of the line-number gutter column, sized to the largest
/// old/new line number across all files. T1.0 provides a minimal,
/// correctly-typed placeholder; T4.0 wires it to the real render (spec §6
/// narrow-terminal clamp).
pub fn lineno_col_width(files: &[DiffFile]) -> usize {
    let _ = files;
    1
}
