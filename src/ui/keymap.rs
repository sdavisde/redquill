//! The keymap: data, not scattered match arms. [`Action`] is what the rest
//! of the UI reacts to; [`Binding`] pairs a key chord with an [`Action`] and
//! a human-readable description; [`Keymap`] is the lookup table. The help
//! overlay ([`super::help`]) renders directly from [`Keymap::bindings`], so
//! this table is the single source of truth for both dispatch and
//! documentation.

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
    /// Quit, emitting annotations to stdout.
    Quit,
    /// Quit, discarding annotations.
    QuitDiscard,
}

/// One entry in the keymap: a key chord, the action it triggers, and its
/// description for the help overlay.
#[derive(Debug, Clone, Copy)]
pub struct Binding {
    /// The key code that triggers this binding.
    pub code: KeyCode,
    /// Required modifiers. Compared after stripping [`KeyModifiers::SHIFT`]
    /// from character-producing keys — see [`Keymap::lookup`].
    pub mods: KeyModifiers,
    /// The action this binding triggers.
    pub action: Action,
    /// Human-readable description shown in the help overlay.
    pub description: &'static str,
}

impl Binding {
    /// A display label for the key chord, e.g. `"Ctrl-d"`, `"Shift-Tab"`.
    pub fn key_label(&self) -> String {
        let mut label = String::new();
        if self.mods.contains(KeyModifiers::CONTROL) {
            label.push_str("Ctrl-");
        }
        if self.mods.contains(KeyModifiers::ALT) {
            label.push_str("Alt-");
        }
        match self.code {
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

/// The keybinding table: a flat list of [`Binding`]s, looked up by key
/// chord. Remappable in principle (a future config layer would build a
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
                    code: Char('j'),
                    mods: none,
                    action: CursorDown,
                    description: "Move cursor down",
                },
                Binding {
                    code: Char('k'),
                    mods: none,
                    action: CursorUp,
                    description: "Move cursor up",
                },
                Binding {
                    code: Char('d'),
                    mods: ctrl,
                    action: HalfPageDown,
                    description: "Scroll half page down",
                },
                Binding {
                    code: Char('u'),
                    mods: ctrl,
                    action: HalfPageUp,
                    description: "Scroll half page up",
                },
                Binding {
                    code: Char(']'),
                    mods: none,
                    action: NextHunk,
                    description: "Next hunk",
                },
                Binding {
                    code: Char('['),
                    mods: none,
                    action: PrevHunk,
                    description: "Previous hunk",
                },
                Binding {
                    code: Tab,
                    mods: none,
                    action: NextFile,
                    description: "Next file",
                },
                Binding {
                    code: BackTab,
                    mods: none,
                    action: PrevFile,
                    description: "Previous file",
                },
                Binding {
                    code: Char('?'),
                    mods: none,
                    action: ToggleHelp,
                    description: "Toggle help",
                },
                Binding {
                    code: Esc,
                    mods: none,
                    action: ToggleHelp,
                    description: "Close help",
                },
                Binding {
                    code: Char('v'),
                    mods: none,
                    action: EnterVisual,
                    description: "Enter visual selection / cancel",
                },
                Binding {
                    code: Char('c'),
                    mods: none,
                    action: Compose,
                    description: "Comment on line/hunk/file (or visual selection)",
                },
                Binding {
                    code: Char('a'),
                    mods: none,
                    action: ToggleList,
                    description: "Toggle annotation list panel",
                },
                Binding {
                    code: Char('q'),
                    mods: none,
                    action: Quit,
                    description: "Quit and emit annotations",
                },
                Binding {
                    code: Char('Q'),
                    mods: none,
                    action: QuitDiscard,
                    description: "Quit and discard annotations",
                },
                Binding {
                    code: Char('c'),
                    mods: ctrl,
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

    /// Resolves a key event to an [`Action`], if any binding matches.
    ///
    /// `SHIFT` is stripped from the incoming event's modifiers before
    /// comparison whenever the key code already encodes shift itself (an
    /// uppercase char like `Q`, a shifted punctuation char like `?`, or
    /// `BackTab`). Terminals are inconsistent about whether they also set
    /// the `SHIFT` bit in that situation, so bindings for those keys are
    /// defined without `SHIFT` and this keeps lookups terminal-agnostic.
    pub fn lookup(&self, key: KeyEvent) -> Option<Action> {
        let mut mods = key.modifiers;
        if matches!(key.code, KeyCode::Char(_) | KeyCode::BackTab) {
            mods.remove(KeyModifiers::SHIFT);
        }
        self.bindings
            .iter()
            .find(|b| b.code == key.code && b.mods == mods)
            .map(|b| b.action)
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
    }
}
