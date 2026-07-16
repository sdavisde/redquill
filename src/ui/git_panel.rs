//! The git panel: a branch header (name plus `↑N↓M` ahead/behind against the
//! upstream), then CHANGES / UNTRACKED / STASHES sections, then a bottom
//! section carrying file/staged/note counts, the tip commit's summary, and
//! the fetch/pull/push keybind hints — all in the same fixed-width slot the
//! passive file sidebar used to occupy.
//!
//! CHANGES preserves the old sidebar's rows exactly: a green `●` staged
//! marker, a colored change-kind letter, and a dimmed-directory / normal
//! basename path split. UNTRACKED lists working-tree files git isn't
//! tracking yet; STASHES lists `git stash list` entries view-only. The panel
//! is passive in this task — the currently selected diff file is highlighted,
//! but there is no independent cursor or focus yet.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};

use crate::git::{BranchStatus, CommitLogEntry, CommitSummary, DiffTarget};

use super::app::App;
use super::app::{Mode, PanelTab, SuspendedView};
use super::diff_view_state::DiffViewState;
use super::stage_ops::{StagedState, build_review};
use super::theme::Theme;
use super::time_format::{now_unix, relative_time};

/// One navigable panel row: either a diff file (index into `app.view.files`)
/// or a stash (index into `app.stashes`). Section-header and branch-title
/// rows are not navigable and never appear here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PanelRow {
    /// A CHANGES or UNTRACKED file entry.
    File(usize),
    /// A STASHES entry (view-only: Enter is a no-op).
    Stash(usize),
}

/// Flattens the panel's three sections into the ordered list of navigable
/// rows, in exactly the order [`render`] lays them out: tracked CHANGES
/// files, then UNTRACKED files, then STASHES. Section headers are skipped, so
/// the panel cursor can never land on one, and `j`/`k` cross section
/// boundaries seamlessly. The single source of truth shared by the cursor
/// motion helpers and the render highlight.
pub(super) fn navigable_rows(app: &App) -> Vec<PanelRow> {
    let mut rows = Vec::new();
    for i in 0..app.view.files.len() {
        if !app.untracked_paths.contains(&app.view.files[i].path) {
            rows.push(PanelRow::File(i));
        }
    }
    for i in 0..app.view.files.len() {
        if app.untracked_paths.contains(&app.view.files[i].path) {
            rows.push(PanelRow::File(i));
        }
    }
    for i in 0..app.stashes.len() {
        rows.push(PanelRow::Stash(i));
    }
    rows
}

/// Steps a panel cursor by one row within a `len`-row navigable list,
/// clamping at both ends. An empty list pins the cursor at 0.
pub(super) fn moved_cursor(cursor: usize, len: usize, down: bool) -> usize {
    if len == 0 {
        return 0;
    }
    if down {
        (cursor + 1).min(len - 1)
    } else {
        cursor.saturating_sub(1)
    }
}

/// Splits `path` into a dimmed directory prefix and a normal-weight
/// basename, e.g. `"src/auth/"` + `"session.rs"`.
fn split_path(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(idx) => (&path[..=idx], &path[idx + 1..]),
        None => ("", path),
    }
}

/// The staged-indicator column: a `●` for a fully-staged file, `±` for a
/// partially-staged one, blank otherwise, so paths stay column-aligned
/// regardless of state.
fn staged_span(state: StagedState, theme: &Theme) -> Span<'static> {
    match state {
        StagedState::Full => Span::styled("\u{25cf} ", Style::default().fg(theme.staged_indicator)),
        StagedState::Partial => {
            Span::styled("\u{00b1} ", Style::default().fg(theme.staged_indicator))
        }
        StagedState::Unstaged => Span::raw("  "),
    }
}

/// A CHANGES row: staged marker, change-kind letter, then the split path.
fn file_line(letter: char, path: &str, state: StagedState, theme: &Theme) -> Line<'static> {
    let (dir, base) = split_path(path);
    Line::from(vec![
        staged_span(state, theme),
        Span::styled(
            format!("{letter} "),
            Style::default()
                .fg(theme.letter_color(letter))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(dir.to_string(), Style::default().fg(theme.dir_prefix)),
        Span::raw(base.to_string()),
    ])
}

/// An UNTRACKED row: no marker or letter, just the split path, indented to
/// sit under the CHANGES rows.
fn untracked_line(path: &str, theme: &Theme) -> Line<'static> {
    let (dir, base) = split_path(path);
    Line::from(vec![
        Span::raw("  "),
        Span::styled(dir.to_string(), Style::default().fg(theme.dir_prefix)),
        Span::raw(base.to_string()),
    ])
}

/// A section header row (`CHANGES`, `UNTRACKED`, `STASHES (2)`).
fn section_header(text: String, theme: &Theme) -> Line<'static> {
    Line::from(Span::styled(
        text,
        Style::default()
            .fg(theme.help_section_header)
            .add_modifier(Modifier::BOLD),
    ))
}

/// The tip-commit summary line for the bottom section: a dimmed abbreviated
/// hash followed by the subject, or a dimmed `no commits yet` when there is
/// none (git-less context, or a repository with no commits). Rendered into a
/// fixed-width row, so ratatui clips a long subject at the panel edge.
fn commit_line(commit: Option<&CommitSummary>, theme: &Theme) -> Line<'static> {
    match commit {
        Some(commit) => Line::from(vec![
            Span::raw(" "),
            Span::styled(
                format!("{} ", commit.short_hash),
                Style::default().fg(theme.dir_prefix),
            ),
            Span::styled(
                commit.subject.clone(),
                Style::default().fg(theme.footer_text),
            ),
        ]),
        None => Line::from(vec![
            Span::raw(" "),
            Span::styled("no commits yet", Style::default().fg(theme.dir_prefix)),
        ]),
    }
}

