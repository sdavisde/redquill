//! Per-mode key tables: the single source of truth for the modal handlers
//! (List, Staging, Help, Peek in [`super::modes`]/[`super::mod`]) and the
//! help overlay's modal-mode hint sections ([`super::help`]).
//!
//! Normal/Visual/Panel dispatch runs through the data-driven [`super::Keymap`]
//! table. The remaining modes are modal — while one is active every keystroke
//! is handled directly, bypassing the keymap — so their keys can't live in
//! that table. This module gives each of those modes one runtime-built
//! default table instead, so a handler and the help overlay can never
//! document different keys: both read the same table.
//!
//! Each table below is a `static` [`LazyLock<Vec<ModalBinding<A>>>`], built
//! once (lazily, on first access) from plain row data — runtime construction
//! is what lets `crate::config`'s `[keys.<mode>]` overrides layer onto these
//! defaults the same way `crate::ui::keymap_config::effective_keymap` layers
//! `[keys.diff]`/`[keys.panel]` onto `Keymap::default_map()`. Every table is
//! still built exactly once and threaded/read by reference — no
//! per-keystroke parsing.
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
//!
//! Some newer modal tables are not yet config-remappable; wiring one up
//! means adding it to `MODAL_MODE_NAMES` and the `[keys.<mode>]` merge.

use std::sync::LazyLock;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::keymap::FooterHint;

/// One physical key: a code plus the modifiers used to synthesize its
/// [`KeyEvent`]. Matching considers both ([`ModalKey::matches`]): a table can
/// carry both a plain key and a modifier-chorded one for genuinely different
/// actions (Compose's plain `Enter` submits, `Shift-Enter` inserts a
/// newline), which code-only matching couldn't tell apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ModalKey {
    code: KeyCode,
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

    /// A key pressed with Alt held (Project Search's `Alt-c`/`Alt-w`/`Alt-r`
    /// toggles; the modals' `Alt-b`/`Alt-f`/`Alt-d` word motions and
    /// `Alt+arrow`/`Alt+Backspace` variants).
    const fn alt(code: KeyCode) -> ModalKey {
        ModalKey {
            code,
            mods: KeyModifiers::ALT,
        }
    }

    /// A key pressed with Shift held (the modals' `Shift+Enter` newline, which
    /// only reaches the app on kitty-enhancement-capable terminals — see
    /// [`super::init_terminal`]).
    const fn shift(code: KeyCode) -> ModalKey {
        ModalKey {
            code,
            mods: KeyModifiers::SHIFT,
        }
    }

    /// Whether an incoming event is this key: code and modifiers must both
    /// match, with `SHIFT` stripped from the incoming event whenever the code
    /// itself already encodes shift (an uppercase char, a shifted punctuation
    /// char, or `BackTab`) — terminals are inconsistent about also setting the
    /// `SHIFT` bit in that situation, so shift-encoding codes are defined
    /// without `SHIFT` and matching stays terminal-agnostic. Mirrors
    /// `super::keymap::KeyChord::matches` exactly.
    pub(super) fn matches(self, key: KeyEvent) -> bool {
        let mut mods = key.modifiers;
        if matches!(key.code, KeyCode::Char(_) | KeyCode::BackTab) {
            mods.remove(KeyModifiers::SHIFT);
        }
        self.code == key.code && self.mods == mods
    }

    /// A display label for this key, e.g. `"Ctrl-w"`, `"Shift-Tab"`, `"j"` —
    /// mirrors `super::keymap::KeyChord::label`'s rendering exactly, so a
    /// modal table's config notation and its help/footer display can't drift
    /// apart the same way the main keymap's can't (see
    /// `crate::config::keys`'s module doc on that round-trip guarantee).
    pub(super) fn label(self) -> String {
        let mut label = String::new();
        if self.mods.contains(KeyModifiers::CONTROL) {
            label.push_str("Ctrl-");
        }
        if self.mods.contains(KeyModifiers::ALT) {
            label.push_str("Alt-");
        }
        if self.mods.contains(KeyModifiers::SHIFT) {
            label.push_str("Shift-");
        }
        match self.code {
            KeyCode::Char(' ') => label.push_str("Space"),
            KeyCode::Char(c) => label.push(c),
            KeyCode::Tab => label.push_str("Tab"),
            KeyCode::BackTab => label.push_str("Shift-Tab"),
            KeyCode::Esc => label.push_str("Esc"),
            KeyCode::Enter => label.push_str("Enter"),
            KeyCode::Backspace => label.push_str("Backspace"),
            KeyCode::Delete => label.push_str("Delete"),
            KeyCode::Home => label.push_str("Home"),
            KeyCode::End => label.push_str("End"),
            KeyCode::PageUp => label.push_str("PageUp"),
            KeyCode::PageDown => label.push_str("PageDown"),
            KeyCode::Up => label.push_str("Up"),
            KeyCode::Down => label.push_str("Down"),
            KeyCode::Left => label.push_str("Left"),
            KeyCode::Right => label.push_str("Right"),
            KeyCode::Insert => label.push_str("Insert"),
            KeyCode::F(n) => label.push_str(&format!("F{n}")),
            other => label.push_str(&format!("{other:?}")),
        }
        label
    }

    /// Synthesizes the [`KeyEvent`] this table entry stands for, used by the
    /// drift cross-check test to drive the real handlers.
    #[cfg(test)]
    pub(super) fn event(self) -> KeyEvent {
        KeyEvent::new(self.code, self.mods)
    }

    /// Builds a runtime [`ModalKey`] from a parsed grammar chord
    /// (`crate::config::keys::ChordSpec`) — the modal-table counterpart of
    /// `super::keymap::KeySeq::from_spec`. Modal tables never supported
    /// two-chord sequences (unlike the main keymap's `gd`/`gr`), so
    /// `crate::ui::modal_keys_config` rejects a `KeySeqSpec::Two` for a
    /// `[keys.<mode>]` entry as an invalid value before ever reaching here.
    pub(super) fn from_spec(spec: crate::config::keys::ChordSpec) -> ModalKey {
        ModalKey {
            code: spec.code,
            mods: spec.mods,
        }
    }
}

/// One row of a per-mode key table: a `description` for the help overlay,
/// the `keys` that trigger it, and the `action` a table-driven handler
/// dispatches to. Every mode's handler is table-driven, including the
/// free-text modes (Compose, Search, Finder, ...): every documented
/// *control* key (never bare printable-char insertion, which stays a
/// hand-written fallback — see each mode's action enum doc) resolves to a
/// real per-mode action here, which is what lets `[keys.<mode>]` config
/// remap it.
#[derive(Clone)]
pub(super) struct ModalBinding<A: Clone + 'static> {
    /// What the help overlay prints next to the label.
    pub description: &'static str,
    /// Every physical key that triggers this row — owned so
    /// `crate::ui::modal_keys_config`'s `[keys.<mode>]` merge can replace it
    /// with a config-provided `Vec`, the same way `crate::ui::keymap_config`
    /// replaces a `Binding`'s `KeySeq`.
    pub keys: Vec<ModalKey>,
    /// The per-mode action this row dispatches to.
    pub action: A,
    /// `Some` promotes this row into [`super::footer`]'s context-sensitive
    /// footer strip; `None` keeps it help-overlay-only. See
    /// [`super::keymap::FooterHint`] for the merge/rank/display rules — the
    /// same mechanism [`super::keymap::Binding`] uses.
    pub footer: Option<FooterHint>,
}

impl<A: Clone + 'static> ModalBinding<A> {
    /// The display label for this row's keys, computed from `keys` (joined
    /// with `" / "`) rather than stored as a static string — so a
    /// `[keys.<mode>]` override that replaces `keys` is reflected
    /// automatically in the help overlay and footer strip, exactly like
    /// `super::keymap::Binding::key_label` does for the main keymap.
    pub(super) fn key_label(&self) -> String {
        self.keys
            .iter()
            .map(|k| k.label())
            .collect::<Vec<_>>()
            .join(" / ")
    }
}

/// Resolves an incoming event against a table, returning the matched row's
/// action. This is the single dispatch primitive the table-driven modal
/// handlers share, so a handler accepts exactly the keys its table documents.
pub(super) fn resolve<A: Copy + Clone>(table: &[ModalBinding<A>], key: KeyEvent) -> Option<A> {
    table
        .iter()
        .find(|b| b.keys.iter().any(|k| k.matches(key)))
        .map(|b| b.action)
}

// -- List mode -------------------------------------------------------------

/// What a key does in the annotation-list panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ListAction {
    MoveDown,
    MoveUp,
    Jump,
    Edit,
    Delete,
    Close,
}

pub(super) fn list_action_name(action: ListAction) -> &'static str {
    match action {
        ListAction::MoveDown => "move-down",
        ListAction::MoveUp => "move-up",
        ListAction::Jump => "jump",
        ListAction::Edit => "edit",
        ListAction::Delete => "delete",
        ListAction::Close => "close",
    }
}

pub(super) fn list_action_from_name(name: &str) -> Option<ListAction> {
    Some(match name {
        "move-down" => ListAction::MoveDown,
        "move-up" => ListAction::MoveUp,
        "jump" => ListAction::Jump,
        "edit" => ListAction::Edit,
        "delete" => ListAction::Delete,
        "close" => ListAction::Close,
        _ => return None,
    })
}

pub(super) static LIST_KEYS: LazyLock<Vec<ModalBinding<ListAction>>> = LazyLock::new(|| {
    vec![
        ModalBinding {
            description: "Move focus down",
            keys: vec![ModalKey::plain(KeyCode::Char('j'))],
            action: ListAction::MoveDown,
            footer: Some(FooterHint {
                rank: 1,
                label: "move",
            }),
        },
        ModalBinding {
            description: "Move focus up",
            keys: vec![ModalKey::plain(KeyCode::Char('k'))],
            action: ListAction::MoveUp,
            footer: Some(FooterHint {
                rank: 1,
                label: "move",
            }),
        },
        ModalBinding {
            description: "Jump to annotation",
            keys: vec![ModalKey::plain(KeyCode::Enter)],
            action: ListAction::Jump,
            footer: Some(FooterHint {
                rank: 2,
                label: "open",
            }),
        },
        ModalBinding {
            description: "Edit",
            keys: vec![ModalKey::plain(KeyCode::Char('e'))],
            action: ListAction::Edit,
            footer: Some(FooterHint {
                rank: 3,
                label: "edit",
            }),
        },
        ModalBinding {
            description: "Delete",
            keys: vec![ModalKey::plain(KeyCode::Char('d'))],
            action: ListAction::Delete,
            footer: Some(FooterHint {
                rank: 4,
                label: "delete",
            }),
        },
        ModalBinding {
            description: "Close panel",
            keys: vec![
                ModalKey::plain(KeyCode::Char('a')),
                ModalKey::plain(KeyCode::Esc),
            ],
            action: ListAction::Close,
            footer: Some(FooterHint {
                rank: 5,
                label: "close",
            }),
        },
    ]
});

// -- Staging panel ---------------------------------------------------------

/// What a key does in the staging panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StagingAction {
    MoveDown,
    MoveUp,
    Unstage,
    Close,
}

pub(super) fn staging_action_name(action: StagingAction) -> &'static str {
    match action {
        StagingAction::MoveDown => "move-down",
        StagingAction::MoveUp => "move-up",
        StagingAction::Unstage => "unstage",
        StagingAction::Close => "close",
    }
}

pub(super) fn staging_action_from_name(name: &str) -> Option<StagingAction> {
    Some(match name {
        "move-down" => StagingAction::MoveDown,
        "move-up" => StagingAction::MoveUp,
        "unstage" => StagingAction::Unstage,
        "close" => StagingAction::Close,
        _ => return None,
    })
}

pub(super) static STAGING_KEYS: LazyLock<Vec<ModalBinding<StagingAction>>> = LazyLock::new(|| {
    vec![
        ModalBinding {
            description: "Move focus down",
            keys: vec![ModalKey::plain(KeyCode::Char('j'))],
            action: StagingAction::MoveDown,
            footer: Some(FooterHint {
                rank: 1,
                label: "move",
            }),
        },
        ModalBinding {
            description: "Move focus up",
            keys: vec![ModalKey::plain(KeyCode::Char('k'))],
            action: StagingAction::MoveUp,
            footer: Some(FooterHint {
                rank: 1,
                label: "move",
            }),
        },
        ModalBinding {
            description: "Unstage file",
            keys: vec![
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
            description: "Close panel",
            keys: vec![
                ModalKey::plain(KeyCode::Char('s')),
                ModalKey::plain(KeyCode::Esc),
            ],
            action: StagingAction::Close,
            footer: Some(FooterHint {
                rank: 3,
                label: "close",
            }),
        },
    ]
});

// -- Accepted-files panel ---------------------------------------------------

/// What a key does in the accepted-files panel — the review-session
/// analogue of the staging panel (`Mode::Staging` is shared between the two;
/// `super::modes::handle_staging_key` resolves against this table instead of
/// [`STAGING_KEYS`] whenever `App::in_review_session()` holds, so the two
/// panels' key handling can never cross-contaminate). Same physical keys as
/// [`StagingAction`] (`j`/`k` move, `Space`/`Enter` act on the focused entry,
/// `s`/`Esc` close) but review-appropriate descriptions, since "Unstage
/// file" would be untruthful here — a review session's `git status` is
/// always clean, there is nothing to unstage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AcceptedPanelAction {
    MoveDown,
    MoveUp,
    /// Un-accepts the focused entry (see [`super::app::App::un_accept_focused_file`]).
    UnAccept,
    Close,
}

