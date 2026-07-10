//! The help overlay: a centered box listing every binding and its
//! description, rendered directly from the [`Keymap`] table — the same
//! source of truth the event loop dispatches from.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::keymap::Keymap;

/// Centers a `width` x `height` rect inside `area`.
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [area] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    area
}

/// Renders the help overlay, centered over `area`.
pub fn render(frame: &mut Frame, area: Rect, keymap: &Keymap) {
    let bindings = keymap.bindings();
    let key_width = bindings
        .iter()
        .map(|b| b.key_label().len())
        .max()
        .unwrap_or(0);

    let lines: Vec<Line> = bindings
        .iter()
        .map(|b| {
            Line::from(vec![
                Span::styled(
                    format!("{:>width$}", b.key_label(), width = key_width),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::raw(b.description),
            ])
        })
        .collect();

    let height = (lines.len() as u16 + 2).min(area.height);
    let width = (lines.iter().map(|l| l.width()).max().unwrap_or(0) as u16 + 4).min(area.width);
    let popup = centered(area, width, height);

    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .title("help")
        .title_alignment(Alignment::Center);
    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, popup);
}
