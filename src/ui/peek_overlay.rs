//! The LSP peek overlay ([`super::app::Mode::Peek`]): a centered ~70%x60%
//! overlay showing `gd`/`gr` results (a location list plus a
//! syntax-highlighted preview of the selected location) or `K` hover text,
//! without leaving the diff pane underneath.

use std::ops::Range;
use std::path::Path;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};

use crate::highlight::TokenKind;

use super::app::App;
use super::peek::{PeekKind, PeekState};
use super::theme::Theme;

/// Centers a `width_pct`% x `height_pct`% rect inside `area`.
fn centered(area: Rect, width_pct: u16, height_pct: u16) -> Rect {
    let [area] = Layout::horizontal([Constraint::Percentage(width_pct)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::vertical([Constraint::Percentage(height_pct)])
        .flex(Flex::Center)
        .areas(area);
    area
}

/// A location's `relative/path.rs:LINE` display label (1-based line),
/// relative to `root` when the path falls under it, absolute otherwise.
fn location_label(path: &Path, line: u32, root: Option<&Path>) -> String {
    let shown = root.and_then(|r| path.strip_prefix(r).ok()).unwrap_or(path);
    format!("{}:{}", shown.display(), line + 1)
}

/// Clips `range` to `content`'s valid char boundaries, mirroring the
/// clipping [`super::rows`] applies to diff-pane syntax spans — the
/// previewed file's content on disk may not exactly agree with a stale
/// highlight span in edge cases, and slicing must never panic.
fn clip_to_boundary(content: &str, range: &Range<usize>) -> Option<Range<usize>> {
    if range.start >= content.len() || !content.is_char_boundary(range.start) {
        return None;
    }
    let end = range.end.min(content.len());
    let end = (range.start..=end)
        .rev()
        .find(|&e| content.is_char_boundary(e))?;
    if end <= range.start {
        None
    } else {
        Some(range.start..end)
    }
}

/// Renders one preview line's spans: syntax-highlighted where spans cover
/// it, plain elsewhere.
fn preview_line_spans(
    content: &str,
    spans: &[(Range<usize>, TokenKind)],
    theme: &Theme,
) -> Vec<Span<'static>> {
    if spans.is_empty() {
        return vec![Span::raw(content.to_string())];
    }
    let mut out = Vec::new();
    let mut cursor = 0usize;
    for (range, kind) in spans {
        let Some(clipped) = clip_to_boundary(content, range) else {
            continue;
        };
        if clipped.start > cursor {
            out.push(Span::raw(content[cursor..clipped.start].to_string()));
        }
        out.push(Span::styled(
            content[clipped.clone()].to_string(),
            Style::default().fg(theme.token_color(*kind)),
        ));
        cursor = clipped.end.max(cursor);
    }
    if cursor < content.len() {
        out.push(Span::raw(content[cursor..].to_string()));
    }
    if out.is_empty() {
        out.push(Span::raw(String::new()));
    }
    out
}

/// The ~7-line window (centered on `target`, clipped to file bounds) of a
/// cached preview, target line highlighted.
fn preview_lines(
    lines: &[String],
    spans: &[Vec<(Range<usize>, TokenKind)>],
    target: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    if lines.is_empty() {
        return vec![Line::from(Span::styled(
            "(preview unavailable)",
            Style::default().fg(theme.annotation_text),
        ))];
    }
    let radius = 3usize;
    let start = target.saturating_sub(radius);
    let end = (target + radius + 1).min(lines.len());
    let mut out = Vec::with_capacity(end - start);
    for (idx, content) in lines.iter().enumerate().take(end).skip(start) {
        let line_spans = spans.get(idx).map(Vec::as_slice).unwrap_or(&[]);
        let mut line = Line::from(preview_line_spans(content, line_spans, theme));
        if idx == target {
            line.style = Style::default().bg(theme.column_cursor_bg);
        }
        out.push(line);
    }
    out
}

fn render_hover(frame: &mut Frame, popup: Rect, peek: &PeekState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("hover")
        .title_bottom(Line::from(" j/k scroll  Esc/q close "));
    let paragraph = Paragraph::new(peek.hover_text.clone())
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((peek.hover_scroll as u16, 0));
    frame.render_widget(paragraph, popup);
}

fn render_locations(frame: &mut Frame, popup: Rect, peek: &PeekState, app: &App) {
    let title = match peek.kind {
        PeekKind::Definition => "definition".to_string(),
        PeekKind::References => format!("references: {} results", peek.locations.len()),
        PeekKind::Hover => unreachable!("render_locations is never called for Hover"),
    };

    let (list_area, preview_area) = if popup.width >= 60 {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
            .split(popup);
        (chunks[0], chunks[1])
    } else {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(popup);
        (chunks[0], chunks[1])
    };

    let items: Vec<ListItem> = peek
        .locations
        .iter()
        .map(|loc| {
            ListItem::new(location_label(
                &loc.path,
                loc.line,
                app.repo_root.as_deref(),
            ))
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .title_bottom(Line::from(" j/k move  Enter jump  Esc/q close ")),
        )
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    let mut state = ListState::default();
    state.select(Some(peek.selected));
    frame.render_stateful_widget(list, list_area, &mut state);

    let preview_block = Block::default().borders(Borders::ALL).title("preview");
    match peek.locations.get(peek.selected) {
        Some(loc) => {
            let lines = match peek.preview_cache.get(&loc.path) {
                Some(cached) => {
                    preview_lines(&cached.lines, &cached.spans, loc.line as usize, &app.theme)
                }
                None => vec![Line::from(Span::styled(
                    "(preview unavailable)",
                    Style::default().fg(app.theme.annotation_text),
                ))],
            };
            let paragraph = Paragraph::new(lines).block(preview_block);
            frame.render_widget(paragraph, preview_area);
        }
        None => frame.render_widget(preview_block, preview_area),
    }
}

/// Renders the peek overlay, centered over `area`. A no-op if `app.peek` is
/// `None` (the caller should only invoke this in [`super::app::Mode::Peek`]).
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(peek) = &app.peek else {
        return;
    };
    let popup = centered(area, 70, 60);
    frame.render_widget(Clear, popup);

    if matches!(peek.kind, PeekKind::Hover) {
        render_hover(frame, popup, peek);
    } else {
        render_locations(frame, popup, peek, app);
    }
}
