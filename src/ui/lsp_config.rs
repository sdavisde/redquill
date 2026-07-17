//! Merges the `[lsp]` config section (`crate::config::LspConfig`) onto the
//! built-in per-language server table
//! (`crate::lsp::config::default_commands`) to produce the effective
//! `HashMap<ServerLang, LangServerCmd>` that `LspManager` is constructed
//! with.
//!
//! **Layering**: `crate::lsp` must never import `crate::config`, and
//! `crate::config` must never import `crate::lsp` — [`LspConfig`]'s fields
//! are plain `Option<String>`/`Option<Vec<String>>`/`bool` data with no
//! `ServerLang`/`LangServerCmd` knowledge at all. Something has to import
//! *both* to perform the overlay; that's this module, in `crate::ui` — the
//! same edge the `[editor]` section already uses for its own
//! config-plus-built-in-table merge (`resolve_editor_config_tier`/
//! `PRESETS` in `super::editor`). `LspManager` (and everything in
//! `crate::lsp`) only ever receives the resulting plain `HashMap`, never a
//! config type.

use std::collections::HashMap;

use crate::config::LspConfig;
use crate::lsp::{LangServerCmd, ServerLang, default_commands};

/// Overlays `cfg` onto [`default_commands`], producing the effective
/// per-language launch table:
///
/// - An unconfigured language keeps its default command and args exactly.
/// - A configured `command` with no `args` replaces the command only; the
///   language's default args are kept (the common case: an overridden
///   command is a wrapper that forwards its args to the real server, or a
///   drop-in binary that takes the same flags).
/// - Configured `args` with no `command` replace the args only, keeping the
///   default command.
/// - `enabled = false` removes the language from the returned map entirely
///   — `LspManager`/`ServerLang::from_path` then see exactly the same "no
///   server configured for this language" shape as an unrecognized file
///   extension, which already degrades silently today (no server spawned,
///   `gd`/`gr`/`K` fall back to the "no code intelligence here" footer
///   message).
pub fn effective_lsp_commands(cfg: &LspConfig) -> HashMap<ServerLang, LangServerCmd> {
    let mut map = default_commands();
    for (lang, overlay) in [
        (ServerLang::Rust, &cfg.rust),
        (ServerLang::TypeScript, &cfg.typescript),
        (ServerLang::Python, &cfg.python),
        (ServerLang::Go, &cfg.go),
    ] {
        if !overlay.enabled {
            map.remove(&lang);
            continue;
        }
        if let Some(existing) = map.get_mut(&lang) {
            if let Some(command) = &overlay.command {
                existing.command = command.clone();
            }
            if let Some(args) = &overlay.args {
                existing.args = args.clone();
            }
        }
    }
    map
}

#[cfg(test)]
#[path = "lsp_config_tests.rs"]
mod tests;
