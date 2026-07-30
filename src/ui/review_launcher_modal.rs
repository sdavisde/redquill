//! The Review launcher modal ([`super::app::Mode::ReviewLauncher`]): a
//! centered overlay with three tabs — Branches (default), Commits, and Pull
//! Requests — styled like [`super::switcher_modal`] (centered `Clear`-ed
//! bordered block, tab headers as the block title, active tab emphasized,
//! cursor row reverse-highlighted). The Branches tab renders
//! `app.launcher_branches`; the Commits tab renders whichever source
//! [`super::review_launcher`]'s `App::launcher_commits_rows` selects
//! (ahead-of-base or the full log), reusing [`super::git_panel::history_item`]
//! so both surfaces render commits identically. The Pull Requests tab
//! renders `app.launcher_prs` — either a listing, a loading placeholder, the
//! zero-open-PRs empty state, or one of the degraded-state prescriptions
//! (see [`prs_degraded_body_lines`]) — never a blank body. The modal carries
//! no key-hint chrome of its own: the shared footer strip below it shows this
//! mode's footer-tagged rows, tab-scoped through
//! `super::modal_keys::launcher_action_tab_scope` (see `super::footer`).
//! The Commits tab's empty-state line still reads the *effective*
//! `REVIEW_LAUNCHER_KEYS` table (`app.modal_keys.review_launcher`), naming
//! whatever key the table currently binds to `toggle-all-commits`, so a
//! `[keys.review-launcher]` remap shows up with no extra wiring.

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};

use crate::forge::{PullRequest, UnresolvedReason};
use crate::git::{CommitLogEntry, LocalBranch};

use super::app::App;
use super::git_panel::history_item;
use super::modal_keys::{LauncherAction, ModalBinding};
use super::review_launcher::LauncherTab;
use super::stage_ops::PrFetchOutcome;
use super::theme::Theme;
use super::time_format::{now_unix, parse_rfc3339_to_unix, relative_time};

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

/// The ` Branches │ Commits │ Pull Requests ` tab bar, active tab
/// emphasized, rendered as the modal's block title — mirrors
/// [`super::switcher_modal::tab_bar`] exactly.
fn tab_bar(active: LauncherTab, theme: &Theme) -> Line<'static> {
    let active_style = Style::default()
        .fg(theme.help_key)
        .add_modifier(Modifier::BOLD);
    let inactive_style = Style::default().fg(theme.footer_text);
    let (branches_style, commits_style, prs_style) = match active {
        LauncherTab::Branches => (active_style, inactive_style, inactive_style),
        LauncherTab::Commits => (inactive_style, active_style, inactive_style),
        LauncherTab::PullRequests => (inactive_style, inactive_style, active_style),
    };
    Line::from(vec![
        Span::raw(" "),
        Span::styled("Branches", branches_style),
        Span::styled(" \u{2502} ", Style::default().fg(theme.footer_text)),
        Span::styled("Commits", commits_style),
        Span::styled(" \u{2502} ", Style::default().fg(theme.footer_text)),
        Span::styled("Pull Requests", prs_style),
        Span::raw(" "),
    ])
}

/// What `Enter` does on `tab` — shown as its own line so the tabs' differing
/// weight (a lightweight read-only peek vs. starting a full worktree
/// session) is unambiguous before the user presses it. The Pull Requests
/// tab names its real behavior: `Enter` checks the PR out into a managed
/// worktree and starts a review session, the same weight as the Branches
/// tab (see [`super::app::App::confirm_launcher_pr`]).
fn enter_outcome_hint(tab: LauncherTab) -> &'static str {
    match tab {
        LauncherTab::Branches => "Enter: start branch review",
        LauncherTab::Commits => "Enter: review commit (read-only)",
        LauncherTab::PullRequests => "Enter: start PR review",
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
/// newest. `order` restricts which commits render and in what sequence
/// (indices into the active source, in render order — the full `0..len`
/// identity order with no active filter, or
/// [`super::list_filter::ListFilter::indices`]'s filtered/ranked order
/// otherwise, spec 12 FR-12); the loading/raw-empty states above take
/// priority over `order` since a filter has nothing real to narrow yet in
/// either case.
fn commits_rows(
    app: &App,
    order: &[usize],
    theme: &Theme,
    content_width: usize,
) -> Vec<ListItem<'static>> {
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
    order
        .iter()
        .filter_map(|&i| commits.get(i))
        .map(|c| history_item(c, false, now, theme, content_width))
        .collect()
}

