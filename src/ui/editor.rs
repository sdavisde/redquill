//! Pure construction of the argv `g<Space>` spawns the configured editor
//! with, across two independent grammars that share the one call site
//! (`launch_editor` in `super`):
//!
//! - [`build_editor_command`]: today's original two-family heuristic for a
//!   plain editor string (`--editor`/`$VISUAL`/`$EDITOR`/`"nvim"`) — splits
//!   the string into a program and leading args, then appends either
//!   `+line` (vim/nvim/emacs/nano/helix's shared convention) or, for VS
//!   Code/VSCodium specifically, `--goto path:line`.
//! - [`build_from_template`]: the lazygit-style `[editor] edit_at_line`/
//!   `preset` template grammar — `{{filename}}`/`{{line}}` placeholders
//!   substituted per whitespace
//!   token, never through a shell, so a filename containing spaces survives
//!   intact as long as it's the *only* thing in its token (the common case:
//!   `{{filename}}` alone, or a mixed token like `{{filename}}:{{line}}`).
//!
//! [`EditorConfigTier`]/[`resolve_editor_config_tier`] resolve the
//! `[editor]` section itself (`crate::config::EditorConfig`) into either a
//! template (explicit `edit_at_line`, or `preset` expanded via the built-in
//! [`PRESETS`] table) or nothing — the second-highest of
//! `main::resolve_editor`'s five precedence tiers (`--editor` flag >
//! config > `$VISUAL` > `$EDITOR` > `"nvim"`). [`EditorLaunch`] is what that
//! whole resolution ultimately produces: either variant reaches
//! [`super::launch_editor`], which picks the right grammar function above.
//!
//! Kept pure and unit-tested so the shell-free argv construction is
//! exercised without spawning a real process. The actual spawn
//! (`std::process::Command`, inherited stdio, a synchronous `.status()`
//! wait) happens at the one call site in [`super`]'s event loop — the
//! sanctioned exception to "never block the render loop," since the
//! terminal has already been suspended (`restore_terminal`) by the time it
//! runs.

use std::path::Path;

use crate::config::EditorConfig;

/// Splits `editor` into a program and leading args, then builds the full
/// argv to open `path` at `line` (1-based). Falls back to `"nvim"` when
/// `editor` is empty or whitespace-only (mirrors `main::resolve_editor`'s
/// "empty is unset" rule; defensively re-applied here in case a caller ever
/// passes an unvalidated string). `path` is expected to be repo-relative —
/// the caller spawns with the repo root as the child's working directory, so
/// the editor opens exactly the argument shown to the user (`nvim +42
/// path/to/file.rs`), not an absolute path.
pub(super) fn build_editor_command(editor: &str, path: &Path, line: u32) -> (String, Vec<String>) {
    let mut tokens = editor.split_whitespace();
    let program = tokens.next().unwrap_or("nvim").to_string();
    let mut args: Vec<String> = tokens.map(str::to_string).collect();
    let path_str = path.to_string_lossy().into_owned();

    // VS Code/VSCodium's own `--goto path:line` flag, rather than the `+N`
    // convention every other editor here honors — matched on the program's
    // basename so a full path (`/usr/local/bin/code`) still special-cases
    // correctly.
    let basename = Path::new(&program)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| program.clone());
    if basename == "code" || basename == "codium" {
        args.push("--goto".to_string());
        args.push(format!("{path_str}:{line}"));
    } else {
        args.push(format!("+{line}"));
        args.push(path_str);
    }
    (program, args)
}

/// Splits `template` into whitespace-separated tokens *before* substituting
/// `{{filename}}`/`{{line}}` into each — the ordering that lets a filename
/// containing spaces survive as a single argv element even though the
/// substituted text now contains internal whitespace (substituting first
/// and splitting after would re-split the filename itself, and templates
/// never go through a shell in the first place — see the module doc).
///
/// A token may mix literal text with one or both placeholders (e.g.
/// `{{filename}}:{{line}}`); `{{line}}` is optional (a template that never
/// mentions it simply ignores `line`). Returns `None` when `template` has
/// no `{{filename}}` placeholder at all — a template without
/// `{{filename}}` is an invalid value; callers treat
/// `None` as a config-tier miss, falling through to the next precedence
/// tier rather than spawning anything.
pub(super) fn build_from_template(
    template: &str,
    path: &Path,
    line: u32,
) -> Option<(String, Vec<String>)> {
    if !template.contains("{{filename}}") {
        return None;
    }
    let path_str = path.to_string_lossy();
    let line_str = line.to_string();
    let mut tokens = template.split_whitespace().map(|token| {
        token
            .replace("{{filename}}", &path_str)
            .replace("{{line}}", &line_str)
    });
    let program = tokens.next()?;
    let args: Vec<String> = tokens.collect();
    Some((program, args))
}

