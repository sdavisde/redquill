//! The commit-message modal (spec 04): a centered overlay for typing the
//! message `git commit -m` will receive. Renders the multi-line text buffer
//! (first line = subject, `Ctrl-j` adds body lines), places the terminal
//! cursor at the buffer's edit position, and shows the staged-file count in
//! the title with key hints in the footer. Modeled on
//! [`super::compose_modal`] (a centered, `Clear`-ed, bordered block).

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::app::App;
use super::textwrap;

/// The horizontal slice a 60%-wide modal occupies within `area` (full height,
/// centered). Its width feeds the wrap layout, and its `x`/`width` are shared
/// by the final popup — the vertical centering only sets `y`/`height`.
fn horizontal_slice(area: Rect) -> Rect {
    let [slice] = Layout::horizontal([Constraint::Percentage(60)])
        .flex(Flex::Center)
        .areas(area);
    slice
}

/// Centers a `height`-tall popup vertically within the (already
/// horizontally-centered) `slice`.
fn centered_in(slice: Rect, height: u16) -> Rect {
    let [popup] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(slice);
    popup
}

/// The modal title, e.g. `Commit 2 staged files` (`1 staged file` in the
/// singular).
fn title(staged_count: usize) -> String {
    let files = if staged_count == 1 { "file" } else { "files" };
    format!("Commit {staged_count} staged {files}")
}

/// The review-session warning line (spec 08 Unit 5): a prominent, banner-
/// colored reminder naming the branch under review, shown above the message
/// buffer whenever the commit modal opens during a review session — the
/// nothing-staged gate ([`App::open_commit_message`]) is otherwise
/// unchanged, so this is purely an added confirm-first *reminder*, never a
/// block. `None` outside a review session, so the modal's layout and height
/// are byte-for-byte unchanged there.
fn review_warning(app: &App) -> Option<String> {
    app.in_review_session().then(|| {
        format!(
            "\u{26a0} committing on {} \u{2014} the branch under review",
            app.review_branch().unwrap_or("this branch")
        )
    })
}

/// Renders the commit-message modal, centered over `area`. A no-op if
/// `app.commit_message` is `None` (the caller should only invoke this in
/// [`super::app::Mode::CommitMessage`]).
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(state) = &app.commit_message else {
        return;
    };

    let footer = " Enter commit  Shift-Enter/Ctrl-j newline  Esc cancel ";
    let warning = review_warning(app);

    // Soft-wrap against the modal's inner width (60% slice minus the two
    // border columns); the wrapped-row count sets the height and the cursor
    // math below shares the exact same layout.
    let slice = horizontal_slice(area);
    let wrap_width = (slice.width.saturating_sub(2)).max(1) as usize;
    let wrapped = textwrap::layout(&state.buffer.lines, wrap_width);

    let warning_rows: u16 = u16::from(warning.is_some());
    let content_height = wrapped.rows.len() as u16;
    let height = (content_height + warning_rows + 2)
        .max(4)
        .min(area.height.saturating_sub(2));
    let popup = centered_in(slice, height);

    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title(app.staged.len()))
        .title_bottom(Line::from(footer));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    // The warning (when present) claims the inner area's first row, banner-
    // colored so it visually ties back to the review-session banner at the
    // top of the screen; the message buffer renders in whatever remains —
    // rendered separately from `block` above (rather than via `.block()`)
    // precisely so this split is possible.
    let text_area = if let Some(warning) = &warning {
        let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(inner);
        let mut line = Line::from(Span::raw(warning.clone()));
        let pad = (rows[0].width as usize).saturating_sub(line.width());
        if pad > 0 {
            line.spans.push(Span::raw(" ".repeat(pad)));
        }
        line.style = Style::default()
            .bg(app.theme.review_banner_bg)
            .fg(app.theme.review_banner_fg)
            .add_modifier(Modifier::BOLD);
        frame.render_widget(Paragraph::new(line), rows[0]);
        rows[1]
    } else {
        inner
    };

    // Scroll offset derived (not stored) from the cursor's visual row so the
    // cursor is always on screen: keep it on the last visible row once the
    // content outgrows the viewport, otherwise no scroll.
    let (cursor_vrow, cursor_vcol) =
        wrapped.cursor_position(state.buffer.cursor_row, state.buffer.cursor_col);
    let visible_rows = text_area.height as usize;
    let scroll = cursor_vrow.saturating_sub(visible_rows.saturating_sub(1));

    let lines: Vec<Line> = wrapped
        .rows
        .iter()
        .map(|r| Line::from(textwrap::row_str(&state.buffer.lines[r.logical_line], r)))
        .collect();
    let paragraph = Paragraph::new(lines).scroll((scroll as u16, 0));
    frame.render_widget(paragraph, text_area);

    // Place the terminal cursor at its true wrapped position minus the scroll
    // offset. The only clamp is the terminal reality that column == width has
    // no cell (right border) — not the old edge-clamp that let long lines lie.
    let cursor_x = text_area.x + (cursor_vcol as u16).min(text_area.width.saturating_sub(1));
    let cursor_y = text_area.y + (cursor_vrow.saturating_sub(scroll)) as u16;
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
        // 100 wide so the 60%-width modal's footer (which now names the
        // Shift-Enter/Ctrl-j newline keys) renders without truncation.
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 100, 24);
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
        assert!(content.contains("Shift-Enter/Ctrl-j newline"));
        assert!(content.contains("Esc cancel"));
    }

    #[test]
    fn title_pluralizes_staged_files() {
        assert_eq!(title(1), "Commit 1 staged file");
        assert_eq!(title(2), "Commit 2 staged files");
    }

    // -- Review-session warning line (spec 08 Unit 5) ------------------------

    #[test]
    fn review_session_shows_a_warning_naming_the_reviewed_branch() {
        let mut app = App::new(vec![sample_file()]);
        app.target = crate::git::DiffTarget::Review {
            base: "main".to_string(),
            branch: "feature/thing".to_string(),
        };
        app.staged = vec![StagedFile {
            path: "src/main.rs".to_string(),
            letter: 'M',
        }];
        let mut state = CommitMessageState::new(0);
        state.buffer = TextBuffer::from_str("fix: parser");
        app.commit_message = Some(state);
        let content = render_modal(&app);
        assert!(content.contains("committing on feature/thing"));
        assert!(content.contains("the branch under review"));
        // The nothing-staged gate and message content are still there —
        // the warning is additive, not a replacement.
        assert!(content.contains("Commit 1 staged file"));
        assert!(content.contains("fix: parser"));
    }

    /// Outside a review session, no warning line appears — the modal's
    /// content is byte-for-byte the pre-existing rendering (a regression pin
    /// against the review-warning addition).
    #[test]
    fn no_review_warning_outside_a_review_session() {
        let mut app = App::new(vec![sample_file()]);
        assert_eq!(app.target, crate::git::DiffTarget::WorkingTree);
        app.staged = vec![StagedFile {
            path: "src/main.rs".to_string(),
            letter: 'M',
        }];
        let mut state = CommitMessageState::new(0);
        state.buffer = TextBuffer::from_str("fix: parser");
        app.commit_message = Some(state);
        let content = render_modal(&app);
        assert!(!content.contains("the branch under review"));
        assert!(!content.contains('\u{26a0}'));
    }
}