/// The remote-operation keybind hints for the bottom section: `f fetch`,
/// `p pull`, `P push` (or `P publish` while the branch has no upstream —
/// `push_publishes`, mirroring the footer strip's relabel), with each key
/// emphasized in the help-key color and its label dimmed. These surface the
/// existing bindings (see the panel keymap); the panel doesn't add any new
/// action.
fn remote_keys_line(theme: &Theme, push_publishes: bool) -> Line<'static> {
    let key = |k: &'static str| {
        Span::styled(
            k,
            Style::default()
                .fg(theme.help_key)
                .add_modifier(Modifier::BOLD),
        )
    };
    let label = |l: &'static str| Span::styled(l, Style::default().fg(theme.footer_text));
    Line::from(vec![
        Span::raw(" "),
        key("f"),
        label(" fetch  "),
        key("p"),
        label(" pull  "),
        key("P"),
        label(if push_publishes { " publish" } else { " push" }),
    ])
}

/// The branch header shown as the panel's block title: `git: <name>` plus, if
/// an upstream exists and either count is nonzero, an `↑N↓M` indicator.
/// Detached HEAD carries a short oid as its name; no upstream shows no arrows.
fn branch_title(branch: Option<&BranchStatus>) -> String {
    let Some(branch) = branch else {
        return "git".to_string();
    };
    let mut title = format!("git: {}", branch.name);
    if branch.upstream.is_some()
        && let Some((ahead, behind)) = branch.ahead_behind
    {
        let mut arrows = String::new();
        if ahead > 0 {
            arrows.push_str(&format!("\u{2191}{ahead}"));
        }
        if behind > 0 {
            arrows.push_str(&format!("\u{2193}{behind}"));
        }
        if !arrows.is_empty() {
            title.push(' ');
            title.push_str(&arrows);
        }
    }
    title
}

/// One tab label in the panel's title (`Changes` / `History`, spec 05 Unit
/// 3): underlined and bold when `active`, dimmed otherwise — a Zed-style tab
/// strip rendered as part of the border title rather than a separate row, so
/// it stays "inside the existing panel chrome" per the spec's design notes.
fn tab_span(label: &'static str, active: bool, theme: &Theme) -> Span<'static> {
    if active {
        Span::styled(
            label,
            Style::default()
                .fg(theme.focused_border)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )
    } else {
        Span::styled(label, Style::default().fg(theme.dir_prefix))
    }
}

/// The panel's full block title: the branch header plus the Changes/History
/// tab strip.
fn panel_title(branch: Option<&BranchStatus>, tab: PanelTab, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::raw(branch_title(branch)),
        Span::raw("  "),
        tab_span("Changes", tab == PanelTab::Changes, theme),
        Span::raw(" "),
        tab_span("History", tab == PanelTab::History, theme),
    ])
}

/// A History-tab row: two lines (subject + unpushed marker; dimmed `author ·
/// relative-time · short-sha`), matching Zed's row anatomy (spec 05 Design
/// Considerations). `now` is the caller's wall-clock read (kept a parameter
/// so [`super::time_format::relative_time`] stays pure and independently
/// testable). Long subjects/meta lines are left to ratatui's own line
/// clipping to the panel width, the same way file paths elsewhere in this
/// panel are never manually truncated.
fn history_item(
    entry: &CommitLogEntry,
    unpushed: bool,
    now: i64,
    theme: &Theme,
) -> ListItem<'static> {
    let mut subject_spans = Vec::new();
    if unpushed {
        subject_spans.push(Span::styled(
            "\u{25cf} ",
            Style::default().fg(theme.staged_indicator),
        ));
    }
    subject_spans.push(Span::raw(entry.subject.clone()));
    let meta = format!(
        "  {} \u{b7} {} \u{b7} {}",
        entry.author_name,
        relative_time(now, entry.timestamp),
        entry.short_sha
    );
    ListItem::new(vec![
        Line::from(subject_spans),
        Line::from(Span::styled(meta, Style::default().fg(theme.dir_prefix))),
    ])
}

