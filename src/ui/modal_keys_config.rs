//! Merges every `[keys.<mode>]` config table (`crate::config::KeysConfig::modal`,
//! plus its two named fields' modal counterparts don't exist — `diff`/`panel`
//! stay `crate::ui::keymap_config`'s job) onto `crate::ui::modal_keys`'s
//! compiled-in default tables, producing the effective
//! [`super::modal_keys::ModalKeymaps`] every modal handler and render call
//! reads (spec 07 Unit 4 task 5.3/5.4). Built exactly once, from
//! [`super::run`] alongside [`super::keymap_config::effective_keymap`], then
//! stored on [`super::app::App`] — no per-keystroke parsing.
//!
//! **Layering**: like `super::keymap_config`/`super::lsp_config`/
//! `super::editor`'s preset table, this is an edge module allowed to import
//! both `crate::config` and the `crate::ui::modal_keys` runtime types —
//! `crate::config` itself must never import `crate::ui` (see that module's
//! doc).
//!
//! **Merge semantics** (spec 07 Unit 4 FR, reused verbatim from the main
//! keymap's task 4 contract): an action named in a `[keys.<mode>]` table
//! gets *exactly* the listed keys — its default keys for that action are
//! dropped, not appended to; an action not named keeps its defaults
//! untouched; an empty list (`= []`) unbinds the action entirely; a
//! same-table collision (a user-provided key sequence already claimed by
//! another row, default or an earlier-applied override) is resolved
//! user-wins, with one [`ConfigWarning`] recorded. An unknown action name,
//! or a two-chord key sequence (modal tables never supported `gd`-style
//! sequences), is itself an invalid value.

use std::collections::BTreeMap;

use crate::config::ConfigWarning;
use crate::config::keys::KeySeqSpec;

use super::modal_keys::{self, ModalBinding, ModalKey, ModalKeymaps};

/// Builds the effective modal keymaps: [`ModalKeymaps::default`] with every
/// `[keys.<mode>]` override applied, plus every warning the merge produced.
/// The caller ([`super::run`]) appends these to the warnings already
/// collected from config load and the main-keymap merge, so all three
/// surface through the same dismissible status-line notice.
pub(super) fn effective_modal_keys(
    keys: &crate::config::KeysConfig,
) -> (ModalKeymaps, Vec<ConfigWarning>) {
    let mut warnings = Vec::new();
    let empty = BTreeMap::new();
    let overrides_for = |mode: &str| keys.modal.get(mode).unwrap_or(&empty);

    let keymaps = ModalKeymaps {
        list: apply_modal_overrides(
            modal_keys::LIST_KEYS.clone(),
            overrides_for("list"),
            "keys.list",
            modal_keys::list_action_name,
            modal_keys::list_action_from_name,
            &mut warnings,
        ),
        staging: apply_modal_overrides(
            modal_keys::STAGING_KEYS.clone(),
            overrides_for("staging"),
            "keys.staging",
            modal_keys::staging_action_name,
            modal_keys::staging_action_from_name,
            &mut warnings,
        ),
        peek: apply_modal_overrides(
            modal_keys::PEEK_KEYS.clone(),
            overrides_for("peek"),
            "keys.peek",
            modal_keys::peek_action_name,
            modal_keys::peek_action_from_name,
            &mut warnings,
        ),
        switcher: apply_modal_overrides(
            modal_keys::SWITCHER_KEYS.clone(),
            overrides_for("switcher"),
            "keys.switcher",
            modal_keys::switcher_action_name,
            modal_keys::switcher_action_from_name,
            &mut warnings,
        ),
        help: apply_modal_overrides(
            modal_keys::HELP_KEYS.clone(),
            overrides_for("help"),
            "keys.help",
            modal_keys::help_action_name,
            modal_keys::help_action_from_name,
            &mut warnings,
        ),
        help_search: apply_modal_overrides(
            modal_keys::HELP_SEARCH_HINTS.clone(),
            overrides_for("help-search"),
            "keys.help-search",
            modal_keys::help_search_action_name,
            modal_keys::help_search_action_from_name,
            &mut warnings,
        ),
        compose: apply_modal_overrides(
            modal_keys::COMPOSE_HINTS.clone(),
            overrides_for("compose"),
            "keys.compose",
            modal_keys::compose_action_name,
            modal_keys::compose_action_from_name,
            &mut warnings,
        ),
        commit_message: apply_modal_overrides(
            modal_keys::COMMIT_MESSAGE_HINTS.clone(),
            overrides_for("commit-message"),
            "keys.commit-message",
            modal_keys::commit_message_action_name,
            modal_keys::commit_message_action_from_name,
            &mut warnings,
        ),
        search: apply_modal_overrides(
            modal_keys::SEARCH_HINTS.clone(),
            overrides_for("search"),
            "keys.search",
            modal_keys::search_action_name,
            modal_keys::search_action_from_name,
            &mut warnings,
        ),
        finder: apply_modal_overrides(
            modal_keys::FINDER_HINTS.clone(),
            overrides_for("finder"),
            "keys.finder",
            modal_keys::finder_action_name,
            modal_keys::finder_action_from_name,
            &mut warnings,
        ),
        project_search_input: apply_modal_overrides(
            modal_keys::PROJECT_SEARCH_INPUT_HINTS.clone(),
            overrides_for("project-search-input"),
            "keys.project-search-input",
            modal_keys::project_search_input_action_name,
            modal_keys::project_search_input_action_from_name,
            &mut warnings,
        ),
        project_search_results: apply_modal_overrides(
            modal_keys::PROJECT_SEARCH_RESULTS_HINTS.clone(),
            overrides_for("project-search-results"),
            "keys.project-search-results",
            modal_keys::project_search_results_action_name,
            modal_keys::project_search_results_action_from_name,
            &mut warnings,
        ),
        // The end-review modal (spec 08 Unit 2) isn't `[keys.end-review]`
        // remappable yet — it's absent from the cross-checked
        // `MODAL_MODE_NAMES` lists — so it always takes its compiled-in
        // defaults verbatim, with no override lookup. See
        // `modal_keys::END_REVIEW_KEYS`'s doc for what wiring remapping later
        // would entail.
        end_review: modal_keys::END_REVIEW_KEYS.clone(),
    };

    // Every mode name the config actually provided a table for that isn't
    // one of the twelve known modes was already flagged (unknown key) at
    // parse time in `crate::config::keys::KeysConfig::from_value`, which
    // hardcodes the same twelve names — see that module's `MODAL_MODE_NAMES`
    // doc and this module's tests for the cross-check that the two lists
    // agree.
    (keymaps, warnings)
}

