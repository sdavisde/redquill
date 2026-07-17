//! The pull/push confirm modal ([`super::app::Mode::ConfirmRemoteOp`], spec
//! 08 Unit 5): a compact, content-sized overlay naming the branch under
//! review and the pending op, with a plain confirm/cancel hint line below —
//! modeled on [`super::end_review_modal`]'s sizing (`centered` rather than a
//! percentage-of-screen split; a confirmation reads as a floating dialog,
//! not a panel that should track the terminal size) but simpler, since this
//! is a binary gate rather than a three-option selection.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Padding, Paragraph};

use super::app::{App, Mode};
use super::help::centered;

/// The modal's interior text width — sized to comfortably fit the widest
/// fixed hint line, with a little breathing room. The question line (which
/// embeds the branch name) truncates instead of wrapping or growing the
/// modal on a long branch name, same as [`super::end_review_modal`]'s
/// question line.
const CONTENT_WIDTH: u16 = 48;
/// Total modal width: content + 2 columns of padding + 2 columns of border.
const MODAL_WIDTH: u16 = CONTENT_WIDTH + 2 + 2;
/// Total modal height: question (1) + blank (1) + hint (1), plus borders (2).
const MODAL_HEIGHT: u16 = 5;

/// `RemoteOp::label()` (`"pull"`/`"push"`/`"publish"`) with its first letter
/// capitalized, for the question line's leading verb (`"Push feature — the
/// branch under review?"`, matching the spec's example verbatim). ASCII-only
/// input (the label is always one of the four fixed English words), so a
/// byte-level capitalize is safe here — this is not a general-purpose
/// title-case helper.
fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Renders the pull/push confirm modal, centered over `area`. A no-op
/// outside [`Mode::ConfirmRemoteOp`] (the caller should only invoke this
/// there).
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Mode::ConfirmRemoteOp { op, .. } = app.mode else {
        return;
    };
    let branch = app.review_branch().unwrap_or("this branch");

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
        Constraint::Length(1), // blank
        Constraint::Length(1), // hint
    ])
    .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(
                "{} {branch} \u{2014} the branch under review?",
                capitalize(op.label())
            ),
            Style::default().add_modifier(Modifier::BOLD),
        ))),
        rows[0],
    );

    let confirm_key = app
        .modal_keys
        .confirm_remote_op
        .iter()
        .find(|b| b.action == super::modal_keys::ConfirmRemoteOpAction::Confirm)
        .map(|b| b.key_label())
        .unwrap_or_default();
    let cancel_key = app
        .modal_keys
        .confirm_remote_op
        .iter()
        .find(|b| b.action == super::modal_keys::ConfirmRemoteOpAction::Cancel)
        .map(|b| b.key_label())
        .unwrap_or_default();
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                confirm_key,
                Style::default()
                    .fg(app.theme.help_key)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(" {} confirm   ", op.label())),
            Span::styled(
                cancel_key,
                Style::default()
                    .fg(app.theme.help_key)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" cancel"),
        ])),
        rows[2],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::FileDiff;
    use crate::git::{DiffTarget, RawFilePatch, RemoteOp};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::super::app::PanelTab;

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
        let buffer = terminal.backend().buffer().clone();
        if std::env::var_os("REDQUILL_PROOF_DUMP").is_some() {
            let w = buffer.area.width as usize;
            let symbols: Vec<&str> = buffer.content().iter().map(|c| c.symbol()).collect();
            for row in symbols.chunks(w) {
                eprintln!("{}", row.concat());
            }
        }
        buffer.content().iter().map(|c| c.symbol()).collect()
    }

    #[test]
    fn question_names_the_op_and_the_reviewed_branch() {
        let mut app = App::new(vec![sample_file()]);
        app.target = DiffTarget::Review {
            base: "main".to_string(),
            branch: "feature/thing".to_string(),
        };
        app.mode = Mode::ConfirmRemoteOp {
            op: RemoteOp::Push,
            cursor: 0,
            tab: PanelTab::Changes,
        };
        let content = render_modal(&app);
        assert!(content.contains("Push feature/thing"));
        assert!(content.contains("the branch under review?"));
        assert!(content.contains("confirm"));
        assert!(content.contains("cancel"));
    }

    #[test]
    fn pull_question_says_pull() {
        let mut app = App::new(vec![sample_file()]);
        app.target = DiffTarget::Review {
            base: "main".to_string(),
            branch: "feature".to_string(),
        };
        app.mode = Mode::ConfirmRemoteOp {
            op: RemoteOp::Pull,
            cursor: 0,
            tab: PanelTab::Changes,
        };
        let content = render_modal(&app);
        assert!(content.contains("Pull feature"));
    }

    #[test]
    fn render_is_a_no_op_outside_the_modal() {
        let app = App::new(vec![sample_file()]);
        assert_eq!(app.mode, Mode::Normal);
        let content = render_modal(&app);
        assert!(!content.contains("the branch under review?"));
    }
}
