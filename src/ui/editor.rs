//! Pure construction of the argv `g<Space>` spawns the configured editor
//! with. [`build_editor_command`] splits the configured editor string into a
//! program (its first whitespace-separated token) plus any leading args
//! (e.g. `"code --wait"` -> program `code`, leading args `["--wait"]`), then
//! appends either a `+line` positional (the convention vim/nvim/emacs/
//! nano/helix all honor to open at a given line) or, for VS Code/VSCodium
//! specifically, `--goto path:line` (their own line-jump flag — `+N` isn't
//! meaningful to them).
//!
//! Kept pure and unit-tested so the shell-free argv construction is
//! exercised without spawning a real process. The actual spawn
//! (`std::process::Command`, inherited stdio, a synchronous `.status()` wait)
//! happens at the one call site in [`super`]'s event loop — the sanctioned
//! exception to "never block the render loop," since the terminal has
//! already been suspended (`restore_terminal`) by the time it runs.

use std::path::Path;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nvim_uses_plus_line_convention() {
        let (program, args) = build_editor_command("nvim", Path::new("path/to/file.rs"), 42);
        assert_eq!(program, "nvim");
        assert_eq!(args, vec!["+42".to_string(), "path/to/file.rs".to_string()]);
    }

    #[test]
    fn code_with_leading_args_uses_goto_flag() {
        let (program, args) = build_editor_command("code --wait", Path::new("path/to/file.rs"), 42);
        assert_eq!(program, "code");
        assert_eq!(
            args,
            vec![
                "--wait".to_string(),
                "--goto".to_string(),
                "path/to/file.rs:42".to_string(),
            ]
        );
    }

    #[test]
    fn codium_special_cases_like_code() {
        let (program, args) = build_editor_command("codium", Path::new("f.rs"), 7);
        assert_eq!(program, "codium");
        assert_eq!(args, vec!["--goto".to_string(), "f.rs:7".to_string()]);
    }

    #[test]
    fn full_path_to_code_binary_still_special_cases_by_basename() {
        let (program, args) = build_editor_command("/usr/local/bin/code", Path::new("f.rs"), 3);
        assert_eq!(program, "/usr/local/bin/code");
        assert_eq!(args, vec!["--goto".to_string(), "f.rs:3".to_string()]);
    }

    #[test]
    fn empty_editor_string_falls_back_to_nvim() {
        let (program, args) = build_editor_command("", Path::new("f.rs"), 1);
        assert_eq!(program, "nvim");
        assert_eq!(args, vec!["+1".to_string(), "f.rs".to_string()]);
    }

    #[test]
    fn whitespace_only_editor_string_falls_back_to_nvim() {
        let (program, args) = build_editor_command("   ", Path::new("f.rs"), 1);
        assert_eq!(program, "nvim");
        assert_eq!(args, vec!["+1".to_string(), "f.rs".to_string()]);
    }
}
