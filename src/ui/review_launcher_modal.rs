//! The Review launcher modal ([`super::app::Mode::ReviewLauncher`]): a
//! centered overlay with two tabs — Branches (default) and Commits — styled
//! like [`super::switcher_modal`] (centered `Clear`-ed bordered block, tab
//! headers as the block title, active tab emphasized, cursor row reverse-
//! highlighted). The Branches tab renders `app.launcher_branches`; the
//! Commits tab renders whichever source [`super::review_launcher`]'s
//! `App::launcher_commits_rows` selects (ahead-of-base or the full log),
//! reusing [`super::git_panel::history_item`] so both surfaces render
//! commits identically. The footer hint line is built from the *effective*
//! `REVIEW_LAUNCHER_KEYS` table (`app.modal_keys.review_launcher`) rather
//! than a hardcoded string, so a `[keys.review-launcher]` remap shows up
//! here with no extra wiring — including the Commits tab's empty-state
//! line, which names whatever key the table currently binds to
//! `toggle-all-commits`.

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};

use crate::git::{CommitLogEntry, LocalBranch};

use super::app::App;
use super::git_panel::history_item;
use super::modal_keys::{LauncherAction, ModalBinding};
use super::review_launcher::LauncherTab;
use super::theme::Theme;
use super::time_format::now_unix;

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

/// The key label the Commits tab's empty state names, read from the
/// *effective* launcher table rather than hardcoded `"a"` — a
/// `[keys.review-launcher]` remap of `toggle-all-commits` is reflected here
/// automatically, keeping the hint truthful (FR-13).
fn toggle_all_commits_key_label(table: &[ModalBinding<LauncherAction>]) -> String {
    table
        .iter()
        .find(|b| b.action == LauncherAction::ToggleAllCommits)
        .and_then(|b| b.keys.first())
        .map(|k| k.label())
        .unwrap_or_else(|| "a".to_string())
}

/// The Commits tab's empty-state line: distinguishes "the ahead-of-base
/// range has nothing in it" (names the toggle key, so there's always a next
/// step) from "the full log itself is empty" (a brand-new repo — the same
/// wording the History tab's own empty state uses, since toggling further
/// wouldn't help).
fn commits_empty_state_line(table: &[ModalBinding<LauncherAction>], all_commits: bool) -> String {
    if all_commits {
        "no commits".to_string()
    } else {
        format!(
            "no commits ahead of base — press {} for all commits",
            toggle_all_commits_key_label(table)
        )
    }
}

/// The Commits tab's rows: a loading placeholder while the active source's
/// first fetch is still in flight, the empty-state line naming the toggle
/// key once a load has landed with nothing in it, or real
/// [`CommitLogEntry`] rows rendered via [`history_item`] (matching the
/// History tab's row style exactly) — newest first, cursor starting on the
/// newest.
fn commits_rows(app: &App, theme: &Theme, content_width: usize) -> Vec<ListItem<'static>> {
    if app.launcher_commits_loading() {
        return vec![ListItem::new(Line::from(Span::styled(
            "  loading\u{2026}",
            Style::default().fg(theme.footer_text),
        )))];
    }
    let commits: &[CommitLogEntry] = app.launcher_commits_rows();
    if commits.is_empty() {
        return vec![ListItem::new(Line::from(Span::styled(
            format!(
                "  {}",
                commits_empty_state_line(&app.modal_keys.review_launcher, app.launcher_all_commits)
            ),
            Style::default().fg(theme.footer_text),
        )))];
    }
    let now = now_unix();
    commits
        .iter()
        .map(|c| history_item(c, false, now, theme, content_width))
        .collect()
}

