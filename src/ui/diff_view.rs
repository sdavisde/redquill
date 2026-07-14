//! The diff pane: gutter with right-aligned old/new line numbers, a
//! `+`/`-`/` ` marker, and content. Content composes three color layers —
//! syntax-token foreground, a diff-origin background tint, and a stronger
//! word-diff-changed background — plus a search-match background and the
//! cursor-row highlight, all routed through [`super::theme::Theme`].

use std::collections::HashSet;
use std::ops::Range;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::annotate::Classification;
use crate::diff::{FileChangeKind, LineOrigin, WordSpan};
use crate::git::CommitLogEntry;
use crate::highlight::TokenKind;

use super::app::App;
use super::rows::{LineRow, Row, StagedMarker};
use super::theme::{Theme, blend};
use super::time_format::absolute_date;

/// Width of the annotated-line dot column, rendered before the gutter.
pub(super) const DOT_WIDTH: usize = 2;

/// Left padding for [`Row::Annotation`] display rows for a buffer whose
/// gutter is `gutter_width` columns wide, aligned under the gutter/marker
/// columns so the bullet and continuation text sit clear of the line-number
/// columns. Mirrors [`line_row_line`]'s fixed layout: dot + old gutter + " "
/// + new gutter + " " + origin marker.
fn annotation_indent(gutter_width: usize) -> usize {
    DOT_WIDTH + gutter_width * 2 + 3
}

pub(super) fn dot_span(annotated: bool, theme: &Theme) -> Span<'static> {
    let text = if annotated { "\u{25cf} " } else { "  " };
    Span::styled(text, Style::default().fg(theme.dot_marker))
}

pub(super) fn origin_marker(origin: LineOrigin) -> &'static str {
    match origin {
        LineOrigin::Added => "+",
        LineOrigin::Removed => "-",
        LineOrigin::Context => " ",
    }
}

