//! The git panel: a branch header (name plus `↑N↓M` ahead/behind against the
//! upstream), then the changed-file tree, then a bottom-pinned STASHES
//! section, then a bottom section carrying file/staged/note counts, the tip
//! commit's summary, and the fetch/pull/push keybind hints — all in a
//! fixed-width slot alongside the diff view.
//!
//! The Changes tab renders one unified file tree (see [`super::file_tree`])
//! that mirrors an editor's file explorer: directories group their files and
//! are collapsible, single-child directory chains fold into one row, and each
//! file carries a Nerd Font type glyph (see [`super::icons`]) tinted by — and
//! trailed with the letter of — its change kind. Untracked files sit in the
//! same tree, marked with `?`. STASHES are pinned to their own region just
//! above the footer (`git stash list` entries, view-only) so the file tree
//! keeps the room above it even when there are only a few files. The panel
//! cursor navigates the tree's directory and file rows; stash rows are passive.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

use crate::diff::FileChangeKind;
use crate::git::{BranchStatus, CommitLogEntry, CommitSummary, DiffTarget};
use crate::review::ReviewStatus;

use super::app::App;
use super::app::{Mode, PanelTab, SuspendedView};
use super::diff_view_state::DiffViewState;
use super::file_tree::{TreeFile, TreeNode, TreeRow, flatten};
use super::icons;
use super::keymap::{Action, Keymap, Scope};
use super::stage_ops::{StagedState, build_review};
use super::theme::Theme;
use super::time_format::{now_unix, relative_time};

/// One navigable panel row: a tree directory (identified by its full-path
/// key, collapse toggles against it) or a diff file (index into
/// `app.view.files`). Section-header, stash, and branch-title rows are not
/// navigable and never appear here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PanelRow {
    /// A collapsible directory row; the `String` is its tree key.
    Dir(String),
    /// A changed-file entry.
    File(usize),
}

/// The changed-file tree for the Changes tab, flattened against the panel's
/// current collapse state. The single source of truth shared by the renderer
/// and the cursor model, so the visible rows and the navigable rows can never
/// disagree.
pub(super) fn panel_tree_rows(app: &App) -> Vec<TreeRow> {
    let files: Vec<TreeFile> = app
        .view
        .files
        .iter()
        .enumerate()
        .map(|(i, f)| TreeFile {
            file_index: i,
            path: f.path.clone(),
            kind: f.kind,
            untracked: app.untracked_paths.contains(&f.path),
        })
        .collect();
    flatten(&files, &app.panel_collapsed_dirs)
}

/// Flattens the file tree into the ordered list of navigable rows, in exactly
/// the order [`render`] lays them out. Directory and file rows are both
/// navigable (a directory row toggles its own collapse on `Enter`); stash rows
/// are not, so the panel cursor can never land on one.
pub(super) fn navigable_rows(app: &App) -> Vec<PanelRow> {
    panel_tree_rows(app)
        .into_iter()
        .map(|r| match r.node {
            TreeNode::Dir { key, .. } => PanelRow::Dir(key),
            TreeNode::File { file_index, .. } => PanelRow::File(file_index),
        })
        .collect()
}

/// Steps a panel cursor by one row within a `len`-row navigable list,
/// clamping at both ends. An empty list pins the cursor at 0. Delegates to
/// the shared motion layer's linear-cursor helper (see `super::motion`) so
/// the git panel's step math is the same arithmetic every other
/// motion-supporting list uses.
pub(super) fn moved_cursor(cursor: usize, len: usize, down: bool) -> usize {
    super::motion::step(cursor, len, 1, down)
}

/// Splits `path` into a dimmed directory prefix and a normal-weight
/// basename, e.g. `"src/auth/"` + `"session.rs"`.
fn split_path(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(idx) => (&path[..=idx], &path[idx + 1..]),
        None => ("", path),
    }
}

