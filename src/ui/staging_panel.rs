//! The staging panel: every file with staged changes, one line each (status
//! letter + path), with the focused row highlighted. Toggled with `s`;
//! deliberately styled like the annotation list panel so the two feel like
//! siblings. During a review session this same widget renders the
//! accepted-files panel instead: `App::staged` is fed from `review_states`
//! rather than `git status` (see `App::refresh_accepted_list`), so only the
//! title and empty-state hint text/key differ. Supports the shared `/`
//! fuzzy filter (spec 12 FR-7..FR-9), same chrome as the annotation list
//! panel (`list_panel.rs`).

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

use crate::diff::StatDisplay;

use super::app::App;
use super::elide::elide_left;
use super::keymap::{Action, Keymap, Scope};
use super::stage_ops::StagedFile;
use super::stat_display::stat_display_spans;
use super::theme::Theme;

/// One list row: status letter, path, then the `+A -R` counts right-aligned
/// to `width` (the list's inner width in cells). The counts are kept whole and
/// the path gives instead, elided from the left
/// ([`super::elide::elide_left`]) — same arrangement as the diff pane's file
/// headers and the git panel's file rows, since the panel is narrow and full
/// paths overrun it routinely.
fn item_line(entry: &StagedFile, stats: StatDisplay, theme: &Theme, width: usize) -> Line<'static> {
    let letter = Span::styled(
        format!("{} ", entry.letter),
        Style::default()
            .fg(theme.letter_color(entry.letter))
            .add_modifier(Modifier::BOLD),
    );
    let stat_spans = stat_display_spans(stats, theme);
    let stat_w = stat_spans.as_ref().map_or(0, |(_, w)| *w);
    let prefix_w = letter.width();
    // One cell of gap so the path never abuts the counts.
    let path = elide_left(&entry.path, width.saturating_sub(prefix_w + stat_w + 1));
    let pad = width
        .saturating_sub(prefix_w + path.chars().count() + stat_w)
        .max(1);
    let mut spans = vec![letter, Span::raw(path), Span::raw(" ".repeat(pad))];
    if let Some((stat_spans, _)) = stat_spans {
        spans.extend(stat_spans);
    }
    Line::from(spans)
}

/// Renders the staging panel into `area` — or, during a review session, the
/// accepted-files panel (see the module doc). An empty list (and no active
/// filter) renders a hint line instead; the hint's key is resolved from
/// `keymap` (diff scope, [`Action::ToggleStage`]/[`Action::ToggleAccept`])
/// rather than hardcoded, so a `[keys.diff]` remap can't leave this text
/// naming a stale key.
///
/// A `/` filter (spec 12 FR-7..FR-9) adds a one-row chrome line above the
/// list showing the live/locked query, narrows the rendered rows to the
/// filtered view, and shows a "no matches" hint in place of a blank list.
pub fn render(frame: &mut Frame, area: Rect, app: &App, keymap: &Keymap) {
    let review = app.in_review_session();
    let title = if review { "accepted" } else { "staged" };
    let block = Block::default().borders(Borders::ALL).title(title);

    if app.staged.is_empty() && app.staging_filter.is_none() {
        let text = if review {
            match keymap.label_for(Scope::Diff, Action::ToggleAccept) {
                Some(key) => format!("no files accepted yet — press {key} on a file to accept it"),
                None => "no files accepted yet".to_string(),
            }
        } else {
            match keymap.label_for(Scope::Diff, Action::ToggleStage) {
                Some(key) => format!("nothing staged yet — press {key} on a hunk to stage it"),
                None => "nothing staged yet".to_string(),
            }
        };
        let hint = Paragraph::new(text).block(block);
        frame.render_widget(hint, area);
        return;
    }

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let (chrome_area, list_area) = match app.staging_filter.as_ref() {
        Some(_) => {
            let [chrome, list] =
                Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
            (Some(chrome), list)
        }
        None => (None, inner),
    };

    if let (Some(chrome_area), Some(filter)) = (chrome_area, app.staging_filter.as_ref()) {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                super::list_filter::chrome_text(filter),
                Style::default().fg(app.theme.search_prompt),
            ))),
            chrome_area,
        );
    }

    if let Some(filter) = app.staging_filter.as_ref().filter(|f| f.is_empty()) {
        let hint = Paragraph::new(super::list_filter::empty_hint(filter));
        frame.render_widget(hint, list_area);
        return;
    }

    let stats_for = |e: &StagedFile| {
        app.stats
            .get(&e.path)
            .copied()
            .unwrap_or(StatDisplay::Omitted)
    };
    let width = list_area.width as usize;
    let items: Vec<ListItem> = match app.staging_filter.as_ref() {
        Some(filter) => filter
            .indices()
            .iter()
            .filter_map(|&i| app.staged.get(i))
            .map(|e| ListItem::new(item_line(e, stats_for(e), &app.theme, width)))
            .collect(),
        None => app
            .staged
            .iter()
            .map(|e| ListItem::new(item_line(e, stats_for(e), &app.theme, width)))
            .collect(),
    };
    let list = List::new(items).highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = ListState::default();
    state.select(Some(app.staging_cursor));
    frame.render_stateful_widget(list, list_area, &mut state);
}
