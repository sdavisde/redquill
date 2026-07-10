//! The diff pane: gutter with right-aligned old/new line numbers, a
//! `+`/`-`/` ` marker, and content — added lines green, removed red,
//! context default, word-diff-changed spans given a stronger treatment.

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::annotate::Classification;
use crate::diff::{FileChangeKind, LineOrigin, WordSpan};

use super::app::App;
use super::rows::{LineRow, Row};

const GUTTER_WIDTH: usize = 5;
/// Width of the annotated-line dot column, rendered before the gutter.
const DOT_WIDTH: usize = 2;
/// Left padding for [`Row::Annotation`] display rows, aligned under the
/// gutter/marker columns so the bullet and continuation text sit clear of
/// the line-number columns.
const ANNOTATION_INDENT: usize = DOT_WIDTH + GUTTER_WIDTH * 2 + 3;

fn dot_span(annotated: bool) -> Span<'static> {
    let text = if annotated { "\u{25cf} " } else { "  " };
    Span::styled(text, Style::default().fg(Color::Yellow))
}

fn kind_color(kind: FileChangeKind) -> Color {
    match kind {
        FileChangeKind::Added => Color::Green,
        FileChangeKind::Deleted => Color::Red,
        FileChangeKind::Modified => Color::Yellow,
        FileChangeKind::Renamed | FileChangeKind::Copied => Color::Blue,
    }
}

fn origin_marker(origin: LineOrigin) -> &'static str {
    match origin {
        LineOrigin::Added => "+",
        LineOrigin::Removed => "-",
        LineOrigin::Context => " ",
    }
}

fn origin_color(origin: LineOrigin) -> Color {
    match origin {
        LineOrigin::Added => Color::Green,
        LineOrigin::Removed => Color::Red,
        LineOrigin::Context => Color::Reset,
    }
}

fn gutter_number(n: Option<u32>) -> String {
    match n {
        Some(n) => format!("{n:>width$}", width = GUTTER_WIDTH),
        None => " ".repeat(GUTTER_WIDTH),
    }
}

/// Renders a single content line's spans, applying the base origin tint and
/// layering a stronger (bold + tinted background) treatment on any
/// word-diff-changed span.
fn content_spans(row: &LineRow) -> Vec<Span<'static>> {
    let base = Style::default().fg(origin_color(row.origin));
    match &row.word_spans {
        Some(spans) if !row.content.is_empty() => spans
            .iter()
            .map(|span: &WordSpan| {
                let text = row.content[span.text_range.clone()].to_string();
                let style = if span.changed {
                    base.add_modifier(Modifier::BOLD).bg(Color::Rgb(40, 40, 20))
                } else {
                    base
                };
                Span::styled(text, style)
            })
            .collect(),
        _ => vec![Span::styled(row.content.clone(), base)],
    }
}

fn line_row_line(row: &LineRow, selected: bool) -> Line<'static> {
    let gutter_style = Style::default().fg(Color::DarkGray);
    let mut spans = vec![
        dot_span(row.annotated),
        Span::styled(gutter_number(row.old_line), gutter_style),
        Span::raw(" "),
        Span::styled(gutter_number(row.new_line), gutter_style),
        Span::raw(" "),
        Span::styled(
            origin_marker(row.origin),
            Style::default()
                .fg(origin_color(row.origin))
                .add_modifier(Modifier::BOLD),
        ),
    ];
    spans.extend(content_spans(row));
    if row.no_newline {
        spans.push(Span::styled(
            " \u{2424}",
            Style::default().fg(Color::DarkGray),
        ));
    }
    let mut line = Line::from(spans);
    if selected {
        line.style = Style::default().bg(Color::Rgb(30, 30, 40));
    }
    line
}

fn file_header_line(
    path: &str,
    old_path: &Option<String>,
    kind: FileChangeKind,
    selected: bool,
    annotated: bool,
) -> Line<'static> {
    let mut spans = vec![
        dot_span(annotated),
        Span::styled(
            format!("{} ", kind.letter()),
            Style::default()
                .fg(kind_color(kind))
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some(old) = old_path {
        spans.push(Span::raw(format!("{old} \u{2192} {path}")));
    } else {
        spans.push(Span::styled(
            path.to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        ));
    }
    let mut line = Line::from(spans);
    if selected {
        line.style = Style::default().bg(Color::Rgb(30, 30, 40));
    }
    line
}

fn hunk_header_line(text: &str, selected: bool, annotated: bool) -> Line<'static> {
    let spans = vec![
        dot_span(annotated),
        Span::styled(
            text.to_string(),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
        ),
    ];
    let mut line = Line::from(spans);
    if selected {
        line.style = Style::default().bg(Color::Rgb(30, 30, 40));
    }
    line
}

fn binary_line(selected: bool) -> Line<'static> {
    let mut line = Line::from(Span::styled(
        "binary file — content not shown",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    ));
    if selected {
        line.style = Style::default().bg(Color::Rgb(30, 30, 40));
    }
    line
}

/// Renders one [`Row::Annotation`] display row: the first line of an
/// annotation's body gets the `●` marker and `[classification]` tag,
/// continuation lines are indented plain text. Always dim/italic — this row
/// is never addressable, so it's never drawn "selected".
fn annotation_row_line(text: &str, classification: Option<Classification>) -> Line<'static> {
    let style = Style::default()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::ITALIC);
    let content = match classification {
        Some(c) => format!(
            "{}\u{25cf} [{}] {}",
            " ".repeat(ANNOTATION_INDENT),
            c.label(),
            text
        ),
        None => format!("{}{}", " ".repeat(ANNOTATION_INDENT + 2), text),
    };
    Line::from(Span::styled(content, style))
}

fn row_line(row: &Row, index: usize, cursor: usize) -> Line<'static> {
    let selected = index == cursor;
    match row {
        Row::FileHeader {
            path,
            old_path,
            kind,
            annotated,
        } => file_header_line(path, old_path, *kind, selected, *annotated),
        Row::HunkHeader {
            text, annotated, ..
        } => hunk_header_line(text, selected, *annotated),
        Row::Line(line) => line_row_line(line, selected),
        Row::Binary => binary_line(selected),
        Row::Annotation {
            text,
            classification,
            ..
        } => annotation_row_line(text, *classification),
    }
}

/// Renders the diff pane into `area`: a bordered block titled with the
/// selected file's path, containing the visible slice of `app.rows`
/// starting at `app.scroll`. Renders a centered "no changes" message when
/// there are no files at all.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let title = app
        .files
        .get(app.selected_file)
        .map(|f| f.path.as_str())
        .unwrap_or("diff");
    let block = Block::default().borders(Borders::ALL).title(title);

    if app.files.is_empty() {
        let paragraph = Paragraph::new("no changes")
            .block(block)
            .alignment(Alignment::Center);
        frame.render_widget(paragraph, area);
        return;
    }

    let inner_height = area.height.saturating_sub(2) as usize;
    let start = app.scroll;
    let end = (start + inner_height.max(1)).min(app.rows.len());
    let lines: Vec<Line<'static>> = app.rows[start..end]
        .iter()
        .enumerate()
        .map(|(i, row)| row_line(row, start + i, app.cursor))
        .collect();

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

/// The diff pane's inner content height for a given outer `area` (accounts
/// for the block's top/bottom border), used to keep half-page motion in
/// sync with what's actually visible.
pub fn viewport_height(area: Rect) -> usize {
    area.height.saturating_sub(2).max(1) as usize
}
