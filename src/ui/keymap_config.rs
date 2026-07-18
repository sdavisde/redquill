//! Merges the `[keys.diff]`/`[keys.panel]` config section
//! (`crate::config::KeysConfig`) onto `Keymap::default_map()` to produce the
//! effective startup keymap. Built exactly once, from
//! [`super::run`], before the event loop starts — the keymap is then
//! threaded by reference through the whole session, so there is no
//! per-keystroke parsing (see `super::perf_tests`'s dispatch tripwires).
//!
//! **Layering**: like `super::lsp_config`/`super::editor`'s preset table,
//! this is the edge module allowed to import both `crate::config` and the
//! `crate::ui::keymap` runtime types — `crate::config` itself must never
//! import `crate::ui` (see that module's doc).
//!
//! **Merge semantics**: an action named in
//! `[keys.diff]`/`[keys.panel]` gets *exactly* the listed keys — its
//! default keys for that action in that scope are dropped, not appended to;
//! an action not named keeps its defaults untouched; an empty list
//! (`= []`) unbinds the action entirely; if a user-provided key sequence
//! collides with another binding already in the same scope (a default, or
//! an earlier-applied override), the user binding wins and the colliding
//! entry is dropped, with one [`ConfigWarning`] recorded per collision. An
//! unknown action name is itself an invalid value (entry ignored, one
//! warning), per the same contract every other section uses.

use std::collections::BTreeMap;

use crate::config::ConfigWarning;
use crate::config::keys::KeySeqSpec;

use super::keymap::{Action, Binding, KeySeq, Keymap, Scope, action_from_name, action_name};

/// Builds the effective startup keymap: [`Keymap::default_map`] with
/// `[keys.diff]`/`[keys.panel]` overrides applied, plus every warning the
/// merge produced (unknown action names, same-scope collisions). The
/// caller ([`super::run`]) appends these to the config-load warnings
/// already on `App` (from `crate::config::load`, collected before the
/// keymap is built).
pub(crate) fn effective_keymap(keys: &crate::config::KeysConfig) -> (Keymap, Vec<ConfigWarning>) {
    let default = Keymap::default_map();
    let all: Vec<Binding> = default.bindings().to_vec();

    let diff_defaults: Vec<Binding> = all
        .iter()
        .filter(|b| b.scope == Scope::Diff)
        .copied()
        .collect();
    let panel_defaults: Vec<Binding> = all
        .iter()
        .filter(|b| b.scope == Scope::Panel)
        .copied()
        .collect();
    let global_defaults: Vec<Binding> = all
        .iter()
        .filter(|b| b.scope == Scope::Global)
        .copied()
        .collect();

    let mut warnings = Vec::new();
    let mut bindings = apply_overrides(
        Scope::Diff,
        diff_defaults,
        &keys.diff,
        &all,
        "keys.diff",
        &mut warnings,
    );
    bindings.extend(apply_overrides(
        Scope::Panel,
        panel_defaults,
        &keys.panel,
        &all,
        "keys.panel",
        &mut warnings,
    ));
    // `Scope::Global` rows have no config-driven overrides yet, so they
    // pass through unchanged, exactly like every row did before `[keys.*]`
    // config existed at all.
    bindings.extend(global_defaults);
    (Keymap::from_bindings(bindings), warnings)
}

/// Applies one scope's `[keys.*]` overrides onto that scope's default
/// bindings (`defaults`). `description_source` (every default binding,
/// across both scopes) is searched for a template description/footer to
/// reuse when an override introduces a row for an action that scope didn't
/// have a default row for — every `Action` has at least one row somewhere
/// in `Keymap::default_map()` (the "every user-visible action is in the
/// keymap" invariant CLAUDE.md, and `super::help`'s own drift test,
/// enforce), so the fallback text below is defensive, not reachable in
/// practice.
fn apply_overrides(
    scope: Scope,
    defaults: Vec<Binding>,
    overrides: &BTreeMap<String, Vec<KeySeqSpec>>,
    description_source: &[Binding],
    section_name: &str,
    warnings: &mut Vec<ConfigWarning>,
) -> Vec<Binding> {
    // Resolve action names up front: unknown names warn-and-drop here, so
    // the "which actions are being replaced" set below only ever contains
    // real actions.
    let mut resolved: Vec<(Action, Vec<KeySeq>)> = Vec::new();
    for (name, specs) in overrides {
        match action_from_name(name) {
            Some(action) => {
                let seqs = specs.iter().map(|s| KeySeq::from_spec(*s)).collect();
                resolved.push((action, seqs));
            }
            None => warnings.push(ConfigWarning::InvalidValue {
                section: section_name.to_string(),
                key: name.clone(),
                message: "unknown action name".to_string(),
            }),
        }
    }

    // An action named in config gets exactly the listed keys: drop every
    // default binding for that action in this scope up front (including
    // when the list is empty — the explicit-unbind case).
    let replaced: Vec<Action> = resolved.iter().map(|(a, _)| *a).collect();
    let mut out: Vec<Binding> = defaults
        .into_iter()
        .filter(|b| !replaced.contains(&b.action))
        .collect();

    for (action, seqs) in resolved {
        let template = description_source.iter().find(|b| b.action == action);
        let description = template.map_or("(configured action)", |b| b.description);
        let footer = template.and_then(|b| b.footer);
        for seq in seqs {
            // Same-scope collision: a user binding always wins, so any
            // existing row (default or an earlier override in this same
            // pass) already sitting on this exact key sequence is dropped,
            // with one warning recorded — unless it's the same action
            // (e.g. a duplicate key listed twice for one action), which is
            // harmless and not a collision.
            if let Some(pos) = out.iter().position(|b| b.scope == scope && b.keys == seq) {
                if out[pos].action != action {
                    let collided = out.remove(pos);
                    warnings.push(ConfigWarning::InvalidValue {
                        section: section_name.to_string(),
                        key: action_name(action).to_string(),
                        message: format!(
                            "key \"{}\" collides with \"{}\"; \"{}\" wins",
                            super::keymap::key_seq_label(seq),
                            action_name(collided.action),
                            action_name(action),
                        ),
                    });
                } else {
                    out.remove(pos);
                }
            }
            out.push(Binding {
                keys: seq,
                action,
                description,
                scope,
                footer,
            });
        }
    }
    out
}

#[cfg(test)]
#[path = "keymap_config_tests.rs"]
mod tests;