/// The accepted-files panel's key table, for the help overlay, footer strip,
/// and [`super::modes::handle_staging_key`]'s review-session dispatch. Not
/// config-remappable yet — see module doc.
pub(super) static ACCEPTED_PANEL_KEYS: LazyLock<Vec<ModalBinding<AcceptedPanelAction>>> =
    LazyLock::new(|| {
        vec![
            ModalBinding {
                description: "Move focus down",
                keys: vec![ModalKey::plain(KeyCode::Char('j'))],
                action: AcceptedPanelAction::MoveDown,
                footer: Some(FooterHint {
                    rank: 1,
                    label: "move",
                }),
            },
            ModalBinding {
                description: "Move focus up",
                keys: vec![ModalKey::plain(KeyCode::Char('k'))],
                action: AcceptedPanelAction::MoveUp,
                footer: Some(FooterHint {
                    rank: 1,
                    label: "move",
                }),
            },
            ModalBinding {
                description: "Un-accept file (re-expands its section)",
                keys: vec![
                    ModalKey::plain(KeyCode::Char(' ')),
                    ModalKey::plain(KeyCode::Enter),
                ],
                action: AcceptedPanelAction::UnAccept,
                footer: Some(FooterHint {
                    rank: 2,
                    label: "un-accept",
                }),
            },
            ModalBinding {
                description: "Close panel",
                keys: vec![
                    ModalKey::plain(KeyCode::Char('s')),
                    ModalKey::plain(KeyCode::Esc),
                ],
                action: AcceptedPanelAction::Close,
                footer: Some(FooterHint {
                    rank: 3,
                    label: "close",
                }),
            },
        ]
    });

// -- Peek overlay ----------------------------------------------------------

/// What a key does in the LSP peek overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PeekAction {
    MoveDown,
    MoveUp,
    Enter,
    Close,
}

pub(super) fn peek_action_name(action: PeekAction) -> &'static str {
    match action {
        PeekAction::MoveDown => "move-down",
        PeekAction::MoveUp => "move-up",
        PeekAction::Enter => "enter",
        PeekAction::Close => "close",
    }
}

pub(super) fn peek_action_from_name(name: &str) -> Option<PeekAction> {
    Some(match name {
        "move-down" => PeekAction::MoveDown,
        "move-up" => PeekAction::MoveUp,
        "enter" => PeekAction::Enter,
        "close" => PeekAction::Close,
        _ => return None,
    })
}

pub(super) static PEEK_KEYS: LazyLock<Vec<ModalBinding<PeekAction>>> = LazyLock::new(|| {
    vec![
        ModalBinding {
            description: "Move selection / scroll hover down",
            keys: vec![ModalKey::plain(KeyCode::Char('j'))],
            action: PeekAction::MoveDown,
            footer: Some(FooterHint {
                rank: 1,
                label: "move",
            }),
        },
        ModalBinding {
            description: "Move selection / scroll hover up",
            keys: vec![ModalKey::plain(KeyCode::Char('k'))],
            action: PeekAction::MoveUp,
            footer: Some(FooterHint {
                rank: 1,
                label: "move",
            }),
        },
        ModalBinding {
            description: "Jump to location (definition/references)",
            keys: vec![ModalKey::plain(KeyCode::Enter)],
            action: PeekAction::Enter,
            footer: Some(FooterHint {
                rank: 2,
                label: "jump",
            }),
        },
        ModalBinding {
            description: "Close",
            keys: vec![ModalKey::plain(KeyCode::Esc)],
            action: PeekAction::Close,
            footer: Some(FooterHint {
                rank: 3,
                label: "close",
            }),
        },
    ]
});

// -- Switcher modal ----------------------------------------------------------

/// What a key does in the branch/worktree switcher modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SwitcherAction {
    ToggleTab,
    MoveDown,
    MoveUp,
    Confirm,
    Close,
}

pub(super) fn switcher_action_name(action: SwitcherAction) -> &'static str {
    match action {
        SwitcherAction::ToggleTab => "toggle-tab",
        SwitcherAction::MoveDown => "move-down",
        SwitcherAction::MoveUp => "move-up",
        SwitcherAction::Confirm => "confirm",
        SwitcherAction::Close => "close",
    }
}

pub(super) fn switcher_action_from_name(name: &str) -> Option<SwitcherAction> {
    Some(match name {
        "toggle-tab" => SwitcherAction::ToggleTab,
        "move-down" => SwitcherAction::MoveDown,
        "move-up" => SwitcherAction::MoveUp,
        "confirm" => SwitcherAction::Confirm,
        "close" => SwitcherAction::Close,
        _ => return None,
    })
}

pub(super) static SWITCHER_KEYS: LazyLock<Vec<ModalBinding<SwitcherAction>>> =
    LazyLock::new(|| {
        vec![
            ModalBinding {
                description: "Switch tab (Branches / Worktrees)",
                keys: vec![
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
                description: "Move selection down",
                keys: vec![
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
                description: "Move selection up",
                keys: vec![
                    ModalKey::plain(KeyCode::Char('k')),
                    ModalKey::plain(KeyCode::Up),
                ],
                action: SwitcherAction::MoveUp,
                // Not also tagged: its label ("k / Up") is already a compound
                // key display, so merging it with MoveDown's would double up
                // the " / " separators in the footer merge. The MoveDown
                // row's own label reads fine alone.
                footer: None,
            },
            ModalBinding {
                description: "Switch to the selected branch/worktree",
                keys: vec![ModalKey::plain(KeyCode::Enter)],
                action: SwitcherAction::Confirm,
                footer: Some(FooterHint {
                    rank: 3,
                    label: "switch",
                }),
            },
            ModalBinding {
                description: "Close",
                keys: vec![ModalKey::plain(KeyCode::Esc)],
                action: SwitcherAction::Close,
                footer: Some(FooterHint {
                    rank: 4,
                    label: "close",
                }),
            },
        ]
    });

// -- End-review modal --------------------------------------------------------

/// What a key does in the end-review modal (`q` in a review session): the
/// three exits, each labeled with its consequence in [`END_REVIEW_KEYS`]'s
/// `description` rather than just "pause"/"finish" (per the spec's design
/// note that the modal must name what happens to the worktree) — plus a
/// lazygit-style highlighted-selection pair (`MoveDown`/`MoveUp`) and
/// `Confirm` (`Enter`, acting on whichever of the three exits is
/// highlighted). The mnemonics (`p`/`f`/`c`/`Esc`) keep dispatching
/// immediately regardless of the highlight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EndReviewAction {
    /// Quit emitting nothing; the worktree, review state, and every
    /// annotation (already durable via save-on-change) are kept.
    Pause,
    /// Emit the complete annotation set (restored-from-earlier-sessions and
    /// this session's own, together, exactly once), remove the worktree
    /// (and delete the persisted review-state entry — statuses and
    /// annotations together), then quit. On removal failure the modal
    /// closes with the git error surfaced instead of quitting.
    Finish,
    /// Close the modal and keep reviewing; nothing happens.
    Cancel,
    /// Move the highlighted option down (see [`super::app::App::end_review_move_down`]).
    MoveDown,
    /// Move the highlighted option up (see [`super::app::App::end_review_move_up`]).
    MoveUp,
    /// Act on whichever option is currently highlighted — resolved via
    /// [`EndReviewAction::from_cursor`] and then dispatched exactly like the
    /// matching mnemonic (see [`super::modes::handle_end_review_key`]).
    Confirm,
}

impl EndReviewAction {
    /// The highest valid cursor value (Cancel, the last of the three
    /// options in the modal's display order) — the clamp
    /// [`super::app::App::end_review_move_down`] moves against.
    pub(super) const LAST_CURSOR: usize = 2;

    /// Maps a highlighted-option cursor (0/1/2, the modal's display order)
    /// to the mnemonic action it stands in for, so `Confirm` (`Enter`) acts
    /// on whichever option is highlighted. Any out-of-range cursor
    /// (defensively, since nothing should ever produce one — see
    /// [`super::app::App::end_review_move_down`]'s clamp) falls back to
    /// `Cancel`, the least destructive option, rather than panicking.
    pub(super) fn from_cursor(cursor: usize) -> EndReviewAction {
        match cursor {
            0 => EndReviewAction::Pause,
            1 => EndReviewAction::Finish,
            _ => EndReviewAction::Cancel,
        }
    }
}

/// End-review modal control keys (`j`/`k`/arrow selection plus `Enter`
/// confirm), for the help overlay, footer strip,
/// [`super::modes::handle_end_review_key`]'s dispatch, and the modal's own
/// body ([`super::end_review_modal`]). Not config-remappable yet — see
/// module doc.
pub(super) static END_REVIEW_KEYS: LazyLock<Vec<ModalBinding<EndReviewAction>>> =
    LazyLock::new(|| {
        vec![
            ModalBinding {
                description: "Pause — quit, emit nothing (keep worktree)",
                keys: vec![ModalKey::plain(KeyCode::Char('p'))],
                action: EndReviewAction::Pause,
                footer: Some(FooterHint {
                    rank: 1,
                    label: "pause",
                }),
            },
            ModalBinding {
                description: "Finish — emit annotations once, remove worktree, quit",
                keys: vec![ModalKey::plain(KeyCode::Char('f'))],
                action: EndReviewAction::Finish,
                footer: Some(FooterHint {
                    rank: 2,
                    label: "finish",
                }),
            },
            ModalBinding {
                description: "Cancel — close this modal, keep reviewing",
                keys: vec![
                    ModalKey::plain(KeyCode::Char('c')),
                    ModalKey::plain(KeyCode::Esc),
                ],
                action: EndReviewAction::Cancel,
                footer: Some(FooterHint {
                    rank: 3,
                    label: "cancel",
                }),
            },
            ModalBinding {
                description: "Move highlighted option down",
                keys: vec![
                    ModalKey::plain(KeyCode::Char('j')),
                    ModalKey::plain(KeyCode::Down),
                ],
                action: EndReviewAction::MoveDown,
                footer: Some(FooterHint {
                    rank: 4,
                    label: "move",
                }),
            },
            ModalBinding {
                description: "Move highlighted option up",
                keys: vec![
                    ModalKey::plain(KeyCode::Char('k')),
                    ModalKey::plain(KeyCode::Up),
                ],
                action: EndReviewAction::MoveUp,
                // Not tagged — same reasoning as SWITCHER_KEYS's MoveUp row.
                footer: None,
            },
            ModalBinding {
                description: "Confirm the highlighted option",
                keys: vec![ModalKey::plain(KeyCode::Enter)],
                action: EndReviewAction::Confirm,
                footer: Some(FooterHint {
                    rank: 5,
                    label: "confirm",
                }),
            },
        ]
    });

// -- Pull/push confirm modal --------------------------------------------------

/// What a key does in the pull/push confirm modal (`p`/`P` in a review
/// session): a plain confirm/cancel gate naming the branch under review,
/// opened by [`super::modes::handle_panel_key`] in place of
/// running [`crate::git::RemoteOp::Pull`]/[`crate::git::RemoteOp::Push`]
/// immediately (`f` fetch is unaffected — reviewers are expected to fetch).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConfirmRemoteOpAction {
    /// Runs the pending op (see [`super::app::App::confirm_remote_op`]).
    Confirm,
    /// Closes the modal, running nothing (see
    /// [`super::app::App::cancel_confirm_remote_op`]).
    Cancel,
}

/// The pull/push confirm modal's key table, for the help overlay, footer
/// strip, and [`super::modes::handle_confirm_remote_op_key`]'s dispatch. Not
/// config-remappable yet — see module doc.
pub(super) static CONFIRM_REMOTE_OP_KEYS: LazyLock<Vec<ModalBinding<ConfirmRemoteOpAction>>> =
    LazyLock::new(|| {
        vec![
            ModalBinding {
                description: "Confirm — run the op against the branch under review",
                keys: vec![
                    ModalKey::plain(KeyCode::Enter),
                    ModalKey::plain(KeyCode::Char('y')),
                ],
                action: ConfirmRemoteOpAction::Confirm,
                footer: Some(FooterHint {
                    rank: 1,
                    label: "confirm",
                }),
            },
            ModalBinding {
                description: "Cancel — close this modal, run nothing",
                keys: vec![
                    ModalKey::plain(KeyCode::Esc),
                    ModalKey::plain(KeyCode::Char('n')),
                ],
                action: ConfirmRemoteOpAction::Cancel,
                footer: Some(FooterHint {
                    rank: 2,
                    label: "cancel",
                }),
            },
        ]
    });