/// The eleven built-in `[editor] preset` names, each
/// mapped to a known-correct `edit_at_line` template for that editor's own
/// line-jump CLI convention — every template here is validated by its own
/// test in `editor_tests.rs`. Growing this table (spec Open Question 3) is
/// one data row plus one test.
const PRESETS: &[(&str, &str)] = &[
    ("vim", "vim +{{line}} {{filename}}"),
    ("nvim", "nvim +{{line}} {{filename}}"),
    ("helix", "hx {{filename}}:{{line}}"),
    ("vscode", "code --goto {{filename}}:{{line}}"),
    ("vscodium", "codium --goto {{filename}}:{{line}}"),
    ("zed", "zed {{filename}}:{{line}}"),
    ("emacs", "emacs +{{line}} {{filename}}"),
    ("nano", "nano +{{line}} {{filename}}"),
    ("micro", "micro {{filename}}:{{line}}"),
    ("sublime", "subl {{filename}}:{{line}}"),
    ("kakoune", "kak +{{line}} {{filename}}"),
];

/// Looks up `name` in the built-in [`PRESETS`] table; `None` means an
/// unrecognized preset name — an invalid value (see
/// [`EditorConfigTier::UnknownPreset`]).
fn preset_template(name: &str) -> Option<&'static str> {
    PRESETS
        .iter()
        .find(|(preset_name, _)| *preset_name == name)
        .map(|(_, template)| *template)
}

/// What `main::resolve_editor`'s config-precedence tier resolves an
/// `[editor]` section to. [`EditorConfigTier::Absent`]: neither `preset`
/// nor `edit_at_line` was set — falls through to `$VISUAL`.
/// [`EditorConfigTier::Template`]: a usable template was found — explicit
/// `edit_at_line` wins over `preset` whenever both are set, since
/// [`resolve_editor_config_tier`] checks it first. [`EditorConfigTier::UnknownPreset`]:
/// `preset` named something outside [`PRESETS`] — an invalid value the
/// caller (`main::run_tui`) reports through the same `ConfigWarning`
/// collection `crate::config::load` produces, then treats exactly like
/// `Absent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorConfigTier {
    /// No config-tier editor is set.
    Absent,
    /// A resolved, ready-to-use `edit_at_line`-style template.
    Template(String),
    /// `preset` named something outside the built-in table.
    UnknownPreset(String),
}

/// Resolves `cfg` (`[editor]`, already parsed and type-checked by
/// `crate::config::EditorConfig::from_value`) into an [`EditorConfigTier`].
/// Pure, and pre-dates any process spawn — see the module doc for why the
/// *preset name* validity check lives here rather than in `crate::config`
/// (the preset table is domain data that belongs beside the template
/// grammar it feeds, and `crate::config` must never import `crate::ui`).
pub fn resolve_editor_config_tier(cfg: &EditorConfig) -> EditorConfigTier {
    if let Some(template) = &cfg.edit_at_line {
        return EditorConfigTier::Template(template.clone());
    }
    match &cfg.preset {
        Some(name) => match preset_template(name) {
            Some(template) => EditorConfigTier::Template(template.to_string()),
            None => EditorConfigTier::UnknownPreset(name.clone()),
        },
        None => EditorConfigTier::Absent,
    }
}

/// What `main::resolve_editor`'s five-tier precedence ultimately produces:
/// either a config-supplied [`EditorLaunch::Template`] (bypasses
/// [`build_editor_command`]'s family heuristic entirely — the template *is*
/// the whole argv rule, built by [`build_from_template`]) or a plain
/// [`EditorLaunch::Command`] string from `--editor`/`$VISUAL`/`$EDITOR`/the
/// `"nvim"` default (still goes through the family heuristic). `Default`
/// matches today's shipped fallback so an `App` built without `main`'s
/// resolution (every pre-existing unit test) still has a usable editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorLaunch {
    /// A `[editor]`-config template, applied via [`build_from_template`].
    Template(String),
    /// A plain editor command, applied via [`build_editor_command`].
    Command(String),
}

impl Default for EditorLaunch {
    fn default() -> Self {
        EditorLaunch::Command("nvim".to_string())
    }
}

#[cfg(test)]
#[path = "editor_tests.rs"]
mod tests;
