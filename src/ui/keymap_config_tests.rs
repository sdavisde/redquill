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
    // `y` is a diff-scope key no default binds, so this exercises a pure
    // override-vs-override collision (`x` is now `DeleteAnnotation`).
    let mut keys = KeysConfig::default();
    keys.diff.insert(
        "next-hunk".to_string(),
        one(KeyCode::Char('y'), KeyModifiers::NONE),
    );
    keys.diff.insert(
        "prev-hunk".to_string(),
        one(KeyCode::Char('y'), KeyModifiers::NONE),
    );
    let (km, warnings) = effective_keymap(&keys);
    assert_eq!(warnings.len(), 1);
    // Whichever wins, only one action should claim `y` in diff scope.
    let claimants: Vec<Action> = km
        .bindings()
        .iter()
        .filter(|b| b.scope == Scope::Diff && b.key_label() == "y")
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

// -- `[keys.global]` merge semantics -----------------------------------------

#[test]
fn global_override_replaces_the_default_key_rather_than_appending() {
    let mut keys = KeysConfig::default();
    keys.global.insert(
        "toggle-command-log".to_string(),
        one(KeyCode::Char('L'), KeyModifiers::NONE),
    );
    let (km, warnings) = effective_keymap(&keys);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");

    let rows = find(&km, Scope::Global, Action::ToggleCommandLog);
    assert_eq!(rows.len(), 1, "must have exactly one binding, not appended");
    assert_eq!(rows[0].key_label(), "L");

    // The default `@` is gone from both scopes it used to reach through the
    // Global fallback; `L` reaches them instead.
    for scope in [Scope::Diff, Scope::Panel] {
        assert_eq!(
            km.lookup_in(
                scope,
                crossterm::event::KeyEvent::new(KeyCode::Char('@'), KeyModifiers::NONE)
            ),
            None
        );
        assert_eq!(
            km.lookup_in(
                scope,
                crossterm::event::KeyEvent::new(KeyCode::Char('L'), KeyModifiers::NONE)
            ),
            Some(Action::ToggleCommandLog)
        );
    }
}

#[test]
fn global_empty_array_unbinds_the_action_in_every_scope() {
    let mut keys = KeysConfig::default();
    keys.global
        .insert("dismiss-config-warning".to_string(), Vec::new());
    let (km, warnings) = effective_keymap(&keys);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    assert!(find(&km, Scope::Global, Action::DismissConfigWarning).is_empty());
    for scope in [Scope::Diff, Scope::Panel] {
        assert_eq!(
            km.lookup_in(
                scope,
                crossterm::event::KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE)
            ),
            None
        );
    }
}

#[test]
fn global_unknown_action_name_is_a_warning_and_does_not_touch_the_keymap() {
    let mut keys = KeysConfig::default();
    keys.global.insert(
        "not-a-real-action".to_string(),
        one(KeyCode::Char('J'), KeyModifiers::NONE),
    );
    let (km, warnings) = effective_keymap(&keys);
    assert_eq!(warnings.len(), 1);
    match &warnings[0] {
        crate::config::ConfigWarning::InvalidValue { section, key, .. } => {
            assert_eq!(section, "keys.global");
            assert_eq!(key, "not-a-real-action");
        }
        other => panic!("expected InvalidValue, got {other:?}"),
    }
    assert_eq!(km.bindings().len(), Keymap::default_map().bindings().len());
}

/// Collision checking stays per scope: rebinding `dismiss-config-warning`
/// onto `?` collides with `Global`'s own `toggle-help` default (both rows
/// live in `Scope::Global`), so the default `?` row is dropped with a
/// warning and the user's binding wins.
#[test]
fn global_override_colliding_with_another_global_default_wins_with_a_warning() {
    let mut keys = KeysConfig::default();
    keys.global.insert(
        "dismiss-config-warning".to_string(),
        one(KeyCode::Char('?'), KeyModifiers::NONE),
    );
    let (km, warnings) = effective_keymap(&keys);
    assert_eq!(warnings.len(), 1, "expected exactly one collision warning");
    assert_eq!(
        km.lookup_in(
            Scope::Global,
            crossterm::event::KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE)
        ),
        Some(Action::DismissConfigWarning),
        "user override must win the collision"
    );
    assert!(
        find(&km, Scope::Global, Action::ToggleHelp).is_empty(),
        "the colliding default (ToggleHelp on `?`) must be dropped"
    );
}

