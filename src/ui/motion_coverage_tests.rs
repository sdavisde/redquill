//! FR-5's structural heart: the shared motion set ([`super::motion::Motion`])
//! must resolve in every consuming context — the diff view, the git panel,
//! and every modal list — via that context's *real* dispatch tables (the
//! main [`Keymap`] for diff/panel scope, [`modal_keys`]'s tables for the
//! rest), not a hand-maintained checklist that could quietly drift from the
//! actual bindings. [`motion::covers_all`] is the one generic checker every
//! context's coverage assertion below reuses; the final test in this module
//! proves that checker isn't a tautology by feeding it a real context's
//! resolver with one motion deliberately withheld and confirming it fails.

use super::keymap::{Action, Keymap, Scope};
use super::modal_keys::{
    self, AcceptedPanelAction, LauncherAction, ListAction, PeekAction, StagingAction,
    SwitcherAction,
};
use super::motion::{self, Motion};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

fn ctrl_key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

/// Diff scope's resolver: the pre-existing bindings, including the real
/// two-key `gg` (resolved the same two-call way `Keymap::resolve` requires).
fn diff_resolves(km: &Keymap, m: Motion) -> bool {
    match m {
        Motion::StepDown => km.lookup(key('j')) == Some(Action::CursorDown),
        Motion::StepUp => km.lookup(key('k')) == Some(Action::CursorUp),
        Motion::HalfPageDown => km.lookup(ctrl_key('d')) == Some(Action::HalfPageDown),
        Motion::HalfPageUp => km.lookup(ctrl_key('u')) == Some(Action::HalfPageUp),
        Motion::FullPageDown => km.lookup(ctrl_key('f')) == Some(Action::FullPageDown),
        Motion::FullPageUp => km.lookup(ctrl_key('b')) == Some(Action::FullPageUp),
        Motion::JumpToTop => {
            let mut pending = None;
            assert_eq!(
                km.resolve(&mut pending, key('g')),
                None,
                "g must start a sequence"
            );
            km.resolve(&mut pending, key('g')) == Some(Action::JumpToTop)
        }
        Motion::JumpToBottom => km.lookup(key('G')) == Some(Action::JumpToBottom),
    }
}

/// Panel scope's resolver: single `g`/`G` (not `gg`) per this layer's
/// non-diff convention (see `motion`'s module doc).
fn panel_resolves(km: &Keymap, m: Motion) -> bool {
    match m {
        Motion::StepDown => km.lookup_in(Scope::Panel, key('j')) == Some(Action::PanelCursorDown),
        Motion::StepUp => km.lookup_in(Scope::Panel, key('k')) == Some(Action::PanelCursorUp),
        Motion::HalfPageDown => {
            km.lookup_in(Scope::Panel, ctrl_key('d')) == Some(Action::PanelHalfPageDown)
        }
        Motion::HalfPageUp => {
            km.lookup_in(Scope::Panel, ctrl_key('u')) == Some(Action::PanelHalfPageUp)
        }
        Motion::FullPageDown => {
            km.lookup_in(Scope::Panel, ctrl_key('f')) == Some(Action::PanelFullPageDown)
        }
        Motion::FullPageUp => {
            km.lookup_in(Scope::Panel, ctrl_key('b')) == Some(Action::PanelFullPageUp)
        }
        Motion::JumpToTop => km.lookup_in(Scope::Panel, key('g')) == Some(Action::PanelJumpToTop),
        Motion::JumpToBottom => {
            km.lookup_in(Scope::Panel, key('G')) == Some(Action::PanelJumpToBottom)
        }
    }
}

fn list_resolves(m: Motion) -> bool {
    use modal_keys::LIST_KEYS;
    match m {
        Motion::StepDown => modal_keys::resolve(&LIST_KEYS, key('j')) == Some(ListAction::MoveDown),
        Motion::StepUp => modal_keys::resolve(&LIST_KEYS, key('k')) == Some(ListAction::MoveUp),
        Motion::HalfPageDown => {
            modal_keys::resolve(&LIST_KEYS, ctrl_key('d')) == Some(ListAction::HalfPageDown)
        }
        Motion::HalfPageUp => {
            modal_keys::resolve(&LIST_KEYS, ctrl_key('u')) == Some(ListAction::HalfPageUp)
        }
        Motion::FullPageDown => {
            modal_keys::resolve(&LIST_KEYS, ctrl_key('f')) == Some(ListAction::FullPageDown)
        }
        Motion::FullPageUp => {
            modal_keys::resolve(&LIST_KEYS, ctrl_key('b')) == Some(ListAction::FullPageUp)
        }
        Motion::JumpToTop => {
            modal_keys::resolve(&LIST_KEYS, key('g')) == Some(ListAction::JumpToTop)
        }
        Motion::JumpToBottom => {
            modal_keys::resolve(&LIST_KEYS, key('G')) == Some(ListAction::JumpToBottom)
        }
    }
}

