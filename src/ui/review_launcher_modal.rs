//! The Review launcher modal ([`super::app::Mode::ReviewLauncher`]): a
//! centered overlay with two tabs — Branches (default) and Commits — styled
//! like [`super::switcher_modal`] (centered `Clear`-ed bordered block, tab
//! headers as the block title, active tab emphasized). The list area is a
//! placeholder until the Branches/Commits tabs are wired up to real data in
//! follow-up work; the footer hint line is built from the *effective*
//! `REVIEW_LAUNCHER_KEYS` table (`app.modal_keys.review_launcher`) rather
//! than a hardcoded string, so a `[keys.review-launcher]` remap shows up here
//! with no extra wiring.

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem};

use super::app::App;
use super::modal_keys::{LauncherAction, ModalBinding};
use super::review_launcher::LauncherTab;
use super::theme::Theme;

/// Centers a `width_pct`% x `height_pct`% rect inside `area` — the same
/// two-axis `Flex::Center` sizing [`super::switcher_modal::centered`] uses.
fn centered(area: Rect, width_pct: u16, height_pct: u16) -> Rect {
    let [area] = Layout::horizontal([Constraint::Percentage(width_pct)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::vertical([Constraint::Percentage(height_pct)])
        .flex(Flex::Center)
        .areas(area);
    area
}

/// The ` Branches │ Commits ` tab bar, active tab emphasized, rendered as the
/// modal's block title — mirrors [`super::switcher_modal::tab_bar`] exactly.
fn tab_bar(active: LauncherTab, theme: &Theme) -> Line<'static> {
    let active_style = Style::default()
        .fg(theme.help_key)
        .add_modifier(Modifier::BOLD);
    let inactive_style = Style::default().fg(theme.footer_text);
    let (branches_style, commits_style) = match active {
        LauncherTab::Branches => (active_style, inactive_style),
        LauncherTab::Commits => (inactive_style, active_style),
    };
    Line::from(vec![
        Span::raw(" "),
        Span::styled("Branches", branches_style),
        Span::styled(" \u{2502} ", Style::default().fg(theme.footer_text)),
        Span::styled("Commits", commits_style),
        Span::raw(" "),
    ])
}

/// What `Enter` does on `tab` — shown as its own line so the two tabs'
/// differing weight (a lightweight read-only peek vs. starting a full
/// worktree session) is unambiguous before the user presses it.
fn enter_outcome_hint(tab: LauncherTab) -> &'static str {
    match tab {
        LauncherTab::Branches => "Enter: start branch review",
        LauncherTab::Commits => "Enter: review commit (read-only)",
    }
}

/// The active tab's placeholder row. Neither tab's real list is wired up yet
/// (Branches/Commits data lands in follow-up work), so this names that
/// honestly rather than showing a misleadingly-empty list.
fn placeholder_rows(tab: LauncherTab, theme: &Theme) -> Vec<ListItem<'static>> {
    let text = match tab {
        LauncherTab::Branches => "  branch list — coming soon",
        LauncherTab::Commits => "  commit list — coming soon",
    };
    vec![ListItem::new(Line::from(Span::styled(
        text,
        Style::default().fg(theme.footer_text),
    )))]
}

/// Builds the bottom-border hint line from the *effective* launcher table:
/// one `key label` per footer-tagged row (its first bound key — a remap's
/// alternate keys still resolve, this is just the label shown), joined in
/// table order. Mirrors `super::footer`'s "only footer-tagged rows get a
/// hint" rule, but reads `app.modal_keys.review_launcher` directly rather
/// than going through that module's merge helper, since this renders inside
/// the modal's own border rather than the shared footer strip.
fn hint_line(table: &[ModalBinding<LauncherAction>]) -> String {
    table
        .iter()
        .filter_map(|b| {
            let hint = b.footer?;
            let key = b.keys.first()?.label();
            Some(format!("{key} {}", hint.label))
        })
        .collect::<Vec<_>>()
        .join("  ")
}

/// Renders the Review launcher modal, centered over `area`. A no-op if
/// `app.mode` isn't [`super::app::Mode::ReviewLauncher`] (the caller should
/// only invoke this in that mode).
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let super::app::Mode::ReviewLauncher { tab, .. } = app.mode else {
        return;
    };
    let popup = centered(area, 80, 60);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(tab_bar(tab, &app.theme))
        .title_bottom(Line::from(format!(
            " {} ",
            hint_line(&app.modal_keys.review_launcher)
        )));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let [hint_area, list_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);

    frame.render_widget(
        Line::from(Span::styled(
            enter_outcome_hint(tab),
            Style::default().fg(app.theme.footer_text),
        )),
        hint_area,
    );

    let list = List::new(placeholder_rows(tab, &app.theme));
    frame.render_widget(list, list_area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::FileDiff;
    use crate::git::RawFilePatch;
    use crate::ui::app::{Mode, ModeOrigin};
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

    fn render_launcher(app: &App) -> String {
        // Wide enough that the bottom-border hint line (built from the full
        // effective key table) never gets clipped by ratatui's title-width
        // truncation.
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
    fn renders_nothing_outside_review_launcher_mode() {
        let app = App::new(vec![sample_file()]);
        let content = render_launcher(&app);
        assert!(content.trim().is_empty());
    }

    #[test]
    fn branches_tab_shows_its_placeholder_and_enter_outcome() {
        let mut app = App::new(vec![sample_file()]);
        app.mode = Mode::ReviewLauncher {
            tab: LauncherTab::Branches,
            cursor: 0,
            origin: ModeOrigin::Normal,
        };
        let content = render_launcher(&app);
        assert!(content.contains("Branches"));
        assert!(content.contains("Commits"));
        assert!(content.contains("branch list"));
        assert!(content.contains("start branch review"));
    }

    #[test]
    fn commits_tab_shows_its_placeholder_and_enter_outcome() {
        let mut app = App::new(vec![sample_file()]);
        app.mode = Mode::ReviewLauncher {
            tab: LauncherTab::Commits,
            cursor: 0,
            origin: ModeOrigin::Normal,
        };
        let content = render_launcher(&app);
        assert!(content.contains("commit list"));
        assert!(content.contains("review commit (read-only)"));
    }

    #[test]
    fn footer_hint_line_reflects_the_effective_table() {
        let mut app = App::new(vec![sample_file()]);
        app.mode = Mode::ReviewLauncher {
            tab: LauncherTab::Branches,
            cursor: 0,
            origin: ModeOrigin::Normal,
        };
        let content = render_launcher(&app);
        assert!(content.contains("switch tab"));
        assert!(content.contains("confirm"));
        assert!(content.contains("close"));
    }
}
