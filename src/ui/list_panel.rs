//! The annotation list panel: every annotation in insertion order, one line
//! each (`path:line-range (side) [classification] first line of body`),
//! scrollable, with the focused row highlighted. Toggled with `a`.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

use crate::annotate::{Annotation, Side, Target};

use super::app::App;
use super::theme::Theme;

fn side_marker(side: Side) -> &'static str {
    match side {
        Side::New => "(+)",
        Side::Old => "(-)",
    }
}

/// The `path:line-range (side)` summary of an annotation's target.
fn target_summary(target: &Target) -> String {
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
    }
}

fn item_line(annotation: &Annotation, theme: &Theme) -> Line<'static> {
    let first_line = annotation.body.lines().next().unwrap_or("");
    Line::from(vec![
        Span::raw(format!("{} ", target_summary(&annotation.target))),
        Span::styled(
            format!("[{}] ", annotation.classification.label()),
            Style::default().fg(theme.classification_tag),
        ),
        Span::raw(first_line.to_string()),
    ])
}

/// Renders the annotation list panel into `area`. An empty store renders a
/// hint line instead of an empty list.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title("notes");

    if app.annotations.is_empty() {
        let hint = Paragraph::new("no annotations yet — press c to add one").block(block);
        frame.render_widget(hint, area);
        return;
    }

    let items: Vec<ListItem> = app
        .annotations
        .iter()
        .map(|a| ListItem::new(item_line(a, &app.theme)))
        .collect();
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = ListState::default();
    state.select(Some(app.list_cursor));
    frame.render_stateful_widget(list, area, &mut state);
}
