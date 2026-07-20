//! The finished-review cleanup confirm modal
//! ([`super::app::Mode::CleanupReviews`]): a centered, bordered overlay that
//! enumerates every finished review about to be deleted — PR number/title,
//! worktree path, and an explicit unpublished-work warning when nonzero — with
//! a confirm/cancel hint line below. Nothing is deleted until the reviewer
//! confirms; the modal is the safety boundary, so it names exactly what a
//! confirm removes. Reads [`App::cleanup_reviews`]; renders nothing outside
//! [`Mode::CleanupReviews`].

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Padding, Paragraph};

use crate::review::FinishedReview;

use super::app::{App, Mode};
use super::help::centered;
use super::modal_keys::CleanupReviewsAction;
use super::theme::Theme;

/// One finished-review entry's rows: a primary `#N title` line, a secondary
/// worktree-path line, and — only when nonzero — a warning line naming the
/// unpublished annotation/reply count that a delete would discard.
fn entry_items(entry: &FinishedReview, theme: &Theme) -> Vec<ListItem<'static>> {
    let title = if entry.title.is_empty() {
        format!("#{}", entry.number)
    } else {
        format!("#{} {}", entry.number, entry.title)
    };
    let mut lines = vec![
        Line::from(Span::styled(
            title,
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!("  {}", entry.worktree_path.display()),
            Style::default().fg(theme.footer_text),
        )),
    ];
    if entry.unpublished_count > 0 {
        lines.push(Line::from(Span::styled(
            format!(
                "  \u{26a0} {} unpublished comment(s)/reply(ies) will be discarded",
                entry.unpublished_count
            ),
            Style::default().fg(theme.status_message),
        )));
    }
    vec![ListItem::new(lines)]
}

/// The confirm/cancel hint line, keys read from the effective table so a remap
/// shows up here with no extra wiring.
fn hint_line(app: &App) -> Line<'static> {
    let key = |action: CleanupReviewsAction| {
        app.modal_keys
            .cleanup_reviews
            .iter()
            .find(|b| b.action == action)
            .map(|b| b.key_label())
            .unwrap_or_default()
    };
    Line::from(vec![
        Span::styled(
            key(CleanupReviewsAction::Confirm),
            Style::default()
                .fg(app.theme.help_key)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" delete   "),
        Span::styled(
            key(CleanupReviewsAction::Cancel),
            Style::default()
                .fg(app.theme.help_key)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" cancel"),
    ])
}

/// Renders the cleanup confirm modal, centered over `area`. A no-op outside
/// [`Mode::CleanupReviews`].
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    if !matches!(app.mode, Mode::CleanupReviews { .. }) {
        return;
    }

    let width = 72u16.min(area.width.saturating_sub(2));
    // Each entry is 2 lines (title + worktree), 3 when it carries an
    // unpublished-work warning; plus borders (2), a blank, and the hint line.
    let entry_lines: u16 = app
        .cleanup_reviews
        .iter()
        .map(|e| if e.unpublished_count > 0 { 3 } else { 2 })
        .sum();
    let height = (entry_lines + 4).min(area.height.saturating_sub(2));
    let popup = centered(area, width, height);
    frame.render_widget(Clear, popup);

    let count = app.cleanup_reviews.len();
    let block = Block::default()
        .borders(Borders::ALL)
        .padding(Padding::horizontal(1))
        .title(format!(" Clean up {count} finished review(s) "))
        .title_bottom(Line::from(
            " delete removes worktree, branch, and saved state ",
        ));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let rows = Layout::vertical([
        Constraint::Min(0),    // the enumerated entries
        Constraint::Length(1), // blank
        Constraint::Length(1), // confirm/cancel hint
    ])
    .split(inner);

    let items: Vec<ListItem<'static>> = app
        .cleanup_reviews
        .iter()
        .flat_map(|entry| entry_items(entry, &app.theme))
        .collect();
    frame.render_widget(List::new(items), rows[0]);
    frame.render_widget(Paragraph::new(hint_line(app)), rows[2]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::FileDiff;
    use crate::git::RawFilePatch;
    use crate::review::store::ForgeProviderKind;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::path::PathBuf;

    use super::super::app::ModeOrigin;

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

    fn finished(number: u64, title: &str, unpublished: usize) -> FinishedReview {
        FinishedReview {
            branch: format!("redquill/pr/{number}"),
            number,
            title: title.to_string(),
            provider: ForgeProviderKind::GitHub,
            host: "github.com".to_string(),
            worktree_path: PathBuf::from(format!("/tmp/redquill/worktrees/pr-{number}")),
            unpublished_count: unpublished,
        }
    }

    fn cleanup_app(entries: Vec<FinishedReview>) -> App {
        let mut app = App::new(vec![sample_file()]);
        app.cleanup_reviews = entries;
        app.mode = Mode::CleanupReviews {
            origin: ModeOrigin::Normal,
        };
        app
    }

    fn render_modal(app: &App) -> String {
        let backend = TestBackend::new(90, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 90, 24);
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
    fn lists_each_entry_number_title_and_worktree_path() {
        let app = cleanup_app(vec![
            finished(1, "old feature", 0),
            finished(2, "stale fix", 0),
        ]);
        let content = render_modal(&app);
        assert!(content.contains("#1 old feature"));
        assert!(content.contains("#2 stale fix"));
        assert!(content.contains("pr-1"));
        assert!(content.contains("pr-2"));
        assert!(content.contains("delete"));
        assert!(content.contains("cancel"));
    }

    #[test]
    fn warns_about_unpublished_work_only_when_nonzero() {
        let app = cleanup_app(vec![finished(3, "has drafts", 2)]);
        let content = render_modal(&app);
        assert!(
            content.contains("2 unpublished"),
            "the unpublished-work warning must name the count: {content}"
        );
    }

    #[test]
    fn no_unpublished_warning_when_all_published() {
        let app = cleanup_app(vec![finished(4, "clean", 0)]);
        let content = render_modal(&app);
        assert!(!content.contains("unpublished"));
    }

    #[test]
    fn renders_nothing_outside_cleanup_mode() {
        let app = App::new(vec![sample_file()]);
        let content = render_modal(&app);
        assert!(content.trim().is_empty());
    }
}
