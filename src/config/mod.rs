//! Config layer (spec 07): a single, optional TOML file at
//! `$XDG_CONFIG_HOME/redquill/config.toml` (falling back to
//! `~/.config/redquill/config.toml` on Linux and macOS; the platform config
//! directory on Windows), read exactly once at startup by [`load`]. There is
//! no reload mechanism — see `src/main.rs` for the one call site.
//!
//! **Degradation contract** (rust-best-practices: silent-vs-surfaced must be
//! written down per subsystem):
//! - No config file: silent. [`Config::default()`], zero warnings — every
//!   default matches today's shipped behavior exactly.
//! - A TOML syntax error: the *whole file* is ignored (full defaults), with
//!   one [`ConfigWarning::SyntaxError`] naming the file path and the
//!   parser's own line/column-carrying message.
//! - A parseable file with an unknown key or an invalid value for a known
//!   key: every other valid setting still applies; the offending entry falls
//!   back to its default and is collected as one [`ConfigWarning`] each.
//! - Nothing here is ever written to stdout — stdout is reserved for the
//!   annotation markdown (see `crate::annotate`). Warnings are surfaced
//!   through the UI's dismissible status-line notice instead (see
//!   `crate::ui::App::config_warnings`).
//!
//! **Extensibility** (the contract a future `[theme]` section — or `[editor]`
//! /`[lsp]`/`[keys]` in this spec's later units — follows): adding a section
//! is (1) a new section struct with `Default` matching today's behavior, (2)
//! one new field on [`Config`], (3) one new arm in [`Config::from_table`]
//! validating that section's known keys the same way [`LayoutConfig`]/
//! [`SearchConfig`] do. None of path discovery ([`load`]), the outer TOML
//! parse, or the warning-notice plumbing in `crate::ui` need to change.
//!
//! **Layering**: this module is edge-side, like `crate::git` — it may depend
//! on other domain modules (it reuses `crate::search::query::CaseMode`
//! rather than duplicating a case-sensitivity enum) but must never import
//! `crate::ui` or any TUI/ratatui type; `Config` crosses into
//! `crate::ui::App` as plain data, never the other way around.

mod load;

pub use load::{PathEnv, load, load_from, resolve_config_path};

use thiserror::Error;

use crate::search::query::CaseMode;

/// The sidebar's side (`[layout] sidebar_side`); default
/// [`SidebarSide::Right`], matching today's shipped, only-ever-right layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarSide {
    /// Sidebar renders on the left.
    Left,
    /// Sidebar renders on the right (today's only behavior).
    #[default]
    Right,
}

impl SidebarSide {
    fn parse(s: &str) -> Option<SidebarSide> {
        match s {
            "left" => Some(SidebarSide::Left),
            "right" => Some(SidebarSide::Right),
            _ => None,
        }
    }
}

/// The smallest accepted `[layout] sidebar_width`, in columns. Below this the
/// sidebar has no useful room for filenames; out-of-range values are an
/// invalid value (warning + default), not a silent clamp — see
/// `crate::ui`'s own `sidebar_width` for the separate render-time clamp to
/// whatever width the terminal actually has this frame.
pub const SIDEBAR_WIDTH_MIN: u16 = 20;
/// The largest accepted `[layout] sidebar_width`, in columns.
pub const SIDEBAR_WIDTH_MAX: u16 = 200;

/// `[layout]`: sidebar placement and width. `sidebar_width: None` (unset)
/// preserves today's 30%-of-terminal-clamped-to-`[40, 72]` formula exactly;
/// `Some(w)` uses `w` (already validated against [`SIDEBAR_WIDTH_MIN`]/
/// [`SIDEBAR_WIDTH_MAX`] at load time here), further clamped to the
/// terminal's actual width at render time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LayoutConfig {
    pub sidebar_side: SidebarSide,
    pub sidebar_width: Option<u16>,
}

/// `[search]`: Project Search (`g/`) startup defaults. In-session toggles
/// (`Alt-c`/`Alt-w`/`Alt-r`) are unaffected once a search session is already
/// open — this only seeds the state a *fresh* session opens with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SearchConfig {
    pub case: CaseMode,
    pub whole_word: bool,
    pub literal: bool,
}

/// The full, partial-override config: one field per `[section]`, each
/// defaulting to today's shipped behavior. See the module doc's
/// extensibility note for how a future section is added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Config {
    pub layout: LayoutConfig,
    pub search: SearchConfig,
}