/// The Branches tab's rows: local branches excluding the current one,
/// mirroring the retired review-branch modal's `no other local branches`
/// empty state exactly (nothing to review when every branch is checked out
/// already, or the repo has only one) — regardless of `order`, since
/// filtering nothing is still nothing. `order` restricts which branches
/// render and in what sequence (see [`commits_rows`]'s identical
/// convention, spec 12 FR-12).
fn branch_rows(branches: &[LocalBranch], order: &[usize], theme: &Theme) -> Vec<ListItem<'static>> {
    if branches.is_empty() {
        return vec![ListItem::new(Line::from(Span::styled(
            "  no other local branches",
            Style::default().fg(theme.footer_text),
        )))];
    }
    order
        .iter()
        .filter_map(|&i| branches.get(i))
        .map(|b| ListItem::new(Line::from(Span::raw(format!("  {}", b.name)))))
        .collect()
}

/// Where to point a reviewer to install a forge CLI they don't have yet —
/// fixed per CLI name (never derived from anything user-controlled, since
/// this is display copy only).
fn cli_install_pointer(cli: &str) -> &'static str {
    match cli {
        "glab" => "install glab: https://gitlab.com/gitlab-org/cli",
        _ => "install gh: https://cli.github.com",
    }
}

/// The PR row's "updated" text: a relative label (`"2d ago"`, matching the
/// Commits tab and the forge thread overlay) when `updated_at` parses as
/// RFC 3339, or the raw provider string verbatim when it doesn't — a
/// malformed/unexpected timestamp shape degrades to the same value already
/// on screen today rather than blanking the field or panicking.
fn pr_updated_display(now: i64, updated_at: &str) -> String {
    parse_rfc3339_to_unix(updated_at)
        .map(|ts| relative_time(now, ts))
        .unwrap_or_else(|| updated_at.to_string())
}

/// One PR row: `#<number> <title>` as the primary line (matching
/// [`branch_rows`]' plain-name weight), a right-aligned "draft" marker when
/// applicable, and a secondary meta line (author, source branch, updated
/// time) — draft and updated-time both visually secondary to number+title,
/// mirroring [`history_item`]'s two-tier weight.
fn pr_row(pr: &PullRequest, now: i64, theme: &Theme, content_width: usize) -> ListItem<'static> {
    let mut primary = Line::from(vec![
        Span::styled(
            format!("#{} ", pr.number),
            Style::default().fg(theme.dir_prefix),
        ),
        Span::raw(pr.title.clone()),
    ]);
    if pr.is_draft {
        let label = "draft";
        let used = primary.width();
        let pad = content_width
            .saturating_sub(used + label.chars().count() + 1)
            .max(1);
        primary.spans.push(Span::raw(" ".repeat(pad)));
        primary
            .spans
            .push(Span::styled(label, Style::default().fg(theme.dir_prefix)));
    }
    let meta = format!(
        "\u{2502} {} \u{b7} {} \u{b7} {}",
        pr.author,
        pr.head_ref,
        pr_updated_display(now, &pr.updated_at)
    );
    ListItem::new(vec![
        primary,
        Line::from(Span::styled(meta, Style::default().fg(theme.dir_prefix))),
    ])
}

/// The zero-open-PRs empty state: success, not a diagnostic — rendered in
/// `kind_added` (the same "positive change" color the diff view uses for
/// additions) rather than `status_message`'s diagnostic tone, so it reads
/// visually distinct from every degraded body below.
fn prs_empty_state_line(repo_label: &str, theme: &Theme) -> Line<'static> {
    Line::from(Span::styled(
        format!("No open pull requests on {repo_label}"),
        Style::default().fg(theme.kind_added),
    ))
}