pub(super) fn gutter_number(n: Option<u32>, gutter_width: usize) -> String {
    match n {
        Some(n) => format!("{n:>gutter_width$}"),
        None => " ".repeat(gutter_width),
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
pub(super) fn content_spans(
    row: &LineRow,
    cursor_col: Option<usize>,
    theme: &Theme,
) -> Vec<Span<'static>> {
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

/// The line-level background. The cursor row always carries a highlight,
/// but it composites *over* whatever tint the row would otherwise show
/// (search match first, then the diff-origin tint) via [`blend`], so a
/// selected added line still reads green — just brighter — and a selected
/// search match still reads as both. Unselected rows keep the match-then-
/// origin precedence; `None` if nothing applies (an unselected, unmatched
/// context line).
pub(super) fn line_bg(
    origin: LineOrigin,
    selected: bool,
    is_match: bool,
    theme: &Theme,
) -> Option<ratatui::style::Color> {
    let tint = if is_match {
        Some(theme.search_match_bg)
    } else {
        theme.origin_bg(origin)
    };
    if selected {
        Some(match tint {
            Some(tint) => blend(theme.selected_row_bg, tint),
            None => theme.selected_row_bg,
        })
    } else {
        tint
    }
}

fn line_row_line(
    row: &LineRow,
    selected: bool,
    is_match: bool,
    cursor_col: Option<usize>,
    gutter_width: usize,
    theme: &Theme,
) -> Line<'static> {
    // The cursor row's line numbers get bold + a brighter fg (same width,
    // no marker glyph) so the gutter itself signals the cursor position.
    let gutter_style = if selected {
        Style::default()
            .fg(theme.gutter_cursor_fg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.gutter)
    };
    let mut spans = vec![
        dot_span(row.annotated, theme),
        Span::styled(gutter_number(row.old_line, gutter_width), gutter_style),
        Span::raw(" "),
        Span::styled(gutter_number(row.new_line, gutter_width), gutter_style),
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

/// The staged-marker glyph shown in a section header's marker slot: `●`
/// fully staged, `±` partially staged, blank otherwise (kept width-stable
/// so headers align).
fn staged_marker_span(marker: StagedMarker, theme: &Theme) -> Span<'static> {
    match marker {
        StagedMarker::Staged => {
            Span::styled(" \u{25cf}", Style::default().fg(theme.staged_indicator))
        }
        StagedMarker::Partial => {
            Span::styled(" \u{00b1}", Style::default().fg(theme.staged_indicator))
        }
        StagedMarker::None => Span::raw("  "),
    }
}

/// Same trailing-space padding as [`annotation_row_line`] (see its doc for
/// why): without it, `Paragraph` only paints `file_header_bg` (or the
/// selected/search-match bg) onto the header's own text, leaving the rest
/// of the row showing the terminal background.
#[allow(clippy::too_many_arguments)]
fn file_header_line(
    path: &str,
    old_path: &Option<String>,
    kind: FileChangeKind,
    selected: bool,
    is_match: bool,
    annotated: bool,
    collapsed: bool,
    staged_marker: StagedMarker,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    // Collapse indicator: ▾ expanded, ▸ collapsed.
    let indicator = if collapsed { "\u{25b8} " } else { "\u{25be} " };
    let mut spans = vec![
        dot_span(annotated, theme),
        Span::styled(
            indicator.to_string(),
            Style::default()
                .fg(theme.gutter)
                .add_modifier(Modifier::BOLD),
        ),
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
    spans.push(staged_marker_span(staged_marker, theme));
    let mut line = Line::from(spans);
    let pad = width.saturating_sub(line.width());
    if pad > 0 {
        line.spans.push(Span::raw(" ".repeat(pad)));
    }
    // The cursor row's highlight composites over the header's own tint
    // (match tint first) rather than replacing it — same layering as
    // [`line_bg`].
    let tint = if is_match {
        theme.search_match_bg
    } else {
        theme.file_header_bg
    };
    line.style = Style::default().bg(if selected {
        blend(theme.selected_row_bg, tint)
    } else {
        tint
    });
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
    // Hunk headers have no standing tint of their own, so the cursor row
    // gets the plain selection highlight — blended with the match tint
    // when the header is also a search match.
    if selected {
        let bg = if is_match {
            blend(theme.selected_row_bg, theme.search_match_bg)
        } else {
            theme.selected_row_bg
        };
        line.style = Style::default().bg(bg);
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
/// is never addressable, so it's never drawn "selected". A left accent bar
/// is carved out of the leading indent's first column (rather than shifting
/// content right) and runs down every line of the block, first and
/// continuation alike, so a multi-line annotation reads as one attached
/// unit; a standing background tint fills the rest of the row. The bar
/// glyph (`│`, centered in its cell) matches the corner/edge glyphs of
/// [`annotation_border_line`] so the block's top/bottom borders visually
/// join it into one open-right outline.
///
/// Ratatui's `Paragraph` only paints a `Line`'s style onto the cells its
/// spans actually occupy — it does not extend a line's background across
/// the rest of an under-filled row. So `annotation_bg` reaches the pane's
/// right edge only because a trailing space span is appended out to `width`
/// (the pane's inner content width, see [`render`]); a row already at or
/// past `width` gets no padding (`Paragraph` truncates it regardless).
fn annotation_row_line(
    text: &str,
    classification: Option<Classification>,
    gutter_width: usize,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    let style = Style::default()
        .fg(theme.annotation_text)
        .add_modifier(Modifier::ITALIC);
    let indent = annotation_indent(gutter_width);
    let bar = Span::styled("\u{2502}", Style::default().fg(theme.annotation_accent));
    let rest = match classification {
        Some(c) => format!(
            "{}\u{25cf} [{}] {}",
            " ".repeat(indent - 1),
            c.label(),
            text
        ),
        None => format!("{}{}", " ".repeat(indent + 1), text),
    };
    let mut line = Line::from(vec![bar, Span::styled(rest, style)]);
    let pad = width.saturating_sub(line.width());
    if pad > 0 {
        line.spans.push(Span::raw(" ".repeat(pad)));
    }
    line.style = Style::default().bg(theme.annotation_bg);
    line
}

/// Renders one [`Row::AnnotationBorder`] row: a corner glyph (`╭` top,
/// `╰` bottom) at column 0 — the same column [`annotation_row_line`]'s
/// accent bar occupies, so the two visually connect — followed by `─`
/// filling the rest of the pane's content `width`. `width` is the inner
/// content width the renderer measured for the current frame (see
/// [`render`]); a `width` of `0` still renders the bare corner glyph rather
/// than panicking or producing an empty line.
fn annotation_border_line(top: bool, width: usize, theme: &Theme) -> Line<'static> {
    let corner = if top { '\u{256d}' } else { '\u{2570}' };
    let text: String = std::iter::once(corner)
        .chain(std::iter::repeat_n('\u{2500}', width.saturating_sub(1)))
        .collect();
    let mut line = Line::from(Span::styled(
        text,
        Style::default().fg(theme.annotation_accent),
    ));
    line.style = Style::default().bg(theme.annotation_bg);
    line
}

/// Renders one row (any [`Row`] variant) as a full-width [`Line`]: the
/// diff pane's own per-frame renderer. `gutter_width` is the whole
/// multibuffer's dynamic gutter digit width (see
/// [`super::rows::build_multibuffer`]), shared by every row so line-number
/// and annotation-indent columns stay aligned across files. `width` is the
/// pane's inner content width (see [`render`]), used to draw
/// [`Row::AnnotationBorder`]'s dashes across the full pane width and to pad
/// [`Row::FileHeader`] and [`Row::Annotation`] rows' trailing cells so their
/// standing background reaches the right edge (see [`file_header_line`] and
/// [`annotation_row_line`] for why the padding is needed); [`Row::Line`] and
/// [`Row::HunkHeader`] ignore it.
#[allow(clippy::too_many_arguments)]
pub(super) fn row_line(
    row: &Row,
    index: usize,
    cursor: usize,
    matches: &HashSet<usize>,
    cursor_col: Option<usize>,
    gutter_width: usize,
    width: usize,
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
            collapsed,
            staged_marker,
            ..
        } => file_header_line(
            path,
            old_path,
            *kind,
            selected,
            is_match,
            *annotated,
            *collapsed,
            *staged_marker,
            width,
            theme,
        ),
        Row::HunkHeader {
            text, annotated, ..
        } => hunk_header_line(text, selected, *annotated, is_match, theme),
        Row::Line(line) => line_row_line(
            line,
            selected,
            is_match,
            if selected { cursor_col } else { None },
            gutter_width,
            theme,
        ),
        Row::Binary => binary_line(selected, theme),
        Row::Annotation {
            text,
            classification,
            ..
        } => annotation_row_line(text, *classification, gutter_width, width, theme),
        Row::AnnotationBorder { top } => annotation_border_line(*top, width, theme),
    }
}

/// The commit-view header block (spec 05 Unit 3): short SHA, author name,
/// absolute date, and the commit subject, rendered above the diff whenever a
/// commit view (opened from the git panel's History tab) is displayed.
fn commit_header_line(commit: &CommitLogEntry, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::raw(" "),
        Span::styled(
            commit.short_sha.clone(),
            Style::default()
                .fg(theme.dir_prefix)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            commit.author_name.clone(),
            Style::default().fg(theme.footer_text),
        ),
        Span::raw("  "),
        Span::styled(
            absolute_date(commit.timestamp),
            Style::default().fg(theme.dir_prefix),
        ),
        Span::raw("  "),
        Span::styled(
            commit.subject.clone(),
            Style::default()
                .fg(theme.footer_text)
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

/// How many rows the commit-view header block reserves at the top of the
/// diff pane: one when a commit view is displayed, zero otherwise. The
/// single number [`split_area`] derives its layout from, so [`render`] and
/// [`viewport_height`] can never disagree about how much of the pane the
/// header consumes.
fn commit_header_rows(has_commit_header: bool) -> u16 {
    u16::from(has_commit_header)
}

/// Splits `area` into the commit-header rect (empty/zero-height when there is
/// no commit view) and the remaining diff-pane rect.
fn split_area(area: Rect, has_commit_header: bool) -> (Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(commit_header_rows(has_commit_header)),
            Constraint::Min(0),
        ])
        .split(area);
    (chunks[0], chunks[1])
}

/// Renders the diff pane into `area`: a bordered block titled with the
/// selected file's path, containing the visible slice of `app.view.rows`
/// starting at `app.view.scroll`. Renders a centered "no changes" message when
/// there are no files at all. When `app.active_commit` is `Some` (a commit
/// view opened from the History tab), a one-row header block (short SHA,
/// author, absolute date, subject — see [`commit_header_line`]) renders above
/// the bordered diff block instead of inside it, per spec 05 Unit 3.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let (header_area, diff_area) = split_area(area, app.active_commit.is_some());
    if let Some(commit) = &app.active_commit {
        frame.render_widget(
            Paragraph::new(commit_header_line(commit, &app.theme)),
            header_area,
        );
    }

    let title = app
        .view
        .files
        .get(app.view.selected_file)
        .map(|f| f.path.as_str())
        .unwrap_or("diff");
    let mut block = Block::default().borders(Borders::ALL).title(title);
    // The diff pane is the focused pane whenever the git panel is not.
    if !app.git_panel_focused() {
        block = block.border_style(
            Style::default()
                .fg(app.theme.focused_border)
                .add_modifier(Modifier::BOLD),
        );
    }

    if app.view.files.is_empty() {
        let paragraph = Paragraph::new("no changes")
            .block(block)
            .alignment(Alignment::Center);
        frame.render_widget(paragraph, diff_area);
        return;
    }

    let inner_height = diff_area.height.saturating_sub(2) as usize;
    let inner_width = diff_area.width.saturating_sub(2) as usize;
    let start = app.view.scroll;
    let end = (start + inner_height.max(1)).min(app.view.rows.len());
    let matches: HashSet<usize> = app.search.matches.iter().copied().collect();
    let cursor_col = app.view.effective_column();
    let lines: Vec<Line<'static>> = app.view.rows[start..end]
        .iter()
        .enumerate()
        .map(|(i, row)| {
            row_line(
                row,
                start + i,
                app.view.cursor,
                &matches,
                cursor_col,
                app.view.gutter_width,
                inner_width,
                &app.theme,
            )
        })
        .collect();

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, diff_area);
}