/// Renders the git panel into `area`.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    // The list fills the panel above a three-row bottom section: counts, the
    // tip-commit summary, and the fetch/pull/push keybind hints.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(area);

    let theme = &app.theme;
    let focused = app.git_panel_focused();
    let tab = app.panel_tab();
    let mut items: Vec<ListItem> = Vec::new();
    // Flat index (into `items`) of the currently selected diff file, used to
    // highlight it when the panel is *not* focused (as the old sidebar did).
    let mut selected_row: Option<usize> = None;
    // `items` index of each navigable row (parallel to `navigable_rows` on the
    // Changes tab, or one-to-one with `app.history` on the History tab), used
    // to highlight the panel cursor when the panel *is* focused.
    let mut nav_item_indices: Vec<usize> = Vec::new();

    match tab {
        PanelTab::Changes => {
            // CHANGES: tracked files (those with a real patch), in display order.
            let tracked: Vec<usize> = (0..app.view.files.len())
                .filter(|&i| !app.untracked_paths.contains(&app.view.files[i].path))
                .collect();
            if !tracked.is_empty() {
                items.push(ListItem::new(section_header("CHANGES".to_string(), theme)));
                for &i in &tracked {
                    let f = &app.view.files[i];
                    let state = app.staged_states.get(&f.path).copied().unwrap_or_default();
                    let mut line = file_line(f.kind.letter(), &f.path, state, theme);
                    if let Some(old) = &f.old_path {
                        let (_, old_base) = split_path(old);
                        line.spans.push(Span::styled(
                            format!(" \u{2190} {old_base}"),
                            Style::default().fg(theme.dir_prefix),
                        ));
                    }
                    if i == app.view.selected_file {
                        selected_row = Some(items.len());
                    }
                    nav_item_indices.push(items.len());
                    items.push(ListItem::new(line));
                }
            }

            // UNTRACKED: files git isn't tracking yet.
            let untracked: Vec<usize> = (0..app.view.files.len())
                .filter(|&i| app.untracked_paths.contains(&app.view.files[i].path))
                .collect();
            if !untracked.is_empty() {
                items.push(ListItem::new(section_header(
                    "UNTRACKED".to_string(),
                    theme,
                )));
                for &i in &untracked {
                    let f = &app.view.files[i];
                    if i == app.view.selected_file {
                        selected_row = Some(items.len());
                    }
                    nav_item_indices.push(items.len());
                    items.push(ListItem::new(untracked_line(&f.path, theme)));
                }
            }

            // STASHES: view-only, `<index> <message>` rows under a counted header.
            if !app.stashes.is_empty() {
                items.push(ListItem::new(section_header(
                    format!("STASHES ({})", app.stashes.len()),
                    theme,
                )));
                for (i, stash) in app.stashes.iter().enumerate() {
                    nav_item_indices.push(items.len());
                    items.push(ListItem::new(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(format!("{i} "), Style::default().fg(theme.dir_prefix)),
                        Span::raw(stash.message.clone()),
                    ])));
                }
            }
        }
        PanelTab::History => {
            if app.history.is_empty() {
                let text = if app.history_loading() {
                    "loading\u{2026}"
                } else {
                    "no commits"
                };
                items.push(ListItem::new(Line::from(Span::styled(
                    text,
                    Style::default().fg(theme.dir_prefix),
                ))));
            } else {
                let ahead = app
                    .branch
                    .as_ref()
                    .and_then(|b| b.ahead_behind)
                    .map(|(ahead, _)| ahead as usize)
                    .unwrap_or(0);
                let now = now_unix();
                for (i, entry) in app.history.iter().enumerate() {
                    nav_item_indices.push(items.len());
                    items.push(history_item(entry, i < ahead, now, theme));
                }
            }
        }
    }

    // When focused, the panel cursor drives the highlight; otherwise the
    // selected diff file does (the old passive behavior, Changes tab only —
    // History has no "currently selected diff file" to fall back to). The
    // panel cursor is kept in range at its mutation points and re-clamped on
    // every refresh (see `App::apply_snapshot`), so no clamp is needed here.
    let highlight = if focused {
        nav_item_indices.get(app.panel_cursor()).copied()
    } else {
        selected_row
    };

    let mut block =
        Block::default()
            .borders(Borders::ALL)
            .title(panel_title(app.branch.as_ref(), tab, theme));
    if focused {
        block = block.border_style(
            Style::default()
                .fg(theme.focused_border)
                .add_modifier(Modifier::BOLD),
        );
    }
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = ListState::default();
    state.select(highlight);
    frame.render_stateful_widget(list, chunks[0], &mut state);

    let notes = app.annotations.len();
    let mut counts_text = format!(" [{} files]", app.view.files.len());
    if !app.staged.is_empty() {
        counts_text.push_str(&format!(" [{} staged]", app.staged.len()));
    }
    if notes > 0 {
        counts_text.push_str(&format!(" [{notes} notes]"));
    }
    let counts = Line::from(Span::styled(
        counts_text,
        Style::default().fg(theme.footer_text),
    ));

    // The three bottom rows share the fixed-height slot below the list: the
    // counts, then the tip-commit summary, then the remote keybind hints.
    let footer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(chunks[1]);
    frame.render_widget(counts, footer[0]);
    frame.render_widget(commit_line(app.last_commit.as_ref(), theme), footer[1]);
    frame.render_widget(remote_keys_line(theme, app.push_publishes()), footer[2]);
}

/// The git panel's focus and navigation handlers, split out of `app.rs`
/// alongside the panel's row model and renderer so all panel logic lives in
/// one module.
impl App {
    /// Whether the git panel currently holds focus (drives border emphasis
    /// and which pane's cursor renders).
    pub fn git_panel_focused(&self) -> bool {
        matches!(self.mode, Mode::Panel { .. })
    }

    /// The git panel's cursor when the panel is focused, else 0. The cursor
    /// lives in [`Mode::Panel`], so an unfocused panel has none to go stale;
    /// [`render`] and [`App::panel_select`] read it only while focused.
    pub(super) fn panel_cursor(&self) -> usize {
        match self.mode {
            Mode::Panel { cursor, .. } => cursor,
            _ => 0,
        }
    }

    /// The git panel's active tab (see [`PanelTab`]) when the panel is
    /// focused, else `self.last_panel_tab` — so callers that need "which tab
    /// would render/act on" (e.g. deciding a footer label) get a sensible
    /// answer even when the panel isn't currently focused.
    pub(super) fn panel_tab(&self) -> PanelTab {
        match self.mode {
            Mode::Panel { tab, .. } => tab,
            _ => self.last_panel_tab,
        }
    }

    /// The active tab's row count: [`navigable_rows`]'s length on Changes,
    /// `self.history`'s length on History. The single tab-aware length every
    /// cursor-clamping call site (`close_switcher`, `close_commit_message`,
    /// `apply_snapshot`'s panel-cursor clamp) shares, so they can't
    /// disagree about which list a stray cursor is being clamped against.
    pub(super) fn panel_row_count(&self) -> usize {
        match self.panel_tab() {
            PanelTab::Changes => navigable_rows(self).len(),
            PanelTab::History => self.history.len(),
        }
    }

