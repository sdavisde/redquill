//! The end-review modal ([`super::app::Mode::EndReview`], spec 08 Unit 2): a
//! centered overlay listing the three exits — pause / finish / cancel —
//! rendered from the app's effective end-review table
//! (`app.modal_keys.end_review`, the default [`super::modal_keys::END_REVIEW_KEYS`])
//! so the modal's own text can never drift from the table `q`'s handler
//! dispatches through (see [`super::modes::handle_end_review_key`]). Modeled
//! on [`super::switcher_modal`] (centered, `Clear`-ed, bordered block).

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use super::app::{App, Mode};

/// Centers a `width_pct`% x `height_pct`% rect inside `area`, matching
/// [`super::switcher_modal`]'s helper of the same shape.
fn centered(area: Rect, width_pct: u16, height_pct: u16) -> Rect {
    let [area] = Layout::horizontal([Constraint::Percentage(width_pct)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::vertical([Constraint::Percentage(height_pct)])
        .flex(Flex::Center)
        .areas(area);
    area
}

/// Renders the end-review modal, centered over `area`. A no-op outside
/// [`Mode::EndReview`] (the caller should only invoke this there).
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    if !matches!(app.mode, Mode::EndReview { .. }) {
        return;
    }
    let branch = app.review_branch().unwrap_or("this branch");

    let popup = centered(area, 60, 40);
    frame.render_widget(Clear, popup);

    let mut lines = vec![
        Line::from(Span::styled(
            format!("End review of {branch}?"),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::raw(""),
    ];
    for binding in &app.modal_keys.end_review {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:<9}", binding.key_label()),
                Style::default()
                    .fg(app.theme.help_key)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(binding.description),
        ]));
    }
    if let Some(message) = &app.status_message {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            message.clone(),
            Style::default().fg(app.theme.status_message),
        )));
    }

    let block = Block::default().borders(Borders::ALL).title(" End review ");
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        popup,
    );
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
    fn renders_nothing_outside_end_review_mode() {
        let app = App::new(vec![sample_file()]);
        assert!(render_modal(&app).trim().is_empty());
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
}
