//! `App` owns the scroll offset and the parsed diff model; it is the
//! render/event driver (spec §5). Fills in the flattened-row model, event
//! loop, and styling described in `04-tasks-first-render.md` §4.0 (DUW
//! 4.3/4.4).

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line as RtLine, Span};
use ratatui::widgets::Paragraph;

use crate::diff::{DiffFile, Hunk, LineKind};
use crate::ui::UiError;
use crate::ui::keymap::{Action, KeyChord, Keymap};
use crate::ui::terminal::TerminalGuard;

/// Owns scroll offset and the parsed model; the render/event driver.
pub struct App {
    files: Vec<DiffFile>,
    /// Line offset into the flattened diff. `usize`, not `u16`: the render
    /// path slices the visible range itself, so nothing forces ratatui's
    /// `u16` scroll bound on the model (spec §5).
    scroll: usize,
    should_quit: bool,
}

/// The `ui/` entry point (FR-render-wire-1/2): constructs the `Keymap` and
/// [`TerminalGuard`], then drives a render-on-event loop — never a busy
/// loop (FR-render-view-5) — until `q`/`Q` (or whatever the `Keymap` binds
/// to `Quit`/`QuitDiscard`) sets `should_quit`.
pub fn run(files: Vec<DiffFile>) -> Result<(), UiError> {
    let keymap = Keymap::default_map();
    let mut guard = TerminalGuard::enter()?;

    // `lineno_col_width` and `total_rows` depend only on the (immutable)
    // parsed model, not on viewport size, so both are computed once here —
    // never recomputed per frame (spec §6 large-diff perf target).
    let lineno_width = lineno_col_width(&files);
    let total = total_rows(&files);

    let mut viewport = guard.terminal_mut().size()?.height as usize;
    let mut app = App {
        files,
        scroll: 0,
        should_quit: false,
    };
    app.scroll = clamp_scroll(app.scroll, total, viewport);

    draw(&mut guard, &app, &keymap, lineno_width)?;

    loop {
        // FR-render-view-5: block on the next event; the loop never spins.
        let event = crossterm::event::read()?;

        let mut changed = false;
        match event {
            Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                // All key behavior routes through `keymap.resolve` — no
                // `match ev.code` behavior arms live here (FR-render-keymap-1).
                let action = keymap.resolve(key_event);
                changed = apply_action(&mut app, action, total, viewport);
            }
            Event::Resize(_, height) => {
                // spec §6: resize re-renders to the new size without panic.
                viewport = height as usize;
                app.scroll = clamp_scroll(app.scroll, total, viewport);
                changed = true;
            }
            // Every other event kind (mouse, focus, paste) is ignored this
            // task — no behavior is bound to them.
            _ => {}
        }

        if app.should_quit {
            break;
        }

        // Only redraw on state change (spec §4.3 "only redraw on change").
        if changed {
            draw(&mut guard, &app, &keymap, lineno_width)?;
        }
    }

    Ok(())
}

/// Draws one frame. Kept as a thin wrapper around `Terminal::draw` so `run`'s
/// loop body stays readable; all layout/styling lives in `render_frame`.
fn draw(
    guard: &mut TerminalGuard,
    app: &App,
    keymap: &Keymap,
    lineno_width: usize,
) -> Result<(), UiError> {
    guard
        .terminal_mut()
        .draw(|frame| render_frame(frame, app, keymap, lineno_width))?;
    Ok(())
}

/// Builds and styles ONLY the visible row slice for this frame — never the
/// whole flattened buffer (spec §6 "very large diff" / FR-render-view-5
/// perf target).
fn render_frame(frame: &mut Frame, app: &App, keymap: &Keymap, lineno_width: usize) {
    // Test-only manual-verification hook for FR-render-term-2 (panic-safe
    // terminal restore). Inert unless the operator explicitly sets
    // REDQUILL_PANIC_TEST=1; see docs/specs/04-spec-first-render/04-proofs/
    // T4.0-proof.md for the exact manual verification steps this trigger
    // exists to support. Never fires in normal use (task 4.6).
    if std::env::var_os("REDQUILL_PANIC_TEST").is_some() {
        panic!("redquill: REDQUILL_PANIC_TEST fired mid-render (manual FR-render-term-2 check)");
    }

    let area = frame.area();

    if app.files.is_empty() {
        // FR-render-wire-2: an empty diff still enters the TUI and shows an
        // explicit empty-state message — no special-case bypass of the
        // render loop. The quit hint derives its key from the `Keymap`
        // rather than hardcoding a key name (FR-render-keymap-5).
        let quit_hint = keymap
            .chord_for(Action::Quit)
            .map(describe_chord)
            .unwrap_or_else(|| "the bound quit key".to_string());
        let message = format!("No changes to review. Press {quit_hint} to quit.");
        frame.render_widget(Paragraph::new(message), area);
        return;
    }

    let total = total_rows(&app.files);
    let viewport = area.height as usize;
    let visible = visible_range(app.scroll, total, viewport);
    let rows = rows_in_range(&app.files, visible);

    let lines: Vec<RtLine> = rows
        .iter()
        .map(|row| render_row(row, &app.files, lineno_width, area.width))
        .collect();

    frame.render_widget(Paragraph::new(lines), area);
}

