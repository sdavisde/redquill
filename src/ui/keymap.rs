//! The keymap: data, not scattered match arms. [`Action`] is what the rest
//! of the UI reacts to; [`Binding`] pairs a key *sequence* (one or two
//! keys — `gd`/`gr` are the only two-key sequences today) with an [`Action`]
//! and a human-readable description; [`Keymap`] is the lookup table. The
//! help overlay ([`super::help`]) renders directly from [`Keymap::bindings`],
//! so this table is the single source of truth for both dispatch and
//! documentation.
//!
//! Single-key bindings resolve in one call to [`Keymap::lookup`], unchanged
//! from before two-key sequences existed. Two-key sequences need a second
//! event to complete, so the event loop tracks a pending prefix key across
//! calls and resolves it via [`Keymap::resolve`] (see that method's docs).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Every user-visible action reachable from the keymap.
///
/// `Quit` and `QuitDiscard` are intercepted by the event loop before
/// reaching [`super::app::App::apply`] (they end the session rather than
/// mutate state), but they still need table entries so the help overlay
/// documents them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Move the cursor down one row.
    CursorDown,
    /// Move the cursor up one row.
    CursorUp,
    /// Move the column cursor left within the cursor row's content.
    CursorLeft,
    /// Move the column cursor right within the cursor row's content.
    CursorRight,
    /// Move the column cursor to the start of the cursor row's content.
    CursorLineStart,
    /// Move the column cursor to the last character of the cursor row's
    /// content.
    CursorLineEnd,
    /// Jump the column cursor to the start of the next word.
    WordForward,
    /// Jump the column cursor to the start of the previous word.
    WordBackward,
    /// Move the cursor down half a viewport.
    HalfPageDown,
    /// Move the cursor up half a viewport.
    HalfPageUp,
    /// Move the cursor down a full viewport.
    FullPageDown,
    /// Move the cursor up a full viewport.
    FullPageUp,
    /// Jump the cursor to the top of the diff buffer.
    JumpToTop,
    /// Jump the cursor to the bottom of the diff buffer.
    JumpToBottom,
    /// Jump to the next hunk, crossing file boundaries if needed.
    NextHunk,
    /// Jump to the previous hunk, crossing file boundaries if needed.
    PrevHunk,
    /// Jump the cursor to the next file's section header.
    NextFile,
    /// Jump the cursor to the previous file's section header.
    PrevFile,
    /// Toggle the collapse state of the file section under the cursor.
    ToggleCollapse,
    /// Scroll the viewport so the cursor sits at its vertical center.
    RecenterCursor,
    /// Scroll the viewport so the cursor sits near its top.
    ScrollCursorTop,
    /// Scroll the viewport so the cursor sits near its bottom.
    ScrollCursorBottom,
    /// Toggle the help overlay.
    ToggleHelp,
    /// Enter Visual mode at the cursor row (Normal), or cancel Visual mode
    /// back to Normal (Visual). No-op on non-line rows in Normal mode.
    EnterVisual,
    /// Open the Compose modal: on the cursor row's target (Normal), or on
    /// the current Visual selection's range (Visual).
    Compose,
    /// Toggle the annotation list panel.
    ToggleList,
    /// Stage/unstage at the cursor: the enclosing hunk on line/hunk rows,
    /// the whole file on file-header/binary rows, the selected lines in
    /// Visual mode. Stages on the working-tree target, unstages on the
    /// staged target; a no-op with a message on read-only range targets.
    ToggleStage,
    /// Stage the whole file under the cursor (auto-collapsing its section),
    /// or unstage it when fully staged (auto-expanding). Stages on the
    /// working-tree target; a no-op with a message on read-only ranges.
    StageFile,
    /// Toggle the staging panel (files with staged changes).
    ToggleStagingPanel,
    /// Open the search input, composing a pattern to match against the
    /// current file's line content and hunk-header section text.
    Search,
    /// Jump to the next search match, wrapping around.
    SearchNext,
    /// Jump to the previous search match, wrapping around.
    SearchPrev,
    /// Search for the word under the column cursor, jumping to the next
    /// occurrence.
    SearchWordForward,
    /// Search for the word under the column cursor, jumping to the previous
    /// occurrence.
    SearchWordBackward,
    /// Request `textDocument/definition` for the cursor's position.
    GotoDefinition,
    /// Request `textDocument/references` for the cursor's position.
    GotoReferences,
    /// Request `textDocument/hover` for the cursor's position.
    Hover,
    /// Toggle focus between the diff view and the git panel.
    FocusGitPanel,
    /// Move the git panel's cursor down one navigable row (panel scope).
    PanelCursorDown,
    /// Move the git panel's cursor up one navigable row (panel scope).
    PanelCursorUp,
    /// Open the git panel cursor's file in the diff and return focus to it;
    /// a no-op on stash/header rows (panel scope). On the History tab,
    /// opens the highlighted commit into the main diff view instead.
    PanelSelect,
    /// Toggles the git panel between its Changes and History tabs (panel
    /// scope).
    TogglePanelTab,
    /// Fetch from the upstream remote on a background thread (panel scope).
    RemoteFetch,
    /// Pull from the upstream remote on a background thread (panel scope).
    RemotePull,
    /// Push to the upstream remote on a background thread (panel scope).
    RemotePush,
    /// Open the commit-message modal for the staged changes (panel scope);
    /// a footer message when nothing is staged.
    CommitStaged,
    /// Open the branch/worktree switcher modal (panel scope).
    OpenSwitcher,
    /// Open the Review launcher modal (`R`, works everywhere —
    /// [`Scope::Global`]): a tabbed overlay hosting branch review and
    /// single-commit review behind one entry point (see
    /// [`super::review_launcher::LauncherTab`]), the sole in-app entry point
    /// for starting a branch review.
    OpenReviewLauncher,
    /// Open the fuzzy file finder overlay (`gp`, diff scope).
    OpenFileFinder,
    /// Open the full-screen Project Search view (`g/`, diff scope).
    OpenProjectSearch,
    /// Suspend the TUI and open the configured editor on the file under the
    /// cursor at the cursor's line (`g<Space>`, diff scope). Intercepted in
    /// [`super::dispatch_key`] like `Quit`/`QuitDiscard` — the actual
    /// suspend/spawn/resume dance lives in the event loop, not
    /// [`super::app::App::apply`].
    OpenEditor,
    /// Toggle the command-log pane (bound in both scopes).
    ToggleCommandLog,
    /// Re-read the working tree and rebuild the diff, picking up edits made
    /// outside redquill (e.g. by an agent) since the last refresh (`r`, diff
    /// scope).
    Refresh,
    /// Quit, emitting annotations to stdout.
    Quit,
    /// Quit, discarding annotations.
    QuitDiscard,
    /// Dismiss the config-warning status-line notice, if one is showing.
    /// Bound in both scopes since the notice can be visible whether or not
    /// the git panel is focused.
    DismissConfigWarning,
    /// `Space` means accept in a review session; `super::dispatch_key`
    /// translates the resolved [`Action::ToggleStage`] into this action only
    /// while `App::in_review_session()` holds, so this variant is never
    /// produced by a plain keymap lookup.
    ToggleAccept,
    /// `S` in a review session: accepts the cursor file unconditionally from
    /// anywhere inside it (see [`crate::review::accept`]), mirroring
    /// [`Action::StageFile`]'s "works from anywhere" gesture. Reached via
    /// the same dispatch-time translation as [`Action::ToggleAccept`].
    AcceptFile,
    /// `d` in a review session: toggles the cursor file between `Deferred`
    /// and `Unreviewed` (see [`crate::review::toggle_defer`]). Unlike the
    /// two actions above, this is bound directly in [`Scope::Diff`]; its
    /// handler self-guards on `App::in_review_session()`, so outside a
    /// review session `d` stays a total no-op.
    ToggleDefer,
}

/// The kebab-case config action-name for every [`Action`] variant (the
/// `[keys.diff]`/`[keys.panel]` left-hand side). An exhaustive
/// match: a new `Action` variant fails to *compile* here until it's named,
/// which is a stronger guarantee than a test — see
/// [`tests::action_names_are_total_and_bijective`] for the runtime half
/// (uniqueness, and that every name resolves back via [`action_from_name`]).
pub(crate) fn action_name(action: Action) -> &'static str {
    use Action::*;
    match action {
        CursorDown => "cursor-down",
        CursorUp => "cursor-up",
        CursorLeft => "cursor-left",
        CursorRight => "cursor-right",
        CursorLineStart => "cursor-line-start",
        CursorLineEnd => "cursor-line-end",
        WordForward => "word-forward",
        WordBackward => "word-backward",
        HalfPageDown => "half-page-down",
        HalfPageUp => "half-page-up",
        FullPageDown => "full-page-down",
        FullPageUp => "full-page-up",
        JumpToTop => "jump-to-top",
        JumpToBottom => "jump-to-bottom",
        NextHunk => "next-hunk",
        PrevHunk => "prev-hunk",
        NextFile => "next-file",
        PrevFile => "prev-file",
        ToggleCollapse => "toggle-collapse",
        RecenterCursor => "recenter-cursor",
        ScrollCursorTop => "scroll-cursor-top",
        ScrollCursorBottom => "scroll-cursor-bottom",
        ToggleHelp => "toggle-help",
        EnterVisual => "enter-visual",
        Compose => "compose",
        ToggleList => "toggle-list",
        ToggleStage => "toggle-stage",
        StageFile => "stage-file",
        ToggleStagingPanel => "toggle-staging-panel",
        Search => "search",
        SearchNext => "search-next",
        SearchPrev => "search-prev",
        SearchWordForward => "search-word-forward",
        SearchWordBackward => "search-word-backward",
        GotoDefinition => "goto-definition",
        GotoReferences => "goto-references",
        Hover => "hover",
        FocusGitPanel => "focus-git-panel",
        PanelCursorDown => "panel-cursor-down",
        PanelCursorUp => "panel-cursor-up",
        PanelSelect => "panel-select",
        TogglePanelTab => "toggle-panel-tab",
        RemoteFetch => "remote-fetch",
        RemotePull => "remote-pull",
        RemotePush => "remote-push",
        CommitStaged => "commit-staged",
        OpenSwitcher => "open-switcher",
        OpenReviewLauncher => "open-review-launcher",
        OpenFileFinder => "open-file-finder",
        OpenProjectSearch => "open-project-search",
        OpenEditor => "open-editor",
        ToggleCommandLog => "toggle-command-log",
        Refresh => "refresh",
        Quit => "quit",
        QuitDiscard => "quit-discard",
        DismissConfigWarning => "dismiss-config-warning",
        ToggleAccept => "toggle-accept",
        AcceptFile => "accept-file",
        ToggleDefer => "toggle-defer",
    }
}

