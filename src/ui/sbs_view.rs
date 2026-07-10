//! The side-by-side diff pane: old content on the left, new content on the
//! right, each with its own gutter, separated by a single-column divider.
//! Built entirely as a rendering-time view over the same [`super::rows::Row`]
//! model the unified pane ([`super::diff_view`]) renders — see
//! [`super::rows::build_sbs_rows`] — so this module owns no state of its
//! own and reuses that module's span-building helpers (`pub(super)` there)
//! rather than re-implementing word-diff/syntax/cursor layering.
//!
//! Every visual row is emitted as one [`Line`], with the two sides' spans
//! padded or truncated to a fixed character width (computed from the
//! pane's rendered area each frame) so the divider lines up down the
//! screen regardless of each side's content length. Full-width rows (file
//! header, hunk header, annotations, the binary placeholder) reuse
//! [`super::diff_view`]'s row renderer unchanged, so they render
//! byte-for-byte like unified view's equivalent row.

use std::collections::HashSet;

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::app::App;
use super::diff_view::{GUTTER_WIDTH, content_spans, dot_span, line_bg, origin_marker, row_line};
use super::rows::{LineRow, Row, SbsRow};
use super::theme::Theme;

/// Width, in characters, of the single-column divider between the old and
/// new panes.
const DIVIDER_WIDTH: usize = 1;

/// Splits `inner_width` (the pane's content width, borders excluded) into
/// `(left_width, right_width)`, reserving [`DIVIDER_WIDTH`] for the
/// divider. Saturates to `(0, 0)` on a pane too narrow even for the
/// divider — [`fit_width`] degrades gracefully to empty cells rather than
/// panicking.
fn column_widths(inner_width: usize) -> (usize, usize) {
    let usable = inner_width.saturating_sub(DIVIDER_WIDTH);
    let left = usable / 2;
    let right = usable - left;
    (left, right)
}

/// Truncates or right-pads `spans` to exactly `width` characters, cutting a
/// span's text mid-way if it would overshoot. Guarantees the returned
/// spans' total char count is exactly `width` (so the divider that follows
/// always lands in the same column), and never panics — a `width` of `0`
/// simply yields no spans.
fn fit_width(spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut used = 0usize;
    for span in spans {
        if used >= width {
            break;
        }
        let remaining = width - used;
        let char_count = span.content.chars().count();
        if char_count <= remaining {
            used += char_count;
            out.push(span);
        } else {
            let truncated: String = span.content.chars().take(remaining).collect();
            used += remaining;
            out.push(Span::styled(truncated, span.style));
            break;
        }
    }
    if used < width {
        out.push(Span::raw(" ".repeat(width - used)));
    }
    out
}

