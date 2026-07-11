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
    /// Jump to the next hunk, crossing file boundaries if needed.
    NextHunk,
    /// Jump to the previous hunk, crossing file boundaries if needed.
    PrevHunk,
    /// Switch to the next file in the sidebar.
    NextFile,
    /// Switch to the previous file in the sidebar.
    PrevFile,
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
}

impl Binding {
    /// A display label for the key sequence, e.g. `"Ctrl-d"`, `"gd"`.
    pub fn key_label(&self) -> String {
        match self.keys {
            KeySeq::One(chord) => chord.label(),
            KeySeq::Two(first, second) => format!("{}{}", first.label(), second.label()),
        }
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
        Keymap {
            bindings: vec![
                Binding {
                    keys: KeySeq::one(Char('j'), none),
                    action: CursorDown,
                    description: "Move cursor down",
                },
                Binding {
                    keys: KeySeq::one(Char('k'), none),
                    action: CursorUp,
                    description: "Move cursor up",
                },
                Binding {
                    keys: KeySeq::one(Char('h'), none),
                    action: CursorLeft,
                    description: "Move column cursor left",
                },
                Binding {
                    keys: KeySeq::one(Char('l'), none),
                    action: CursorRight,
                    description: "Move column cursor right",
                },
                Binding {
                    keys: KeySeq::one(Char('w'), none),
                    action: WordForward,
                    description: "Jump column cursor to next word",
                },
                Binding {
                    keys: KeySeq::one(Char('b'), none),
                    action: WordBackward,
                    description: "Jump column cursor to previous word",
                },
                Binding {
                    keys: KeySeq::one(Char('d'), ctrl),
                    action: HalfPageDown,
                    description: "Scroll half page down",
                },
                Binding {
                    keys: KeySeq::one(Char('u'), ctrl),
                    action: HalfPageUp,
                    description: "Scroll half page up",
                },
                Binding {
                    keys: KeySeq::one(Char(']'), none),
                    action: NextHunk,
                    description: "Next hunk",
                },
                Binding {
                    keys: KeySeq::one(Char('['), none),
                    action: PrevHunk,
                    description: "Previous hunk",
                },
                Binding {
                    keys: KeySeq::one(Tab, none),
                    action: NextFile,
                    description: "Next file",
                },
                Binding {
                    keys: KeySeq::one(BackTab, none),
                    action: PrevFile,
                    description: "Previous file",
                },
                Binding {
                    keys: KeySeq::one(Char('?'), none),
                    action: ToggleHelp,
                    description: "Toggle help",
                },
                Binding {
                    keys: KeySeq::one(Esc, none),
                    action: ToggleHelp,
                    description: "Close help",
                },
                Binding {
                    keys: KeySeq::one(Char('v'), none),
                    action: EnterVisual,
                    description: "Enter visual selection / cancel",
                },
                Binding {
                    keys: KeySeq::one(Char('c'), none),
                    action: Compose,
                    description: "Comment on line/hunk/file (or visual selection)",
                },
                Binding {
                    keys: KeySeq::one(Char('a'), none),
                    action: ToggleList,
                    description: "Toggle annotation list panel",
                },
                Binding {
                    keys: KeySeq::one(Char(' '), none),
                    action: ToggleStage,
                    description: "Stage/unstage hunk (lines in visual mode)",
                },
                Binding {
                    keys: KeySeq::one(Char('s'), none),
                    action: ToggleStagingPanel,
                    description: "Toggle staging panel",
                },
                Binding {
                    keys: KeySeq::one(Char('/'), none),
                    action: Search,
                    description: "Search",
                },
                Binding {
                    keys: KeySeq::one(Char('n'), none),
                    action: SearchNext,
                    description: "Next search match",
                },
                Binding {
                    keys: KeySeq::one(Char('N'), none),
                    action: SearchPrev,
                    description: "Previous search match",
                },
                Binding {
                    keys: KeySeq::two(Char('g'), none, Char('d'), none),
                    action: GotoDefinition,
                    description: "Go to definition",
                },
                Binding {
                    keys: KeySeq::two(Char('g'), none, Char('r'), none),
                    action: GotoReferences,
                    description: "Find references",
                },
                Binding {
                    keys: KeySeq::one(Char('K'), none),
                    action: Hover,
                    description: "Hover docs",
                },
                Binding {
                    keys: KeySeq::one(Char('q'), none),
                    action: Quit,
                    description: "Quit and emit annotations",
                },
                Binding {
                    keys: KeySeq::one(Char('Q'), none),
                    action: QuitDiscard,
                    description: "Quit and discard annotations",
                },
                Binding {
                    keys: KeySeq::one(Char('c'), ctrl),
                    action: QuitDiscard,
                    description: "Quit and discard annotations",
                },
            ],
        }
    }

    /// All bindings, in table order — what the help overlay iterates.
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// Resolves a single key event to an [`Action`], matching only
    /// [`KeySeq::One`] bindings — unchanged behavior from before two-key
    /// sequences existed. Two-key sequences (`gd`, `gr`) can't be resolved
    /// from one event; see [`Keymap::resolve`].
    pub fn lookup(&self, key: KeyEvent) -> Option<Action> {
        self.bindings.iter().find_map(|b| match b.keys {
            KeySeq::One(chord) if chord.matches(key) => Some(b.action),
            _ => None,
        })
    }

    /// Whether `key` is the first key of some bound two-key sequence.
    pub fn starts_sequence(&self, key: KeyEvent) -> bool {
        self.bindings
            .iter()
            .any(|b| matches!(b.keys, KeySeq::Two(first, _) if first.matches(key)))
    }

    /// Resolves a two-key sequence: `first` is the already-consumed pending
    /// prefix, `second` the key that completes it. `None` if no binding
    /// matches both — the caller silently cancels the pending prefix in
    /// that case.
    pub fn lookup_double(&self, first: KeyEvent, second: KeyEvent) -> Option<Action> {
        self.bindings.iter().find_map(|b| match b.keys {
            KeySeq::Two(f, s) if f.matches(first) && s.matches(second) => Some(b.action),
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
        if let Some(prefix) = pending.take() {
            if key.code == KeyCode::Esc {
                return None;
            }
            return self.lookup_double(prefix, key);
        }
        if key.code == KeyCode::Esc {
            return None;
        }
        if self.starts_sequence(key) {
            *pending = Some(key);
            return None;
        }
        self.lookup(key)
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
    fn t_resolves_to_no_action() {
        let km = Keymap::default_map();
        assert_eq!(km.lookup(key(KeyCode::Char('t'), KeyModifiers::NONE)), None);
    }

    #[test]
    fn unbound_key_is_none() {
        let km = Keymap::default_map();
        assert_eq!(km.lookup(key(KeyCode::Char('z'), KeyModifiers::NONE)), None);
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
}
