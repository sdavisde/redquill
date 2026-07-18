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
//! [`HelpOverlayState::scroll`] (one field of the overlay's consolidated
//! state, owned by `App`); this renderer clamps it to the content each frame
//! and writes the clamped value back.
//!
//! `/` filters the list, lazygit-style (state in
//! [`HelpOverlayState::search`], driven by [`super::handle_help_key`]):
//! typing narrows every section to rows whose key label or description
//! smartcase-matches the query ([`row_matches`]), dropping sections that end
//! up empty, and a locked-in filter shows in place of the subtitle.
//!
//! Two tabs ([`HelpOverlayState::tab`]) share this chrome: **This context**
//! (default on open) is [`this_context_sections`] — only the bindings for the
//! mode/scope `?` was pressed from, plus "Works everywhere"; **All keys** is
//! [`all_keys_sections`], the full reference described above. `Tab`/`l` and
//! `Shift-Tab`/`h` switch tabs (see [`super::modal_keys::HELP_KEYS`]),
//! resetting the filter and scroll each time.

use std::cell::Cell;

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Clear, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};

use super::app::ModeOrigin;
use super::keymap::{Action, Binding, Keymap, Scope};
use super::modal_keys::{ModalBinding, ModalKeymaps};
use super::search;
use super::theme::Theme;

/// The help overlay's group sections, in render order. Every [`Action`]'s
/// [`group_of`] must be one of these, or its binding would never render — the
/// `help_overlay_covers_every_keymap_binding` test pins that invariant.
const GROUP_ORDER: [&str; 9] = [
    "Navigation",
    "Annotate",
    "Stage",
    "Review",
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
        CursorDown | CursorUp | CursorLeft | CursorRight | CursorLineStart | CursorLineEnd
        | WordForward | WordBackward | HalfPageDown | HalfPageUp | FullPageDown | FullPageUp
        | JumpToTop | JumpToBottom | NextHunk | PrevHunk | NextFile | PrevFile | ToggleCollapse
        | RecenterCursor | ScrollCursorTop | ScrollCursorBottom => "Navigation",
        EnterVisual | Compose | EditAnnotation | DeleteAnnotation => "Annotate",
        ToggleStage | StageFile | ToggleStagingPanel => "Stage",
        ToggleAccept | AcceptFile | ToggleDefer => "Review",
        Search | SearchNext | SearchPrev | SearchWordForward | SearchWordBackward => "Search",
        ToggleList | ToggleHelp | FocusGitPanel | ToggleCommandLog | Refresh | OpenFileFinder
        | OpenProjectSearch | OpenEditor | DismissConfigWarning | OpenReviewLauncher => "Panels",
        GotoDefinition | GotoReferences | Hover => "Code intelligence",
        PanelCursorDown | PanelCursorUp | PanelSelect | TogglePanelTab | RemoteFetch
        | RemotePull | RemotePush | CommitStaged | OpenSwitcher => "Git panel",
        Quit | QuitDiscard => "Quit",
    }
}

