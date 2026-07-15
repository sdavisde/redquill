//! Per-mode key tables: the single source of truth for the modal handlers
//! (List, Staging, Help, Peek in [`super::modes`]/[`super::mod`]) and the
//! help overlay's modal-mode hint sections ([`super::help`]).
//!
//! Normal/Visual/Panel dispatch runs through the data-driven [`super::Keymap`]
//! table. The remaining modes are modal — while one is active every keystroke
//! is handled directly, bypassing the keymap — so their keys can't live in the
//! keymap yet (that waits on the future config layer). This module gives each
//! of those modes one `const` table instead, so a handler and the help overlay
//! can never document different keys: both read the same table.
//!
//! - **List / Staging / Peek / Help** are one-action-per-key, so their tables
//!   carry a small per-mode action enum and their handlers dispatch straight
//!   off the table via [`resolve`] (a `match` on the action, which the compiler
//!   keeps exhaustive).
//! - **Compose / Search** are free-text input (every printable char inserts),
//!   which isn't expressible as one action per key, so their handlers keep a
//!   hand-written `match`. Their tables ([`ModalBinding<()>`]) document only the
//!   non-text *control* keys (Esc/Enter/…) for the overlay, and the drift
//!   cross-check test feeds those keys back through the real handlers.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::keymap::FooterHint;

/// One physical key: a code plus the modifiers used to synthesize its
/// [`KeyEvent`]. Matching is intentionally code-only ([`ModalKey::matches`]),
/// preserving the historical behavior of the modal handlers, which all
/// dispatched on `key.code` alone.
#[derive(Clone, Copy)]
pub(super) struct ModalKey {
    code: KeyCode,
    /// Only consumed by the test-only [`ModalKey::event`]; dispatch matches
    /// on the code alone (see [`ModalKey::matches`]), so non-test builds
    /// never read it.
    #[cfg_attr(not(test), allow(dead_code))]
    mods: KeyModifiers,
}

impl ModalKey {
    /// A key pressed with no modifiers.
    const fn plain(code: KeyCode) -> ModalKey {
        ModalKey {
            code,
            mods: KeyModifiers::NONE,
        }
    }

    /// A key pressed with Ctrl held (Compose's `Ctrl-j`/`Ctrl-t`).
    const fn ctrl(code: KeyCode) -> ModalKey {
        ModalKey {
            code,
            mods: KeyModifiers::CONTROL,
        }
    }

    /// Whether an incoming event is this key. Compares the key *code* only —
    /// the modal handlers never distinguished by modifier (they matched on
    /// `key.code`), so this keeps the table-driven dispatch behavior-identical.
    pub(super) fn matches(self, key: KeyEvent) -> bool {
        self.code == key.code
    }

    /// Synthesizes the [`KeyEvent`] this table entry stands for, used by the
    /// drift cross-check test to drive the real handlers.
    #[cfg(test)]
    pub(super) fn event(self) -> KeyEvent {
        KeyEvent::new(self.code, self.mods)
    }
}

/// One row of a per-mode key table: a display `label` and `description` for
/// the help overlay, the `keys` that trigger it, and the `action` a
/// table-driven handler dispatches to (`()` for the hint-only Compose/Search
/// tables, whose handlers keep a hand-written match).
pub(super) struct ModalBinding<A: 'static> {
    /// How the key(s) are shown in the help overlay, e.g. `"a / Esc"`.
    pub label: &'static str,
    /// What the help overlay prints next to the label.
    pub description: &'static str,
    /// Every physical key that triggers this row.
    pub keys: &'static [ModalKey],
    /// The per-mode action this row dispatches to.
    pub action: A,
    /// `Some` promotes this row into [`super::footer`]'s context-sensitive
    /// footer strip; `None` keeps it help-overlay-only. See
    /// [`super::keymap::FooterHint`] for the merge/rank/display rules — the
    /// same mechanism [`super::keymap::Binding`] uses.
    pub footer: Option<FooterHint>,
}

/// Resolves an incoming event against a table, returning the matched row's
/// action. This is the single dispatch primitive the table-driven modal
/// handlers share, so a handler accepts exactly the keys its table documents.
pub(super) fn resolve<A: Copy>(table: &[ModalBinding<A>], key: KeyEvent) -> Option<A> {
    table
        .iter()
        .find(|b| b.keys.iter().any(|k| k.matches(key)))
        .map(|b| b.action)
}

// -- List mode -------------------------------------------------------------

/// What a key does in the annotation-list panel.
#[derive(Clone, Copy)]
pub(super) enum ListAction {
    MoveDown,
    MoveUp,
    Jump,
    Edit,
    Delete,
    Close,
}

pub(super) const LIST_KEYS: &[ModalBinding<ListAction>] = &[
    ModalBinding {
        label: "j",
        description: "Move focus down",
        keys: &[ModalKey::plain(KeyCode::Char('j'))],
        action: ListAction::MoveDown,
        footer: Some(FooterHint {
            rank: 1,
            label: "move",
        }),
    },
    ModalBinding {
        label: "k",
        description: "Move focus up",
        keys: &[ModalKey::plain(KeyCode::Char('k'))],
        action: ListAction::MoveUp,
        footer: Some(FooterHint {
            rank: 1,
            label: "move",
        }),
    },
    ModalBinding {
        label: "Enter",
        description: "Jump to annotation",
        keys: &[ModalKey::plain(KeyCode::Enter)],
        action: ListAction::Jump,
        footer: Some(FooterHint {
            rank: 2,
            label: "open",
        }),
    },
    ModalBinding {
        label: "e",
        description: "Edit",
        keys: &[ModalKey::plain(KeyCode::Char('e'))],
        action: ListAction::Edit,
        footer: Some(FooterHint {
            rank: 3,
            label: "edit",
        }),
    },
    ModalBinding {
        label: "d",
        description: "Delete",
        keys: &[ModalKey::plain(KeyCode::Char('d'))],
        action: ListAction::Delete,
        footer: Some(FooterHint {
            rank: 4,
            label: "delete",
        }),
    },
    ModalBinding {
        label: "a / Esc",
        description: "Close panel",
        keys: &[
            ModalKey::plain(KeyCode::Char('a')),
            ModalKey::plain(KeyCode::Esc),
        ],
        action: ListAction::Close,
        footer: Some(FooterHint {
            rank: 5,
            label: "close",
        }),
    },
];

