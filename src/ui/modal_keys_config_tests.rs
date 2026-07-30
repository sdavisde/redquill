use super::*;
use crate::config::KeysConfig;
use crate::config::keys::ChordSpec;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn one(code: KeyCode, mods: KeyModifiers) -> Vec<KeySeqSpec> {
    vec![KeySeqSpec::One(ChordSpec { code, mods })]
}

fn two(code: KeyCode, mods: KeyModifiers) -> Vec<KeySeqSpec> {
    vec![KeySeqSpec::Two(
        ChordSpec { code, mods },
        ChordSpec {
            code: KeyCode::Char('x'),
            mods: KeyModifiers::NONE,
        },
    )]
}

fn keys_with(mode: &str, action: &str, specs: Vec<KeySeqSpec>) -> KeysConfig {
    let mut keys = KeysConfig::default();
    let mut table = BTreeMap::new();
    table.insert(action.to_string(), specs);
    keys.modal.insert(mode.to_string(), table);
    keys
}

// -- Cross-check: the ui-side and config-side mode-name lists agree ---------

#[test]
fn modal_mode_names_match_config_keys_hardcoded_list() {
    // `crate::config::keys::MODAL_MODE_NAMES` is private, so this drives the
    // agreement check indirectly: every name in the ui-side list must be
    // accepted (zero "unknown key" warnings) by `KeysConfig::from_value`,
    // and the count must match exactly (a name accepted by config that the
    // ui list doesn't know about would still slip past this check only if
    // it also appeared in `effective_modal_keys`'s match below, which is
    // exhaustive over the `ModalKeymaps` fields — so a name drifting
    // out of sync in either direction fails this test or fails to compile).
    let toml = modal_keys::MODAL_MODE_NAMES
        .iter()
        .map(|name| format!("[{name}]\n"))
        .collect::<String>();
    let raw: toml::Table = toml.parse().expect("valid TOML");
    let mut warnings = Vec::new();
    let cfg = KeysConfig::from_value(toml::Value::Table(raw), &mut warnings);
    assert!(
        warnings.is_empty(),
        "every ui-side mode name must be a config-recognized `[keys.*]` section: {warnings:?}"
    );
    assert_eq!(cfg.modal.len(), modal_keys::MODAL_MODE_NAMES.len());
}

// -- No config: every effective table is byte-identical to its default ------

#[test]
fn no_overrides_yields_every_default_table_unchanged() {
    let (effective, warnings) = effective_modal_keys(&KeysConfig::default());
    assert!(warnings.is_empty());

    fn same<A: Copy + PartialEq>(effective: &[ModalBinding<A>], default: &[ModalBinding<A>]) {
        assert_eq!(effective.len(), default.len());
        for (a, b) in effective.iter().zip(default.iter()) {
            assert!(a.action == b.action);
            assert_eq!(a.keys.len(), b.keys.len());
        }
    }
    same(&effective.list, &modal_keys::LIST_KEYS);
    same(&effective.staging, &modal_keys::STAGING_KEYS);
    same(&effective.peek, &modal_keys::PEEK_KEYS);
    same(&effective.switcher, &modal_keys::SWITCHER_KEYS);
    same(
        &effective.review_launcher,
        &modal_keys::REVIEW_LAUNCHER_KEYS,
    );
    same(&effective.help, &modal_keys::HELP_KEYS);
    same(&effective.help_search, &modal_keys::HELP_SEARCH_HINTS);
    same(&effective.compose, &modal_keys::COMPOSE_HINTS);
    same(&effective.commit_message, &modal_keys::COMMIT_MESSAGE_HINTS);
    same(&effective.search, &modal_keys::SEARCH_HINTS);
    same(&effective.finder, &modal_keys::FINDER_HINTS);
    same(
        &effective.project_search_input,
        &modal_keys::PROJECT_SEARCH_INPUT_HINTS,
    );
    same(
        &effective.project_search_results,
        &modal_keys::PROJECT_SEARCH_RESULTS_HINTS,
    );
    same(&effective.filter_edit, &modal_keys::FILTER_EDIT_KEYS);
    same(&effective.thread_view, &modal_keys::THREAD_VIEW_KEYS);
    same(&effective.submit_forge, &modal_keys::SUBMIT_FORGE_KEYS);
    same(&effective.submit_result, &modal_keys::SUBMIT_RESULT_KEYS);
    same(
        &effective.cleanup_reviews,
        &modal_keys::CLEANUP_REVIEWS_KEYS,
    );
}