/// Mutates `app` per `action` (FR-render-view-5) and reports whether state
/// actually changed, so `run`'s loop only redraws on change. Every action
/// not named below is an explicit no-op — bound in the `Keymap` but not
/// implemented until its owning roadmap task lands (spec §7) — never a
/// silent crash or hidden behavior (FR-render-keymap-3).
fn apply_action(app: &mut App, action: Action, total_rows: usize, viewport: usize) -> bool {
    let before = (app.scroll, app.should_quit);
    match action {
        Action::ScrollDown => {
            app.scroll = clamp_scroll(app.scroll.saturating_add(1), total_rows, viewport);
        }
        Action::ScrollUp => {
            app.scroll = clamp_scroll(app.scroll.saturating_sub(1), total_rows, viewport);
        }
        Action::HalfPageDown => {
            let half = half_page(viewport);
            app.scroll = clamp_scroll(app.scroll.saturating_add(half), total_rows, viewport);
        }
        Action::HalfPageUp => {
            let half = half_page(viewport);
            app.scroll = clamp_scroll(app.scroll.saturating_sub(half), total_rows, viewport);
        }
        Action::Quit | Action::QuitDiscard => {
            app.should_quit = true;
        }
        Action::NextHunk
        | Action::PrevHunk
        | Action::NextFile
        | Action::PrevFile
        | Action::Search
        | Action::SearchNext
        | Action::SearchPrev
        | Action::Comment
        | Action::VisualSelect
        | Action::StageToggle
        | Action::ToggleStagePanel
        | Action::GotoDefinition
        | Action::FindReferences
        | Action::Hover
        | Action::AnnotationList
        | Action::Help
        | Action::Noop => {
            // Explicit no-op (FR-render-keymap-3): bound but deferred.
        }
    }
    (app.scroll, app.should_quit) != before
}

/// Half the CURRENT viewport height, recomputed per event (spec §9 default),
/// floored at 1 so a half-page move always makes progress even on a very
/// short viewport.
fn half_page(viewport: usize) -> usize {
    (viewport / 2).max(1)
}

/// Formats a `KeyChord` for display (e.g. the empty-state quit hint). The
/// ONLY place outside `default_map` that turns a key into text, and it does
/// so generically from whatever chord `chord_for` returns — never by
/// matching a specific key name (FR-render-keymap-5).
fn describe_chord(chord: KeyChord) -> String {
    let mut out = String::new();
    if chord.modifiers.contains(KeyModifiers::CONTROL) {
        out.push_str("Ctrl-");
    }
    match chord.code {
        KeyCode::Char(c) => out.push(c),
        other => out.push_str(&format!("{other:?}")),
    }
    out
}

/// One flattened render row, addressed by (file, hunk, line) index rather
/// than pre-rendered text, so materializing the visible slice never builds
/// styled content for rows outside the current viewport (spec §6 large-diff
/// perf target / FR-render-view-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    FileHeader(usize),
    HunkHeader(usize, usize),
    /// Binary or zero-hunk file: header + this placeholder row (spec §6).
    Placeholder(usize),
    Line(usize, usize, usize),
}

/// The row count contributed by a single file: 1 header, then either the
/// hunk headers + lines, or (binary / zero-hunk) a single placeholder row.
fn file_row_count(file: &DiffFile) -> usize {
    if file.hunks.is_empty() {
        2 // file header + placeholder row (spec §6)
    } else {
        1 + file.hunks.iter().map(|h| 1 + h.lines.len()).sum::<usize>()
    }
}