// -- Staging panel ---------------------------------------------------------

/// What a key does in the staging panel.
#[derive(Clone, Copy)]
pub(super) enum StagingAction {
    MoveDown,
    MoveUp,
    Unstage,
    Close,
}

pub(super) const STAGING_KEYS: &[ModalBinding<StagingAction>] = &[
    ModalBinding {
        label: "j",
        description: "Move focus down",
        keys: &[ModalKey::plain(KeyCode::Char('j'))],
        action: StagingAction::MoveDown,
        footer: Some(FooterHint {
            rank: 1,
            label: "move",
        }),
    },
    ModalBinding {
        label: "k",
        description: "Move focus up",
        keys: &[ModalKey::plain(KeyCode::Char('k'))],
        action: StagingAction::MoveUp,
        footer: Some(FooterHint {
            rank: 1,
            label: "move",
        }),
    },
    ModalBinding {
        label: "Space / Enter",
        description: "Unstage file",
        keys: &[
            ModalKey::plain(KeyCode::Char(' ')),
            ModalKey::plain(KeyCode::Enter),
        ],
        action: StagingAction::Unstage,
        footer: Some(FooterHint {
            rank: 2,
            label: "unstage",
        }),
    },
    ModalBinding {
        label: "s / Esc",
        description: "Close panel",
        keys: &[
            ModalKey::plain(KeyCode::Char('s')),
            ModalKey::plain(KeyCode::Esc),
        ],
        action: StagingAction::Close,
        footer: Some(FooterHint {
            rank: 3,
            label: "close",
        }),
    },
];

// -- Peek overlay ----------------------------------------------------------

/// What a key does in the LSP peek overlay.
#[derive(Clone, Copy)]
pub(super) enum PeekAction {
    MoveDown,
    MoveUp,
    Enter,
    Close,
}

pub(super) const PEEK_KEYS: &[ModalBinding<PeekAction>] = &[
    ModalBinding {
        label: "j",
        description: "Move selection / scroll hover down",
        keys: &[ModalKey::plain(KeyCode::Char('j'))],
        action: PeekAction::MoveDown,
        footer: Some(FooterHint {
            rank: 1,
            label: "move",
        }),
    },
    ModalBinding {
        label: "k",
        description: "Move selection / scroll hover up",
        keys: &[ModalKey::plain(KeyCode::Char('k'))],
        action: PeekAction::MoveUp,
        footer: Some(FooterHint {
            rank: 1,
            label: "move",
        }),
    },
    ModalBinding {
        label: "Enter",
        description: "Jump to location (definition/references)",
        keys: &[ModalKey::plain(KeyCode::Enter)],
        action: PeekAction::Enter,
        footer: Some(FooterHint {
            rank: 2,
            label: "jump",
        }),
    },
    ModalBinding {
        label: "Esc",
        description: "Close",
        keys: &[ModalKey::plain(KeyCode::Esc)],
        action: PeekAction::Close,
        footer: Some(FooterHint {
            rank: 3,
            label: "close",
        }),
    },
];

// -- Switcher modal ----------------------------------------------------------

/// What a key does in the branch/worktree switcher modal.
#[derive(Clone, Copy)]
pub(super) enum SwitcherAction {
    ToggleTab,
    MoveDown,
    MoveUp,
    Confirm,
    Close,
}

pub(super) const SWITCHER_KEYS: &[ModalBinding<SwitcherAction>] = &[
    ModalBinding {
        label: "Tab / h / l",
        description: "Switch tab (Branches / Worktrees)",
        keys: &[
            ModalKey::plain(KeyCode::Tab),
            ModalKey::plain(KeyCode::BackTab),
            ModalKey::plain(KeyCode::Char('h')),
            ModalKey::plain(KeyCode::Char('l')),
            ModalKey::plain(KeyCode::Left),
            ModalKey::plain(KeyCode::Right),
        ],
        action: SwitcherAction::ToggleTab,
        footer: Some(FooterHint {
            rank: 1,
            label: "switch tab",
        }),
    },
    ModalBinding {
        label: "j / Down",
        description: "Move selection down",
        keys: &[
            ModalKey::plain(KeyCode::Char('j')),
            ModalKey::plain(KeyCode::Down),
        ],
        action: SwitcherAction::MoveDown,
        footer: Some(FooterHint {
            rank: 2,
            label: "move",
        }),
    },
    ModalBinding {
        label: "k / Up",
        description: "Move selection up",
        keys: &[
            ModalKey::plain(KeyCode::Char('k')),
            ModalKey::plain(KeyCode::Up),
        ],
        action: SwitcherAction::MoveUp,
        // Not also tagged: its label ("k / Up") is already a compound key
        // display, so merging it with MoveDown's would double up the " / "
        // separators (see the identical note on HELP_KEYS's ScrollUp). The
        // MoveDown row's own label reads fine alone.
        footer: None,
    },
    ModalBinding {
        label: "Enter",
        description: "Switch to the selected branch/worktree",
        keys: &[ModalKey::plain(KeyCode::Enter)],
        action: SwitcherAction::Confirm,
        footer: Some(FooterHint {
            rank: 3,
            label: "switch",
        }),
    },
    ModalBinding {
        label: "Esc",
        description: "Close",
        keys: &[ModalKey::plain(KeyCode::Esc)],
        action: SwitcherAction::Close,
        footer: Some(FooterHint {
            rank: 4,
            label: "close",
        }),
    },
];

