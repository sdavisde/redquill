//! The help overlay: a centered, scrollable box listing every binding,
//! grouped, plus the modal-mode key hints (Compose, List, Staging panel,
//! Search, Peek, the branch/worktree switcher) that aren't in the
//! [`Keymap`] table. Those modes handle keys
//! modally, bypassing the table; their hint sections render from the shared
//! per-mode tables in [`super::modal_keys`] — the same tables their handlers
//! dispatch from — so the overlay and the handlers can't drift apart.
//!
//! The full binding list is taller than most terminals, so the box caps its
//! height to a fraction of the screen and scrolls the overflow (a right-edge
//! scrollbar shows position); `j`/`k`/arrows, PageUp/PageDown, and `g`/`G`
//! drive it from [`super::handle_help_key`]. The scroll offset lives in
//! [`super::app::App::help_scroll`]; this renderer clamps it to the content
//! each frame and writes the clamped value back.
//!
//! `/` filters the list, lazygit-style (state in
//! [`super::app::App::help_search`], driven by [`super::handle_help_key`]):
//! typing narrows every section to rows whose key label or description
//! smartcase-matches the query ([`row_matches`]), dropping sections that end
//! up empty, and a locked-in filter shows in place of the subtitle.

use std::cell::Cell;

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Clear, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};

use super::keymap::{Action, Binding, Keymap, Scope};
use super::modal_keys::{
    COMMIT_MESSAGE_HINTS, COMPOSE_HINTS, HELP_SEARCH_HINTS, LIST_KEYS, ModalBinding, PEEK_KEYS,
    SEARCH_HINTS, STAGING_KEYS, SWITCHER_KEYS,
};
use super::search;
use super::theme::Theme;

/// The help overlay's group sections, in render order. Every [`Action`]'s
/// [`group_of`] must be one of these, or its binding would never render — the
/// `help_overlay_covers_every_keymap_binding` test pins that invariant.
const GROUP_ORDER: [&str; 8] = [
    "Navigation",
    "Annotate",
    "Stage",
    "Search",
    "Panels",
    "Code intelligence",
    "Git panel",
    "Quit",
];

/// Which help-overlay group an [`Action`] belongs to.
fn group_of(action: Action) -> &'static str {
    use Action::*;
    match action {
        CursorDown | CursorUp | CursorLeft | CursorRight | WordForward | WordBackward
        | HalfPageDown | HalfPageUp | JumpToTop | JumpToBottom | NextHunk | PrevHunk | NextFile
        | PrevFile | ToggleCollapse => "Navigation",
        EnterVisual | Compose => "Annotate",
        ToggleStage | StageFile | ToggleStagingPanel => "Stage",
        Search | SearchNext | SearchPrev => "Search",
        ToggleList | ToggleHelp | FocusGitPanel | ToggleCommandLog | Refresh => "Panels",
        GotoDefinition | GotoReferences | Hover => "Code intelligence",
        PanelCursorDown | PanelCursorUp | PanelSelect | TogglePanelTab | RemoteFetch
        | RemotePull | RemotePush | CommitStaged | OpenSwitcher => "Git panel",
        Quit | QuitDiscard => "Quit",
    }
}

/// Centers a `width` x `height` rect inside `area`.
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [area] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    area
}

fn section_header(label: &str, theme: &Theme) -> Line<'static> {
    Line::from(Span::styled(
        label.to_string(),
        Style::default()
            .fg(theme.help_section_header)
            .add_modifier(Modifier::BOLD),
    ))
}

fn key_line(key: &str, description: &str, key_width: usize, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{key:<key_width$}"),
            Style::default()
                .fg(theme.help_key)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("   "),
        Span::raw(description.to_string()),
    ])
}

/// Whether `action` is a capability the current diff target can't perform,
/// so it must be hidden from the help overlay (and, via the same predicate,
/// the footer strip — see `super::footer`'s `keymap_hints`/`pending_hints`).
///
/// - On a read-only range target the file/hunk/line stage gestures are inert
///   no-ops (see [`super::app::App::stage_file`] / [`super::staging::toggle_stage`]),
///   so listing them would be untruthful; the staging-panel toggle stays
///   visible because it still works (it shows the index regardless of
///   target).
/// - On any target whose new side isn't the live working tree, `gd`/`gr`/`K`
///   are inert no-ops too (see [`super::code_intel::request`], gated on
///   [`crate::git::DiffTarget::supports_code_intel`]), so they're hidden the
///   same way.
pub(super) fn binding_hidden(
    action: Action,
    staging_allowed: bool,
    code_intel_allowed: bool,
) -> bool {
    (!staging_allowed && matches!(action, Action::ToggleStage | Action::StageFile))
        || (!code_intel_allowed
            && matches!(
                action,
                Action::GotoDefinition | Action::GotoReferences | Action::Hover
            ))
}