/// The Branches tab's rows: local branches excluding the current one,
/// mirroring the retired review-branch modal's `no other local branches`
/// empty state exactly (nothing to review when every branch is checked out
/// already, or the repo has only one).
fn branch_rows(branches: &[LocalBranch], theme: &Theme) -> Vec<ListItem<'static>> {
    if branches.is_empty() {
        return vec![ListItem::new(Line::from(Span::styled(
            "  no other local branches",
            Style::default().fg(theme.footer_text),
        )))];
    }
    branches
        .iter()
        .map(|b| ListItem::new(Line::from(Span::raw(format!("  {}", b.name)))))
        .collect()
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
    let super::app::Mode::ReviewLauncher { tab, cursor, .. } = app.mode else {
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

    let rows = if app.status_message.is_some() {
        Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(inner)
    } else {
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(inner)
    };
    let hint_area = rows[0];
    let list_area = rows[1];

    frame.render_widget(
        Line::from(Span::styled(
            enter_outcome_hint(tab),
            Style::default().fg(app.theme.footer_text),
        )),
        hint_area,
    );

    let (items, selectable) = match tab {
        LauncherTab::Branches => (
            branch_rows(&app.launcher_branches, &app.theme),
            !app.launcher_branches.is_empty(),
        ),
        LauncherTab::Commits => {
            let selectable =
                !app.launcher_commits_loading() && !app.launcher_commits_rows().is_empty();
            (
                commits_rows(app, &app.theme, list_area.width as usize),
                selectable,
            )
        }
    };
    let list = List::new(items).highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    let mut list_state = ListState::default();
    if selectable {
        list_state.select(Some(cursor));
    }
    frame.render_stateful_widget(list, list_area, &mut list_state);

    if let Some(message) = &app.status_message {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                message.clone(),
                Style::default().fg(app.theme.status_message),
            ))),
            rows[2],
        );
    }
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
    fn branches_tab_shows_its_empty_state_and_enter_outcome_without_a_backend() {
        let mut app = App::new(vec![sample_file()]);
        app.mode = Mode::ReviewLauncher {
            tab: LauncherTab::Branches,
            cursor: 0,
            origin: ModeOrigin::Normal,
        };
        let content = render_launcher(&app);
        assert!(content.contains("Branches"));
        assert!(content.contains("Commits"));
        assert!(content.contains("no other local branches"));
        assert!(content.contains("start branch review"));
    }

    #[test]
    fn branches_tab_renders_real_branches_with_the_cursor_highlighted() {
        let mut app = App::new(vec![sample_file()]);
        app.launcher_branches = vec![
            crate::git::LocalBranch {
                name: "alpha".to_string(),
                is_current: false,
                worktree: None,
            },
            crate::git::LocalBranch {
                name: "zulu".to_string(),
                is_current: false,
                worktree: None,
            },
        ];
        app.mode = Mode::ReviewLauncher {
            tab: LauncherTab::Branches,
            cursor: 1,
            origin: ModeOrigin::Normal,
        };
        let content = render_launcher(&app);
        assert!(content.contains("alpha"));
        assert!(content.contains("zulu"));
        assert!(!content.contains("no other local branches"));
    }

    #[test]
    fn a_status_message_renders_inside_the_modal() {
        let mut app = App::new(vec![sample_file()]);
        app.mode = Mode::ReviewLauncher {
            tab: LauncherTab::Branches,
            cursor: 0,
            origin: ModeOrigin::Normal,
        };
        app.set_status_message("already reviewing feature \u{2014} press q to finish or pause");
        let content = render_launcher(&app);
        assert!(content.contains("already reviewing feature"));
    }

    #[test]
    fn commits_tab_shows_its_empty_state_and_enter_outcome_without_a_backend() {
        let mut app = App::new(vec![sample_file()]);
        app.mode = Mode::ReviewLauncher {
            tab: LauncherTab::Commits,
            cursor: 0,
            origin: ModeOrigin::Normal,
        };
        let content = render_launcher(&app);
        assert!(content.contains("no commits ahead of base"));
        assert!(content.contains("all commits"), "must name the toggle key");
        assert!(content.contains("review commit (read-only)"));
    }

    #[test]
    fn commits_tab_shows_a_loading_placeholder_while_a_fetch_is_in_flight() {
        let mut app = App::new(vec![sample_file()]);
        let id = app.launcher_commits_tasks.spawn(|| {
            Some(vec![crate::git::CommitLogEntry {
                sha: "a".to_string(),
                short_sha: "a".to_string(),
                subject: "one".to_string(),
                author_name: "Dev".to_string(),
                timestamp: 1_700_000_000,
            }])
        });
        app.launcher_commits_in_flight =
            Some(super::super::review_launcher::InFlightLauncherCommits {
                id,
                generation: app.launcher_commits_generation,
            });
        app.mode = Mode::ReviewLauncher {
            tab: LauncherTab::Commits,
            cursor: 0,
            origin: ModeOrigin::Normal,
        };
        let content = render_launcher(&app);
        assert!(content.contains("loading"));
    }

    #[test]
    fn commits_tab_renders_real_commits_with_the_cursor_highlighted() {
        let mut app = App::new(vec![sample_file()]);
        app.launcher_commits = vec![
            crate::git::CommitLogEntry {
                sha: "aaa".to_string(),
                short_sha: "aaa".to_string(),
                subject: "add widget".to_string(),
                author_name: "Dev".to_string(),
                timestamp: 1_700_000_000,
            },
            crate::git::CommitLogEntry {
                sha: "bbb".to_string(),
                short_sha: "bbb".to_string(),
                subject: "fix widget".to_string(),
                author_name: "Dev".to_string(),
                timestamp: 1_700_000_100,
            },
        ];
        app.mode = Mode::ReviewLauncher {
            tab: LauncherTab::Commits,
            cursor: 1,
            origin: ModeOrigin::Normal,
        };
        let content = render_launcher(&app);
        assert!(content.contains("add widget"));
        assert!(content.contains("fix widget"));
        assert!(!content.contains("no commits"));
    }

    #[test]
    fn commits_tab_all_commits_empty_state_says_no_commits_without_a_toggle_hint() {
        let mut app = App::new(vec![sample_file()]);
        app.launcher_all_commits = true;
        app.mode = Mode::ReviewLauncher {
            tab: LauncherTab::Commits,
            cursor: 0,
            origin: ModeOrigin::Normal,
        };
        let content = render_launcher(&app);
        assert!(content.contains("no commits"));
        assert!(
            !content.contains("ahead of base"),
            "the full log's own empty state names no ahead-of-base range"
        );
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