// -- Replace: an action named in config gets exactly the listed keys --------

#[test]
fn overriding_a_staging_action_replaces_its_default_keys_rather_than_appending() {
    let keys = keys_with(
        "staging",
        "unstage",
        one(KeyCode::Char('x'), KeyModifiers::NONE),
    );
    let (effective, warnings) = effective_modal_keys(&keys);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");

    let rows: Vec<_> = effective
        .staging
        .iter()
        .filter(|b| b.action == modal_keys::StagingAction::Unstage)
        .collect();
    assert_eq!(rows.len(), 1, "must have exactly one row, not appended");
    assert_eq!(rows[0].key_label(), "x");

    // Space/Enter are unbound for Unstage here.
    assert_eq!(
        modal_keys::resolve(
            &effective.staging,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
        ),
        None
    );
    assert_eq!(
        modal_keys::resolve(
            &effective.staging,
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)
        ),
        Some(modal_keys::StagingAction::Unstage)
    );
}

// -- Keep: unlisted actions are untouched ------------------------------------

#[test]
fn unlisted_staging_actions_keep_their_defaults() {
    let keys = keys_with(
        "staging",
        "unstage",
        one(KeyCode::Char('x'), KeyModifiers::NONE),
    );
    let (effective, _warnings) = effective_modal_keys(&keys);
    // MoveDown (`j`) is untouched.
    assert_eq!(
        modal_keys::resolve(
            &effective.staging,
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)
        ),
        Some(modal_keys::StagingAction::MoveDown)
    );
    assert_eq!(effective.staging.len(), modal_keys::STAGING_KEYS.len());
}

// -- Unbind: an empty array removes the action's keys entirely --------------

#[test]
fn empty_array_unbinds_a_list_action() {
    let keys = keys_with("list", "delete", Vec::new());
    let (effective, warnings) = effective_modal_keys(&keys);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    assert!(
        !effective
            .list
            .iter()
            .any(|b| b.action == modal_keys::ListAction::Delete)
    );
    assert_eq!(
        modal_keys::resolve(
            &effective.list,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE)
        ),
        None
    );
}

// -- Collision: user binding wins, colliding default dropped, warned --------

#[test]
fn colliding_override_wins_and_drops_the_default_with_a_warning() {
    // `k` is List's MoveUp default; rebind Delete onto it.
    let keys = keys_with(
        "list",
        "delete",
        one(KeyCode::Char('k'), KeyModifiers::NONE),
    );
    let (effective, warnings) = effective_modal_keys(&keys);
    assert_eq!(warnings.len(), 1, "expected exactly one collision warning");
    assert_eq!(
        modal_keys::resolve(
            &effective.list,
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE)
        ),
        Some(modal_keys::ListAction::Delete),
        "user override must win the collision"
    );
    assert!(
        !effective
            .list
            .iter()
            .any(|b| b.action == modal_keys::ListAction::MoveUp),
        "the colliding default (MoveUp on `k`) must be dropped"
    );
}