/// The body for every non-[`PrFetchOutcome::Loaded`] outcome: imperative,
/// copy-pasteable command lines first, prose explanation last (Design
/// Considerations' ordering for degraded-state bodies), with the real
/// hostname interpolated wherever the outcome carries one. Command lines
/// use `help_key`'s accent color (matching how the help overlay marks a
/// literal keystroke); prose uses `status_message`'s diagnostic tone.
fn prs_degraded_body_lines(outcome: &PrFetchOutcome, theme: &Theme) -> Vec<Line<'static>> {
    let cmd = |s: String| Line::from(Span::styled(s, Style::default().fg(theme.help_key)));
    let prose = |s: String| Line::from(Span::styled(s, Style::default().fg(theme.status_message)));
    match outcome {
        PrFetchOutcome::Loaded { .. } => Vec::new(),
        PrFetchOutcome::NoForgeRemote => vec![prose(
            "no forge remote — add a GitHub/GitLab `origin` remote to use this tab".to_string(),
        )],
        PrFetchOutcome::Unresolved { hostname, reason } => {
            let why = match reason {
                UnresolvedReason::NoCredentials => {
                    format!("neither CLI holds credentials for {hostname} — run one of the above")
                }
                UnresolvedReason::Ambiguous => format!(
                    "both CLIs hold credentials for {hostname} — redquill can't tell which forge this is"
                ),
            };
            vec![
                cmd(format!("gh auth login --hostname {hostname}")),
                cmd(format!("glab auth login --hostname {hostname}")),
                prose(why),
            ]
        }
        PrFetchOutcome::CliMissing { cli, hostname } => vec![
            cmd(cli_install_pointer(cli).to_string()),
            cmd(format!("{cli} auth login --hostname {hostname}")),
            prose(format!("{cli} isn't on PATH")),
        ],
        PrFetchOutcome::Unauthenticated { cli, hostname } => vec![
            cmd(format!("{cli} auth login --hostname {hostname}")),
            prose(format!(
                "{cli} is installed but not logged in for {hostname}"
            )),
        ],
        PrFetchOutcome::ListFailed { message } => vec![
            prose(message.clone()),
            prose("switch tabs and back to retry".to_string()),
        ],
    }
}

/// The key label the finished-reviews footer names, read from the *effective*
/// launcher table so a `[keys.review-launcher]` remap of
/// `cleanup-finished-reviews` is reflected here automatically (FR-5/FR-22).
fn cleanup_key_label(table: &[ModalBinding<LauncherAction>]) -> String {
    table
        .iter()
        .find(|b| b.action == LauncherAction::Cleanup)
        .and_then(|b| b.keys.first())
        .map(|k| k.label())
        .unwrap_or_else(|| "X".to_string())
}

/// Whether a landed listing of `count` rows might be missing rows beyond
/// the provider's fixed page cap ([`crate::forge::PR_LIST_CAP`]) — the only
/// signal available, since neither `gh` nor `glab` reports a listing's true
/// total, only the rows it actually returned. Pure so it's unit-testable
/// without a render pass; called fresh from the just-applied listing on
/// every render (first load and refresh alike), never cached in a
/// separately-updated flag.
fn prs_reached_cap(count: usize) -> bool {
    count >= crate::forge::PR_LIST_CAP
}

/// The "showing first N open PRs" truncation note: appended below the
/// listing whenever [`prs_reached_cap`] holds for the landed row count.
/// Says "first N", never an exact total — a landing at the cap is
/// indistinguishable from one that's merely equal to it, so the copy
/// doesn't overclaim. Styled in the secondary `footer_text` tone, matching
/// [`finished_reviews_footer_line`].
fn truncated_listing_footer_line(theme: &Theme) -> Line<'static> {
    Line::from(Span::styled(
        format!(
            "showing first {} open PRs \u{2014} narrow with /",
            crate::forge::PR_LIST_CAP
        ),
        Style::default().fg(theme.footer_text),
    ))
}

/// The "N finished review(s)" footer line: rendered whenever cleanup
/// candidates exist (managed reviews whose PR is no longer open), alongside
/// both a real listing and the zero-open-PRs empty state (FR-22). Names the
/// cleanup key so there is always a next step. Styled in the secondary
/// `footer_text` tone so it reads as chrome, not as a selectable PR row.
fn finished_reviews_footer_line(
    count: usize,
    table: &[ModalBinding<LauncherAction>],
    theme: &Theme,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{count} finished review(s) \u{2014} press "),
            Style::default().fg(theme.footer_text),
        ),
        Span::styled(
            cleanup_key_label(table),
            Style::default().fg(theme.help_key),
        ),
        Span::styled(" to clean up", Style::default().fg(theme.footer_text)),
    ])
}

