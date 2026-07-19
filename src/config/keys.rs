//! `[keys.diff]`/`[keys.panel]`/`[keys.global]` config: the key-string
//! grammar plus the raw partial-override section data. Config-side, so this
//! module knows nothing about `crate::ui::keymap::{Action, Keymap, Binding,
//! KeySeq}` — only crossterm's `KeyCode`/`KeyModifiers` (already a
//! dependency, used throughout `crate::ui`) and plain strings/maps. Action-
//! name resolution and the actual default-plus-override merge happen at the
//! edge, in `crate::ui::keymap_config` — see that module's doc and
//! `crate::config`'s own module doc's layering note for why the split falls
//! here.
//!
//! **Grammar**: one physical key is `[modifier-]*name`, where `modifier` is
//! `ctrl`/`alt`/`shift` (case-insensitive, checked left to right, any number
//! may stack) and `name` is either a single character (`"a"`, `"?"`, `"$"`)
//! or one of a fixed vocabulary of named keys: `esc`/`escape`,
//! `enter`/`return`, `tab`, `space`, `backspace`, `delete`/`del`, `home`,
//! `end`, `pageup`/`pgup`, `pagedown`/`pgdn`, `up`, `down`, `left`, `right`,
//! `insert`/`ins`, `f1`..`f24`. `shift-tab` collapses to the same
//! `(BackTab, NONE)` representation `KeyChord::label` renders it as (see
//! [`parse_chord`]'s doc), rather than the generic `(Tab, SHIFT)` a naive
//! prefix application would produce. A `[keys.*]` value is one physical key
//! (`"ctrl-k"`) or two, space-separated (`"g d"` — the two-chord sequences
//! the main keymap already supports, like `gd`/`za`); zero, or three-or-more,
//! chords is rejected ([`KeyGrammarError`]).
//!
//! Every string this grammar accepts round-trips against
//! `crate::ui::keymap::KeyChord::label`'s rendering — pinned by a test in
//! `crate::ui::keymap`'s own test module (which has private access to
//! `KeyChord`) that feeds every default binding's label back through
//! [`parse_key_string`] — so a key the help overlay shows is always
//! writable back into config, and vice versa.

use std::collections::BTreeMap;

use crossterm::event::{KeyCode, KeyModifiers};
use thiserror::Error;

use super::ConfigWarning;

/// One parsed physical key: a code plus its modifiers, in the same shape
/// `crate::ui::keymap::KeyChord` uses internally — this is the config-side
/// mirror (plain, public fields, no dependency on `crate::ui`) that the edge
/// module converts into a real `KeyChord`/`KeySeq`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChordSpec {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

/// One parsed `[keys.*]` value: a single chord, or a two-chord sequence
/// (space-separated in the source string).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySeqSpec {
    One(ChordSpec),
    Two(ChordSpec, ChordSpec),
}

/// Why a `[keys.*]` key string failed to parse — folded into a
/// [`ConfigWarning::InvalidValue`] at the call site; never fatal (the entry
/// is dropped, everything else in the file still applies).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum KeyGrammarError {
    #[error("empty key string")]
    Empty,
    #[error("too many chords in \"{0}\" (max 2, space-separated)")]
    TooManyChords(String),
    #[error("unrecognized key name in \"{0}\"")]
    UnknownKeyName(String),
}

