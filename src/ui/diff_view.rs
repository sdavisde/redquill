//! The diff pane: gutter with right-aligned old/new line numbers, a
//! `+`/`-`/` ` marker, and content. Content composes three color layers —
//! syntax-token foreground, a diff-origin background tint, and a stronger
//! word-diff-changed background — plus a search-match background and the
//! cursor-row highlight, all routed through [`super::theme::Theme`].

use std::collections::HashSet;
use std::ops::Range;

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::annotate::Classification;
use crate::diff::{FileChangeKind, LineOrigin, WordSpan};
use crate::highlight::TokenKind;

use super::app::App;
use super::rows::{LineRow, Row};
use super::theme::Theme;

const GUTTER_WIDTH: usize = 5;
/// Width of the annotated-line dot column, rendered before the gutter.
const DOT_WIDTH: usize = 2;
/// Left padding for [`Row::Annotation`] display rows, aligned under the
/// gutter/marker columns so the bullet and continuation text sit clear of
/// the line-number columns.
const ANNOTATION_INDENT: usize = DOT_WIDTH + GUTTER_WIDTH * 2 + 3;

fn dot_span(annotated: bool, theme: &Theme) -> Span<'static> {
    let text = if annotated { "\u{25cf} " } else { "  " };
    Span::styled(text, Style::default().fg(theme.dot_marker))
}

fn origin_marker(origin: LineOrigin) -> &'static str {
    match origin {
        LineOrigin::Added => "+",
        LineOrigin::Removed => "-",
        LineOrigin::Context => " ",
    }
}

fn gutter_number(n: Option<u32>) -> String {
    match n {
        Some(n) => format!("{n:>width$}", width = GUTTER_WIDTH),
        None => " ".repeat(GUTTER_WIDTH),
    }
}

/// Byte offsets where either word-diff, syntax-highlight, or column-cursor
/// styling changes within a line of length `content_len`, sorted, deduped,
/// always including `0` and `content_len`, and filtered to valid char
/// boundaries of `content` so slicing on any adjacent pair is panic-safe
/// even if a span's own bounds don't quite line up (best-effort content
/// sourcing).
fn style_boundaries(
    content: &str,
    word: &[WordSpan],
    syntax: &[(Range<usize>, TokenKind)],
    cursor: Option<Range<usize>>,
) -> Vec<usize> {
    let len = content.len();
    let mut points: Vec<usize> = std::iter::once(0)
        .chain(std::iter::once(len))
        .chain(
            word.iter()
                .flat_map(|s| [s.text_range.start, s.text_range.end]),
        )
        .chain(syntax.iter().flat_map(|(r, _)| [r.start, r.end]))
        .chain(cursor.iter().flat_map(|r| [r.start, r.end]))
        .filter(|&p| p <= len && content.is_char_boundary(p))
        .collect();
    points.sort_unstable();
    points.dedup();
    points
}

/// The byte range `[start, start + len_utf8)` of the `char_idx`-th char in
/// `content`, or `None` if `content` has fewer than `char_idx + 1` chars.
fn char_byte_range(content: &str, char_idx: usize) -> Option<Range<usize>> {
    let (start, ch) = content.char_indices().nth(char_idx)?;
    Some(start..start + ch.len_utf8())
}

/// The style for the sub-range `[start, end)` (a point `start` suffices —
/// boundaries guarantee uniform styling across the whole sub-range):
/// foreground from whichever syntax span covers it (falling back to the
/// origin's base foreground), plus a bold+background treatment if it falls
/// within a changed word-diff span.
fn style_for_range(
    start: usize,
    base_fg: ratatui::style::Color,
    word: &[WordSpan],
    syntax: &[(Range<usize>, TokenKind)],
    theme: &Theme,
) -> Style {
    let syntax_kind = syntax
        .iter()
        .find(|(r, _)| r.start <= start && start < r.end)
        .map(|(_, k)| *k);
    let changed = word
        .iter()
        .any(|s| s.changed && s.text_range.start <= start && start < s.text_range.end);
    let fg = syntax_kind.map(|k| theme.token_color(k)).unwrap_or(base_fg);
    let mut style = Style::default().fg(fg);
    if changed {
        style = style.add_modifier(Modifier::BOLD).bg(theme.word_diff_bg);
    }
    style
}

/// Renders a single content line's spans, layering syntax-token
/// foregrounds under word-diff-changed spans' stronger (bold + tinted
/// background) treatment, then the column cursor's cell highlight on top
/// (`cursor_col`: a char index into `row.content`, `Some` only on the
/// cursor row).
fn content_spans(row: &LineRow, cursor_col: Option<usize>, theme: &Theme) -> Vec<Span<'static>> {
    if row.content.is_empty() {
        return vec![Span::raw(String::new())];
    }
    let word = row.word_spans.as_deref().unwrap_or(&[]);
    let syntax = row.syntax_spans.as_deref().unwrap_or(&[]);
    let base_fg = theme.origin_fg(row.origin);
    let cursor_range = cursor_col.and_then(|col| char_byte_range(&row.content, col));
    let boundaries = style_boundaries(&row.content, word, syntax, cursor_range.clone());

    let mut spans = Vec::with_capacity(boundaries.len().saturating_sub(1));
    for w in boundaries.windows(2) {
        let (start, end) = (w[0], w[1]);
        if start >= end {
            continue;
        }
        let text = row.content[start..end].to_string();
        let mut style = style_for_range(start, base_fg, word, syntax, theme);
        if let Some(cr) = &cursor_range
            && cr.start <= start
            && start < cr.end
        {
            style = style.bg(theme.column_cursor_bg);
        }
        spans.push(Span::styled(text, style));
    }
    if spans.is_empty() {
        let mut style = Style::default().fg(base_fg);
        if cursor_range.is_some_and(|cr| cr.start == 0) {
            style = style.bg(theme.column_cursor_bg);
        }
        spans.push(Span::styled(row.content.clone(), style));
    }
    spans
}