/// Centers a `width` x `height` rect inside `area`. Shared with
/// [`super::welcome`], which centers its situation/hints block the same way
/// inside the diff pane rather than duplicating the two-axis `Flex::Center`
/// dance.
pub(super) fn centered(area: Rect, width: u16, height: u16) -> Rect {
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

/// Whether `action` is a capability the current diff target/session can't
/// perform, so it must be hidden from the help overlay (and, via the same
/// predicate, the footer strip — see `super::footer`'s
/// `keymap_hints`/`pending_hints`).
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
/// - Outside a review session, the accept/defer actions are
///   hidden entirely — per the "inapplicable keys are omitted, not
///   inert-but-listed" convention this repo follows for capability gating —
///   and, since [`crate::git::DiffTarget::Review`]'s [`crate::git::StagingMode`]
///   is always [`crate::git::StagingMode::ReadOnly`], `staging_allowed`
///   already hides `ToggleStage`/`StageFile` during a review session, so the
///   two families of rows never both show for the same key at once.
pub(super) fn binding_hidden(
    action: Action,
    staging_allowed: bool,
    code_intel_allowed: bool,
    review_session: bool,
) -> bool {
    (!staging_allowed && matches!(action, Action::ToggleStage | Action::StageFile))
        || (!code_intel_allowed
            && matches!(
                action,
                Action::GotoDefinition | Action::GotoReferences | Action::Hover
            ))
        || (!review_session
            && matches!(
                action,
                Action::ToggleAccept | Action::AcceptFile | Action::ToggleDefer
            ))
}

/// Flattens a per-mode key table (see [`super::modal_keys`]) into the
/// `(key label, description)` rows the overlay prints — the erased view that
/// lets tables with different per-mode action types share one render loop.
/// Takes the *effective* table (`app.modal_keys.*`), not the compiled-in
/// `'static` default, so a config remap shows up here with no additional
/// wiring.
fn modal_hints<A: Clone>(table: &[ModalBinding<A>]) -> Vec<(String, &'static str)> {
    table
        .iter()
        .map(|b| (b.key_label(), b.description))
        .collect()
}

/// The modal-mode hint sections, in render order: each mode's section title
/// paired with the rows of its shared key table. The same tables drive the
/// modal handlers' dispatch, so the overlay can't document keys the handlers
/// don't accept (and vice versa — the `modal_keys` cross-check test pins the
/// handler side). `help-search` (the overlay's own `/` filter, a free-text
/// input like Compose/Search) gets a section here for the same reason those
/// do; `help` doesn't, since it's the enum-dispatch table for the overlay's
/// own scroll/close keys, already documented on the footer.
///
/// `review_session` swaps the "Staging panel" slot for the accepted-files
/// panel's own table/title during a review session —
/// `Mode::Staging` is one mode shared by both panels
/// (`super::modes::handle_staging_key` dispatches to whichever table
/// applies), so only one of the two ever documents itself here at a time,
/// exactly like `Action::ToggleStage`/`Action::ToggleAccept`'s mutual
/// exclusion in [`binding_hidden`].
fn modal_sections(modal_keys: &ModalKeymaps, review_session: bool) -> [Section; 14] {
    let staging_section = if review_session {
        (
            "Accepted files panel (s, review sessions)",
            modal_hints(&modal_keys.accepted_panel),
        )
    } else {
        ("Staging panel", modal_hints(&modal_keys.staging))
    };
    [
        ("Compose mode", modal_hints(&modal_keys.compose)),
        ("List mode", modal_hints(&modal_keys.list)),
        staging_section,
        ("Search input", modal_hints(&modal_keys.search)),
        ("Peek mode", modal_hints(&modal_keys.peek)),
        (
            "Branch/worktree switcher",
            modal_hints(&modal_keys.switcher),
        ),
        (
            "Review launcher (R, works everywhere)",
            modal_hints(&modal_keys.review_launcher),
        ),
        (
            "Commit message (c, git panel)",
            modal_hints(&modal_keys.commit_message),
        ),
        ("Help filter (/)", modal_hints(&modal_keys.help_search)),
        ("Fuzzy file finder (gp)", modal_hints(&modal_keys.finder)),
        (
            "Project search — input focus (g/)",
            modal_hints(&modal_keys.project_search_input),
        ),
        (
            "Project search — results focus",
            modal_hints(&modal_keys.project_search_results),
        ),
        (
            "End review modal (q, review session)",
            modal_hints(&modal_keys.end_review),
        ),
        (
            "Pull/push confirm (p/P, review session)",
            modal_hints(&modal_keys.confirm_remote_op),
        ),
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

/// The help overlay's two tabs: "This context" (default on open) shows only
/// the bindings that apply to the mode/scope `?` was opened from, plus the
/// "Works everywhere" global section; "All keys" is the pre-existing full
/// grouped reference across every scope and mode, unchanged in content
/// (FR-3). With exactly two tabs, `Tab`/`l` (next) and `Shift-Tab`/`h`
/// (previous) both just flip between them — see [`HelpTab::toggled`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HelpTab {
    #[default]
    ThisContext,
    AllKeys,
}

impl HelpTab {
    /// The other tab. One method serves both `next` and `previous`
    /// dispatch since a two-tab pair has no third state to distinguish them.
    pub(super) fn toggled(self) -> HelpTab {
        match self {
            HelpTab::ThisContext => HelpTab::AllKeys,
            HelpTab::AllKeys => HelpTab::ThisContext,
        }
    }
}

/// The help overlay's full state, owned by [`super::app::App`] as one field
/// rather than four loose ones (`help_open`/`help_scroll`/`help_viewport`/
/// `help_search`, the shape this consolidates): whether it's open, the
/// scroll/viewport/search fields [`HelpViewState`] borrows each frame,
/// `origin` — the mode `?` was pressed from, captured via
/// [`ModeOrigin::capture`] the same way [`super::app::Mode::ReviewLauncher`]
/// captures where `R` was pressed from — and `tab`, the active
/// [`HelpTab`]. `origin` picks which bindings the "This context" tab shows
/// (see [`this_context_sections`]); switching tabs resets `search` to `None`
/// and `scroll` to `0` (see [`super::handle_help_key`]).
pub struct HelpOverlayState {
    /// Whether the overlay is open.
    pub(super) open: bool,
    /// The vertical scroll offset (see [`HelpViewState::scroll`]).
    pub(super) scroll: Cell<u16>,
    /// The scrollable region's height (see [`HelpViewState::viewport`]).
    pub(super) viewport: Cell<u16>,
    /// The `/` keybind filter (see [`HelpViewState::search`]).
    pub(super) search: Option<(String, bool)>,
    /// The mode/scope the overlay was opened over.
    pub(super) origin: ModeOrigin,
    /// The active tab.
    pub(super) tab: HelpTab,
}

impl HelpOverlayState {
    /// A closed overlay with no scroll/filter, a `Normal` origin, and the
    /// `ThisContext` tab (the harmless defaults a fresh [`super::app::App`]
    /// starts with; opening `?` overwrites `origin` with the real one and
    /// always lands on `ThisContext` — see [`super::app::App::apply`]'s
    /// `ToggleHelp` arm).
    pub(super) fn new() -> Self {
        HelpOverlayState {
            open: false,
            scroll: Cell::new(0),
            viewport: Cell::new(0),
            search: None,
            origin: ModeOrigin::Normal,
            tab: HelpTab::ThisContext,
        }
    }
}

/// The overlay's scroll/filter/tab state, owned by the caller ([`App`]) and
/// threaded through [`render`] each frame. Grouped into one struct rather
/// than loose parameters to keep `render`'s argument count sane.
pub struct HelpViewState<'a> {
    /// The vertical scroll offset (advanced by [`super::handle_help_key`]);
    /// `render` clamps it to the (possibly filtered) content and writes the
    /// clamped value back.
    pub scroll: &'a Cell<u16>,
    /// The scrollable region's height, recorded by `render` each frame so the
    /// key handler can page by a real viewport (PageUp/PageDown).
    pub viewport: &'a Cell<u16>,
    /// Mirrors [`HelpOverlayState::search`]: `Some((query, editing))`
    /// filters every section to rows matching `query` (dropping sections
    /// that end up empty) and shows the query in place of the subtitle —
    /// with a live text cursor while `editing`, or a "locked" hint once
    /// `Enter` has confirmed it. `None` renders the unfiltered list.
    pub search: Option<(&'a str, bool)>,
    /// The active tab (see [`HelpOverlayState::tab`]).
    pub tab: HelpTab,
}

/// The two tables the overlay renders from, plus the origin the overlay
/// opened over, bundled to keep [`render`]'s argument count under clippy's
/// `too_many_arguments` threshold: the main keymap, every modal mode's
/// effective table (`app`'s post-`[keys.*]`-override tables), and the
/// mode/scope `?` was pressed from are always passed together.
pub struct HelpTables<'a> {
    pub keymap: &'a Keymap,
    pub modal_keys: &'a ModalKeymaps,
    pub origin: ModeOrigin,
}

/// One titled block of `(key label, description)` rows — the shape every
/// section builder below returns and [`render`] lays out uniformly (the same
/// shape [`modal_sections`] already returned, since it predates this alias).
/// Capability gating ([`binding_hidden`]) is already applied by the builders;
/// the `/` filter is deliberately not — `render` applies [`row_matches`] to
/// the rows each frame, since the query is the one thing that legitimately
/// varies without re-deriving these tables.
type Section = (&'static str, Vec<(String, &'static str)>);

