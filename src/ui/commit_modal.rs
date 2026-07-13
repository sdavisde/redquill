//! The commit-message modal (spec 04): a centered overlay for typing the
//! message `git commit -m` will receive. Renders the multi-line text buffer
//! (first line = subject, `Ctrl-j` adds body lines), places the terminal
//! cursor at the buffer's edit position, and shows the staged-file count in
//! the title with key hints in the footer. Modeled on
//! [`super::compose_modal`] (a centered, `Clear`-ed, bordered block).

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Position, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::app::App;

/// Centers a rect `width_pct`% wide and `height` rows tall inside `area`.
fn centered(area: Rect, width_pct: u16, height: u16) -> Rect {
    let [area] = Layout::horizontal([Constraint::Percentage(width_pct)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    area
}

/// The modal title, e.g. `Commit 2 staged files` (`1 staged file` in the
/// singular).
fn title(staged_count: usize) -> String {
    let files = if staged_count == 1 { "file" } else { "files" };
    format!("Commit {staged_count} staged {files}")
}

/// Renders the commit-message modal, centered over `area`. A no-op if
/// `app.commit_message` is `None` (the caller should only invoke this in
/// [`super::app::Mode::CommitMessage`]).
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(state) = &app.commit_message else {
        return;
    };

    let footer = " Enter commit  Ctrl-j newline  Esc cancel ";

    let content_height = state.buffer.lines.len() as u16;
    let height = (content_height + 2)
        .max(4)
        .min(area.height.saturating_sub(2));
    let popup = centered(area, 60, height);

    frame.render_widget(Clear, popup);

    let lines: Vec<Line> = state
        .buffer
        .lines
        .iter()
        .map(|l| Line::from(l.clone()))
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title(app.staged.len()))
        .title_bottom(Line::from(footer));
    let inner = block.inner(popup);
    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, popup);

    // Place the terminal cursor at the buffer's edit position, clamped
    // inside the inner content area so a very long line doesn't push it
    // off-screen.
    let cursor_x = inner.x + (state.buffer.cursor_col as u16).min(inner.width.saturating_sub(1));
    let cursor_y = inner.y + (state.buffer.cursor_row as u16).min(inner.height.saturating_sub(1));
    frame.set_cursor_position(Position::new(cursor_x, cursor_y));
}

#[cfg(test)]
mod tests {
    use super::super::commit_message::CommitMessageState;
    use super::super::compose::TextBuffer;
    use super::super::stage_ops::StagedFile;
    use super::*;
    use crate::diff::FileDiff;
    use crate::git::RawFilePatch;
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

    #[test]
    fn renders_nothing_when_state_is_none() {
        let app = App::new(vec![sample_file()]);
        let content = render_modal(&app);
        assert!(content.trim().is_empty());
    }

    #[test]
    fn renders_title_message_and_key_hints() {
        let mut app = App::new(vec![sample_file()]);
        app.staged = vec![StagedFile {
            path: "src/main.rs".to_string(),
            letter: 'M',
        }];
        let mut state = CommitMessageState::new(0);
        state.buffer = TextBuffer::from_str("fix: parser\n\nbody line");
        app.commit_message = Some(state);
        let content = render_modal(&app);
        assert!(content.contains("Commit 1 staged file"));
        assert!(content.contains("fix: parser"));
        assert!(content.contains("body line"));
        assert!(content.contains("Enter commit"));
        assert!(content.contains("Ctrl-j newline"));
        assert!(content.contains("Esc cancel"));
    }

    #[test]
    fn title_pluralizes_staged_files() {
        assert_eq!(title(1), "Commit 1 staged file");
        assert_eq!(title(2), "Commit 2 staged files");
    }
}