// -- Review launcher modal ---------------------------------------------------

/// What a key does in the Review launcher modal (`R`, `Scope::Global` —
/// opens [`super::app::Mode::ReviewLauncher`]): `Tab`/`Shift-Tab`/`h`/`l`
/// switch between the Branches and Commits tabs, `j`/`k`/arrows move the
/// active tab's cursor, `Enter` confirms the highlighted row — starts a
/// branch review on the Branches tab, opens a read-only commit view on the
/// Commits tab — `Esc` closes the modal and restores the mode `R` was
/// pressed from, and `a` toggles the Commits tab's data source between
/// ahead-of-base and the full recent-HEAD log. Same shape as
/// [`SwitcherAction`] for the first five — tab toggle, cursor pair, confirm,
/// close — plus the one launcher-specific row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LauncherAction {
    ToggleTab,
    MoveDown,
    MoveUp,
    /// Acts on the highlighted row of the active tab (see
    /// [`super::app::App::review_launcher_confirm`]).
    Confirm,
    Close,
    /// The Commits tab's "all commits" toggle — switches its data source
    /// between ahead-of-base and the full recent-HEAD log (see
    /// [`super::app::App::review_launcher_toggle_all_commits`]).
    ToggleAllCommits,
}

pub(super) fn launcher_action_name(action: LauncherAction) -> &'static str {
    match action {
        LauncherAction::ToggleTab => "toggle-tab",
        LauncherAction::MoveDown => "move-down",
        LauncherAction::MoveUp => "move-up",
        LauncherAction::Confirm => "confirm",
        LauncherAction::Close => "close",
        LauncherAction::ToggleAllCommits => "toggle-all-commits",
    }
}

pub(super) fn launcher_action_from_name(name: &str) -> Option<LauncherAction> {
    Some(match name {
        "toggle-tab" => LauncherAction::ToggleTab,
        "move-down" => LauncherAction::MoveDown,
        "move-up" => LauncherAction::MoveUp,
        "confirm" => LauncherAction::Confirm,
        "close" => LauncherAction::Close,
        "toggle-all-commits" => LauncherAction::ToggleAllCommits,
        _ => return None,
    })
}

/// The Review launcher's key table, for the help overlay, footer strip,
/// [`super::modes::handle_review_launcher_key`]'s dispatch, and the
/// `[keys.review-launcher]` config override (see `super::modal_keys_config`).
pub(super) static REVIEW_LAUNCHER_KEYS: LazyLock<Vec<ModalBinding<LauncherAction>>> =
    LazyLock::new(|| {
        vec![
            ModalBinding {
                description: "Switch tab (Branches / Commits)",
                keys: vec![
                    ModalKey::plain(KeyCode::Tab),
                    ModalKey::plain(KeyCode::BackTab),
                    ModalKey::plain(KeyCode::Char('h')),
                    ModalKey::plain(KeyCode::Char('l')),
                    ModalKey::plain(KeyCode::Left),
                    ModalKey::plain(KeyCode::Right),
                ],
                action: LauncherAction::ToggleTab,
                footer: Some(FooterHint {
                    rank: 1,
                    label: "switch tab",
                }),
            },
            ModalBinding {
                description: "Move selection down",
                keys: vec![
                    ModalKey::plain(KeyCode::Char('j')),
                    ModalKey::plain(KeyCode::Down),
                ],
                action: LauncherAction::MoveDown,
                footer: Some(FooterHint {
                    rank: 2,
                    label: "move",
                }),
            },
            ModalBinding {
                description: "Move selection up",
                keys: vec![
                    ModalKey::plain(KeyCode::Char('k')),
                    ModalKey::plain(KeyCode::Up),
                ],
                action: LauncherAction::MoveUp,
                // Not tagged — same reasoning as SWITCHER_KEYS's MoveUp row.
                footer: None,
            },
            ModalBinding {
                description: "Confirm the highlighted row",
                keys: vec![ModalKey::plain(KeyCode::Enter)],
                action: LauncherAction::Confirm,
                footer: Some(FooterHint {
                    rank: 3,
                    label: "confirm",
                }),
            },
            ModalBinding {
                description: "Close",
                keys: vec![ModalKey::plain(KeyCode::Esc)],
                action: LauncherAction::Close,
                footer: Some(FooterHint {
                    rank: 4,
                    label: "close",
                }),
            },
            ModalBinding {
                description: "Commits tab: toggle between ahead-of-base and all commits",
                keys: vec![ModalKey::plain(KeyCode::Char('a'))],
                action: LauncherAction::ToggleAllCommits,
                footer: Some(FooterHint {
                    rank: 5,
                    label: "all commits",
                }),
            },
        ]
    });

// -- Fuzzy file finder --------------------------------------------------------

/// What a key does in the fuzzy file finder overlay. Free-text input
/// (printable chars extend the query) plus these control keys.
/// `MoveUp`/`MoveDown` are split from the historic single "Move selection"
/// hint row for the same reason [`ComposeAction`]'s cursor motions are —
/// genuinely different actions need independently remappable rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FinderAction {
    MoveUp,
    MoveDown,
    Open,
    Close,
    DeleteChar,
}

pub(super) fn finder_action_name(action: FinderAction) -> &'static str {
    match action {
        FinderAction::MoveUp => "move-up",
        FinderAction::MoveDown => "move-down",
        FinderAction::Open => "open",
        FinderAction::Close => "close",
        FinderAction::DeleteChar => "delete-char",
    }
}

pub(super) fn finder_action_from_name(name: &str) -> Option<FinderAction> {
    Some(match name {
        "move-up" => FinderAction::MoveUp,
        "move-down" => FinderAction::MoveDown,
        "open" => FinderAction::Open,
        "close" => FinderAction::Close,
        "delete-char" => FinderAction::DeleteChar,
        _ => return None,
    })
}

/// Fuzzy file finder control keys, for the help overlay,
/// footer strip, and [`super::modes::handle_finder_key`]'s dispatch. `Up`/
/// `Down` (not `j`/`k`) navigate results — `j`/`k` must stay typeable into the
/// query, unlike the switcher modal (which has no free-text input to
/// protect).
pub(super) static FINDER_HINTS: LazyLock<Vec<ModalBinding<FinderAction>>> = LazyLock::new(|| {
    vec![
        ModalBinding {
            description: "Move selection up",
            keys: vec![ModalKey::plain(KeyCode::Up)],
            action: FinderAction::MoveUp,
            // Same `FooterHint` as `MoveDown` below (rank *and* label
            // identical): `super::footer::modal_hints`'s merge combines the
            // two rows' key labels into one "Up/Down move" footer entry, the
            // same as the pre-split single "Move selection" row rendered.
            footer: Some(FooterHint {
                rank: 1,
                label: "move",
            }),
        },
        ModalBinding {
            description: "Move selection down",
            keys: vec![ModalKey::plain(KeyCode::Down)],
            action: FinderAction::MoveDown,
            footer: Some(FooterHint {
                rank: 1,
                label: "move",
            }),
        },
        ModalBinding {
            description: "Open the selected file (read-only whole-file view)",
            keys: vec![ModalKey::plain(KeyCode::Enter)],
            action: FinderAction::Open,
            footer: Some(FooterHint {
                rank: 2,
                label: "open",
            }),
        },
        ModalBinding {
            description: "Close (returns to the prior view unchanged)",
            keys: vec![ModalKey::plain(KeyCode::Esc)],
            action: FinderAction::Close,
            footer: Some(FooterHint {
                rank: 3,
                label: "close",
            }),
        },
        ModalBinding {
            description: "Delete character",
            keys: vec![ModalKey::plain(KeyCode::Backspace)],
            action: FinderAction::DeleteChar,
            footer: None,
        },
    ]
});

// -- Project Search -----------------------------------------------------------

/// What a key does in the full-screen Project Search view while
/// [`super::project_search::SearchFocus::Input`] has focus. Free-text input
/// plus these control keys; `MoveUp`/`MoveDown` are split from a single
/// "Move result selection" row for the same reason [`ComposeAction`]'s
/// cursor motions are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProjectSearchInputAction {
    MoveUp,
    MoveDown,
    Open,
    FocusResults,
    ToggleFocus,
    DeleteChar,
    ToggleCase,
    ToggleWholeWord,
    ToggleLiteral,
}

pub(super) fn project_search_input_action_name(action: ProjectSearchInputAction) -> &'static str {
    use ProjectSearchInputAction::*;
    match action {
        MoveUp => "move-up",
        MoveDown => "move-down",
        Open => "open",
        FocusResults => "focus-results",
        ToggleFocus => "toggle-focus",
        DeleteChar => "delete-char",
        ToggleCase => "toggle-case",
        ToggleWholeWord => "toggle-whole-word",
        ToggleLiteral => "toggle-literal",
    }
}

pub(super) fn project_search_input_action_from_name(
    name: &str,
) -> Option<ProjectSearchInputAction> {
    use ProjectSearchInputAction::*;
    Some(match name {
        "move-up" => MoveUp,
        "move-down" => MoveDown,
        "open" => Open,
        "focus-results" => FocusResults,
        "toggle-focus" => ToggleFocus,
        "delete-char" => DeleteChar,
        "toggle-case" => ToggleCase,
        "toggle-whole-word" => ToggleWholeWord,
        "toggle-literal" => ToggleLiteral,
        _ => return None,
    })
}

/// Project Search control keys while [`super::project_search::SearchFocus::Input`]
/// has focus, for the help overlay, footer strip, and
/// [`super::modes::handle_project_search_key`]'s dispatch. `Up`/`Down` (not
/// `j`/`k`) navigate results here for the same reason the finder's do —
/// `j`/`k`/`c`/`w`/`r` must stay typeable into the query, so only the
/// `Alt`-chorded forms of the toggle letters are bound. See
/// [`PROJECT_SEARCH_RESULTS_HINTS`] for the other focus's table.
pub(super) static PROJECT_SEARCH_INPUT_HINTS: LazyLock<
    Vec<ModalBinding<ProjectSearchInputAction>>,
> = LazyLock::new(|| {
    use ProjectSearchInputAction::*;
    vec![
        ModalBinding {
            description: "Move result selection up",
            keys: vec![ModalKey::plain(KeyCode::Up)],
            action: MoveUp,
            footer: Some(FooterHint {
                rank: 1,
                label: "move",
            }),
        },
        ModalBinding {
            description: "Move result selection down",
            keys: vec![ModalKey::plain(KeyCode::Down)],
            action: MoveDown,
            footer: Some(FooterHint {
                rank: 1,
                label: "move",
            }),
        },
        ModalBinding {
            description: "Open the selected result (read-only whole-file view, cursor on the hit)",
            keys: vec![ModalKey::plain(KeyCode::Enter)],
            action: Open,
            footer: Some(FooterHint {
                rank: 2,
                label: "open",
            }),
        },
        ModalBinding {
            description: "Move focus to the results list (view stays open)",
            keys: vec![ModalKey::plain(KeyCode::Esc)],
            action: FocusResults,
            footer: Some(FooterHint {
                rank: 3,
                label: "results",
            }),
        },
        ModalBinding {
            description: "Toggle focus between input and results",
            keys: vec![ModalKey::plain(KeyCode::Tab)],
            action: ToggleFocus,
            footer: Some(FooterHint {
                rank: 4,
                label: "focus",
            }),
        },
        ModalBinding {
            description: "Delete character",
            keys: vec![ModalKey::plain(KeyCode::Backspace)],
            action: DeleteChar,
            footer: None,
        },
        ModalBinding {
            description: "Cycle case sensitivity (smart / sensitive / insensitive)",
            keys: vec![ModalKey::alt(KeyCode::Char('c'))],
            action: ToggleCase,
            footer: Some(FooterHint {
                rank: 5,
                label: "case",
            }),
        },
        ModalBinding {
            description: "Toggle whole-word matching",
            keys: vec![ModalKey::alt(KeyCode::Char('w'))],
            action: ToggleWholeWord,
            footer: Some(FooterHint {
                rank: 6,
                label: "word",
            }),
        },
        ModalBinding {
            description: "Toggle regex / literal matching",
            keys: vec![ModalKey::alt(KeyCode::Char('r'))],
            action: ToggleLiteral,
            footer: Some(FooterHint {
                rank: 7,
                label: "regex",
            }),
        },
    ]
});

/// What a key does in the full-screen Project Search view while
/// [`super::project_search::SearchFocus::Results`] has focus. Nothing types
/// into the query from here, so every letter key is a genuine control
/// action, not just the `Alt`-chorded toggles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProjectSearchResultsAction {
    EditQuery,
    Close,
    MoveUp,
    MoveDown,
    Open,
    ToggleFocus,
    ToggleCase,
    ToggleWholeWord,
    ToggleLiteral,
}