    /// Toggles git-panel focus: from Normal/Visual it focuses the panel, on
    /// whichever tab it was last showing (`self.last_panel_tab`), cursor
    /// reset to the top; from the focused panel it returns to Normal. A
    /// no-op while another modal (Compose/List/Staging/Search/Peek) owns the
    /// keyboard, mirroring the other panel toggles.
    pub(super) fn toggle_git_panel(&mut self) {
        match self.mode {
            Mode::Panel { .. } => self.mode = Mode::Normal,
            Mode::Compose
            | Mode::List
            | Mode::Staging
            | Mode::Search
            | Mode::Peek
            | Mode::Switcher
            | Mode::CommitMessage
            | Mode::Finder
            | Mode::ProjectSearch
            | Mode::EndReview { .. } => {}
            Mode::Normal | Mode::Visual { .. } => {
                self.mode = Mode::Panel {
                    cursor: 0,
                    tab: self.last_panel_tab,
                };
                if self.last_panel_tab == PanelTab::History {
                    self.ensure_history_loaded();
                }
                self.panel_follow();
            }
        }
    }

    /// Switches the git panel between its Changes and History tabs (`Tab`,
    /// panel scope, spec 05 Unit 3): resets the cursor to the top (mirrors
    /// focusing the panel), remembers the new tab in `last_panel_tab` so
    /// re-focusing the panel later lands back here, and kicks off the
    /// History tab's first page fetch the first time it's opened. A no-op
    /// unless the panel is focused.
    pub(super) fn toggle_panel_tab(&mut self) {
        let Mode::Panel { cursor, tab } = &mut self.mode else {
            return;
        };
        *tab = match *tab {
            PanelTab::Changes => PanelTab::History,
            PanelTab::History => PanelTab::Changes,
        };
        *cursor = 0;
        let new_tab = *tab;
        self.last_panel_tab = new_tab;
        if new_tab == PanelTab::History {
            self.ensure_history_loaded();
        }
    }

    /// Moves the panel cursor down one navigable row of the active tab,
    /// clamped at the last; on the History tab this also triggers the next
    /// page's prefetch once the cursor nears the end of what's loaded. A
    /// no-op unless the panel is focused.
    pub fn panel_move_down(&mut self) {
        let len = self.panel_row_count();
        if let Mode::Panel { cursor, .. } = &mut self.mode {
            *cursor = moved_cursor(*cursor, len, true);
        }
        if self.panel_tab() == PanelTab::History {
            self.maybe_prefetch_history(self.panel_cursor());
        }
        self.panel_follow();
    }

    /// Moves the panel cursor up one navigable row of the active tab,
    /// clamped at the first. A no-op unless the panel is focused.
    pub fn panel_move_up(&mut self) {
        let len = self.panel_row_count();
        if let Mode::Panel { cursor, .. } = &mut self.mode {
            *cursor = moved_cursor(*cursor, len, false);
        }
        self.panel_follow();
    }

    /// Follows the panel cursor into the diff: if it rests on a file row
    /// whose file isn't already selected, scrolls the multibuffer to that
    /// file's section (expanding it if collapsed) via
    /// [`App::select_file_by_path`]. Stash rows, an empty panel, and an
    /// out-of-range cursor leave the diff untouched. A no-op on the History
    /// tab — its rows have nothing to auto-follow into; opening a commit
    /// needs an explicit `Enter` (see [`App::panel_select`]). Pure in-memory
    /// on the Changes tab — never re-runs git. Always stays in
    /// `Mode::Panel`; the caller decides whether to also move focus (see
    /// [`App::panel_select`]).
    pub(super) fn panel_follow(&mut self) {
        if self.panel_tab() == PanelTab::History {
            return;
        }
        let rows = navigable_rows(self);
        if let Some(PanelRow::File(i)) = rows.get(self.panel_cursor())
            && *i != self.view.selected_file
        {
            let path = self.view.files[*i].path.clone();
            self.select_file_by_path(&path);
        }
    }

    /// Acts on the panel cursor's current row: on the Changes tab, a file
    /// row follows the diff to that file (via [`App::panel_follow`]) and
    /// returns focus to the diff; a stash row (or an out-of-range cursor) is
    /// a no-op, leaving the panel focused. On the History tab, opens the
    /// highlighted commit into the main diff view (see
    /// [`App::open_commit_view`]); an out-of-range cursor (an empty or
    /// still-loading list) is a no-op.
    pub fn panel_select(&mut self) {
        match self.panel_tab() {
            PanelTab::Changes => {
                let rows = navigable_rows(self);
                if let Some(PanelRow::File(_)) = rows.get(self.panel_cursor()) {
                    self.panel_follow();
                    self.mode = Mode::Normal;
                }
            }
            PanelTab::History => {
                if let Some(entry) = self.history.get(self.panel_cursor()) {
                    let sha = entry.sha.clone();
                    self.open_commit_view(sha);
                }
            }
        }
    }

    /// Whether a commit view is currently suspending a prior view (see
    /// [`App::suspended_view`]) — the single predicate
    /// [`super::dispatch_key`]'s Esc handling and any future call site
    /// consult, so "is a commit view open?" can't be answered inconsistently
    /// (mirrors [`App::overlay_active`]'s role for overlays).
    pub(super) fn viewing_commit(&self) -> bool {
        self.suspended_view.is_some()
    }