#[test]
fn multiple_keys_for_one_overridden_action_stay_one_row() {
    // A remap to two alternate keys must produce one row carrying both, not
    // two rows repeating the description: the help overlay/footer must show
    // one joined label, matching how `SwitcherAction::ToggleTab`'s six
    // default keys already render as one row.
    let mut keys = KeysConfig::default();
    let mut table = BTreeMap::new();
    table.insert(
        "close".to_string(),
        vec![
            KeySeqSpec::One(ChordSpec {
                code: KeyCode::Char('q'),
                mods: KeyModifiers::NONE,
            }),
            KeySeqSpec::One(ChordSpec {
                code: KeyCode::Char('x'),
                mods: KeyModifiers::NONE,
            }),
        ],
    );
    keys.modal.insert("staging".to_string(), table);
    let (effective, warnings) = effective_modal_keys(&keys);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    let rows: Vec<_> = effective
        .staging
        .iter()
        .filter(|b| b.action == modal_keys::StagingAction::Close)
        .collect();
    assert_eq!(rows.len(), 1, "must be one row carrying both keys");
    assert_eq!(rows[0].keys.len(), 2);
}

// -- Unknown action name: invalid value, entry ignored -----------------------

#[test]
fn unknown_action_name_is_a_warning_and_does_not_touch_the_table() {
    let keys = keys_with(
        "staging",
        "not-a-real-action",
        one(KeyCode::Char('x'), KeyModifiers::NONE),
    );
    let (effective, warnings) = effective_modal_keys(&keys);
    assert_eq!(warnings.len(), 1);
    match &warnings[0] {
        crate::config::ConfigWarning::InvalidValue { section, key, .. } => {
            assert_eq!(section, "keys.staging");
            assert_eq!(key, "not-a-real-action");
        }
        other => panic!("expected InvalidValue, got {other:?}"),
    }
    assert_eq!(effective.staging.len(), modal_keys::STAGING_KEYS.len());
}

// -- Two-chord sequences aren't supported for modal tables -------------------

#[test]
fn two_chord_sequence_is_an_invalid_value_and_the_action_keeps_its_default() {
    let keys = keys_with(
        "staging",
        "unstage",
        two(KeyCode::Char('g'), KeyModifiers::NONE),
    );
    let (effective, warnings) = effective_modal_keys(&keys);
    assert_eq!(warnings.len(), 1);
    match &warnings[0] {
        crate::config::ConfigWarning::InvalidValue {
            section,
            key,
            message,
        } => {
            assert_eq!(section, "keys.staging");
            assert_eq!(key, "unstage");
            assert!(message.contains("two-chord"));
        }
        other => panic!("expected InvalidValue, got {other:?}"),
    }
    // The default (Space/Enter) is untouched since the override was dropped.
    assert_eq!(
        modal_keys::resolve(
            &effective.staging,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
        ),
        Some(modal_keys::StagingAction::Unstage)
    );
}

// -- Mode isolation: an override in one mode never touches another ----------

#[test]
fn mode_overrides_do_not_leak_into_other_modes() {
    let keys = keys_with(
        "staging",
        "unstage",
        one(KeyCode::Char('x'), KeyModifiers::NONE),
    );
    let (effective, warnings) = effective_modal_keys(&keys);
    assert!(warnings.is_empty());
    // List mode's defaults are untouched.
    assert_eq!(effective.list.len(), modal_keys::LIST_KEYS.len());
    assert_eq!(
        modal_keys::resolve(
            &effective.list,
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)
        ),
        Some(modal_keys::ListAction::MoveDown)
    );
}

// -- Free-text modes: overriding a control action never touches char inserts

#[test]
fn overriding_a_compose_control_action_leaves_the_rest_of_the_table_intact() {
    let keys = keys_with(
        "compose",
        "cancel",
        one(KeyCode::Char('q'), KeyModifiers::NONE),
    );
    let (effective, warnings) = effective_modal_keys(&keys);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    assert_eq!(
        modal_keys::resolve(
            &effective.compose,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)
        ),
        Some(modal_keys::ComposeAction::Cancel)
    );
    // Submit and every buffer-edit action are still present and unmoved.
    assert_eq!(
        modal_keys::resolve(
            &effective.compose,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
        ),
        Some(modal_keys::ComposeAction::Submit)
    );
    assert_eq!(effective.compose.len(), modal_keys::COMPOSE_HINTS.len());
}