/// Total flattened row count across every file (FR-render-view-1). Computed
/// once at startup in `run` — not per frame (FR-render-view-5).
fn total_rows(files: &[DiffFile]) -> usize {
    files.iter().map(file_row_count).sum()
}

/// Materializes only the rows inside `range` (typically `visible_range`'s
/// output) by walking the file/hunk/line structure and stopping as soon as
/// the window is filled. Never builds a full per-file row index for the
/// whole diff (FR-render-view-5 / spec §6 "very large diff").
fn rows_in_range(files: &[DiffFile], range: std::ops::Range<usize>) -> Vec<Row> {
    let mut out = Vec::new();
    if range.start >= range.end {
        return out;
    }

    let mut idx = 0usize;
    'files: for (fi, file) in files.iter().enumerate() {
        if idx >= range.end {
            break;
        }
        if idx >= range.start {
            out.push(Row::FileHeader(fi));
        }
        idx += 1;

        if file.hunks.is_empty() {
            if idx >= range.end {
                break;
            }
            if idx >= range.start {
                out.push(Row::Placeholder(fi));
            }
            idx += 1;
            continue;
        }

        for (hi, hunk) in file.hunks.iter().enumerate() {
            if idx >= range.end {
                break 'files;
            }
            if idx >= range.start {
                out.push(Row::HunkHeader(fi, hi));
            }
            idx += 1;

            for li in 0..hunk.lines.len() {
                if idx >= range.end {
                    break 'files;
                }
                if idx >= range.start {
                    out.push(Row::Line(fi, hi, li));
                }
                idx += 1;
            }
        }
    }
    out
}

/// One-letter status marker for a file header row.
fn status_marker(status: crate::diff::ChangeStatus) -> &'static str {
    use crate::diff::ChangeStatus;
    match status {
        ChangeStatus::Modified => "M",
        ChangeStatus::Added => "A",
        ChangeStatus::Deleted => "D",
        ChangeStatus::Renamed { .. } => "R",
    }
}

/// The file header row's text (FR-render-view-1): status marker, path, and
/// (for a rename) the old path it moved from.
fn file_header_text(file: &DiffFile) -> String {
    let marker = status_marker(file.status);
    match &file.old_path {
        Some(old) if old != &file.path => format!("{marker} {old} -> {}", file.path),
        _ => format!("{marker} {}", file.path),
    }
}

/// The hunk header row's text: the `@@ -a,b +c,d @@ section` line
/// (FR-render-view-1).
fn hunk_header_text(hunk: &Hunk) -> String {
    let mut text = format!(
        "@@ -{},{} +{},{} @@",
        hunk.old_start, hunk.old_count, hunk.new_start, hunk.new_count
    );
    if let Some(section) = &hunk.section {
        text.push(' ');
        text.push_str(section);
    }
    text
}

/// Right-aligned old/new line-number field, or blank space of the same
/// width when a side has no line number (e.g. a pure addition's old side).
fn lineno_field(n: Option<u32>, width: usize) -> String {
    match n {
        Some(n) => format!("{n:>width$}"),
        None => " ".repeat(width),
    }
}

/// Builds the styled spans for a line row's content, truncating to
/// `max_width` chars with a trailing marker (spec §9 default: truncate, no
/// wrap) and rendering `changed_spans` regions with `emphasis_style`
/// (FR-render-view-3/4). Indices are char-based, matching `changed_spans`'
/// documented convention.
fn styled_content_spans(
    content: &str,
    changed_spans: &[std::ops::Range<usize>],
    base_style: Style,
    emphasis_style: Style,
    max_width: usize,
) -> Vec<Span<'static>> {
    if max_width == 0 {
        return Vec::new();
    }

    let chars: Vec<char> = content.chars().collect();
    let truncated = chars.len() > max_width;
    let visible_len = if truncated {
        max_width.saturating_sub(1)
    } else {
        chars.len()
    };

    let mut spans = Vec::new();
    let mut idx = 0;
    while idx < visible_len {
        let emphasized = changed_spans.iter().any(|r| r.contains(&idx));
        let start = idx;
        while idx < visible_len && changed_spans.iter().any(|r| r.contains(&idx)) == emphasized {
            idx += 1;
        }
        let text: String = chars[start..idx].iter().collect();
        let style = if emphasized {
            emphasis_style
        } else {
            base_style
        };
        spans.push(Span::styled(text, style));
    }
    if truncated {
        spans.push(Span::styled("…".to_string(), base_style));
    }
    spans
}