/// The Pull Requests tab's rows: a loading placeholder while the first
/// fetch is in flight, real [`PullRequest`] rows via [`pr_row`] on a
/// successful non-empty listing, the zero-results empty state naming the
/// repo, or a degraded-state prescription — never a blank body. `order`
/// restricts which PRs render and in what sequence (see [`commits_rows`]'s
/// identical convention, spec 12 FR-12); loading/empty/degraded states take
/// priority over `order`, same precedence as every other tab. When the
/// landed listing hit the provider's page cap ([`prs_reached_cap`]), a
/// truncation note is appended below it. When cleanup candidates exist, a
/// non-selectable "N finished review(s)" footer line is appended after
/// that (FR-22).
fn prs_rows(
    app: &App,
    order: &[usize],
    theme: &Theme,
    content_width: usize,
) -> Vec<ListItem<'static>> {
    if app.launcher_prs_loading() {
        return vec![ListItem::new(Line::from(Span::styled(
            "  loading\u{2026}",
            Style::default().fg(theme.footer_text),
        )))];
    }
    let Some(outcome) = app.launcher_prs.as_ref() else {
        return vec![ListItem::new(Line::from(Span::styled(
            "  loading\u{2026}",
            Style::default().fg(theme.footer_text),
        )))];
    };
    let now = now_unix();
    let mut items = match outcome {
        PrFetchOutcome::Loaded { repo_label, prs } if prs.is_empty() => {
            vec![ListItem::new(prs_empty_state_line(repo_label, theme))]
        }
        PrFetchOutcome::Loaded { prs, .. } => order
            .iter()
            .filter_map(|&i| prs.get(i))
            .map(|pr| pr_row(pr, now, theme, content_width))
            .collect(),
        degraded => return vec![ListItem::new(prs_degraded_body_lines(degraded, theme))],
    };
    if let PrFetchOutcome::Loaded { prs, .. } = outcome
        && prs_reached_cap(prs.len())
    {
        items.push(ListItem::new(truncated_listing_footer_line(theme)));
    }
    let finished = app.launcher_finished_reviews.len();
    if finished > 0 {
        items.push(ListItem::new(finished_reviews_footer_line(
            finished,
            &app.modal_keys.review_launcher,
            theme,
        )));
    }
    items
}