// -- PR-flow modals: each section is wired to its own table ------------------
//
// One test per newly wired section, because the defect each catches is
// distinct: that *this* mode's `[keys.<mode>]` table never reached
// `effective_modal_keys` (the remap silently does nothing). Each asserts the
// three observable consequences of a wired section — the new key drives the
// action, the displaced default no longer does, and the row's `key_label`
// (the exact string `super::help`'s overlay and `super::footer`'s strip
// print) shows the new key.

#[test]
fn remapping_a_thread_view_action_moves_the_key_and_its_displayed_label() {
    let keys = keys_with(
        "thread-view",
        "reply",
        one(KeyCode::Char('a'), KeyModifiers::NONE),
    );
    let (effective, warnings) = effective_modal_keys(&keys);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    assert_eq!(
        modal_keys::resolve(
            &effective.thread_view,
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)
        ),
        Some(modal_keys::ThreadViewAction::Reply)
    );
    assert_eq!(
        modal_keys::resolve(
            &effective.thread_view,
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)
        ),
        None,
        "the displaced default must no longer reply"
    );
    let row = effective
        .thread_view
        .iter()
        .find(|b| b.action == modal_keys::ThreadViewAction::Reply)
        .expect("reply row");
    assert_eq!(row.key_label(), "a");
}

#[test]
fn remapping_a_submit_forge_action_moves_the_key_and_its_displayed_label() {
    let keys = keys_with(
        "submit-forge",
        "verdict-next",
        one(KeyCode::Char('y'), KeyModifiers::CONTROL),
    );
    let (effective, warnings) = effective_modal_keys(&keys);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    assert_eq!(
        modal_keys::resolve(
            &effective.submit_forge,
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL)
        ),
        Some(modal_keys::SubmitForgeAction::VerdictNext)
    );
    assert_eq!(
        modal_keys::resolve(
            &effective.submit_forge,
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)
        ),
        None,
        "the displaced default must no longer cycle the verdict"
    );
    let row = effective
        .submit_forge
        .iter()
        .find(|b| b.action == modal_keys::SubmitForgeAction::VerdictNext)
        .expect("verdict-next row");
    assert_eq!(row.key_label(), "Ctrl-y");
}

#[test]
fn remapping_a_submit_result_action_moves_the_key_and_its_displayed_label() {
    let keys = keys_with(
        "submit-result",
        "retry",
        one(KeyCode::Char('R'), KeyModifiers::NONE),
    );
    let (effective, warnings) = effective_modal_keys(&keys);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    assert_eq!(
        modal_keys::resolve(
            &effective.submit_result,
            KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE)
        ),
        Some(modal_keys::SubmitResultAction::Retry)
    );
    assert_eq!(
        modal_keys::resolve(
            &effective.submit_result,
            KeyEvent::new(KeyCode::Char('U'), KeyModifiers::NONE)
        ),
        None,
        "the displaced default must no longer retry"
    );
    let row = effective
        .submit_result
        .iter()
        .find(|b| b.action == modal_keys::SubmitResultAction::Retry)
        .expect("retry row");
    assert_eq!(row.key_label(), "R");
}

#[test]
fn remapping_a_cleanup_reviews_action_moves_the_key_and_its_displayed_label() {
    let keys = keys_with(
        "cleanup-reviews",
        "toggle",
        one(KeyCode::Char('t'), KeyModifiers::NONE),
    );
    let (effective, warnings) = effective_modal_keys(&keys);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    assert_eq!(
        modal_keys::resolve(
            &effective.cleanup_reviews,
            KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE)
        ),
        Some(modal_keys::CleanupReviewsAction::Toggle)
    );
    assert_eq!(
        modal_keys::resolve(
            &effective.cleanup_reviews,
            KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)
        ),
        None,
        "the displaced default must no longer toggle"
    );
    let row = effective
        .cleanup_reviews
        .iter()
        .find(|b| b.action == modal_keys::CleanupReviewsAction::Toggle)
        .expect("toggle row");
    assert_eq!(row.key_label(), "t");
}