/// Reverse of [`action_name`]: resolves a config-file action-name string
/// back to the [`Action`] it names, or `None` for an unrecognized string (an
/// [`super::super::config::ConfigWarning::InvalidValue`]-worthy "unknown
/// action name" at the config edge — see `super::keymap_config`).
pub(crate) fn action_from_name(name: &str) -> Option<Action> {
    use Action::*;
    Some(match name {
        "cursor-down" => CursorDown,
        "cursor-up" => CursorUp,
        "cursor-left" => CursorLeft,
        "cursor-right" => CursorRight,
        "cursor-line-start" => CursorLineStart,
        "cursor-line-end" => CursorLineEnd,
        "word-forward" => WordForward,
        "word-backward" => WordBackward,
        "half-page-down" => HalfPageDown,
        "half-page-up" => HalfPageUp,
        "full-page-down" => FullPageDown,
        "full-page-up" => FullPageUp,
        "jump-to-top" => JumpToTop,
        "jump-to-bottom" => JumpToBottom,
        "next-hunk" => NextHunk,
        "prev-hunk" => PrevHunk,
        "next-file" => NextFile,
        "prev-file" => PrevFile,
        "toggle-collapse" => ToggleCollapse,
        "recenter-cursor" => RecenterCursor,
        "scroll-cursor-top" => ScrollCursorTop,
        "scroll-cursor-bottom" => ScrollCursorBottom,
        "toggle-help" => ToggleHelp,
        "enter-visual" => EnterVisual,
        "compose" => Compose,
        "toggle-list" => ToggleList,
        "toggle-stage" => ToggleStage,
        "stage-file" => StageFile,
        "toggle-staging-panel" => ToggleStagingPanel,
        "search" => Search,
        "search-next" => SearchNext,
        "search-prev" => SearchPrev,
        "search-word-forward" => SearchWordForward,
        "search-word-backward" => SearchWordBackward,
        "goto-definition" => GotoDefinition,
        "goto-references" => GotoReferences,
        "hover" => Hover,
        "focus-git-panel" => FocusGitPanel,
        "panel-cursor-down" => PanelCursorDown,
        "panel-cursor-up" => PanelCursorUp,
        "panel-select" => PanelSelect,
        "toggle-panel-tab" => TogglePanelTab,
        "remote-fetch" => RemoteFetch,
        "remote-pull" => RemotePull,
        "remote-push" => RemotePush,
        "commit-staged" => CommitStaged,
        "open-switcher" => OpenSwitcher,
        "open-review-launcher" => OpenReviewLauncher,
        "open-file-finder" => OpenFileFinder,
        "open-project-search" => OpenProjectSearch,
        "open-editor" => OpenEditor,
        "toggle-command-log" => ToggleCommandLog,
        "refresh" => Refresh,
        "quit" => Quit,
        "quit-discard" => QuitDiscard,
        "dismiss-config-warning" => DismissConfigWarning,
        "toggle-accept" => ToggleAccept,
        "accept-file" => AcceptFile,
        "toggle-defer" => ToggleDefer,
        _ => return None,
    })
}

/// One key chord: a code plus its required modifiers, matched against an
/// incoming [`KeyEvent`] with `SHIFT` stripped whenever the code itself
/// already encodes shift (an uppercase char, a shifted punctuation char, or
/// `BackTab`) — terminals are inconsistent about whether they also set the
/// `SHIFT` bit in that situation, so chords for those keys are defined
/// without `SHIFT` and matching stays terminal-agnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyChord {
    code: KeyCode,
    mods: KeyModifiers,
}

impl KeyChord {
    fn new(code: KeyCode, mods: KeyModifiers) -> KeyChord {
        KeyChord { code, mods }
    }

    fn matches(self, key: KeyEvent) -> bool {
        let mut mods = key.modifiers;
        if matches!(key.code, KeyCode::Char(_) | KeyCode::BackTab) {
            mods.remove(KeyModifiers::SHIFT);
        }
        self.code == key.code && self.mods == mods
    }

    /// A display label for this chord, e.g. `"Ctrl-d"`, `"Shift-Tab"`, `"g"`.
    fn label(self) -> String {
        let mut label = String::new();
        if self.mods.contains(KeyModifiers::CONTROL) {
            label.push_str("Ctrl-");
        }
        if self.mods.contains(KeyModifiers::ALT) {
            label.push_str("Alt-");
        }
        match self.code {
            KeyCode::Char(' ') => label.push_str("Space"),
            KeyCode::Char(c) => label.push(c),
            KeyCode::Tab => label.push_str("Tab"),
            KeyCode::BackTab => label.push_str("Shift-Tab"),
            KeyCode::Esc => label.push_str("Esc"),
            KeyCode::Enter => label.push_str("Enter"),
            other => label.push_str(&format!("{other:?}")),
        }
        label
    }
}

/// The input context a [`Binding`] resolves in. Every binding that existed
/// before the git panel is [`Scope::Diff`]; panel-focused navigation lives in
/// [`Scope::Panel`]. Resolution filters by scope so the same physical key
/// (`j`, `` ` ``) can mean different things depending on which pane is
/// focused, and so the focus toggle is bindable in both directions.
///
/// [`Scope::Global`] is a third, orthogonal scope for "works
/// everywhere" keys (`?` help, `@` command log, `!` dismiss warning, the
/// quit family): rather than duplicating one row per scope, a `Global`
/// binding is defined once and consulted from both [`Scope::Diff`] and
/// [`Scope::Panel`] queries as a fallback *after* that scope's own rows —
/// see [`Keymap::scope_chain`], the single place this resolution order is
/// expressed. A scope-specific row always shadows a `Global` row bound to
/// the same key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// The diff view (Normal/Visual): every pre-existing binding.
    Diff,
    /// The git panel while it holds focus.
    Panel,
    /// Bindings that resolve identically from every table-driven scope.
    /// Consulted as a fallback after the active scope's own rows — see the
    /// enum doc and [`Keymap::scope_chain`].
    Global,
}

/// The key sequence a [`Binding`] triggers on: one key (every binding
/// before `gd`/`gr` existed) or two (a `g`-prefixed sequence).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySeq {
    One(KeyChord),
    Two(KeyChord, KeyChord),
}

impl KeySeq {
    fn one(code: KeyCode, mods: KeyModifiers) -> KeySeq {
        KeySeq::One(KeyChord::new(code, mods))
    }

    fn two(code1: KeyCode, mods1: KeyModifiers, code2: KeyCode, mods2: KeyModifiers) -> KeySeq {
        KeySeq::Two(KeyChord::new(code1, mods1), KeyChord::new(code2, mods2))
    }

    /// Builds a runtime [`KeySeq`] from a parsed grammar spec
    /// (`crate::config::keys::KeySeqSpec`) — the one place the config-side
    /// key-string grammar's plain chord data becomes the real
    /// `KeyChord`/`KeySeq` representation. Used only by
    /// [`super::keymap_config`], the edge module that merges
    /// `[keys.diff]`/`[keys.panel]` overrides onto `Keymap::default_map()`.
    pub(crate) fn from_spec(spec: crate::config::keys::KeySeqSpec) -> KeySeq {
        use crate::config::keys::KeySeqSpec;
        match spec {
            KeySeqSpec::One(c) => KeySeq::one(c.code, c.mods),
            KeySeqSpec::Two(a, b) => KeySeq::two(a.code, a.mods, b.code, b.mods),
        }
    }
}

/// A display label for a bare key sequence (no action attached) — the same
/// rendering [`Binding::key_label`] uses, factored out so
/// [`super::keymap_config`]'s collision-warning text can label a key
/// without constructing a throwaway [`Binding`].
pub(crate) fn key_seq_label(seq: KeySeq) -> String {
    match seq {
        KeySeq::One(chord) => chord.label(),
        KeySeq::Two(first, second) => format!("{}{}", first.label(), second.label()),
    }
}

/// Presence promotes a [`Binding`] (or a [`super::modal_keys::ModalBinding`])
/// into the context-sensitive footer strip built by [`super::footer`]. Two
/// rows sharing an identical `FooterHint` (same `rank` *and* `label`) are
/// merged into one hint whose key text joins both rows' key labels with `/`
/// (e.g. `j` + `k`, both tagged `(1, "move")`, become `j/k move`) — the
/// mechanism [`super::footer`] uses for every "j/k" pairing rather than
/// hardcoding the merge per mode.
///
/// `rank` only controls *drop order* under narrow-width pressure, not screen
/// position: a strip always renders in the order its hints were built, and
/// `rank == 0` is the sole exception — reserved for the `? help` escape
/// hatch, it is displayed last (after every other hint) but is the last
/// dropped when hints don't fit (see `super::footer::sort_for_display`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FooterHint {
    /// Drop priority under width pressure: higher drops first. `0` is
    /// reserved for the always-last-dropped `? help` hint.
    pub rank: u8,
    /// The short label shown in the footer strip (distinct from
    /// `description`, which is verbose enough for the help overlay but too
    /// long for a one-line strip).
    pub label: &'static str,
}

/// One entry in the keymap: a key sequence, the action it triggers, and its
/// description for the help overlay.
#[derive(Debug, Clone, Copy)]
pub struct Binding {
    /// The key sequence that triggers this binding.
    pub keys: KeySeq,
    /// The action this binding triggers.
    pub action: Action,
    /// Human-readable description shown in the help overlay.
    pub description: &'static str,
    /// The input context this binding resolves in (diff view vs. focused
    /// git panel).
    pub scope: Scope,
    /// `Some` promotes this row into [`super::footer`]'s context-sensitive
    /// footer strip; `None` (the default) keeps it help-overlay-only. Set via
    /// [`Binding::footer`].
    pub footer: Option<FooterHint>,
}

impl Binding {
    /// A display label for the key sequence, e.g. `"Ctrl-d"`, `"gd"`.
    pub fn key_label(&self) -> String {
        key_seq_label(self.keys)
    }

    /// Promotes this row into the footer strip (see [`FooterHint`]).
    /// Chainable onto the `d(...)`/`p(...)` table constructors so promoted
    /// rows read as `d(keys, action, description).footer(rank, "label")`.
    pub fn footer(mut self, rank: u8, label: &'static str) -> Binding {
        self.footer = Some(FooterHint { rank, label });
        self
    }
}

/// The keybinding table: a flat list of [`Binding`]s, looked up by key
/// sequence. Remappable in principle (a future config layer would build a
/// different `Vec<Binding>`), but only [`Keymap::default_map`] exists today.
#[derive(Debug, Clone)]
pub struct Keymap {
    bindings: Vec<Binding>,
}