/// Renders the Review launcher modal, centered over `area`. A no-op if
/// `app.mode` isn't [`super::app::Mode::ReviewLauncher`] (the caller should
/// only invoke this in that mode).
///
/// A `/` filter (spec 12 FR-12) adds a one-row chrome line below the
/// outcome-hint line showing the live/locked query, narrows the active
/// tab's rows to the filtered view, and shows a "no matches" hint in place
/// of a blank list; an underlying empty/loading tab keeps its pre-existing
/// hint regardless (see [`branch_rows`]/[`commits_rows`]'s identical
/// precedence — mirrors [`super::switcher_modal::render`]'s equivalent
/// filter-chrome layout for the switcher modal).
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let super::app::Mode::ReviewLauncher { tab, cursor, .. } = app.mode else {
        return;
    };
    let popup = centered(area, 80, 60);
    frame.render_widget(Clear, popup);

    // No bottom-border key hints: the shared footer strip below the modal
    // already shows this mode's footer-tagged rows, tab-scoped.
    let block = Block::default()
        .borders(Borders::ALL)
        .title(tab_bar(tab, &app.theme));
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
    let mut list_area = rows[1];

    frame.render_widget(
        Line::from(Span::styled(
            enter_outcome_hint(tab),
            Style::default().fg(app.theme.footer_text),
        )),
        hint_area,
    );

    let chrome_area = if app.launcher_filter.is_some() {
        let [chrome, rest] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(list_area);
        list_area = rest;
        Some(chrome)
    } else {
        None
    };
    if let (Some(chrome_area), Some(filter)) = (chrome_area, app.launcher_filter.as_ref()) {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                super::list_filter::chrome_text(filter),
                Style::default().fg(app.theme.search_prompt),
            ))),
            chrome_area,
        );
    }

    let raw_len = match tab {
        LauncherTab::Branches => app.launcher_branches.len(),
        LauncherTab::Commits => app.launcher_commits_rows().len(),
        LauncherTab::PullRequests => app.launcher_prs_rows().len(),
    };
    let loading = (tab == LauncherTab::Commits && app.launcher_commits_loading())
        || (tab == LauncherTab::PullRequests && app.launcher_prs_loading());
    if raw_len > 0
        && !loading
        && let Some(filter) = app.launcher_filter.as_ref().filter(|f| f.is_empty())
    {
        let hint = Paragraph::new(super::list_filter::empty_hint(filter));
        frame.render_widget(hint, list_area);
        if let Some(message) = &app.status_message {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    message.clone(),
                    Style::default().fg(app.theme.status_message),
                ))),
                rows[2],
            );
        }
        return;
    }

    let order: Vec<usize> = match &app.launcher_filter {
        Some(f) => f.indices().to_vec(),
        None => (0..raw_len).collect(),
    };

    let (items, selectable) = match tab {
        LauncherTab::Branches => (
            branch_rows(&app.launcher_branches, &order, &app.theme),
            !app.launcher_branches.is_empty() && !order.is_empty(),
        ),
        LauncherTab::Commits => {
            let selectable =
                !loading && !app.launcher_commits_rows().is_empty() && !order.is_empty();
            (
                commits_rows(app, &order, &app.theme, list_area.width as usize),
                selectable,
            )
        }
        LauncherTab::PullRequests => {
            let selectable = !loading && !app.launcher_prs_rows().is_empty() && !order.is_empty();
            (
                prs_rows(app, &order, &app.theme, list_area.width as usize),
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
        // Wide enough that the tab bar and empty-state lines never get
        // clipped by ratatui's title-width truncation.
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
        app.launcher_commits_in_flight = Some(id);
        app.mode = Mode::ReviewLauncher {
            tab: LauncherTab::Commits,
            cursor: 0,
            origin: ModeOrigin::Normal,
        };
        let content = render_launcher(&app);
        assert!(content.contains("loading"));
    }

    /// The modal frame itself carries no key-hint chrome — the shared footer
    /// strip owns the hints (its tab-scoping is asserted in
    /// `super::super::footer`'s tests).
    #[test]
    fn the_modal_frame_carries_no_key_hint_chrome() {
        let mut app = App::new(vec![sample_file()]);
        app.mode = Mode::ReviewLauncher {
            tab: LauncherTab::Branches,
            cursor: 0,
            origin: ModeOrigin::Normal,
        };
        let content = render_launcher(&app);
        assert!(
            !content.contains("switch tab"),
            "hints belong to the footer strip, not the modal frame:\n{content}"
        );
    }

    // -- `/` filter chrome (spec 12 FR-12) ------------------------------------

    fn two_branches() -> Vec<LocalBranch> {
        vec![
            LocalBranch {
                name: "alpha".to_string(),
                is_current: false,
                worktree: None,
            },
            LocalBranch {
                name: "zulu".to_string(),
                is_current: false,
                worktree: None,
            },
        ]
    }

    #[test]
    fn filter_chrome_shows_the_live_query_and_narrows_the_branches_tab() {
        let mut app = App::new(vec![sample_file()]);
        app.launcher_branches = two_branches();
        app.mode = Mode::ReviewLauncher {
            tab: LauncherTab::Branches,
            cursor: 0,
            origin: ModeOrigin::Normal,
        };
        let labels: Vec<String> = app
            .launcher_branches
            .iter()
            .map(|b| b.name.clone())
            .collect();
        let mut filter = super::super::list_filter::ListFilter::open(&labels);
        filter.push_char('z', &labels);
        app.launcher_filter = Some(filter);

        let content = render_launcher(&app);
        assert!(
            content.contains("/z"),
            "must show the live query:\n{content}"
        );
        assert!(
            content.contains("zulu"),
            "the matching branch must render:\n{content}"
        );
        assert!(
            !content.contains("alpha"),
            "the non-matching branch must be narrowed out:\n{content}"
        );
    }

    #[test]
    fn filter_empty_state_renders_a_no_matches_hint_instead_of_a_blank_list() {
        let mut app = App::new(vec![sample_file()]);
        app.launcher_branches = two_branches();
        app.mode = Mode::ReviewLauncher {
            tab: LauncherTab::Branches,
            cursor: 0,
            origin: ModeOrigin::Normal,
        };
        let labels: Vec<String> = app
            .launcher_branches
            .iter()
            .map(|b| b.name.clone())
            .collect();
        let mut filter = super::super::list_filter::ListFilter::open(&labels);
        filter.push_char('q', &labels);
        filter.lock();
        app.launcher_filter = Some(filter);

        let content = render_launcher(&app);
        assert!(
            content.contains("no matches"),
            "an empty filtered view must show the hint, not a blank list:\n{content}"
        );
    }

    #[test]
    fn an_empty_branch_list_keeps_its_own_empty_state_even_under_a_filter() {
        // Filtering nothing is still nothing — the underlying "no other
        // local branches" hint must win over the filter's generic
        // "no matches" hint (mirrors `switcher_modal`'s identical
        // precedence).
        let mut app = App::new(vec![sample_file()]);
        app.mode = Mode::ReviewLauncher {
            tab: LauncherTab::Branches,
            cursor: 0,
            origin: ModeOrigin::Normal,
        };
        app.launcher_filter = Some(super::super::list_filter::ListFilter::open(&[]));

        let content = render_launcher(&app);
        assert!(content.contains("no other local branches"));
    }

    // -- Pull Requests tab: every FR-5 state ---------------------------------

    fn pr(number: u64, title: &str, author: &str, head_ref: &str, is_draft: bool) -> PullRequest {
        PullRequest {
            number,
            title: title.to_string(),
            author: author.to_string(),
            head_ref: head_ref.to_string(),
            base_ref: "main".to_string(),
            is_draft,
            updated_at: "2026-07-15T12:00:00Z".to_string(),
        }
    }

    fn prs_app(outcome: PrFetchOutcome) -> App {
        let mut app = App::new(vec![sample_file()]);
        app.mode = Mode::ReviewLauncher {
            tab: LauncherTab::PullRequests,
            cursor: 0,
            origin: ModeOrigin::Normal,
        };
        app.launcher_prs = Some(outcome);
        app
    }

    #[test]
    fn prs_tab_renders_number_title_author_branch_draft_and_updated_time() {
        let app = prs_app(PrFetchOutcome::Loaded {
            repo_label: "org/repo".to_string(),
            prs: vec![pr(42, "add widgets", "octocat", "widgets-branch", true)],
        });
        let content = render_launcher(&app);
        assert!(content.contains("#42"));
        assert!(content.contains("add widgets"));
        assert!(content.contains("octocat"));
        assert!(content.contains("widgets-branch"));
        assert!(content.contains("draft"));
        // Relative, not the raw ISO timestamp (see
        // `pr_updated_display_relatives_a_valid_timestamp_and_falls_back_on_garbage`
        // below for the exact bucketing) — a fixed past date always renders
        // some "... ago" label, so this stays true no matter when the suite
        // runs.
        assert!(content.contains("ago"));
        assert!(!content.contains("2026-07-15T12:00:00Z"));
    }

    #[test]
    fn pr_updated_display_relatives_a_valid_timestamp_and_falls_back_on_garbage() {
        let now = 1_700_000_000; // fixed instant, independent of wall clock
        let cases: &[(&str, &str)] = &[
            // Valid, recent: relative, not raw.
            ("2023-11-14T22:13:20Z", "just now"),
            // Valid, long past: relative, not raw — this is the case that
            // would silently regress to printing the raw ISO string if
            // someone dropped the parse step.
            ("2022-11-14T22:13:20Z", "1y ago"),
            // Unparseable: falls back to the raw string verbatim rather than
            // panicking or blanking the field.
            ("not-a-timestamp", "not-a-timestamp"),
            ("", ""),
        ];
        for &(input, expected) in cases {
            assert_eq!(pr_updated_display(now, input), expected, "input {input:?}");
        }
    }

    #[test]
    fn prs_tab_zero_open_prs_names_the_repo_and_reads_as_success_not_a_diagnostic() {
        let app = prs_app(PrFetchOutcome::Loaded {
            repo_label: "sdavisde/redquill".to_string(),
            prs: Vec::new(),
        });
        let content = render_launcher(&app);
        assert!(content.contains("No open pull requests on sdavisde/redquill"));
        // Distinct from a degraded body: no auth/install/retry language
        // anywhere near the empty-state line.
        assert!(!content.contains("auth login"));
        assert!(!content.contains("retry"));
    }

    /// Every degraded PR-tab outcome must render a body that names its
    /// recovery action — none may fall through to a blank list.
    #[test]
    fn prs_tab_degraded_bodies_name_the_recovery_action() {
        let cases: Vec<(PrFetchOutcome, Vec<&str>)> = vec![
            (
                PrFetchOutcome::Unresolved {
                    hostname: "git.example.com".to_string(),
                    reason: UnresolvedReason::NoCredentials,
                },
                vec![
                    "gh auth login --hostname git.example.com",
                    "glab auth login --hostname git.example.com",
                ],
            ),
            (
                PrFetchOutcome::CliMissing {
                    cli: "gh",
                    hostname: "github.com".to_string(),
                },
                vec![
                    "install gh",
                    "cli.github.com",
                    "gh auth login --hostname github.com",
                ],
            ),
            (
                PrFetchOutcome::Unauthenticated {
                    cli: "gh",
                    hostname: "github.com".to_string(),
                },
                vec!["gh auth login --hostname github.com"],
            ),
            (
                PrFetchOutcome::ListFailed {
                    message: "rate limit exceeded".to_string(),
                },
                vec!["rate limit exceeded", "retry"],
            ),
            (PrFetchOutcome::NoForgeRemote, vec!["no forge remote"]),
        ];
        for (outcome, expected) in cases {
            let app = prs_app(outcome);
            let content = render_launcher(&app);
            for needle in expected {
                assert!(
                    content.contains(needle),
                    "missing {needle:?} in:\n{content}"
                );
            }
        }
    }

    fn finished(number: u64) -> crate::review::FinishedReview {
        crate::review::FinishedReview {
            branch: format!("redquill/pr/{number}"),
            number,
            title: format!("PR {number}"),
            provider: crate::review::store::ForgeProviderKind::GitHub,
            host: "github.com".to_string(),
            worktree_path: std::path::PathBuf::from(format!("/tmp/wt/{number}")),
            unpublished_count: 0,
        }
    }

    #[test]
    fn prs_tab_finished_reviews_footer_renders_alongside_a_listing() {
        let mut app = prs_app(PrFetchOutcome::Loaded {
            repo_label: "org/repo".to_string(),
            prs: vec![pr(1, "one", "dev", "feature", false)],
        });
        app.launcher_finished_reviews = vec![finished(9), finished(10)];
        let content = render_launcher(&app);
        assert!(content.contains("2 finished review(s)"));
        assert!(content.contains("clean up"));
        // The listed PR still renders too.
        assert!(content.contains("#1"));
    }

    #[test]
    fn prs_tab_finished_reviews_footer_renders_alongside_the_zero_open_empty_state() {
        let mut app = prs_app(PrFetchOutcome::Loaded {
            repo_label: "sdavisde/redquill".to_string(),
            prs: Vec::new(),
        });
        app.launcher_finished_reviews = vec![finished(9)];
        let content = render_launcher(&app);
        assert!(content.contains("No open pull requests on sdavisde/redquill"));
        assert!(content.contains("1 finished review(s)"));
    }

    #[test]
    fn prs_reached_cap_is_true_at_the_cap_and_false_one_below_it() {
        let cap = crate::forge::PR_LIST_CAP;
        assert!(!prs_reached_cap(cap - 1));
        assert!(prs_reached_cap(cap));
        assert!(prs_reached_cap(cap + 1));
    }

    fn prs_of_len(count: usize) -> Vec<PullRequest> {
        (0..count as u64)
            .map(|n| pr(n, "title", "dev", "branch", false))
            .collect()
    }

    #[test]
    fn prs_tab_shows_the_truncation_note_when_the_listing_hits_the_cap() {
        let app = prs_app(PrFetchOutcome::Loaded {
            repo_label: "org/repo".to_string(),
            prs: prs_of_len(crate::forge::PR_LIST_CAP),
        });
        // Taller than `render_launcher`'s fixed 100x24: each PR row is two
        // terminal lines ([`pr_row`]'s primary + meta line), so `PR_LIST_CAP`
        // rows plus the appended truncation note need well over 200 lines of
        // viewport to avoid scrolling off (the cursor sits on row 0, so
        // ratatui doesn't auto-scroll to reveal it otherwise).
        let backend = TestBackend::new(100, 400);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 100, 400);
        terminal.draw(|frame| render(frame, area, &app)).unwrap();
        let content: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(content.contains("showing first 100 open PRs"));
        assert!(content.contains("narrow with /"));
    }

    #[test]
    fn prs_tab_omits_the_truncation_note_when_the_listing_is_below_the_cap() {
        let app = prs_app(PrFetchOutcome::Loaded {
            repo_label: "org/repo".to_string(),
            prs: prs_of_len(crate::forge::PR_LIST_CAP - 1),
        });
        let content = render_launcher(&app);
        assert!(!content.contains("showing first"));
    }
}
