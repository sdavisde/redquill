//! The reviewer's key -> [`Action`] table, kept as plain data so a rebind is
//! a one-line change in [`Keymap::default_map`] (FR-render-keymap-5). No
//! other item in this crate may hardcode a key name; anything that displays
//! or reacts to a key goes through [`Keymap::resolve`] or
//! [`Keymap::chord_for`].
//!
//! T1.0 stubs the shapes below; T2.0 fills in the real bindings and
//! resolution logic (see `04-tasks-first-render.md` §2.0).

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
/// Normalization (stripping irrelevant modifier bits) is filled in by T2.0's
/// `from_event`; the shape here is frozen for Wave 2/3 consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyChord {
    /// Normalize a `crossterm` [`KeyEvent`] into a stable [`Keymap`] lookup
    /// key. T1.0 provides a direct, minimally-typed copy; T2.0 strips
    /// irrelevant modifier bits so plain characters hash consistently.
    pub fn from_event(ev: KeyEvent) -> Self {
        KeyChord {
            code: ev.code,
            modifiers: ev.modifiers,
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
    /// The README draft bindings plus arrow/page aliases. T1.0 stubs this as
    /// an empty map; T2.0 fills in every binding (FR-render-keymap-2).
    pub fn default_map() -> Self {
        Keymap {
            bindings: HashMap::new(),
        }
    }

    /// Resolve a key event to an [`Action`]; unbound keys resolve to
    /// [`Action::Noop`] (FR-render-keymap-4). T1.0 stubs `default_map` as an
    /// empty table, so this lookup is always a `Noop` for now; T2.0 fills in
    /// the real bindings.
    pub fn resolve(&self, ev: KeyEvent) -> Action {
        let chord = KeyChord::from_event(ev);
        self.bindings.get(&chord).copied().unwrap_or(Action::Noop)
    }

    /// Reverse lookup: the chord bound to `action`, for display (e.g. a
    /// future help overlay) — FR-render-keymap-5. T1.0 stubs this as
    /// always-`None`; T2.0 fills in the real reverse scan.
    pub fn chord_for(&self, action: Action) -> Option<KeyChord> {
        let _ = action;
        None
    }
}
