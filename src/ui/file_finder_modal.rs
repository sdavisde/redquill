//! The fuzzy file finder overlay ([`super::app::Mode::Finder`], spec 06 Unit
//! 1): a centered modal in the style of [`super::switcher_modal`] — input
//! line on top, ranked result list below, matched characters emphasized.

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState};

use super::app::App;

/// Centers a `width_pct`% x `height_pct`% rect inside `area` — the same
/// two-axis `Flex::Center` dance [`super::switcher_modal`] uses.
fn centered(area: Rect, width_pct: u16, height_pct: u16) -> Rect {
    let [area] = Layout::horizontal([Constraint::Percentage(width_pct)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::vertical([Constraint::Percentage(height_pct)])
        .flex(Flex::Center)
        .areas(area);
    area
}

/// Builds one result row's spans: the path with every char index in
/// `positions` emphasized via `theme.search_match_fg` (blue) plus bold — the
/// same match-emphasis treatment [`super::project_search_view`]'s results
/// list uses (spec 06 round-1 UX fix: the matched substring's *text* itself
/// carries the emphasis, not just a background tint), so "this substring
/// matched your query" reads consistently, and with high contrast, across
/// the app. Split out from [`match_row`] so it's directly unit-testable
/// without constructing a `ListItem`.
fn match_spans(path: &str, positions: &[u32], theme: &super::theme::Theme) -> Vec<Span<'static>> {
    let matched_style = Style::default()
        .fg(theme.search_match_fg)
        .add_modifier(Modifier::BOLD);
    let plain_style = Style::default();
    let mut spans = Vec::with_capacity(path.chars().count());
    for (i, ch) in path.chars().enumerate() {
        let style = if positions.contains(&(i as u32)) {
            matched_style
        } else {
            plain_style
        };
        spans.push(Span::styled(ch.to_string(), style));
    }
    spans
}

/// Builds one result row as a [`ListItem`] from [`match_spans`].
fn match_row(path: &str, positions: &[u32], theme: &super::theme::Theme) -> ListItem<'static> {
    ListItem::new(Line::from(match_spans(path, positions, theme)))
}

/// Renders the fuzzy file finder modal, centered over `area`. A no-op if
/// `app.finder` is `None` (the caller should only invoke this in
/// [`super::app::Mode::Finder`]).
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(finder) = &app.finder else {
        return;
    };
    let popup = centered(area, 70, 60);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" find file ")
        .title_bottom(Line::from(" Enter open  Up/Down move  Esc close "));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let [input_area, list_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);

    let input_text = format!("> {}", finder.query);
    frame.render_widget(
        Line::from(Span::styled(
            input_text,
            Style::default().fg(app.theme.search_prompt),
        )),
        input_area,
    );

    let loading = finder.candidates.is_empty() && finder.query.is_empty();
    let items: Vec<ListItem> = if loading {
        vec![ListItem::new(Line::from(Span::styled(
            "  loading files\u{2026}",
            Style::default().fg(app.theme.footer_text),
        )))]
    } else if finder.matches.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "  no matches",
            Style::default().fg(app.theme.footer_text),
        )))]
    } else {
        finder
            .matches
            .iter()
            .filter_map(|m| finder.candidates.get(m.index).map(|c| (c, &m.positions)))
            .map(|(candidate, positions)| match_row(&candidate.path, positions, &app.theme))
            .collect()
    };

    let list = List::new(items).highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    let mut list_state = ListState::default();
    if !finder.matches.is_empty() {
        list_state.select(Some(finder.cursor));
    }
    frame.render_stateful_widget(list, list_area, &mut list_state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::FileDiff;
    use crate::git::RawFilePatch;
    use crate::search::FileCandidate;
    use crate::ui::app::Mode;
    use crate::ui::file_finder::FinderState;
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

    fn render_finder(app: &App) -> String {
        let backend = TestBackend::new(60, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 60, 24);
        terminal.draw(|frame| render(frame, area, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    fn finder_state() -> FinderState {
        FinderState {
            query: String::new(),
            candidates: Vec::new(),
            matches: Vec::new(),
            cursor: 0,
            return_mode: Mode::Normal,
        }
    }

    #[test]
    fn renders_nothing_when_finder_is_none() {
        let app = App::new(vec![sample_file()]);
        let content = render_finder(&app);
        assert!(content.trim().is_empty());
    }

    #[test]
    fn shows_loading_placeholder_before_candidates_arrive() {
        let mut app = App::new(vec![sample_file()]);
        app.finder = Some(finder_state());
        let content = render_finder(&app);
        assert!(content.contains("loading"));
    }

    #[test]
    fn shows_query_and_matched_paths() {
        let mut app = App::new(vec![sample_file()]);
        let mut state = finder_state();
        state.query = "main".to_string();
        state.candidates = vec![FileCandidate {
            path: "src/main.rs".to_string(),
        }];
        state.matches = crate::search::rank(&state.candidates, &state.query);
        app.finder = Some(state);
        let content = render_finder(&app);
        assert!(content.contains("main"));
        assert!(content.contains("src/main.rs"));
    }

    #[test]
    fn shows_no_matches_placeholder_for_a_non_matching_query() {
        let mut app = App::new(vec![sample_file()]);
        let mut state = finder_state();
        state.query = "zzz".to_string();
        state.candidates = vec![FileCandidate {
            path: "src/main.rs".to_string(),
        }];
        app.finder = Some(state);
        let content = render_finder(&app);
        assert!(content.contains("no matches"));
    }

    #[test]
    fn matched_chars_get_blue_bold_foreground_not_a_background_tint() {
        let theme = crate::ui::theme::Theme::default();
        let spans = match_spans("main.rs", &[0, 1, 2, 3], &theme);
        for (i, span) in spans.iter().enumerate() {
            if i < 4 {
                assert_eq!(span.style.fg, Some(theme.search_match_fg));
                assert!(span.style.add_modifier.contains(Modifier::BOLD));
                assert_eq!(
                    span.style.bg, None,
                    "match emphasis must ride the foreground, not a background tint"
                );
            } else {
                assert_eq!(span.style.fg, None, "unmatched chars must stay unstyled");
                assert_eq!(span.style.bg, None);
            }
        }
    }
}