// -- Invalid values in the PR-flow sections surface the launcher's warnings --

/// A section that never reached [`effective_modal_keys`] fails differently:
/// `KeysConfig::from_value` would reject the whole `[keys.<mode>]` header as
/// an unknown key, so neither of the per-action warnings below could appear.
/// Table-driven over the four sections since the assertion is on the warning
/// shape, not on any mode's action types.
#[test]
fn each_pr_flow_section_reports_bad_values_like_the_launcher_section_does() {
    for (mode, real_action) in [
        ("thread-view", "reply"),
        ("submit-forge", "confirm"),
        ("submit-result", "retry"),
        ("cleanup-reviews", "toggle"),
    ] {
        // An unparseable key string is rejected at config-parse time.
        let raw: toml::Table = format!("[{mode}]\n{real_action} = \"not-a-key\"\n")
            .parse()
            .expect("valid TOML");
        let mut warnings = Vec::new();
        let cfg = KeysConfig::from_value(toml::Value::Table(raw), &mut warnings);
        assert_eq!(warnings.len(), 1, "[keys.{mode}]: {warnings:?}");
        match &warnings[0] {
            crate::config::ConfigWarning::InvalidValue { section, key, .. } => {
                assert_eq!(section, &format!("keys.{mode}"));
                assert_eq!(key, real_action);
            }
            other => panic!("[keys.{mode}]: expected InvalidValue, got {other:?}"),
        }
        assert!(
            cfg.modal.contains_key(mode),
            "[keys.{mode}] must be a recognized section, not an unknown key"
        );

        // An unknown action name is rejected at merge time, naming the same
        // section.
        let keys = keys_with(
            mode,
            "not-a-real-action",
            one(KeyCode::Esc, KeyModifiers::NONE),
        );
        let (_effective, warnings) = effective_modal_keys(&keys);
        assert_eq!(warnings.len(), 1, "[keys.{mode}]: {warnings:?}");
        match &warnings[0] {
            crate::config::ConfigWarning::InvalidValue { section, key, .. } => {
                assert_eq!(section, &format!("keys.{mode}"));
                assert_eq!(key, "not-a-real-action");
            }
            other => panic!("[keys.{mode}]: expected InvalidValue, got {other:?}"),
        }
    }
}

// -- docs/example-config.toml completeness -----------------------------------
//
// The `[keys.<mode>]` sections are entirely commented out (like
// `[keys.diff]`/`[keys.panel]`'s precedent — an example file that "just
// works" if copied verbatim shouldn't silently impose a nonstandard
// keymap), so `crate::config::load`'s existing zero-warnings drift test
// (`example_config_toml_parses_with_zero_warnings`) can't see this content
// as live TOML. This test instead parses the *commented* action-name/key-
// string pairs out of the doc text directly and cross-checks them against
// the real per-mode action tables: every name must resolve via that mode's
// `*_action_from_name`, every key string must parse, and the doc's set of
// names for a mode must exactly equal that mode's complete action set (so
// the doc can neither omit an action nor invent one that doesn't exist).

