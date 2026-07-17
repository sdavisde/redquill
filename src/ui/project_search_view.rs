//! Full-screen render for [`super::app::Mode::ProjectSearch`]: a
//! query/toggle-indicator input line, a summary/error line, and a
//! scrollable results list grouped by file with the matched line text
//! (match span emphasized) — replacing the diff pane's content for the
//! frame (see `super::mod`'s `draw`), the way the History tab replaces the
//! sidebar's content rather than overlaying a modal on top of it.
//!
//! **Focus model** (see [`super::project_search::SearchFocus`]): the input
//! line's prompt color and
//! text cursor, and the results list's selection style, both track
//! `state.focus` so which half is "listening" is visually unambiguous —
//! [`input_prompt_style`]/[`should_show_input_cursor`]/[`selection_style`]
//! are the three pure decision points, each independently unit-tested below
//! without needing a terminal.

use std::ops::Range;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState};

use super::app::App;
use super::project_search::{MIN_QUERY_LEN, ProjectSearchState, SearchFocus};
use super::theme::Theme;

/// The toggle-indicator string shown at the end of the input line, e.g.
/// `[regex] [case:smart] [word:off]` — always all three, so every toggle's
/// current state is visible regardless of whether it's "on".
fn toggle_indicators(state: &ProjectSearchState) -> String {
    use crate::search::CaseMode;
    let kind = if state.literal { "literal" } else { "regex" };
    let case = match state.case {
        CaseMode::Smart => "smart",
        CaseMode::Sensitive => "sensitive",
        CaseMode::Insensitive => "insensitive",
    };
    let word = if state.whole_word { "word" } else { "any" };
    format!("[{kind}] [case:{case}] [{word}]")
}

/// The `/query` prompt's foreground: [`Theme::search_prompt`] (bright) while
/// [`SearchFocus::Input`] has focus, matching the blinking text cursor
/// alongside it; the same dim [`Theme::footer_text`] the toggle indicators
/// already use while [`SearchFocus::Results`] has focus, so the input line
/// visibly recedes the moment it stops receiving keystrokes.
fn input_prompt_style(focus: SearchFocus, theme: &Theme) -> Style {
    match focus {
        SearchFocus::Input => Style::default().fg(theme.search_prompt),
        SearchFocus::Results => Style::default().fg(theme.footer_text),
    }
}

/// Whether the input line should carry the blinking terminal text cursor —
/// only while [`SearchFocus::Input`] has focus, so it never sits blinking
/// over an input the user has explicitly stepped away from into the results
/// list.
fn should_show_input_cursor(focus: SearchFocus) -> bool {
    matches!(focus, SearchFocus::Input)
}

/// Builds the input line's content: `/query` prompt (styled per
/// [`input_prompt_style`]) plus the toggle indicators. Split out from
/// [`render_input_line`] so the prompt's focus-dependent style is directly
/// unit-testable without a terminal.
fn input_line(state: &ProjectSearchState, theme: &Theme) -> Line<'static> {
    let prompt = format!("/{}", state.query);
    Line::from(vec![
        Span::styled(prompt, input_prompt_style(state.focus, theme)),
        Span::raw("  "),
        Span::styled(
            toggle_indicators(state),
            Style::default().fg(theme.footer_text),
        ),
    ])
}

/// Renders the input line: `/query` prompt plus the toggle indicators.
fn render_input_line(frame: &mut Frame, area: Rect, state: &ProjectSearchState, theme: &Theme) {
    frame.render_widget(input_line(state, theme), area);
}

/// Renders the summary/error line beneath the input: the regex-compile error
/// (if the current query didn't compile — never wipes `groups`, see
/// `super::project_search`'s module doc), else the latest scan summary
/// ("N matches in M files", plus capped/skipped indicators), else a
/// "scanning…"/"type at least N characters" placeholder.
fn render_summary_line(frame: &mut Frame, area: Rect, state: &ProjectSearchState, theme: &Theme) {
    let (text, is_error) = if let Some(err) = &state.error {
        (format!("invalid pattern: {err}"), true)
    } else if let Some(summary) = &state.summary {
        let mut parts = vec![format!(
            "{} matches in {} files",
            summary.total_hits, summary.files_matched
        )];
        if summary.capped {
            parts.push("capped — refine your query".to_string());
        }
        if summary.binary_skipped > 0 || summary.oversized_skipped > 0 || summary.errored > 0 {
            parts.push(format!(
                "skipped {} binary, {} oversized, {} unreadable",
                summary.binary_skipped, summary.oversized_skipped, summary.errored
            ));
        }
        (parts.join("  ·  "), false)
    } else if state.scan.is_some() {
        ("scanning…".to_string(), false)
    } else if state.query.chars().count() < MIN_QUERY_LEN {
        (
            format!("type at least {MIN_QUERY_LEN} characters to search"),
            false,
        )
    } else {
        (String::new(), false)
    };
    let style = if is_error {
        Style::default().fg(theme.status_message)
    } else {
        Style::default().fg(theme.footer_text)
    };
    frame.render_widget(Line::from(Span::styled(text, style)), area);
}

