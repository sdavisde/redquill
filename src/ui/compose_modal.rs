//! The Compose modal: a centered overlay for creating or editing an
//! annotation. Renders the multi-line text buffer, places the terminal
//! cursor at the buffer's edit position, and shows the target/classification
//! in the title with key hints in the footer.

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Position, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::annotate::{Side, Target};

use super::app::App;
use super::textwrap;

/// The horizontal slice a 60%-wide modal occupies within `area` (full height,
/// centered). Its width feeds the wrap layout, and its `x`/`width` are shared
/// by the final popup — the vertical centering only sets `y`/`height`.
fn horizontal_slice(area: Rect) -> Rect {
    let [slice] = Layout::horizontal([Constraint::Percentage(60)])
        .flex(Flex::Center)
        .areas(area);
    slice
}

/// Centers a `height`-tall popup vertically within the (already
/// horizontally-centered) `slice`.
fn centered_in(slice: Rect, height: u16) -> Rect {
    let [popup] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(slice);
    popup
}

fn side_marker(side: Side) -> &'static str {
    match side {
        Side::New => "(+)",
        Side::Old => "(-)",
    }
}

/// The modal title's target label, e.g. `src/foo.rs:44 (+)`.
fn target_label(target: &Target) -> String {
    match target {
        Target::Line { path, line, side } => format!("{path}:{line} {}", side_marker(*side)),
        Target::Range {
            path,
            start,
            end,
            side,
        } => format!("{path}:{start}-{end} {}", side_marker(*side)),
        Target::Hunk { path, start, end } => {
            format!("{path}:{start}-{end} {}", side_marker(Side::New))
        }
        Target::File { path } => path.clone(),
        Target::WorktreeLine { path, line } => format!("{path}:{line} (=)"),
        Target::WorktreeRange { path, start, end } => format!("{path}:{start}-{end} (=)"),
    }
}

/// Renders the Compose modal, centered over `area`. A no-op if `app.compose`
/// is `None` (the caller should only invoke this in [`super::app::Mode::Compose`]).
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(compose) = &app.compose else {
        return;
    };

    let title = format!(
        "{} — {}",
        target_label(&compose.target),
        compose.classification.label()
    );
    let footer = " Enter submit  Ctrl-j newline  Ctrl-t classification  Esc cancel ";

    // Soft-wrap against the modal's inner width (60% slice minus the two
    // border columns), so the wrapped-row count sets the modal height and the
    // cursor math below shares the exact same layout.
    let slice = horizontal_slice(area);
    let wrap_width = (slice.width.saturating_sub(2)).max(1) as usize;
    let wrapped = textwrap::layout(&compose.buffer.lines, wrap_width);

    let content_height = wrapped.rows.len() as u16;
    let height = (content_height + 2)
        .max(4)
        .min(area.height.saturating_sub(2));
    let popup = centered_in(slice, height);

    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_bottom(Line::from(footer));
    let inner = block.inner(popup);

    // Scroll offset derived (not stored) from the cursor's visual row so the
    // cursor is always on screen: keep it on the last visible row once the
    // content outgrows the viewport, otherwise no scroll.
    let (cursor_vrow, cursor_vcol) =
        wrapped.cursor_position(compose.buffer.cursor_row, compose.buffer.cursor_col);
    let visible_rows = inner.height as usize;
    let scroll = cursor_vrow.saturating_sub(visible_rows.saturating_sub(1));

    let lines: Vec<Line> = wrapped
        .rows
        .iter()
        .map(|r| Line::from(textwrap::row_str(&compose.buffer.lines[r.logical_line], r)))
        .collect();
    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((scroll as u16, 0));
    frame.render_widget(paragraph, popup);

    // Place the terminal cursor at its true wrapped position minus the scroll
    // offset. The only clamp is the terminal reality that column == width has
    // no cell (right border) — not the old edge-clamp that let long lines lie.
    let cursor_x = inner.x + (cursor_vcol as u16).min(inner.width.saturating_sub(1));
    let cursor_y = inner.y + (cursor_vrow.saturating_sub(scroll)) as u16;
    frame.set_cursor_position(Position::new(cursor_x, cursor_y));
}
