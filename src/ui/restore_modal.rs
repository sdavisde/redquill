//! The restore confirm modal ([`super::app::Mode::ConfirmRestore`]): a
//! compact overlay naming the one file about to lose its uncommitted changes,
//! with a plain confirm/cancel hint line below.
//!
//! Modeled on [`super::confirm_remote_op_modal`]'s binary-gate shape, with one
//! addition: a second line spelling out what is lost, in the deleted-file
//! color. Every other confirm in this app guards something recoverable — this
//! one doesn't, and the wording says so rather than leaving the reviewer to
//! infer it from a verb.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Padding, Paragraph};

use super::app::{App, Mode};
use super::help::centered;
use super::modal_keys::RestoreAction;
use crate::git::RestoreScope;

/// The modal's interior text width.
const CONTENT_WIDTH: u16 = 52;
/// Total modal width: content + 2 columns of padding + 2 columns of border.
const MODAL_WIDTH: u16 = CONTENT_WIDTH + 2 + 2;
/// Total modal height: question (1) + consequence (1) + blank (1) + hint (1),
/// plus borders (2).
const MODAL_HEIGHT: u16 = 6;

/// Shortens `path` to at most `max` columns by dropping leading components,
/// marked with a leading `…`.
///
/// Truncating from the *left* rather than the right is the point: the tail of
/// a path (the file name) is what identifies the file being destroyed, so it
/// is the part that must survive. Falls back to a plain tail slice when even
/// one component doesn't fit. Counts `char`s, matching the width math
/// elsewhere in the UI.
fn elide_path_left(path: &str, max: usize) -> String {
    if path.chars().count() <= max {
        return path.to_string();
    }
    if max == 0 {
        return String::new();
    }
    // Room for the leading marker.
    let budget = max - 1;
    // Prefer cutting at a component boundary so the result reads as a path.
    if let Some(cut) = path.char_indices().find_map(|(i, _)| {
        let rest = &path[i..];
        (rest.chars().count() <= budget && rest.starts_with('/')).then_some(i)
    }) {
        return format!("\u{2026}{}", &path[cut..]);
    }
    let tail: String = path
        .chars()
        .skip(path.chars().count().saturating_sub(budget))
        .collect();
    format!("\u{2026}{tail}")
}

/// The key label bound to `action`, or an empty string if the table has no
/// row for it.
fn key_for(app: &App, action: RestoreAction) -> String {
    app.modal_keys
        .restore
        .iter()
        .find(|b| b.action == action)
        .map(|b| b.key_label())
        .unwrap_or_default()
}

/// Renders the restore confirm modal, centered over `area`. A no-op outside
/// [`Mode::ConfirmRestore`], and a no-op when no request is pending (the two
/// always move together — see [`super::restore`]).
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    if !matches!(app.mode, Mode::ConfirmRestore { .. }) {
        return;
    }
    let Some(request) = app.restore_request.as_ref() else {
        return;
    };

    let width = MODAL_WIDTH.min(area.width.saturating_sub(2));
    let height = MODAL_HEIGHT.min(area.height.saturating_sub(2));
    let popup = centered(area, width, height);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .padding(Padding::horizontal(1))
        .title(" Confirm ");
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let rows = Layout::vertical([
        Constraint::Length(1), // question
        Constraint::Length(1), // consequence
        Constraint::Length(1), // blank
        Constraint::Length(1), // hint
    ])
    .split(inner);

    // The question's fixed text is the verb plus a trailing "?"; the path
    // gets whatever's left.
    let index_only = request.scope == RestoreScope::IndexOnly;
    let verb = if request.untracked {
        "Delete"
    } else if index_only {
        "Unstage all changes to"
    } else {
        "Restore"
    };
    let path_budget = (inner.width as usize).saturating_sub(verb.chars().count() + 2);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("{verb} {}?", elide_path_left(&request.path, path_budget)),
            Style::default().add_modifier(Modifier::BOLD),
        ))),
        rows[0],
    );

    // Each case states what actually happens. A rename says so explicitly:
    // the file comes back under a *different* name than the header shows,
    // which is the one outcome a reviewer would not otherwise predict.
    let consequence = match (&request.old_path, request.untracked, index_only) {
        (_, true, _) => "Untracked \u{2014} the file is removed, not restored.".to_string(),
        (Some(old), _, _) => {
            let prefix = "Renamed from ";
            let suffix = " \u{2014} the rename is undone too.";
            let budget = (inner.width as usize)
                .saturating_sub(prefix.chars().count() + suffix.chars().count());
            format!("{prefix}{}{suffix}", elide_path_left(old, budget))
        }
        (None, _, true) => "The index only. Working-tree changes are kept.".to_string(),
        (None, _, false) => "Discards all changes, staged and unstaged. No undo.".to_string(),
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            consequence,
            Style::default().fg(app.theme.kind_deleted),
        ))),
        rows[1],
    );

    let key_style = Style::default()
        .fg(app.theme.help_key)
        .add_modifier(Modifier::BOLD);
    let confirm_label = if request.untracked {
        " delete   "
    } else if index_only {
        " unstage   "
    } else {
        " restore   "
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(key_for(app, RestoreAction::Confirm), key_style),
            Span::raw(confirm_label),
            Span::styled(key_for(app, RestoreAction::Cancel), key_style),
            Span::raw(" cancel"),
        ])),
        rows[3],
    );
}

#[cfg(test)]
#[path = "restore_modal_tests.rs"]
mod tests;