/// The byte ranges within `text` (already stripped of its line terminator)
/// that fall in `match_spans`, split into styled runs — matched runs get
/// `theme.search_match_fg` (blue) plus bold, everything else plain. The
/// matched substring's *text* carries the emphasis rather than a background
/// tint — see [`Theme::search_match_fg`]'s doc for why this
/// is scoped to Project Search and the fuzzy finder rather than the in-diff
/// `/` search, which keeps its own `search_match_bg` treatment. Operates on
/// `char_indices` so byte offsets always land on a char boundary, never
/// panicking on multi-byte UTF-8 content.
fn highlighted_line_spans(
    text: &str,
    match_spans: &[Range<usize>],
    theme: &Theme,
) -> Vec<Span<'static>> {
    let matched_style = Style::default()
        .fg(theme.search_match_fg)
        .add_modifier(Modifier::BOLD);
    let plain_style = Style::default();

    let mut spans = Vec::new();
    let mut current = String::new();
    let mut current_matched = false;
    let mut started = false;
    for (byte_idx, ch) in text.char_indices() {
        let matched = match_spans.iter().any(|r| r.contains(&byte_idx));
        if started && matched != current_matched {
            let style = if current_matched {
                matched_style
            } else {
                plain_style
            };
            spans.push(Span::styled(std::mem::take(&mut current), style));
        }
        current.push(ch);
        current_matched = matched;
        started = true;
    }
    if !current.is_empty() {
        let style = if current_matched {
            matched_style
        } else {
            plain_style
        };
        spans.push(Span::styled(current, style));
    }
    spans
}

/// The results list's selection highlight, by focus: a full `REVERSED`
/// block while [`SearchFocus::Results`] has focus, matching every other
/// list surface's selection convention, versus a plain `UNDERLINED` marker
/// while [`SearchFocus::Input`] has focus — still shows where `Enter` would
/// jump, without reading as "drive me with j/k" when keystrokes are
/// actually going to the query. A matched span's own blue+bold
/// [`Theme::search_match_fg`] survives either style.
fn selection_style(focus: SearchFocus) -> Style {
    match focus {
        SearchFocus::Input => Style::default().add_modifier(Modifier::UNDERLINED),
        SearchFocus::Results => Style::default().add_modifier(Modifier::REVERSED),
    }
}

/// Renders the grouped results list: one bold file-heading row per group,
/// followed by each hit's `path:line` + matched line text (match span
/// emphasized via [`highlighted_line_spans`]). The selection highlight
/// (`cursor`, a flat index across groups) lands on the corresponding hit row,
/// never a heading — headings aren't selectable, and its style tracks
/// `state.focus` (see [`selection_style`]). Shows a placeholder line instead
/// of the list when there's nothing to show yet (below the minimum query
/// length, still scanning with no hits so far, or no matches at all).
fn render_results(frame: &mut Frame, area: Rect, state: &ProjectSearchState, theme: &Theme) {
    if state.groups.is_empty() {
        let placeholder = if state.query.chars().count() < MIN_QUERY_LEN || state.error.is_some() {
            None
        } else if state.scan.is_some() {
            Some("searching…")
        } else {
            Some("no matches")
        };
        if let Some(placeholder) = placeholder {
            frame.render_widget(
                Line::from(Span::styled(
                    format!("  {placeholder}"),
                    Style::default().fg(theme.footer_text),
                )),
                area,
            );
        }
        return;
    }

    let mut items: Vec<ListItem> = Vec::new();
    let mut selected_row: Option<usize> = None;
    let mut hit_index = 0usize;
    for group in &state.groups {
        items.push(ListItem::new(Line::from(Span::styled(
            group.path.clone(),
            Style::default()
                .bg(theme.file_header_bg)
                .add_modifier(Modifier::BOLD),
        ))));
        for hit in &group.hits {
            if hit_index == state.cursor {
                selected_row = Some(items.len());
            }
            let mut spans = vec![Span::styled(
                format!("  {}:{} ", hit.path, hit.line_number),
                Style::default().fg(theme.footer_text),
            )];
            spans.extend(highlighted_line_spans(
                &hit.line_text,
                &hit.match_spans,
                theme,
            ));
            items.push(ListItem::new(Line::from(spans)));
            hit_index += 1;
        }
    }

    let list = List::new(items).highlight_style(selection_style(state.focus));
    let mut list_state = ListState::default();
    list_state.select(selected_row);
    frame.render_stateful_widget(list, area, &mut list_state);
}