    /// Opens `sha` (a full commit hash from `self.history`) into the main
    /// diff view: builds its review via [`build_review`] against
    /// `DiffTarget::Commit(sha)`, suspends the prior view state the first
    /// time a commit is opened (further commits opened without returning in
    /// between replace the displayed commit but leave the *original*
    /// suspension untouched, so `Esc` always returns to the true starting
    /// point — see [`App::suspended_view`]'s doc), and returns focus to the
    /// diff (`Mode::Normal`) so navigation/annotation gestures work
    /// immediately. A git/parse failure leaves the current view unchanged
    /// with a footer message; no git backend degrades the same way every
    /// other git-backed gesture does.
    pub(super) fn open_commit_view(&mut self, sha: String) {
        let Some(ops) = self.stage_ops.as_deref() else {
            self.set_status_message("commit view unavailable (no git backend)");
            return;
        };
        let target = DiffTarget::Commit(sha.clone());
        let snapshot = match build_review(ops, &target) {
            Ok(s) => s,
            Err(e) => {
                self.set_status_message(e.to_string());
                return;
            }
        };
        // The commit's header metadata is already in `self.history` (it was
        // clicked from there), so opening a commit needs no extra git call
        // just to populate the header block.
        let header = self.history.iter().find(|c| c.sha == sha).cloned();

        if self.suspended_view.is_none() {
            let new_view = DiffViewState::new(snapshot.files);
            let old_view = std::mem::replace(&mut self.view, new_view);
            self.suspended_view = Some(SuspendedView {
                target: std::mem::replace(&mut self.target, target),
                view: old_view,
                patches: std::mem::replace(&mut self.patches, snapshot.patches),
                staged: std::mem::replace(&mut self.staged, snapshot.staged),
                staged_states: std::mem::replace(&mut self.staged_states, snapshot.staged_states),
            });
        } else {
            self.target = target;
            self.view = DiffViewState::new(snapshot.files);
            self.patches = snapshot.patches;
            self.staged = snapshot.staged;
            self.staged_states = snapshot.staged_states;
        }
        self.active_commit = header;
        self.recompute_untracked();
        // The just-suspended (or just-replaced) content shares this cache by
        // path; clearing it prevents the commit's highlighted spans from
        // cross-contaminating the suspended view's cache entries (and vice
        // versa on return).
        self.highlight_cache.clear();
        self.rebuild_rows();
        self.mode = Mode::Normal;
    }

    /// Restores the view suspended by [`App::open_commit_view`] (`Esc` from
    /// a commit view): the prior target, diff-view state (files, rows,
    /// cursor, scroll, collapse map), patches, and staged state come back
    /// verbatim, `active_commit` clears, and focus returns to
    /// `Mode::Normal`. A no-op if no commit view is open.
    pub(super) fn return_from_commit_view(&mut self) {
        let Some(suspended) = self.suspended_view.take() else {
            return;
        };
        self.target = suspended.target;
        self.view = suspended.view;
        self.patches = suspended.patches;
        self.staged = suspended.staged;
        self.staged_states = suspended.staged_states;
        self.active_commit = None;
        self.recompute_untracked();
        self.highlight_cache.clear();
        self.rebuild_rows();
        self.mode = Mode::Normal;
    }
}

#[cfg(test)]
mod tests {
    use super::super::stage_ops::{StagedFile, StagedState};
    use super::*;
    use crate::diff::FileDiff;
    use crate::git::{RawFilePatch, StashEntry};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::collections::HashMap;

    fn sample_file(path: &str) -> FileDiff {
        let raw = format!(
            "diff --git a/{path} b/{path}\n\
             index 111..222 100644\n\
             --- a/{path}\n\
             +++ b/{path}\n\
             @@ -1,1 +1,1 @@\n\
             -old\n\
             +new\n"
        );
        FileDiff::from_patch(&RawFilePatch {
            path: path.to_string(),
            old_path: None,
            raw,
            is_binary: false,
        })
        .unwrap()
    }