/// The line-level background, in priority order: the cursor row always
/// wins (must stay visible over everything else), then a search match,
/// then the diff-origin tint. `None` if nothing applies (an unselected,
/// unmatched context line).
fn line_bg(
    origin: LineOrigin,
    selected: bool,
    is_match: bool,
    theme: &Theme,
) -> Option<ratatui::style::Color> {
    if selected {
        Some(theme.selected_row_bg)
    } else if is_match {
        Some(theme.search_match_bg)
    } else {
        theme.origin_bg(origin)
    }
}

fn line_row_line(
    row: &LineRow,
    selected: bool,
    is_match: bool,
    cursor_col: Option<usize>,
    theme: &Theme,
) -> Line<'static> {
    let gutter_style = Style::default().fg(theme.gutter);
    let mut spans = vec![
        dot_span(row.annotated, theme),
        Span::styled(gutter_number(row.old_line), gutter_style),
        Span::raw(" "),
        Span::styled(gutter_number(row.new_line), gutter_style),
        Span::raw(" "),
        Span::styled(
            origin_marker(row.origin),
            Style::default()
                .fg(theme.origin_fg(row.origin))
                .add_modifier(Modifier::BOLD),
        ),
    ];
    spans.extend(content_spans(row, cursor_col, theme));
    if row.no_newline {
        spans.push(Span::styled(" \u{2424}", Style::default().fg(theme.gutter)));
    }
    let mut line = Line::from(spans);
    if let Some(bg) = line_bg(row.origin, selected, is_match, theme) {
        line.style = Style::default().bg(bg);
    }
    line
}

fn file_header_line(
    path: &str,
    old_path: &Option<String>,
    kind: FileChangeKind,
    selected: bool,
    annotated: bool,
    theme: &Theme,
) -> Line<'static> {
    let mut spans = vec![
        dot_span(annotated, theme),
        Span::styled(
            format!("{} ", kind.letter()),
            Style::default()
                .fg(theme.kind_color(kind))
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
        line.style = Style::default().bg(theme.selected_row_bg);
    }
    line
}

fn hunk_header_line(
    text: &str,
    selected: bool,
    annotated: bool,
    is_match: bool,
    theme: &Theme,
) -> Line<'static> {
    let spans = vec![
        dot_span(annotated, theme),
        Span::styled(
            text.to_string(),
            Style::default()
                .fg(theme.hunk_header)
                .add_modifier(Modifier::DIM),
        ),
    ];
    let mut line = Line::from(spans);
    if selected {
        line.style = Style::default().bg(theme.selected_row_bg);
    } else if is_match {
        line.style = Style::default().bg(theme.search_match_bg);
    }
    line
}

fn binary_line(selected: bool, theme: &Theme) -> Line<'static> {
    let mut line = Line::from(Span::styled(
        "binary file — content not shown",
        Style::default()
            .fg(theme.binary_placeholder)
            .add_modifier(Modifier::ITALIC),
    ));
    if selected {
        line.style = Style::default().bg(theme.selected_row_bg);
    }
    line
}

/// Renders one [`Row::Annotation`] display row: the first line of an
/// annotation's body gets the `●` marker and `[classification]` tag,
/// continuation lines are indented plain text. Always dim/italic — this row
/// is never addressable, so it's never drawn "selected".
fn annotation_row_line(
    text: &str,
    classification: Option<Classification>,
    theme: &Theme,
) -> Line<'static> {
    let style = Style::default()
        .fg(theme.annotation_text)
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

fn row_line(
    row: &Row,
    index: usize,
    cursor: usize,
    matches: &HashSet<usize>,
    cursor_col: Option<usize>,
    theme: &Theme,
) -> Line<'static> {
    let selected = index == cursor;
    let is_match = matches.contains(&index);
    match row {
        Row::FileHeader {
            path,
            old_path,
            kind,
            annotated,
        } => file_header_line(path, old_path, *kind, selected, *annotated, theme),
        Row::HunkHeader {
            text, annotated, ..
        } => hunk_header_line(text, selected, *annotated, is_match, theme),
        Row::Line(line) => line_row_line(
            line,
            selected,
            is_match,
            if selected { cursor_col } else { None },
            theme,
        ),
        Row::Binary => binary_line(selected, theme),
        Row::Annotation {
            text,
            classification,
            ..
        } => annotation_row_line(text, *classification, theme),
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
    let matches: HashSet<usize> = app.search.matches.iter().copied().collect();
    let cursor_col = app.effective_column();
    let lines: Vec<Line<'static>> = app.rows[start..end]
        .iter()
        .enumerate()
        .map(|(i, row)| row_line(row, start + i, app.cursor, &matches, cursor_col, &app.theme))
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