/// The diff pane's inner content height for a given outer `area` (accounts
/// for the block's top/bottom border, and — via [`split_area`] — the
/// commit-view header row when `has_commit_header` is set), used to keep
/// half-page motion in sync with what's actually visible.
pub fn viewport_height(area: Rect, has_commit_header: bool) -> usize {
    let (_, diff_area) = split_area(area, has_commit_header);
    diff_area.height.saturating_sub(2).max(1) as usize
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::*;

    fn sample_line_row(old_line: Option<u32>, new_line: Option<u32>) -> LineRow {
        LineRow {
            hunk_index: 0,
            old_line,
            new_line,
            origin: LineOrigin::Context,
            content: "x".to_string(),
            word_spans: None,
            no_newline: false,
            annotated: false,
            syntax_spans: None,
        }
    }

    /// Renders `spans` to a plain string, the way the terminal would show
    /// them concatenated.
    fn spans_to_string(spans: &[Span<'static>]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn gutter_number_pads_to_the_requested_width() {
        assert_eq!(gutter_number(Some(5), 3), "  5");
        assert_eq!(gutter_number(None, 4), "    ");
        assert_eq!(gutter_number(Some(1000), 4), "1000");
    }

    #[test]
    fn gutter_number_does_not_truncate_a_number_wider_than_the_column() {
        // A stale/undersized width must never cut digits off — only ever
        // pad, never truncate.
        assert_eq!(gutter_number(Some(12345), 3), "12345");
    }

    #[test]
    fn annotation_indent_grows_with_gutter_width() {
        assert_eq!(annotation_indent(3), DOT_WIDTH + 3 * 2 + 3);
        assert_eq!(annotation_indent(5), DOT_WIDTH + 5 * 2 + 3);
        assert!(annotation_indent(5) > annotation_indent(3));
    }

    #[test]
    fn line_row_line_honors_the_passed_gutter_width() {
        let row = sample_line_row(Some(7), Some(9));
        let theme = Theme::default();
        let line = line_row_line(&row, false, false, None, 4, &theme);
        let text = spans_to_string(&line.spans);
        // "   7" (old, width 4) + " " + "   9" (new, width 4) + " " + " "
        // (context marker) precede the dot's own two leading chars.
        assert!(text.contains("   7    9"));
    }

    #[test]
    fn line_row_line_at_a_wider_width_stays_aligned() {
        let row = sample_line_row(Some(7), Some(9));
        let theme = Theme::default();
        let narrow = spans_to_string(&line_row_line(&row, false, false, None, 3, &theme).spans);
        let wide = spans_to_string(&line_row_line(&row, false, false, None, 5, &theme).spans);
        // Same numbers, wider gutter: the rendered line grows by exactly
        // 2 columns per side (one per gutter column).
        assert_eq!(wide.chars().count(), narrow.chars().count() + 4);
    }

    #[test]
    fn annotation_row_line_places_accent_bar_and_bg_on_first_line() {
        let theme = Theme::default();
        let line = annotation_row_line("note", Some(Classification::Nit), 3, 40, &theme);
        assert_eq!(line.style.bg, Some(theme.annotation_bg));
        assert_eq!(line.spans[0].content.as_ref(), "\u{2502}");
        assert_eq!(line.spans[0].style.fg, Some(theme.annotation_accent));
        // The bar is carved out of the leading indent's first column, so
        // the bullet still lands at the same visual column as before.
        let indent = annotation_indent(3);
        let chars: Vec<char> = spans_to_string(&line.spans).chars().collect();
        assert_eq!(chars[0], '\u{2502}');
        assert!(chars[1..indent].iter().all(|&c| c == ' '));
        assert_eq!(chars[indent], '\u{25cf}');
    }

    #[test]
    fn annotation_row_line_places_accent_bar_on_continuation_lines() {
        let theme = Theme::default();
        let line = annotation_row_line("more text", None, 3, 40, &theme);
        assert_eq!(line.style.bg, Some(theme.annotation_bg));
        assert_eq!(line.spans[0].content.as_ref(), "\u{2502}");
        let indent = annotation_indent(3);
        let chars: Vec<char> = spans_to_string(&line.spans).chars().collect();
        assert_eq!(chars[0], '\u{2502}');
        assert!(chars[1..indent + 2].iter().all(|&c| c == ' '));
        let suffix: String = chars
            .iter()
            .skip(indent + 2)
            .take("more text".chars().count())
            .collect();
        assert_eq!(suffix, "more text");
    }

    #[test]
    fn annotation_row_line_pads_trailing_cells_to_the_pane_width() {
        let theme = Theme::default();
        // Both the tagged first line and an untagged continuation line must
        // reach exactly `width` display columns, so `annotation_bg` (applied
        // by Paragraph only to occupied cells) reaches the pane's right edge.
        let first = annotation_row_line("note", Some(Classification::Nit), 3, 40, &theme);
        assert_eq!(first.width(), 40);
        let continuation = annotation_row_line("more text", None, 3, 40, &theme);
        assert_eq!(continuation.width(), 40);
        // The padding is plain text appended as its own trailing span, not a
        // change to existing spans' content.
        let last = first.spans.last().expect("padding span present");
        assert!(last.content.chars().all(|c| c == ' '));
    }

    #[test]
    fn annotation_row_line_skips_padding_when_content_already_fills_or_exceeds_width() {
        let theme = Theme::default();
        // A width narrower than (or equal to) the row's own content must
        // not panic and must add no padding span.
        let unpadded = annotation_row_line("note", Some(Classification::Nit), 3, 0, &theme);
        assert_eq!(unpadded.spans.len(), 2);
        let content_width = unpadded.width();
        let exact =
            annotation_row_line("note", Some(Classification::Nit), 3, content_width, &theme);
        assert_eq!(exact.spans.len(), 2);
    }

    #[test]
    fn annotation_border_line_top_uses_top_left_corner_and_fills_width() {
        let theme = Theme::default();
        let line = annotation_border_line(true, 10, &theme);
        assert_eq!(line.style.bg, Some(theme.annotation_bg));
        let text = spans_to_string(&line.spans);
        assert_eq!(
            text,
            "\u{256d}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}"
        );
        assert_eq!(line.spans[0].style.fg, Some(theme.annotation_accent));
    }

    #[test]
    fn annotation_border_line_bottom_uses_bottom_left_corner() {
        let theme = Theme::default();
        let line = annotation_border_line(false, 5, &theme);
        let text = spans_to_string(&line.spans);
        assert_eq!(text, "\u{2570}\u{2500}\u{2500}\u{2500}\u{2500}");
    }

    #[test]
    fn annotation_border_line_handles_zero_width_without_panicking() {
        let theme = Theme::default();
        let line = annotation_border_line(true, 0, &theme);
        let text = spans_to_string(&line.spans);
        assert_eq!(text, "\u{256d}");
    }

    #[test]
    fn file_header_line_gets_standing_bg_when_not_selected() {
        let theme = Theme::default();
        let line = file_header_line(
            "src/main.rs",
            &None,
            FileChangeKind::Modified,
            false,
            false,
            false,
            false,
            StagedMarker::None,
            40,
            &theme,
        );
        assert_eq!(line.style.bg, Some(theme.file_header_bg));
    }

    #[test]
    fn file_header_line_standing_bg_applies_when_collapsed_too() {
        let theme = Theme::default();
        let line = file_header_line(
            "src/main.rs",
            &None,
            FileChangeKind::Modified,
            false,
            false,
            false,
            true,
            StagedMarker::None,
            40,
            &theme,
        );
        assert_eq!(line.style.bg, Some(theme.file_header_bg));
    }

    #[test]
    fn file_header_line_pads_trailing_cells_to_the_pane_width() {
        let theme = Theme::default();
        let line = file_header_line(
            "src/main.rs",
            &None,
            FileChangeKind::Modified,
            false,
            false,
            false,
            false,
            StagedMarker::None,
            60,
            &theme,
        );
        assert_eq!(line.width(), 60);
    }

    #[test]
    fn file_header_line_skips_padding_when_content_already_fills_or_exceeds_width() {
        let theme = Theme::default();
        let unpadded = file_header_line(
            "src/main.rs",
            &None,
            FileChangeKind::Modified,
            false,
            false,
            false,
            false,
            StagedMarker::None,
            0,
            &theme,
        );
        let content_width = unpadded.width();
        let exact = file_header_line(
            "src/main.rs",
            &None,
            FileChangeKind::Modified,
            false,
            false,
            false,
            false,
            StagedMarker::None,
            content_width,
            &theme,
        );
        assert_eq!(exact.width(), content_width);
    }

    #[test]
    fn file_header_line_selected_blends_the_cursor_highlight_with_its_standing_bg() {
        let theme = Theme::default();
        let line = file_header_line(
            "src/main.rs",
            &None,
            FileChangeKind::Modified,
            true,
            false,
            false,
            false,
            StagedMarker::None,
            40,
            &theme,
        );
        assert_eq!(
            line.style.bg,
            Some(blend(theme.selected_row_bg, theme.file_header_bg))
        );
    }

    #[test]
    fn hunk_header_line_selected_gets_the_full_row_highlight() {
        let theme = Theme::default();
        let line = hunk_header_line("@@ -1,2 +1,2 @@", true, false, false, &theme);
        assert_eq!(line.style.bg, Some(theme.selected_row_bg));
        // Selected + search match still reads as the cursor row: neither
        // the bare match tint nor the bare selection color.
        let both = hunk_header_line("@@ -1,2 +1,2 @@", true, false, true, &theme);
        assert_ne!(both.style.bg, Some(theme.search_match_bg));
        assert_ne!(both.style.bg, Some(theme.selected_row_bg));
        assert!(both.style.bg.is_some());
    }

    #[test]
    fn line_bg_selected_blends_with_the_origin_tint_instead_of_replacing_it() {
        let theme = Theme::default();
        let sel_added = line_bg(LineOrigin::Added, true, false, &theme);
        let sel_removed = line_bg(LineOrigin::Removed, true, false, &theme);
        let sel_context = line_bg(LineOrigin::Context, true, false, &theme);
        // Context under the cursor is the plain selection highlight.
        assert_eq!(sel_context, Some(theme.selected_row_bg));
        // Added/removed keep their tint through selection: distinct from
        // the plain selection bg, from their unselected tints, and from
        // each other.
        assert_ne!(sel_added, sel_context);
        assert_ne!(sel_removed, sel_context);
        assert_ne!(sel_added, sel_removed);
        assert_ne!(sel_added, Some(theme.added_bg));
        assert_ne!(sel_removed, Some(theme.removed_bg));
        // The tint's dominant channel survives the blend: selected+added
        // is greener, selected+removed redder, than selected+context.
        let (Some(Color::Rgb(_, ag, _)), Some(Color::Rgb(rr, _, _)), Some(Color::Rgb(cr, cg, _))) =
            (sel_added, sel_removed, sel_context)
        else {
            panic!("default theme blends must stay Rgb");
        };
        assert!(ag > cg);
        assert!(rr > cr);
    }

    #[test]
    fn line_bg_selected_search_match_still_reads_as_the_cursor_row() {
        let theme = Theme::default();
        let both = line_bg(LineOrigin::Context, true, true, &theme);
        assert!(both.is_some());
        assert_ne!(both, Some(theme.search_match_bg));
        assert_ne!(both, Some(theme.selected_row_bg));
    }

    #[test]
    fn line_bg_unselected_rows_keep_match_then_origin_precedence() {
        let theme = Theme::default();
        assert_eq!(
            line_bg(LineOrigin::Added, false, true, &theme),
            Some(theme.search_match_bg)
        );
        assert_eq!(
            line_bg(LineOrigin::Added, false, false, &theme),
            Some(theme.added_bg)
        );
        assert_eq!(line_bg(LineOrigin::Context, false, false, &theme), None);
    }

    #[test]
    fn line_row_line_bolds_and_brightens_gutter_numbers_on_the_cursor_row() {
        let row = sample_line_row(Some(7), Some(9));
        let theme = Theme::default();
        let selected = line_row_line(&row, true, false, None, 3, &theme);
        for idx in [1, 3] {
            assert_eq!(selected.spans[idx].style.fg, Some(theme.gutter_cursor_fg));
            assert!(
                selected.spans[idx]
                    .style
                    .add_modifier
                    .contains(Modifier::BOLD)
            );
        }
        let unselected = line_row_line(&row, false, false, None, 3, &theme);
        for idx in [1, 3] {
            assert_eq!(unselected.spans[idx].style.fg, Some(theme.gutter));
            assert!(
                !unselected.spans[idx]
                    .style
                    .add_modifier
                    .contains(Modifier::BOLD)
            );
        }
        // The gutter never changes width, only weight and color.
        assert_eq!(
            spans_to_string(&selected.spans),
            spans_to_string(&unselected.spans)
        );
    }

    #[test]
    fn row_line_threads_gutter_width_into_line_rows() {
        let row = Row::Line(sample_line_row(Some(1), Some(2)));
        let matches = HashSet::new();
        let theme = Theme::default();
        let narrow = spans_to_string(&row_line(&row, 0, 0, &matches, None, 3, 80, &theme).spans);
        let wide = spans_to_string(&row_line(&row, 0, 0, &matches, None, 5, 80, &theme).spans);
        assert_ne!(narrow, wide);
        assert_eq!(wide.chars().count(), narrow.chars().count() + 4);
    }

    #[test]
    fn row_line_threads_width_into_annotation_border_rows_only() {
        let matches = HashSet::new();
        let theme = Theme::default();
        let border = Row::AnnotationBorder { top: true };
        let narrow = spans_to_string(&row_line(&border, 0, 0, &matches, None, 3, 6, &theme).spans);
        let wide = spans_to_string(&row_line(&border, 0, 0, &matches, None, 3, 12, &theme).spans);
        assert_eq!(narrow.chars().count(), 6);
        assert_eq!(wide.chars().count(), 12);

        // A Line row ignores `width` entirely.
        let line = Row::Line(sample_line_row(Some(1), Some(2)));
        let at_6 = spans_to_string(&row_line(&line, 0, 0, &matches, None, 3, 6, &theme).spans);
        let at_12 = spans_to_string(&row_line(&line, 0, 0, &matches, None, 3, 12, &theme).spans);
        assert_eq!(at_6, at_12);
    }
}
