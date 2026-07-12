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
    /// Jump the column cursor to the start of the next word.
    WordForward,
    /// Jump the column cursor to the start of the previous word.
    WordBackward,
    /// Move the cursor down half a viewport.
    HalfPageDown,
    /// Move the cursor up half a viewport.
    HalfPageUp,
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
    /// a no-op on stash/header rows (panel scope).
    PanelSelect,
    /// Fetch from the upstream remote on a background thread (panel scope).
    RemoteFetch,
    /// Pull from the upstream remote on a background thread (panel scope).
    RemotePull,
    /// Push to the upstream remote on a background thread (panel scope).
    RemotePush,
    /// Open the branch/worktree switcher modal (panel scope).
    OpenSwitcher,
    /// Toggle the command-log pane (bound in both scopes).
    ToggleCommandLog,
    /// Re-read the working tree and rebuild the diff, picking up edits made
    /// outside redquill (e.g. by an agent) since the last refresh.
    Refresh,
    /// Quit, emitting annotations to stdout.
    Quit,
    /// Quit, discarding annotations.
    QuitDiscard,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// The diff view (Normal/Visual): every pre-existing binding.
    Diff,
    /// The git panel while it holds focus.
    Panel,
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
        match self.keys {
            KeySeq::One(chord) => chord.label(),
            KeySeq::Two(first, second) => format!("{}{}", first.label(), second.label()),
        }
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
                d(KeySeq::one(Char('?'), none), ToggleHelp, "Toggle help").footer(0, "help"),
                d(KeySeq::one(Esc, none), ToggleHelp, "Close help"),
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
                d(
                    KeySeq::one(Char('`'), none),
                    FocusGitPanel,
                    "Open git panel",
                )
                .footer(8, "git panel"),
                d(
                    KeySeq::one(Char('@'), none),
                    ToggleCommandLog,
                    "Toggle command log pane",
                ),
                d(
                    KeySeq::one(Char('R'), none),
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
                    KeySeq::two(Char('g'), none, Char('d'), none),
                    GotoDefinition,
                    "Go to definition",
                ),
                d(
                    KeySeq::two(Char('g'), none, Char('r'), none),
                    GotoReferences,
                    "Find references",
                ),
                d(KeySeq::one(Char('K'), none), Hover, "Hover docs"),
                d(
                    KeySeq::one(Char('q'), none),
                    Quit,
                    "Quit and emit annotations",
                ),
                d(
                    KeySeq::one(Char('Q'), none),
                    QuitDiscard,
                    "Quit and discard annotations",
                ),
                d(
                    KeySeq::one(Char('c'), ctrl),
                    QuitDiscard,
                    "Quit and discard annotations",
                ),
                // -- Panel scope: resolved only while the git panel is focused.
                p(
                    KeySeq::one(Char('`'), none),
                    FocusGitPanel,
                    "Close git panel",
                )
                .footer(6, "close"),
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
                    "Focus diff on this file",
                )
                .footer(2, "open file"),
                p(
                    KeySeq::one(Char('f'), none),
                    RemoteFetch,
                    "Fetch from remote",
                )
                .footer(3, "fetch"),
                p(KeySeq::one(Char('p'), none), RemotePull, "Pull from remote").footer(4, "pull"),
                p(KeySeq::one(Char('P'), none), RemotePush, "Push to remote").footer(5, "push"),
                // Wired here specifically so the git panel's footer strip can
                // promise a working `? help` escape hatch (see `super::footer`):
                // before this row, `?` was diff-scope only and did nothing while
                // the panel held focus. README's git panel table documents it
                // alongside this addition.
                p(KeySeq::one(Char('?'), none), ToggleHelp, "Toggle help").footer(0, "help"),
                p(
                    KeySeq::one(Char('b'), none),
                    OpenSwitcher,
                    "Open branch/worktree switcher",
                ),
                p(
                    KeySeq::one(Char('@'), none),
                    ToggleCommandLog,
                    "Toggle command log pane",
                ),
                // The focused git panel is a first-class view, not an overlay,
                // so the quit family works here exactly as in the diff view.
                p(
                    KeySeq::one(Char('q'), none),
                    Quit,
                    "Quit and emit annotations",
                ),
                p(
                    KeySeq::one(Char('Q'), none),
                    QuitDiscard,
                    "Quit and discard annotations",
                ),
                p(
                    KeySeq::one(Char('c'), ctrl),
                    QuitDiscard,
                    "Quit and discard annotations",
                ),
            ],
        }
    }

    /// All bindings, in table order — what the help overlay iterates.
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// Resolves a single key event to an [`Action`] in [`Scope::Diff`],
    /// matching only [`KeySeq::One`] bindings — unchanged behavior from
    /// before scopes existed (every pre-existing binding is diff-scope).
    /// Two-key sequences (`gd`, `gr`) can't be resolved from one event; see
    /// [`Keymap::resolve`].
    pub fn lookup(&self, key: KeyEvent) -> Option<Action> {
        self.lookup_in(Scope::Diff, key)
    }

    /// Resolves a single key event within `scope`. Bindings in other scopes
    /// are invisible here, so the same physical key resolves differently
    /// depending on which pane is focused.
    pub fn lookup_in(&self, scope: Scope, key: KeyEvent) -> Option<Action> {
        self.bindings.iter().find_map(|b| match b.keys {
            KeySeq::One(chord) if b.scope == scope && chord.matches(key) => Some(b.action),
            _ => None,
        })
    }

    /// Whether `key` is the first key of some bound two-key sequence in
    /// [`Scope::Diff`].
    pub fn starts_sequence(&self, key: KeyEvent) -> bool {
        self.starts_sequence_in(Scope::Diff, key)
    }

    /// Whether `key` starts a bound two-key sequence within `scope`.
    pub fn starts_sequence_in(&self, scope: Scope, key: KeyEvent) -> bool {
        self.bindings.iter().any(|b| {
            b.scope == scope && matches!(b.keys, KeySeq::Two(first, _) if first.matches(key))
        })
    }

    /// All two-key bindings in `scope` whose first chord matches `prefix` —
    /// the pending-prefix completions [`super::footer`] shows while a
    /// sequence is in progress (e.g. after `z`: just `za`; after `g`: `gd`
    /// and `gr`). Table-driven so a newly bound two-key sequence shows up
    /// here automatically, never hardcoded per-prefix.
    pub fn completions_for(&self, scope: Scope, prefix: KeyEvent) -> Vec<&Binding> {
        self.bindings
            .iter()
            .filter(|b| {
                b.scope == scope && matches!(b.keys, KeySeq::Two(first, _) if first.matches(prefix))
            })
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
        // Plain 'd' with no modifier is unbound.
        assert_eq!(km.lookup(key(KeyCode::Char('d'), KeyModifiers::NONE)), None);
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
    fn shift_r_resolves_to_refresh_regardless_of_shift_bit() {
        let km = Keymap::default_map();
        assert_eq!(
            km.lookup(key(KeyCode::Char('R'), KeyModifiers::NONE)),
            Some(Action::Refresh)
        );
        assert_eq!(
            km.lookup(key(KeyCode::Char('R'), KeyModifiers::SHIFT)),
            Some(Action::Refresh)
        );
        // Lowercase `r` is unbound (only `gr` uses `r`, as a sequence tail).
        assert_eq!(km.lookup(key(KeyCode::Char('r'), KeyModifiers::NONE)), None);
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
                assert_eq!(km.lookup_in(Scope::Diff, ev), Some(b.action));
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

    // -- Remote ops and command log (task 4.0) ------------------------------

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

    // -- Switcher modal (task 3.0) -------------------------------------------

    /// `b` opens the switcher only in panel scope; in diff scope it stays
    /// bound to `WordBackward` (column-cursor motion), unaffected.
    #[test]
    fn b_opens_switcher_only_in_panel_scope() {
        let km = Keymap::default_map();
        let b = key(KeyCode::Char('b'), KeyModifiers::NONE);
        assert_eq!(km.lookup_in(Scope::Panel, b), Some(Action::OpenSwitcher));
        assert_eq!(km.lookup_in(Scope::Diff, b), Some(Action::WordBackward));
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
        for scope in [Scope::Diff, Scope::Panel] {
            let question_mark = km
                .bindings()
                .iter()
                .find(|b| {
                    b.scope == scope && b.action == Action::ToggleHelp && b.key_label() == "?"
                })
                .unwrap_or_else(|| panic!("no `?` ToggleHelp binding in {scope:?}"));
            assert_eq!(
                question_mark.footer,
                Some(FooterHint {
                    rank: 0,
                    label: "help"
                })
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
    fn completions_for_z_is_just_za() {
        let km = Keymap::default_map();
        let z = key(KeyCode::Char('z'), KeyModifiers::NONE);
        let completions = km.completions_for(Scope::Diff, z);
        assert_eq!(completions.len(), 1);
        assert_eq!(completions[0].action, Action::ToggleCollapse);
        assert_eq!(completions[0].key_label(), "za");
    }

    #[test]
    fn completions_for_g_is_gg_gd_and_gr() {
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
                Action::JumpToTop
            ]
        );
    }

    #[test]
    fn completions_for_an_unbound_prefix_is_empty() {
        let km = Keymap::default_map();
        let x = key(KeyCode::Char('x'), KeyModifiers::NONE);
        assert!(km.completions_for(Scope::Diff, x).is_empty());
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
}