/// The "Works everywhere" section: every `Scope::Global` binding not hidden
/// by capability gating, in keymap order.
fn global_section(
    bindings: &[Binding],
    staging_allowed: bool,
    code_intel_allowed: bool,
    review_session: bool,
) -> Section {
    let rows = bindings
        .iter()
        .filter(|b| b.scope == Scope::Global)
        .filter(|b| {
            !binding_hidden(
                b.action,
                staging_allowed,
                code_intel_allowed,
                review_session,
            )
        })
        .map(|b| (b.key_label(), b.description))
        .collect();
    ("Works everywhere", rows)
}

/// The diff-scope group sections, one per [`GROUP_ORDER`] entry in that
/// order: each group's `Scope::Diff` bindings, capability-gated rows
/// dropped, in keymap order. Empty groups are kept here — `render` drops
/// them after applying the `/` query, the same point the other sections drop
/// theirs — so this stays index-aligned with `GROUP_ORDER` for any future
/// caller iterating both together.
fn diff_group_sections(
    bindings: &[Binding],
    staging_allowed: bool,
    code_intel_allowed: bool,
    review_session: bool,
) -> Vec<Section> {
    GROUP_ORDER
        .iter()
        .map(|&group| {
            let rows = bindings
                .iter()
                .filter(|b| b.scope == Scope::Diff && group_of(b.action) == group)
                .filter(|b| {
                    !binding_hidden(
                        b.action,
                        staging_allowed,
                        code_intel_allowed,
                        review_session,
                    )
                })
                .map(|b| (b.key_label(), b.description))
                .collect();
            (group, rows)
        })
        .collect()
}

/// The focused-git-panel section: every `Scope::Panel` binding not hidden by
/// capability gating, in keymap order — the panel's per-file rows
/// (stage/accept/defer) obey the same [`binding_hidden`] rules as their
/// diff-scope counterparts.
fn panel_section(
    bindings: &[Binding],
    staging_allowed: bool,
    code_intel_allowed: bool,
    review_session: bool,
) -> Section {
    let rows = bindings
        .iter()
        .filter(|b| b.scope == Scope::Panel)
        .filter(|b| {
            !binding_hidden(
                b.action,
                staging_allowed,
                code_intel_allowed,
                review_session,
            )
        })
        .map(|b| (b.key_label(), b.description))
        .collect();
    ("Git panel (focused)", rows)
}

/// The common-workflows header's curated content (FR-6): an intent phrase
/// paired with the `Action` it triggers. Kept to a target of 5 entries (hard
/// cap 7, enforced by
/// [`tests::workflow_entries_are_bound_in_the_default_keymap_and_within_the_cap`])
/// so a new user can scan the whole thing at a glance. Every entry here must
/// resolve to a real binding in the *default* keymap — that same test fails
/// the build if a curation edit orphans one; a user's own remap or unbind
/// only ever degrades this at runtime (see [`workflow_rows`]), never here.
const WORKFLOW_ENTRIES: [(&str, Action); 5] = [
    ("Review a branch or commit", Action::OpenReviewLauncher),
    ("Comment on a line", Action::Compose),
    // Phrased for both scopes: the diff view stages the hunk under the
    // cursor, the git panel stages the highlighted file.
    ("Stage the change under the cursor", Action::ToggleStage),
    ("Search the diff", Action::Search),
    ("Quit and copy annotations", Action::Quit),
];

/// One resolved common-workflows row: a curated intent phrase paired with the
/// key label(s) that trigger it in the *effective*, current-context keymap
/// (FR-7) — built by [`workflow_rows`]. `pub(super)` so
/// `super::keymap_config_tests`' remap-display test (FR-7) can inspect a
/// resolved row directly rather than re-deriving this shape.
pub(super) struct WorkflowRow {
    pub(super) phrase: &'static str,
    pub(super) key: String,
}

/// Resolves [`WORKFLOW_ENTRIES`] against the effective keymap for the
/// overlay's `origin`, in table order. Two things can drop an entry, both
/// deliberate graceful degradation rather than a stale or wrong row (FR-8,
/// FR-9):
///
/// - `origin` picks the scope its action must resolve in — [`Scope::Diff`]
///   for `Normal`/`Visual`, [`Scope::Panel`] for `Panel`, mirroring
///   [`this_context_sections`]'s own origin-to-scope mapping, so the header
///   never shows a key that means something else in the mode `?` was opened
///   from (e.g. plain `c` is `CommitStaged` in the git panel, not `Compose`).
///   [`Keymap::label_for`] still falls back to [`Scope::Global`] within that
///   scope, so `Scope::Global` entries (the launcher, quit) resolve from
///   either origin.
/// - the same [`binding_hidden`] capability gating as regular help rows
///   (staging on a read-only target, code-intel off-target, review-only
///   actions outside a session) hides an inapplicable entry (FR-9).
///
/// An entry a user has unbound in their config resolves to `None` from
/// [`Keymap::label_for`] and is silently omitted — the graceful half of
/// FR-8; the build-breaking half (a curated entry unbound in the *default*
/// keymap) is a separate drift test, not this function's job.
pub(super) fn workflow_rows(
    origin: ModeOrigin,
    keymap: &Keymap,
    staging_allowed: bool,
    code_intel_allowed: bool,
    review_session: bool,
) -> Vec<WorkflowRow> {
    let scope = match origin {
        ModeOrigin::Normal | ModeOrigin::Visual { .. } => Scope::Diff,
        ModeOrigin::Panel { .. } => Scope::Panel,
    };
    WORKFLOW_ENTRIES
        .iter()
        .filter(|(_, action)| {
            !binding_hidden(*action, staging_allowed, code_intel_allowed, review_session)
        })
        .filter_map(|&(phrase, action)| {
            keymap
                .label_for(scope, action)
                .map(|key| WorkflowRow { phrase, key })
        })
        .collect()
}