    /// Renders `app`'s panel to a 32x24 `TestBackend` and returns the flat
    /// buffer text.
    fn render_panel(app: &App) -> String {
        let backend = TestBackend::new(32, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 32, 24);
        terminal.draw(|frame| render(frame, area, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    fn branch(name: &str, upstream: Option<&str>, ab: Option<(u32, u32)>) -> BranchStatus {
        BranchStatus {
            name: name.to_string(),
            detached: false,
            upstream: upstream.map(|s| s.to_string()),
            ahead_behind: ab,
        }
    }

    #[test]
    fn header_shows_branch_name_and_ahead_behind_with_upstream() {
        let mut app = App::new(vec![sample_file("session.rs")]);
        app.branch = Some(branch("main", Some("origin/main"), Some((2, 1))));
        let content = render_panel(&app);
        assert!(content.contains("git: main"));
        assert!(content.contains("\u{2191}2\u{2193}1"));
    }

    #[test]
    fn header_detached_head_shows_short_oid_without_arrows() {
        let mut app = App::new(vec![sample_file("session.rs")]);
        app.branch = Some(BranchStatus {
            name: "85d7cc5".to_string(),
            detached: true,
            upstream: None,
            ahead_behind: None,
        });
        let content = render_panel(&app);
        assert!(content.contains("git: 85d7cc5"));
        assert!(!content.contains("\u{2191}"));
        assert!(!content.contains("\u{2193}"));
    }

    #[test]
    fn header_no_upstream_shows_no_arrows() {
        let mut app = App::new(vec![sample_file("session.rs")]);
        app.branch = Some(branch("feature", None, None));
        let content = render_panel(&app);
        assert!(content.contains("git: feature"));
        assert!(!content.contains("\u{2191}"));
        assert!(!content.contains("\u{2193}"));
    }

    #[test]
    fn zero_ahead_behind_shows_no_arrows() {
        let mut app = App::new(vec![sample_file("session.rs")]);
        app.branch = Some(branch("main", Some("origin/main"), Some((0, 0))));
        let content = render_panel(&app);
        assert!(content.contains("git: main"));
        assert!(!content.contains("\u{2191}"));
        assert!(!content.contains("\u{2193}"));
    }

    #[test]
    fn changes_section_preserves_staged_marker_and_letter() {
        let mut app = App::new(vec![sample_file("session.rs")]);
        app.branch = Some(branch("main", Some("origin/main"), Some((2, 1))));
        app.staged = vec![StagedFile {
            path: "session.rs".to_string(),
            letter: 'M',
        }];
        app.staged_states = HashMap::from([("session.rs".to_string(), StagedState::Full)]);
        let content = render_panel(&app);
        assert!(content.contains("CHANGES"));
        assert!(content.contains("\u{25cf}")); // staged dot preserved
        assert!(content.contains("M session.rs")); // change-kind letter
    }

    #[test]
    fn untracked_section_lists_untracked_files() {
        let mut app = App::new(vec![sample_file("session.rs"), sample_file("notes.md")]);
        app.untracked_paths = vec!["notes.md".to_string()];
        let content = render_panel(&app);
        assert!(content.contains("CHANGES"));
        assert!(content.contains("session.rs"));
        assert!(content.contains("UNTRACKED"));
        assert!(content.contains("notes.md"));
    }

    #[test]
    fn stashes_section_shows_counted_header_and_indexed_rows() {
        let mut app = App::new(vec![sample_file("session.rs")]);
        app.stashes = vec![
            StashEntry {
                stash_ref: "stash@{0}".to_string(),
                branch: Some("main".to_string()),
                message: "wip: parser".to_string(),
            },
            StashEntry {
                stash_ref: "stash@{1}".to_string(),
                branch: Some("main".to_string()),
                message: "spike: tabs".to_string(),
            },
        ];
        let content = render_panel(&app);
        assert!(content.contains("STASHES (2)"));
        assert!(content.contains("0 wip: parser"));
        assert!(content.contains("1 spike: tabs"));
    }

    #[test]
    fn footer_shows_file_and_staged_counts() {
        let mut app = App::new(vec![sample_file("session.rs")]);
        app.staged = vec![StagedFile {
            path: "session.rs".to_string(),
            letter: 'M',
        }];
        let content = render_panel(&app);
        assert!(content.contains("[1 files]"));
        assert!(content.contains("[1 staged]"));
    }

    // -- Bottom section: last commit + remote keybind hints ----------------

    #[test]
    fn bottom_section_shows_last_commit_hash_and_subject() {
        let mut app = App::new(vec![sample_file("session.rs")]);
        app.last_commit = Some(CommitSummary {
            short_hash: "a1b2c3d".to_string(),
            subject: "fix: parser".to_string(),
        });
        let content = render_panel(&app);
        assert!(content.contains("a1b2c3d"));
        assert!(content.contains("fix: parser"));
    }

    #[test]
    fn bottom_section_shows_no_commits_yet_without_a_last_commit() {
        let app = App::new(vec![sample_file("session.rs")]);
        let content = render_panel(&app);
        assert!(content.contains("no commits yet"));
    }

    #[test]
    fn bottom_section_shows_fetch_pull_push_keybind_hints() {
        let mut app = App::new(vec![sample_file("session.rs")]);
        app.branch = Some(branch("main", Some("origin/main"), Some((0, 0))));
        let content = render_panel(&app);
        assert!(content.contains("f fetch"));
        assert!(content.contains("p pull"));
        assert!(content.contains("P push"));
        assert!(!content.contains("P publish"));
    }

    /// On a branch with no upstream, `P` publishes (see
    /// `App::remote_push_op`), so the keybind line must say so.
    #[test]
    fn bottom_section_relabels_push_to_publish_on_an_unpublished_branch() {
        let mut app = App::new(vec![sample_file("session.rs")]);
        app.branch = Some(branch("feature", None, None));
        let content = render_panel(&app);
        assert!(content.contains("P publish"));
        assert!(!content.contains("P push"));
    }

    // -- Empty states ------------------------------------------------------

    #[test]
    fn empty_stashes_hide_the_stashes_section() {
        let app = App::new(vec![sample_file("session.rs")]);
        let content = render_panel(&app);
        assert!(!content.contains("STASHES"));
    }

    #[test]
    fn no_untracked_files_hide_the_untracked_section() {
        let app = App::new(vec![sample_file("session.rs")]);
        let content = render_panel(&app);
        assert!(!content.contains("UNTRACKED"));
        // The tracked file still appears under CHANGES.
        assert!(content.contains("CHANGES"));
        assert!(content.contains("session.rs"));
    }

    // -- Cursor model (flattening + clamping) ------------------------------

    /// An app with two tracked files, one untracked file, and two stashes —
    /// the fixture the flattening/clamping tests share.
    fn mixed_app() -> App {
        let mut app = App::new(vec![
            sample_file("a.rs"),
            sample_file("b.rs"),
            sample_file("notes.md"),
        ]);
        app.untracked_paths = vec!["notes.md".to_string()];
        app.stashes = vec![
            StashEntry {
                stash_ref: "stash@{0}".to_string(),
                branch: Some("main".to_string()),
                message: "wip: parser".to_string(),
            },
            StashEntry {
                stash_ref: "stash@{1}".to_string(),
                branch: Some("main".to_string()),
                message: "spike: tabs".to_string(),
            },
        ];
        app
    }

    /// The three sections flatten into files-then-stashes, in render order,
    /// with no header rows present (headers are not a `PanelRow` variant).
    #[test]
    fn navigable_rows_flatten_sections_in_render_order() {
        let app = mixed_app();
        let rows = navigable_rows(&app);
        assert_eq!(
            rows,
            vec![
                PanelRow::File(0), // a.rs   (CHANGES)
                PanelRow::File(1), // b.rs   (CHANGES)
                PanelRow::File(2), // notes.md (UNTRACKED)
                PanelRow::Stash(0),
                PanelRow::Stash(1),
            ]
        );
    }

    #[test]
    fn moved_cursor_clamps_at_the_top() {
        // Already at 0, moving up stays at 0.
        assert_eq!(moved_cursor(0, 5, false), 0);
    }

    #[test]
    fn moved_cursor_clamps_at_the_bottom() {
        // At the last row (len 5 -> index 4), moving down stays at 4.
        assert_eq!(moved_cursor(4, 5, true), 4);
    }

    #[test]
    fn moved_cursor_crosses_section_boundaries() {
        // With mixed_app's 5 navigable rows, stepping down from the last
        // CHANGES row (index 1) lands on the UNTRACKED file (index 2), and
        // from there onto the first STASH (index 3) — the flat list makes
        // section boundaries invisible to motion.
        let app = mixed_app();
        let rows = navigable_rows(&app);
        let len = rows.len();
        let after_changes = moved_cursor(1, len, true);
        assert_eq!(after_changes, 2);
        assert_eq!(rows[after_changes], PanelRow::File(2));
        let into_stashes = moved_cursor(2, len, true);
        assert_eq!(into_stashes, 3);
        assert_eq!(rows[into_stashes], PanelRow::Stash(0));
    }

    #[test]
    fn moved_cursor_on_empty_list_stays_at_zero() {
        // No files, no stashes -> nothing navigable; both directions pin 0.
        let app = App::new(vec![]);
        let len = navigable_rows(&app).len();
        assert_eq!(len, 0);
        assert_eq!(moved_cursor(0, len, true), 0);
        assert_eq!(moved_cursor(0, len, false), 0);
    }

    #[test]
    fn navigable_rows_with_empty_stash_section_omit_stash_rows() {
        let mut app = mixed_app();
        app.stashes.clear();
        let rows = navigable_rows(&app);
        assert_eq!(
            rows,
            vec![PanelRow::File(0), PanelRow::File(1), PanelRow::File(2)]
        );
        assert!(rows.iter().all(|r| matches!(r, PanelRow::File(_))));
    }

    // -- Auto-follow (Task 2) -----------------------------------------------

    /// Moving the panel cursor onto a file row scrolls the diff to that
    /// file without leaving `Mode::Panel` — follow, don't focus-jump.
    #[test]
    fn panel_cursor_motion_follows_file_rows() {
        let mut app = mixed_app();
        app.mode = Mode::Panel {
            cursor: 0,
            tab: PanelTab::Changes,
        }; // a.rs, already selected (selected_file starts at 0)
        app.panel_move_down(); // -> b.rs
        assert_eq!(app.panel_cursor(), 1);
        assert_eq!(app.view.selected_file, 1);
        assert!(matches!(app.mode, Mode::Panel { .. }));
    }

    /// Moving onto a stash row leaves the diff's file selection exactly
    /// where it last followed to — stash rows have nothing to follow to.
    #[test]
    fn panel_cursor_on_stash_row_leaves_diff_selection() {
        let mut app = mixed_app();
        app.mode = Mode::Panel {
            cursor: 1,
            tab: PanelTab::Changes,
        };
        app.panel_follow(); // -> b.rs selected
        assert_eq!(app.view.selected_file, 1);
        app.panel_move_down(); // -> notes.md
        assert_eq!(app.view.selected_file, 2);
        app.panel_move_down(); // -> stash 0, nothing to follow to
        assert_eq!(app.panel_cursor(), 3);
        assert_eq!(app.view.selected_file, 2); // unchanged from the last file row
    }

    /// An empty panel (no files, no stashes) is a no-op, not a panic.
    #[test]
    fn panel_follow_on_empty_panel_is_noop() {
        let mut app = App::new(vec![]);
        app.mode = Mode::Panel {
            cursor: 0,
            tab: PanelTab::Changes,
        };
        app.panel_follow();
        assert_eq!(app.panel_cursor(), 0);
        assert_eq!(app.view.selected_file, 0);
    }

    /// Focusing the panel (`` ` ``) resets the cursor to the top row and
    /// follows it, so the diff snaps back to the first file even if it had
    /// scrolled elsewhere while the panel was unfocused.
    #[test]
    fn focusing_panel_follows_to_first_file() {
        let mut app = mixed_app();
        assert!(app.select_file_by_path("b.rs"));
        assert_eq!(app.view.selected_file, 1);
        app.toggle_git_panel();
        assert!(matches!(app.mode, Mode::Panel { .. }));
        assert_eq!(app.panel_cursor(), 0);
        assert_eq!(app.view.selected_file, 0); // followed back to a.rs
    }

    /// Following onto a collapsed file's row expands it — a collapsed
    /// section has nothing to follow to otherwise.
    #[test]
    fn panel_follow_expands_collapsed_target() {
        let mut app = mixed_app();
        app.view.set_collapsed("b.rs", true);
        app.rebuild_rows();
        assert!(app.view.is_collapsed("b.rs"));
        app.mode = Mode::Panel {
            cursor: 0,
            tab: PanelTab::Changes,
        };
        app.panel_move_down(); // onto b.rs's row
        assert_eq!(app.panel_cursor(), 1);
        assert_eq!(app.view.selected_file, 1);
        assert!(!app.view.is_collapsed("b.rs"));
    }

    /// Enter on a file row follows to it and returns focus to the diff.
    #[test]
    fn enter_on_file_row_returns_focus_with_file_selected() {
        let mut app = mixed_app();
        app.mode = Mode::Panel {
            cursor: 1,
            tab: PanelTab::Changes,
        }; // b.rs
        app.panel_select();
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.view.selected_file, 1);
    }

    /// Enter on a stash row stays put — no file to focus on.
    #[test]
    fn enter_on_stash_row_is_noop_keeping_panel_focus() {
        let mut app = mixed_app();
        app.mode = Mode::Panel {
            cursor: 3,
            tab: PanelTab::Changes,
        }; // stash 0
        app.panel_select();
        assert!(matches!(app.mode, Mode::Panel { .. }));
    }

    // -- History tab (spec 05 Unit 3) ---------------------------------------
    //
    // These are UI-state/TestBackend-buffer proofs, not real-terminal
    // screenshots (this sandbox has no controlling TTY — see
    // `05-task-03-proofs.md`'s TTY-deferred section); they exercise the same
    // rendering code path a real terminal would.

    use super::super::background::TaskId;
    use super::super::history::InFlightHistory;
    use crate::git::CommitLogEntry;

    fn commit(sha: &str, subject: &str, author: &str, ts: i64) -> CommitLogEntry {
        CommitLogEntry {
            sha: sha.to_string(),
            short_sha: sha[..sha.len().min(7)].to_string(),
            subject: subject.to_string(),
            author_name: author.to_string(),
            timestamp: ts,
        }
    }

    fn app_on_history_tab() -> App {
        let mut app = App::new(vec![sample_file("session.rs")]);
        app.mode = Mode::Panel {
            cursor: 0,
            tab: PanelTab::History,
        };
        app
    }

    #[test]
    fn history_tab_shows_a_loading_placeholder_before_the_first_page_lands() {
        let mut app = app_on_history_tab();
        // Simulate a fetch in flight without actually spawning a thread.
        app.history_in_flight = Some(InFlightHistory {
            id: TaskId(0),
            generation: app.history_generation,
        });
        let content = render_panel(&app);
        assert!(content.contains("loading"));
    }

    #[test]
    fn history_tab_shows_no_commits_when_nothing_is_in_flight_and_history_is_empty() {
        let app = app_on_history_tab();
        let content = render_panel(&app);
        assert!(content.contains("no commits"));
        assert!(!content.contains("loading"));
    }

    #[test]
    fn history_tab_renders_commit_rows_with_subject_meta_and_unpushed_marker() {
        let mut app = app_on_history_tab();
        app.branch = Some(branch("main", Some("origin/main"), Some((1, 0))));
        app.history = vec![
            commit("abc1234full", "feat: new thing", "Jane Dev", 1_700_000_000),
            commit("def5678full", "fix: old bug", "Jane Dev", 1_600_000_000),
        ];
        let content = render_panel(&app);
        assert!(content.contains("feat: new thing"));
        assert!(content.contains("fix: old bug"));
        // The unpushed marker (●) decorates only the first `ahead` (1) row.
        assert!(content.contains("\u{25cf}"));
        assert!(content.contains("Jane Dev"));
        assert!(content.contains("abc1234"));
    }

    #[test]
    fn panel_title_shows_both_tab_labels_regardless_of_which_is_active() {
        let mut app = App::new(vec![sample_file("session.rs")]);
        app.mode = Mode::Panel {
            cursor: 0,
            tab: PanelTab::Changes,
        };
        let content = render_panel(&app);
        assert!(content.contains("Changes"));
        assert!(content.contains("History"));
    }

    #[test]
    fn moving_the_cursor_down_the_history_tab_stops_at_the_last_loaded_row() {
        let mut app = app_on_history_tab();
        app.history = vec![
            commit("a", "one", "Dev", 1_700_000_000),
            commit("b", "two", "Dev", 1_700_000_000),
        ];
        app.history_exhausted = true;
        app.panel_move_down();
        app.panel_move_down();
        app.panel_move_down(); // clamps at the last row
        assert_eq!(app.panel_cursor(), 1);
    }

    /// `Tab` switches tabs, resets the cursor, and remembers the tab for the
    /// next time the panel is focused.
    #[test]
    fn toggle_panel_tab_switches_and_resets_cursor() {
        let mut app = mixed_app();
        app.mode = Mode::Panel {
            cursor: 2,
            tab: PanelTab::Changes,
        };
        app.toggle_panel_tab();
        assert_eq!(app.panel_tab(), PanelTab::History);
        assert_eq!(app.panel_cursor(), 0);
        assert_eq!(app.last_panel_tab, PanelTab::History);

        app.toggle_panel_tab();
        assert_eq!(app.panel_tab(), PanelTab::Changes);
        assert_eq!(app.panel_cursor(), 0);
    }

    /// Re-focusing the panel lands on whichever tab was last active, not
    /// always Changes.
    #[test]
    fn refocusing_the_panel_remembers_the_last_active_tab() {
        let mut app = mixed_app();
        app.mode = Mode::Panel {
            cursor: 0,
            tab: PanelTab::Changes,
        };
        app.toggle_panel_tab(); // -> History
        app.toggle_git_panel(); // unfocus
        assert_eq!(app.mode, Mode::Normal);
        app.toggle_git_panel(); // refocus
        assert_eq!(app.panel_tab(), PanelTab::History);
    }
}