/// Box-drawing tree guides for each flattened row: a vertical bar in every
/// ancestor column that still has rows below it, then a `├`/`└` connector for
/// the row itself (root rows included). Derived from the rows' depths alone,
/// so the guides stay in lockstep with whatever [`panel_tree_rows`] laid out.
/// Each guide cell is two columns wide — the same width the old blank
/// indentation used, so the tree now *fills* that left space with structure
/// rather than wasting it.
fn tree_guides(rows: &[TreeRow]) -> Vec<String> {
    // `is_last[r]`: row `r` is the last among its siblings at its own depth —
    // i.e. the next row at depth <= its own dedents rather than continuing.
    let mut is_last = vec![true; rows.len()];
    for r in 0..rows.len() {
        let d = rows[r].depth;
        for row in rows.iter().skip(r + 1) {
            if row.depth < d {
                break;
            }
            if row.depth == d {
                is_last[r] = false;
                break;
            }
        }
    }
    let mut guides = vec![String::new(); rows.len()];
    // The ancestor row index at each depth, rebuilt as we walk the preorder.
    let mut stack: Vec<usize> = Vec::new();
    for r in 0..rows.len() {
        let d = rows[r].depth;
        stack.truncate(d);
        let mut g = String::new();
        for &ancestor in &stack {
            g.push_str(if is_last[ancestor] { "  " } else { "\u{2502} " });
        }
        g.push_str(if is_last[r] { "\u{2514} " } else { "\u{251c} " });
        guides[r] = g;
        stack.push(r);
    }
    guides
}

/// The right-aligned status cluster for a file row: an optional staged/review
/// glyph, then the change-kind letter. The staged and review markers are
/// mutually exclusive (a review session never stages), so at most one glyph
/// precedes the letter. Moving these to the right frees the entire left
/// column that the two blank marker slots used to reserve on every row.
/// Returns the spans and their total display width. `Accepted` reuses the
/// staged ● — see theme.rs's staged_indicator rationale.
fn status_cluster(
    letter: char,
    color: Color,
    state: StagedState,
    review: ReviewStatus,
    theme: &Theme,
) -> (Vec<Span<'static>>, usize) {
    let marker: Option<(&'static str, Color)> = match (state, review) {
        (StagedState::Full, _) => Some(("\u{25cf}", theme.staged_indicator)),
        (StagedState::Partial, _) => Some(("\u{00b1}", theme.staged_indicator)),
        (_, ReviewStatus::Accepted) => Some(("\u{25cf}", theme.staged_indicator)),
        (_, ReviewStatus::Deferred) => Some(("~", theme.review_deferred_marker)),
        (_, ReviewStatus::ChangedSinceAccepted) => Some(("!", theme.review_changed_marker)),
        _ => None,
    };
    let mut spans = Vec::new();
    let mut width = 0;
    if let Some((glyph, marker_color)) = marker {
        spans.push(Span::styled(
            glyph.to_string(),
            Style::default().fg(marker_color),
        ));
        spans.push(Span::raw(" "));
        width += 2;
    }
    spans.push(Span::styled(
        letter.to_string(),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ));
    width += 1;
    (spans, width)
}

/// A directory row: its tree guide, a fold chevron, a folder glyph (open or
/// closed), then the (possibly compressed) directory name in bold.
fn dir_line(guide: &str, name: &str, collapsed: bool, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(guide.to_string(), Style::default().fg(theme.dir_prefix)),
        Span::styled(
            format!(
                "{} {} ",
                icons::chevron(collapsed),
                icons::dir_icon(collapsed)
            ),
            Style::default().fg(theme.dir_prefix),
        ),
        Span::styled(
            name.to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ])
}

/// A file row: its tree guide, a change-kind-tinted type glyph, the basename,
/// an optional `← old` rename tail, then the right-aligned status cluster
/// (staged/review marker + change-kind letter). `content_width` is the list's
/// inner width in cells; when the row is too wide for a right-aligned cluster,
/// the cluster still trails after a single space so the status never vanishes.
#[allow(clippy::too_many_arguments)]
fn file_line(
    guide: &str,
    name: &str,
    kind: FileChangeKind,
    untracked: bool,
    state: StagedState,
    review: ReviewStatus,
    old_path: Option<&str>,
    theme: &Theme,
    content_width: usize,
) -> Line<'static> {
    let letter = if untracked { '?' } else { kind.letter() };
    let color = theme.letter_color(letter);
    let mut spans = vec![
        Span::styled(guide.to_string(), Style::default().fg(theme.dir_prefix)),
        Span::styled(
            format!("{} ", icons::file_icon(name)),
            Style::default().fg(color),
        ),
        Span::raw(name.to_string()),
    ];
    if let Some(old) = old_path {
        let (_, old_base) = split_path(old);
        spans.push(Span::styled(
            format!(" \u{2190} {old_base}"),
            Style::default().fg(theme.dir_prefix),
        ));
    }
    let (cluster, cluster_w) = status_cluster(letter, color, state, review, theme);
    let mut line = Line::from(spans);
    let used = line.width() as usize;
    let pad = content_width.saturating_sub(used + cluster_w + 1).max(1);
    line.spans.push(Span::raw(" ".repeat(pad)));
    line.spans.extend(cluster);
    line
}