// -- Fuzzy file finder (hint-only) -----------------------------------------

/// Fuzzy file finder control keys (spec 06 Unit 1), for the help overlay and
/// footer strip. Like Compose/Search, the finder is free-text input
/// (printable chars extend the query) *plus* result navigation, so
/// [`super::modes::handle_finder_key`] keeps a hand-written match; this table
/// documents the non-text control keys and the drift cross-check drives them
/// through that handler. `Up`/`Down` (not `j`/`k`) navigate results — `j`/`k`
/// must stay typeable into the query, unlike the switcher modal (which has
/// no free-text input to protect).
pub(super) const FINDER_HINTS: &[ModalBinding<()>] = &[
    ModalBinding {
        label: "Up/Down",
        description: "Move selection",
        keys: &[ModalKey::plain(KeyCode::Up), ModalKey::plain(KeyCode::Down)],
        action: (),
        footer: Some(FooterHint {
            rank: 1,
            label: "move",
        }),
    },
    ModalBinding {
        label: "Enter",
        description: "Open the selected file (read-only whole-file view)",
        keys: &[ModalKey::plain(KeyCode::Enter)],
        action: (),
        footer: Some(FooterHint {
            rank: 2,
            label: "open",
        }),
    },
    ModalBinding {
        label: "Esc",
        description: "Close (returns to the prior view unchanged)",
        keys: &[ModalKey::plain(KeyCode::Esc)],
        action: (),
        footer: Some(FooterHint {
            rank: 3,
            label: "close",
        }),
    },
    ModalBinding {
        label: "Backspace",
        description: "Delete character",
        keys: &[ModalKey::plain(KeyCode::Backspace)],
        action: (),
        footer: None,
    },
];

// -- Help overlay ----------------------------------------------------------

/// What a key does while the help overlay is open (it scrolls, since the
/// binding list can outgrow the screen, or closes). Not rendered as an overlay
/// section — these keys already ride the overlay's bottom-border footer — but
/// kept here so [`super::handle_help_key`] dispatches off the table and the
/// drift cross-check covers it too.
#[derive(Clone, Copy)]
pub(super) enum HelpAction {
    Close,
    ScrollDown,
    ScrollUp,
    PageDown,
    PageUp,
    Top,
    Bottom,
    /// Starts filtering the keybind list (see [`super::App::help_search`]).
    Search,
}

pub(super) const HELP_KEYS: &[ModalBinding<HelpAction>] = &[
    ModalBinding {
        label: "Esc / Enter / ?",
        description: "Close help",
        keys: &[
            ModalKey::plain(KeyCode::Esc),
            ModalKey::plain(KeyCode::Enter),
            ModalKey::plain(KeyCode::Char('?')),
        ],
        action: HelpAction::Close,
        footer: Some(FooterHint {
            rank: 3,
            label: "close",
        }),
    },
    ModalBinding {
        label: "j / Down",
        description: "Scroll down",
        keys: &[
            ModalKey::plain(KeyCode::Char('j')),
            ModalKey::plain(KeyCode::Down),
        ],
        action: HelpAction::ScrollDown,
        // `ScrollUp` isn't also tagged: its label ("k / Up") is already a
        // compound key display, so merging it in would double up the " / "
        // separators (`super::footer`'s merge is for atomic key text like
        // "j" + "k"). This row's own label already reads fine alone.
        footer: Some(FooterHint {
            rank: 1,
            label: "scroll",
        }),
    },
    ModalBinding {
        label: "k / Up",
        description: "Scroll up",
        keys: &[
            ModalKey::plain(KeyCode::Char('k')),
            ModalKey::plain(KeyCode::Up),
        ],
        action: HelpAction::ScrollUp,
        footer: None,
    },
    ModalBinding {
        label: "PageDown",
        description: "Page down",
        keys: &[ModalKey::plain(KeyCode::PageDown)],
        action: HelpAction::PageDown,
        footer: None,
    },
    ModalBinding {
        label: "PageUp",
        description: "Page up",
        keys: &[ModalKey::plain(KeyCode::PageUp)],
        action: HelpAction::PageUp,
        footer: None,
    },
    ModalBinding {
        label: "g / Home",
        description: "Scroll to top",
        keys: &[
            ModalKey::plain(KeyCode::Char('g')),
            ModalKey::plain(KeyCode::Home),
        ],
        action: HelpAction::Top,
        footer: None,
    },
    ModalBinding {
        label: "G / End",
        description: "Scroll to bottom",
        keys: &[
            ModalKey::plain(KeyCode::Char('G')),
            ModalKey::plain(KeyCode::End),
        ],
        action: HelpAction::Bottom,
        footer: None,
    },
    ModalBinding {
        label: "/",
        description: "Filter keybinds",
        keys: &[ModalKey::plain(KeyCode::Char('/'))],
        action: HelpAction::Search,
        footer: Some(FooterHint {
            rank: 2,
            label: "filter",
        }),
    },
];

// -- Help overlay filter (hint-only) ---------------------------------------

/// Help-filter control keys, for the overlay's own hint text only. Filtering
/// is free-text input like Compose/Search, so [`super::handle_help_key`]'s
/// editing branch keeps a hand-written match; this table documents the
/// non-text keys and the drift cross-check test feeds them back through that
/// handler. Unlike `COMPOSE_HINTS`/`SEARCH_HINTS`, `Enter` here doesn't
/// submit-and-leave — it locks the filter in and hands control back to
/// `HELP_KEYS`' scroll keys, so its description says that explicitly.
pub(super) const HELP_SEARCH_HINTS: &[ModalBinding<()>] = &[
    ModalBinding {
        label: "Enter",
        description: "Lock in the filter (scroll keys resume)",
        keys: &[ModalKey::plain(KeyCode::Enter)],
        action: (),
        footer: None,
    },
    ModalBinding {
        label: "Esc",
        description: "Clear the filter",
        keys: &[ModalKey::plain(KeyCode::Esc)],
        action: (),
        footer: None,
    },
    ModalBinding {
        label: "Backspace",
        description: "Delete character",
        keys: &[ModalKey::plain(KeyCode::Backspace)],
        action: (),
        footer: None,
    },
];

