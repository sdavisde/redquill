//! The end-review modal ([`super::app::Mode::EndReview`]): a compact
//! overlay, sized to its content rather than stretched — a three-option
//! confirmation reads as a floating dialog, not a panel that should track
//! the terminal size. Lists the three exits — pause / finish / cancel — as
//! a `j`/`k`/arrow-navigable, `Enter`-confirmable selection list, alongside
//! the pre-existing `p`/`f`/`c`/`Esc` mnemonics, which keep dispatching
//! immediately regardless of the highlight.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Padding, Paragraph};

use super::app::{App, Mode};
use super::help::centered;
use super::modal_keys::EndReviewAction;

/// The modal's interior text width (before the block's 1-column
/// left/right padding and its 1-column border on each side) — sized to
/// comfortably fit the widest fixed-width line (the caption), with a
/// little breathing room. The question line (which embeds the branch name)
/// truncates instead of wrapping or growing the modal on a long branch name
/// — `Paragraph` clips a single-line, unwrapped `Line` to its area rather
/// than overflowing it, so no extra truncation code is needed here.
const CONTENT_WIDTH: u16 = 48;
/// Total modal width: content + 2 columns of padding + 2 columns of border.
const MODAL_WIDTH: u16 = CONTENT_WIDTH + 2 + 2;
/// The caption under the three options. Two facts a reviewer at this modal
/// needs and can't read off the option rows: pause and finish both land back
/// on the working tree rather than exiting redquill, and the annotations a
/// finish earmarks — this session's and any restored from an earlier one —
/// print to stdout exactly once, when redquill itself exits. Pause never
/// emits. Kept within [`CONTENT_WIDTH`], which is sized to this line.
const CAPTION: &str = "both return to the working tree; finish emits once";

/// The three exits' display order, short label, and short description —
/// the modal's own tuned prose, distinct from [`super::modal_keys::END_REVIEW_KEYS`]'s
/// longer help-overlay `description` strings. `EndReviewAction`'s
/// `MoveDown`/`MoveUp`/`Confirm` variants never appear here; they're
/// control keys, not exits, so they have no row of their own.
const OPTIONS: [(EndReviewAction, &str, &str); 3] = [
    (EndReviewAction::Pause, "Pause", "keep worktree & state"),
    (EndReviewAction::Finish, "Finish", "remove worktree"),
    (EndReviewAction::Cancel, "Cancel", "keep reviewing"),
];

/// One option row's key label, looked up from the effective table (rather
/// than hardcoded) so a future `[keys.end-review]` remap — see
/// [`super::modal_keys::END_REVIEW_KEYS`]'s doc on that follow-up — would
/// show up here automatically. Falls back to an empty label (never a panic)
/// if the action is somehow missing from the table.
fn key_label_for(app: &App, action: EndReviewAction) -> String {
    app.modal_keys
        .end_review
        .iter()
        .find(|b| b.action == action)
        .map(|b| b.key_label())
        .unwrap_or_default()
}

/// One option's rendered row: the key (padded to `key_width` — the widest
/// of the three, e.g. Cancel's `c / Esc` — so every row's label column
/// lines up regardless of how many keys a row's own label lists) in the
/// accent color and bold, then two spaces, then the short label (padded for
/// alignment) and description.
fn option_row(
    key: &str,
    key_width: usize,
    label: &str,
    desc: &str,
    key_color: Color,
) -> ListItem<'static> {
    Line::from(vec![
        Span::styled(
            format!("{key:<key_width$}"),
            Style::default().fg(key_color).add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("  {label:<8}{desc}")),
    ])
    .into()
}

/// The modal's total height: borders (2) plus its content rows — question
/// (1), a blank separator (1), the three options (`OPTIONS.len()`), another
/// blank separator (1), and the caption (1) — plus, when a status message
/// is showing (a failed finish, surfaced in place rather than closing the
/// modal), one more blank separator and message row.
fn modal_height(app: &App) -> u16 {
    let content_rows = 1 + 1 + OPTIONS.len() as u16 + 1 + 1;
    let status_rows = if app.status_message.is_some() { 2 } else { 0 };
    content_rows + status_rows + 2
}