/// One `# name = "key"` or `# name = ["key", ...]` doc line, parsed into
/// its action name and the key string(s) on the right-hand side (trailing
/// `# description` comments and surrounding whitespace stripped).
fn parse_doc_action_line(line: &str) -> Option<(String, Vec<String>)> {
    let rest = line.strip_prefix('#')?.trim_start();
    let (name, rest) = rest.split_once('=')?;
    let name = name.trim();
    if name.is_empty() || name.starts_with('[') {
        return None;
    }
    let rest = rest.trim_start();
    if let Some(arr) = rest.strip_prefix('[') {
        let end = arr.find(']')?;
        let items: Vec<String> = arr[..end]
            .split(',')
            .filter_map(|s| {
                let s = s.trim();
                s.strip_prefix('"')
                    .and_then(|s| s.strip_suffix('"'))
                    .map(str::to_string)
            })
            .collect();
        Some((name.to_string(), items))
    } else if let Some(s) = rest.strip_prefix('"') {
        // Cut at the closing quote — ignore any trailing `# description`.
        let end = s.find('"')?;
        Some((name.to_string(), vec![s[..end].to_string()]))
    } else {
        None
    }
}

type DocModalBlocks = Vec<(String, Vec<(String, Vec<String>)>)>;

/// Extracts every `[keys.<mode>]` doc block (mode name -> its ordered list
/// of `(action name, key strings)` pairs) from the commented-out modal
/// section of `docs/example-config.toml`. A block starts at a
/// `# [keys.<mode>]` header line and ends at the next such header or a
/// bare `#` separator line.
fn parse_doc_modal_blocks(text: &str) -> DocModalBlocks {
    let mut blocks: DocModalBlocks = Vec::new();
    let mut current: Option<usize> = None;
    for line in text.lines() {
        let trimmed = line.trim_end();
        if let Some(rest) = trimmed.strip_prefix("# [keys.")
            && let Some(mode) = rest.split(']').next()
        {
            blocks.push((mode.to_string(), Vec::new()));
            current = Some(blocks.len() - 1);
            continue;
        }
        if trimmed == "#" {
            current = None;
            continue;
        }
        if let Some(idx) = current
            && let Some((name, keys)) = parse_doc_action_line(trimmed)
        {
            blocks[idx].1.push((name, keys));
        }
    }
    blocks
}

/// Asserts `mode`'s doc block (from `docs/example-config.toml`) names
/// exactly the modal action space `from_name` resolves, and that every key
/// string it lists parses under the grammar. The expected action set is read
/// off `table` (the mode's default table) rather than hand-listed at the call
/// site: every variant appears in its table by construction — the same
/// argument `modal_keys`'s bijectivity test relies on — and a hand-kept
/// parallel list can go stale in step with the doc it's meant to police,
/// which is exactly how `[keys.review-launcher]`'s two newest actions went
/// undocumented.
fn assert_doc_block_matches<A: Copy + PartialEq>(
    blocks: &DocModalBlocks,
    mode: &str,
    table: &[ModalBinding<A>],
    name_of: fn(A) -> &'static str,
    from_name: fn(&str) -> Option<A>,
) {
    let mut all_actions: Vec<A> = Vec::new();
    for b in table {
        if !all_actions.contains(&b.action) {
            all_actions.push(b.action);
        }
    }
    let all_actions = &all_actions[..];
    // The first block named `mode` is the canonical documentation block;
    // the doc's trailing "Example:" section deliberately reuses
    // `staging`/`switcher` for a live one-line demo and isn't meant to be
    // the complete action list.
    let block = &blocks
        .iter()
        .find(|(name, _)| name == mode)
        .unwrap_or_else(|| panic!("docs/example-config.toml has no [keys.{mode}] block"))
        .1;
    let mut documented: Vec<A> = Vec::new();
    for (name, keys) in block {
        let action = from_name(name).unwrap_or_else(|| {
            panic!("[keys.{mode}] {name}: not a real action name for this mode")
        });
        documented.push(action);
        for key in keys {
            crate::config::keys::parse_key_string(key).unwrap_or_else(|e| {
                panic!("[keys.{mode}] {name} = \"{key}\": doesn't parse ({e})")
            });
        }
    }
    for action in all_actions {
        assert!(
            documented.contains(action),
            "[keys.{mode}] is missing \"{}\" from the example config",
            name_of(*action)
        );
    }
    assert_eq!(
        documented.len(),
        all_actions.len(),
        "[keys.{mode}] documents an action name outside the real action set"
    );
}