/// Renders one flattened `Row` into a styled ratatui line, given the current
/// frame width (for content truncation) and the shared line-number column
/// width (for gutter alignment) — FR-render-view-1/2/3/4.
fn render_row(
    row: &Row,
    files: &[DiffFile],
    lineno_width: usize,
    area_width: u16,
) -> RtLine<'static> {
    match *row {
        Row::FileHeader(fi) => {
            let file = &files[fi];
            RtLine::styled(
                file_header_text(file),
                Style::default().add_modifier(Modifier::BOLD),
            )
        }
        Row::HunkHeader(fi, hi) => {
            let hunk = &files[fi].hunks[hi];
            RtLine::styled(hunk_header_text(hunk), Style::default().fg(Color::Cyan))
        }
        Row::Placeholder(_fi) => {
            // spec §6: binary / zero-hunk files render a placeholder row,
            // not an empty gap.
            RtLine::styled(
                "binary / no textual diff".to_string(),
                Style::default().fg(Color::DarkGray),
            )
        }
        Row::Line(fi, hi, li) => {
            let line = &files[fi].hunks[hi].lines[li];
            // FR-render-view-3: added/removed get distinct plain colors;
            // context is default — no syntax highlighting.
            let base_color = match line.kind {
                LineKind::Added => Color::Green,
                LineKind::Removed => Color::Red,
                LineKind::Context => Color::Reset,
            };
            let base_style = Style::default().fg(base_color);
            // FR-render-view-4: changed_spans regions get distinct emphasis.
            let emphasis_style = base_style.add_modifier(Modifier::REVERSED);

            let gutter = gutter_marker(line.kind).to_string();
            let old_field = lineno_field(line.old_lineno, lineno_width);
            let new_field = lineno_field(line.new_lineno, lineno_width);

            // gutter + ' ' + old + ' ' + new + ' ' before content starts.
            let prefix_width = 1 + 1 + lineno_width + 1 + lineno_width + 1;
            let max_content_width = (area_width as usize).saturating_sub(prefix_width);

            let mut spans = vec![
                Span::styled(gutter, base_style),
                Span::raw(" "),
                Span::styled(old_field, Style::default().fg(Color::DarkGray)),
                Span::raw(" "),
                Span::styled(new_field, Style::default().fg(Color::DarkGray)),
                Span::raw(" "),
            ];
            spans.extend(styled_content_spans(
                &line.content,
                &line.changed_spans,
                base_style,
                emphasis_style,
                max_content_width,
            ));
            RtLine::from(spans)
        }
    }
}

/// Clamp a scroll offset to `[0, total_rows.saturating_sub(viewport)]`.
pub fn clamp_scroll(offset: usize, total_rows: usize, viewport: usize) -> usize {
    let max_scroll = total_rows.saturating_sub(viewport);
    offset.min(max_scroll)
}

/// The clamped `[scroll, scroll + viewport)` window into the flattened rows.
pub fn visible_range(scroll: usize, total_rows: usize, viewport: usize) -> std::ops::Range<usize> {
    let start = clamp_scroll(scroll, total_rows, viewport);
    let end = (start + viewport).min(total_rows);
    start..end
}

/// The gutter marker for a line's kind: `'+'` / `'-'` / `' '`
/// (FR-render-view-2).
pub fn gutter_marker(kind: LineKind) -> char {
    match kind {
        LineKind::Added => '+',
        LineKind::Removed => '-',
        LineKind::Context => ' ',
    }
}

/// The digit width of the line-number gutter column, sized to the largest
/// old/new line number across all files (spec §6 narrow-terminal clamp /
/// FR-render-view-2).
pub fn lineno_col_width(files: &[DiffFile]) -> usize {
    let max_lineno = files
        .iter()
        .flat_map(|f| f.hunks.iter())
        .flat_map(|h| h.lines.iter())
        .flat_map(|line| line.old_lineno.into_iter().chain(line.new_lineno))
        .max()
        .unwrap_or(0);
    digit_width(max_lineno)
}