pub(super) fn project_search_results_action_name(
    action: ProjectSearchResultsAction,
) -> &'static str {
    use ProjectSearchResultsAction::*;
    match action {
        EditQuery => "edit-query",
        Close => "close",
        MoveUp => "move-up",
        MoveDown => "move-down",
        Open => "open",
        ToggleFocus => "toggle-focus",
        ToggleCase => "toggle-case",
        ToggleWholeWord => "toggle-whole-word",
        ToggleLiteral => "toggle-literal",
    }
}

pub(super) fn project_search_results_action_from_name(
    name: &str,
) -> Option<ProjectSearchResultsAction> {
    use ProjectSearchResultsAction::*;
    Some(match name {
        "edit-query" => EditQuery,
        "close" => Close,
        "move-up" => MoveUp,
        "move-down" => MoveDown,
        "open" => Open,
        "toggle-focus" => ToggleFocus,
        "toggle-case" => ToggleCase,
        "toggle-whole-word" => ToggleWholeWord,
        "toggle-literal" => ToggleLiteral,
        _ => return None,
    })
}

/// Project Search control keys while
/// [`super::project_search::SearchFocus::Results`] has focus, for the help
/// overlay, footer strip, and [`super::modes::handle_project_search_key`]'s
/// dispatch (focus-gated inline, not a separate handler function — the drift
/// cross-check runs this table against that same handler with the app forced
/// into Results focus). `j`/`k` are free to navigate results here, matching
/// the plain-letter convention every other list surface
/// ([`LIST_KEYS`]/[`STAGING_KEYS`]/[`PEEK_KEYS`]) already uses, and `/`
/// returns to Input focus (query preserved).
pub(super) static PROJECT_SEARCH_RESULTS_HINTS: LazyLock<
    Vec<ModalBinding<ProjectSearchResultsAction>>,
> = LazyLock::new(|| {
    use ProjectSearchResultsAction::*;
    vec![
        ModalBinding {
            description: "Edit query (focus input; query preserved)",
            keys: vec![ModalKey::plain(KeyCode::Char('/'))],
            action: EditQuery,
            footer: Some(FooterHint {
                rank: 1,
                label: "edit query",
            }),
        },
        ModalBinding {
            description: "Close (returns to the exact prior diff position)",
            keys: vec![ModalKey::plain(KeyCode::Esc)],
            action: Close,
            footer: Some(FooterHint {
                rank: 2,
                label: "back",
            }),
        },
        ModalBinding {
            description: "Move result selection up",
            keys: vec![
                ModalKey::plain(KeyCode::Char('k')),
                ModalKey::plain(KeyCode::Up),
            ],
            action: MoveUp,
            footer: Some(FooterHint {
                rank: 3,
                label: "move",
            }),
        },
        ModalBinding {
            description: "Move result selection down",
            keys: vec![
                ModalKey::plain(KeyCode::Char('j')),
                ModalKey::plain(KeyCode::Down),
            ],
            action: MoveDown,
            footer: Some(FooterHint {
                rank: 3,
                label: "move",
            }),
        },
        ModalBinding {
            description: "Open the selected result (read-only whole-file view, cursor on the hit)",
            keys: vec![ModalKey::plain(KeyCode::Enter)],
            action: Open,
            footer: Some(FooterHint {
                rank: 4,
                label: "open",
            }),
        },
        ModalBinding {
            description: "Toggle focus between input and results",
            keys: vec![ModalKey::plain(KeyCode::Tab)],
            action: ToggleFocus,
            footer: Some(FooterHint {
                rank: 5,
                label: "focus",
            }),
        },
        ModalBinding {
            description: "Cycle case sensitivity (smart / sensitive / insensitive)",
            keys: vec![ModalKey::alt(KeyCode::Char('c'))],
            action: ToggleCase,
            footer: Some(FooterHint {
                rank: 6,
                label: "case",
            }),
        },
        ModalBinding {
            description: "Toggle whole-word matching",
            keys: vec![ModalKey::alt(KeyCode::Char('w'))],
            action: ToggleWholeWord,
            footer: Some(FooterHint {
                rank: 7,
                label: "word",
            }),
        },
        ModalBinding {
            description: "Toggle regex / literal matching",
            keys: vec![ModalKey::alt(KeyCode::Char('r'))],
            action: ToggleLiteral,
            footer: Some(FooterHint {
                rank: 8,
                label: "regex",
            }),
        },
    ]
});

// -- Help overlay ----------------------------------------------------------

/// What a key does while the help overlay is open (it scrolls, since the
/// binding list can outgrow the screen, or closes). Not rendered as an overlay
/// section — these keys already ride the overlay's bottom-border footer — but
/// kept here so [`super::handle_help_key`] dispatches off the table and the
/// drift cross-check covers it too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HelpAction {
    Close,
    ScrollDown,
    ScrollUp,
    PageDown,
    PageUp,
    Top,
    Bottom,
    /// Starts filtering the keybind list (see
    /// [`super::help::HelpOverlayState::search`]).
    Search,
}

pub(super) fn help_action_name(action: HelpAction) -> &'static str {
    match action {
        HelpAction::Close => "close",
        HelpAction::ScrollDown => "scroll-down",
        HelpAction::ScrollUp => "scroll-up",
        HelpAction::PageDown => "page-down",
        HelpAction::PageUp => "page-up",
        HelpAction::Top => "top",
        HelpAction::Bottom => "bottom",
        HelpAction::Search => "search",
    }
}

pub(super) fn help_action_from_name(name: &str) -> Option<HelpAction> {
    Some(match name {
        "close" => HelpAction::Close,
        "scroll-down" => HelpAction::ScrollDown,
        "scroll-up" => HelpAction::ScrollUp,
        "page-down" => HelpAction::PageDown,
        "page-up" => HelpAction::PageUp,
        "top" => HelpAction::Top,
        "bottom" => HelpAction::Bottom,
        "search" => HelpAction::Search,
        _ => return None,
    })
}

pub(super) static HELP_KEYS: LazyLock<Vec<ModalBinding<HelpAction>>> = LazyLock::new(|| {
    vec![
        ModalBinding {
            description: "Close help",
            keys: vec![
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
            description: "Scroll down",
            keys: vec![
                ModalKey::plain(KeyCode::Char('j')),
                ModalKey::plain(KeyCode::Down),
            ],
            action: HelpAction::ScrollDown,
            // `ScrollUp` isn't tagged either — same reasoning as
            // SWITCHER_KEYS's MoveUp row.
            footer: Some(FooterHint {
                rank: 1,
                label: "scroll",
            }),
        },
        ModalBinding {
            description: "Scroll up",
            keys: vec![
                ModalKey::plain(KeyCode::Char('k')),
                ModalKey::plain(KeyCode::Up),
            ],
            action: HelpAction::ScrollUp,
            footer: None,
        },
        ModalBinding {
            description: "Page down",
            keys: vec![ModalKey::plain(KeyCode::PageDown)],
            action: HelpAction::PageDown,
            footer: None,
        },
        ModalBinding {
            description: "Page up",
            keys: vec![ModalKey::plain(KeyCode::PageUp)],
            action: HelpAction::PageUp,
            footer: None,
        },
        ModalBinding {
            description: "Scroll to top",
            keys: vec![
                ModalKey::plain(KeyCode::Char('g')),
                ModalKey::plain(KeyCode::Home),
            ],
            action: HelpAction::Top,
            footer: None,
        },
        ModalBinding {
            description: "Scroll to bottom",
            keys: vec![
                ModalKey::plain(KeyCode::Char('G')),
                ModalKey::plain(KeyCode::End),
            ],
            action: HelpAction::Bottom,
            footer: None,
        },
        ModalBinding {
            description: "Filter keybinds",
            keys: vec![ModalKey::plain(KeyCode::Char('/'))],
            action: HelpAction::Search,
            footer: Some(FooterHint {
                rank: 2,
                label: "filter",
            }),
        },
    ]
});

// -- Help overlay filter (hint-only) ---------------------------------------

/// What a key does while editing the help overlay's `/` filter. Free-text
/// input plus these three control keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HelpSearchAction {
    Lock,
    Clear,
    DeleteChar,
}

pub(super) fn help_search_action_name(action: HelpSearchAction) -> &'static str {
    match action {
        HelpSearchAction::Lock => "lock",
        HelpSearchAction::Clear => "clear",
        HelpSearchAction::DeleteChar => "delete-char",
    }
}

pub(super) fn help_search_action_from_name(name: &str) -> Option<HelpSearchAction> {
    Some(match name {
        "lock" => HelpSearchAction::Lock,
        "clear" => HelpSearchAction::Clear,
        "delete-char" => HelpSearchAction::DeleteChar,
        _ => return None,
    })
}

/// Help-filter control keys, for the overlay's own hint text and
/// [`super::handle_help_key`]'s filter-editing dispatch. Unlike
/// `COMPOSE_HINTS`/`SEARCH_HINTS`, `Enter` here doesn't submit-and-leave — it
/// locks the filter in and hands control back to `HELP_KEYS`' scroll keys, so
/// its description says that explicitly.
pub(super) static HELP_SEARCH_HINTS: LazyLock<Vec<ModalBinding<HelpSearchAction>>> =
    LazyLock::new(|| {
        vec![
            ModalBinding {
                description: "Lock in the filter (scroll keys resume)",
                keys: vec![ModalKey::plain(KeyCode::Enter)],
                action: HelpSearchAction::Lock,
                footer: None,
            },
            ModalBinding {
                description: "Clear the filter",
                keys: vec![ModalKey::plain(KeyCode::Esc)],
                action: HelpSearchAction::Clear,
                footer: None,
            },
            ModalBinding {
                description: "Delete character",
                keys: vec![ModalKey::plain(KeyCode::Backspace)],
                action: HelpSearchAction::DeleteChar,
                footer: None,
            },
        ]
    });

// -- Shared buffer-editing actions (Compose + commit-message) ---------------

/// The text-editing control actions Compose and the commit-message modal
/// share verbatim — both wrap a [`super::compose::TextBuffer`] and offer the
/// identical desktop-editor motion/delete keymap around it, differing only
/// in their lifecycle keys (Compose additionally cycles a classification;
/// both cancel/submit differently — see
/// [`ComposeAction`]/[`CommitMessageAction`]). One shared enum (rather than
/// two copies) keeps the two modes' editing behavior from drifting apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BufferEditAction {
    Newline,
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    WordLeft,
    WordRight,
    LineStart,
    LineEnd,
    DocStart,
    DocEnd,
    DeleteBack,
    DeleteForward,
    DeleteWordBack,
    DeleteWordForward,
}

impl BufferEditAction {
    /// Applies this edit to `buffer` — the single place Compose's and the
    /// commit-message modal's resolved control actions funnel into.
    pub(super) fn apply(self, buffer: &mut super::compose::TextBuffer) {
        use BufferEditAction::*;
        match self {
            Newline => buffer.newline(),
            MoveLeft => buffer.move_left(),
            MoveRight => buffer.move_right(),
            MoveUp => buffer.move_up(),
            MoveDown => buffer.move_down(),
            WordLeft => buffer.move_word_left(),
            WordRight => buffer.move_word_right(),
            LineStart => buffer.move_line_start(),
            LineEnd => buffer.move_line_end(),
            DocStart => buffer.move_doc_start(),
            DocEnd => buffer.move_doc_end(),
            DeleteBack => buffer.backspace(),
            DeleteForward => buffer.delete_forward(),
            DeleteWordBack => buffer.delete_word_back(),
            DeleteWordForward => buffer.delete_word_forward(),
        }
    }
}

/// The kebab-case config action-name for every [`BufferEditAction`] variant,
/// shared by `[keys.compose]`'s and `[keys.commit-message]`'s namespaces (see
/// [`compose_action_name`]/[`commit_message_action_name`]).
pub(super) fn buffer_edit_action_name(action: BufferEditAction) -> &'static str {
    use BufferEditAction::*;
    match action {
        Newline => "newline",
        MoveLeft => "move-left",
        MoveRight => "move-right",
        MoveUp => "move-up",
        MoveDown => "move-down",
        WordLeft => "word-left",
        WordRight => "word-right",
        LineStart => "line-start",
        LineEnd => "line-end",
        DocStart => "doc-start",
        DocEnd => "doc-end",
        DeleteBack => "delete-back",
        DeleteForward => "delete-forward",
        DeleteWordBack => "delete-word-back",
        DeleteWordForward => "delete-word-forward",
    }
}

