//! The staging panel: every file with staged changes, one line each
//! (status letter + path), with the focused row highlighted. Toggled with
//! `s`; deliberately styled like the annotation list panel so the two feel
//! like siblings.
//!
//! During a review session (spec 08 Unit 5) this same widget renders the
//! **accepted-files panel** instead: `App::staged` is fed from
//! `review_states` rather than `git status` (see `App::refresh_accepted_list`
//! in `super::review_ops`), so the row content here needs no session
//! branching at all — only the title and the empty-state hint text/key
//! differ, since "staged"/"nothing staged yet" would be untruthful during a
//! review (its `git status` is always clean).

use ratatui::Frame;
use ratatui::layout::Rect;
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
/// accepted-files panel (spec 08 Unit 5; see the module doc). An empty list
/// renders a hint line instead of an empty list; the hint's key is resolved
/// from `keymap` (diff scope, [`Action::ToggleStage`]/[`Action::ToggleAccept`])
/// rather than hardcoded, so a `[keys.diff]` remap can't leave this text
/// naming a stale key (spec 07 Unit 4, task 4.6) — an unbound action falls
/// back to generic wording rather than showing no key at all.
pub fn render(frame: &mut Frame, area: Rect, app: &App, keymap: &Keymap) {
    let review = app.in_review_session();
    let title = if review { "accepted" } else { "staged" };
    let block = Block::default().borders(Borders::ALL).title(title);

    if app.staged.is_empty() {
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

    let items: Vec<ListItem> = app
        .staged
        .iter()
        .map(|e| ListItem::new(item_line(e, &app.theme)))
        .collect();
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = ListState::default();
    state.select(Some(app.staging_cursor));
    frame.render_stateful_widget(list, area, &mut state);
}
