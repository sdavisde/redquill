use super::*;
use crate::config::KeysConfig;
use crate::config::keys::{ChordSpec, KeySeqSpec};
use crossterm::event::{KeyCode, KeyModifiers};

fn one(code: KeyCode, mods: KeyModifiers) -> Vec<KeySeqSpec> {
    vec![KeySeqSpec::One(ChordSpec { code, mods })]
}

fn find(km: &Keymap, scope: Scope, action: Action) -> Vec<&Binding> {
    km.bindings()
        .iter()
        .filter(|b| b.scope == scope && b.action == action)
        .collect()
}

// -- Replace: an action named in config gets exactly the listed keys --------

#[test]
fn overriding_an_action_replaces_its_default_keys_rather_than_appending() {
    let mut keys = KeysConfig::default();
    keys.diff.insert(
        "next-file".to_string(),
        one(KeyCode::Char('J'), KeyModifiers::NONE),
    );
    let (km, warnings) = effective_keymap(&keys);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");

    let rows = find(&km, Scope::Diff, Action::NextFile);
    assert_eq!(rows.len(), 1, "must have exactly one binding, not appended");
    assert_eq!(rows[0].key_label(), "J");

    // Tab is unbound by default.
    assert_eq!(
        km.lookup_in(
            Scope::Diff,
            crossterm::event::KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)
        ),
        None
    );
    assert_eq!(
        km.lookup_in(
            Scope::Diff,
            crossterm::event::KeyEvent::new(KeyCode::Char('J'), KeyModifiers::NONE)
        ),
        Some(Action::NextFile)
    );
}

#[test]
fn overriding_an_action_with_multiple_keys_binds_all_of_them() {
    let mut keys = KeysConfig::default();
    keys.diff.insert(
        "quit".to_string(),
        vec![
            KeySeqSpec::One(ChordSpec {
                code: KeyCode::Char('q'),
                mods: KeyModifiers::NONE,
            }),
            KeySeqSpec::One(ChordSpec {
                code: KeyCode::Char('k'),
                mods: KeyModifiers::NONE,
            }),
        ],
    );
    let (km, warnings) = effective_keymap(&keys);
    // `q`'s default is now `Scope::Global`, so this override doesn't collide
    // with anything within Diff scope on that key; `k` is CursorUp's
    // Diff-scope default, so overriding `quit` to include `k` collides with
    // it (user wins, one warning) and Quit's Diff-scope override ends up
    // with both keys.
    assert_eq!(warnings.len(), 1, "expected exactly one collision warning");
    let rows = find(&km, Scope::Diff, Action::Quit);
    assert_eq!(rows.len(), 2);
    assert_eq!(
        km.lookup_in(
            Scope::Diff,
            crossterm::event::KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE)
        ),
        Some(Action::Quit)
    );
}

// -- Keep: unlisted actions are untouched ------------------------------------

#[test]
fn unlisted_actions_keep_their_defaults() {
    let mut keys = KeysConfig::default();
    keys.diff.insert(
        "next-file".to_string(),
        one(KeyCode::Char('J'), KeyModifiers::NONE),
    );
    let (km, _warnings) = effective_keymap(&keys);
    // PrevFile (BackTab) is untouched.
    assert_eq!(
        km.lookup_in(
            Scope::Diff,
            crossterm::event::KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE)
        ),
        Some(Action::PrevFile)
    );
    // Every other default binding count is unchanged (spot check via total
    // binding count staying the same as pure `default_map` for this scope,
    // since one action's single key was swapped 1-for-1).
    let default_diff_count = Keymap::default_map()
        .bindings()
        .iter()
        .filter(|b| b.scope == Scope::Diff)
        .count();
    let effective_diff_count = km
        .bindings()
        .iter()
        .filter(|b| b.scope == Scope::Diff)
        .count();
    assert_eq!(default_diff_count, effective_diff_count);
}

// -- Unbind: an empty array removes the action's bindings entirely ----------

#[test]
fn empty_array_unbinds_the_action() {
    let mut keys = KeysConfig::default();
    keys.diff.insert("toggle-collapse".to_string(), Vec::new());
    let (km, warnings) = effective_keymap(&keys);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    assert!(find(&km, Scope::Diff, Action::ToggleCollapse).is_empty());
    // The physical key `za` no longer resolves to anything.
    let mut pending = Some(crossterm::event::KeyEvent::new(
        KeyCode::Char('z'),
        KeyModifiers::NONE,
    ));
    assert_eq!(
        km.resolve_in(
            Scope::Diff,
            &mut pending,
            crossterm::event::KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)
        ),
        None
    );
}