impl Keymap {
    /// The default keymap, matching the README's binding table.
    pub fn default_map() -> Keymap {
        use Action::*;
        use KeyCode::*;
        let none = KeyModifiers::NONE;
        let ctrl = KeyModifiers::CONTROL;
        // Diff- and panel-scope binding constructors, so the table below
        // reads as data and every entry declares its scope by construction.
        let d = |keys: KeySeq, action: Action, description: &'static str| Binding {
            keys,
            action,
            description,
            scope: Scope::Diff,
            footer: None,
        };
        let p = |keys: KeySeq, action: Action, description: &'static str| Binding {
            keys,
            action,
            description,
            scope: Scope::Panel,
            footer: None,
        };
        // "Works everywhere" rows: resolved from both `Diff` and `Panel`
        // (see `Scope::Global`/`Keymap::scope_chain`), defined once instead
        // of duplicated per scope.
        let g = |keys: KeySeq, action: Action, description: &'static str| Binding {
            keys,
            action,
            description,
            scope: Scope::Global,
            footer: None,
        };
        Keymap {
            bindings: vec![
                d(KeySeq::one(Char('j'), none), CursorDown, "Move cursor down").footer(1, "move"),
                d(KeySeq::one(Char('k'), none), CursorUp, "Move cursor up").footer(1, "move"),
                d(
                    KeySeq::one(Char('h'), none),
                    CursorLeft,
                    "Move column cursor left",
                ),
                d(
                    KeySeq::one(Char('l'), none),
                    CursorRight,
                    "Move column cursor right",
                ),
                d(
                    KeySeq::one(Char('0'), none),
                    CursorLineStart,
                    "Move column cursor to start of line",
                ),
                d(
                    KeySeq::one(Char('$'), none),
                    CursorLineEnd,
                    "Move column cursor to end of line",
                ),
                d(
                    KeySeq::one(Char('w'), none),
                    WordForward,
                    "Jump column cursor to next word",
                ),
                d(
                    KeySeq::one(Char('b'), none),
                    WordBackward,
                    "Jump column cursor to previous word",
                ),
                d(
                    KeySeq::one(Char('d'), ctrl),
                    HalfPageDown,
                    "Scroll half page down",
                ),
                d(
                    KeySeq::one(Char('u'), ctrl),
                    HalfPageUp,
                    "Scroll half page up",
                ),
                d(
                    KeySeq::one(Char('f'), ctrl),
                    FullPageDown,
                    "Scroll full page down",
                ),
                d(
                    KeySeq::one(Char('b'), ctrl),
                    FullPageUp,
                    "Scroll full page up",
                ),
                d(
                    KeySeq::two(Char('g'), none, Char('g'), none),
                    JumpToTop,
                    "Jump to top of diff",
                ),
                d(
                    KeySeq::one(Char('G'), none),
                    JumpToBottom,
                    "Jump to bottom of diff",
                ),
                d(KeySeq::one(Char(']'), none), NextHunk, "Next hunk").footer(2, "hunk"),
                d(KeySeq::one(Char('['), none), PrevHunk, "Previous hunk"),
                d(KeySeq::one(Tab, none), NextFile, "Next file section"),
                d(
                    KeySeq::one(BackTab, none),
                    PrevFile,
                    "Previous file section",
                ),
                d(
                    KeySeq::two(Char('z'), none, Char('a'), none),
                    ToggleCollapse,
                    "Collapse/expand file section",
                )
                .footer(3, "fold"),
                d(
                    KeySeq::two(Char('z'), none, Char('z'), none),
                    RecenterCursor,
                    "Center viewport on cursor",
                ),
                d(
                    KeySeq::two(Char('z'), none, Char('t'), none),
                    ScrollCursorTop,
                    "Scroll cursor to top of viewport",
                ),
                d(
                    KeySeq::two(Char('z'), none, Char('b'), none),
                    ScrollCursorBottom,
                    "Scroll cursor to bottom of viewport",
                ),
                // `?` itself is bound in `Scope::Global` (see the block at
                // the end of this table) — it's a "works everywhere" key,
                // not diff-specific. This row's `Action` (`ToggleHelp`) only
                // tells the table Esc is *bound*, so the overlay lists it;
                // the actual dispatch is a hand-written cascade in
                // `mod.rs`'s `dispatch_key` (close help / cancel a Visual
                // selection / return from a commit view opened via the
                // History tab) — the same multi-duty-single-key pattern
                // Visual-cancel uses.
                d(
                    KeySeq::one(Esc, none),
                    ToggleHelp,
                    "Close help / cancel selection / return from a commit view",
                ),
                d(
                    KeySeq::one(Char('v'), none),
                    EnterVisual,
                    "Enter visual selection / cancel",
                ),
                d(
                    KeySeq::one(Char('c'), none),
                    Compose,
                    "Comment on line/hunk/file (or visual selection)",
                )
                .footer(6, "comment"),
                d(
                    KeySeq::one(Char('a'), none),
                    ToggleList,
                    "Toggle annotation list panel",
                ),
                d(
                    KeySeq::one(Char(' '), none),
                    ToggleStage,
                    "Stage/unstage hunk (lines in visual mode)",
                )
                .footer(4, "stage hunk"),
                d(
                    KeySeq::one(Char('S'), none),
                    StageFile,
                    "Stage/unstage file under cursor",
                )
                .footer(5, "stage file"),
                d(
                    KeySeq::one(Char('s'), none),
                    ToggleStagingPanel,
                    "Toggle staging panel",
                ),
                // These rows exist so the help overlay/footer can document
                // Space/`S`'s review-session meaning; dispatch translates
                // them (see dispatch_key). `ToggleDefer` is bound directly.
                d(
                    KeySeq::one(Char(' '), none),
                    ToggleAccept,
                    "Accept/un-accept file under cursor",
                )
                .footer(4, "accept"),
                d(
                    KeySeq::one(Char('S'), none),
                    AcceptFile,
                    "Accept file under cursor",
                )
                .footer(5, "accept file"),
                d(
                    KeySeq::one(Char('d'), none),
                    ToggleDefer,
                    "Defer/un-defer file under cursor",
                )
                .footer(6, "defer"),
                d(
                    KeySeq::one(Char('`'), none),
                    FocusGitPanel,
                    "Open git panel",
                )
                .footer(8, "git panel"),
                // `@` and `!` are bound in `Scope::Global` (see the block at
                // the end of this table) — both are "works everywhere" keys,
                // not diff-specific. `R` (uppercase) is Global too now — the
                // Review launcher — so `Refresh` moved to lowercase `r` to
                // free it up.
                d(
                    KeySeq::one(Char('r'), none),
                    Refresh,
                    "Refresh diff from working tree",
                ),
                d(KeySeq::one(Char('/'), none), Search, "Search").footer(7, "search"),
                d(
                    KeySeq::one(Char('n'), none),
                    SearchNext,
                    "Next search match",
                ),
                d(
                    KeySeq::one(Char('N'), none),
                    SearchPrev,
                    "Previous search match",
                ),
                d(
                    KeySeq::one(Char('*'), none),
                    SearchWordForward,
                    "Search word under cursor, next occurrence",
                ),
                d(
                    KeySeq::one(Char('#'), none),
                    SearchWordBackward,
                    "Search word under cursor, previous occurrence",
                ),
                d(
                    KeySeq::two(Char('g'), none, Char('d'), none),
                    GotoDefinition,
                    "Go to definition",
                ),
                d(
                    KeySeq::two(Char('g'), none, Char('r'), none),
                    GotoReferences,
                    "Find references",
                ),
                d(
                    KeySeq::two(Char('g'), none, Char('p'), none),
                    OpenFileFinder,
                    "Open fuzzy file finder",
                ),
                d(
                    KeySeq::two(Char('g'), none, Char('/'), none),
                    OpenProjectSearch,
                    "Open project search",
                ),
                d(
                    KeySeq::two(Char('g'), none, Char(' '), none),
                    OpenEditor,
                    "Open file at cursor in editor",
                ),
                d(KeySeq::one(Char('K'), none), Hover, "Hover docs"),
                // The quit family (`q`/`Q`/Ctrl-C) is bound in `Scope::Global`
                // (see the block at the end of this table): the focused git
                // panel is a first-class view, not an overlay, so quitting
                // must work identically from both it and the diff view —
                // one row per key rather than a duplicate per scope.
                // -- Panel scope: resolved only while the git panel is focused.
                p(
                    KeySeq::one(Char('`'), none),
                    FocusGitPanel,
                    "Close git panel",
                )
                .footer(7, "close"),
                p(
                    KeySeq::one(Char('j'), none),
                    PanelCursorDown,
                    "Move panel cursor down",
                )
                .footer(1, "move"),
                p(
                    KeySeq::one(Char('k'), none),
                    PanelCursorUp,
                    "Move panel cursor up",
                )
                .footer(1, "move"),
                p(
                    KeySeq::one(Enter, none),
                    PanelSelect,
                    "Open file / fold directory (History tab: open the commit)",
                )
                .footer(2, "open"),
                p(
                    KeySeq::one(Tab, none),
                    TogglePanelTab,
                    "Switch Changes / History tab",
                )
                .footer(8, "tab"),
                p(
                    KeySeq::one(Char('f'), none),
                    RemoteFetch,
                    "Fetch from remote",
                )
                .footer(3, "fetch"),
                p(KeySeq::one(Char('p'), none), RemotePull, "Pull from remote").footer(4, "pull"),
                p(
                    KeySeq::one(Char('P'), none),
                    RemotePush,
                    "Push to remote (publishes an unpublished branch)",
                )
                .footer(5, "push"),
                // Plain `c` is free in panel scope: `Compose` binds it in
                // diff scope only, so the same physical key can commit here
                // without touching the annotate gesture.
                p(
                    KeySeq::one(Char('c'), none),
                    CommitStaged,
                    "Commit staged changes",
                )
                .footer(6, "commit"),
                // `?`/`@`/`!`/the quit family are bound in `Scope::Global`
                // (see the block below) — the panel's footer strip can still
                // promise a working `? help` escape hatch (see
                // `super::footer`), it's just sourced from the Global row
                // rather than a panel-scope duplicate.
                p(
                    KeySeq::one(Char('b'), none),
                    OpenSwitcher,
                    "Open branch/worktree switcher",
                ),
                // -- Global scope: resolved from both Diff and Panel (see
                // `Keymap::scope_chain`) — "works everywhere" keys defined
                // once rather than duplicated per scope.
                // The focused git panel is a first-class view, not an
                // overlay, so the quit family works here exactly as in the
                // diff view.
                g(KeySeq::one(Char('?'), none), ToggleHelp, "Toggle help").footer(0, "help"),
                // Supersedes the old panel-only review-branch entry (`R`,
                // panel scope) — reachable from anywhere now, not just the
                // focused git panel.
                g(
                    KeySeq::one(Char('R'), none),
                    OpenReviewLauncher,
                    "Open Review launcher (branches / commits)",
                ),
                g(
                    KeySeq::one(Char('@'), none),
                    ToggleCommandLog,
                    "Toggle command log pane",
                ),
                g(
                    KeySeq::one(Char('!'), none),
                    DismissConfigWarning,
                    "Dismiss config warning notice",
                ),
                g(
                    KeySeq::one(Char('q'), none),
                    Quit,
                    "Quit and emit annotations",
                ),
                g(
                    KeySeq::one(Char('Q'), none),
                    QuitDiscard,
                    "Quit and discard annotations",
                ),
                g(
                    KeySeq::one(Char('c'), ctrl),
                    QuitDiscard,
                    "Quit and discard annotations",
                ),
            ],
        }
    }

    /// Builds a [`Keymap`] from an explicit binding list — the constructor
    /// `super::keymap_config`'s merge machinery uses to assemble the
    /// effective (post-config-override) table from `default_map`'s
    /// bindings (see that module for the merge semantics).
    pub(crate) fn from_bindings(bindings: Vec<Binding>) -> Keymap {
        Keymap { bindings }
    }

    /// All bindings, in table order — what the help overlay iterates.
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// The scopes consulted, in priority order, for a `scope`-parameterized
    /// query (`lookup_in`, `starts_sequence_in`, `completions_for`,
    /// `label_for`; `resolve_in` inherits it by composing the first two):
    /// `scope` itself first — so a scope-specific row shadows a `Global` row
    /// bound to the same key — then [`Scope::Global`] as a fallback. A
    /// direct query *of* [`Scope::Global`] has nothing broader to fall back
    /// to, so it searches only itself. This is the single place the
    /// "active scope, then Global" rule is expressed; every
    /// method below composes through it so the rule can't drift between
    /// them. A fixed-size array (not a `Vec`) so the per-keystroke hot path
    /// stays allocation-free.
    fn scope_chain(scope: Scope) -> [Option<Scope>; 2] {
        if scope == Scope::Global {
            [Some(Scope::Global), None]
        } else {
            [Some(scope), Some(Scope::Global)]
        }
    }

    /// The display label of the first binding for `action` within `scope`,
    /// falling back to [`Scope::Global`] (e.g. `"Space"` for
    /// [`Action::ToggleStage`] in [`Scope::Diff`]), or `None` if `action`
    /// isn't bound there at all (a config override unbound it, or the
    /// action was never bound in that scope or `Global` to begin with).
    /// Every place that spells a main-keymap action's key out in prose —
    /// rather than rendering the [`Keymap`] table directly, like the help
    /// overlay and footer already do — reads through this, so a remap or
    /// unbind can never leave stale key text on screen: see
    /// `super::welcome`'s next-step hints, `super::git_panel`'s remote-op
    /// line, and the staging/list panels' empty-state hints.
    pub(crate) fn label_for(&self, scope: Scope, action: Action) -> Option<String> {
        Self::scope_chain(scope)
            .into_iter()
            .flatten()
            .find_map(|s| {
                self.bindings
                    .iter()
                    .find(|b| b.scope == s && b.action == action)
                    .map(Binding::key_label)
            })
    }

    /// Resolves a single key event to an [`Action`] in [`Scope::Diff`],
    /// matching only [`KeySeq::One`] bindings — unchanged behavior from
    /// before scopes existed (every pre-existing binding is diff-scope).
    /// Two-key sequences (`gd`, `gr`) can't be resolved from one event; see
    /// [`Keymap::resolve`].
    pub fn lookup(&self, key: KeyEvent) -> Option<Action> {
        self.lookup_in(Scope::Diff, key)
    }

    /// Resolves a single key event within `scope`, falling back to
    /// [`Scope::Global`] when `scope` has no row for `key` (see
    /// [`Keymap::scope_chain`]) — a scope-specific binding always shadows a
    /// `Global` one bound to the same key, so the same physical key can
    /// still mean different things depending on which pane is focused.
    pub fn lookup_in(&self, scope: Scope, key: KeyEvent) -> Option<Action> {
        Self::scope_chain(scope)
            .into_iter()
            .flatten()
            .find_map(|s| {
                self.bindings.iter().find_map(|b| match b.keys {
                    KeySeq::One(chord) if b.scope == s && chord.matches(key) => Some(b.action),
                    _ => None,
                })
            })
    }

    /// Whether `key` is the first key of some bound two-key sequence in
    /// [`Scope::Diff`].
    pub fn starts_sequence(&self, key: KeyEvent) -> bool {
        self.starts_sequence_in(Scope::Diff, key)
    }

    /// Whether `key` starts a bound two-key sequence within `scope` or
    /// [`Scope::Global`] (see [`Keymap::scope_chain`]).
    pub fn starts_sequence_in(&self, scope: Scope, key: KeyEvent) -> bool {
        Self::scope_chain(scope).into_iter().flatten().any(|s| {
            self.bindings.iter().any(|b| {
                b.scope == s && matches!(b.keys, KeySeq::Two(first, _) if first.matches(key))
            })
        })
    }

    /// All two-key bindings in `scope` (plus [`Scope::Global`], per
    /// [`Keymap::scope_chain`]) whose first chord matches `prefix` — the
    /// pending-prefix completions [`super::footer`] shows while a sequence
    /// is in progress (e.g. after `z`: just `za`; after `g`: `gd` and `gr`).
    /// Table-driven so a newly bound two-key sequence shows up here
    /// automatically, never hardcoded per-prefix. When a `Global` sequence
    /// shares its exact two-chord key with a `scope`-specific one (the
    /// shadowing case), only the `scope`-specific row is returned, matching
    /// dispatch.
    pub fn completions_for(&self, scope: Scope, prefix: KeyEvent) -> Vec<&Binding> {
        let mut out: Vec<&Binding> = Vec::new();
        let mut claimed: Vec<KeySeq> = Vec::new();
        for s in Self::scope_chain(scope).into_iter().flatten() {
            for b in self.bindings.iter().filter(|b| {
                b.scope == s && matches!(b.keys, KeySeq::Two(first, _) if first.matches(prefix))
            }) {
                if claimed.contains(&b.keys) {
                    continue;
                }
                claimed.push(b.keys);
                out.push(b);
            }
        }
        out
    }

    /// The which-key prefixes: the first chord of every [`KeySeq::Two`]
    /// binding in `scope`'s chain (see [`Keymap::scope_chain`]), each
    /// returned once as the [`KeyEvent`] that starts it, in first-appearance
    /// table order. Derived from the table rather than hardcoded to `g`/`z`,
    /// so a config remap that introduces (or removes) a two-key prefix
    /// changes this set for free — the which-key popup ([`super::which_key`])
    /// never assumes a fixed prefix list.
    pub fn which_key_prefixes(&self, scope: Scope) -> Vec<KeyEvent> {
        let mut seen: Vec<KeyChord> = Vec::new();
        let mut out = Vec::new();
        for s in Self::scope_chain(scope).into_iter().flatten() {
            for b in self.bindings.iter().filter(|b| b.scope == s) {
                if let KeySeq::Two(first, _) = b.keys
                    && !seen.contains(&first)
                {
                    seen.push(first);
                    out.push(KeyEvent::new(first.code, first.mods));
                }
            }
        }
        out
    }

    /// `(key label, description)` rows for every continuation bound after a
    /// pending `prefix`, in table order — the which-key popup's content. A
    /// thin display mapping over [`Keymap::completions_for`], so the popup
    /// can never list a binding dispatch itself wouldn't also resolve.
    pub fn continuations_for(&self, scope: Scope, prefix: KeyEvent) -> Vec<(String, &'static str)> {
        self.completions_for(scope, prefix)
            .into_iter()
            .map(|b| (b.key_label(), b.description))
            .collect()
    }

    /// Resolves a two-key sequence in [`Scope::Diff`]: `first` is the
    /// already-consumed pending prefix, `second` the key that completes it.
    /// `None` if no binding matches both — the caller silently cancels the
    /// pending prefix in that case.
    pub fn lookup_double(&self, first: KeyEvent, second: KeyEvent) -> Option<Action> {
        self.lookup_double_in(Scope::Diff, first, second)
    }

    /// Resolves a two-key sequence within `scope`.
    pub fn lookup_double_in(
        &self,
        scope: Scope,
        first: KeyEvent,
        second: KeyEvent,
    ) -> Option<Action> {
        self.bindings.iter().find_map(|b| match b.keys {
            KeySeq::Two(f, s) if b.scope == scope && f.matches(first) && s.matches(second) => {
                Some(b.action)
            }
            _ => None,
        })
    }

    /// Resolves one key event against this keymap, tracking a pending
    /// two-key prefix in `pending` across calls. This is the event loop's
    /// single entry point for Normal/Visual-mode key dispatch:
    ///
    /// - No prefix pending, `key` starts a sequence (`g`): records it in
    ///   `pending` and resolves nothing yet.
    /// - No prefix pending, `key` doesn't start a sequence: resolves via
    ///   [`Keymap::lookup`] (plain single-key dispatch).
    /// - A prefix is pending: resolves the completed sequence via
    ///   [`Keymap::lookup_double`] (or nothing, on an unknown second key —
    ///   this silently cancels the pending prefix either way) and clears
    ///   `pending`.
    ///
    /// `Esc` always clears a pending prefix and resolves nothing here. When
    /// nothing was pending, this still returns `None` for a bare `Esc` — the
    /// event loop's own Esc handling (closing help / canceling Visual mode)
    /// runs on top of this, not through the keymap table.
    pub fn resolve(&self, pending: &mut Option<KeyEvent>, key: KeyEvent) -> Option<Action> {
        self.resolve_in(Scope::Diff, pending, key)
    }

    /// [`Keymap::resolve`], but resolving within `scope`. Panel scope carries
    /// no two-key sequences today, so this reduces to a single-key
    /// [`Keymap::lookup_in`] there; the pending-prefix machinery is exercised
    /// only in diff scope.
    pub fn resolve_in(
        &self,
        scope: Scope,
        pending: &mut Option<KeyEvent>,
        key: KeyEvent,
    ) -> Option<Action> {
        if let Some(prefix) = pending.take() {
            if key.code == KeyCode::Esc {
                return None;
            }
            return self.lookup_double_in(scope, prefix, key);
        }
        if key.code == KeyCode::Esc {
            return None;
        }
        if self.starts_sequence_in(scope, key) {
            *pending = Some(key);
            return None;
        }
        self.lookup_in(scope, key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn plain_letter_bindings_resolve() {
        let km = Keymap::default_map();
        assert_eq!(
            km.lookup(key(KeyCode::Char('j'), KeyModifiers::NONE)),
            Some(Action::CursorDown)
        );
        assert_eq!(
            km.lookup(key(KeyCode::Char('k'), KeyModifiers::NONE)),
            Some(Action::CursorUp)
        );
    }

    #[test]
    fn ctrl_modifier_is_required() {
        let km = Keymap::default_map();
        assert_eq!(
            km.lookup(key(KeyCode::Char('d'), KeyModifiers::CONTROL)),
            Some(Action::HalfPageDown)
        );
        // Plain 'd' (no modifier) is a different binding entirely — the
        // review-session defer toggle, not half-page-down.
        assert_eq!(
            km.lookup(key(KeyCode::Char('d'), KeyModifiers::NONE)),
            Some(Action::ToggleDefer)
        );
    }

    #[test]
    fn tab_and_backtab_switch_files() {
        let km = Keymap::default_map();
        assert_eq!(
            km.lookup(key(KeyCode::Tab, KeyModifiers::NONE)),
            Some(Action::NextFile)
        );
        assert_eq!(
            km.lookup(key(KeyCode::BackTab, KeyModifiers::NONE)),
            Some(Action::PrevFile)
        );
        // Terminals that also set SHIFT on BackTab still resolve correctly.
        assert_eq!(
            km.lookup(key(KeyCode::BackTab, KeyModifiers::SHIFT)),
            Some(Action::PrevFile)
        );
    }

    #[test]
    fn uppercase_q_is_quit_discard_regardless_of_shift_bit() {
        let km = Keymap::default_map();
        assert_eq!(
            km.lookup(key(KeyCode::Char('Q'), KeyModifiers::NONE)),
            Some(Action::QuitDiscard)
        );
        assert_eq!(
            km.lookup(key(KeyCode::Char('Q'), KeyModifiers::SHIFT)),
            Some(Action::QuitDiscard)
        );
        assert_eq!(
            km.lookup(key(KeyCode::Char('q'), KeyModifiers::NONE)),
            Some(Action::Quit)
        );
    }

    #[test]
    fn ctrl_c_is_quit_discard() {
        let km = Keymap::default_map();
        assert_eq!(
            km.lookup(key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(Action::QuitDiscard)
        );
    }

    #[test]
    fn help_bindings_resolve() {
        let km = Keymap::default_map();
        assert_eq!(
            km.lookup(key(KeyCode::Char('?'), KeyModifiers::NONE)),
            Some(Action::ToggleHelp)
        );
        assert_eq!(
            km.lookup(key(KeyCode::Esc, KeyModifiers::NONE)),
            Some(Action::ToggleHelp)
        );
    }

    #[test]
    fn space_and_s_resolve_to_staging_actions() {
        let km = Keymap::default_map();
        assert_eq!(
            km.lookup(key(KeyCode::Char(' '), KeyModifiers::NONE)),
            Some(Action::ToggleStage)
        );
        assert_eq!(
            km.lookup(key(KeyCode::Char('s'), KeyModifiers::NONE)),
            Some(Action::ToggleStagingPanel)
        );
    }

    #[test]
    fn shift_s_resolves_to_stage_file_regardless_of_shift_bit() {
        let km = Keymap::default_map();
        // Uppercase `S` stages/unstages the file under the cursor; matching
        // strips SHIFT for Char, so both encodings resolve (mirrors K/Q/N).
        assert_eq!(
            km.lookup(key(KeyCode::Char('S'), KeyModifiers::NONE)),
            Some(Action::StageFile)
        );
        assert_eq!(
            km.lookup(key(KeyCode::Char('S'), KeyModifiers::SHIFT)),
            Some(Action::StageFile)
        );
        // Lowercase `s` still opens the staging panel, unaffected.
        assert_eq!(
            km.lookup(key(KeyCode::Char('s'), KeyModifiers::NONE)),
            Some(Action::ToggleStagingPanel)
        );
    }

    #[test]
    fn shift_r_opens_the_review_launcher_and_lowercase_r_refreshes() {
        let km = Keymap::default_map();
        // `R` is now `Scope::Global` (the Review launcher), regardless of
        // whether the terminal also sets the SHIFT bit; `Diff`-scope
        // `lookup` falls back to it (see `Keymap::scope_chain`).
        assert_eq!(
            km.lookup(key(KeyCode::Char('R'), KeyModifiers::NONE)),
            Some(Action::OpenReviewLauncher)
        );
        assert_eq!(
            km.lookup(key(KeyCode::Char('R'), KeyModifiers::SHIFT)),
            Some(Action::OpenReviewLauncher)
        );
        // Lowercase `r` refreshes now (freed up from Refresh's old `R`).
        assert_eq!(
            km.lookup(key(KeyCode::Char('r'), KeyModifiers::NONE)),
            Some(Action::Refresh)
        );
    }

    #[test]
    fn t_resolves_to_no_action() {
        let km = Keymap::default_map();
        assert_eq!(km.lookup(key(KeyCode::Char('t'), KeyModifiers::NONE)), None);
    }

    #[test]
    fn z_starts_a_sequence_and_za_toggles_collapse() {
        let km = Keymap::default_map();
        // `z` is now a two-key prefix, not itself a bound single key.
        assert!(km.starts_sequence(key(KeyCode::Char('z'), KeyModifiers::NONE)));
        assert_eq!(km.lookup(key(KeyCode::Char('z'), KeyModifiers::NONE)), None);
        let z = key(KeyCode::Char('z'), KeyModifiers::NONE);
        assert_eq!(
            km.lookup_double(z, key(KeyCode::Char('a'), KeyModifiers::NONE)),
            Some(Action::ToggleCollapse)
        );
    }

    #[test]
    fn resolve_completes_za_across_two_events() {
        let km = Keymap::default_map();
        let mut pending = None;
        assert_eq!(
            km.resolve(&mut pending, key(KeyCode::Char('z'), KeyModifiers::NONE)),
            None
        );
        assert!(pending.is_some());
        let action = km.resolve(&mut pending, key(KeyCode::Char('a'), KeyModifiers::NONE));
        assert_eq!(action, Some(Action::ToggleCollapse));
        assert_eq!(pending, None);
    }

    #[test]
    fn zz_zt_zb_resolve_via_lookup_double() {
        let km = Keymap::default_map();
        let z = key(KeyCode::Char('z'), KeyModifiers::NONE);
        assert_eq!(
            km.lookup_double(z, key(KeyCode::Char('z'), KeyModifiers::NONE)),
            Some(Action::RecenterCursor)
        );
        assert_eq!(
            km.lookup_double(z, key(KeyCode::Char('t'), KeyModifiers::NONE)),
            Some(Action::ScrollCursorTop)
        );
        assert_eq!(
            km.lookup_double(z, key(KeyCode::Char('b'), KeyModifiers::NONE)),
            Some(Action::ScrollCursorBottom)
        );
    }

    #[test]
    fn resolve_completes_zz_across_two_events() {
        let km = Keymap::default_map();
        let mut pending = None;
        assert_eq!(
            km.resolve(&mut pending, key(KeyCode::Char('z'), KeyModifiers::NONE)),
            None
        );
        let action = km.resolve(&mut pending, key(KeyCode::Char('z'), KeyModifiers::NONE));
        assert_eq!(action, Some(Action::RecenterCursor));
        assert_eq!(pending, None);
    }

    #[test]
    fn key_label_formats_modifiers_and_special_keys() {
        let km = Keymap::default_map();
        let labels: Vec<String> = km.bindings().iter().map(Binding::key_label).collect();
        assert!(labels.contains(&"Ctrl-d".to_string()));
        assert!(labels.contains(&"Shift-Tab".to_string()));
        assert!(labels.contains(&"Tab".to_string()));
        assert!(labels.contains(&"Esc".to_string()));
        assert!(labels.contains(&"?".to_string()));
        assert!(labels.contains(&"Space".to_string()));
    }

    // -- Column-cursor motion keys ------------------------------------------

    #[test]
    fn column_motion_keys_resolve() {
        let km = Keymap::default_map();
        assert_eq!(
            km.lookup(key(KeyCode::Char('h'), KeyModifiers::NONE)),
            Some(Action::CursorLeft)
        );
        assert_eq!(
            km.lookup(key(KeyCode::Char('l'), KeyModifiers::NONE)),
            Some(Action::CursorRight)
        );
        assert_eq!(
            km.lookup(key(KeyCode::Char('w'), KeyModifiers::NONE)),
            Some(Action::WordForward)
        );
        assert_eq!(
            km.lookup(key(KeyCode::Char('b'), KeyModifiers::NONE)),
            Some(Action::WordBackward)
        );
        assert_eq!(
            km.lookup(key(KeyCode::Char('0'), KeyModifiers::NONE)),
            Some(Action::CursorLineStart)
        );
        assert_eq!(
            km.lookup(key(KeyCode::Char('$'), KeyModifiers::NONE)),
            Some(Action::CursorLineEnd)
        );
    }

    #[test]
    fn full_page_keys_require_ctrl_and_are_distinct_from_half_page() {
        let km = Keymap::default_map();
        assert_eq!(
            km.lookup(key(KeyCode::Char('f'), KeyModifiers::CONTROL)),
            Some(Action::FullPageDown)
        );
        assert_eq!(
            km.lookup(key(KeyCode::Char('b'), KeyModifiers::CONTROL)),
            Some(Action::FullPageUp)
        );
        // Plain 'f'/'b' with no modifier are unbound (`b` alone is the word-
        // backward motion only without Ctrl, already covered above).
        assert_eq!(km.lookup(key(KeyCode::Char('f'), KeyModifiers::NONE)), None);
    }

    #[test]
    fn star_and_hash_resolve_to_search_word_actions() {
        let km = Keymap::default_map();
        assert_eq!(
            km.lookup(key(KeyCode::Char('*'), KeyModifiers::NONE)),
            Some(Action::SearchWordForward)
        );
        assert_eq!(
            km.lookup(key(KeyCode::Char('#'), KeyModifiers::NONE)),
            Some(Action::SearchWordBackward)
        );
    }

    // -- Two-key sequences (gd/gr) -------------------------------------------

    #[test]
    fn hover_is_a_single_key_binding() {
        let km = Keymap::default_map();
        assert_eq!(
            km.lookup(key(KeyCode::Char('K'), KeyModifiers::NONE)),
            Some(Action::Hover)
        );
    }

    #[test]
    fn g_starts_a_sequence_but_is_not_itself_bound() {
        let km = Keymap::default_map();
        assert!(km.starts_sequence(key(KeyCode::Char('g'), KeyModifiers::NONE)));
        assert!(!km.starts_sequence(key(KeyCode::Char('x'), KeyModifiers::NONE)));
        assert_eq!(km.lookup(key(KeyCode::Char('g'), KeyModifiers::NONE)), None);
    }

    #[test]
    fn gd_and_gr_resolve_via_lookup_double() {
        let km = Keymap::default_map();
        let g = key(KeyCode::Char('g'), KeyModifiers::NONE);
        assert_eq!(
            km.lookup_double(g, key(KeyCode::Char('d'), KeyModifiers::NONE)),
            Some(Action::GotoDefinition)
        );
        assert_eq!(
            km.lookup_double(g, key(KeyCode::Char('r'), KeyModifiers::NONE)),
            Some(Action::GotoReferences)
        );
    }

    #[test]
    fn gp_resolves_to_open_file_finder_via_lookup_double() {
        let km = Keymap::default_map();
        let g = key(KeyCode::Char('g'), KeyModifiers::NONE);
        assert_eq!(
            km.lookup_double(g, key(KeyCode::Char('p'), KeyModifiers::NONE)),
            Some(Action::OpenFileFinder)
        );
    }

    #[test]
    fn g_slash_resolves_to_open_project_search_via_lookup_double() {
        let km = Keymap::default_map();
        let g = key(KeyCode::Char('g'), KeyModifiers::NONE);
        assert_eq!(
            km.lookup_double(g, key(KeyCode::Char('/'), KeyModifiers::NONE)),
            Some(Action::OpenProjectSearch)
        );
    }

    #[test]
    fn g_space_resolves_to_open_editor_via_lookup_double() {
        let km = Keymap::default_map();
        let g = key(KeyCode::Char('g'), KeyModifiers::NONE);
        assert_eq!(
            km.lookup_double(g, key(KeyCode::Char(' '), KeyModifiers::NONE)),
            Some(Action::OpenEditor)
        );
    }

    #[test]
    fn unknown_second_key_after_prefix_is_none() {
        let km = Keymap::default_map();
        let g = key(KeyCode::Char('g'), KeyModifiers::NONE);
        assert_eq!(
            km.lookup_double(g, key(KeyCode::Char('z'), KeyModifiers::NONE)),
            None
        );
    }

    #[test]
    fn key_label_formats_two_key_sequences() {
        let km = Keymap::default_map();
        let labels: Vec<String> = km.bindings().iter().map(Binding::key_label).collect();
        assert!(labels.contains(&"gd".to_string()));
        assert!(labels.contains(&"gr".to_string()));
        assert!(labels.contains(&"gg".to_string()));
        assert!(labels.contains(&"gp".to_string()));
        assert!(labels.contains(&"g/".to_string()));
        assert!(labels.contains(&"gSpace".to_string()));
    }

    #[test]
    fn gg_resolves_to_jump_to_top_across_two_events() {
        let km = Keymap::default_map();
        let mut pending = None;
        let g = key(KeyCode::Char('g'), KeyModifiers::NONE);
        // First `g` records the pending prefix and resolves nothing.
        assert_eq!(km.resolve(&mut pending, g), None);
        assert_eq!(pending, Some(g));
        // Second `g` completes the sequence.
        let action = km.resolve(&mut pending, g);
        assert_eq!(action, Some(Action::JumpToTop));
        assert_eq!(pending, None);
    }

    #[test]
    fn shift_g_resolves_to_jump_to_bottom_regardless_of_shift_bit() {
        let km = Keymap::default_map();
        // Uppercase `G` jumps to the bottom; matching strips SHIFT for Char,
        // so both encodings resolve (mirrors K/Q/N/S/R).
        assert_eq!(
            km.lookup(key(KeyCode::Char('G'), KeyModifiers::NONE)),
            Some(Action::JumpToBottom)
        );
        assert_eq!(
            km.lookup(key(KeyCode::Char('G'), KeyModifiers::SHIFT)),
            Some(Action::JumpToBottom)
        );
        // Lowercase `g` is a two-key prefix, not itself bound.
        assert_eq!(km.lookup(key(KeyCode::Char('g'), KeyModifiers::NONE)), None);
    }

    /// A different key after `g` still cancels the pending prefix silently —
    /// `gg` doesn't change that (an unknown second key resolves to nothing).
    #[test]
    fn g_then_unknown_key_cancels_silently_with_gg_bound() {
        let km = Keymap::default_map();
        let mut pending = None;
        km.resolve(&mut pending, key(KeyCode::Char('g'), KeyModifiers::NONE));
        let action = km.resolve(&mut pending, key(KeyCode::Char('x'), KeyModifiers::NONE));
        assert_eq!(action, None);
        assert_eq!(pending, None);
    }

    // -- resolve(): the pending-prefix state machine -------------------------

    #[test]
    fn resolve_dispatches_single_keys_immediately_with_no_pending() {
        let km = Keymap::default_map();
        let mut pending = None;
        assert_eq!(
            km.resolve(&mut pending, key(KeyCode::Char('j'), KeyModifiers::NONE)),
            Some(Action::CursorDown)
        );
        assert_eq!(pending, None);
    }

    #[test]
    fn resolve_g_sets_pending_and_resolves_nothing() {
        let km = Keymap::default_map();
        let mut pending = None;
        let g = key(KeyCode::Char('g'), KeyModifiers::NONE);
        assert_eq!(km.resolve(&mut pending, g), None);
        assert_eq!(pending, Some(g));
    }

    #[test]
    fn resolve_completes_gd_and_clears_pending() {
        let km = Keymap::default_map();
        let mut pending = None;
        km.resolve(&mut pending, key(KeyCode::Char('g'), KeyModifiers::NONE));
        let action = km.resolve(&mut pending, key(KeyCode::Char('d'), KeyModifiers::NONE));
        assert_eq!(action, Some(Action::GotoDefinition));
        assert_eq!(pending, None);
    }

    #[test]
    fn resolve_cancels_silently_on_unknown_second_key() {
        let km = Keymap::default_map();
        let mut pending = None;
        km.resolve(&mut pending, key(KeyCode::Char('g'), KeyModifiers::NONE));
        let action = km.resolve(&mut pending, key(KeyCode::Char('z'), KeyModifiers::NONE));
        assert_eq!(action, None);
        assert_eq!(pending, None);
    }

    #[test]
    fn resolve_esc_cancels_a_pending_prefix() {
        let km = Keymap::default_map();
        let mut pending = None;
        km.resolve(&mut pending, key(KeyCode::Char('g'), KeyModifiers::NONE));
        let action = km.resolve(&mut pending, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(action, None);
        assert_eq!(pending, None);
    }

    #[test]
    fn resolve_bare_esc_with_no_pending_resolves_to_none() {
        let km = Keymap::default_map();
        let mut pending = None;
        let action = km.resolve(&mut pending, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(action, None);
        assert_eq!(pending, None);
    }

    // -- Scopes (diff vs. panel) --------------------------------------------

    /// Every pre-existing binding is diff-scope, so the scope-agnostic
    /// `lookup` and the diff-scoped `lookup_in` must agree for every
    /// single-key binding in the table — the "unfocused behavior is
    /// unchanged" guarantee, proven binding-by-binding.
    #[test]
    fn every_preexisting_single_key_binding_resolves_unchanged_in_diff_scope() {
        let km = Keymap::default_map();
        for b in km.bindings() {
            if b.scope != Scope::Diff {
                continue;
            }
            if let KeySeq::One(chord) = b.keys {
                let ev = key(chord.code, chord.mods);
                assert_eq!(
                    km.lookup(ev),
                    km.lookup_in(Scope::Diff, ev),
                    "diff-scope binding {:?} must resolve identically via lookup and lookup_in",
                    b.action
                );
                // `ToggleAccept`/`AcceptFile` deliberately
                // share Space/`S` with `ToggleStage`/`StageFile` — see
                // `Action::ToggleAccept`'s doc: those rows exist purely so
                // the help overlay/footer can document review's meaning for
                // those keys, and are reachable only via
                // `super::dispatch_key`'s review-session translation, never
                // a direct keymap lookup — so they're exempt from "this row
                // resolves to itself" (the `lookup`/`lookup_in` agreement
                // above still holds for them either way).
                if !matches!(b.action, Action::ToggleAccept | Action::AcceptFile) {
                    assert_eq!(km.lookup_in(Scope::Diff, ev), Some(b.action));
                }
            }
        }
    }

    #[test]
    fn backtick_focuses_panel_in_diff_scope() {
        let km = Keymap::default_map();
        assert_eq!(
            km.lookup_in(Scope::Diff, key(KeyCode::Char('`'), KeyModifiers::NONE)),
            Some(Action::FocusGitPanel)
        );
    }

    #[test]
    fn backtick_toggles_focus_back_in_panel_scope() {
        let km = Keymap::default_map();
        assert_eq!(
            km.lookup_in(Scope::Panel, key(KeyCode::Char('`'), KeyModifiers::NONE)),
            Some(Action::FocusGitPanel)
        );
    }

    /// `j`/`k` mean panel-cursor motion in panel scope but diff-cursor motion
    /// in diff scope — the scope dimension in action.
    #[test]
    fn jk_resolve_to_panel_motion_only_in_panel_scope() {
        let km = Keymap::default_map();
        let j = key(KeyCode::Char('j'), KeyModifiers::NONE);
        let k = key(KeyCode::Char('k'), KeyModifiers::NONE);
        assert_eq!(km.lookup_in(Scope::Panel, j), Some(Action::PanelCursorDown));
        assert_eq!(km.lookup_in(Scope::Panel, k), Some(Action::PanelCursorUp));
        assert_eq!(km.lookup_in(Scope::Diff, j), Some(Action::CursorDown));
        assert_eq!(km.lookup_in(Scope::Diff, k), Some(Action::CursorUp));
    }

    #[test]
    fn enter_selects_file_only_in_panel_scope() {
        let km = Keymap::default_map();
        let enter = key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(km.lookup_in(Scope::Panel, enter), Some(Action::PanelSelect));
        assert_eq!(km.lookup_in(Scope::Diff, enter), None);
    }

    /// A diff-only binding (`space` → stage) is invisible in panel scope, so
    /// the focused panel never fires review-loop actions.
    #[test]
    fn diff_only_bindings_do_not_resolve_in_panel_scope() {
        let km = Keymap::default_map();
        assert_eq!(
            km.lookup_in(Scope::Diff, key(KeyCode::Char(' '), KeyModifiers::NONE)),
            Some(Action::ToggleStage)
        );
        assert_eq!(
            km.lookup_in(Scope::Panel, key(KeyCode::Char(' '), KeyModifiers::NONE)),
            None
        );
        // `s` (staging panel) is likewise diff-only.
        assert_eq!(
            km.lookup_in(Scope::Panel, key(KeyCode::Char('s'), KeyModifiers::NONE)),
            None
        );
    }

    // -- Remote ops and command log ------------------------------------------

    /// `f`/`p`/`P` are panel-scope remote ops and resolve to nothing in diff
    /// scope, so they never fire during the ordinary review loop.
    #[test]
    fn remote_ops_resolve_only_in_panel_scope() {
        let km = Keymap::default_map();
        let f = key(KeyCode::Char('f'), KeyModifiers::NONE);
        let p = key(KeyCode::Char('p'), KeyModifiers::NONE);
        let big_p = key(KeyCode::Char('P'), KeyModifiers::NONE);
        assert_eq!(km.lookup_in(Scope::Panel, f), Some(Action::RemoteFetch));
        assert_eq!(km.lookup_in(Scope::Panel, p), Some(Action::RemotePull));
        assert_eq!(km.lookup_in(Scope::Panel, big_p), Some(Action::RemotePush));
        assert_eq!(km.lookup_in(Scope::Diff, f), None);
        assert_eq!(km.lookup_in(Scope::Diff, p), None);
        assert_eq!(km.lookup_in(Scope::Diff, big_p), None);
    }

    /// The focused git panel is a first-class view, so the quit family
    /// (`q`/`Q`/Ctrl-C) resolves in panel scope exactly as in diff scope —
    /// `q` must work from the panel, not just the diff view.
    #[test]
    fn quit_family_resolves_in_panel_scope() {
        let km = Keymap::default_map();
        assert_eq!(
            km.lookup_in(Scope::Panel, key(KeyCode::Char('q'), KeyModifiers::NONE)),
            Some(Action::Quit)
        );
        assert_eq!(
            km.lookup_in(Scope::Panel, key(KeyCode::Char('Q'), KeyModifiers::NONE)),
            Some(Action::QuitDiscard)
        );
        assert_eq!(
            km.lookup_in(Scope::Panel, key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(Action::QuitDiscard)
        );
    }

    // -- Scope::Global: behavior pin + resolution mechanics ------------------

    /// Behavior pin: `?`/`@`/`!`/the quit family resolve to the same action
    /// from both `Scope::Diff` and `Scope::Panel`, regardless of whether
    /// that's backed by a duplicate row per scope or a single shared row —
    /// this must hold unchanged before and after any row migration between
    /// the two.
    #[test]
    fn cross_scope_duplicated_bindings_resolve_identically() {
        let km = Keymap::default_map();
        for scope in [Scope::Diff, Scope::Panel] {
            assert_eq!(
                km.lookup_in(scope, key(KeyCode::Char('?'), KeyModifiers::NONE)),
                Some(Action::ToggleHelp),
                "`?` must resolve to ToggleHelp in {scope:?}"
            );
            assert_eq!(
                km.lookup_in(scope, key(KeyCode::Char('@'), KeyModifiers::NONE)),
                Some(Action::ToggleCommandLog),
                "`@` must resolve to ToggleCommandLog in {scope:?}"
            );
            assert_eq!(
                km.lookup_in(scope, key(KeyCode::Char('!'), KeyModifiers::NONE)),
                Some(Action::DismissConfigWarning),
                "`!` must resolve to DismissConfigWarning in {scope:?}"
            );
            assert_eq!(
                km.lookup_in(scope, key(KeyCode::Char('q'), KeyModifiers::NONE)),
                Some(Action::Quit),
                "`q` must resolve to Quit in {scope:?}"
            );
            assert_eq!(
                km.lookup_in(scope, key(KeyCode::Char('Q'), KeyModifiers::NONE)),
                Some(Action::QuitDiscard),
                "`Q` must resolve to QuitDiscard in {scope:?}"
            );
            assert_eq!(
                km.lookup_in(scope, key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
                Some(Action::QuitDiscard),
                "Ctrl-C must resolve to QuitDiscard in {scope:?}"
            );
        }
    }

    /// Drift check, the mirror image of the pin above: every actual
    /// `Scope::Global` row in the default table dispatches identically from
    /// both `Diff` and `Panel`, not just the five hand-picked keys — this
    /// would catch a future `Global` addition that only works from one
    /// scope.
    #[test]
    fn every_global_binding_dispatches_from_both_diff_and_panel_scope() {
        let km = Keymap::default_map();
        for b in km.bindings().iter().filter(|b| b.scope == Scope::Global) {
            let KeySeq::One(chord) = b.keys else {
                continue;
            };
            let ev = key(chord.code, chord.mods);
            assert_eq!(
                km.lookup_in(Scope::Diff, ev),
                Some(b.action),
                "Global binding {:?} ({}) must dispatch from Diff scope",
                b.action,
                b.key_label()
            );
            assert_eq!(
                km.lookup_in(Scope::Panel, ev),
                Some(b.action),
                "Global binding {:?} ({}) must dispatch from Panel scope",
                b.action,
                b.key_label()
            );
        }
    }

    #[test]
    fn lookup_in_falls_back_to_global_when_the_active_scope_has_no_row_for_the_key() {
        let km = Keymap::from_bindings(vec![Binding {
            keys: KeySeq::one(KeyCode::Char('x'), KeyModifiers::NONE),
            action: Action::ToggleCommandLog,
            description: "test",
            scope: Scope::Global,
            footer: None,
        }]);
        let x = key(KeyCode::Char('x'), KeyModifiers::NONE);
        assert_eq!(km.lookup_in(Scope::Diff, x), Some(Action::ToggleCommandLog));
        assert_eq!(
            km.lookup_in(Scope::Panel, x),
            Some(Action::ToggleCommandLog)
        );
    }

    #[test]
    fn a_scope_specific_binding_shadows_a_global_one_for_the_same_key() {
        let km = Keymap::from_bindings(vec![
            Binding {
                keys: KeySeq::one(KeyCode::Char('x'), KeyModifiers::NONE),
                action: Action::ToggleCommandLog,
                description: "global",
                scope: Scope::Global,
                footer: None,
            },
            Binding {
                keys: KeySeq::one(KeyCode::Char('x'), KeyModifiers::NONE),
                action: Action::CursorDown,
                description: "diff-specific",
                scope: Scope::Diff,
                footer: None,
            },
        ]);
        let x = key(KeyCode::Char('x'), KeyModifiers::NONE);
        // Diff scope: the Diff-specific row shadows the Global one.
        assert_eq!(km.lookup_in(Scope::Diff, x), Some(Action::CursorDown));
        // Panel scope: no Panel-specific row for `x`, so Global answers.
        assert_eq!(
            km.lookup_in(Scope::Panel, x),
            Some(Action::ToggleCommandLog)
        );
    }

    #[test]
    fn querying_global_scope_directly_does_not_fall_back_further() {
        let km = Keymap::from_bindings(vec![Binding {
            keys: KeySeq::one(KeyCode::Char('x'), KeyModifiers::NONE),
            action: Action::CursorDown,
            description: "diff-only",
            scope: Scope::Diff,
            footer: None,
        }]);
        assert_eq!(
            km.lookup_in(Scope::Global, key(KeyCode::Char('x'), KeyModifiers::NONE)),
            None
        );
    }

    #[test]
    fn starts_sequence_in_consults_global_scope_too() {
        let km = Keymap::from_bindings(vec![Binding {
            keys: KeySeq::two(
                KeyCode::Char('g'),
                KeyModifiers::NONE,
                KeyCode::Char('x'),
                KeyModifiers::NONE,
            ),
            action: Action::ToggleCommandLog,
            description: "test",
            scope: Scope::Global,
            footer: None,
        }]);
        let g = key(KeyCode::Char('g'), KeyModifiers::NONE);
        assert!(km.starts_sequence_in(Scope::Diff, g));
        assert!(km.starts_sequence_in(Scope::Panel, g));
    }

    #[test]
    fn completions_for_merges_scope_and_global_and_scope_shadows_global_on_a_shared_sequence() {
        let km = Keymap::from_bindings(vec![
            Binding {
                keys: KeySeq::two(
                    KeyCode::Char('g'),
                    KeyModifiers::NONE,
                    KeyCode::Char('a'),
                    KeyModifiers::NONE,
                ),
                action: Action::ToggleCommandLog,
                description: "global-ga",
                scope: Scope::Global,
                footer: None,
            },
            Binding {
                keys: KeySeq::two(
                    KeyCode::Char('g'),
                    KeyModifiers::NONE,
                    KeyCode::Char('b'),
                    KeyModifiers::NONE,
                ),
                action: Action::DismissConfigWarning,
                description: "global-gb",
                scope: Scope::Global,
                footer: None,
            },
            Binding {
                keys: KeySeq::two(
                    KeyCode::Char('g'),
                    KeyModifiers::NONE,
                    KeyCode::Char('a'),
                    KeyModifiers::NONE,
                ),
                action: Action::CursorDown,
                description: "diff-ga-shadow",
                scope: Scope::Diff,
                footer: None,
            },
        ]);
        let actions: Vec<Action> = km
            .completions_for(Scope::Diff, key(KeyCode::Char('g'), KeyModifiers::NONE))
            .into_iter()
            .map(|b| b.action)
            .collect();
        assert_eq!(actions.len(), 2, "expected `gb` plus the shadowing `ga`");
        assert!(actions.contains(&Action::CursorDown));
        assert!(actions.contains(&Action::DismissConfigWarning));
        assert!(!actions.contains(&Action::ToggleCommandLog));
    }

    #[test]
    fn label_for_falls_back_to_global_when_the_scope_has_no_row_for_the_action() {
        let km = Keymap::from_bindings(vec![Binding {
            keys: KeySeq::one(KeyCode::Char('?'), KeyModifiers::NONE),
            action: Action::ToggleHelp,
            description: "help",
            scope: Scope::Global,
            footer: None,
        }]);
        assert_eq!(
            km.label_for(Scope::Diff, Action::ToggleHelp),
            Some("?".to_string())
        );
        assert_eq!(
            km.label_for(Scope::Panel, Action::ToggleHelp),
            Some("?".to_string())
        );
    }

    // -- Switcher modal -------------------------------------------------------

    /// `b` opens the switcher only in panel scope; in diff scope it stays
    /// bound to `WordBackward` (column-cursor motion), unaffected.
    #[test]
    fn b_opens_switcher_only_in_panel_scope() {
        let km = Keymap::default_map();
        let b = key(KeyCode::Char('b'), KeyModifiers::NONE);
        assert_eq!(km.lookup_in(Scope::Panel, b), Some(Action::OpenSwitcher));
        assert_eq!(km.lookup_in(Scope::Diff, b), Some(Action::WordBackward));
    }

    // -- Commit staged --------------------------------------------------------

    /// Plain `c` commits only in panel scope; in diff scope it stays bound
    /// to `Compose` (annotate), unaffected — the scope dimension keeps the
    /// same physical key meaning different things per pane.
    #[test]
    fn c_commits_only_in_panel_scope_and_still_composes_in_diff_scope() {
        let km = Keymap::default_map();
        let c = key(KeyCode::Char('c'), KeyModifiers::NONE);
        assert_eq!(km.lookup_in(Scope::Panel, c), Some(Action::CommitStaged));
        assert_eq!(km.lookup_in(Scope::Diff, c), Some(Action::Compose));
        // Ctrl-c stays the quit-discard chord in both scopes, undisturbed by
        // the new plain-`c` panel row.
        let ctrl_c = key(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(
            km.lookup_in(Scope::Panel, ctrl_c),
            Some(Action::QuitDiscard)
        );
        assert_eq!(km.lookup_in(Scope::Diff, ctrl_c), Some(Action::QuitDiscard));
    }

    /// The `c commit` hint is promoted into the panel footer strip, matching
    /// the README's git-panel table.
    #[test]
    fn commit_staged_is_promoted_into_the_panel_footer() {
        let km = Keymap::default_map();
        let row = km
            .bindings()
            .iter()
            .find(|b| b.scope == Scope::Panel && b.action == Action::CommitStaged)
            .expect("panel-scope CommitStaged binding must exist");
        assert_eq!(row.key_label(), "c");
        assert_eq!(row.footer.map(|h| h.label), Some("commit"));
    }

    /// `@` toggles the command log from *both* scopes (it is a view toggle,
    /// not tied to which pane holds focus).
    // -- Footer promotion (`FooterHint`) -------------------------------------

    #[test]
    fn footer_builder_sets_rank_and_label() {
        let b = Binding {
            keys: KeySeq::one(KeyCode::Char('j'), KeyModifiers::NONE),
            action: Action::CursorDown,
            description: "Move cursor down",
            scope: Scope::Diff,
            footer: None,
        }
        .footer(1, "move");
        assert_eq!(
            b.footer,
            Some(FooterHint {
                rank: 1,
                label: "move"
            })
        );
    }

    #[test]
    fn cursor_down_and_up_are_promoted_with_the_same_footer_hint() {
        let km = Keymap::default_map();
        let down = km
            .bindings()
            .iter()
            .find(|b| b.scope == Scope::Diff && b.action == Action::CursorDown)
            .unwrap();
        let up = km
            .bindings()
            .iter()
            .find(|b| b.scope == Scope::Diff && b.action == Action::CursorUp)
            .unwrap();
        assert_eq!(
            down.footer,
            Some(FooterHint {
                rank: 1,
                label: "move"
            })
        );
        assert_eq!(down.footer, up.footer);
    }

    #[test]
    fn help_toggle_is_promoted_with_rank_zero_in_both_scopes() {
        let km = Keymap::default_map();
        // `?` is a single `Scope::Global` row (not one per scope), so its
        // `FooterHint` promotion applies uniformly wherever it resolves —
        // both `Diff` and `Panel`, via `Keymap::lookup_in`'s scope-then-
        // global fallback.
        let question_mark = km
            .bindings()
            .iter()
            .find(|b| {
                b.scope == Scope::Global && b.action == Action::ToggleHelp && b.key_label() == "?"
            })
            .expect("no `?` ToggleHelp binding");
        assert_eq!(
            question_mark.footer,
            Some(FooterHint {
                rank: 0,
                label: "help"
            })
        );
        for scope in [Scope::Diff, Scope::Panel] {
            assert_eq!(
                km.lookup_in(scope, key(KeyCode::Char('?'), KeyModifiers::NONE)),
                Some(Action::ToggleHelp),
                "`?` must resolve to ToggleHelp in {scope:?} via the Global fallback"
            );
        }
        // The diff-scope `Esc` row (Close help) is deliberately *not*
        // promoted — only `?` is, so the footer strip shows one help hint.
        let esc_close = km
            .bindings()
            .iter()
            .find(|b| {
                b.scope == Scope::Diff && b.action == Action::ToggleHelp && b.key_label() == "Esc"
            })
            .unwrap();
        assert_eq!(esc_close.footer, None);
    }

    // -- Panel-scope `?` (help from the git panel) ---------------------------

    /// The git panel's footer strip promises `? help`, so `?` must actually
    /// toggle help while the panel is focused — this closes the gap where
    /// `?` was diff-scope only.
    #[test]
    fn question_mark_toggles_help_in_panel_scope() {
        let km = Keymap::default_map();
        assert_eq!(
            km.lookup_in(Scope::Panel, key(KeyCode::Char('?'), KeyModifiers::NONE)),
            Some(Action::ToggleHelp)
        );
    }

    // -- `completions_for`: pending-prefix completions -----------------------

    #[test]
    fn completions_for_z_is_za_zz_zt_and_zb() {
        let km = Keymap::default_map();
        let z = key(KeyCode::Char('z'), KeyModifiers::NONE);
        let mut actions: Vec<Action> = km
            .completions_for(Scope::Diff, z)
            .into_iter()
            .map(|b| b.action)
            .collect();
        actions.sort_by_key(|a| format!("{a:?}"));
        let mut expected = vec![
            Action::ToggleCollapse,
            Action::RecenterCursor,
            Action::ScrollCursorTop,
            Action::ScrollCursorBottom,
        ];
        expected.sort_by_key(|a| format!("{a:?}"));
        assert_eq!(actions, expected);
    }

    #[test]
    fn completions_for_g_is_gg_gd_gr_gp_and_g_slash() {
        let km = Keymap::default_map();
        let g = key(KeyCode::Char('g'), KeyModifiers::NONE);
        let mut actions: Vec<Action> = km
            .completions_for(Scope::Diff, g)
            .into_iter()
            .map(|b| b.action)
            .collect();
        actions.sort_by_key(|a| format!("{a:?}"));
        assert_eq!(
            actions,
            vec![
                Action::GotoDefinition,
                Action::GotoReferences,
                Action::JumpToTop,
                Action::OpenEditor,
                Action::OpenFileFinder,
                Action::OpenProjectSearch,
            ]
        );
    }

    #[test]
    fn completions_for_an_unbound_prefix_is_empty() {
        let km = Keymap::default_map();
        let x = key(KeyCode::Char('x'), KeyModifiers::NONE);
        assert!(km.completions_for(Scope::Diff, x).is_empty());
    }

    // -- `which_key_prefixes`/`continuations_for`: which-key popup content --

    #[test]
    fn which_key_prefixes_derives_g_and_z_in_first_appearance_order() {
        let km = Keymap::default_map();
        let codes: Vec<KeyCode> = km
            .which_key_prefixes(Scope::Diff)
            .into_iter()
            .map(|k| k.code)
            .collect();
        // `gg` (JumpToTop) is the table's first two-key binding, `za`
        // (ToggleCollapse) the first `z`-prefixed one — order is
        // first-appearance, not alphabetical.
        assert_eq!(codes, vec![KeyCode::Char('g'), KeyCode::Char('z')]);
    }

    #[test]
    fn continuations_for_g_lists_every_g_prefixed_binding_in_table_order() {
        let km = Keymap::default_map();
        let rows = km.continuations_for(Scope::Diff, key(KeyCode::Char('g'), KeyModifiers::NONE));
        assert_eq!(
            rows,
            vec![
                ("gg".to_string(), "Jump to top of diff"),
                ("gd".to_string(), "Go to definition"),
                ("gr".to_string(), "Find references"),
                ("gp".to_string(), "Open fuzzy file finder"),
                ("g/".to_string(), "Open project search"),
                ("gSpace".to_string(), "Open file at cursor in editor"),
            ]
        );
    }

    #[test]
    fn continuations_for_z_lists_every_z_prefixed_binding_in_table_order() {
        let km = Keymap::default_map();
        let rows = km.continuations_for(Scope::Diff, key(KeyCode::Char('z'), KeyModifiers::NONE));
        assert_eq!(
            rows,
            vec![
                ("za".to_string(), "Collapse/expand file section"),
                ("zz".to_string(), "Center viewport on cursor"),
                ("zt".to_string(), "Scroll cursor to top of viewport"),
                ("zb".to_string(), "Scroll cursor to bottom of viewport"),
            ]
        );
    }

    #[test]
    fn which_key_prefixes_is_empty_for_a_scope_with_no_two_key_bindings() {
        // Panel scope carries no two-key sequences today (see
        // `Keymap::resolve_in`'s doc) — the which-key popup never appears
        // there.
        let km = Keymap::default_map();
        assert!(km.which_key_prefixes(Scope::Panel).is_empty());
    }

    /// `continuations_for` is a thin display mapping over `completions_for`,
    /// so the two can never drift apart for any discovered prefix — the
    /// popup can't show a binding dispatch itself wouldn't resolve, for
    /// every prefix the table currently defines, not just the two known
    /// today.
    #[test]
    fn continuations_for_matches_completions_for_across_every_discovered_prefix() {
        let km = Keymap::default_map();
        for prefix in km.which_key_prefixes(Scope::Diff) {
            let rows = km.continuations_for(Scope::Diff, prefix);
            let expected: Vec<(String, &'static str)> = km
                .completions_for(Scope::Diff, prefix)
                .into_iter()
                .map(|b| (b.key_label(), b.description))
                .collect();
            assert_eq!(rows, expected);
        }
    }

    #[test]
    fn at_toggles_command_log_in_both_scopes() {
        let km = Keymap::default_map();
        let at = key(KeyCode::Char('@'), KeyModifiers::NONE);
        assert_eq!(
            km.lookup_in(Scope::Diff, at),
            Some(Action::ToggleCommandLog)
        );
        assert_eq!(
            km.lookup_in(Scope::Panel, at),
            Some(Action::ToggleCommandLog)
        );
        // Terminals that report `@` with the SHIFT bit set still resolve it.
        assert_eq!(
            km.lookup_in(Scope::Diff, key(KeyCode::Char('@'), KeyModifiers::SHIFT)),
            Some(Action::ToggleCommandLog)
        );
    }

    // -- Config action-name mapping -------------------------------------------

    /// Bijectivity's runtime half: `action_name` is total by construction
    /// (an exhaustive match — a missing arm fails the *build*, not just this
    /// test), so what's left to check at runtime is (1) no two actions that
    /// actually appear in the keymap share a name and (2) every name
    /// resolves back to the same action via `action_from_name`. Iterating
    /// `default_map().bindings()` rather than every `Action` variant
    /// directly mirrors `help.rs`'s own `group_of` drift test — the
    /// "every user-visible action is reachable from the keymap" convention
    /// (CLAUDE.md) means the two enumerations coincide in practice, and Rust
    /// has no built-in enum-variant reflection without a derive-macro
    /// dependency this repo doesn't carry.
    #[test]
    fn action_names_are_total_and_bijective() {
        let km = Keymap::default_map();
        let mut seen_names = std::collections::HashSet::new();
        let mut seen_actions: Vec<Action> = Vec::new();
        for b in km.bindings() {
            if seen_actions.contains(&b.action) {
                continue;
            }
            seen_actions.push(b.action);
            let name = action_name(b.action);
            assert!(
                seen_names.insert(name),
                "duplicate action name {name:?} (action {:?})",
                b.action
            );
            assert_eq!(
                action_from_name(name),
                Some(b.action),
                "name {name:?} must resolve back to {:?}",
                b.action
            );
        }
    }

    #[test]
    fn unknown_action_name_resolves_to_none() {
        assert_eq!(action_from_name("not-a-real-action"), None);
    }

    // -- Grammar/label consistency --------------------------------------------

    /// Every default binding's `key_label()` (what `?` and the footer show)
    /// must round-trip through `crate::config::keys::parse_key_string` back
    /// into the *same* chord(s) — so config notation and help display can
    /// never drift apart. Two-key sequences are decomposed and each
    /// constituent chord's label is checked independently, since the
    /// overlay's compact two-key label (`"gd"`) is a display convenience
    /// distinct from the grammar's space-separated sequence notation
    /// (`"g d"`); the grammar operates one chord at a time either way.
    #[test]
    fn default_binding_key_labels_round_trip_through_the_config_grammar() {
        use crate::config::keys::{KeySeqSpec, parse_key_string};

        fn assert_round_trips(chord: KeyChord) {
            let label = chord.label();
            let parsed = parse_key_string(&label)
                .unwrap_or_else(|e| panic!("label {label:?} failed to parse: {e}"));
            match parsed {
                KeySeqSpec::One(spec) => {
                    assert_eq!(
                        KeyChord::new(spec.code, spec.mods),
                        chord,
                        "label {label:?} round-tripped to a different chord"
                    );
                }
                KeySeqSpec::Two(..) => {
                    panic!("single chord label {label:?} parsed as a two-chord sequence")
                }
            }
        }

        let km = Keymap::default_map();
        for b in km.bindings() {
            match b.keys {
                KeySeq::One(chord) => assert_round_trips(chord),
                KeySeq::Two(first, second) => {
                    assert_round_trips(first);
                    assert_round_trips(second);
                }
            }
        }
    }
}