/// Digit count of `n` in base 10, minimum 1 (so a diff with only `0`-valued
/// or no line numbers still gets a 1-wide column).
fn digit_width(mut n: u32) -> usize {
    if n == 0 {
        return 1;
    }
    let mut width = 0;
    while n > 0 {
        width += 1;
        n /= 10;
    }
    width
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{ChangeStatus, Hunk, Line};

    fn line(kind: LineKind, old: Option<u32>, new: Option<u32>) -> Line {
        Line {
            kind,
            old_lineno: old,
            new_lineno: new,
            content: String::new(),
            no_newline: false,
            changed_spans: Vec::new(),
        }
    }

    fn hunk(lines: Vec<Line>) -> Hunk {
        Hunk {
            old_start: 1,
            old_count: lines.len() as u32,
            new_start: 1,
            new_count: lines.len() as u32,
            section: None,
            lines,
        }
    }

    fn file(hunks: Vec<Hunk>) -> DiffFile {
        DiffFile {
            path: "f.rs".to_string(),
            old_path: None,
            status: ChangeStatus::Modified,
            mode_change: None,
            is_binary: false,
            hunks,
        }
    }

    // --- clamp_scroll (4.1) ------------------------------------------------

    #[test]
    fn clamp_scroll_past_end_clamps_to_max_scroll() {
        assert_eq!(clamp_scroll(100, 50, 10), 40);
    }

    #[test]
    fn clamp_scroll_viewport_larger_than_content_clamps_to_zero() {
        assert_eq!(clamp_scroll(5, 3, 10), 0);
    }

    #[test]
    fn clamp_scroll_exact_fit_clamps_to_zero() {
        assert_eq!(clamp_scroll(5, 10, 10), 0);
    }

    // --- visible_range (4.1) -----------------------------------------------

    #[test]
    fn visible_range_clamps_to_end_window() {
        assert_eq!(visible_range(45, 50, 10), 40..50);
    }

    #[test]
    fn visible_range_viewport_larger_than_content() {
        assert_eq!(visible_range(5, 3, 10), 0..3);
    }

    // --- gutter_marker (4.1) -------------------------------------------------

    #[test]
    fn gutter_marker_covers_all_line_kinds() {
        assert_eq!(gutter_marker(LineKind::Added), '+');
        assert_eq!(gutter_marker(LineKind::Removed), '-');
        assert_eq!(gutter_marker(LineKind::Context), ' ');
    }

    // --- lineno_col_width (4.1/4.2) ------------------------------------------

    #[test]
    fn lineno_col_width_single_digit() {
        let files = vec![file(vec![hunk(vec![line(
            LineKind::Context,
            Some(3),
            Some(3),
        )])])];
        assert_eq!(lineno_col_width(&files), 1);
    }

    #[test]
    fn lineno_col_width_multi_digit() {
        let files = vec![file(vec![hunk(vec![line(
            LineKind::Context,
            Some(123),
            Some(123),
        )])])];
        assert_eq!(lineno_col_width(&files), 3);
    }

    #[test]
    fn lineno_col_width_multi_file_max_wins() {
        let files = vec![
            file(vec![hunk(vec![line(LineKind::Context, Some(9), Some(9))])]),
            file(vec![hunk(vec![line(
                LineKind::Context,
                Some(4567),
                Some(1),
            )])]),
        ];
        assert_eq!(lineno_col_width(&files), 4);
    }

    #[test]
    fn lineno_col_width_empty_files_defaults_to_one() {
        assert_eq!(lineno_col_width(&[]), 1);
    }

    // --- flattened row model (4.2) -------------------------------------------

    #[test]
    fn total_rows_counts_header_hunk_header_and_lines() {
        let files = vec![file(vec![hunk(vec![
            line(LineKind::Context, Some(1), Some(1)),
            line(LineKind::Added, None, Some(2)),
        ])])];
        // 1 file header + 1 hunk header + 2 lines = 4.
        assert_eq!(total_rows(&files), 4);
    }

    #[test]
    fn total_rows_binary_or_zero_hunk_file_is_header_plus_placeholder() {
        let files = vec![file(vec![])];
        assert_eq!(total_rows(&files), 2);
    }

    #[test]
    fn rows_in_range_full_window_matches_total_rows() {
        let files = vec![file(vec![hunk(vec![
            line(LineKind::Context, Some(1), Some(1)),
            line(LineKind::Added, None, Some(2)),
        ])])];
        let total = total_rows(&files);
        let rows = rows_in_range(&files, 0..total);
        assert_eq!(
            rows,
            vec![
                Row::FileHeader(0),
                Row::HunkHeader(0, 0),
                Row::Line(0, 0, 0),
                Row::Line(0, 0, 1),
            ]
        );
    }

    #[test]
    fn rows_in_range_slices_a_window_not_the_whole_buffer() {
        let files = vec![file(vec![hunk(vec![
            line(LineKind::Context, Some(1), Some(1)),
            line(LineKind::Added, None, Some(2)),
            line(LineKind::Removed, Some(3), None),
        ])])];
        // Window covering only the hunk header + first line (rows 1..3).
        let rows = rows_in_range(&files, 1..3);
        assert_eq!(rows, vec![Row::HunkHeader(0, 0), Row::Line(0, 0, 0)]);
    }

    #[test]
    fn rows_in_range_binary_file_yields_header_then_placeholder() {
        let files = vec![file(vec![])];
        let rows = rows_in_range(&files, 0..2);
        assert_eq!(rows, vec![Row::FileHeader(0), Row::Placeholder(0)]);
    }

    // --- apply_action (4.3) ---------------------------------------------------

    #[test]
    fn scroll_down_moves_one_line_and_reports_changed() {
        let mut app = App {
            files: vec![],
            scroll: 0,
            should_quit: false,
        };
        let changed = apply_action(&mut app, Action::ScrollDown, 10, 5);
        assert!(changed);
        assert_eq!(app.scroll, 1);
    }

    #[test]
    fn scroll_down_clamps_at_bottom_and_reports_unchanged() {
        let mut app = App {
            files: vec![],
            scroll: 5,
            should_quit: false,
        };
        let changed = apply_action(&mut app, Action::ScrollDown, 10, 5);
        assert!(!changed);
        assert_eq!(app.scroll, 5);
    }

    #[test]
    fn half_page_down_moves_half_the_viewport_recomputed_per_event() {
        let mut app = App {
            files: vec![],
            scroll: 0,
            should_quit: false,
        };
        apply_action(&mut app, Action::HalfPageDown, 100, 20);
        assert_eq!(app.scroll, 10);
    }

    #[test]
    fn half_page_up_floors_at_one_row_on_a_short_viewport() {
        let mut app = App {
            files: vec![],
            scroll: 5,
            should_quit: false,
        };
        apply_action(&mut app, Action::HalfPageUp, 10, 1);
        assert_eq!(app.scroll, 4);
    }

    #[test]
    fn quit_sets_should_quit_and_reports_changed() {
        let mut app = App {
            files: vec![],
            scroll: 0,
            should_quit: false,
        };
        let changed = apply_action(&mut app, Action::Quit, 10, 5);
        assert!(changed);
        assert!(app.should_quit);
    }

    #[test]
    fn quit_discard_also_sets_should_quit() {
        let mut app = App {
            files: vec![],
            scroll: 0,
            should_quit: false,
        };
        apply_action(&mut app, Action::QuitDiscard, 10, 5);
        assert!(app.should_quit);
    }

    #[test]
    fn every_other_action_is_an_explicit_noop() {
        let mut app = App {
            files: vec![],
            scroll: 3,
            should_quit: false,
        };
        for action in [
            Action::NextHunk,
            Action::PrevHunk,
            Action::NextFile,
            Action::PrevFile,
            Action::Search,
            Action::SearchNext,
            Action::SearchPrev,
            Action::Comment,
            Action::VisualSelect,
            Action::StageToggle,
            Action::ToggleStagePanel,
            Action::GotoDefinition,
            Action::FindReferences,
            Action::Hover,
            Action::AnnotationList,
            Action::Help,
            Action::Noop,
        ] {
            let changed = apply_action(&mut app, action, 10, 5);
            assert!(!changed, "{action:?} must be a no-op this task");
            assert_eq!(app.scroll, 3);
            assert!(!app.should_quit);
        }
    }

    // --- describe_chord (4.4, FR-render-keymap-5) ------------------------------

    #[test]
    fn describe_chord_formats_plain_and_control_keys() {
        assert_eq!(
            describe_chord(KeyChord {
                code: KeyCode::Char('q'),
                modifiers: KeyModifiers::NONE,
            }),
            "q"
        );
        assert_eq!(
            describe_chord(KeyChord {
                code: KeyCode::Char('d'),
                modifiers: KeyModifiers::CONTROL,
            }),
            "Ctrl-d"
        );
    }

    // --- styled_content_spans truncation (4.2, spec §9 default) ---------------

    #[test]
    fn styled_content_spans_truncates_with_trailing_marker() {
        let spans = styled_content_spans("0123456789", &[], Style::default(), Style::default(), 5);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "0123…");
    }

    #[test]
    fn styled_content_spans_fits_without_truncation() {
        let spans = styled_content_spans("hi", &[], Style::default(), Style::default(), 5);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "hi");
    }
}