/// Reverse of [`buffer_edit_action_name`].
pub(super) fn buffer_edit_action_from_name(name: &str) -> Option<BufferEditAction> {
    use BufferEditAction::*;
    Some(match name {
        "newline" => Newline,
        "move-left" => MoveLeft,
        "move-right" => MoveRight,
        "move-up" => MoveUp,
        "move-down" => MoveDown,
        "word-left" => WordLeft,
        "word-right" => WordRight,
        "line-start" => LineStart,
        "line-end" => LineEnd,
        "doc-start" => DocStart,
        "doc-end" => DocEnd,
        "delete-back" => DeleteBack,
        "delete-forward" => DeleteForward,
        "delete-word-back" => DeleteWordBack,
        "delete-word-forward" => DeleteWordForward,
        _ => return None,
    })
}

// -- Compose modal -----------------------------------------------------------

/// What a key does in the Compose modal. Every row here is a documented
/// *control* key; bare printable-character insertion is never an action —
/// [`super::modes::handle_compose_key`] resolves against [`COMPOSE_HINTS`]
/// first and only inserts a literal character for an unresolved, unmodified
/// `Char`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ComposeAction {
    Cancel,
    Submit,
    CycleClassification,
    Edit(BufferEditAction),
}

pub(super) fn compose_action_name(action: ComposeAction) -> &'static str {
    match action {
        ComposeAction::Cancel => "cancel",
        ComposeAction::Submit => "submit",
        ComposeAction::CycleClassification => "cycle-classification",
        ComposeAction::Edit(edit) => buffer_edit_action_name(edit),
    }
}

pub(super) fn compose_action_from_name(name: &str) -> Option<ComposeAction> {
    Some(match name {
        "cancel" => ComposeAction::Cancel,
        "submit" => ComposeAction::Submit,
        "cycle-classification" => ComposeAction::CycleClassification,
        other => ComposeAction::Edit(buffer_edit_action_from_name(other)?),
    })
}

/// Compose-mode control keys, for the help overlay, footer, and
/// [`super::modes::handle_compose_key`]'s dispatch. Compose is free-text
/// input (printable chars insert) *plus* these control keys, so the handler
/// resolves against this table first and falls back to character insertion
/// only for an unresolved, unmodified `Char` (see [`ComposeAction`]'s doc).
/// `MoveLeft`/`MoveRight`/`MoveUp`/`MoveDown` are four separate rows because
/// each documented control key must be independently remappable, and a user
/// can't remap "all four arrow keys at once" as a single action without
/// losing directionality.
pub(super) static COMPOSE_HINTS: LazyLock<Vec<ModalBinding<ComposeAction>>> = LazyLock::new(|| {
    vec![
        ModalBinding {
            description: "Submit",
            keys: vec![ModalKey::plain(KeyCode::Enter)],
            action: ComposeAction::Submit,
            footer: Some(FooterHint {
                rank: 1,
                label: "save",
            }),
        },
        ModalBinding {
            description: "Cancel",
            keys: vec![ModalKey::plain(KeyCode::Esc)],
            action: ComposeAction::Cancel,
            footer: Some(FooterHint {
                rank: 2,
                label: "discard",
            }),
        },
        ModalBinding {
            description: "Insert newline (Shift-Enter needs a kitty-capable terminal)",
            keys: vec![
                ModalKey::shift(KeyCode::Enter),
                ModalKey::ctrl(KeyCode::Char('j')),
            ],
            action: ComposeAction::Edit(BufferEditAction::Newline),
            footer: None,
        },
        ModalBinding {
            description: "Cycle classification",
            keys: vec![ModalKey::ctrl(KeyCode::Char('t'))],
            action: ComposeAction::CycleClassification,
            footer: None,
        },
        ModalBinding {
            description: "Move cursor left",
            keys: vec![ModalKey::plain(KeyCode::Left)],
            action: ComposeAction::Edit(BufferEditAction::MoveLeft),
            footer: None,
        },
        ModalBinding {
            description: "Move cursor right",
            keys: vec![ModalKey::plain(KeyCode::Right)],
            action: ComposeAction::Edit(BufferEditAction::MoveRight),
            footer: None,
        },
        ModalBinding {
            description: "Move cursor up",
            keys: vec![ModalKey::plain(KeyCode::Up)],
            action: ComposeAction::Edit(BufferEditAction::MoveUp),
            footer: None,
        },
        ModalBinding {
            description: "Move cursor down",
            keys: vec![ModalKey::plain(KeyCode::Down)],
            action: ComposeAction::Edit(BufferEditAction::MoveDown),
            footer: None,
        },
        ModalBinding {
            description: "Move word left",
            keys: vec![
                ModalKey::ctrl(KeyCode::Left),
                ModalKey::alt(KeyCode::Left),
                ModalKey::alt(KeyCode::Char('b')),
            ],
            action: ComposeAction::Edit(BufferEditAction::WordLeft),
            footer: None,
        },
        ModalBinding {
            description: "Move word right",
            keys: vec![
                ModalKey::ctrl(KeyCode::Right),
                ModalKey::alt(KeyCode::Right),
                ModalKey::alt(KeyCode::Char('f')),
            ],
            action: ComposeAction::Edit(BufferEditAction::WordRight),
            footer: None,
        },
        ModalBinding {
            description: "Move to line start",
            keys: vec![
                ModalKey::plain(KeyCode::Home),
                ModalKey::ctrl(KeyCode::Char('a')),
            ],
            action: ComposeAction::Edit(BufferEditAction::LineStart),
            footer: None,
        },
        ModalBinding {
            description: "Move to line end",
            keys: vec![
                ModalKey::plain(KeyCode::End),
                ModalKey::ctrl(KeyCode::Char('e')),
            ],
            action: ComposeAction::Edit(BufferEditAction::LineEnd),
            footer: None,
        },
        ModalBinding {
            description: "Move to document start",
            keys: vec![ModalKey::ctrl(KeyCode::Home)],
            action: ComposeAction::Edit(BufferEditAction::DocStart),
            footer: None,
        },
        ModalBinding {
            description: "Move to document end",
            keys: vec![ModalKey::ctrl(KeyCode::End)],
            action: ComposeAction::Edit(BufferEditAction::DocEnd),
            footer: None,
        },
        ModalBinding {
            description: "Delete character before the cursor",
            keys: vec![ModalKey::plain(KeyCode::Backspace)],
            action: ComposeAction::Edit(BufferEditAction::DeleteBack),
            footer: None,
        },
        ModalBinding {
            description: "Delete character at the cursor",
            keys: vec![ModalKey::plain(KeyCode::Delete)],
            action: ComposeAction::Edit(BufferEditAction::DeleteForward),
            footer: None,
        },
        ModalBinding {
            description: "Delete word before the cursor",
            keys: vec![
                ModalKey::ctrl(KeyCode::Backspace),
                ModalKey::alt(KeyCode::Backspace),
                ModalKey::ctrl(KeyCode::Char('w')),
                ModalKey::ctrl(KeyCode::Char('h')),
            ],
            action: ComposeAction::Edit(BufferEditAction::DeleteWordBack),
            footer: None,
        },
        ModalBinding {
            description: "Delete word at the cursor",
            keys: vec![
                ModalKey::ctrl(KeyCode::Delete),
                ModalKey::alt(KeyCode::Char('d')),
            ],
            action: ComposeAction::Edit(BufferEditAction::DeleteWordForward),
            footer: None,
        },
    ]
});

// -- Commit-message modal -----------------------------------------------------

/// What a key does in the commit-message modal. Same shape as
/// [`ComposeAction`] minus classification cycling — see that type's doc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CommitMessageAction {
    Cancel,
    Submit,
    Edit(BufferEditAction),
}

pub(super) fn commit_message_action_name(action: CommitMessageAction) -> &'static str {
    match action {
        CommitMessageAction::Cancel => "cancel",
        CommitMessageAction::Submit => "submit",
        CommitMessageAction::Edit(edit) => buffer_edit_action_name(edit),
    }
}

pub(super) fn commit_message_action_from_name(name: &str) -> Option<CommitMessageAction> {
    Some(match name {
        "cancel" => CommitMessageAction::Cancel,
        "submit" => CommitMessageAction::Submit,
        other => CommitMessageAction::Edit(buffer_edit_action_from_name(other)?),
    })
}

/// Commit-message control keys, for the help overlay, footer, and
/// [`super::modes::handle_commit_message_key`]'s dispatch. Like Compose, the
/// modal is free-text input *plus* these control keys; see [`COMPOSE_HINTS`]'s
/// doc for why "Move cursor" is four separate rows.
pub(super) static COMMIT_MESSAGE_HINTS: LazyLock<Vec<ModalBinding<CommitMessageAction>>> =
    LazyLock::new(|| {
        vec![
            ModalBinding {
                description: "Commit staged changes with this message",
                keys: vec![ModalKey::plain(KeyCode::Enter)],
                action: CommitMessageAction::Submit,
                footer: Some(FooterHint {
                    rank: 1,
                    label: "commit",
                }),
            },
            ModalBinding {
                description: "Cancel back to the git panel",
                keys: vec![ModalKey::plain(KeyCode::Esc)],
                action: CommitMessageAction::Cancel,
                footer: Some(FooterHint {
                    rank: 2,
                    label: "cancel",
                }),
            },
            ModalBinding {
                description: "Insert newline / body line (Shift-Enter needs a kitty-capable terminal)",
                keys: vec![
                    ModalKey::shift(KeyCode::Enter),
                    ModalKey::ctrl(KeyCode::Char('j')),
                ],
                action: CommitMessageAction::Edit(BufferEditAction::Newline),
                footer: None,
            },
            ModalBinding {
                description: "Move cursor left",
                keys: vec![ModalKey::plain(KeyCode::Left)],
                action: CommitMessageAction::Edit(BufferEditAction::MoveLeft),
                footer: None,
            },
            ModalBinding {
                description: "Move cursor right",
                keys: vec![ModalKey::plain(KeyCode::Right)],
                action: CommitMessageAction::Edit(BufferEditAction::MoveRight),
                footer: None,
            },
            ModalBinding {
                description: "Move cursor up",
                keys: vec![ModalKey::plain(KeyCode::Up)],
                action: CommitMessageAction::Edit(BufferEditAction::MoveUp),
                footer: None,
            },
            ModalBinding {
                description: "Move cursor down",
                keys: vec![ModalKey::plain(KeyCode::Down)],
                action: CommitMessageAction::Edit(BufferEditAction::MoveDown),
                footer: None,
            },
            ModalBinding {
                description: "Move word left",
                keys: vec![
                    ModalKey::ctrl(KeyCode::Left),
                    ModalKey::alt(KeyCode::Left),
                    ModalKey::alt(KeyCode::Char('b')),
                ],
                action: CommitMessageAction::Edit(BufferEditAction::WordLeft),
                footer: None,
            },
            ModalBinding {
                description: "Move word right",
                keys: vec![
                    ModalKey::ctrl(KeyCode::Right),
                    ModalKey::alt(KeyCode::Right),
                    ModalKey::alt(KeyCode::Char('f')),
                ],
                action: CommitMessageAction::Edit(BufferEditAction::WordRight),
                footer: None,
            },
            ModalBinding {
                description: "Move to line start",
                keys: vec![
                    ModalKey::plain(KeyCode::Home),
                    ModalKey::ctrl(KeyCode::Char('a')),
                ],
                action: CommitMessageAction::Edit(BufferEditAction::LineStart),
                footer: None,
            },
            ModalBinding {
                description: "Move to line end",
                keys: vec![
                    ModalKey::plain(KeyCode::End),
                    ModalKey::ctrl(KeyCode::Char('e')),
                ],
                action: CommitMessageAction::Edit(BufferEditAction::LineEnd),
                footer: None,
            },
            ModalBinding {
                description: "Move to document start",
                keys: vec![ModalKey::ctrl(KeyCode::Home)],
                action: CommitMessageAction::Edit(BufferEditAction::DocStart),
                footer: None,
            },
            ModalBinding {
                description: "Move to document end",
                keys: vec![ModalKey::ctrl(KeyCode::End)],
                action: CommitMessageAction::Edit(BufferEditAction::DocEnd),
                footer: None,
            },
            ModalBinding {
                description: "Delete character before the cursor",
                keys: vec![ModalKey::plain(KeyCode::Backspace)],
                action: CommitMessageAction::Edit(BufferEditAction::DeleteBack),
                footer: None,
            },
            ModalBinding {
                description: "Delete character at the cursor",
                keys: vec![ModalKey::plain(KeyCode::Delete)],
                action: CommitMessageAction::Edit(BufferEditAction::DeleteForward),
                footer: None,
            },
            ModalBinding {
                description: "Delete word before the cursor",
                keys: vec![
                    ModalKey::ctrl(KeyCode::Backspace),
                    ModalKey::alt(KeyCode::Backspace),
                    ModalKey::ctrl(KeyCode::Char('w')),
                    ModalKey::ctrl(KeyCode::Char('h')),
                ],
                action: CommitMessageAction::Edit(BufferEditAction::DeleteWordBack),
                footer: None,
            },
            ModalBinding {
                description: "Delete word at the cursor",
                keys: vec![
                    ModalKey::ctrl(KeyCode::Delete),
                    ModalKey::alt(KeyCode::Char('d')),
                ],
                action: CommitMessageAction::Edit(BufferEditAction::DeleteWordForward),
                footer: None,
            },
        ]
    });

