//! The file sidebar: one row per changed file, a green `●` staged marker
//! for files with staged changes, a colored change-kind letter plus path
//! (dimmed directory, normal basename), the selected file highlighted, and
//! a footer summarizing file/staged/note counts.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};

use super::app::App;

/// The color convention for a change-kind letter, matching git's own
/// `--name-status` letters (plus `?` for untracked). Shared with the
/// staging panel, which shows the same letters.
pub(super) fn letter_color(letter: char) -> Color {
    match letter {
        'A' => Color::Green,
        'M' => Color::Yellow,
        'D' => Color::Red,
        'R' | 'C' => Color::Blue,
        '?' => Color::DarkGray,
        _ => Color::White,
    }
}

/// Splits `path` into a dimmed directory prefix and a normal-weight
/// basename, e.g. `"src/auth/"` + `"session.rs"`.
fn split_path(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(idx) => (&path[..=idx], &path[idx + 1..]),
        None => ("", path),
    }
}

/// The staged-indicator column: a green `●` for files with staged changes,
/// blank otherwise, so paths stay column-aligned either way.
fn staged_span(staged: bool) -> Span<'static> {
    if staged {
        Span::styled("\u{25cf} ", Style::default().fg(Color::Green))
    } else {
        Span::raw("  ")
    }
}

fn file_line(letter: char, path: &str, staged: bool) -> Line<'static> {
    let (dir, base) = split_path(path);
    Line::from(vec![
        staged_span(staged),
        Span::styled(
            format!("{letter} "),
            Style::default()
                .fg(letter_color(letter))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(dir.to_string(), Style::default().fg(Color::DarkGray)),
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
        .files
        .iter()
        .map(|f| {
            let staged = app.staged.iter().any(|s| s.path == f.path);
            let line = if let Some(old) = &f.old_path {
                let (_, old_base) = split_path(old);
                let mut line = file_line(f.kind.letter(), &f.path, staged);
                line.spans.push(Span::styled(
                    format!(" \u{2190} {old_base}"),
                    Style::default().fg(Color::DarkGray),
                ));
                line
            } else {
                file_line(f.kind.letter(), &f.path, staged)
            };
            ListItem::new(line)
        })
        .collect();

    let block = Block::default().borders(Borders::ALL).title("files");
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = ListState::default();
    if !app.files.is_empty() {
        state.select(Some(app.selected_file));
    }
    frame.render_stateful_widget(list, chunks[0], &mut state);

    let notes = app.annotations.len();
    let mut footer_text = format!(" [{} files]", app.files.len());
    if !app.staged.is_empty() {
        footer_text.push_str(&format!(" [{} staged]", app.staged.len()));
    }
    if notes > 0 {
        footer_text.push_str(&format!(" [{notes} notes]"));
    }
    let footer = Line::from(Span::styled(
        footer_text,
        Style::default().fg(Color::DarkGray),
    ));
    frame.render_widget(footer, chunks[1]);
}