/// A common-workflows row: intent phrase left, key right (FR-7's Design
/// Considerations) — the mirror image of [`key_line`]'s key-left layout,
/// which is what visually sets the header apart from the sections below it.
fn workflow_line(phrase: &str, key: &str, phrase_width: usize, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::raw(format!("{phrase:<phrase_width$}")),
        Span::raw("   "),
        Span::styled(
            key.to_string(),
            Style::default()
                .fg(theme.help_key)
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

/// The "This context" tab's sections: the bindings that apply to the
/// mode/scope `?` was opened from (`origin`), followed by the "Works
/// everywhere" global section. Every [`ModeOrigin`] variant maps to exactly
/// one scope — `Normal`/`Visual` both read the `Scope::Diff` groups (in
/// [`GROUP_ORDER`] order), `Panel` reads the single `Scope::Panel`
/// section — so this always renders a proper subset of
/// [`all_keys_sections`]'s content, never a duplicate or a divergent set.
///
/// The common-workflows header (FR-6/FR-7) renders above these sections but
/// isn't part of this `Vec<Section>` — its phrase-left/key-right layout is
/// deliberately not the (key, description) shape [`Section`] rows share, so
/// [`render`] builds it separately via [`workflow_rows`] and prepends its
/// lines before this function's output.
fn this_context_sections(
    origin: ModeOrigin,
    bindings: &[Binding],
    staging_allowed: bool,
    code_intel_allowed: bool,
    review_session: bool,
) -> Vec<Section> {
    let mut sections = match origin {
        ModeOrigin::Normal | ModeOrigin::Visual { .. } => diff_group_sections(
            bindings,
            staging_allowed,
            code_intel_allowed,
            review_session,
        ),
        ModeOrigin::Panel { .. } => vec![panel_section(
            bindings,
            staging_allowed,
            code_intel_allowed,
            review_session,
        )],
    };
    sections.push(global_section(
        bindings,
        staging_allowed,
        code_intel_allowed,
        review_session,
    ));
    sections
}

/// The "All keys" tab's sections: today's full grouped reference — "Works
/// everywhere" first, then every [`GROUP_ORDER`] diff-scope group, the
/// focused-panel section, and the modal-mode hint sections — unchanged in
/// content from before the tabbed overlay existed (FR-3).
fn all_keys_sections(
    bindings: &[Binding],
    modal_keys: &ModalKeymaps,
    staging_allowed: bool,
    code_intel_allowed: bool,
    review_session: bool,
) -> Vec<Section> {
    let mut sections = vec![global_section(
        bindings,
        staging_allowed,
        code_intel_allowed,
        review_session,
    )];
    sections.extend(diff_group_sections(
        bindings,
        staging_allowed,
        code_intel_allowed,
        review_session,
    ));
    sections.push(panel_section(
        bindings,
        staging_allowed,
        code_intel_allowed,
        review_session,
    ));
    sections.extend(modal_sections(modal_keys, review_session));
    sections
}

/// The "This context │ All keys" tab bar, active tab emphasized — mirrors
/// [`super::switcher_modal::tab_bar`] / [`super::review_launcher_modal::tab_bar`]
/// for idiom consistency across this repo's tabbed overlays. Rendered as an
/// additional centered block title alongside the overlay's existing
/// left "keybinds" / right "esc close" titles, so the chrome the spec calls
/// out to keep ("centered, scrollable, `/` filter line") costs no extra row.
fn tab_bar(active: HelpTab, theme: &Theme) -> Line<'static> {
    let active_style = Style::default()
        .fg(theme.help_key)
        .add_modifier(Modifier::BOLD);
    let inactive_style = Style::default().fg(theme.footer_text);
    let (context_style, all_style) = match active {
        HelpTab::ThisContext => (active_style, inactive_style),
        HelpTab::AllKeys => (inactive_style, active_style),
    };
    Line::from(vec![
        Span::styled("This context", context_style),
        Span::styled(" \u{2502} ", Style::default().fg(theme.footer_text)),
        Span::styled("All keys", all_style),
    ])
}

/// Renders the help overlay, centered over `area`. `state.tab` picks which
/// sections show: [`HelpTab::ThisContext`] (default on open) is
/// [`this_context_sections`] — just the bindings for the mode/scope `?` was
/// pressed from, plus "Works everywhere"; [`HelpTab::AllKeys`] is
/// [`all_keys_sections`] — the full grouped reference (Works everywhere /
/// Navigation / Annotate / Stage / Review / Search / Panels / Code
/// intelligence / Git panel / Quit, then the modal-mode sections). The `/`
/// filter narrows whichever tab is active.
/// `staging_allowed` is `false` on a read-only range target, hiding the
/// inert file/hunk staging gestures; `code_intel_allowed` is `false`
/// whenever the target's new side isn't the live working tree, hiding the
/// inert `gd`/`gr`/`K` gestures; `review_session` is `false` outside a
/// review session, hiding the accept/defer gestures — see [`binding_hidden`]
/// for how the three combine.
///
/// The box caps its height to ~3/5 of `area` and scrolls the overflow; see
/// [`HelpViewState`] for the scroll/filter/tab fields `state` carries.
#[allow(clippy::too_many_arguments)]
pub fn render(
    frame: &mut Frame,
    area: Rect,
    tables: &HelpTables,
    theme: &Theme,
    staging_allowed: bool,
    code_intel_allowed: bool,
    review_session: bool,
    state: &HelpViewState,
) {
    let scroll = state.scroll;
    let viewport = state.viewport;
    let search = state.search;
    let query = search.map_or("", |(q, _)| q);
    let editing = search.is_some_and(|(_, editing)| editing);

    let bindings = tables.keymap.bindings();
    let sections: Vec<Section> = match state.tab {
        HelpTab::ThisContext => this_context_sections(
            tables.origin,
            bindings,
            staging_allowed,
            code_intel_allowed,
            review_session,
        ),
        HelpTab::AllKeys => all_keys_sections(
            bindings,
            tables.modal_keys,
            staging_allowed,
            code_intel_allowed,
            review_session,
        ),
    };
    // Column width is computed from the active tab's unfiltered rows, so it
    // never jumps around as the query narrows what's actually shown, and
    // "This context" (a strict subset) isn't stretched to "All keys"' wider
    // labels.
    let key_width = sections
        .iter()
        .flat_map(|(_, rows)| rows.iter().map(|(k, _)| k.len()))
        .max()
        .unwrap_or(0);

    let mut lines: Vec<Line> = Vec::new();

    // The common-workflows header (FR-6/FR-7) sits above the sections below,
    // "This context" only — it names an origin/scope, which "All keys"
    // deliberately spans every scope at once, so there's no single context
    // for the header to resolve keys against there. The `/` filter narrows
    // it exactly like a section (query matched against phrase or key,
    // dropped entirely when nothing matches), which is why its row count
    // feeds `any_match` below rather than being computed separately.
    let workflow_rows_filtered: Vec<WorkflowRow> = if state.tab == HelpTab::ThisContext {
        workflow_rows(
            tables.origin,
            tables.keymap,
            staging_allowed,
            code_intel_allowed,
            review_session,
        )
        .into_iter()
        .filter(|row| row_matches(&row.key, row.phrase, query))
        .collect()
    } else {
        Vec::new()
    };
    if !workflow_rows_filtered.is_empty() {
        let phrase_width = workflow_rows_filtered
            .iter()
            .map(|row| row.phrase.chars().count())
            .max()
            .unwrap_or(0);
        lines.push(section_header("Common workflows", theme));
        for row in &workflow_rows_filtered {
            lines.push(workflow_line(row.phrase, &row.key, phrase_width, theme));
        }
        lines.push(Line::from(""));
    }

    let filtered_sections: Vec<(&str, Vec<(&str, &str)>)> = sections
        .iter()
        .map(|(title, rows)| {
            let rows: Vec<(&str, &str)> = rows
                .iter()
                .map(|(k, d)| (k.as_str(), *d))
                .filter(|(k, d)| row_matches(k, d, query))
                .collect();
            (*title, rows)
        })
        .filter(|(_, rows)| !rows.is_empty())
        .collect();
    let any_match = !filtered_sections.is_empty() || !workflow_rows_filtered.is_empty();
    for (i, (title, rows)) in filtered_sections.iter().enumerate() {
        lines.push(section_header(title, theme));
        for (key, desc) in rows {
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
    let footer = "j/k scroll  \u{00b7}  pgup/pgdn page  \u{00b7}  g/G ends  \u{00b7}  / filter  \u{00b7}  tab/shift-tab tabs  \u{00b7}  esc close";
    let total = lines.len() as u16;

    // Width: fit the widest content line (or the subtitle/footer), plus a
    // column for the scrollbar, plus borders and 1-col side padding. Capped
    // so it never spills off a narrow terminal and never grows absurdly wide.
    // 130 comfortably fits the widest default row — a modal key label built
    // from several alternate encodings for one action (e.g. the switcher's
    // `ToggleTab`, bound to `Tab`/`Shift-Tab`/`h`/`l`/`Left`/`Right`) — with
    // room to spare.
    let content_w = lines.iter().map(|l| l.width()).max().unwrap_or(0) as u16;
    let inner_w = content_w
        .max(subtitle.chars().count() as u16)
        .max(footer.chars().count() as u16)
        .saturating_add(1); // scrollbar gutter
    let width = (inner_w + 4).min(area.width.saturating_sub(2)).min(130);

    // Height: borders (2) + subtitle (1) + spacer (1) = 4 rows of chrome
    // around the list (the footer hint rides the bottom border, costing no
    // row). Cap to ~3/5 of the screen so it reads as a floating panel and
    // scrolls rather than filling every row.
    let chrome = 4u16;
    let desired = total.saturating_add(chrome);
    let cap = (area.height.saturating_mul(3) / 5).max(chrome + 1);
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
        .title_top(tab_bar(state.tab, theme).centered())
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
        // Scale the state to the scrollable range (`max_scroll`), not the
        // full content length: `position` can only ever reach `max_scroll`
        // (see the clamp above), so scaling the track to `total` instead
        // left the thumb short of the bottom no matter how far down the
        // clamped offset went.
        let mut sb_state = ScrollbarState::new(max_scroll as usize)
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
                binding_hidden(action, true, false, false),
                "{action:?} must be hidden when code-intel is unsupported"
            );
            assert!(
                !binding_hidden(action, true, true, false),
                "{action:?} must be shown when code-intel is supported"
            );
        }
    }

    #[test]
    fn staging_actions_are_unaffected_by_code_intel_allowed() {
        for action in [Action::ToggleStage, Action::StageFile] {
            assert!(binding_hidden(action, false, true, false));
            assert!(!binding_hidden(action, true, true, false));
        }
    }

    #[test]
    fn unrelated_actions_are_never_hidden_by_either_flag() {
        assert!(!binding_hidden(Action::CursorDown, false, false, false));
        assert!(!binding_hidden(Action::Quit, false, false, false));
    }

    // -- Capability gating: review-session actions ---------------------------

    #[test]
    fn review_actions_hidden_only_outside_a_review_session() {
        for action in [
            Action::ToggleAccept,
            Action::AcceptFile,
            Action::ToggleDefer,
        ] {
            assert!(
                binding_hidden(action, true, true, false),
                "{action:?} must be hidden outside a review session"
            );
            assert!(
                !binding_hidden(action, true, true, true),
                "{action:?} must be shown during a review session"
            );
        }
    }

    #[test]
    fn staging_actions_are_hidden_during_a_review_session() {
        // A review target's `staging_mode()` is always `ReadOnly`, so
        // `staging_allowed` is always `false` there — this pins that the two
        // families of bindings (staging vs. review) never both show for the
        // same physical key at once.
        for action in [Action::ToggleStage, Action::StageFile] {
            assert!(binding_hidden(action, false, true, true));
        }
    }

    #[test]
    fn review_actions_are_unaffected_by_code_intel_allowed() {
        for action in [
            Action::ToggleAccept,
            Action::AcceptFile,
            Action::ToggleDefer,
        ] {
            assert!(binding_hidden(action, true, false, false));
            assert!(!binding_hidden(action, true, false, true));
        }
    }

    // -- HelpTab ---------------------------------------------------------

    #[test]
    fn help_tab_defaults_to_this_context() {
        assert_eq!(HelpTab::default(), HelpTab::ThisContext);
        assert_eq!(HelpOverlayState::new().tab, HelpTab::ThisContext);
    }

    #[test]
    fn help_tab_toggles_between_the_two_tabs() {
        assert_eq!(HelpTab::ThisContext.toggled(), HelpTab::AllKeys);
        assert_eq!(HelpTab::AllKeys.toggled(), HelpTab::ThisContext);
    }

    // -- Scrollbar thumb travel --------------------------------------------

    /// Renders the overlay at a fixed size with the given scroll offset
    /// (`render` clamps it to the content, so `u16::MAX` is a legitimate way
    /// to ask for "the end") and returns the resulting buffer.
    fn render_help_at_scroll(offset: u16) -> ratatui::buffer::Buffer {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let keymap = Keymap::default_map();
        let modal_keys = ModalKeymaps::default();
        let theme = Theme::default();
        let tables = HelpTables {
            keymap: &keymap,
            modal_keys: &modal_keys,
            origin: ModeOrigin::Normal,
        };
        let scroll = Cell::new(offset);
        let viewport = Cell::new(0);
        // All keys is the full reference, long enough that a 100x22 terminal
        // forces scrolling.
        let state = HelpViewState {
            scroll: &scroll,
            viewport: &viewport,
            search: None,
            tab: HelpTab::AllKeys,
        };

        let backend = TestBackend::new(100, 22);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render(frame, area, &tables, &theme, true, true, true, &state);
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    /// The symbol at `(x, y)`, or `""` if out of bounds — a thin wrapper over
    /// [`ratatui::buffer::Buffer::cell`] since the tests below only ever look
    /// at cells they've already located within the buffer.
    fn symbol_at(buffer: &ratatui::buffer::Buffer, x: u16, y: u16) -> &str {
        buffer.cell((x, y)).map_or("", |cell| cell.symbol())
    }

    /// The (x, y) of the first cell in `buffer` whose symbol is `symbol` —
    /// used to locate the popup's rounded-border corners without duplicating
    /// `render`'s own width/height arithmetic.
    fn find_symbol(buffer: &ratatui::buffer::Buffer, symbol: &str) -> (u16, u16) {
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                if symbol_at(buffer, x, y) == symbol {
                    return (x, y);
                }
            }
        }
        panic!("symbol {symbol:?} not found in rendered buffer");
    }

    /// Locates the scrollbar's track column and its top/bottom rows from the
    /// popup's rounded-border corners: chrome is border(1) + subtitle(1) +
    /// spacer(1) = 3 rows above the list, and the scrollbar sits one column
    /// in from the right border (the block's horizontal padding).
    fn scrollbar_track(buffer: &ratatui::buffer::Buffer) -> (u16, u16, u16) {
        let (_, top) = find_symbol(buffer, "\u{256d}"); // ╭
        let (right, _) = find_symbol(buffer, "\u{256e}"); // ╮
        let (_, bottom) = find_symbol(buffer, "\u{2570}"); // ╰
        let col = right - 2;
        let list_top = top + 3;
        let list_bottom = bottom - 1;
        (col, list_top, list_bottom)
    }

    const THUMB: &str = "\u{2588}"; // █ (block::FULL)

    /// At scroll offset 0 the thumb sits at the very top of the track — the
    /// inverse of the bottom-of-track proof below, pinning both ends of the
    /// travel range this bug report was about.
    #[test]
    fn help_overlay_scrollbar_thumb_is_at_track_top_when_scroll_offset_is_zero() {
        let buffer = render_help_at_scroll(0);
        let (col, list_top, list_bottom) = scrollbar_track(&buffer);

        assert_eq!(
            symbol_at(&buffer, col, list_top),
            THUMB,
            "offset 0 must put the thumb at the track's top cell"
        );
        assert_ne!(
            symbol_at(&buffer, col, list_bottom),
            THUMB,
            "offset 0 must not already show the thumb at the track's bottom cell"
        );
    }

    /// Scrolled all the way to the bottom (`G`/`End` sets `scroll` to
    /// `max_scroll`, and `render` clamps any larger value to that same
    /// bound), the scrollbar thumb must reach the last cell of the track —
    /// the bug this test was written to catch: with `ScrollbarState` built as
    /// `new(total).position(offset)`, the track is scaled to `total` while
    /// `position` can only ever reach `total - list_h`, so the thumb never
    /// reaches bottom no matter how far the offset is clamped.
    #[test]
    fn help_overlay_scrollbar_thumb_reaches_track_bottom_when_scrolled_to_the_end() {
        let buffer = render_help_at_scroll(u16::MAX);
        let (col, list_top, list_bottom) = scrollbar_track(&buffer);

        assert_eq!(
            symbol_at(&buffer, col, list_bottom),
            THUMB,
            "scrolled to the end, the thumb must reach the track's bottom cell"
        );
        assert_ne!(
            symbol_at(&buffer, col, list_top),
            THUMB,
            "scrolled to the end, the track's top cell must no longer show the thumb"
        );
    }

    // -- this_context_sections / all_keys_sections ------------------------

    fn all_rows(sections: &[Section]) -> Vec<(String, &'static str)> {
        sections
            .iter()
            .flat_map(|(_, rows)| rows.iter().cloned())
            .collect()
    }

    /// Normal/Visual origin: with nothing capability-hidden, "This context"
    /// shows every `Scope::Diff` binding plus every `Scope::Global` binding,
    /// and nothing from `Scope::Panel` — a `Panel`-only action (e.g.
    /// `RemoteFetch`) must be absent. (Capability gating itself is covered
    /// separately by `this_context_sections_apply_capability_gating`.)
    #[test]
    fn this_context_sections_for_normal_origin_is_diff_scope_plus_global() {
        let keymap = Keymap::default_map();
        let bindings = keymap.bindings();
        let sections = this_context_sections(ModeOrigin::Normal, bindings, true, true, true);
        let rows = all_rows(&sections);

        for binding in bindings
            .iter()
            .filter(|b| matches!(b.scope, Scope::Diff | Scope::Global))
        {
            assert!(
                rows.iter()
                    .any(|(k, d)| k == &binding.key_label() && *d == binding.description),
                "Normal origin must show {:?} ({})",
                binding.action,
                binding.key_label()
            );
        }
        assert!(
            bindings
                .iter()
                .filter(|b| b.scope == Scope::Panel)
                .all(|b| !rows
                    .iter()
                    .any(|(k, d)| k == &b.key_label() && *d == b.description)),
            "Normal origin must not show any Scope::Panel binding"
        );
    }

    /// Visual origin renders identically to Normal — both read the
    /// `Scope::Diff` groups per FR-2.
    #[test]
    fn this_context_sections_for_visual_origin_matches_normal() {
        let keymap = Keymap::default_map();
        let bindings = keymap.bindings();
        let normal = this_context_sections(ModeOrigin::Normal, bindings, true, true, false);
        let visual = this_context_sections(
            ModeOrigin::Visual { anchor: 3 },
            bindings,
            true,
            true,
            false,
        );
        assert_eq!(all_rows(&normal), all_rows(&visual));
    }

    /// Panel origin: "This context" shows every `Scope::Panel` binding plus
    /// every `Scope::Global` binding, and nothing from `Scope::Diff` — a
    /// diff-only action (e.g. `CursorDown`) must be absent.
    #[test]
    fn this_context_sections_for_panel_origin_is_panel_scope_plus_global() {
        let keymap = Keymap::default_map();
        let bindings = keymap.bindings();
        let origin = ModeOrigin::Panel {
            cursor: 0,
            tab: super::super::app::PanelTab::Changes,
        };
        let sections = this_context_sections(origin, bindings, true, true, false);
        let rows = all_rows(&sections);

        for binding in bindings
            .iter()
            .filter(|b| matches!(b.scope, Scope::Panel | Scope::Global))
            // Capability-hidden rows (here: the review-only accept/defer
            // rows, since the flags say "no review session") are gated out
            // of the section exactly like their diff-scope counterparts.
            .filter(|b| !binding_hidden(b.action, true, true, false))
        {
            assert!(
                rows.iter()
                    .any(|(k, d)| k == &binding.key_label() && *d == binding.description),
                "Panel origin must show {:?} ({})",
                binding.action,
                binding.key_label()
            );
        }
        assert!(
            bindings
                .iter()
                .filter(|b| b.scope == Scope::Diff)
                .all(|b| !rows
                    .iter()
                    .any(|(k, d)| k == &b.key_label() && *d == b.description)),
            "Panel origin must not show any Scope::Diff binding"
        );
    }

    /// Capability gating still applies on "This context": a hidden action
    /// (staging disallowed) is absent even though it's `Scope::Diff`.
    #[test]
    fn this_context_sections_apply_capability_gating() {
        let keymap = Keymap::default_map();
        let bindings = keymap.bindings();
        let sections = this_context_sections(ModeOrigin::Normal, bindings, false, true, false);
        let rows = all_rows(&sections);
        assert!(
            !rows
                .iter()
                .any(|(_, d)| *d == "Stage/unstage file under cursor"),
            "staging rows must be hidden when staging_allowed is false"
        );
    }

    // -- Common-workflows header: WORKFLOW_ENTRIES / workflow_rows ---------

    /// FR-8's build-breaking drift test: every curated entry's `Action` must
    /// have a binding somewhere in the *default* keymap, and the table must
    /// stay within FR-6's hard cap of 7. A curation typo (an `Action` no
    /// longer bound by default) fails this test rather than silently
    /// vanishing from the header at runtime.
    #[test]
    fn workflow_entries_are_bound_in_the_default_keymap_and_within_the_cap() {
        assert!(
            WORKFLOW_ENTRIES.len() <= 7,
            "FR-6 hard cap: at most 7 curated entries, got {}",
            WORKFLOW_ENTRIES.len()
        );
        let keymap = Keymap::default_map();
        for (phrase, action) in WORKFLOW_ENTRIES {
            assert!(
                keymap.bindings().iter().any(|b| b.action == action),
                "curated entry {phrase:?} maps to {action:?}, which has no \
                 binding in the default keymap"
            );
        }
    }

    /// `workflow_rows` resolves every curated entry, in table order, to the
    /// default keymap's key for its action — the happy path FR-7 exists for.
    #[test]
    fn workflow_rows_resolves_every_entry_in_order_for_normal_origin() {
        let keymap = Keymap::default_map();
        let rows = workflow_rows(ModeOrigin::Normal, &keymap, true, true, true);
        assert_eq!(rows.len(), WORKFLOW_ENTRIES.len());
        for (row, &(phrase, action)) in rows.iter().zip(WORKFLOW_ENTRIES.iter()) {
            assert_eq!(row.phrase, phrase);
            assert_eq!(row.key, keymap.label_for(Scope::Diff, action).unwrap());
        }
    }

    /// A `[keys.global]` remap of a curated entry's action changes the key
    /// `workflow_rows` displays — FR-7's live-resolution promise, proved
    /// against the effective (post-config-merge) keymap rather than the
    /// compiled-in default.
    #[test]
    fn workflow_rows_reflects_a_remapped_action() {
        let mut keys = crate::config::KeysConfig::default();
        keys.global.insert(
            "open-review-launcher".to_string(),
            vec![crate::config::keys::KeySeqSpec::One(
                crate::config::keys::ChordSpec {
                    code: crossterm::event::KeyCode::Char('L'),
                    mods: crossterm::event::KeyModifiers::NONE,
                },
            )],
        );
        let (keymap, warnings) = super::super::keymap_config::effective_keymap(&keys);
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        let rows = workflow_rows(ModeOrigin::Normal, &keymap, true, true, true);
        let row = rows
            .iter()
            .find(|r| r.phrase == "Review a branch or commit")
            .expect("the launcher entry must still resolve after the remap");
        assert_eq!(row.key, "L");
    }

    /// An entry whose action a user has unbound entirely (`[keys.diff]
    /// compose = []`) is omitted from the header rather than shown with a
    /// stale or blank key — FR-8's graceful-degradation half.
    #[test]
    fn workflow_rows_omits_an_entry_unbound_in_the_effective_keymap() {
        let mut keys = crate::config::KeysConfig::default();
        keys.diff.insert("compose".to_string(), Vec::new());
        let (keymap, warnings) = super::super::keymap_config::effective_keymap(&keys);
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        let rows = workflow_rows(ModeOrigin::Normal, &keymap, true, true, true);
        assert!(
            !rows.iter().any(|r| r.phrase == "Comment on a line"),
            "an unbound action's entry must not render"
        );
        // Every other curated entry is untouched by this one unbind.
        assert_eq!(rows.len(), WORKFLOW_ENTRIES.len() - 1);
    }

    /// Capability gating hides a header entry exactly like a regular help
    /// row (FR-9): staging disallowed on a read-only target hides "Stage
    /// the change under the cursor" (`Action::ToggleStage`), even though
    /// it's bound.
    #[test]
    fn workflow_rows_hides_a_capability_gated_entry() {
        let keymap = Keymap::default_map();
        let rows = workflow_rows(ModeOrigin::Normal, &keymap, false, true, true);
        assert!(
            !rows
                .iter()
                .any(|r| r.phrase == "Stage the change under the cursor"),
            "staging entry must be hidden when staging_allowed is false"
        );
        assert_eq!(rows.len(), WORKFLOW_ENTRIES.len() - 1);
    }

    /// Panel origin resolves in `Scope::Panel`: the Diff-only `Compose`
    /// entry has no Panel or Global binding, so it drops out entirely, while
    /// the launcher/quit (`Scope::Global`), the stage entry (Space stages
    /// the highlighted file in panel scope too), and — since spec 11 Unit 2
    /// bound panel `/` to `Search` — the search entry all still resolve —
    /// the header never claims a key means something it doesn't in the
    /// panel (plain `c` commits staged changes there, it doesn't open
    /// Compose).
    #[test]
    fn workflow_rows_for_panel_origin_keeps_panel_resolvable_entries() {
        let keymap = Keymap::default_map();
        let origin = ModeOrigin::Panel {
            cursor: 0,
            tab: super::super::app::PanelTab::Changes,
        };
        let rows = workflow_rows(origin, &keymap, true, true, true);
        let phrases: Vec<&str> = rows.iter().map(|r| r.phrase).collect();
        assert_eq!(
            phrases,
            vec![
                "Review a branch or commit",
                "Stage the change under the cursor",
                "Search the diff",
                "Quit and copy annotations"
            ]
        );
    }

    /// "All keys" is a strict superset of "This context" for any origin —
    /// every row the tab shows is also somewhere on the full reference.
    #[test]
    fn all_keys_sections_is_a_superset_of_this_context_sections() {
        let keymap = Keymap::default_map();
        let modal_keys = ModalKeymaps::default();
        let bindings = keymap.bindings();
        let all_keys_rows = all_rows(&all_keys_sections(bindings, &modal_keys, true, true, true));
        for origin in [
            ModeOrigin::Normal,
            ModeOrigin::Panel {
                cursor: 0,
                tab: super::super::app::PanelTab::Changes,
            },
        ] {
            let context_rows = all_rows(&this_context_sections(origin, bindings, true, true, true));
            for (k, d) in &context_rows {
                assert!(
                    all_keys_rows.iter().any(|(ak, ad)| ak == k && ad == d),
                    "This context row {k:?} ({d:?}) for {origin:?} must also appear on All keys"
                );
            }
        }
    }
}
