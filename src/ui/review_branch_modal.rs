//! The review-branch modal ([`super::app::Mode::ReviewBranch`]): a centered
//! overlay listing local branches (the currently checked-out one already
//! excluded by [`super::review_branch::App::open_review_branch_modal`]),
//! styled like [`super::switcher_modal`]'s Branches tab. A failed
//! `worktree_add`/reroot surfaces as a message line inside the modal rather
//! than closing it (mirrors [`super::end_review_modal`]'s status-message
//! row) — the modal stays open so the reviewer can see the failure and
//! retry or pick a different branch.

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};

use super::app::App;

/// Centers a `width_pct`% x `height_pct`% rect inside `area` — the same
/// percentage-of-screen sizing [`super::switcher_modal`]'s `centered` uses
/// (distinct from [`super::help::centered`]'s fixed-cell sizing, imported
/// here only for its two-axis `Flex::Center` shape).
fn centered_pct(area: Rect, width_pct: u16, height_pct: u16) -> Rect {
    let [area] = Layout::horizontal([Constraint::Percentage(width_pct)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::vertical([Constraint::Percentage(height_pct)])
        .flex(Flex::Center)
        .areas(area);
    area
}

/// The branch rows: plain names, `no other local branches` if the list is
/// empty (every branch is checked out already, or the repo has only one).
fn branch_rows(app: &App) -> Vec<ListItem<'static>> {
    let Some(state) = &app.review_branch_modal else {
        return Vec::new();
    };
    if state.branches.is_empty() {
        return vec![ListItem::new(Line::from(Span::styled(
            "  no other local branches",
            Style::default().fg(app.theme.footer_text),
        )))];
    }
    state
        .branches
        .iter()
        .map(|b| ListItem::new(Line::from(Span::raw(format!("  {}", b.name)))))
        .collect()
}

/// Renders the review-branch modal, centered over `area`. A no-op if
/// `app.review_branch_modal` is `None` (the caller should only invoke this
/// in [`super::app::Mode::ReviewBranch`]).
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(state) = &app.review_branch_modal else {
        return;
    };
    let popup = centered_pct(area, 80, 60);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Review branch ")
        .title_bottom(Line::from(" Enter review  j/k move  Esc close "));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let rows = if app.status_message.is_some() {
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(inner)
    } else {
        Layout::vertical([Constraint::Min(0)]).split(inner)
    };

    let items = branch_rows(app);
    let empty = state.branches.is_empty();
    let list = List::new(items).highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    let mut list_state = ListState::default();
    if !empty {
        list_state.select(Some(state.cursor));
    }
    frame.render_stateful_widget(list, rows[0], &mut list_state);

    if let Some(message) = &app.status_message {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                message.clone(),
                Style::default().fg(app.theme.status_message),
            ))),
            rows[1],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::FileDiff;
    use crate::git::{LocalBranch, RawFilePatch};
    use crate::ui::app::Mode;
    use crate::ui::review_branch::ReviewBranchState;
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

    fn branch(name: &str) -> LocalBranch {
        LocalBranch {
            name: name.to_string(),
            is_current: false,
            worktree: None,
        }
    }

    #[test]
    fn renders_nothing_when_modal_is_none() {
        let app = App::new(vec![sample_file()]);
        assert!(render_modal(&app).trim().is_empty());
    }

    #[test]
    fn renders_branch_list_and_title() {
        let mut app = App::new(vec![sample_file()]);
        app.mode = Mode::ReviewBranch;
        app.review_branch_modal = Some(ReviewBranchState::new(
            vec![branch("feature-a"), branch("feature-b")],
            0,
        ));
        let content = render_modal(&app);
        assert!(content.contains("Review branch"));
        assert!(content.contains("feature-a"));
        assert!(content.contains("feature-b"));
    }

    #[test]
    fn empty_branch_list_shows_empty_state() {
        let mut app = App::new(vec![sample_file()]);
        app.mode = Mode::ReviewBranch;
        app.review_branch_modal = Some(ReviewBranchState::new(Vec::new(), 0));
        let content = render_modal(&app);
        assert!(content.contains("no other local branches"));
    }

    #[test]
    fn a_status_message_renders_inside_the_modal() {
        let mut app = App::new(vec![sample_file()]);
        app.mode = Mode::ReviewBranch;
        app.review_branch_modal = Some(ReviewBranchState::new(vec![branch("feature-a")], 0));
        app.set_status_message("review failed: fatal: branch not found");
        let content = render_modal(&app);
        assert!(content.contains("review failed"));
        assert!(
            content.contains("feature-a"),
            "the branch list must still render alongside the message"
        );
    }
}