/// One problem [`load`] encountered, in a form ready for the UI's warning
/// notice (`Display` gives the human-readable message; nothing here is ever
/// written to stdout). Never fatal — every warning corresponds to one
/// ignored entry (or the whole file, for [`ConfigWarning::SyntaxError`])
/// falling back to its default while everything else still applies.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ConfigWarning {
    /// The file failed to parse as TOML at all; the whole file is ignored
    /// and every section uses its default.
    #[error("{path}: {message}")]
    SyntaxError { path: String, message: String },
    /// A known key in `section` had a value that couldn't be used (wrong
    /// type, out of range, or an unrecognized enum string); that key's
    /// default applies instead.
    #[error("[{section}] {key}: {message}")]
    InvalidValue {
        section: String,
        key: String,
        message: String,
    },
    /// A key in `section` isn't one this version of redquill recognizes;
    /// ignored.
    #[error("[{section}] unknown key `{key}`")]
    UnknownKey { section: String, key: String },
}

impl ConfigWarning {
    fn invalid(section: &str, key: &str, message: impl Into<String>) -> ConfigWarning {
        ConfigWarning::InvalidValue {
            section: section.to_string(),
            key: key.to_string(),
            message: message.into(),
        }
    }

    fn unknown(section: &str, key: &str) -> ConfigWarning {
        ConfigWarning::UnknownKey {
            section: section.to_string(),
            key: key.to_string(),
        }
    }
}

impl Config {
    /// Builds a [`Config`] from an already-parsed top-level TOML table (see
    /// [`load`], the sole real caller): known sections (`layout`, `search`)
    /// are validated key-by-key — unknown keys and invalid values are
    /// collected as warnings and defaulted, never fatal; any other
    /// top-level key is itself an unknown key.
    fn from_table(mut raw: toml::Table) -> (Config, Vec<ConfigWarning>) {
        let mut warnings = Vec::new();
        let mut config = Config::default();

        if let Some(value) = raw.remove("layout") {
            config.layout = LayoutConfig::from_value(value, &mut warnings);
        }
        if let Some(value) = raw.remove("search") {
            config.search = SearchConfig::from_value(value, &mut warnings);
        }
        for key in raw.keys() {
            warnings.push(ConfigWarning::unknown("top-level", key));
        }
        (config, warnings)
    }
}

impl LayoutConfig {
    fn from_value(value: toml::Value, warnings: &mut Vec<ConfigWarning>) -> LayoutConfig {
        let mut cfg = LayoutConfig::default();
        let Some(table) = value.as_table() else {
            warnings.push(ConfigWarning::invalid(
                "layout",
                "layout",
                "expected a table",
            ));
            return cfg;
        };
        for (key, val) in table {
            match key.as_str() {
                "sidebar_side" => match val.as_str().and_then(SidebarSide::parse) {
                    Some(side) => cfg.sidebar_side = side,
                    None => warnings.push(ConfigWarning::invalid(
                        "layout",
                        key,
                        "expected \"left\" or \"right\"",
                    )),
                },
                "sidebar_width" => match val.as_integer() {
                    Some(n)
                        if (i64::from(SIDEBAR_WIDTH_MIN)..=i64::from(SIDEBAR_WIDTH_MAX))
                            .contains(&n) =>
                    {
                        cfg.sidebar_width = Some(n as u16);
                    }
                    _ => warnings.push(ConfigWarning::invalid(
                        "layout",
                        key,
                        format!("expected an integer in {SIDEBAR_WIDTH_MIN}..={SIDEBAR_WIDTH_MAX}"),
                    )),
                },
                other => warnings.push(ConfigWarning::unknown("layout", other)),
            }
        }
        cfg
    }
}

impl SearchConfig {
    fn from_value(value: toml::Value, warnings: &mut Vec<ConfigWarning>) -> SearchConfig {
        let mut cfg = SearchConfig::default();
        let Some(table) = value.as_table() else {
            warnings.push(ConfigWarning::invalid(
                "search",
                "search",
                "expected a table",
            ));
            return cfg;
        };
        for (key, val) in table {
            match key.as_str() {
                "case" => match val.as_str().and_then(parse_case_mode) {
                    Some(mode) => cfg.case = mode,
                    None => warnings.push(ConfigWarning::invalid(
                        "search",
                        key,
                        "expected \"smart\", \"sensitive\", or \"insensitive\"",
                    )),
                },
                "whole_word" => match val.as_bool() {
                    Some(b) => cfg.whole_word = b,
                    None => {
                        warnings.push(ConfigWarning::invalid("search", key, "expected a boolean"))
                    }
                },
                "literal" => match val.as_bool() {
                    Some(b) => cfg.literal = b,
                    None => {
                        warnings.push(ConfigWarning::invalid("search", key, "expected a boolean"))
                    }
                },
                other => warnings.push(ConfigWarning::unknown("search", other)),
            }
        }
        cfg
    }
}

fn parse_case_mode(s: &str) -> Option<CaseMode> {
    match s {
        "smart" => Some(CaseMode::Smart),
        "sensitive" => Some(CaseMode::Sensitive),
        "insensitive" => Some(CaseMode::Insensitive),
        _ => None,
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