/// Fills in `bg` on every span in `spans` that doesn't already carry its
/// own background (word-diff-changed and column-cursor spans set their own,
/// which must win), replicating the unified pane's layering — a line-level
/// background (cursor row / search match / origin tint) underneath
/// span-level overrides — but per side rather than per whole line, since
/// only one side of a paired row may be selected at a time.
fn apply_bg_fallback(spans: Vec<Span<'static>>, bg: Option<Color>) -> Vec<Span<'static>> {
    let Some(bg) = bg else {
        return spans;
    };
    spans
        .into_iter()
        .map(|s| {
            if s.style.bg.is_none() {
                Span::styled(s.content, s.style.bg(bg))
            } else {
                s
            }
        })
        .collect()
}

/// Builds one side's cell content: gutter, origin marker, and word-diff/
/// syntax/column-cursor-layered content (via
/// [`super::diff_view::content_spans`]), fit to `width` and backgrounded
/// per [`super::diff_view::line_bg`]'s cursor/match/origin priority — an
/// empty cell (the opposite side of an unpaired line) is just blank.
#[allow(clippy::too_many_arguments)]
fn side_cell(
    line: Option<(&LineRow, Option<u32>, usize)>,
    cursor: usize,
    matches: &HashSet<usize>,
    cursor_col: Option<usize>,
    width: usize,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let Some((line, gutter, source_idx)) = line else {
        return fit_width(Vec::new(), width);
    };
    let selected = source_idx == cursor;
    let is_match = matches.contains(&source_idx);
    let gutter_style = Style::default().fg(theme.gutter);
    let mut spans = vec![
        dot_span(line.annotated, theme),
        Span::styled(diff_view_gutter_number(gutter), gutter_style),
        Span::raw(" "),
        Span::styled(
            origin_marker(line.origin),
            Style::default()
                .fg(theme.origin_fg(line.origin))
                .add_modifier(Modifier::BOLD),
        ),
    ];
    spans.extend(content_spans(
        line,
        if selected { cursor_col } else { None },
        theme,
    ));
    if line.no_newline {
        spans.push(Span::styled(" \u{2424}", Style::default().fg(theme.gutter)));
    }
    let fitted = fit_width(spans, width);
    let bg = line_bg(line.origin, selected, is_match, theme);
    apply_bg_fallback(fitted, bg)
}

/// Right-aligned gutter number, matching [`super::diff_view`]'s own (kept
/// private there); duplicated here rather than exported since it's a
/// one-line formatting helper, not shared layering logic.
fn diff_view_gutter_number(n: Option<u32>) -> String {
    match n {
        Some(n) => format!("{n:>width$}", width = GUTTER_WIDTH),
        None => " ".repeat(GUTTER_WIDTH),
    }
}

/// Renders one [`SbsRow`] as a full [`Line`]: full-width rows reuse
/// [`super::diff_view::row_line`] unchanged; split rows build a left cell,
/// a divider, and a right cell.
#[allow(clippy::too_many_arguments)]
fn sbs_row_line(
    sbs_row: &SbsRow,
    rows: &[Row],
    cursor: usize,
    matches: &HashSet<usize>,
    cursor_col: Option<usize>,
    left_width: usize,
    right_width: usize,
    theme: &Theme,
) -> Line<'static> {
    let line_row_at = |i: usize| -> Option<&LineRow> {
        match rows.get(i) {
            Some(Row::Line(l)) => Some(l),
            _ => None,
        }
    };

    match *sbs_row {
        SbsRow::Full(i) => rows
            .get(i)
            .map(|r| row_line(r, i, cursor, matches, cursor_col, theme))
            .unwrap_or_else(|| Line::from("")),
        SbsRow::Context(i) => {
            let Some(l) = line_row_at(i) else {
                return Line::from("");
            };
            let left = side_cell(
                Some((l, l.old_line, i)),
                cursor,
                matches,
                cursor_col,
                left_width,
                theme,
            );
            let right = side_cell(
                Some((l, l.new_line, i)),
                cursor,
                matches,
                cursor_col,
                right_width,
                theme,
            );
            joined_line(left, right, theme)
        }
        SbsRow::Paired { old, new } => {
            let (Some(old_line), Some(new_line)) = (line_row_at(old), line_row_at(new)) else {
                return Line::from("");
            };
            let left = side_cell(
                Some((old_line, old_line.old_line, old)),
                cursor,
                matches,
                cursor_col,
                left_width,
                theme,
            );
            let right = side_cell(
                Some((new_line, new_line.new_line, new)),
                cursor,
                matches,
                cursor_col,
                right_width,
                theme,
            );
            joined_line(left, right, theme)
        }
        SbsRow::OldOnly(i) => {
            let Some(l) = line_row_at(i) else {
                return Line::from("");
            };
            let left = side_cell(
                Some((l, l.old_line, i)),
                cursor,
                matches,
                cursor_col,
                left_width,
                theme,
            );
            let right = side_cell(None, cursor, matches, cursor_col, right_width, theme);
            joined_line(left, right, theme)
        }
        SbsRow::NewOnly(i) => {
            let Some(l) = line_row_at(i) else {
                return Line::from("");
            };
            let left = side_cell(None, cursor, matches, cursor_col, left_width, theme);
            let right = side_cell(
                Some((l, l.new_line, i)),
                cursor,
                matches,
                cursor_col,
                right_width,
                theme,
            );
            joined_line(left, right, theme)
        }
    }
}

fn joined_line(
    left: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
    theme: &Theme,
) -> Line<'static> {
    let mut spans = left;
    spans.push(Span::styled("\u{2502}", Style::default().fg(theme.gutter)));
    spans.extend(right);
    Line::from(spans)
}

/// Renders the side-by-side diff pane into `area`: a bordered block titled
/// with the selected file's path, containing the visible slice of
/// `app.sbs_rows` starting at `app.sbs_scroll`. Mirrors
/// [`super::diff_view::render`]'s empty-diff handling.
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
    let inner_width = area.width.saturating_sub(2) as usize;
    let (left_width, right_width) = column_widths(inner_width);

    let start = app.sbs_scroll;
    let end = (start + inner_height.max(1)).min(app.sbs_rows.len());
    let matches: HashSet<usize> = app.search.matches.iter().copied().collect();
    let cursor_col = app.effective_column();

    let lines: Vec<Line<'static>> = app.sbs_rows[start..end]
        .iter()
        .map(|sbs_row| {
            sbs_row_line(
                sbs_row,
                &app.rows,
                app.cursor,
                &matches,
                cursor_col,
                left_width,
                right_width,
                &app.theme,
            )
        })
        .collect();

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotate::{Classification, Target};
    use crate::diff::FileDiff;
    use crate::git::RawFilePatch;
    use crate::ui::rows::build_rows;
    use crate::ui::rows::{SyntaxSpans, build_sbs_rows};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn file_with_raw(path: &str, raw: &str) -> FileDiff {
        FileDiff::from_patch(&RawFilePatch {
            path: path.to_string(),
            old_path: None,
            raw: raw.to_string(),
            is_binary: false,
        })
        .unwrap()
    }

    fn sample_raw() -> &'static str {
        "\
diff --git a/f.rs b/f.rs
index 111..222 100644
--- a/f.rs
+++ b/f.rs
@@ -1,2 +1,2 @@
-let x = foo;
+let x = bar;
 ctx