fn staging_resolves(m: Motion) -> bool {
    use modal_keys::STAGING_KEYS;
    match m {
        Motion::StepDown => {
            modal_keys::resolve(&STAGING_KEYS, key('j')) == Some(StagingAction::MoveDown)
        }
        Motion::StepUp => {
            modal_keys::resolve(&STAGING_KEYS, key('k')) == Some(StagingAction::MoveUp)
        }
        Motion::HalfPageDown => {
            modal_keys::resolve(&STAGING_KEYS, ctrl_key('d')) == Some(StagingAction::HalfPageDown)
        }
        Motion::HalfPageUp => {
            modal_keys::resolve(&STAGING_KEYS, ctrl_key('u')) == Some(StagingAction::HalfPageUp)
        }
        Motion::FullPageDown => {
            modal_keys::resolve(&STAGING_KEYS, ctrl_key('f')) == Some(StagingAction::FullPageDown)
        }
        Motion::FullPageUp => {
            modal_keys::resolve(&STAGING_KEYS, ctrl_key('b')) == Some(StagingAction::FullPageUp)
        }
        Motion::JumpToTop => {
            modal_keys::resolve(&STAGING_KEYS, key('g')) == Some(StagingAction::JumpToTop)
        }
        Motion::JumpToBottom => {
            modal_keys::resolve(&STAGING_KEYS, key('G')) == Some(StagingAction::JumpToBottom)
        }
    }
}

fn accepted_panel_resolves(m: Motion) -> bool {
    use modal_keys::ACCEPTED_PANEL_KEYS;
    match m {
        Motion::StepDown => {
            modal_keys::resolve(&ACCEPTED_PANEL_KEYS, key('j'))
                == Some(AcceptedPanelAction::MoveDown)
        }
        Motion::StepUp => {
            modal_keys::resolve(&ACCEPTED_PANEL_KEYS, key('k')) == Some(AcceptedPanelAction::MoveUp)
        }
        Motion::HalfPageDown => {
            modal_keys::resolve(&ACCEPTED_PANEL_KEYS, ctrl_key('d'))
                == Some(AcceptedPanelAction::HalfPageDown)
        }
        Motion::HalfPageUp => {
            modal_keys::resolve(&ACCEPTED_PANEL_KEYS, ctrl_key('u'))
                == Some(AcceptedPanelAction::HalfPageUp)
        }
        Motion::FullPageDown => {
            modal_keys::resolve(&ACCEPTED_PANEL_KEYS, ctrl_key('f'))
                == Some(AcceptedPanelAction::FullPageDown)
        }
        Motion::FullPageUp => {
            modal_keys::resolve(&ACCEPTED_PANEL_KEYS, ctrl_key('b'))
                == Some(AcceptedPanelAction::FullPageUp)
        }
        Motion::JumpToTop => {
            modal_keys::resolve(&ACCEPTED_PANEL_KEYS, key('g'))
                == Some(AcceptedPanelAction::JumpToTop)
        }
        Motion::JumpToBottom => {
            modal_keys::resolve(&ACCEPTED_PANEL_KEYS, key('G'))
                == Some(AcceptedPanelAction::JumpToBottom)
        }
    }
}

fn switcher_resolves(m: Motion) -> bool {
    use modal_keys::SWITCHER_KEYS;
    match m {
        Motion::StepDown => {
            modal_keys::resolve(&SWITCHER_KEYS, key('j')) == Some(SwitcherAction::MoveDown)
        }
        Motion::StepUp => {
            modal_keys::resolve(&SWITCHER_KEYS, key('k')) == Some(SwitcherAction::MoveUp)
        }
        Motion::HalfPageDown => {
            modal_keys::resolve(&SWITCHER_KEYS, ctrl_key('d')) == Some(SwitcherAction::HalfPageDown)
        }
        Motion::HalfPageUp => {
            modal_keys::resolve(&SWITCHER_KEYS, ctrl_key('u')) == Some(SwitcherAction::HalfPageUp)
        }
        Motion::FullPageDown => {
            modal_keys::resolve(&SWITCHER_KEYS, ctrl_key('f')) == Some(SwitcherAction::FullPageDown)
        }
        Motion::FullPageUp => {
            modal_keys::resolve(&SWITCHER_KEYS, ctrl_key('b')) == Some(SwitcherAction::FullPageUp)
        }
        Motion::JumpToTop => {
            modal_keys::resolve(&SWITCHER_KEYS, key('g')) == Some(SwitcherAction::JumpToTop)
        }
        Motion::JumpToBottom => {
            modal_keys::resolve(&SWITCHER_KEYS, key('G')) == Some(SwitcherAction::JumpToBottom)
        }
    }
}

