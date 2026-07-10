//! The reviewer's key -> [`Action`] table, kept as plain data so a rebind is
//! a one-line change in [`Keymap::default_map`] (FR-render-keymap-5). No
//! other item in this crate may hardcode a key name; anything that displays
//! or reacts to a key goes through [`Keymap::resolve`] or
//! [`Keymap::chord_for`].
//!
//! All default bindings below are provisional: the operator is expected to
//! tune them by feel once the tool is in hand, so every rebind must cost
//! exactly one entry in [`Keymap::default_map`] (plus a README table row and
//! at most one default-map test assertion) — nothing else in the crate may
//! need to change (FR-render-keymap-5).

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Every reviewer action; bound by the [`Keymap`]. Most are no-ops until
/// their owning roadmap task lands (see spec §5/§7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    ScrollDown,
    ScrollUp,
    HalfPageDown,
    HalfPageUp,
    NextHunk,
    PrevHunk,
    NextFile,
    PrevFile,
    Search,
    SearchNext,
    SearchPrev,
    Comment,
    VisualSelect,
    StageToggle,
    ToggleStagePanel,
    GotoDefinition,
    FindReferences,
    Hover,
    AnnotationList,
    Help,
    Quit,
    QuitDiscard,
    Noop,
}

/// A normalized key press: `code` plus the modifier bits that matter.
///
/// `from_event` strips modifier bits that don't carry independent meaning
/// (e.g. `SHIFT` on a character key, since the shifted character is already
/// encoded in `code`) so the same physical key press always hashes to the
/// same [`KeyChord`] regardless of how a given terminal reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyChord {
    /// Normalize a `crossterm` [`KeyEvent`] into a stable [`Keymap`] lookup
    /// key (FR-render-keymap-1).
    ///
    /// Terminals vary in whether a shifted character key (e.g. `Q`, `?`, or
    /// `BackTab` for Shift-Tab) also sets the `SHIFT` modifier bit alongside
    /// the already-shifted `code`. Keeping that bit would make the same
    /// physical key press hash to two different chords depending on the
    /// terminal, so only modifiers that carry information not already
    /// present in `code` (currently just `CONTROL`) survive normalization.
    pub fn from_event(ev: KeyEvent) -> Self {
        KeyChord {
            code: ev.code,
            modifiers: ev.modifiers & KeyModifiers::CONTROL,
        }
    }
}

/// `KeyEvent -> Action` as plain data (no closures/trait objects) so a
/// future config-file loader can build one without refactoring this layer
/// (spec §5 remap-readiness note).
pub struct Keymap {
    bindings: HashMap<KeyChord, Action>,
}