/// Renders the full-screen Project Search view over `area`. A no-op if
/// `app.project_search` is `None` (the caller should only invoke this in
/// [`super::app::Mode::ProjectSearch`]).
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(state) = &app.project_search else {
        return;
    };
    let [input_area, summary_area, results_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(area);

    render_input_line(frame, input_area, state, &app.theme);
    render_summary_line(frame, summary_area, state, &app.theme);
    render_results(frame, results_area, state, &app.theme);

    if should_show_input_cursor(state.focus) {
        // Mirrors `Mode::Search`'s own footer-input cursor math in
        // `super::mod`'s `draw` — `/` prompt plus the query's char count,
        // clamped inside the input line so a long query never walks the
        // cursor off the right edge.
        let prompt_len = 1 + state.query.chars().count() as u16;
        let cursor_x = input_area
            .x
            .saturating_add(prompt_len)
            .min(input_area.x + input_area.width.saturating_sub(1));
        frame.set_cursor_position(Position::new(cursor_x, input_area.y));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::FileDiff;
    use crate::git::RawFilePatch;
    use crate::search::{CaseMode, ScanSummary, SearchHit};
    use crate::ui::app::Mode;
    use crate::ui::project_search::ResultGroup;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn sample_file() -> FileDiff {
        let raw = "\
diff --git a/src/main.rs b/src/main.rs
index 111..222 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,2 +1,2 @@
 fn main() {
-    old();
+    new();
";
        FileDiff::from_patch(&RawFilePatch {
            path: "src/main.rs".to_string(),
            old_path: None,
            raw: raw.to_string(),
            is_binary: false,
        })
        .unwrap()
    }

    fn render_view(app: &App) -> String {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 80, 24);
        terminal.draw(|frame| render(frame, area, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    fn hit(path: &str, line: u64, text: &str, spans: Vec<Range<usize>>) -> SearchHit {
        SearchHit {
            path: path.to_string(),
            line_number: line,
            line_text: text.to_string(),
            match_spans: spans,
            generation: 0,
        }
    }

    #[test]
    fn renders_nothing_when_project_search_is_none() {
        let app = App::new(vec![sample_file()]);
        let content = render_view(&app);
        assert!(content.trim().is_empty());
    }

    #[test]
    fn shows_the_query_and_all_three_toggle_indicators() {
        let mut app = App::new(vec![sample_file()]);
        let mut state = ProjectSearchState::new(Mode::Normal);
        state.query = "needle".to_string();
        state.whole_word = true;
        state.literal = true;
        state.case = CaseMode::Sensitive;
        app.project_search = Some(state);

        let content = render_view(&app);
        assert!(content.contains("/needle"));
        assert!(content.contains("[literal]"));
        assert!(content.contains("[case:sensitive]"));
        assert!(content.contains("[word]"));
    }

    #[test]
    fn shows_type_more_characters_placeholder_below_min_length() {
        let mut app = App::new(vec![sample_file()]);
        let mut state = ProjectSearchState::new(Mode::Normal);
        state.query = "n".to_string();
        app.project_search = Some(state);

        let content = render_view(&app);
        assert!(content.contains("type at least"));
    }

    #[test]
    fn shows_grouped_results_with_path_and_line() {
        let mut app = App::new(vec![sample_file()]);
        let mut state = ProjectSearchState::new(Mode::Normal);
        state.query = "needle".to_string();
        state.groups = vec![ResultGroup {
            path: "src/lib.rs".to_string(),
            #[allow(clippy::single_range_in_vec_init)]
            hits: vec![hit("src/lib.rs", 42, "let needle = 1;", vec![4..10])],
        }];
        state.summary = Some(ScanSummary {
            generation: 0,
            files_scanned: 5,
            files_matched: 1,
            total_hits: 1,
            binary_skipped: 0,
            oversized_skipped: 0,
            errored: 0,
            capped: false,
            aborted: false,
        });
        app.project_search = Some(state);

        let content = render_view(&app);
        assert!(content.contains("src/lib.rs"));
        assert!(content.contains("src/lib.rs:42"));
        assert!(content.contains("let needle = 1;"));
        assert!(content.contains("1 matches in 1 files"));
    }

    #[test]
    fn shows_capped_indicator_when_summary_is_capped() {
        let mut app = App::new(vec![sample_file()]);
        let mut state = ProjectSearchState::new(Mode::Normal);
        state.query = "needle".to_string();
        state.summary = Some(ScanSummary {
            generation: 0,
            files_scanned: 1,
            files_matched: 1,
            total_hits: 10_000,
            binary_skipped: 0,
            oversized_skipped: 0,
            errored: 0,
            capped: true,
            aborted: false,
        });
        app.project_search = Some(state);

        let content = render_view(&app);
        assert!(content.contains("capped"));
    }

    #[test]
    fn shows_error_line_without_wiping_prior_results() {
        let mut app = App::new(vec![sample_file()]);
        let mut state = ProjectSearchState::new(Mode::Normal);
        state.query = "(unclosed".to_string();
        state.error = Some("invalid search pattern: some detail".to_string());
        state.groups = vec![ResultGroup {
            path: "src/lib.rs".to_string(),
            hits: vec![hit("src/lib.rs", 1, "prior good hit", vec![])],
        }];
        app.project_search = Some(state);

        let content = render_view(&app);
        assert!(content.contains("invalid pattern"));
        assert!(
            content.contains("prior good hit"),
            "prior results must still render alongside the error"
        );
    }

    #[test]
    fn shows_no_matches_placeholder_when_scan_found_nothing() {
        let mut app = App::new(vec![sample_file()]);
        let mut state = ProjectSearchState::new(Mode::Normal);
        state.query = "zzz".to_string();
        state.summary = Some(ScanSummary::default());
        app.project_search = Some(state);

        let content = render_view(&app);
        assert!(content.contains("no matches"));
    }

    // -- Match highlight styling --------------------------------------------

    #[test]
    fn matched_run_gets_blue_bold_foreground_not_a_background_tint() {
        let theme = Theme::default();
        #[allow(clippy::single_range_in_vec_init)]
        let spans = highlighted_line_spans("let needle = 1;", &[4..10], &theme);
        let matched = spans
            .iter()
            .find(|s| s.content.as_ref() == "needle")
            .expect("the matched run must be its own span");
        assert_eq!(matched.style.fg, Some(theme.search_match_fg));
        assert!(matched.style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(
            matched.style.bg, None,
            "match emphasis must ride the foreground, not a background tint"
        );
    }

    #[test]
    fn plain_runs_around_a_match_stay_unstyled() {
        let theme = Theme::default();
        #[allow(clippy::single_range_in_vec_init)]
        let spans = highlighted_line_spans("let needle = 1;", &[4..10], &theme);
        let plain = spans
            .iter()
            .find(|s| s.content.as_ref() == "let ")
            .expect("a plain run must precede the match");
        assert_eq!(plain.style.fg, None);
        assert_eq!(plain.style.bg, None);
    }

    // -- Focus model -----------------------------------------------------

    #[test]
    fn input_prompt_style_is_bright_in_input_focus_and_dim_in_results_focus() {
        let theme = Theme::default();
        assert_eq!(
            input_prompt_style(SearchFocus::Input, &theme).fg,
            Some(theme.search_prompt)
        );
        assert_eq!(
            input_prompt_style(SearchFocus::Results, &theme).fg,
            Some(theme.footer_text),
            "the input line must visibly recede once it stops receiving keystrokes"
        );
    }

    #[test]
    fn input_cursor_shows_only_in_input_focus() {
        assert!(should_show_input_cursor(SearchFocus::Input));
        assert!(!should_show_input_cursor(SearchFocus::Results));
    }

    #[test]
    fn selection_style_is_reversed_in_results_focus_and_underlined_in_input_focus() {
        assert!(
            selection_style(SearchFocus::Results)
                .add_modifier
                .contains(Modifier::REVERSED)
        );
        assert!(
            !selection_style(SearchFocus::Input)
                .add_modifier
                .contains(Modifier::REVERSED)
        );
        assert!(
            selection_style(SearchFocus::Input)
                .add_modifier
                .contains(Modifier::UNDERLINED),
            "Input focus still marks the selected row, just not as strongly as Results focus"
        );
    }

    #[test]
    fn render_sets_the_terminal_cursor_only_while_input_focused() {
        let mut app = App::new(vec![sample_file()]);
        let mut state = ProjectSearchState::new(Mode::Normal);
        state.query = "needle".to_string();
        state.focus = SearchFocus::Input;
        app.project_search = Some(state);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 80, 24);
        terminal.draw(|frame| render(frame, area, &app)).unwrap();
        assert!(
            terminal.backend().cursor_visible(),
            "Input focus must show the terminal cursor"
        );

        // Ratatui hides the cursor on any frame whose draw closure doesn't
        // call `Frame::set_cursor_position` (see ratatui-core's
        // `draw_hides_cursor_when_frame_cursor_is_not_set`), so a Results-
        // focused frame — which `render` deliberately skips the call on —
        // leaves it hidden.
        app.project_search.as_mut().unwrap().focus = SearchFocus::Results;
        terminal.draw(|frame| render(frame, area, &app)).unwrap();
        assert!(
            !terminal.backend().cursor_visible(),
            "Results focus must not show the input's text cursor"
        );
    }
}