// -- Compose modal (hint-only) ---------------------------------------------

/// Compose-mode control keys, for the help overlay only. Compose is free-text
/// input (printable chars insert), so [`super::handle_compose_key`] keeps a
/// hand-written match; this table documents the non-text keys and the drift
/// cross-check drives them through that handler.
pub(super) const COMPOSE_HINTS: &[ModalBinding<()>] = &[
    ModalBinding {
        label: "Enter",
        description: "Submit",
        keys: &[ModalKey::plain(KeyCode::Enter)],
        action: (),
        footer: Some(FooterHint {
            rank: 1,
            label: "save",
        }),
    },
    ModalBinding {
        label: "Esc",
        description: "Cancel",
        keys: &[ModalKey::plain(KeyCode::Esc)],
        action: (),
        footer: Some(FooterHint {
            rank: 2,
            label: "discard",
        }),
    },
    ModalBinding {
        label: "Ctrl-j",
        description: "Insert newline",
        keys: &[ModalKey::ctrl(KeyCode::Char('j'))],
        action: (),
        footer: None,
    },
    ModalBinding {
        label: "Ctrl-t",
        description: "Cycle classification",
        keys: &[ModalKey::ctrl(KeyCode::Char('t'))],
        action: (),
        footer: None,
    },
    ModalBinding {
        label: "Backspace",
        description: "Delete character",
        keys: &[ModalKey::plain(KeyCode::Backspace)],
        action: (),
        footer: None,
    },
    ModalBinding {
        label: "Left/Right/Up/Down",
        description: "Move within text",
        keys: &[
            ModalKey::plain(KeyCode::Left),
            ModalKey::plain(KeyCode::Right),
            ModalKey::plain(KeyCode::Up),
            ModalKey::plain(KeyCode::Down),
        ],
        action: (),
        footer: None,
    },
];

// -- Commit-message modal (hint-only) ----------------------------------------

/// Commit-message control keys (spec 04), for the help overlay and footer
/// strip. Like Compose, the modal is free-text input (printable chars
/// insert), so [`super::modes::handle_commit_message_key`] keeps a
/// hand-written match; this table documents the non-text keys and the drift
/// cross-check drives them through that handler.
pub(super) const COMMIT_MESSAGE_HINTS: &[ModalBinding<()>] = &[
    ModalBinding {
        label: "Enter",
        description: "Commit staged changes with this message",
        keys: &[ModalKey::plain(KeyCode::Enter)],
        action: (),
        footer: Some(FooterHint {
            rank: 1,
            label: "commit",
        }),
    },
    ModalBinding {
        label: "Esc",
        description: "Cancel back to the git panel",
        keys: &[ModalKey::plain(KeyCode::Esc)],
        action: (),
        footer: Some(FooterHint {
            rank: 2,
            label: "cancel",
        }),
    },
    ModalBinding {
        label: "Ctrl-j",
        description: "Insert newline (message body)",
        keys: &[ModalKey::ctrl(KeyCode::Char('j'))],
        action: (),
        footer: None,
    },
    ModalBinding {
        label: "Backspace",
        description: "Delete character",
        keys: &[ModalKey::plain(KeyCode::Backspace)],
        action: (),
        footer: None,
    },
    ModalBinding {
        label: "Left/Right/Up/Down",
        description: "Move within text",
        keys: &[
            ModalKey::plain(KeyCode::Left),
            ModalKey::plain(KeyCode::Right),
            ModalKey::plain(KeyCode::Up),
            ModalKey::plain(KeyCode::Down),
        ],
        action: (),
        footer: None,
    },
];

// -- Search input (hint-only) ----------------------------------------------