/// A `[keys.diff]` override reusing a key already claimed by a `Global`
/// default is shadowing, not a collision — no warning, and the Diff-scope
/// override wins in Diff scope while Panel scope still gets the Global row.
#[test]
fn a_diff_override_reusing_a_global_default_key_shadows_without_a_warning() {
    let mut keys = KeysConfig::default();
    keys.diff.insert(
        "next-hunk".to_string(),
        one(KeyCode::Char('@'), KeyModifiers::NONE),
    );
    let (km, warnings) = effective_keymap(&keys);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    assert_eq!(
        km.lookup_in(
            Scope::Diff,
            crossterm::event::KeyEvent::new(KeyCode::Char('@'), KeyModifiers::NONE)
        ),
        Some(Action::NextHunk),
        "the Diff-scope override shadows the Global default"
    );
    assert_eq!(
        km.lookup_in(
            Scope::Panel,
            crossterm::event::KeyEvent::new(KeyCode::Char('@'), KeyModifiers::NONE)
        ),
        Some(Action::ToggleCommandLog),
        "Panel scope still falls back to the untouched Global default"
    );
}

// -- Review launcher rebind: `[keys.global]`/`[keys.diff]` remap ------------

/// `[keys.global] open-review-launcher = "L"` remaps the Review launcher off
/// its default `R`, reachable from both table-driven scopes via the Global
/// fallback.
#[test]
fn global_override_remaps_open_review_launcher() {
    let mut keys = KeysConfig::default();
    keys.global.insert(
        "open-review-launcher".to_string(),
        one(KeyCode::Char('L'), KeyModifiers::NONE),
    );
    let (km, warnings) = effective_keymap(&keys);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");

    for scope in [Scope::Diff, Scope::Panel] {
        assert_eq!(
            km.lookup_in(
                scope,
                crossterm::event::KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE)
            ),
            None,
            "the default R is gone"
        );
        assert_eq!(
            km.lookup_in(
                scope,
                crossterm::event::KeyEvent::new(KeyCode::Char('L'), KeyModifiers::NONE)
            ),
            Some(Action::OpenReviewLauncher)
        );
    }
}

/// `[keys.diff] refresh = "F"` remaps refresh off its post-rebind default
/// `r`, proving the diff-scope override still applies cleanly to the moved
/// binding.
#[test]
fn diff_override_remaps_refresh_off_its_post_rebind_default() {
    let mut keys = KeysConfig::default();
    keys.diff.insert(
        "refresh".to_string(),
        one(KeyCode::Char('F'), KeyModifiers::NONE),
    );
    let (km, warnings) = effective_keymap(&keys);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");

    assert_eq!(
        km.lookup_in(
            Scope::Diff,
            crossterm::event::KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)
        ),
        None,
        "the default r is gone"
    );
    assert_eq!(
        km.lookup_in(
            Scope::Diff,
            crossterm::event::KeyEvent::new(KeyCode::Char('F'), KeyModifiers::NONE)
        ),
        Some(Action::Refresh)
    );
    // The Review launcher's own R (Global) is untouched by a Diff-scope
    // override of an unrelated action.
    assert_eq!(
        km.lookup_in(
            Scope::Diff,
            crossterm::event::KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE)
        ),
        Some(Action::OpenReviewLauncher)
    );
}

/// The diff-view annotation edit/delete rows (`e`/`x`) are ordinary
/// main-table rows, so `[keys.diff]` can remap them like any other action.
#[test]
fn diff_overrides_remap_the_annotation_edit_and_delete_rows() {
    let mut keys = KeysConfig::default();
    keys.diff.insert(
        "edit-annotation".to_string(),
        one(KeyCode::Char('E'), KeyModifiers::NONE),
    );
    keys.diff.insert(
        "delete-annotation".to_string(),
        one(KeyCode::Char('D'), KeyModifiers::NONE),
    );
    let (km, warnings) = effective_keymap(&keys);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");

    // The remapped keys resolve to the new actions...
    assert_eq!(
        km.lookup_in(
            Scope::Diff,
            crossterm::event::KeyEvent::new(KeyCode::Char('E'), KeyModifiers::NONE)
        ),
        Some(Action::EditAnnotation)
    );
    assert_eq!(
        km.lookup_in(
            Scope::Diff,
            crossterm::event::KeyEvent::new(KeyCode::Char('D'), KeyModifiers::NONE)
        ),
        Some(Action::DeleteAnnotation)
    );
    // ...and the defaults are gone.
    assert_eq!(
        km.lookup_in(
            Scope::Diff,
            crossterm::event::KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE)
        ),
        None
    );
    assert_eq!(
        km.lookup_in(
            Scope::Diff,
            crossterm::event::KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)
        ),
        None
    );
}