fn peek_resolves(m: Motion) -> bool {
    use modal_keys::PEEK_KEYS;
    match m {
        Motion::StepDown => modal_keys::resolve(&PEEK_KEYS, key('j')) == Some(PeekAction::MoveDown),
        Motion::StepUp => modal_keys::resolve(&PEEK_KEYS, key('k')) == Some(PeekAction::MoveUp),
        Motion::HalfPageDown => {
            modal_keys::resolve(&PEEK_KEYS, ctrl_key('d')) == Some(PeekAction::HalfPageDown)
        }
        Motion::HalfPageUp => {
            modal_keys::resolve(&PEEK_KEYS, ctrl_key('u')) == Some(PeekAction::HalfPageUp)
        }
        Motion::FullPageDown => {
            modal_keys::resolve(&PEEK_KEYS, ctrl_key('f')) == Some(PeekAction::FullPageDown)
        }
        Motion::FullPageUp => {
            modal_keys::resolve(&PEEK_KEYS, ctrl_key('b')) == Some(PeekAction::FullPageUp)
        }
        Motion::JumpToTop => {
            modal_keys::resolve(&PEEK_KEYS, key('g')) == Some(PeekAction::JumpToTop)
        }
        Motion::JumpToBottom => {
            modal_keys::resolve(&PEEK_KEYS, key('G')) == Some(PeekAction::JumpToBottom)
        }
    }
}

/// The Review launcher's resolver (spec 12 FR-13): one shared
/// `REVIEW_LAUNCHER_KEYS` table drives both the Branches and Commits
/// tabs (the actions aren't tab-specific — see `LauncherAction`'s doc), so
/// checking it once covers both, mirroring how `switcher_resolves` above
/// covers the switcher's own two tabs (Branches/Worktrees) through one
/// `SWITCHER_KEYS` table rather than a resolver per tab.
fn launcher_resolves(m: Motion) -> bool {
    use modal_keys::REVIEW_LAUNCHER_KEYS;
    match m {
        Motion::StepDown => {
            modal_keys::resolve(&REVIEW_LAUNCHER_KEYS, key('j')) == Some(LauncherAction::MoveDown)
        }
        Motion::StepUp => {
            modal_keys::resolve(&REVIEW_LAUNCHER_KEYS, key('k')) == Some(LauncherAction::MoveUp)
        }
        Motion::HalfPageDown => {
            modal_keys::resolve(&REVIEW_LAUNCHER_KEYS, ctrl_key('d'))
                == Some(LauncherAction::HalfPageDown)
        }
        Motion::HalfPageUp => {
            modal_keys::resolve(&REVIEW_LAUNCHER_KEYS, ctrl_key('u'))
                == Some(LauncherAction::HalfPageUp)
        }
        Motion::FullPageDown => {
            modal_keys::resolve(&REVIEW_LAUNCHER_KEYS, ctrl_key('f'))
                == Some(LauncherAction::FullPageDown)
        }
        Motion::FullPageUp => {
            modal_keys::resolve(&REVIEW_LAUNCHER_KEYS, ctrl_key('b'))
                == Some(LauncherAction::FullPageUp)
        }
        Motion::JumpToTop => {
            modal_keys::resolve(&REVIEW_LAUNCHER_KEYS, key('g')) == Some(LauncherAction::JumpToTop)
        }
        Motion::JumpToBottom => {
            modal_keys::resolve(&REVIEW_LAUNCHER_KEYS, key('G'))
                == Some(LauncherAction::JumpToBottom)
        }
    }
}

/// FR-5: every consuming context dispatches the complete motion set.
#[test]
fn every_consuming_context_covers_the_full_motion_set() {
    let km = Keymap::default_map();
    assert!(
        motion::covers_all(&Motion::ALL, |m| diff_resolves(&km, m)),
        "diff view (Scope::Diff) is missing a motion"
    );
    assert!(
        motion::covers_all(&Motion::ALL, |m| panel_resolves(&km, m)),
        "git panel (Scope::Panel) is missing a motion"
    );
    assert!(
        motion::covers_all(&Motion::ALL, list_resolves),
        "annotation list is missing a motion"
    );
    assert!(
        motion::covers_all(&Motion::ALL, staging_resolves),
        "staging panel is missing a motion"
    );
    assert!(
        motion::covers_all(&Motion::ALL, accepted_panel_resolves),
        "accepted-files panel is missing a motion"
    );
    assert!(
        motion::covers_all(&Motion::ALL, switcher_resolves),
        "switcher is missing a motion"
    );
    assert!(
        motion::covers_all(&Motion::ALL, peek_resolves),
        "LSP peek is missing a motion"
    );
    assert!(
        motion::covers_all(&Motion::ALL, launcher_resolves),
        "Review launcher (Branches/Commits tabs) is missing a motion"
    );
}

/// Negative proof this suite's checks have teeth: wrapping a real context's
/// resolver (peek's) to withhold exactly one motion — as if a future change
/// forgot to wire it up — must make the identical `covers_all` check the
/// test above uses for every real context fail. If this ever passed, the
/// positive assertions above would be worthless (they'd pass no matter what
/// was actually bound).
#[test]
fn covers_all_would_catch_a_context_missing_one_motion() {
    let missing_jump_to_bottom = |m: Motion| m != Motion::JumpToBottom && peek_resolves(m);
    assert!(
        !motion::covers_all(&Motion::ALL, missing_jump_to_bottom),
        "a context missing JumpToBottom must fail the coverage check"
    );
}