/// Search-input control keys, for the help overlay only. Like Compose, Search
/// is free-text input, so [`super::handle_search_key`] keeps its hand-written
/// match; this table documents the non-text keys.
pub(super) const SEARCH_HINTS: &[ModalBinding<()>] = &[
    ModalBinding {
        label: "Enter",
        description: "Confirm search",
        keys: &[ModalKey::plain(KeyCode::Enter)],
        action: (),
        footer: None,
    },
    ModalBinding {
        label: "Esc",
        description: "Cancel (clears pattern if buffer empty)",
        keys: &[ModalKey::plain(KeyCode::Esc)],
        action: (),
        footer: None,
    },
    ModalBinding {
        label: "Backspace",
        description: "Delete character",
        keys: &[ModalKey::plain(KeyCode::Backspace)],
        action: (),
        footer: None,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotate::{Classification, Target};
    use crate::diff::FileDiff;
    use crate::git::RawFilePatch;
    use crate::lsp::SourceLocation;
    use crate::ui::modes::{
        handle_compose_key, handle_list_key, handle_peek_key, handle_search_key, handle_staging_key,
    };
    use crate::ui::{App, Mode, StagedFile, compose, handle_help_key, peek};
    use std::path::PathBuf;

    fn sample_file() -> FileDiff {
        let raw = "\
diff --git a/src/main.rs b/src/main.rs
index 111..222 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,2 +1,2 @@
 fn main() {
-    old();
+    new();
";
        FileDiff::from_patch(&RawFilePatch {
            path: "src/main.rs".to_string(),
            old_path: None,
            raw: raw.to_string(),
            is_binary: false,
        })
        .unwrap()
    }

    fn app() -> App {
        App::new(vec![sample_file()])
    }

    /// An `App` in List mode over two annotations, so every list action has
    /// a visible effect.
    fn list_app() -> App {
        let mut app = app();
        app.annotations
            .add(Target::file("src/main.rs"), Classification::Question, "one")
            .unwrap();
        app.annotations
            .add(Target::file("src/main.rs"), Classification::Question, "two")
            .unwrap();
        app.mode = Mode::List;
        app
    }

    /// Every `LIST_KEYS` entry, fed through the *real* handler as the key
    /// event it documents, must perform the action it documents. Matching on
    /// the action enum is exhaustive, so a new table row can't ship without
    /// an assertion here.
    #[test]
    fn every_list_table_entry_drives_its_documented_action() {
        for binding in LIST_KEYS {
            for key in binding.keys {
                let mut app = list_app();
                let label = binding.label;
                match binding.action {
                    ListAction::MoveDown => {
                        handle_list_key(&mut app, key.event());
                        assert_eq!(app.list_cursor, 1, "List {label}: focus must move down");
                    }
                    ListAction::MoveUp => {
                        app.list_cursor = 1;
                        handle_list_key(&mut app, key.event());
                        assert_eq!(app.list_cursor, 0, "List {label}: focus must move up");
                    }
                    ListAction::Jump => {
                        handle_list_key(&mut app, key.event());
                        assert_eq!(app.mode, Mode::Normal, "List {label}: jump must close");
                    }
                    ListAction::Edit => {
                        handle_list_key(&mut app, key.event());
                        assert_eq!(app.mode, Mode::Compose, "List {label}: edit opens Compose");
                        assert!(app.compose.is_some());
                    }
                    ListAction::Delete => {
                        handle_list_key(&mut app, key.event());
                        assert_eq!(app.annotations.len(), 1, "List {label}: delete removes one");
                    }
                    ListAction::Close => {
                        handle_list_key(&mut app, key.event());
                        assert_eq!(app.mode, Mode::Normal, "List {label}: must close the panel");
                    }
                }
            }
        }
    }

    /// An `App` in Staging mode over two staged files (no git backend, so
    /// unstaging degrades to a footer message — still an observable effect).
    fn staging_app() -> App {
        let mut app = app();
        app.staged = vec![
            StagedFile {
                path: "a.rs".to_string(),
                letter: 'M',
            },
            StagedFile {
                path: "b.rs".to_string(),
                letter: 'M',
            },
        ];
        app.mode = Mode::Staging;
        app
    }

    #[test]
    fn every_staging_table_entry_drives_its_documented_action() {
        for binding in STAGING_KEYS {
            for key in binding.keys {
                let mut app = staging_app();
                let label = binding.label;
                match binding.action {
                    StagingAction::MoveDown => {
                        handle_staging_key(&mut app, key.event());
                        assert_eq!(app.staging_cursor, 1, "Staging {label}: focus moves down");
                    }
                    StagingAction::MoveUp => {
                        app.staging_cursor = 1;
                        handle_staging_key(&mut app, key.event());
                        assert_eq!(app.staging_cursor, 0, "Staging {label}: focus moves up");
                    }
                    StagingAction::Unstage => {
                        handle_staging_key(&mut app, key.event());
                        assert!(
                            app.status_message.is_some(),
                            "Staging {label}: unstage must act (footer message)"
                        );
                    }
                    StagingAction::Close => {
                        handle_staging_key(&mut app, key.event());
                        assert_eq!(app.mode, Mode::Normal, "Staging {label}: must close");
                    }
                }
            }
        }
    }

    /// An `App` in Peek mode over two canned References locations whose paths
    /// aren't in the diff (so Enter degrades to a footer message — still an
    /// observable effect).
    fn peek_app() -> App {
        let mut app = app();
        app.peek = Some(peek::PeekState::locations(
            peek::PeekKind::References,
            vec![
                SourceLocation {
                    path: PathBuf::from("/elsewhere/one.rs"),
                    line: 0,
                    character: 0,
                },
                SourceLocation {
                    path: PathBuf::from("/elsewhere/two.rs"),
                    line: 0,
                    character: 0,
                },
            ],
        ));
        app.mode = Mode::Peek;
        app
    }

    #[test]
    fn every_peek_table_entry_drives_its_documented_action() {
        for binding in PEEK_KEYS {
            for key in binding.keys {
                let mut app = peek_app();
                let label = binding.label;
                match binding.action {
                    PeekAction::MoveDown => {
                        handle_peek_key(&mut app, key.event());
                        assert_eq!(
                            app.peek.as_ref().unwrap().selected,
                            1,
                            "Peek {label}: selection moves down"
                        );
                    }
                    PeekAction::MoveUp => {
                        app.peek.as_mut().unwrap().selected = 1;
                        handle_peek_key(&mut app, key.event());
                        assert_eq!(
                            app.peek.as_ref().unwrap().selected,
                            0,
                            "Peek {label}: selection moves up"
                        );
                    }
                    PeekAction::Enter => {
                        handle_peek_key(&mut app, key.event());
                        assert!(
                            app.status_message.is_some(),
                            "Peek {label}: Enter must act (not-in-diff message)"
                        );
                    }
                    PeekAction::Close => {
                        handle_peek_key(&mut app, key.event());
                        assert_eq!(app.mode, Mode::Normal, "Peek {label}: must close");
                    }
                }
            }
        }
    }

    /// An `App` in Switcher mode over two branches and two worktrees, so
    /// every switcher action has a visible effect.
    fn switcher_app() -> App {
        let mut app = app();
        let branches = vec![
            crate::git::LocalBranch {
                name: "main".to_string(),
                is_current: true,
                worktree: None,
            },
            crate::git::LocalBranch {
                name: "feature".to_string(),
                is_current: false,
                worktree: None,
            },
        ];
        let worktrees = vec![
            crate::git::WorktreeEntry {
                path: PathBuf::from("/repo"),
                head: Some("aaa".to_string()),
                branch: Some("main".to_string()),
                bare: false,
                detached: false,
                locked: None,
                prunable: None,
            },
            crate::git::WorktreeEntry {
                path: PathBuf::from("/repo/wt"),
                head: Some("bbb".to_string()),
                branch: Some("feature".to_string()),
                bare: false,
                detached: false,
                locked: None,
                prunable: None,
            },
        ];
        app.switcher = Some(crate::ui::switcher::SwitcherState::new(
            branches, worktrees, None, 0,
        ));
        app.mode = Mode::Switcher;
        app
    }

    #[test]
    fn every_switcher_table_entry_drives_its_documented_action() {
        use crate::ui::modes::handle_switcher_key;
        use crate::ui::switcher::SwitcherTab;

        for binding in SWITCHER_KEYS {
            for key in binding.keys {
                let mut app = switcher_app();
                let label = binding.label;
                match binding.action {
                    SwitcherAction::ToggleTab => {
                        handle_switcher_key(&mut app, key.event());
                        assert_eq!(
                            app.switcher.as_ref().unwrap().tab,
                            SwitcherTab::Worktrees,
                            "Switcher {label}: must switch tab"
                        );
                    }
                    SwitcherAction::MoveDown => {
                        handle_switcher_key(&mut app, key.event());
                        assert_eq!(
                            app.switcher.as_ref().unwrap().branch_cursor,
                            1,
                            "Switcher {label}: cursor moves down"
                        );
                    }
                    SwitcherAction::MoveUp => {
                        app.switcher.as_mut().unwrap().branch_cursor = 1;
                        handle_switcher_key(&mut app, key.event());
                        assert_eq!(
                            app.switcher.as_ref().unwrap().branch_cursor,
                            0,
                            "Switcher {label}: cursor moves up"
                        );
                    }
                    SwitcherAction::Confirm => {
                        // Task 3 stub: Enter is a documented no-op (Task 4
                        // wires it up) — the modal must at least stay open.
                        handle_switcher_key(&mut app, key.event());
                        assert_eq!(
                            app.mode,
                            Mode::Switcher,
                            "Switcher {label}: modal stays open"
                        );
                    }
                    SwitcherAction::Close => {
                        handle_switcher_key(&mut app, key.event());
                        assert!(
                            matches!(app.mode, Mode::Panel { .. }),
                            "Switcher {label}: must close back to the panel"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn every_help_table_entry_drives_its_documented_action() {
        for binding in HELP_KEYS {
            for key in binding.keys {
                let mut app = app();
                app.help_open = true;
                app.help_scroll.set(25);
                app.help_viewport.set(10);
                handle_help_key(&mut app, key.event());
                let label = binding.label;
                match binding.action {
                    HelpAction::Close => {
                        assert!(!app.help_open, "Help {label}: must close the overlay");
                        assert_eq!(app.help_scroll.get(), 0);
                    }
                    HelpAction::ScrollDown => {
                        assert_eq!(app.help_scroll.get(), 26, "Help {label}: scrolls down")
                    }
                    HelpAction::ScrollUp => {
                        assert_eq!(app.help_scroll.get(), 24, "Help {label}: scrolls up")
                    }
                    HelpAction::PageDown => {
                        assert_eq!(app.help_scroll.get(), 35, "Help {label}: pages down")
                    }
                    HelpAction::PageUp => {
                        assert_eq!(app.help_scroll.get(), 15, "Help {label}: pages up")
                    }
                    HelpAction::Top => {
                        assert_eq!(app.help_scroll.get(), 0, "Help {label}: jumps to top")
                    }
                    HelpAction::Bottom => {
                        assert_eq!(app.help_scroll.get(), u16::MAX, "Help {label}: to bottom")
                    }
                    HelpAction::Search => {
                        assert_eq!(
                            app.help_search,
                            Some((String::new(), true)),
                            "Help {label}: must start filter-editing with an empty query"
                        );
                        assert_eq!(app.help_scroll.get(), 0, "Help {label}: must reset scroll");
                    }
                }
            }
        }
    }

    /// An `App` mid-help-filter with a non-empty query, so every documented
    /// control key produces an observable state change.
    fn help_search_app() -> App {
        let mut app = app();
        app.help_open = true;
        app.help_search = Some(("ab".to_string(), true));
        app
    }

    #[test]
    fn every_help_search_hint_key_is_consumed_by_the_handler() {
        for binding in HELP_SEARCH_HINTS {
            for key in binding.keys {
                let mut app = help_search_app();
                let before = app.help_search.clone();
                handle_help_key(&mut app, key.event());
                assert_ne!(
                    before, app.help_search,
                    "Help filter {}: documented key must be consumed by handle_help_key",
                    binding.label
                );
            }
        }
    }

    /// Reverse drift check for the help filter: non-text keys outside the
    /// hint table must do nothing while editing — the scroll keys stay inert
    /// mid-filter, same as `Mode::Search`. Chars are exempt (free-text input).
    #[test]
    fn help_search_handler_ignores_control_keys_absent_from_its_table() {
        let universe: Vec<KeyEvent> = [
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::Delete,
            KeyCode::Insert,
            KeyCode::F(1),
        ]
        .into_iter()
        .map(|code| KeyEvent::new(code, KeyModifiers::NONE))
        .collect();
        for ev in universe {
            if resolve(HELP_SEARCH_HINTS, ev).is_some() {
                continue;
            }
            let mut app = help_search_app();
            let before = app.help_search.clone();
            handle_help_key(&mut app, ev);
            assert_eq!(
                before, app.help_search,
                "handle_help_key consumed {ev:?} while filter-editing, which HELP_SEARCH_HINTS doesn't document"
            );
        }
    }

    // -- Compose / Search: hand-written handlers cross-checked against the
    // hint tables. Their dispatch stays a match (free-text input), so these
    // tests are what keeps the tables honest in both directions: every
    // documented control key must be consumed, and no undocumented control
    // key may do anything.

    /// An `App` mid-Compose with a three-line draft and the cursor at the
    /// middle of the middle line, so *every* documented control key produces
    /// an observable state change.
    fn compose_app() -> App {
        let mut app = app();
        app.apply(crate::ui::Action::Compose);
        let state = app.compose.as_mut().unwrap();
        state.buffer = compose::TextBuffer::from_str("ab\ncd\nef");
        state.buffer.cursor_row = 1;
        state.buffer.cursor_col = 1;
        app
    }

    /// Everything a Compose control key could observably change.
    fn compose_snapshot(app: &App) -> (Mode, Option<(compose::TextBuffer, Classification)>) {
        (
            app.mode,
            app.compose
                .as_ref()
                .map(|c| (c.buffer.clone(), c.classification)),
        )
    }

    #[test]
    fn every_compose_hint_key_is_consumed_by_the_handler() {
        for binding in COMPOSE_HINTS {
            for key in binding.keys {
                let mut app = compose_app();
                let before = compose_snapshot(&app);
                handle_compose_key(&mut app, key.event());
                assert_ne!(
                    before,
                    compose_snapshot(&app),
                    "Compose {}: documented key must be consumed by handle_compose_key",
                    binding.label
                );
            }
        }
    }

    /// Control keys the Compose hint table doesn't document must do nothing —
    /// the reverse drift check: a key added to `handle_compose_key` without a
    /// table row fails here. Printable chars are exempt (free-text input).
    #[test]
    fn compose_handler_ignores_control_keys_absent_from_its_table() {
        let mut universe: Vec<KeyEvent> = [
            KeyCode::Home,
            KeyCode::End,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::Delete,
            KeyCode::Insert,
            KeyCode::F(1),
        ]
        .into_iter()
        .map(|code| KeyEvent::new(code, KeyModifiers::NONE))
        .collect();
        for c in 'a'..='z' {
            universe.push(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
        }
        for ev in universe {
            if resolve(COMPOSE_HINTS, ev).is_some() {
                continue; // documented in the table; covered above
            }
            let mut app = compose_app();
            let before = compose_snapshot(&app);
            handle_compose_key(&mut app, ev);
            assert_eq!(
                before,
                compose_snapshot(&app),
                "handle_compose_key consumed {ev:?}, which the Compose hint table doesn't document"
            );
        }
    }

    /// An `App` mid-commit-message with a three-line draft and the cursor at
    /// the middle of the middle line, so *every* documented control key
    /// produces an observable state change. No git backend is attached, so
    /// `Enter` degrades to a footer message (still observable) rather than
    /// spawning git.
    fn commit_message_app() -> App {
        use crate::ui::commit_message::CommitMessageState;
        let mut app = app();
        app.staged = vec![StagedFile {
            path: "src/main.rs".to_string(),
            letter: 'M',
        }];
        app.mode = Mode::Panel {
            cursor: 0,
            tab: crate::ui::app::PanelTab::Changes,
        };
        app.apply(crate::ui::Action::CommitStaged);
        assert_eq!(app.mode, Mode::CommitMessage, "fixture must open the modal");
        let state: &mut CommitMessageState = app.commit_message.as_mut().unwrap();
        state.buffer = compose::TextBuffer::from_str("ab\ncd\nef");
        state.buffer.cursor_row = 1;
        state.buffer.cursor_col = 1;
        app
    }

    /// Everything a commit-message control key could observably change:
    /// the mode (Esc/Enter), the draft buffer (editing keys), and the footer
    /// message (Enter's no-backend rejection).
    fn commit_message_snapshot(app: &App) -> (Mode, Option<compose::TextBuffer>, Option<String>) {
        (
            app.mode,
            app.commit_message.as_ref().map(|c| c.buffer.clone()),
            app.status_message.clone(),
        )
    }

    #[test]
    fn every_commit_message_hint_key_is_consumed_by_the_handler() {
        use crate::ui::modes::handle_commit_message_key;
        for binding in COMMIT_MESSAGE_HINTS {
            for key in binding.keys {
                let mut app = commit_message_app();
                let before = commit_message_snapshot(&app);
                handle_commit_message_key(&mut app, key.event());
                assert_ne!(
                    before,
                    commit_message_snapshot(&app),
                    "Commit message {}: documented key must be consumed by handle_commit_message_key",
                    binding.label
                );
            }
        }
    }

    /// Control keys the commit-message hint table doesn't document must do
    /// nothing — the reverse drift check: a key added to
    /// `handle_commit_message_key` without a table row fails here. Printable
    /// chars are exempt (free-text input).
    #[test]
    fn commit_message_handler_ignores_control_keys_absent_from_its_table() {
        use crate::ui::modes::handle_commit_message_key;
        let mut universe: Vec<KeyEvent> = [
            KeyCode::Home,
            KeyCode::End,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::Delete,
            KeyCode::Insert,
            KeyCode::F(1),
        ]
        .into_iter()
        .map(|code| KeyEvent::new(code, KeyModifiers::NONE))
        .collect();
        for c in 'a'..='z' {
            universe.push(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
        }
        for ev in universe {
            if resolve(COMMIT_MESSAGE_HINTS, ev).is_some() {
                continue; // documented in the table; covered above
            }
            let mut app = commit_message_app();
            let before = commit_message_snapshot(&app);
            handle_commit_message_key(&mut app, ev);
            assert_eq!(
                before,
                commit_message_snapshot(&app),
                "handle_commit_message_key consumed {ev:?}, which the commit-message hint table doesn't document"
            );
        }
    }

    /// An `App` mid-Search with a non-empty pattern buffer, so every
    /// documented control key produces an observable state change.
    fn search_app() -> App {
        let mut app = app();
        app.mode = Mode::Search;
        app.search_input = "ab".to_string();
        app
    }

    fn search_snapshot(app: &App) -> (Mode, String) {
        (app.mode, app.search_input.clone())
    }

    #[test]
    fn every_search_hint_key_is_consumed_by_the_handler() {
        for binding in SEARCH_HINTS {
            for key in binding.keys {
                let mut app = search_app();
                let before = search_snapshot(&app);
                handle_search_key(&mut app, key.event());
                assert_ne!(
                    before,
                    search_snapshot(&app),
                    "Search {}: documented key must be consumed by handle_search_key",
                    binding.label
                );
            }
        }
    }

    /// Reverse drift check for Search: non-text keys outside the hint table
    /// must do nothing. Chars are exempt — every printable char extends the
    /// pattern by design.
    #[test]
    fn search_handler_ignores_control_keys_absent_from_its_table() {
        let universe: Vec<KeyEvent> = [
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::Delete,
            KeyCode::Insert,
            KeyCode::F(1),
        ]
        .into_iter()
        .map(|code| KeyEvent::new(code, KeyModifiers::NONE))
        .collect();
        for ev in universe {
            if resolve(SEARCH_HINTS, ev).is_some() {
                continue;
            }
            let mut app = search_app();
            let before = search_snapshot(&app);
            handle_search_key(&mut app, ev);
            assert_eq!(
                before,
                search_snapshot(&app),
                "handle_search_key consumed {ev:?}, which the Search hint table doesn't document"
            );
        }
    }

    /// An `App` mid-Finder with a non-empty query, three candidates/matches,
    /// and the cursor on the middle match — so `Up`/`Down` (each moving away
    /// from the middle in opposite directions) both produce an observable
    /// change, alongside `Enter`/`Esc`/`Backspace`.
    fn finder_app() -> App {
        use crate::search::{FileCandidate, FuzzyMatch};
        let mut app = app();
        app.mode = Mode::Finder;
        app.finder = Some(crate::ui::file_finder::FinderState {
            query: "ab".to_string(),
            candidates: vec![
                FileCandidate {
                    path: "ab1.rs".to_string(),
                },
                FileCandidate {
                    path: "ab2.rs".to_string(),
                },
                FileCandidate {
                    path: "ab3.rs".to_string(),
                },
            ],
            matches: vec![
                FuzzyMatch {
                    index: 0,
                    score: 10,
                    positions: vec![0, 1],
                },
                FuzzyMatch {
                    index: 1,
                    score: 9,
                    positions: vec![0, 1],
                },
                FuzzyMatch {
                    index: 2,
                    score: 8,
                    positions: vec![0, 1],
                },
            ],
            cursor: 1,
            return_mode: Mode::Normal,
        });
        app
    }

    /// Everything a Finder control key could observably change: the mode
    /// (`Enter`/`Esc` both close the overlay one way or another), whether the
    /// finder is still open, the query buffer, and the selection cursor.
    fn finder_snapshot(app: &App) -> (Mode, bool, String, usize) {
        (
            app.mode,
            app.finder.is_some(),
            app.finder
                .as_ref()
                .map(|f| f.query.clone())
                .unwrap_or_default(),
            app.finder.as_ref().map(|f| f.cursor).unwrap_or(0),
        )
    }

    #[test]
    fn every_finder_hint_key_is_consumed_by_the_handler() {
        use crate::ui::modes::handle_finder_key;
        for binding in FINDER_HINTS {
            for key in binding.keys {
                let mut app = finder_app();
                let before = finder_snapshot(&app);
                handle_finder_key(&mut app, key.event());
                assert_ne!(
                    before,
                    finder_snapshot(&app),
                    "Finder {}: documented key must be consumed by handle_finder_key",
                    binding.label
                );
            }
        }
    }

    /// Reverse drift check for Finder: non-text keys outside the hint table
    /// must do nothing. Chars are exempt — every printable char extends the
    /// query by design (and `j`/`k` in particular must stay typeable, not
    /// hijacked as navigation the way the switcher modal uses them).
    #[test]
    fn finder_handler_ignores_control_keys_absent_from_its_table() {
        use crate::ui::modes::handle_finder_key;
        let universe: Vec<KeyEvent> = [
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::Delete,
            KeyCode::Insert,
            KeyCode::F(1),
        ]
        .into_iter()
        .map(|code| KeyEvent::new(code, KeyModifiers::NONE))
        .collect();
        for ev in universe {
            if resolve(FINDER_HINTS, ev).is_some() {
                continue; // documented (Up/Down); covered above
            }
            let mut app = finder_app();
            let before = finder_snapshot(&app);
            handle_finder_key(&mut app, ev);
            assert_eq!(
                before,
                finder_snapshot(&app),
                "handle_finder_key consumed {ev:?}, which the Finder hint table doesn't document"
            );
        }
    }

    /// A key no table documents resolves to nothing in every table, so the
    /// table-driven handlers ignore it by construction.
    #[test]
    fn unbound_keys_resolve_to_nothing_in_every_table() {
        let ev = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
        assert!(resolve(LIST_KEYS, ev).is_none());
        assert!(resolve(STAGING_KEYS, ev).is_none());
        assert!(resolve(PEEK_KEYS, ev).is_none());
        assert!(resolve(HELP_KEYS, ev).is_none());
        assert!(resolve(COMPOSE_HINTS, ev).is_none());
        assert!(resolve(SEARCH_HINTS, ev).is_none());
        assert!(resolve(SWITCHER_KEYS, ev).is_none());
        assert!(resolve(HELP_SEARCH_HINTS, ev).is_none());
        assert!(resolve(COMMIT_MESSAGE_HINTS, ev).is_none());
        assert!(resolve(FINDER_HINTS, ev).is_none());
    }
}