impl Keymap {
    /// The README draft bindings plus arrow/page aliases (FR-render-keymap-2).
    ///
    /// This is the single source of truth for every default binding; nothing
    /// else in the crate may hardcode a key name (FR-render-keymap-5). Only
    /// `j`/`k`, `Ctrl-d`/`Ctrl-u`, and `q`/`Q` have live behavior this task —
    /// every other action below is bound but resolves to a no-op in the
    /// event loop until its owning roadmap task lands (FR-render-keymap-3).
    pub fn default_map() -> Self {
        let mut bindings = HashMap::new();

        // Move / scroll — live this task.
        bindings.insert(
            KeyChord {
                code: KeyCode::Char('j'),
                modifiers: KeyModifiers::NONE,
            },
            Action::ScrollDown,
        );
        bindings.insert(
            KeyChord {
                code: KeyCode::Char('k'),
                modifiers: KeyModifiers::NONE,
            },
            Action::ScrollUp,
        );
        bindings.insert(
            KeyChord {
                code: KeyCode::Char('d'),
                modifiers: KeyModifiers::CONTROL,
            },
            Action::HalfPageDown,
        );
        bindings.insert(
            KeyChord {
                code: KeyCode::Char('u'),
                modifiers: KeyModifiers::CONTROL,
            },
            Action::HalfPageUp,
        );

        // Hunk / file navigation — bound, no-op until Task 5.
        bindings.insert(
            KeyChord {
                code: KeyCode::Char(']'),
                modifiers: KeyModifiers::NONE,
            },
            Action::NextHunk,
        );
        bindings.insert(
            KeyChord {
                code: KeyCode::Char('['),
                modifiers: KeyModifiers::NONE,
            },
            Action::PrevHunk,
        );
        bindings.insert(
            KeyChord {
                code: KeyCode::Tab,
                modifiers: KeyModifiers::NONE,
            },
            Action::NextFile,
        );
        bindings.insert(
            KeyChord {
                code: KeyCode::BackTab,
                modifiers: KeyModifiers::NONE,
            },
            Action::PrevFile,
        );

        // Search — bound, no-op until search lands.
        bindings.insert(
            KeyChord {
                code: KeyCode::Char('/'),
                modifiers: KeyModifiers::NONE,
            },
            Action::Search,
        );
        bindings.insert(
            KeyChord {
                code: KeyCode::Char('n'),
                modifiers: KeyModifiers::NONE,
            },
            Action::SearchNext,
        );
        bindings.insert(
            KeyChord {
                code: KeyCode::Char('N'),
                modifiers: KeyModifiers::NONE,
            },
            Action::SearchPrev,
        );

        // Annotate / stage — bound, no-op until their roadmap tasks land.
        bindings.insert(
            KeyChord {
                code: KeyCode::Char('c'),
                modifiers: KeyModifiers::NONE,
            },
            Action::Comment,
        );
        bindings.insert(
            KeyChord {
                code: KeyCode::Char('v'),
                modifiers: KeyModifiers::NONE,
            },
            Action::VisualSelect,
        );
        bindings.insert(
            KeyChord {
                code: KeyCode::Char(' '),
                modifiers: KeyModifiers::NONE,
            },
            Action::StageToggle,
        );
        bindings.insert(
            KeyChord {
                code: KeyCode::Char('s'),
                modifiers: KeyModifiers::NONE,
            },
            Action::ToggleStagePanel,
        );

        // LSP peek. The README binds `gd`/`gr`/`K`; `K` is a real single-key
        // no-op binding. `gd`/`gr` are two-key chords, and real chord
        // sequencing is deferred to Task 5 (spec §9 Open Question:
        // "recommended default: stub as no-op single entries now"). Until
        // then they are registered here as placeholder SINGLE-entry
        // bindings on the chord's second character (`d`, `r`) so a
        // `chord_for` reverse lookup has something to find; these two
        // entries will be replaced by real `g`-prefixed chord handling in
        // Task 5, at which point this comment and the bindings below change
        // together (FR-render-keymap-5 one-line-rebind invariant).
        bindings.insert(
            KeyChord {
                code: KeyCode::Char('d'),
                modifiers: KeyModifiers::NONE,
            },
            Action::GotoDefinition,
        );
        bindings.insert(
            KeyChord {
                code: KeyCode::Char('r'),
                modifiers: KeyModifiers::NONE,
            },
            Action::FindReferences,
        );
        bindings.insert(
            KeyChord {
                code: KeyCode::Char('K'),
                modifiers: KeyModifiers::NONE,
            },
            Action::Hover,
        );

        // Annotation list / help — bound, no-op until their roadmap tasks land.
        bindings.insert(
            KeyChord {
                code: KeyCode::Char('a'),
                modifiers: KeyModifiers::NONE,
            },
            Action::AnnotationList,
        );
        bindings.insert(
            KeyChord {
                code: KeyCode::Char('?'),
                modifiers: KeyModifiers::NONE,
            },
            Action::Help,
        );

        // Quit — live this task.
        bindings.insert(
            KeyChord {
                code: KeyCode::Char('q'),
                modifiers: KeyModifiers::NONE,
            },
            Action::Quit,
        );
        bindings.insert(
            KeyChord {
                code: KeyCode::Char('Q'),
                modifiers: KeyModifiers::NONE,
            },
            Action::QuitDiscard,
        );

        // Arrow / page-key aliases (spec §9 default): extra entries, zero
        // new behavior — they resolve to the same actions as their vim
        // counterparts above.
        bindings.insert(
            KeyChord {
                code: KeyCode::Down,
                modifiers: KeyModifiers::NONE,
            },
            Action::ScrollDown,
        );
        bindings.insert(
            KeyChord {
                code: KeyCode::Up,
                modifiers: KeyModifiers::NONE,
            },
            Action::ScrollUp,
        );
        bindings.insert(
            KeyChord {
                code: KeyCode::PageDown,
                modifiers: KeyModifiers::NONE,
            },
            Action::HalfPageDown,
        );
        bindings.insert(
            KeyChord {
                code: KeyCode::PageUp,
                modifiers: KeyModifiers::NONE,
            },
            Action::HalfPageUp,
        );

        Keymap { bindings }
    }