// -- Collision: user binding wins, colliding default dropped, warned --------

#[test]
fn colliding_override_wins_and_drops_the_default_with_a_warning() {
    let mut keys = KeysConfig::default();
    // `k` is CursorUp's default key; rebind NextHunk onto it.
    keys.diff.insert(
        "next-hunk".to_string(),
        one(KeyCode::Char('k'), KeyModifiers::NONE),
    );
    let (km, warnings) = effective_keymap(&keys);
    assert_eq!(warnings.len(), 1, "expected exactly one collision warning");
    assert_eq!(
        km.lookup_in(
            Scope::Diff,
            crossterm::event::KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE)
        ),
        Some(Action::NextHunk),
        "user override must win the collision"
    );
    assert!(
        find(&km, Scope::Diff, Action::CursorUp).is_empty(),
        "the colliding default (CursorUp on `k`) must be dropped"
    );
}

#[test]
fn collision_between_two_override_entries_is_also_caught() {
    let mut keys = KeysConfig::default();
    keys.diff.insert(
        "next-hunk".to_string(),
        one(KeyCode::Char('x'), KeyModifiers::NONE),
    );
    keys.diff.insert(
        "prev-hunk".to_string(),
        one(KeyCode::Char('x'), KeyModifiers::NONE),
    );
    let (km, warnings) = effective_keymap(&keys);
    assert_eq!(warnings.len(), 1);
    // Whichever wins, only one action should claim `x` in diff scope.
    let claimants: Vec<Action> = km
        .bindings()
        .iter()
        .filter(|b| b.scope == Scope::Diff && b.key_label() == "x")
        .map(|b| b.action)
        .collect();
    assert_eq!(claimants.len(), 1);
}

// -- Unknown action name: invalid value, entry ignored -----------------------

#[test]
fn unknown_action_name_is_a_warning_and_does_not_touch_the_keymap() {
    let mut keys = KeysConfig::default();
    keys.diff.insert(
        "not-a-real-action".to_string(),
        one(KeyCode::Char('J'), KeyModifiers::NONE),
    );
    let (km, warnings) = effective_keymap(&keys);
    assert_eq!(warnings.len(), 1);
    match &warnings[0] {
        crate::config::ConfigWarning::InvalidValue { section, key, .. } => {
            assert_eq!(section, "keys.diff");
            assert_eq!(key, "not-a-real-action");
        }
        other => panic!("expected InvalidValue, got {other:?}"),
    }
    // `default_map`'s bindings are entirely unaffected.
    assert_eq!(km.bindings().len(), Keymap::default_map().bindings().len());
}

// -- Scope isolation: a `[keys.panel]` override never touches diff scope ----

#[test]
fn panel_overrides_do_not_affect_diff_scope_and_vice_versa() {
    let mut keys = KeysConfig::default();
    keys.panel.insert(
        "panel-cursor-down".to_string(),
        one(KeyCode::Char('x'), KeyModifiers::NONE),
    );
    let (km, warnings) = effective_keymap(&keys);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    // Diff-scope `j` (CursorDown) is untouched.
    assert_eq!(
        km.lookup_in(
            Scope::Diff,
            crossterm::event::KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)
        ),
        Some(Action::CursorDown)
    );
    assert_eq!(
        km.lookup_in(
            Scope::Panel,
            crossterm::event::KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)
        ),
        Some(Action::PanelCursorDown)
    );
}

// -- No config: effective keymap is byte-identical to default_map ------------

#[test]
fn no_overrides_yields_the_default_map_unchanged() {
    let (km, warnings) = effective_keymap(&KeysConfig::default());
    assert!(warnings.is_empty());
    let default = Keymap::default_map();
    assert_eq!(km.bindings().len(), default.bindings().len());
    for (a, b) in km.bindings().iter().zip(default.bindings().iter()) {
        assert_eq!(a.action, b.action);
        assert_eq!(a.key_label(), b.key_label());
        assert_eq!(a.scope, b.scope);
    }
}