/// A section header row (`STASHES (2)`).
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

/// The remote-operation keybind hints for the bottom section: `<key> fetch`,
/// `<key> pull`, `<key> push` (or `publish` while the branch has no
/// upstream — `push_publishes`, mirroring the footer strip's relabel), with
/// each key emphasized in the help-key color and its label dimmed. Keys are
/// resolved from `keymap` (panel scope) rather than hardcoded, so a
/// `[keys.panel]` remap or unbind of `remote-fetch`/`remote-pull`/
/// `remote-push` can't leave this line showing a stale key — an unbound
/// action's segment is simply omitted, the same graceful-degradation
/// convention `super::welcome`'s hints use.
fn remote_keys_line(theme: &Theme, keymap: &Keymap, push_publishes: bool) -> Line<'static> {
    let key = |k: String| {
        Span::styled(
            k,
            Style::default()
                .fg(theme.help_key)
                .add_modifier(Modifier::BOLD),
        )
    };
    let label = |l: &'static str| Span::styled(l, Style::default().fg(theme.footer_text));
    let mut spans = vec![Span::raw(" ")];
    for (action, text) in [
        (Action::RemoteFetch, " fetch  "),
        (Action::RemotePull, " pull  "),
        (
            Action::RemotePush,
            if push_publishes { " publish" } else { " push" },
        ),
    ] {
        if let Some(k) = keymap.label_for(Scope::Panel, action) {
            spans.push(key(k));
            spans.push(label(text));
        }
    }
    Line::from(spans)
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

/// One tab label in the panel's title (`Changes` / `History`): underlined
/// and bold when `active`, dimmed otherwise — a Zed-style tab strip
/// rendered as part of the border title rather than a separate row, so it
/// stays inside the existing panel chrome.
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

/// A History-tab row: two lines echoing the Changes tab's visual language. A
/// `git log --graph`-style rail runs down the left — a commit dot (`●`) on the
/// subject line, bright when the commit is unpushed and dim otherwise, and a
/// connector bar (`│`) on the meta line flowing to the next commit, mirroring
/// the file tree's box-drawing guides. The short sha is right-aligned into the
/// panel's inner edge (the same right gutter the file rows use for their
/// change letter), and the dimmed meta line carries `author · relative-time`.
/// `now` is the caller's wall-clock read (kept a parameter so
/// [`super::time_format::relative_time`] stays pure and independently
/// testable); `content_width` is the list's inner width in cells. Long
/// subjects are left to ratatui's own clipping.
/// Renders one commit-log row: subject with a leading unpushed/pushed dot
/// and a right-aligned short sha on the first line, author + relative time
/// on the second. Shared beyond the git panel's own History tab by the
/// Review launcher's Commits tab (see `review_launcher_modal`), so both
/// surfaces render commits identically.
pub(super) fn history_item(
    entry: &CommitLogEntry,
    unpushed: bool,
    now: i64,
    theme: &Theme,
    content_width: usize,
) -> ListItem<'static> {
    let dot_color = if unpushed {
        theme.staged_indicator
    } else {
        theme.dir_prefix
    };
    let mut subject_line = Line::from(vec![
        Span::styled("\u{25cf} ", Style::default().fg(dot_color)),
        Span::raw(entry.subject.clone()),
    ]);
    // Right-align the short sha, leaving one trailing cell of margin.
    let sha_w = entry.short_sha.chars().count();
    let used = subject_line.width() as usize;
    let pad = content_width.saturating_sub(used + sha_w + 1).max(1);
    subject_line.spans.push(Span::raw(" ".repeat(pad)));
    subject_line.spans.push(Span::styled(
        entry.short_sha.clone(),
        Style::default().fg(theme.dir_prefix),
    ));

    let meta = format!(
        "\u{2502} {} \u{b7} {}",
        entry.author_name,
        relative_time(now, entry.timestamp),
    );
    ListItem::new(vec![
        subject_line,
        Line::from(Span::styled(meta, Style::default().fg(theme.dir_prefix))),
    ])
}

/// The passive STASHES region rendered just above the footer: a counted
/// header, then one `<index> <message>` row per stash. Not navigable — the
/// panel cursor never lands here. Returns an empty `Vec` (and so occupies no
/// height) when there are no stashes.
fn stash_lines(app: &App, theme: &Theme) -> Vec<Line<'static>> {
    if app.stashes.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![section_header(
        format!("STASHES ({})", app.stashes.len()),
        theme,
    )];
    for (i, stash) in app.stashes.iter().enumerate() {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{i} "), Style::default().fg(theme.dir_prefix)),
            Span::raw(stash.message.clone()),
        ]));
    }
    lines
}