    /// Resolve a key event to an [`Action`] via table lookup — no `match` on
    /// `ev.code` (FR-render-keymap-1). Unbound keys resolve to
    /// [`Action::Noop`] (FR-render-keymap-4).
    pub fn resolve(&self, ev: KeyEvent) -> Action {
        let chord = KeyChord::from_event(ev);
        self.bindings.get(&chord).copied().unwrap_or(Action::Noop)
    }

    /// Reverse lookup: the chord bound to `action`, for display (e.g. a
    /// future help overlay) — the only sanctioned way to show a key name
    /// outside `default_map` (FR-render-keymap-5).
    pub fn chord_for(&self, action: Action) -> Option<KeyChord> {
        self.bindings
            .iter()
            .find_map(|(chord, bound)| (*bound == action).then_some(*chord))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_resolve(ev: KeyEvent) -> Action {
        Keymap::default_map().resolve(ev)
    }

    #[test]
    fn j_scrolls_down() {
        assert_eq!(
            default_resolve(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
            Action::ScrollDown
        );
    }

    #[test]
    fn k_scrolls_up() {
        assert_eq!(
            default_resolve(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE)),
            Action::ScrollUp
        );
    }

    #[test]
    fn ctrl_d_half_page_down() {
        assert_eq!(
            default_resolve(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)),
            Action::HalfPageDown
        );
    }

    #[test]
    fn ctrl_u_half_page_up() {
        assert_eq!(
            default_resolve(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL)),
            Action::HalfPageUp
        );
    }

    #[test]
    fn q_quits_and_emits() {
        assert_eq!(
            default_resolve(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
            Action::Quit
        );
    }

    #[test]
    fn shift_q_quits_and_discards_regardless_of_shift_bit_reporting() {
        // Some terminals report the shifted char alone; others also set the
        // SHIFT modifier bit. from_event must normalize so both resolve the
        // same way.
        assert_eq!(
            default_resolve(KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::SHIFT)),
            Action::QuitDiscard
        );
        assert_eq!(
            default_resolve(KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::NONE)),
            Action::QuitDiscard
        );
    }

    #[test]
    fn question_mark_opens_help_regardless_of_shift_bit_reporting() {
        assert_eq!(
            default_resolve(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT)),
            Action::Help
        );
        assert_eq!(
            default_resolve(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE)),
            Action::Help
        );
    }

    #[test]
    fn shift_tab_aliases_prev_file_regardless_of_shift_bit_reporting() {
        assert_eq!(
            default_resolve(KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE)),
            Action::PrevFile
        );
        assert_eq!(
            default_resolve(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)),
            Action::PrevFile
        );
    }

    #[test]
    fn next_hunk_binding_present_but_noop_this_task() {
        // FR-render-keymap-3: bound, behavior deferred to Task 5.
        assert_eq!(
            default_resolve(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE)),
            Action::NextHunk
        );
    }

    #[test]
    fn unbound_key_resolves_to_noop() {
        assert_eq!(
            default_resolve(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE)),
            Action::Noop
        );
    }

    #[test]
    fn arrow_and_page_aliases_match_vim_counterparts() {
        assert_eq!(
            default_resolve(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            Action::ScrollDown
        );
        assert_eq!(
            default_resolve(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
            Action::ScrollUp
        );
        assert_eq!(
            default_resolve(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE)),
            Action::HalfPageDown
        );
        assert_eq!(
            default_resolve(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE)),
            Action::HalfPageUp
        );
    }

    #[test]
    fn chord_for_quit_reverse_lookup() {
        let keymap = Keymap::default_map();
        let chord = keymap.chord_for(Action::Quit).expect("quit is bound");
        assert_eq!(
            chord,
            KeyChord {
                code: KeyCode::Char('q'),
                modifiers: KeyModifiers::NONE,
            }
        );
        // Self-consistency: resolving that exact chord's event returns Quit.
        assert_eq!(
            keymap.resolve(KeyEvent::new(chord.code, chord.modifiers)),
            Action::Quit
        );
    }

    #[test]
    fn from_event_normalizes_shift_bit_on_char_keys() {
        let with_shift =
            KeyChord::from_event(KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::SHIFT));
        let without_shift =
            KeyChord::from_event(KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::NONE));
        assert_eq!(with_shift, without_shift);
    }

    #[test]
    fn from_event_preserves_control_modifier() {
        let ctrl_d = KeyChord::from_event(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        let plain_d = KeyChord::from_event(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        assert_ne!(ctrl_d, plain_d);
    }
}