/// Applies one mode's `[keys.<mode>]` overrides onto that mode's default
/// table. `name_of`/`from_name` are the mode's bijective action-name pair
/// (e.g. [`modal_keys::list_action_name`]/[`modal_keys::list_action_from_name`]).
fn apply_modal_overrides<A: Copy + PartialEq + 'static>(
    defaults: Vec<ModalBinding<A>>,
    overrides: &BTreeMap<String, Vec<KeySeqSpec>>,
    section_name: &str,
    name_of: fn(A) -> &'static str,
    from_name: fn(&str) -> Option<A>,
    warnings: &mut Vec<ConfigWarning>,
) -> Vec<ModalBinding<A>> {
    // Resolve action names and key strings up front: unknown names and
    // two-chord sequences (modal tables never supported `gd`-style
    // sequences) warn-and-drop here, so the "which actions are being
    // replaced" set below only ever contains real, single-chord overrides.
    let mut resolved: Vec<(A, Vec<ModalKey>)> = Vec::new();
    for (name, specs) in overrides {
        let Some(action) = from_name(name) else {
            warnings.push(ConfigWarning::InvalidValue {
                section: section_name.to_string(),
                key: name.clone(),
                message: "unknown action name".to_string(),
            });
            continue;
        };
        let mut keys = Vec::with_capacity(specs.len());
        let mut ok = true;
        for spec in specs {
            match spec {
                KeySeqSpec::One(chord) => keys.push(ModalKey::from_spec(*chord)),
                KeySeqSpec::Two(..) => {
                    warnings.push(ConfigWarning::InvalidValue {
                        section: section_name.to_string(),
                        key: name.clone(),
                        message: "two-chord sequences aren't supported here".to_string(),
                    });
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            resolved.push((action, keys));
        }
    }

    // Descriptions/footer tags for a replaced action are reused from the
    // pre-filter defaults (every action here has exactly one such row by
    // construction — each enum variant was defined from an existing table
    // row), so this template lookup runs on a clone taken before the
    // "action gets exactly the listed keys" filter below removes the row.
    let templates = defaults.clone();

    // An action named in config gets exactly the listed keys: drop every
    // default row for that action up front (including when the list is
    // empty — the explicit-unbind case).
    let replaced: Vec<A> = resolved.iter().map(|(a, _)| *a).collect();
    let mut out: Vec<ModalBinding<A>> = defaults
        .into_iter()
        .filter(|b| !replaced.contains(&b.action))
        .collect();

    for (action, keys) in resolved {
        let template = templates.iter().find(|b| b.action == action);
        let description = template.map_or("(configured action)", |b| b.description);
        let footer = template.and_then(|b| b.footer);

        // Same-table collision: a user binding always wins, so any existing
        // row for a *different* action already sitting on one of the new
        // keys is dropped, with one warning recorded per collision.
        for key in &keys {
            if let Some(pos) = out
                .iter()
                .position(|b| b.action != action && b.keys.iter().any(|k| k == key))
            {
                let collided = out.remove(pos);
                warnings.push(ConfigWarning::InvalidValue {
                    section: section_name.to_string(),
                    key: name_of(action).to_string(),
                    message: format!(
                        "key \"{}\" collides with \"{}\"; \"{}\" wins",
                        key.label(),
                        name_of(collided.action),
                        name_of(action),
                    ),
                });
            }
        }

        // `= []` is the explicit-unbind case: the default row was already
        // dropped by the filter above, and there's nothing to push back.
        // Every other case is one row per action (not one row per key —
        // `ModalBinding::keys` already carries every alternate key for a
        // single action, e.g. the switcher's `ToggleTab`), so the help
        // overlay/footer still show one joined label instead of duplicating
        // the description once per key.
        if !keys.is_empty() {
            out.push(ModalBinding {
                description,
                keys,
                action,
                footer,
            });
        }
    }
    out
}

#[cfg(test)]
#[path = "modal_keys_config_tests.rs"]
mod tests;