/// Renders the git panel into `area`.
pub fn render(frame: &mut Frame, area: Rect, app: &App, keymap: &Keymap) {
    let theme = &app.theme;
    let focused = app.git_panel_focused();
    let tab = app.panel_tab();

    // The STASHES region is pinned just above the footer on the Changes tab,
    // capped at half the panel so a long stash list can never crowd out the
    // file tree. The tree list fills whatever remains above it.
    let stashes = if tab == PanelTab::Changes {
        stash_lines(app, theme)
    } else {
        Vec::new()
    };
    let stash_h = (stashes.len() as u16).min(area.height / 2);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(stash_h),
            Constraint::Length(3),
        ])
        .split(area);

    // The list's inner width (panel minus its left/right borders), used to
    // right-align each file row's change-kind letter.
    let content_width = chunks[0].width.saturating_sub(2) as usize;

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
            let rows = panel_tree_rows(app);
            let guides = tree_guides(&rows);
            for (idx, row) in rows.iter().enumerate() {
                let guide = guides[idx].as_str();
                match &row.node {
                    TreeNode::Dir { key, name } => {
                        let collapsed = app.panel_collapsed_dirs.contains(key);
                        nav_item_indices.push(items.len());
                        items.push(ListItem::new(dir_line(guide, name, collapsed, theme)));
                    }
                    TreeNode::File {
                        file_index,
                        kind,
                        untracked,
                        name,
                    } => {
                        let f = &app.view.files[*file_index];
                        let state = app.staged_states.get(&f.path).copied().unwrap_or_default();
                        let review = app.review_status(&f.path);
                        let line = file_line(
                            guide,
                            name,
                            *kind,
                            *untracked,
                            state,
                            review,
                            f.old_path.as_deref(),
                            theme,
                            content_width,
                        );
                        if *file_index == app.view.selected_file {
                            selected_row = Some(items.len());
                        }
                        nav_item_indices.push(items.len());
                        items.push(ListItem::new(line));
                    }
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
                    items.push(history_item(entry, i < ahead, now, theme, content_width));
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

    if stash_h > 0 {
        frame.render_widget(Paragraph::new(stashes), chunks[1]);
    }

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
        .split(chunks[2]);
    frame.render_widget(counts, footer[0]);
    frame.render_widget(commit_line(app.last_commit.as_ref(), theme), footer[1]);
    frame.render_widget(
        remote_keys_line(theme, keymap, app.push_publishes()),
        footer[2],
    );
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
            | Mode::ReviewLauncher { .. }
            | Mode::CommitMessage
            | Mode::Finder
            | Mode::ProjectSearch
            | Mode::EndReview { .. }
            | Mode::ConfirmRemoteOp { .. } => {}
            Mode::Normal | Mode::Visual { .. } => {
                self.mode = Mode::Panel {
                    cursor: 0,
                    tab: self.last_panel_tab,
                };
                self.motion_count = None;
                if self.last_panel_tab == PanelTab::History {
                    self.ensure_history_loaded();
                }
                self.panel_follow();
            }
        }
    }

    /// Switches the git panel between its Changes and History tabs (`Tab`,
    /// panel scope): resets the cursor to the top (mirrors
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

    /// The panel's page-size proxy for half/full-page motions: the panel
    /// doesn't track its own render height (its list widget scrolls itself
    /// via `ListState`), so this approximates it with the diff pane's own
    /// tracked viewport height — both panes are visible in the same
    /// terminal, so the order of magnitude is right even though the exact
    /// row count differs.
    fn panel_viewport_proxy(&self) -> usize {
        self.view.viewport_height()
    }

    /// Steps the panel cursor by `step` rows (clamped against the active
    /// tab's row count) and re-runs the same History-tab prefetch and
    /// diff-follow bookkeeping [`App::panel_move_down`]/[`App::panel_move_up`]
    /// do, so every layer-driven move (half/full-page, jumps) behaves
    /// identically to a plain `j`/`k` step. A no-op unless the panel is
    /// focused.
    fn panel_step(&mut self, step: usize, down: bool) {
        let len = self.panel_row_count();
        if let Mode::Panel { cursor, .. } = &mut self.mode {
            *cursor = super::motion::step(*cursor, len, step, down);
        }
        if self.panel_tab() == PanelTab::History {
            self.maybe_prefetch_history(self.panel_cursor());
        }
        self.panel_follow();
    }

    /// Jumps the panel cursor to `target` (clamped against the active tab's
    /// row count), with the same prefetch/follow bookkeeping as
    /// [`App::panel_step`]. A no-op unless the panel is focused.
    fn panel_jump(&mut self, target: usize) {
        let len = self.panel_row_count();
        if let Mode::Panel { cursor, .. } = &mut self.mode {
            *cursor = target.min(len.saturating_sub(1));
        }
        if self.panel_tab() == PanelTab::History {
            self.maybe_prefetch_history(self.panel_cursor());
        }
        self.panel_follow();
    }

    /// Moves the panel cursor down half a viewport (`Ctrl-d`, panel scope;
    /// shared motion set — see `super::motion`).
    pub fn panel_half_page_down(&mut self) {
        let step = super::motion::half_page(self.panel_viewport_proxy());
        self.panel_step(step, true);
    }

    /// Moves the panel cursor up half a viewport (`Ctrl-u`, panel scope).
    pub fn panel_half_page_up(&mut self) {
        let step = super::motion::half_page(self.panel_viewport_proxy());
        self.panel_step(step, false);
    }

    /// Moves the panel cursor down a full viewport (`Ctrl-f`, panel scope).
    pub fn panel_full_page_down(&mut self) {
        let step = super::motion::full_page(self.panel_viewport_proxy());
        self.panel_step(step, true);
    }

    /// Moves the panel cursor up a full viewport (`Ctrl-b`, panel scope).
    pub fn panel_full_page_up(&mut self) {
        let step = super::motion::full_page(self.panel_viewport_proxy());
        self.panel_step(step, false);
    }

    /// Jumps the panel cursor to the first navigable row (`g`/`Home`, panel
    /// scope).
    pub fn panel_jump_to_top(&mut self) {
        self.panel_jump(super::motion::jump_top());
    }

    /// Jumps the panel cursor to the last navigable row (`G`/`End`, panel
    /// scope).
    pub fn panel_jump_to_bottom(&mut self) {
        let len = self.panel_row_count();
        self.panel_jump(super::motion::jump_bottom(len));
    }

    /// Toggles the collapse state of the directory keyed by `key` in the
    /// panel's file tree. Persists in `self.panel_collapsed_dirs`, which
    /// survives refreshes so a background status refresh can't silently
    /// re-expand a folder the user closed.
    pub(super) fn panel_toggle_dir(&mut self, key: &str) {
        if !self.panel_collapsed_dirs.remove(key) {
            self.panel_collapsed_dirs.insert(key.to_string());
        }
    }

    /// Follows the panel cursor into the diff: if it rests on a file row
    /// whose file isn't already selected, scrolls the multibuffer to that
    /// file's section (expanding it if collapsed) via
    /// [`App::select_file_by_path`]. Directory rows, stash rows, an empty
    /// panel, and an out-of-range cursor leave the diff untouched. A no-op on
    /// the History tab — its rows have nothing to auto-follow into; opening a
    /// commit needs an explicit `Enter` (see [`App::panel_select`]). Pure
    /// in-memory on the Changes tab — never re-runs git. Always stays in
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
    /// returns focus to the diff; a directory row toggles its collapse and
    /// keeps the panel focused; an out-of-range cursor is a no-op, leaving
    /// the panel focused. On the History tab, opens the highlighted commit
    /// into the main diff view (see [`App::open_commit_view`]); an
    /// out-of-range cursor (an empty or still-loading list) is a no-op.
    pub fn panel_select(&mut self) {
        match self.panel_tab() {
            PanelTab::Changes => match navigable_rows(self).get(self.panel_cursor()) {
                Some(PanelRow::File(_)) => {
                    self.panel_follow();
                    self.mode = Mode::Normal;
                }
                Some(PanelRow::Dir(key)) => {
                    let key = key.clone();
                    self.panel_toggle_dir(&key);
                }
                None => {}
            },
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
        // The commit's header metadata is already in one of the lists it
        // could have been clicked from — the History tab's `history`, or
        // the Review launcher Commits tab's ahead-of-base `launcher_commits`
        // (the launcher's all-commits toggle reads `history` itself, so
        // that source is already covered) — so opening a commit needs no
        // extra git call just to populate the header block.
        let header = self
            .history
            .iter()
            .chain(self.launcher_commits.iter())
            .find(|c| c.sha == sha)
            .cloned();

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
#[path = "git_panel_tests.rs"]
mod tests;