// -- Search input -------------------------------------------------------------

/// What a key does in the diff view's `/` search input. Free-text input plus
/// these three control keys; bare printable-character insertion is never an
/// action, matching every other free-text mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SearchAction {
    Confirm,
    Cancel,
    DeleteChar,
}

pub(super) fn search_action_name(action: SearchAction) -> &'static str {
    match action {
        SearchAction::Confirm => "confirm",
        SearchAction::Cancel => "cancel",
        SearchAction::DeleteChar => "delete-char",
    }
}

pub(super) fn search_action_from_name(name: &str) -> Option<SearchAction> {
    Some(match name {
        "confirm" => SearchAction::Confirm,
        "cancel" => SearchAction::Cancel,
        "delete-char" => SearchAction::DeleteChar,
        _ => return None,
    })
}

/// Search-input control keys, for the help overlay, footer, and
/// [`super::handle_search_key`]'s dispatch.
pub(super) static SEARCH_HINTS: LazyLock<Vec<ModalBinding<SearchAction>>> = LazyLock::new(|| {
    vec![
        ModalBinding {
            description: "Confirm search",
            keys: vec![ModalKey::plain(KeyCode::Enter)],
            action: SearchAction::Confirm,
            footer: None,
        },
        ModalBinding {
            description: "Cancel (clears pattern if buffer empty)",
            keys: vec![ModalKey::plain(KeyCode::Esc)],
            action: SearchAction::Cancel,
            footer: None,
        },
        ModalBinding {
            description: "Delete character",
            keys: vec![ModalKey::plain(KeyCode::Backspace)],
            action: SearchAction::DeleteChar,
            footer: None,
        },
    ]
});

// -- Effective (post-config-override) modal tables --------------------------

/// The canonical `[keys.<mode>]` table names, in
/// the same order [`ModalKeymaps`]'s fields are declared. One table per modal
/// mode currently defined in this module; adding a fourteenth mode means
/// adding both a field here and a name here, which
/// `crate::config::keys::KeysConfig::from_value`'s parallel hardcoded list
/// must also gain (that module can't import this one — see its layering
/// note — so `crate::ui::modal_keys_config`'s tests cross-check the two
/// lists agree). Test-only: nothing in production code needs the list as
/// data (dispatch is the exhaustive, compiler-checked match in
/// [`ModalKeymaps`]'s construction), only the cross-check test does.
#[cfg(test)]
pub(super) const MODAL_MODE_NAMES: &[&str] = &[
    "list",
    "staging",
    "peek",
    "switcher",
    "review-launcher",
    "help",
    "help-search",
    "compose",
    "commit-message",
    "search",
    "finder",
    "project-search-input",
    "project-search-results",
];

/// Every modal mode's *effective* table — [`LazyLock`] defaults above, each
/// with its `[keys.<mode>]` config overrides already applied — built exactly
/// once in [`super::run`] alongside
/// [`super::keymap_config::effective_keymap`] and stored on
/// [`super::app::App`] so every handler and render call reads a plain owned
/// table with no per-keystroke parsing. [`Default`] (every `App` built
/// without an explicit config load, i.e. every pre-existing unit test) gives
/// back exactly the compiled-in defaults, unmodified.
#[derive(Clone)]
pub struct ModalKeymaps {
    pub(super) list: Vec<ModalBinding<ListAction>>,
    pub(super) staging: Vec<ModalBinding<StagingAction>>,
    pub(super) peek: Vec<ModalBinding<PeekAction>>,
    pub(super) switcher: Vec<ModalBinding<SwitcherAction>>,
    /// The Review launcher modal (`R`, `Scope::Global`).
    pub(super) review_launcher: Vec<ModalBinding<LauncherAction>>,
    pub(super) help: Vec<ModalBinding<HelpAction>>,
    pub(super) help_search: Vec<ModalBinding<HelpSearchAction>>,
    pub(super) compose: Vec<ModalBinding<ComposeAction>>,
    pub(super) commit_message: Vec<ModalBinding<CommitMessageAction>>,
    pub(super) search: Vec<ModalBinding<SearchAction>>,
    pub(super) finder: Vec<ModalBinding<FinderAction>>,
    pub(super) project_search_input: Vec<ModalBinding<ProjectSearchInputAction>>,
    pub(super) project_search_results: Vec<ModalBinding<ProjectSearchResultsAction>>,
    /// The end-review modal. Not config-remappable yet — see
    /// [`END_REVIEW_KEYS`] and the module doc.
    pub(super) end_review: Vec<ModalBinding<EndReviewAction>>,
    /// The accepted-files panel (review sessions' analogue of `staging`).
    /// Not config-remappable yet — see [`ACCEPTED_PANEL_KEYS`].
    pub(super) accepted_panel: Vec<ModalBinding<AcceptedPanelAction>>,
    /// The pull/push confirm modal. Not config-remappable yet — see
    /// [`CONFIRM_REMOTE_OP_KEYS`].
    pub(super) confirm_remote_op: Vec<ModalBinding<ConfirmRemoteOpAction>>,
}

