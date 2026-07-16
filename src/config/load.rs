//! Path discovery plus the one-shot startup load: reads the config file (if
//! any) exactly once, parses it, and returns a fully-defaulted [`Config`]
//! plus every [`ConfigWarning`] collected along the way. Never writes to
//! stdout (see the module doc's degradation contract) and never panics —
//! every I/O or parse failure degrades to defaults, per that contract.

use std::path::{Path, PathBuf};

use super::{Config, ConfigWarning};

/// The environment inputs config-path discovery depends on, injected
/// explicitly (rather than reading `std::env` inside the resolver) so
/// discovery is a pure, unit-testable function; [`load`] is the sole real
/// call site that reads the actual process environment. Integration tests
/// use this same struct, pointed at a tempdir, as their path-override hook
/// — never the developer's real `~/.config/redquill/config.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PathEnv {
    /// `$XDG_CONFIG_HOME`, if set.
    pub xdg_config_home: Option<PathBuf>,
    /// The user's home directory (`$HOME` on Linux/macOS).
    pub home: Option<PathBuf>,
    /// `%APPDATA%` on Windows; unused on Linux/macOS.
    pub appdata: Option<PathBuf>,
}

/// Resolves the config file path per the Unit 1 FR: `$XDG_CONFIG_HOME` if
/// set, else `~/.config` on Linux **and macOS** (deliberately not
/// `~/Library/Application Support`, matching helix/yazi user expectations —
/// the `directories` crate is ruled out for returning the latter); the
/// platform config directory (`%APPDATA%`) on Windows. `None` when the
/// platform's relevant directory can't be determined at all — callers treat
/// that exactly like "no config file" (silent defaults).
pub fn resolve_config_path(env: &PathEnv) -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        return env
            .appdata
            .as_deref()
            .map(|dir| dir.join("redquill").join("config.toml"));
    }
    if let Some(xdg) = &env.xdg_config_home {
        return Some(xdg.join("redquill").join("config.toml"));
    }
    env.home
        .as_deref()
        .map(|home| home.join(".config").join("redquill").join("config.toml"))
}

/// Parses `text` (a whole config file's contents) into a [`Config`] plus
/// every warning encountered: a TOML syntax error nukes the whole file to
/// defaults (one [`ConfigWarning::SyntaxError`] carrying `path` and the
/// parser's own line/column message); otherwise unknown keys and invalid
/// values are collected per `Config::from_table` while every valid setting
/// still applies.
fn parse(text: &str, path: &Path) -> (Config, Vec<ConfigWarning>) {
    match text.parse::<toml::Table>() {
        Ok(raw) => Config::from_table(raw),
        Err(err) => (
            Config::default(),
            vec![ConfigWarning::SyntaxError {
                path: path.display().to_string(),
                message: err.to_string(),
            }],
        ),
    }
}

/// The real, one-shot startup load: resolves the path from the actual
/// process environment, reads the file, and parses it. Called exactly once,
/// from `main`, before the first render — there is no reload path.
pub fn load() -> (Config, Vec<ConfigWarning>) {
    let env = PathEnv {
        xdg_config_home: std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        home: std::env::var_os("HOME").map(PathBuf::from),
        appdata: std::env::var_os("APPDATA").map(PathBuf::from),
    };
    load_from(&env)
}

/// [`load`]'s pure core: the same contract, over an injected [`PathEnv`]
/// rather than the real process environment — the hook integration tests use
/// to point discovery at a tempdir config without touching the developer's
/// real `~/.config`. A missing file (or one that exists but can't be read)
/// is silent — [`Config::default()`], zero warnings, matching today's
/// shipped behavior exactly.
pub fn load_from(env: &PathEnv) -> (Config, Vec<ConfigWarning>) {
    let Some(path) = resolve_config_path(env) else {
        return (Config::default(), Vec::new());
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => parse(&text, &path),
        Err(_) => (Config::default(), Vec::new()),
    }
}

#[cfg(test)]
#[path = "load_tests.rs"]
mod tests;
