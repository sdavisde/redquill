//! The branch/worktree switcher modal ([`super::app::Mode::Switcher`]): a
//! centered overlay with two tabs — Branches (default) and
//! Worktrees — each listing the rows [`super::switcher::SwitcherState`]'s
//! per-tab cursor moves over. Modeled on [`super::compose_modal`] (a
//! centered, `Clear`-ed, bordered block) and [`super::git_panel`]'s row
//! styling (selected row reversed). Supports the shared `/` fuzzy filter
//! (spec 12 FR-7..FR-9), narrowing the active tab; toggling tabs clears the
//! filter (see [`super::switcher::SwitcherState::filter`]'s doc).

use std::path::Path;

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};

use crate::git::WorktreeEntry;

use super::app::App;
use super::switcher::{SwitcherState, SwitcherTab, is_current_worktree};
use super::theme::Theme;

/// Number of hex characters shown for a detached worktree's short head oid.
const SHORT_HEAD_LEN: usize = 7;

/// Centers a `width_pct`% x `height_pct`% rect inside `area`.
fn centered(area: Rect, width_pct: u16, height_pct: u16) -> Rect {
    let [area] = Layout::horizontal([Constraint::Percentage(width_pct)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::vertical([Constraint::Percentage(height_pct)])
        .flex(Flex::Center)
        .areas(area);
    area
}

/// The ` Branches │ Worktrees ` tab bar, active tab emphasized, rendered as
/// the modal's block title.
fn tab_bar(active: SwitcherTab, theme: &Theme) -> Line<'static> {
    let active_style = Style::default()
        .fg(theme.help_key)
        .add_modifier(Modifier::BOLD);
    let inactive_style = Style::default().fg(theme.footer_text);
    let (branches_style, worktrees_style) = match active {
        SwitcherTab::Branches => (active_style, inactive_style),
        SwitcherTab::Worktrees => (inactive_style, active_style),
    };
    Line::from(vec![
        Span::raw(" "),
        Span::styled("Branches", branches_style),
        Span::styled(" \u{2502} ", Style::default().fg(theme.footer_text)),
        Span::styled("Worktrees", worktrees_style),
        Span::raw(" "),
    ])
}

/// A path's final component, falling back to the full display form for a
/// path with none (e.g. `/`).
fn basename(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

/// A worktree's primary badge: its branch name, `detached @ <short-head>`,
/// or `bare`.
fn worktree_badge(wt: &WorktreeEntry) -> String {
    if wt.bare {
        "bare".to_string()
    } else if let Some(branch) = &wt.branch {
        branch.clone()
    } else if wt.detached {
        let short = wt
            .head
            .as_deref()
            .map(|h| &h[..h.len().min(SHORT_HEAD_LEN)])
            .unwrap_or("?");
        format!("detached @ {short}")
    } else {
        "unknown".to_string()
    }
}

/// A worktree's dimmed locked/prunable badges, in that order, empty if
/// neither applies.
fn worktree_extra_badges(wt: &WorktreeEntry) -> Vec<String> {
    let mut badges = Vec::new();
    if let Some(reason) = &wt.locked {
        badges.push(if reason.is_empty() {
            "locked".to_string()
        } else {
            format!("locked: {reason}")
        });
    }
    if let Some(reason) = &wt.prunable {
        badges.push(if reason.is_empty() {
            "prunable".to_string()
        } else {
            format!("prunable: {reason}")
        });
    }
    badges
}

/// The Branches tab's rows, restricted to `order` (indices into
/// `state.branches`, in render order — the full `0..len` identity order with
/// no active filter, or [`super::list_filter::ListFilter::indices`]'s
/// filtered/ranked order otherwise): a `*` marker on the current branch, and
/// a dimmed `(worktree: <basename>)` suffix on any branch checked out in
/// another worktree.
fn branch_rows(state: &SwitcherState, order: &[usize], theme: &Theme) -> Vec<ListItem<'static>> {
    order
        .iter()
        .filter_map(|&i| state.branches.get(i))
        .map(|b| {
            let marker = if b.is_current { "* " } else { "  " };
            let mut spans = vec![Span::raw(marker.to_string()), Span::raw(b.name.clone())];
            if !b.is_current
                && let Some(wt_path) = b.worktree.as_deref()
            {
                spans.push(Span::styled(
                    format!(" (worktree: {})", basename(wt_path)),
                    Style::default()
                        .fg(theme.footer_text)
                        .add_modifier(Modifier::DIM),
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect()
}

/// The Worktrees tab's rows, restricted to `order` (see [`branch_rows`]'s
/// identical convention): basename, the `[branch | detached @ <short> |
/// bare]` badge, dimmed locked/prunable badges, and a `*` marker on the
/// current worktree (by canonicalized path).
fn worktree_rows(
    state: &SwitcherState,
    order: &[usize],
    repo_root: Option<&Path>,
    theme: &Theme,
) -> Vec<ListItem<'static>> {
    order
        .iter()
        .filter_map(|&i| state.worktrees.get(i))
        .map(|wt| {
            let marker = if is_current_worktree(repo_root, wt) {
                "* "
            } else {
                "  "
            };
            let mut spans = vec![
                Span::raw(marker.to_string()),
                Span::raw(basename(&wt.path)),
                Span::raw(format!(" [{}]", worktree_badge(wt))),
            ];
            for badge in worktree_extra_badges(wt) {
                spans.push(Span::styled(
                    format!(" [{badge}]"),
                    Style::default()
                        .fg(theme.footer_text)
                        .add_modifier(Modifier::DIM),
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect()
}

/// Renders the branch/worktree switcher modal, centered over `area`. A
/// no-op if `app.switcher` is `None` (the caller should only invoke this in
/// [`super::app::Mode::Switcher`]).
///
/// A `/` filter (spec 12 FR-7..FR-9) adds a one-row chrome line below the
/// tab bar showing the live/locked query, narrows the active tab's rows to
/// the filtered view, and shows a "no matches" hint in place of a blank
/// list; an underlying empty tab (no branches/worktrees at all) keeps its
/// pre-existing "no local branches"/"no worktrees" hint regardless.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(state) = &app.switcher else {
        return;
    };
    let popup = centered(area, 80, 60);
    frame.render_widget(Clear, popup);

    let (tab_len, cursor) = match state.tab {
        SwitcherTab::Branches => (state.branches.len(), state.branch_cursor),
        SwitcherTab::Worktrees => (state.worktrees.len(), state.worktree_cursor),
    };
    let order: Vec<usize> = match &state.filter {
        Some(f) => f.indices().to_vec(),
        None => (0..tab_len).collect(),
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(tab_bar(state.tab, &app.theme))
        .title_bottom(Line::from(" Enter switch  Tab tabs  / filter  Esc close "));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let (chrome_area, list_area) = match &state.filter {
        Some(_) => {
            let [chrome, list] =
                Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
            (Some(chrome), list)
        }
        None => (None, inner),
    };

    if let (Some(chrome_area), Some(filter)) = (chrome_area, state.filter.as_ref()) {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                super::list_filter::chrome_text(filter),
                Style::default().fg(app.theme.search_prompt),
            ))),
            chrome_area,
        );
    }

    if tab_len > 0
        && let Some(filter) = state.filter.as_ref().filter(|f| f.is_empty())
    {
        let hint = Paragraph::new(super::list_filter::empty_hint(filter));
        frame.render_widget(hint, list_area);
        return;
    }

    let items = match state.tab {
        SwitcherTab::Branches if tab_len == 0 => vec![ListItem::new(Line::from(Span::styled(
            "  no local branches",
            Style::default().fg(app.theme.footer_text),
        )))],
        SwitcherTab::Worktrees if tab_len == 0 => vec![ListItem::new(Line::from(Span::styled(
            "  no worktrees",
            Style::default().fg(app.theme.footer_text),
        )))],
        SwitcherTab::Branches => branch_rows(state, &order, &app.theme),
        SwitcherTab::Worktrees => {
            worktree_rows(state, &order, app.repo_root.as_deref(), &app.theme)
        }
    };
    let list = List::new(items).highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    let mut list_state = ListState::default();
    if tab_len > 0 {
        list_state.select(Some(cursor));
    }
    frame.render_stateful_widget(list, list_area, &mut list_state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::FileDiff;
    use crate::git::{LocalBranch, RawFilePatch};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::path::PathBuf;

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

    fn render_switcher(app: &App) -> String {
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
    fn renders_nothing_when_switcher_is_none() {
        let app = App::new(vec![sample_file()]);
        let content = render_switcher(&app);
        assert!(content.trim().is_empty());
    }

    #[test]
    fn switcher_modal_renders_tabs_branches_and_current_marker() {
        let mut app = App::new(vec![sample_file()]);
        let branches = vec![
            LocalBranch {
                name: "main".to_string(),
                is_current: true,
                worktree: None,
            },
            LocalBranch {
                name: "feature".to_string(),
                is_current: false,
                worktree: Some(PathBuf::from("/repo/.worktrees/feature")),
            },
        ];
        app.switcher = Some(SwitcherState::new(branches, vec![], None, 0));
        let content = render_switcher(&app);
        assert!(content.contains("Branches"));
        assert!(content.contains("Worktrees"));
        assert!(content.contains("main"));
        assert!(content.contains("feature"));
        assert!(content.contains("(worktree: feature)"));
        assert!(content.contains('*'));
    }

    #[test]
    fn switcher_empty_branches_shows_empty_state() {
        let mut app = App::new(vec![sample_file()]);
        app.switcher = Some(SwitcherState::new(vec![], vec![], None, 0));
        let content = render_switcher(&app);
        assert!(content.contains("no local branches"));
    }

    #[test]
    fn switcher_modal_worktrees_tab_marks_current_and_detached() {
        let mut app = App::new(vec![sample_file()]);
        app.repo_root = Some(PathBuf::from("/repo"));
        let worktrees = vec![
            WorktreeEntry {
                path: PathBuf::from("/repo"),
                head: Some("deadbeef".to_string()),
                branch: Some("main".to_string()),
                bare: false,
                detached: false,
                locked: None,
                prunable: None,
            },
            WorktreeEntry {
                path: PathBuf::from("/repo/.worktrees/spike"),
                head: Some("cafef00dabc".to_string()),
                branch: None,
                bare: false,
                detached: true,
                locked: Some("".to_string()),
                prunable: None,
            },
        ];
        let mut state = SwitcherState::new(vec![], worktrees, Some(Path::new("/repo")), 0);
        state.toggle_tab();
        app.switcher = Some(state);
        let content = render_switcher(&app);
        assert!(content.contains("repo"));
        assert!(content.contains("spike"));
        assert!(content.contains("detached @ cafef00"));
        assert!(content.contains("locked"));
        assert!(content.contains('*'));
    }

    #[test]
    fn switcher_empty_worktrees_shows_empty_state() {
        let mut app = App::new(vec![sample_file()]);
        let mut state = SwitcherState::new(vec![], vec![], None, 0);
        state.toggle_tab();
        app.switcher = Some(state);
        let content = render_switcher(&app);
        assert!(content.contains("no worktrees"));
    }
}