impl Default for ModalKeymaps {
    fn default() -> ModalKeymaps {
        ModalKeymaps {
            list: LIST_KEYS.clone(),
            staging: STAGING_KEYS.clone(),
            peek: PEEK_KEYS.clone(),
            switcher: SWITCHER_KEYS.clone(),
            review_launcher: REVIEW_LAUNCHER_KEYS.clone(),
            help: HELP_KEYS.clone(),
            help_search: HELP_SEARCH_HINTS.clone(),
            compose: COMPOSE_HINTS.clone(),
            commit_message: COMMIT_MESSAGE_HINTS.clone(),
            search: SEARCH_HINTS.clone(),
            finder: FINDER_HINTS.clone(),
            project_search_input: PROJECT_SEARCH_INPUT_HINTS.clone(),
            project_search_results: PROJECT_SEARCH_RESULTS_HINTS.clone(),
            end_review: END_REVIEW_KEYS.clone(),
            accepted_panel: ACCEPTED_PANEL_KEYS.clone(),
            confirm_remote_op: CONFIRM_REMOTE_OP_KEYS.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotate::{Classification, Target};
    use crate::diff::FileDiff;
    use crate::git::{DiffTarget, RawFilePatch};
    use crate::lsp::SourceLocation;
    use crate::ui::modes::{
        handle_compose_key, handle_list_key, handle_peek_key, handle_search_key, handle_staging_key,
    };
    use crate::ui::project_search::SearchFocus;
    use crate::ui::{App, Mode, StagedFile, compose, handle_help_key, peek};
    use std::path::PathBuf;

    // -- Bijectivity: every mode's action-name mapping is total and 1:1 -----
    //
    // Mirrors `super::keymap::tests::action_names_are_total_and_bijective`,
    // extended to every modal mode's action enum: every action that appears
    // in the mode's default table gets exactly one name, no two actions
    // share a name, and every name resolves
    // back to the same action via that mode's `*_action_from_name`. Since
    // every enum variant was defined directly from an existing table row
    // (one row per action, `BufferEditAction`'s callers included), iterating
    // the default table exercises every variant — the same "every value
    // appears in the table" argument the main keymap's version relies on.

    /// Runs the bijectivity check for one mode's table/name-pair, called
    /// once per mode below rather than duplicating the loop thirteen times.
    fn assert_action_names_are_total_and_bijective<A: Copy + PartialEq + std::fmt::Debug>(
        table: &[ModalBinding<A>],
        name_of: fn(A) -> &'static str,
        from_name: fn(&str) -> Option<A>,
    ) {
        let mut seen_names = std::collections::HashSet::new();
        let mut seen_actions: Vec<A> = Vec::new();
        for b in table {
            if seen_actions.contains(&b.action) {
                continue;
            }
            seen_actions.push(b.action);
            let name = name_of(b.action);
            assert!(
                seen_names.insert(name),
                "duplicate action name {name:?} (action {:?})",
                b.action
            );
            assert_eq!(
                from_name(name),
                Some(b.action),
                "name {name:?} must resolve back to {:?}",
                b.action
            );
        }
    }

    #[test]
    fn list_action_names_are_total_and_bijective() {
        assert_action_names_are_total_and_bijective(
            &LIST_KEYS,
            list_action_name,
            list_action_from_name,
        );
    }

    #[test]
    fn staging_action_names_are_total_and_bijective() {
        assert_action_names_are_total_and_bijective(
            &STAGING_KEYS,
            staging_action_name,
            staging_action_from_name,
        );
    }

    #[test]
    fn peek_action_names_are_total_and_bijective() {
        assert_action_names_are_total_and_bijective(
            &PEEK_KEYS,
            peek_action_name,
            peek_action_from_name,
        );
    }

    #[test]
    fn switcher_action_names_are_total_and_bijective() {
        assert_action_names_are_total_and_bijective(
            &SWITCHER_KEYS,
            switcher_action_name,
            switcher_action_from_name,
        );
    }

    #[test]
    fn launcher_action_names_are_total_and_bijective() {
        assert_action_names_are_total_and_bijective(
            &REVIEW_LAUNCHER_KEYS,
            launcher_action_name,
            launcher_action_from_name,
        );
    }

    #[test]
    fn help_action_names_are_total_and_bijective() {
        assert_action_names_are_total_and_bijective(
            &HELP_KEYS,
            help_action_name,
            help_action_from_name,
        );
    }

    #[test]
    fn help_search_action_names_are_total_and_bijective() {
        assert_action_names_are_total_and_bijective(
            &HELP_SEARCH_HINTS,
            help_search_action_name,
            help_search_action_from_name,
        );
    }

    #[test]
    fn compose_action_names_are_total_and_bijective() {
        assert_action_names_are_total_and_bijective(
            &COMPOSE_HINTS,
            compose_action_name,
            compose_action_from_name,
        );
    }

    #[test]
    fn commit_message_action_names_are_total_and_bijective() {
        assert_action_names_are_total_and_bijective(
            &COMMIT_MESSAGE_HINTS,
            commit_message_action_name,
            commit_message_action_from_name,
        );
    }

    #[test]
    fn search_action_names_are_total_and_bijective() {
        assert_action_names_are_total_and_bijective(
            &SEARCH_HINTS,
            search_action_name,
            search_action_from_name,
        );
    }

    #[test]
    fn finder_action_names_are_total_and_bijective() {
        assert_action_names_are_total_and_bijective(
            &FINDER_HINTS,
            finder_action_name,
            finder_action_from_name,
        );
    }

    #[test]
    fn project_search_input_action_names_are_total_and_bijective() {
        assert_action_names_are_total_and_bijective(
            &PROJECT_SEARCH_INPUT_HINTS,
            project_search_input_action_name,
            project_search_input_action_from_name,
        );
    }

    #[test]
    fn project_search_results_action_names_are_total_and_bijective() {
        assert_action_names_are_total_and_bijective(
            &PROJECT_SEARCH_RESULTS_HINTS,
            project_search_results_action_name,
            project_search_results_action_from_name,
        );
    }

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
        for binding in LIST_KEYS.iter() {
            for key in &binding.keys {
                let mut app = list_app();
                let label = binding.key_label();
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
        for binding in STAGING_KEYS.iter() {
            for key in &binding.keys {
                let mut app = staging_app();
                let label = binding.key_label();
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

    /// A two-file `FileDiff` fixture (path parameterized) for the
    /// accepted-panel tests below, mirroring `sample_file()`'s shape.
    fn named_file(path: &str) -> FileDiff {
        let raw = format!(
            "diff --git a/{path} b/{path}\nindex 1..2 100644\n--- a/{path}\n+++ b/{path}\n@@ -1,1 +1,1 @@\n-old\n+new\n"
        );
        FileDiff::from_patch(&RawFilePatch {
            path: path.to_string(),
            old_path: None,
            raw,
            is_binary: false,
        })
        .unwrap()
    }

    /// An `App` in a review session (`Mode::Staging`, both `a.rs`/`b.rs`
    /// accepted) with the accepted-files panel populated the way
    /// `App::toggle_staging_panel` would — via the real
    /// `App::refresh_accepted_list`, not a hand-built `staged` list, so
    /// these tests exercise the actual production wiring.
    fn accepted_panel_app() -> App {
        let mut app = App::new(vec![named_file("a.rs"), named_file("b.rs")]);
        app.target = DiffTarget::Review {
            base: "main".to_string(),
            branch: "feature".to_string(),
        };
        app.review_states
            .insert("a.rs".to_string(), crate::review::ReviewStatus::Accepted);
        app.review_states
            .insert("b.rs".to_string(), crate::review::ReviewStatus::Accepted);
        app.view.set_collapsed("a.rs", true);
        app.view.set_collapsed("b.rs", true);
        app.refresh_accepted_list();
        app.mode = Mode::Staging;
        app
    }

    /// Every `ACCEPTED_PANEL_KEYS` entry, fed through the *real* handler as
    /// the key event it documents, must perform the action it documents —
    /// the review-session mirror of `every_staging_table_entry_drives_its_documented_action`.
    #[test]
    fn every_accepted_panel_table_entry_drives_its_documented_action() {
        for binding in ACCEPTED_PANEL_KEYS.iter() {
            for key in &binding.keys {
                let mut app = accepted_panel_app();
                let label = binding.key_label();
                match binding.action {
                    AcceptedPanelAction::MoveDown => {
                        handle_staging_key(&mut app, key.event());
                        assert_eq!(
                            app.staging_cursor, 1,
                            "Accepted panel {label}: focus moves down"
                        );
                    }
                    AcceptedPanelAction::MoveUp => {
                        app.staging_cursor = 1;
                        handle_staging_key(&mut app, key.event());
                        assert_eq!(
                            app.staging_cursor, 0,
                            "Accepted panel {label}: focus moves up"
                        );
                    }
                    AcceptedPanelAction::UnAccept => {
                        handle_staging_key(&mut app, key.event());
                        assert_eq!(
                            app.review_status("a.rs"),
                            crate::review::ReviewStatus::Unreviewed,
                            "Accepted panel {label}: un-accept must act"
                        );
                        assert!(
                            !app.view.is_collapsed("a.rs"),
                            "Accepted panel {label}: un-accept must re-expand"
                        );
                    }
                    AcceptedPanelAction::Close => {
                        handle_staging_key(&mut app, key.event());
                        assert_eq!(app.mode, Mode::Normal, "Accepted panel {label}: must close");
                    }
                }
            }
        }
    }

    /// The mirror-image half of the drift test: outside a review session,
    /// `ACCEPTED_PANEL_KEYS`' own keys still reach `handle_staging_key`, but
    /// resolve through the *local* `STAGING_KEYS` table instead — proving
    /// the two tables never both apply at once for the same `Mode::Staging`
    /// session, and that the local behavior is genuinely untouched (not
    /// merely coincidentally identical).
    #[test]
    fn accepted_panel_keys_do_not_apply_outside_a_review_session() {
        let mut app = staging_app();
        assert_eq!(app.target, DiffTarget::WorkingTree);
        let space = ACCEPTED_PANEL_KEYS
            .iter()
            .find(|b| b.action == AcceptedPanelAction::UnAccept)
            .expect("UnAccept row exists")
            .keys[0];
        handle_staging_key(&mut app, space.event());
        // Resolves as `StagingAction::Unstage` (no git backend -> a footer
        // message), never `AcceptedPanelAction::UnAccept`'s review-status
        // mutation — there is no `review_states` entry to have touched.
        assert!(app.status_message.is_some());
        assert!(app.review_states.is_empty());
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
        for binding in PEEK_KEYS.iter() {
            for key in &binding.keys {
                let mut app = peek_app();
                let label = binding.key_label();
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

        for binding in SWITCHER_KEYS.iter() {
            for key in &binding.keys {
                let mut app = switcher_app();
                let label = binding.key_label();
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

    /// An `App` in the Review launcher, Branches tab, opened from
    /// `Mode::Normal` with no git backend attached — so `ToggleTab`/`Close`
    /// have a visible effect, while `MoveDown`/`MoveUp`/`Confirm` are no-ops
    /// against the empty branch list a backend-less open produces (real
    /// branch-list/confirm coverage lives in `review_launcher.rs`'s own
    /// tests and `review_launcher_integration_tests.rs`'s real-git flows).
    fn launcher_app() -> App {
        let mut app = app();
        app.open_review_launcher();
        app
    }

    #[test]
    fn every_launcher_table_entry_drives_its_documented_action() {
        use crate::ui::modes::handle_review_launcher_key;
        use crate::ui::review_launcher::LauncherTab;

        for binding in REVIEW_LAUNCHER_KEYS.iter() {
            for key in &binding.keys {
                let mut app = launcher_app();
                let label = binding.key_label();
                match binding.action {
                    LauncherAction::ToggleTab => {
                        handle_review_launcher_key(&mut app, key.event());
                        assert_eq!(
                            app.mode,
                            Mode::ReviewLauncher {
                                tab: LauncherTab::Commits,
                                cursor: 0,
                                origin: crate::ui::app::ModeOrigin::Normal,
                            },
                            "Launcher {label}: must switch tab"
                        );
                    }
                    LauncherAction::MoveDown | LauncherAction::MoveUp => {
                        handle_review_launcher_key(&mut app, key.event());
                        assert_eq!(
                            app.mode,
                            Mode::ReviewLauncher {
                                tab: LauncherTab::Branches,
                                cursor: 0,
                                origin: crate::ui::app::ModeOrigin::Normal,
                            },
                            "Launcher {label}: no list data yet, cursor stays at 0"
                        );
                    }
                    LauncherAction::Confirm => {
                        handle_review_launcher_key(&mut app, key.event());
                        assert!(
                            matches!(app.mode, Mode::ReviewLauncher { .. }),
                            "Launcher {label}: modal stays open (no backend, empty branch list)"
                        );
                    }
                    LauncherAction::Close => {
                        handle_review_launcher_key(&mut app, key.event());
                        assert_eq!(
                            app.mode,
                            Mode::Normal,
                            "Launcher {label}: must close back to the origin mode"
                        );
                    }
                    LauncherAction::ToggleAllCommits => {
                        // Only observable on the Commits tab — switch there
                        // first (`ToggleTab`'s own case above already
                        // proves that key works).
                        app.review_launcher_switch_tab();
                        assert!(!app.launcher_all_commits);
                        handle_review_launcher_key(&mut app, key.event());
                        assert!(
                            app.launcher_all_commits,
                            "Launcher {label}: must toggle all-commits on"
                        );
                    }
                }
            }
        }
    }

    /// An `App` mid-review, with `Mode::EndReview` already open (`origin:
    /// Normal`), so every end-review action has an observable effect.
    fn end_review_app() -> App {
        let mut app = app();
        app.target = crate::git::DiffTarget::Review {
            base: "main".to_string(),
            branch: "feature".to_string(),
        };
        app.open_end_review_modal();
        app
    }

    #[test]
    fn every_end_review_table_entry_drives_its_documented_action() {
        use crate::ui::{Flow, QuitOutcome, modes::handle_end_review_key};

        for binding in END_REVIEW_KEYS.iter() {
            for key in &binding.keys {
                let mut app = end_review_app();
                let label = binding.key_label();
                match binding.action {
                    EndReviewAction::Pause => {
                        let flow = handle_end_review_key(&mut app, key.event());
                        assert!(
                            matches!(flow, Flow::Quit(QuitOutcome::Discard)),
                            "End review {label}: pause must quit emitting nothing (spec 08 Unit 6)"
                        );
                    }
                    EndReviewAction::Finish => {
                        // No `review_origin_ops` attached here (no git
                        // backend in this fixture), so finish degrades to a
                        // footer message and stays open rather than quitting
                        // — still proving `f` reaches `App::finish_review`.
                        let _ = handle_end_review_key(&mut app, key.event());
                        assert_eq!(
                            app.mode,
                            Mode::Normal,
                            "End review {label}: a failed finish closes back to the origin mode"
                        );
                        assert!(app.status_message.is_some());
                    }
                    EndReviewAction::Cancel => {
                        let _ = handle_end_review_key(&mut app, key.event());
                        assert_eq!(
                            app.mode,
                            Mode::Normal,
                            "End review {label}: cancel returns to the origin mode"
                        );
                    }
                    EndReviewAction::MoveDown => {
                        let _ = handle_end_review_key(&mut app, key.event());
                        assert_eq!(
                            app.end_review_cursor(),
                            Some(1),
                            "End review {label}: move-down highlights the next option"
                        );
                    }
                    EndReviewAction::MoveUp => {
                        // Start one row down so the up-motion has an
                        // observable effect (the fixture opens at cursor 0,
                        // already clamped).
                        app.end_review_move_down();
                        let _ = handle_end_review_key(&mut app, key.event());
                        assert_eq!(
                            app.end_review_cursor(),
                            Some(0),
                            "End review {label}: move-up highlights the prior option"
                        );
                    }
                    EndReviewAction::Confirm => {
                        // The fixture opens with Pause (cursor 0)
                        // highlighted, so Enter must behave exactly like
                        // the `p` mnemonic.
                        let flow = handle_end_review_key(&mut app, key.event());
                        assert!(
                            matches!(flow, Flow::Quit(QuitOutcome::Discard)),
                            "End review {label}: confirm on the highlighted Pause option must quit emitting nothing"
                        );
                    }
                }
            }
        }
    }

    /// An `App` mid-review, with `Mode::ConfirmRemoteOp { op: Pull, .. }`
    /// already open, so every confirm/cancel action has an observable
    /// effect.
    fn confirm_remote_op_app() -> App {
        let mut app = app();
        app.target = crate::git::DiffTarget::Review {
            base: "main".to_string(),
            branch: "feature".to_string(),
        };
        app.set_repo_root(PathBuf::from("/tmp/review-worktree"));
        app.mode = Mode::Panel {
            cursor: 0,
            tab: crate::ui::app::PanelTab::Changes,
        };
        app.open_confirm_remote_op_modal(crate::git::RemoteOp::Pull);
        app
    }

    #[test]
    fn every_confirm_remote_op_table_entry_drives_its_documented_action() {
        use crate::ui::modes::handle_confirm_remote_op_key;

        for binding in CONFIRM_REMOTE_OP_KEYS.iter() {
            for key in &binding.keys {
                let mut app = confirm_remote_op_app();
                let label = binding.key_label();
                match binding.action {
                    ConfirmRemoteOpAction::Confirm => {
                        handle_confirm_remote_op_key(&mut app, key.event());
                        assert!(
                            matches!(app.mode, Mode::Panel { .. }),
                            "Confirm remote op {label}: confirm returns to the panel"
                        );
                        assert_eq!(
                            app.running_op_label(),
                            Some("pull"),
                            "Confirm remote op {label}: confirm must spawn the pending op"
                        );
                    }
                    ConfirmRemoteOpAction::Cancel => {
                        handle_confirm_remote_op_key(&mut app, key.event());
                        assert!(
                            matches!(app.mode, Mode::Panel { .. }),
                            "Confirm remote op {label}: cancel returns to the panel"
                        );
                        assert_eq!(
                            app.running_op_label(),
                            None,
                            "Confirm remote op {label}: cancel must run nothing"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn every_help_table_entry_drives_its_documented_action() {
        for binding in HELP_KEYS.iter() {
            for key in &binding.keys {
                let mut app = app();
                app.help.open = true;
                app.help.scroll.set(25);
                app.help.viewport.set(10);
                handle_help_key(&mut app, key.event());
                let label = binding.key_label();
                match binding.action {
                    HelpAction::Close => {
                        assert!(!app.help.open, "Help {label}: must close the overlay");
                        assert_eq!(app.help.scroll.get(), 0);
                    }
                    HelpAction::ScrollDown => {
                        assert_eq!(app.help.scroll.get(), 26, "Help {label}: scrolls down")
                    }
                    HelpAction::ScrollUp => {
                        assert_eq!(app.help.scroll.get(), 24, "Help {label}: scrolls up")
                    }
                    HelpAction::PageDown => {
                        assert_eq!(app.help.scroll.get(), 35, "Help {label}: pages down")
                    }
                    HelpAction::PageUp => {
                        assert_eq!(app.help.scroll.get(), 15, "Help {label}: pages up")
                    }
                    HelpAction::Top => {
                        assert_eq!(app.help.scroll.get(), 0, "Help {label}: jumps to top")
                    }
                    HelpAction::Bottom => {
                        assert_eq!(app.help.scroll.get(), u16::MAX, "Help {label}: to bottom")
                    }
                    HelpAction::Search => {
                        assert_eq!(
                            app.help.search,
                            Some((String::new(), true)),
                            "Help {label}: must start filter-editing with an empty query"
                        );
                        assert_eq!(app.help.scroll.get(), 0, "Help {label}: must reset scroll");
                    }
                }
            }
        }
    }

    /// An `App` mid-help-filter with a non-empty query, so every documented
    /// control key produces an observable state change.
    fn help_search_app() -> App {
        let mut app = app();
        app.help.open = true;
        app.help.search = Some(("ab".to_string(), true));
        app
    }

    #[test]
    fn every_help_search_hint_key_is_consumed_by_the_handler() {
        for binding in HELP_SEARCH_HINTS.iter() {
            for key in &binding.keys {
                let mut app = help_search_app();
                let before = app.help.search.clone();
                handle_help_key(&mut app, key.event());
                assert_ne!(
                    before,
                    app.help.search,
                    "Help filter {}: documented key must be consumed by handle_help_key",
                    binding.key_label()
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
            if resolve(&HELP_SEARCH_HINTS, ev).is_some() {
                continue;
            }
            let mut app = help_search_app();
            let before = app.help.search.clone();
            handle_help_key(&mut app, ev);
            assert_eq!(
                before, app.help.search,
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
        for binding in COMPOSE_HINTS.iter() {
            for key in &binding.keys {
                let mut app = compose_app();
                let before = compose_snapshot(&app);
                handle_compose_key(&mut app, key.event());
                assert_ne!(
                    before,
                    compose_snapshot(&app),
                    "Compose {}: documented key must be consumed by handle_compose_key",
                    binding.key_label()
                );
            }
        }
    }

    /// Control keys the Compose hint table doesn't document must do nothing —
    /// the reverse drift check: a key added to `handle_compose_key` without a
    /// table row fails here. Printable chars are exempt (free-text input).
    #[test]
    fn compose_handler_ignores_control_keys_absent_from_its_table() {
        // Home/End/Delete are meaningful editing keys (line start/end, delete
        // forward) documented in COMPOSE_HINTS, so they belong to the
        // consumed-key test above, not this reverse one.
        let mut universe: Vec<KeyEvent> = [
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Tab,
            KeyCode::BackTab,
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
            if resolve(&COMPOSE_HINTS, ev).is_some() {
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
        for binding in COMMIT_MESSAGE_HINTS.iter() {
            for key in &binding.keys {
                let mut app = commit_message_app();
                let before = commit_message_snapshot(&app);
                handle_commit_message_key(&mut app, key.event());
                assert_ne!(
                    before,
                    commit_message_snapshot(&app),
                    "Commit message {}: documented key must be consumed by handle_commit_message_key",
                    binding.key_label()
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
        // Home/End/Delete are meaningful editing keys documented in
        // COMMIT_MESSAGE_HINTS, covered by the consumed-key test above
        // rather than this reverse one.
        let mut universe: Vec<KeyEvent> = [
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Tab,
            KeyCode::BackTab,
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
            if resolve(&COMMIT_MESSAGE_HINTS, ev).is_some() {
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
        for binding in SEARCH_HINTS.iter() {
            for key in &binding.keys {
                let mut app = search_app();
                let before = search_snapshot(&app);
                handle_search_key(&mut app, key.event());
                assert_ne!(
                    before,
                    search_snapshot(&app),
                    "Search {}: documented key must be consumed by handle_search_key",
                    binding.key_label()
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
            if resolve(&SEARCH_HINTS, ev).is_some() {
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
        for binding in FINDER_HINTS.iter() {
            for key in &binding.keys {
                let mut app = finder_app();
                let before = finder_snapshot(&app);
                handle_finder_key(&mut app, key.event());
                assert_ne!(
                    before,
                    finder_snapshot(&app),
                    "Finder {}: documented key must be consumed by handle_finder_key",
                    binding.key_label()
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
            if resolve(&FINDER_HINTS, ev).is_some() {
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
        assert!(resolve(&LIST_KEYS, ev).is_none());
        assert!(resolve(&STAGING_KEYS, ev).is_none());
        assert!(resolve(&PEEK_KEYS, ev).is_none());
        assert!(resolve(&HELP_KEYS, ev).is_none());
        assert!(resolve(&COMPOSE_HINTS, ev).is_none());
        assert!(resolve(&SEARCH_HINTS, ev).is_none());
        assert!(resolve(&SWITCHER_KEYS, ev).is_none());
        assert!(resolve(&HELP_SEARCH_HINTS, ev).is_none());
        assert!(resolve(&COMMIT_MESSAGE_HINTS, ev).is_none());
        assert!(resolve(&FINDER_HINTS, ev).is_none());
        assert!(resolve(&PROJECT_SEARCH_INPUT_HINTS, ev).is_none());
        assert!(resolve(&PROJECT_SEARCH_RESULTS_HINTS, ev).is_none());
    }

    // -- Project Search mode -------------------------------------------------

    /// An `App` mid-Project-Search with a non-empty query, three hits across
    /// two files, and the cursor on the middle hit — so `Up`/`Down` (each
    /// moving away from the middle) both produce an observable change,
    /// alongside `Enter`/`Esc`/`Backspace`/the three `Alt`-chord toggles.
    /// A minimal `StageOps` fake serving `a.rs`/`b.rs` content, so `Enter`
    /// (opening the selected hit's file view) has an observable effect.
    struct ProjectSearchFakeOps;

    impl crate::ui::stage_ops::StageOps for ProjectSearchFakeOps {
        fn diff(
            &self,
            _target: &crate::git::DiffTarget,
        ) -> Result<Vec<crate::git::RawFilePatch>, crate::git::GitError> {
            Ok(Vec::new())
        }
        fn status(&self) -> Result<Vec<crate::git::FileStatus>, crate::git::GitError> {
            Ok(Vec::new())
        }
        fn stage_file(&self, _path: &str) -> Result<(), crate::git::GitError> {
            Ok(())
        }
        fn unstage_file(&self, _path: &str) -> Result<(), crate::git::GitError> {
            Ok(())
        }
        fn apply_cached(&self, _patch: &str) -> Result<(), crate::git::GitError> {
            Ok(())
        }
        fn unapply_cached(&self, _patch: &str) -> Result<(), crate::git::GitError> {
            Ok(())
        }
        fn read_worktree_file(&self, path: &str) -> Option<Vec<u8>> {
            Some(format!("one\ntwo\n{path}\n").into_bytes())
        }
        fn show_file(&self, _spec: &str) -> Option<String> {
            None
        }
    }

    /// Builds the fixture app in `focus` (the two-focus model — see
    /// [`SearchFocus`]). Cursor starts on the middle
    /// hit of three across two files, so `Up`/`Down`/`j`/`k` (each moving
    /// away from the middle) all produce an observable change.
    fn project_search_app_with_focus(focus: SearchFocus) -> App {
        use crate::search::SearchHit;
        use crate::ui::project_search::{ProjectSearchState, ResultGroup};
        let mut app = app();
        app.stage_ops = Some(Box::new(ProjectSearchFakeOps));
        app.mode = Mode::ProjectSearch;
        #[allow(clippy::single_range_in_vec_init)]
        let hit = |path: &str, line: u64| SearchHit {
            path: path.to_string(),
            line_number: line,
            line_text: "needle".to_string(),
            match_spans: vec![0..6],
            generation: 0,
        };
        let mut state = ProjectSearchState::new(Mode::Normal);
        state.query = "ab".to_string();
        state.groups = vec![
            ResultGroup {
                path: "a.rs".to_string(),
                hits: vec![hit("a.rs", 1), hit("a.rs", 2)],
            },
            ResultGroup {
                path: "b.rs".to_string(),
                hits: vec![hit("b.rs", 1)],
            },
        ];
        state.cursor = 1;
        state.focus = focus;
        app.project_search = Some(state);
        app
    }

    /// Everything a Project Search control key could observably change: the
    /// mode (`Enter`/`Esc` both leave the view one way or another), whether
    /// the view is still open, the query buffer, the selection cursor, the
    /// three toggle states, and which half has focus (`Esc`/`Tab`/`/`).
    fn project_search_snapshot(
        app: &App,
    ) -> (Mode, bool, String, usize, bool, bool, bool, SearchFocus) {
        let state = app.project_search.as_ref();
        (
            app.mode,
            state.is_some(),
            state.map(|s| s.query.clone()).unwrap_or_default(),
            state.map(|s| s.cursor).unwrap_or(0),
            state
                .map(|s| s.case != crate::search::CaseMode::Smart)
                .unwrap_or(false),
            state.map(|s| s.whole_word).unwrap_or(false),
            state.map(|s| s.literal).unwrap_or(false),
            state.map(|s| s.focus).unwrap_or_default(),
        )
    }

    #[test]
    fn every_project_search_input_hint_key_is_consumed_by_the_handler() {
        use crate::ui::modes::handle_project_search_key;
        for binding in PROJECT_SEARCH_INPUT_HINTS.iter() {
            for key in &binding.keys {
                let mut app = project_search_app_with_focus(SearchFocus::Input);
                let before = project_search_snapshot(&app);
                handle_project_search_key(&mut app, key.event());
                assert_ne!(
                    before,
                    project_search_snapshot(&app),
                    "Project Search (Input focus) {}: documented key must be consumed by handle_project_search_key",
                    binding.key_label()
                );
            }
        }
    }

    #[test]
    fn every_project_search_results_hint_key_is_consumed_by_the_handler() {
        use crate::ui::modes::handle_project_search_key;
        for binding in PROJECT_SEARCH_RESULTS_HINTS.iter() {
            for key in &binding.keys {
                let mut app = project_search_app_with_focus(SearchFocus::Results);
                let before = project_search_snapshot(&app);
                handle_project_search_key(&mut app, key.event());
                assert_ne!(
                    before,
                    project_search_snapshot(&app),
                    "Project Search (Results focus) {}: documented key must be consumed by handle_project_search_key",
                    binding.key_label()
                );
            }
        }
    }

    /// The reverse-drift universe shared by both focuses: control keys no
    /// table documents, plus every `Alt`-chorded letter other than
    /// `c`/`w`/`r`. Bare printable letters (including `j`/`k`/`c`/`w`/`r`/`/`
    /// with no Alt) are deliberately excluded — which of those are "typing"
    /// vs. "navigation" flips with focus, and that distinction is exactly
    /// what the per-focus consumed-key tests above already pin.
    fn project_search_control_key_universe() -> Vec<KeyEvent> {
        let mut universe: Vec<KeyEvent> = [
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
        for c in 'a'..='z' {
            universe.push(KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT));
        }
        universe
    }

    /// Reverse drift check for Project Search's Input focus: control keys
    /// outside its table must do nothing, including `Alt`-chords on letters
    /// other than `c`/`w`/`r`. Printable chars (including bare `c`/`w`/`r`
    /// with no Alt) are exempt — they must stay typeable into the query.
    #[test]
    fn project_search_input_focus_ignores_control_keys_absent_from_its_table() {
        use crate::ui::modes::handle_project_search_key;
        for ev in project_search_control_key_universe() {
            if resolve(&PROJECT_SEARCH_INPUT_HINTS, ev).is_some() {
                continue; // documented; covered by the consumed-key test above
            }
            let mut app = project_search_app_with_focus(SearchFocus::Input);
            let before = project_search_snapshot(&app);
            handle_project_search_key(&mut app, ev);
            assert_eq!(
                before,
                project_search_snapshot(&app),
                "handle_project_search_key (Input focus) consumed {ev:?}, which the table doesn't document"
            );
        }
    }

    /// Reverse drift check for Project Search's Results focus: same universe,
    /// checked against [`PROJECT_SEARCH_RESULTS_HINTS`] instead — `j`/`k`/`/`
    /// are documented there (they navigate/switch focus, not type), so they
    /// aren't in this bare-letter-exempt universe to begin with, but `Up`/
    /// `Down`/`Tab` land here as bindings the table check skips correctly.
    #[test]
    fn project_search_results_focus_ignores_control_keys_absent_from_its_table() {
        use crate::ui::modes::handle_project_search_key;
        for ev in project_search_control_key_universe() {
            if resolve(&PROJECT_SEARCH_RESULTS_HINTS, ev).is_some() {
                continue; // documented; covered by the consumed-key test above
            }
            let mut app = project_search_app_with_focus(SearchFocus::Results);
            let before = project_search_snapshot(&app);
            handle_project_search_key(&mut app, ev);
            assert_eq!(
                before,
                project_search_snapshot(&app),
                "handle_project_search_key (Results focus) consumed {ev:?}, which the table doesn't document"
            );
        }
    }

    /// Behavioral pin for the round-1 UX fix's core complaint ("vim motions
    /// don't work in the grep view"): bare `j`/`k` type into the query while
    /// Input-focused, but navigate results once Results-focused — the same
    /// letters, different meaning, purely a function of focus.
    #[test]
    fn bare_j_and_k_type_into_the_query_only_while_input_focused() {
        use crate::ui::modes::handle_project_search_key;

        let mut input_app = project_search_app_with_focus(SearchFocus::Input);
        let cursor_before = input_app.project_search.as_ref().unwrap().cursor;
        handle_project_search_key(
            &mut input_app,
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        );
        assert_eq!(
            input_app.project_search.as_ref().unwrap().query,
            "abj",
            "j must type into the query while Input-focused"
        );
        assert_eq!(
            input_app.project_search.as_ref().unwrap().cursor,
            cursor_before,
            "typing must not move the result selection"
        );

        let mut results_app = project_search_app_with_focus(SearchFocus::Results);
        let query_before = results_app.project_search.as_ref().unwrap().query.clone();
        handle_project_search_key(
            &mut results_app,
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        );
        assert_eq!(
            results_app.project_search.as_ref().unwrap().query,
            query_before,
            "j must not type into the query while Results-focused"
        );
        assert_ne!(
            results_app.project_search.as_ref().unwrap().cursor,
            1,
            "j must move the result selection while Results-focused"
        );
    }
}