/// Renders the end-review modal, centered over `area`. A no-op outside
/// [`Mode::EndReview`] (the caller should only invoke this there).
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    if !matches!(app.mode, Mode::EndReview { .. }) {
        return;
    }
    let branch = app.review_branch().unwrap_or("this branch");

    let width = MODAL_WIDTH.min(area.width.saturating_sub(2));
    let height = modal_height(app).min(area.height.saturating_sub(2));
    let popup = centered(area, width, height);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .padding(Padding::horizontal(1))
        .title(" End review ");
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let mut row_constraints = vec![
        Constraint::Length(1),                    // question
        Constraint::Length(1),                    // blank
        Constraint::Length(OPTIONS.len() as u16), // options list
        Constraint::Length(1),                    // blank
        Constraint::Length(1),                    // caption
    ];
    if app.status_message.is_some() {
        row_constraints.push(Constraint::Length(1)); // blank
        row_constraints.push(Constraint::Length(1)); // status message
    }
    let rows = Layout::vertical(row_constraints).split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("End review of {branch}?"),
            Style::default().add_modifier(Modifier::BOLD),
        ))),
        rows[0],
    );

    render_options(frame, rows[2], app);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            CAPTION,
            Style::default()
                .fg(app.theme.footer_text)
                .add_modifier(Modifier::DIM),
        ))),
        rows[4],
    );

    if let Some(message) = &app.status_message {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                message.clone(),
                Style::default().fg(app.theme.status_message),
            ))),
            rows[6],
        );
    }
}

/// Renders the three-option selection list, highlighting whichever option
/// [`App::end_review_cursor`] currently points at — the same
/// reverse-highlight convention [`super::switcher_modal`]'s branch/worktree
/// list uses.
fn render_options(frame: &mut Frame, area: Rect, app: &App) {
    let keys: Vec<String> = OPTIONS
        .iter()
        .map(|(action, _, _)| key_label_for(app, *action))
        .collect();
    let key_width = keys.iter().map(|k| k.chars().count()).max().unwrap_or(0);
    let items: Vec<ListItem> = OPTIONS
        .iter()
        .zip(&keys)
        .map(|((_, label, desc), key)| option_row(key, key_width, label, desc, app.theme.help_key))
        .collect();
    let list = List::new(items).highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    let mut state = ListState::default();
    state.select(Some(app.end_review_cursor().unwrap_or(0)));
    frame.render_stateful_widget(list, area, &mut state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::FileDiff;
    use crate::git::{DiffTarget, RawFilePatch};
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

    fn render_modal(app: &App) -> String {
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

    #[test]
    fn renders_the_branch_name_and_all_three_exits() {
        let mut app = App::new(vec![sample_file()]);
        app.target = DiffTarget::Review {
            base: "main".to_string(),
            branch: "feature/thing".to_string(),
        };
        app.open_end_review_modal();
        let content = render_modal(&app);
        assert!(content.contains("feature/thing"));
        assert!(content.contains("Pause"));
        assert!(content.contains("Finish"));
        assert!(content.contains("Cancel"));
        assert!(content.contains("keep worktree"));
        assert!(content.contains("remove worktree"));
    }

    #[test]
    fn highlighted_option_starts_on_pause_and_follows_the_cursor() {
        let mut app = App::new(vec![sample_file()]);
        app.target = DiffTarget::Review {
            base: "main".to_string(),
            branch: "feature/thing".to_string(),
        };
        app.open_end_review_modal();
        assert_eq!(app.end_review_cursor(), Some(0));
        app.end_review_move_down();
        assert_eq!(app.end_review_cursor(), Some(1));
    }
}