/// Parses one chord token (already whitespace-split): zero or more
/// `ctrl-`/`alt-`/`shift-` prefixes (case-insensitive, stripped left to
/// right) followed by a base key name. `shift-tab` is special-cased to
/// `(BackTab, NONE)` — matching `KeyChord::label`'s rendering of that same
/// physical key — rather than the generic `(Tab, SHIFT)` a naive prefix
/// application would produce; every other `shift-<name>` combination keeps
/// the generic `SHIFT` bit (a general-purpose escape hatch even though no
/// default binding uses it today).
fn parse_chord(token: &str) -> Result<ChordSpec, KeyGrammarError> {
    if token.is_empty() {
        return Err(KeyGrammarError::Empty);
    }
    let mut mods = KeyModifiers::NONE;
    let mut rest = token;
    loop {
        let lower = rest.to_ascii_lowercase();
        if let Some(stripped) = lower.strip_prefix("ctrl-") {
            mods |= KeyModifiers::CONTROL;
            rest = &rest[rest.len() - stripped.len()..];
        } else if let Some(stripped) = lower.strip_prefix("alt-") {
            mods |= KeyModifiers::ALT;
            rest = &rest[rest.len() - stripped.len()..];
        } else if let Some(stripped) = lower.strip_prefix("shift-") {
            mods |= KeyModifiers::SHIFT;
            rest = &rest[rest.len() - stripped.len()..];
        } else {
            break;
        }
    }
    if rest.is_empty() {
        return Err(KeyGrammarError::UnknownKeyName(token.to_string()));
    }
    let base_lower = rest.to_ascii_lowercase();
    // `digits` empty means the base name is just "f" (the plain letter key,
    // not a function-key prefix) — falls through to the bare-char arm below
    // rather than misparsing as an incomplete function-key name.
    let code = if let Some(digits) = base_lower
        .strip_prefix('f')
        .filter(|digits| !digits.is_empty())
    {
        match digits.parse::<u8>() {
            Ok(n) if (1..=24).contains(&n) => KeyCode::F(n),
            _ => return Err(KeyGrammarError::UnknownKeyName(token.to_string())),
        }
    } else {
        match base_lower.as_str() {
            "esc" | "escape" => KeyCode::Esc,
            "enter" | "return" => KeyCode::Enter,
            "tab" => KeyCode::Tab,
            "space" => KeyCode::Char(' '),
            "backspace" => KeyCode::Backspace,
            "delete" | "del" => KeyCode::Delete,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            "pageup" | "pgup" => KeyCode::PageUp,
            "pagedown" | "pgdn" => KeyCode::PageDown,
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "insert" | "ins" => KeyCode::Insert,
            _ => {
                let mut chars = rest.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => KeyCode::Char(c),
                    _ => return Err(KeyGrammarError::UnknownKeyName(token.to_string())),
                }
            }
        }
    };
    if code == KeyCode::Tab && mods == KeyModifiers::SHIFT {
        return Ok(ChordSpec {
            code: KeyCode::BackTab,
            mods: KeyModifiers::NONE,
        });
    }
    Ok(ChordSpec { code, mods })
}

/// Parses a full `[keys.*]` key string: one chord (`"ctrl-k"`), or two
/// space-separated chords (`"g d"`); zero, or three-or-more, chords is
/// rejected.
pub fn parse_key_string(s: &str) -> Result<KeySeqSpec, KeyGrammarError> {
    let tokens: Vec<&str> = s.split_whitespace().collect();
    match tokens.as_slice() {
        [] => Err(KeyGrammarError::Empty),
        [one] => Ok(KeySeqSpec::One(parse_chord(one)?)),
        [first, second] => Ok(KeySeqSpec::Two(parse_chord(first)?, parse_chord(second)?)),
        _ => Err(KeyGrammarError::TooManyChords(s.to_string())),
    }
}

/// The `[keys.<mode>]` table names for every modal mode, besides the main
/// keymap's `diff`/`panel`/`global`. Plain string literals —
/// this module must never import `crate::ui` (see the module doc) — so this
/// list is the config-side half of the contract; `crate::ui::modal_keys`'s
/// `MODAL_MODE_NAMES` is the ui-side half, and
/// `crate::ui::modal_keys_config`'s tests cross-check the two agree (that
/// module is allowed to import both).
const MODAL_MODE_NAMES: &[&str] = &[
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
    "filter-edit",
];