// -- Common-workflows header reflects `[keys.*]` overrides (FR-7) -----------

/// A `[keys.global]` remap of the Review launcher changes the key the
/// common-workflows header displays for "Review a branch or commit" — the
/// header resolves against the effective (post-config-merge) keymap `help`
/// builds this module's `effective_keymap` from, not the compiled-in
/// default.
#[test]
fn keys_global_override_changes_the_workflows_header_key() {
    let mut keys = KeysConfig::default();
    keys.global.insert(
        "open-review-launcher".to_string(),
        one(KeyCode::Char('L'), KeyModifiers::NONE),
    );
    let (km, warnings) = effective_keymap(&keys);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");

    let rows = super::super::help::workflow_rows(
        super::super::app::ModeOrigin::Normal,
        &km,
        true,
        true,
        true,
    );
    let row = rows
        .iter()
        .find(|r| r.phrase == "Review a branch or commit")
        .expect("the launcher entry must still resolve after the remap");
    assert_eq!(row.key, "L");
}

/// A `[keys.diff]` remap of `compose` changes the key shown for "Comment on
/// a line" — proving the diff-scope override path, not just the global one
/// above.
#[test]
fn keys_diff_override_changes_the_workflows_header_key() {
    let mut keys = KeysConfig::default();
    keys.diff.insert(
        "compose".to_string(),
        one(KeyCode::Char('C'), KeyModifiers::NONE),
    );
    let (km, warnings) = effective_keymap(&keys);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");

    let rows = super::super::help::workflow_rows(
        super::super::app::ModeOrigin::Normal,
        &km,
        true,
        true,
        true,
    );
    let row = rows
        .iter()
        .find(|r| r.phrase == "Comment on a line")
        .expect("the compose entry must still resolve after the remap");
    assert_eq!(row.key, "C");
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

// -- Panel file-action rows: remap coverage ----------------------------------

/// The panel's per-file rows are ordinary `[keys.panel]` actions:
/// `toggle-stage`, `stage-file`, and `toggle-defer` all remap cleanly, and
/// diff scope's own rows for the same actions are untouched.
#[test]
fn panel_overrides_remap_the_file_action_rows() {
    let mut keys = KeysConfig::default();
    keys.panel.insert(
        "toggle-stage".to_string(),
        one(KeyCode::Char('x'), KeyModifiers::NONE),
    );
    keys.panel.insert(
        "stage-file".to_string(),
        one(KeyCode::Char('y'), KeyModifiers::NONE),
    );
    keys.panel.insert(
        "toggle-defer".to_string(),
        one(KeyCode::Char('X'), KeyModifiers::NONE),
    );
    let (km, warnings) = effective_keymap(&keys);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");

    let ev = |code| crossterm::event::KeyEvent::new(code, KeyModifiers::NONE);
    assert_eq!(
        km.lookup_in(Scope::Panel, ev(KeyCode::Char('x'))),
        Some(Action::ToggleStage)
    );
    assert_eq!(
        km.lookup_in(Scope::Panel, ev(KeyCode::Char('y'))),
        Some(Action::StageFile)
    );
    assert_eq!(
        km.lookup_in(Scope::Panel, ev(KeyCode::Char('X'))),
        Some(Action::ToggleDefer)
    );
    // With ToggleStage remapped off Space, the panel's Space now resolves
    // to the review phantom row — whose handler self-guards outside review
    // sessions — exactly like diff scope's own Space after the same remap.
    assert_eq!(
        km.lookup_in(Scope::Panel, ev(KeyCode::Char(' '))),
        Some(Action::ToggleAccept)
    );
    // Diff scope's rows for the same actions are untouched.
    assert_eq!(
        km.lookup_in(Scope::Diff, ev(KeyCode::Char(' '))),
        Some(Action::ToggleStage)
    );
    assert_eq!(
        km.lookup_in(Scope::Diff, ev(KeyCode::Char('S'))),
        Some(Action::StageFile)
    );
    assert_eq!(
        km.lookup_in(Scope::Diff, ev(KeyCode::Char('d'))),
        Some(Action::ToggleDefer)
    );
}

// -- Panel coherence: Esc leaves, s and / reach through (spec 11 Unit 2) ----

/// Remapping `focus-git-panel` in `[keys.panel]` replaces *every* default
/// panel row for that action — both `` ` `` and `Esc` — with the configured
/// key(s), the same "an action named in config gets exactly the listed
/// keys" contract every other action follows (see
/// `overriding_an_action_replaces_its_default_keys_rather_than_appending`).
/// A user who wants to keep two keys for it must list both.
#[test]
fn panel_override_of_focus_git_panel_replaces_both_default_keys() {
    let mut keys = KeysConfig::default();
    keys.panel.insert(
        "focus-git-panel".to_string(),
        one(KeyCode::Char('x'), KeyModifiers::NONE),
    );
    let (km, warnings) = effective_keymap(&keys);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");

    let ev = |code| crossterm::event::KeyEvent::new(code, KeyModifiers::NONE);
    assert_eq!(
        km.lookup_in(Scope::Panel, ev(KeyCode::Char('x'))),
        Some(Action::FocusGitPanel)
    );
    assert_eq!(km.lookup_in(Scope::Panel, ev(KeyCode::Char('`'))), None);
    assert_eq!(km.lookup_in(Scope::Panel, ev(KeyCode::Esc)), None);
    // Diff scope's own `` ` `` row (a different action's row entirely —
    // `Scope::Diff`'s `FocusGitPanel` binding is untouched by a
    // `[keys.panel]` override).
    assert_eq!(
        km.lookup_in(Scope::Diff, ev(KeyCode::Char('`'))),
        Some(Action::FocusGitPanel)
    );
}

/// `[keys.panel]` can remap `Esc` and `` ` `` independently, since they're
/// two separate table rows sharing one action — listing both keys under the
/// same override keeps both reachable under new keys.
#[test]
fn panel_override_of_focus_git_panel_can_list_both_keys() {
    let mut keys = KeysConfig::default();
    keys.panel.insert(
        "focus-git-panel".to_string(),
        vec![
            KeySeqSpec::One(ChordSpec {
                code: KeyCode::Char('x'),
                mods: KeyModifiers::NONE,
            }),
            KeySeqSpec::One(ChordSpec {
                code: KeyCode::Char('y'),
                mods: KeyModifiers::NONE,
            }),
        ],
    );
    let (km, warnings) = effective_keymap(&keys);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");

    let ev = |code| crossterm::event::KeyEvent::new(code, KeyModifiers::NONE);
    assert_eq!(
        km.lookup_in(Scope::Panel, ev(KeyCode::Char('x'))),
        Some(Action::FocusGitPanel)
    );
    assert_eq!(
        km.lookup_in(Scope::Panel, ev(KeyCode::Char('y'))),
        Some(Action::FocusGitPanel)
    );
}

/// `s`/`/` remap independently in `[keys.panel]`, exactly like the other
/// panel-scope rows — the coherence keys are ordinary config-remappable
/// table entries, not a special case.
#[test]
fn panel_overrides_remap_the_coherence_rows() {
    let mut keys = KeysConfig::default();
    keys.panel.insert(
        "toggle-staging-panel".to_string(),
        one(KeyCode::Char('x'), KeyModifiers::NONE),
    );
    keys.panel.insert(
        "search".to_string(),
        one(KeyCode::Char('y'), KeyModifiers::NONE),
    );
    let (km, warnings) = effective_keymap(&keys);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");

    let ev = |code| crossterm::event::KeyEvent::new(code, KeyModifiers::NONE);
    assert_eq!(
        km.lookup_in(Scope::Panel, ev(KeyCode::Char('x'))),
        Some(Action::ToggleStagingPanel)
    );
    assert_eq!(
        km.lookup_in(Scope::Panel, ev(KeyCode::Char('y'))),
        Some(Action::Search)
    );
    // The defaults are gone from panel scope...
    assert_eq!(km.lookup_in(Scope::Panel, ev(KeyCode::Char('s'))), None);
    // ...but diff scope's own `/` row (a separate table entry) is untouched.
    assert_eq!(
        km.lookup_in(Scope::Diff, ev(KeyCode::Char('/'))),
        Some(Action::Search)
    );
}
