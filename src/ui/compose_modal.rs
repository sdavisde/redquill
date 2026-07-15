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

/// Centers a rect `width_pct`% wide and `height` rows tall inside `area`.
fn centered(area: Rect, width_pct: u16, height: u16) -> Rect {
    let [area] = Layout::horizontal([Constraint::Percentage(width_pct)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    area
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

    let content_height = compose.buffer.lines.len() as u16;
    let height = (content_height + 2)
        .max(4)
        .min(area.height.saturating_sub(2));
    let popup = centered(area, 60, height);

    frame.render_widget(Clear, popup);

    let lines: Vec<Line> = compose
        .buffer
        .lines
        .iter()
        .map(|l| Line::from(l.clone()))
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_bottom(Line::from(footer));
    let inner = block.inner(popup);
    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, popup);

    // Place the terminal cursor at the buffer's edit position, clamped
    // inside the inner content area so a very long line doesn't push it
    // off-screen.
    let cursor_x = inner.x + (compose.buffer.cursor_col as u16).min(inner.width.saturating_sub(1));
    let cursor_y = inner.y + (compose.buffer.cursor_row as u16).min(inner.height.saturating_sub(1));
    frame.set_cursor_position(Position::new(cursor_x, cursor_y));
}