/// `[keys.diff]`/`[keys.panel]`/`[keys.global]`/`[keys.<mode>]`: raw
/// action-name -> key-string(s) overrides, already grammar-validated (an
/// unparseable key string is dropped with a warning at parse time — see
/// [`KeysConfig::from_value`]) but *not* yet resolved to a real action —
/// that needs the bijective name tables in `crate::ui::keymap`/
/// `crate::ui::modal_keys`, which this module must never import (see the
/// module doc). An empty `Vec` for an action name means "unbind" (the config
/// author wrote `= []`); an action name absent from its map keeps its
/// default untouched. `modal` holds every `[keys.<mode>]` table besides
/// `diff`/`panel`/`global`, keyed by mode name (one of [`MODAL_MODE_NAMES`])
/// — a single map rather than one field per mode, since
/// `crate::ui::modal_keys_config` (the edge module resolving these) already
/// needs one generic merge function reusable across all thirteen modes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeysConfig {
    pub diff: BTreeMap<String, Vec<KeySeqSpec>>,
    pub panel: BTreeMap<String, Vec<KeySeqSpec>>,
    pub global: BTreeMap<String, Vec<KeySeqSpec>>,
    pub modal: BTreeMap<String, BTreeMap<String, Vec<KeySeqSpec>>>,
}

impl KeysConfig {
    /// `pub(crate)` (not `pub(super)`) so `crate::ui::modal_keys_config`'s
    /// tests can drive this directly to cross-check its hardcoded
    /// [`MODAL_MODE_NAMES`] list against `crate::ui::modal_keys`'s — the same
    /// visibility `crate::ui::keymap_config::effective_keymap` already uses
    /// for the analogous main-keymap cross-check.
    pub(crate) fn from_value(value: toml::Value, warnings: &mut Vec<ConfigWarning>) -> KeysConfig {
        let mut cfg = KeysConfig::default();
        let Some(table) = value.as_table() else {
            warnings.push(ConfigWarning::invalid("keys", "keys", "expected a table"));
            return cfg;
        };
        for (section_key, section_val) in table {
            let target: &mut BTreeMap<String, Vec<KeySeqSpec>> = match section_key.as_str() {
                "diff" => &mut cfg.diff,
                "panel" => &mut cfg.panel,
                "global" => &mut cfg.global,
                other if MODAL_MODE_NAMES.contains(&other) => {
                    cfg.modal.entry(other.to_string()).or_default()
                }
                other => {
                    warnings.push(ConfigWarning::unknown("keys", other));
                    continue;
                }
            };
            let section_name = format!("keys.{section_key}");
            let Some(actions) = section_val.as_table() else {
                warnings.push(ConfigWarning::invalid(
                    &section_name,
                    &section_name,
                    "expected a table",
                ));
                continue;
            };
            for (action_name, key_value) in actions {
                match parse_key_value(key_value) {
                    Ok(specs) => {
                        target.insert(action_name.clone(), specs);
                    }
                    Err(message) => {
                        warnings.push(ConfigWarning::invalid(&section_name, action_name, message))
                    }
                }
            }
        }
        cfg
    }
}

/// Parses one `[keys.*]` entry's value: a single key string, or an array of
/// key strings (each independently parsed; `[]` is the explicit-unbind
/// case). On the first bad entry, the whole value is rejected — one ignored
/// entry (falling back to "no override" for that action), not a partial
/// merge of the array, matching the rest of the warning contract.
fn parse_key_value(value: &toml::Value) -> Result<Vec<KeySeqSpec>, String> {
    match value {
        toml::Value::String(s) => parse_key_string(s)
            .map(|spec| vec![spec])
            .map_err(|e| e.to_string()),
        toml::Value::Array(items) => {
            let mut specs = Vec::with_capacity(items.len());
            for item in items {
                match item.as_str() {
                    Some(s) => specs.push(parse_key_string(s).map_err(|e| e.to_string())?),
                    None => return Err("expected a string or array of strings".to_string()),
                }
            }
            Ok(specs)
        }
        _ => Err("expected a string or array of strings".to_string()),
    }
}

#[cfg(test)]
#[path = "keys_tests.rs"]
mod tests;