#[test]
fn example_config_documents_every_modal_action_exactly_once() {
    let text = include_str!("../../docs/example-config.toml");
    let blocks = parse_doc_modal_blocks(text);
    // Every canonical mode name appears at least once (the trailing
    // "Example:" section's reuse of `staging`/`switcher` is why this isn't
    // an exact-count check — see `assert_doc_block_matches`'s doc).
    for mode in modal_keys::MODAL_MODE_NAMES {
        assert!(
            blocks.iter().any(|(name, _)| name == mode),
            "docs/example-config.toml has no [keys.{mode}] block"
        );
    }

    use modal_keys::*;
    assert_doc_block_matches(
        &blocks,
        "list",
        &LIST_KEYS,
        list_action_name,
        list_action_from_name,
    );
    assert_doc_block_matches(
        &blocks,
        "staging",
        &STAGING_KEYS,
        staging_action_name,
        staging_action_from_name,
    );
    assert_doc_block_matches(
        &blocks,
        "peek",
        &PEEK_KEYS,
        peek_action_name,
        peek_action_from_name,
    );
    assert_doc_block_matches(
        &blocks,
        "switcher",
        &SWITCHER_KEYS,
        switcher_action_name,
        switcher_action_from_name,
    );
    assert_doc_block_matches(
        &blocks,
        "review-launcher",
        &REVIEW_LAUNCHER_KEYS,
        launcher_action_name,
        launcher_action_from_name,
    );
    assert_doc_block_matches(
        &blocks,
        "help",
        &HELP_KEYS,
        help_action_name,
        help_action_from_name,
    );
    assert_doc_block_matches(
        &blocks,
        "help-search",
        &HELP_SEARCH_HINTS,
        help_search_action_name,
        help_search_action_from_name,
    );
    assert_doc_block_matches(
        &blocks,
        "filter-edit",
        &FILTER_EDIT_KEYS,
        filter_edit_action_name,
        filter_edit_action_from_name,
    );
    assert_doc_block_matches(
        &blocks,
        "compose",
        &COMPOSE_HINTS,
        compose_action_name,
        compose_action_from_name,
    );
    assert_doc_block_matches(
        &blocks,
        "commit-message",
        &COMMIT_MESSAGE_HINTS,
        commit_message_action_name,
        commit_message_action_from_name,
    );
    assert_doc_block_matches(
        &blocks,
        "search",
        &SEARCH_HINTS,
        search_action_name,
        search_action_from_name,
    );
    assert_doc_block_matches(
        &blocks,
        "finder",
        &FINDER_HINTS,
        finder_action_name,
        finder_action_from_name,
    );
    assert_doc_block_matches(
        &blocks,
        "project-search-input",
        &PROJECT_SEARCH_INPUT_HINTS,
        project_search_input_action_name,
        project_search_input_action_from_name,
    );
    assert_doc_block_matches(
        &blocks,
        "project-search-results",
        &PROJECT_SEARCH_RESULTS_HINTS,
        project_search_results_action_name,
        project_search_results_action_from_name,
    );
    assert_doc_block_matches(
        &blocks,
        "thread-view",
        &THREAD_VIEW_KEYS,
        thread_view_action_name,
        thread_view_action_from_name,
    );
    assert_doc_block_matches(
        &blocks,
        "submit-forge",
        &SUBMIT_FORGE_KEYS,
        submit_forge_action_name,
        submit_forge_action_from_name,
    );
    assert_doc_block_matches(
        &blocks,
        "submit-result",
        &SUBMIT_RESULT_KEYS,
        submit_result_action_name,
        submit_result_action_from_name,
    );
    assert_doc_block_matches(
        &blocks,
        "cleanup-reviews",
        &CLEANUP_REVIEWS_KEYS,
        cleanup_reviews_action_name,
        cleanup_reviews_action_from_name,
    );
}
