//! The file sidebar: one row per changed file, a green `●` staged marker
//! for files with staged changes, a colored change-kind letter plus path
//! (dimmed directory, normal basename), the selected file highlighted, and
//! a footer summarizing file/staged/note counts.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};

use super::app::App;
use super::stage_ops::StagedState;
use super::theme::Theme;

/// Splits `path` into a dimmed directory prefix and a normal-weight
/// basename, e.g. `"src/auth/"` + `"session.rs"`.
fn split_path(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(idx) => (&path[..=idx], &path[idx + 1..]),
        None => ("", path),
    }
}

/// The staged-indicator column: a `●` for a fully-staged file, `±` for a
/// partially-staged one, blank otherwise, so paths stay column-aligned
/// regardless of state.
fn staged_span(state: StagedState, theme: &Theme) -> Span<'static> {
    match state {
        StagedState::Full => Span::styled("\u{25cf} ", Style::default().fg(theme.staged_indicator)),
        StagedState::Partial => {
            Span::styled("\u{00b1} ", Style::default().fg(theme.staged_indicator))
        }
        StagedState::Unstaged => Span::raw("  "),
    }
}

fn file_line(letter: char, path: &str, state: StagedState, theme: &Theme) -> Line<'static> {
    let (dir, base) = split_path(path);
    Line::from(vec![
        staged_span(state, theme),
        Span::styled(
            format!("{letter} "),
            Style::default()
                .fg(theme.letter_color(letter))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(dir.to_string(), Style::default().fg(theme.dir_prefix)),
        Span::raw(base.to_string()),
    ])
}

/// Renders the file sidebar into `area`.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    let items: Vec<ListItem> = app
        .view
        .files
        .iter()
        .map(|f| {
            let state = app.staged_states.get(&f.path).copied().unwrap_or_default();
            let line = if let Some(old) = &f.old_path {
                let (_, old_base) = split_path(old);
                let mut line = file_line(f.kind.letter(), &f.path, state, &app.theme);
                line.spans.push(Span::styled(
                    format!(" \u{2190} {old_base}"),
                    Style::default().fg(app.theme.dir_prefix),
                ));
                line
            } else {
                file_line(f.kind.letter(), &f.path, state, &app.theme)
            };
            ListItem::new(line)
        })
        .collect();

    let block = Block::default().borders(Borders::ALL).title("files");
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = ListState::default();
    if !app.view.files.is_empty() {
        state.select(Some(app.view.file_of_cursor()));
    }
    frame.render_stateful_widget(list, chunks[0], &mut state);

    let notes = app.annotations.len();
    let mut footer_text = format!(" [{} files]", app.view.files.len());
    if !app.staged.is_empty() {
        footer_text.push_str(&format!(" [{} staged]", app.staged.len()));
    }
    if notes > 0 {
        footer_text.push_str(&format!(" [{notes} notes]"));
    }
    let footer = Line::from(Span::styled(
        footer_text,
        Style::default().fg(app.theme.footer_text),
    ));
    frame.render_widget(footer, chunks[1]);
}
