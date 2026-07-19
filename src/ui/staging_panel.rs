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

use super::app::App;
use super::keymap::{Action, Keymap, Scope};
use super::stage_ops::StagedFile;
use super::theme::Theme;

fn item_line(entry: &StagedFile, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{} ", entry.letter),
            Style::default()
                .fg(theme.letter_color(entry.letter))
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(entry.path.clone()),
    ])
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

    let items: Vec<ListItem> = match app.staging_filter.as_ref() {
        Some(filter) => filter
            .indices()
            .iter()
            .filter_map(|&i| app.staged.get(i))
            .map(|e| ListItem::new(item_line(e, &app.theme)))
            .collect(),
        None => app
            .staged
            .iter()
            .map(|e| ListItem::new(item_line(e, &app.theme)))
            .collect(),
    };
    let list = List::new(items).highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = ListState::default();
    state.select(Some(app.staging_cursor));
    frame.render_stateful_widget(list, list_area, &mut state);
}
