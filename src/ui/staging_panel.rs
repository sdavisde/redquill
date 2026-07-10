//! The staging panel: every file with staged changes, one line each
//! (status letter + path), with the focused row highlighted. Toggled with
//! `s`; deliberately styled like the annotation list panel so the two feel
//! like siblings.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

use super::app::App;
use super::stage_ops::StagedFile;
use super::theme::Theme;

fn item_line(entry: &StagedFile, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{} ", entry.letter),
            Style::default()
                .fg(theme.letter_color(entry.letter))
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(entry.path.clone()),
    ])
}

/// Renders the staging panel into `area`. An empty staged list renders a
/// hint line instead of an empty list.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title("staged");

    if app.staged.is_empty() {
        let hint =
            Paragraph::new("nothing staged yet — press space on a hunk to stage it").block(block);
        frame.render_widget(hint, area);
        return;
    }

    let items: Vec<ListItem> = app
        .staged
        .iter()
        .map(|e| ListItem::new(item_line(e, &app.theme)))
        .collect();
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = ListState::default();
    state.select(Some(app.staging_cursor));
    frame.render_stateful_widget(list, area, &mut state);
}