"
    }

    #[test]
    fn fit_width_pads_short_content() {
        let spans = vec![Span::raw("hi")];
        let fitted = fit_width(spans, 5);
        let total: usize = fitted.iter().map(|s| s.content.chars().count()).sum();
        assert_eq!(total, 5);
    }

    #[test]
    fn fit_width_truncates_long_content() {
        let spans = vec![Span::raw("hello world")];
        let fitted = fit_width(spans, 5);
        let total: usize = fitted.iter().map(|s| s.content.chars().count()).sum();
        assert_eq!(total, 5);
        assert_eq!(fitted[0].content, "hello");
    }

    #[test]
    fn fit_width_zero_width_is_empty_and_does_not_panic() {
        let spans = vec![Span::raw("hello")];
        let fitted = fit_width(spans, 0);
        assert!(fitted.is_empty());
    }

    #[test]
    fn renders_both_panes_with_word_diff_visible() {
        let diff = file_with_raw("f.rs", sample_raw());
        let rows = build_rows(
            &diff,
            &crate::annotate::AnnotationStore::new(),
            SyntaxSpans::default(),
        );
        let (sbs_rows, _) = build_sbs_rows(&diff, &rows);

        let mut app = crate::ui::App::new(vec![diff]);
        app.rows = rows;
        app.sbs_rows = sbs_rows;
        app.view = crate::ui::app::ViewMode::SideBySide;

        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &app))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let content: String = buffer.content().iter().map(|c| c.symbol()).collect();

        assert!(content.contains("foo"));
        assert!(content.contains("bar"));
        assert!(content.contains("ctx"));
        // Both a "-" and a "+" origin marker should appear (old and new
        // panes both rendered).
        assert!(content.contains('-'));
        assert!(content.contains('+'));

        // The paired line's changed word ("foo"/"bar") should carry the
        // word-diff background on some cell.
        let has_word_diff_bg = buffer
            .content()
            .iter()
            .any(|cell| cell.bg == app.theme.word_diff_bg);
        assert!(has_word_diff_bg, "expected a word-diff-highlighted cell");
    }

    #[test]
    fn narrow_terminal_renders_without_panic() {
        let diff = file_with_raw("f.rs", sample_raw());
        let rows = build_rows(
            &diff,
            &crate::annotate::AnnotationStore::new(),
            SyntaxSpans::default(),
        );
        let (sbs_rows, _) = build_sbs_rows(&diff, &rows);

        let mut app = crate::ui::App::new(vec![diff]);
        app.rows = rows;
        app.sbs_rows = sbs_rows;
        app.view = crate::ui::app::ViewMode::SideBySide;

        // Under ~100 cols, per the task's narrow-terminal bar.
        let backend = TestBackend::new(30, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &app))
            .unwrap();
        // No panic is the assertion; also sanity-check something rendered.
        let buffer = terminal.backend().buffer().clone();
        let content: String = buffer.content().iter().map(|c| c.symbol()).collect();
        assert!(!content.trim().is_empty());
    }

    #[test]
    fn extremely_narrow_terminal_does_not_panic() {
        let diff = file_with_raw("f.rs", sample_raw());
        let rows = build_rows(
            &diff,
            &crate::annotate::AnnotationStore::new(),
            SyntaxSpans::default(),
        );
        let (sbs_rows, _) = build_sbs_rows(&diff, &rows);

        let mut app = crate::ui::App::new(vec![diff]);
        app.rows = rows;
        app.sbs_rows = sbs_rows;
        app.view = crate::ui::app::ViewMode::SideBySide;

        let backend = TestBackend::new(3, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &app))
            .unwrap();
    }

    #[test]
    fn annotation_row_spans_full_width_in_side_by_side() {
        let diff = file_with_raw("f.rs", sample_raw());
        let mut store = crate::annotate::AnnotationStore::new();
        store
            .add(Target::file("f.rs"), Classification::Praise, "clean")
            .unwrap();
        let rows = build_rows(&diff, &store, SyntaxSpans::default());
        let (sbs_rows, _) = build_sbs_rows(&diff, &rows);

        let mut app = crate::ui::App::new(vec![diff]);
        app.rows = rows;
        app.sbs_rows = sbs_rows;
        app.view = crate::ui::app::ViewMode::SideBySide;

        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &app))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let content: String = buffer.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("clean"));
    }
}