/// Flattens a per-mode key table (see [`super::modal_keys`]) into the
/// `(key label, description)` rows the overlay prints — the erased view that
/// lets tables with different per-mode action types share one render loop.
fn modal_hints<A>(table: &'static [ModalBinding<A>]) -> Vec<(&'static str, &'static str)> {
    table.iter().map(|b| (b.label, b.description)).collect()
}

/// The modal-mode hint sections, in render order: each mode's section title
/// paired with the rows of its shared key table. The same tables drive the
/// modal handlers' dispatch, so the overlay can't document keys the handlers
/// don't accept (and vice versa — the `modal_keys` cross-check test pins the
/// handler side). `HELP_SEARCH_HINTS` (the overlay's own `/` filter, a
/// free-text input like Compose/Search) gets a section here for the same
/// reason those do; `HELP_KEYS` doesn't, since it's the enum-dispatch table
/// for the overlay's own scroll/close keys, already documented on the footer.
fn modal_sections() -> [(&'static str, Vec<(&'static str, &'static str)>); 8] {
    [
        ("Compose mode", modal_hints(COMPOSE_HINTS)),
        ("List mode", modal_hints(LIST_KEYS)),
        ("Staging panel", modal_hints(STAGING_KEYS)),
        ("Search input", modal_hints(SEARCH_HINTS)),
        ("Peek mode", modal_hints(PEEK_KEYS)),
        ("Branch/worktree switcher", modal_hints(SWITCHER_KEYS)),
        (
            "Commit message (c, git panel)",
            modal_hints(COMMIT_MESSAGE_HINTS),
        ),
        ("Help filter (/)", modal_hints(HELP_SEARCH_HINTS)),
    ]
}

/// Whether a keybind row (`label`, `description`) should be kept under the
/// help overlay's `/` filter: a smartcase substring match against either the
/// key label or the description (see [`search::smartcase_contains`]). An
/// empty query keeps everything, so this is a no-op filter when no search is
/// active.
fn row_matches(label: &str, description: &str, query: &str) -> bool {
    query.is_empty()
        || search::smartcase_contains(label, query)
        || search::smartcase_contains(description, query)
}

/// The overlay's scroll/filter state, owned by the caller ([`App`]) and
/// threaded through [`render`] each frame. Grouped into one struct rather
/// than three loose parameters to keep `render`'s argument count sane.
pub struct HelpViewState<'a> {
    /// The vertical scroll offset (advanced by [`super::handle_help_key`]);
    /// `render` clamps it to the (possibly filtered) content and writes the
    /// clamped value back.
    pub scroll: &'a Cell<u16>,
    /// The scrollable region's height, recorded by `render` each frame so the
    /// key handler can page by a real viewport (PageUp/PageDown).
    pub viewport: &'a Cell<u16>,
    /// Mirrors [`super::app::App::help_search`]: `Some((query, editing))`
    /// filters every section to rows matching `query` (dropping sections
    /// that end up empty) and shows the query in place of the subtitle —
    /// with a live text cursor while `editing`, or a "locked" hint once
    /// `Enter` has confirmed it. `None` renders the unfiltered list.
    pub search: Option<(&'a str, bool)>,
}

/// Renders the help overlay, centered over `area`. Bindings from the
/// [`Keymap`] table are grouped Navigation / Annotate / Panels / Quit, with
/// Compose-mode and List-mode hints appended below (those modes bypass the
/// table entirely, so they aren't in it). `staging_allowed` is `false` on a
/// read-only range target, hiding the inert file/hunk staging gestures;
/// `code_intel_allowed` is `false` whenever the target's new side isn't the
/// live working tree, hiding the inert `gd`/`gr`/`K` gestures the same way
/// (see [`binding_hidden`]).
///
/// The box caps its height to ~4/5 of `area` and scrolls the overflow; see
/// [`HelpViewState`] for the scroll/filter fields `state` carries.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    keymap: &Keymap,
    theme: &Theme,
    staging_allowed: bool,
    code_intel_allowed: bool,
    state: &HelpViewState,
) {
    let scroll = state.scroll;
    let viewport = state.viewport;
    let search = state.search;
    let query = search.map_or("", |(q, _)| q);
    let editing = search.is_some_and(|(_, editing)| editing);

    let sections = modal_sections();
    let bindings = keymap.bindings();
    // Column width is computed from the unfiltered rows, so it never jumps
    // around as the query narrows what's actually shown.
    let key_width = bindings
        .iter()
        .map(|b| b.key_label().len())
        .chain(
            sections
                .iter()
                .flat_map(|(_, hints)| hints.iter().map(|(k, _)| k.len())),
        )
        .max()
        .unwrap_or(0);

    let mut lines: Vec<Line> = Vec::new();
    let mut any_match = false;
    for group in GROUP_ORDER {
        let group_bindings: Vec<&Binding> = bindings
            .iter()
            .filter(|b| b.scope == Scope::Diff && group_of(b.action) == group)
            .filter(|b| !binding_hidden(b.action, staging_allowed, code_intel_allowed))
            .filter(|b| row_matches(&b.key_label(), b.description, query))
            .collect();
        if group_bindings.is_empty() {
            continue;
        }
        any_match = true;
        lines.push(section_header(group, theme));
        for b in group_bindings {
            lines.push(key_line(&b.key_label(), b.description, key_width, theme));
        }
        lines.push(Line::from(""));
    }

    // Panel-scope bindings: shown as their own section so `` ` `` /`j`/`k`/
    // Enter are documented in the context where they apply (the git panel
    // focused), grouped by scope per this spec.
    let panel_bindings: Vec<&Binding> = bindings
        .iter()
        .filter(|b| b.scope == Scope::Panel)
        .filter(|b| row_matches(&b.key_label(), b.description, query))
        .collect();
    if !panel_bindings.is_empty() {
        any_match = true;
        lines.push(section_header("Git panel (focused)", theme));
        for b in panel_bindings {
            lines.push(key_line(&b.key_label(), b.description, key_width, theme));
        }
        lines.push(Line::from(""));
    }

    let filtered_sections: Vec<(&str, Vec<(&str, &str)>)> = sections
        .iter()
        .map(|(title, hints)| {
            let hints: Vec<(&str, &str)> = hints
                .iter()
                .filter(|(k, d)| row_matches(k, d, query))
                .copied()
                .collect();
            (*title, hints)
        })
        .filter(|(_, hints)| !hints.is_empty())
        .collect();
    for (i, (title, hints)) in filtered_sections.iter().enumerate() {
        any_match = true;
        lines.push(section_header(title, theme));
        for (key, desc) in hints {
            lines.push(key_line(key, desc, key_width, theme));
        }
        if i + 1 < filtered_sections.len() {
            lines.push(Line::from(""));
        }
    }

    if !query.is_empty() && !any_match {
        lines.push(Line::from(Span::styled(
            format!("no matches for \"{query}\""),
            Style::default().fg(theme.status_message),
        )));
    }

    // The chrome around the scrollable list, herdr-style: a dim subtitle
    // under the title, a blank spacer, then the list; the "how to drive it"
    // hint rides the bottom border. The subtitle doubles as the filter box:
    // `/query` with a live cursor while editing, a "locked" reminder once
    // `Enter` has confirmed it, or the plain description when no filter is
    // active.
    let subtitle = match search {
        Some((q, true)) => format!("/{q}"),
        Some((q, false)) if !q.is_empty() => {
            format!("filter: /{q}  (/ to edit \u{00b7} esc to clear)")
        }
        _ => "available commands and configured shortcuts".to_string(),
    };
    let footer = "j/k scroll  \u{00b7}  pgup/pgdn page  \u{00b7}  g/G ends  \u{00b7}  / filter  \u{00b7}  esc close";
    let total = lines.len() as u16;

    // Width: fit the widest content line (or the subtitle/footer), plus a
    // column for the scrollbar, plus borders and 1-col side padding. Capped
    // so it never spills off a narrow terminal and never grows absurdly wide.
    let content_w = lines.iter().map(|l| l.width()).max().unwrap_or(0) as u16;
    let inner_w = content_w
        .max(subtitle.chars().count() as u16)
        .max(footer.chars().count() as u16)
        .saturating_add(1); // scrollbar gutter
    let width = (inner_w + 4).min(area.width.saturating_sub(2)).min(92);

    // Height: borders (2) + subtitle (1) + spacer (1) = 4 rows of chrome
    // around the list (the footer hint rides the bottom border, costing no
    // row). Cap to ~4/5 of the screen so it reads as a floating panel and
    // scrolls rather than filling every row.
    let chrome = 4u16;
    let desired = total.saturating_add(chrome);
    let cap = (area.height.saturating_mul(4) / 5).max(chrome + 1);
    let height = desired.min(cap).min(area.height.saturating_sub(2));
    let popup = centered(area, width, height);

    frame.render_widget(Clear, popup);

    let pill = Style::default()
        .bg(theme.help_key)
        .fg(Color::Black)
        .add_modifier(Modifier::BOLD);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.help_key))
        .padding(Padding::horizontal(1))
        .title_top(
            Line::from(Span::styled(
                " keybinds ",
                Style::default().add_modifier(Modifier::BOLD),
            ))
            .left_aligned(),
        )
        .title_top(Line::from(Span::styled(" esc close ", pill)).right_aligned())
        .title_bottom(
            Line::from(Span::styled(
                format!(" {footer} "),
                Style::default().fg(theme.footer_text),
            ))
            .centered(),
        );
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let [subtitle_area, _spacer, list_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(inner);

    // Active/locked filters read in the search-prompt color so they stand out
    // from the plain description text.
    let subtitle_color = if search.is_some() {
        theme.search_prompt
    } else {
        theme.footer_text
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            subtitle.clone(),
            Style::default().fg(subtitle_color),
        ))),
        subtitle_area,
    );
    if editing {
        // A live text cursor at the end of the query, like the diff-view
        // search prompt (`mod.rs`'s footer render for `Mode::Search`).
        let cursor_x = subtitle_area
            .x
            .saturating_add(subtitle.chars().count() as u16)
            .min(subtitle_area.x + subtitle_area.width.saturating_sub(1));
        frame.set_cursor_position(Position::new(cursor_x, subtitle_area.y));
    }

    // Clamp the caller's scroll offset to the content now that the viewport
    // height is known, and record that height for PageUp/PageDown paging.
    let list_h = list_area.height;
    let max_scroll = total.saturating_sub(list_h);
    let offset = scroll.get().min(max_scroll);
    scroll.set(offset);
    viewport.set(list_h);

    // Reserve the right column for the scrollbar only when it's needed.
    let scrollable = total > list_h;
    let text_area = if scrollable {
        Rect {
            width: list_area.width.saturating_sub(1),
            ..list_area
        }
    } else {
        list_area
    };
    frame.render_widget(Paragraph::new(lines).scroll((offset, 0)), text_area);

    if scrollable {
        let mut sb_state = ScrollbarState::new(total as usize)
            .position(offset as usize)
            .viewport_content_length(list_h as usize);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None);
        frame.render_stateful_widget(scrollbar, list_area, &mut sb_state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every binding in the keymap must land in one of the overlay's rendered
    /// groups (`GROUP_ORDER`). If an action's `group_of` returned a label not
    /// in that list, its binding would be silently omitted from `?` — the
    /// "every user-visible action is listed in the `?` help overlay" rule
    /// (CLAUDE.md). Because `group_of` is an exhaustive match, this also
    /// guarantees any newly added `Action` is forced into a visible group.
    #[test]
    fn help_overlay_covers_every_keymap_binding() {
        let keymap = Keymap::default_map();
        for binding in keymap.bindings() {
            let group = group_of(binding.action);
            assert!(
                GROUP_ORDER.contains(&group),
                "binding {:?} ({}) maps to group {group:?}, which the help overlay never renders",
                binding.action,
                binding.key_label(),
            );
        }
    }

    // -- Capability gating: binding_hidden ----------------------------------

    #[test]
    fn code_intel_actions_hidden_only_when_code_intel_disallowed() {
        for action in [
            Action::GotoDefinition,
            Action::GotoReferences,
            Action::Hover,
        ] {
            assert!(
                binding_hidden(action, true, false),
                "{action:?} must be hidden when code-intel is unsupported"
            );
            assert!(
                !binding_hidden(action, true, true),
                "{action:?} must be shown when code-intel is supported"
            );
        }
    }

    #[test]
    fn staging_actions_are_unaffected_by_code_intel_allowed() {
        for action in [Action::ToggleStage, Action::StageFile] {
            assert!(binding_hidden(action, false, true));
            assert!(!binding_hidden(action, true, true));
        }
    }

    #[test]
    fn unrelated_actions_are_never_hidden_by_either_flag() {
        assert!(!binding_hidden(Action::CursorDown, false, false));
        assert!(!binding_hidden(Action::Quit, false, false));
    }
}
