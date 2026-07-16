//! CLI entry point: argument parsing and wiring for redquill.
//!
//! Owns the working-tree default, `--staged`, ref-range, and `-o file`
//! flags described in the README, and wires parsed args into the rest of
//! the crate.
//!
//! If stderr is a terminal, launches the interactive TUI (which renders to
//! stderr, keeping stdout free for annotation markdown emitted on quit). If
//! stderr is not a terminal — e.g. piped, or running under a test harness —
//! falls back to the plain-text summary so redquill stays scriptable.

use std::io::IsTerminal;
use std::path::PathBuf;

use clap::Parser;

use redquill::annotate::render_markdown;
use redquill::config;
use redquill::diff::FileDiff;
use redquill::git::{ChangeKind, DiffTarget, GitRunner};
use redquill::ui::{self, App, QuitOutcome, build_review};

/// redquill: a terminal UI for reviewing agentic code changes.
///
/// Reviews the working tree by default. Pass a ref range to review a
/// commit range instead, or `--staged` to review the index.
#[derive(Parser, Debug)]
#[command(name = "redquill", version, about, long_about = None)]
struct Cli {
    /// Ref range to review (e.g. `main..HEAD`). Defaults to the working
    /// tree when omitted. Mutually exclusive with `--staged`.
    #[arg(conflicts_with = "staged")]
    range: Option<String>,

    /// Review the staged index instead of the working tree.
    #[arg(long)]
    staged: bool,

    /// Also write emitted annotations to this file, in addition to stdout.
    #[arg(short = 'o', long = "output", value_name = "FILE")]
    output: Option<PathBuf>,

    /// Editor `g<Space>` opens in the diff viewer, e.g. `"nvim"` or `"code
    /// --wait"`. Overrides `$VISUAL`/`$EDITOR`; falls back to `nvim` if none
    /// of the three are set.
    #[arg(long, value_name = "CMD")]
    editor: Option<String>,
}

/// Fully resolved configuration derived from parsed CLI arguments.
struct RunConfig {
    /// Ref range to diff, if any; `None` means the working tree.
    range: Option<String>,
    /// Whether to review the staged index instead of the working tree.
    staged: bool,
    /// Optional file to additionally write annotations to, alongside stdout.
    output: Option<PathBuf>,
    /// The `--editor` flag, if passed; highest-precedence tier of
    /// [`resolve_editor`].
    editor: Option<String>,
}

impl From<Cli> for RunConfig {
    fn from(cli: Cli) -> Self {
        RunConfig {
            range: cli.range,
            staged: cli.staged,
            output: cli.output,
            editor: cli.editor,
        }
    }
}

impl RunConfig {
    /// Resolves the CLI flags into the diff target to inspect.
    fn diff_target(&self) -> DiffTarget {
        match &self.range {
            Some(range) => DiffTarget::Range(range.clone()),
            None if self.staged => DiffTarget::Staged,
            None => DiffTarget::WorkingTree,
        }
    }
}

/// Prints a one-line summary per changed file for the resolved diff target.
fn run(config: &RunConfig) -> anyhow::Result<()> {
    let runner = GitRunner::discover()?;
    let target = config.diff_target();
    let patches = runner.diff(&target)?;

    let mut printed = 0usize;
    for patch in &patches {
        let binary = if patch.is_binary { " (binary)" } else { "" };
        let diff = FileDiff::from_patch(patch)?;
        match &patch.old_path {
            Some(old) => println!("R  {old} -> {}{binary}", patch.path),
            None => println!("{}  {}{binary}", diff.kind.letter(), patch.path),
        }
        printed += 1;
    }

    // `git diff` never surfaces untracked files; pull them from status so the
    // working-tree summary is complete.
    if matches!(target, DiffTarget::WorkingTree) {
        for entry in runner.status()? {
            if entry.kind == ChangeKind::Untracked {
                println!("?  {}", entry.path);
                printed += 1;
            }
        }
    }

    if printed == 0 {
        println!("no changes");
    }
    Ok(())
}

/// Resolves the editor `g<Space>` opens, in precedence order: the
/// `--editor` flag, then `$VISUAL`, then `$EDITOR`, then `"nvim"`. Takes the
/// flag/env values as explicit args (rather than reading `std::env::var`
/// itself) so precedence is unit-testable without mutating process-global
/// env state; `run_tui` reads the real env vars at the one call site. Empty
/// or whitespace-only strings at any tier are treated as unset and fall
/// through to the next — an exported `EDITOR=""` shouldn't silently break
/// `g<Space>`.
fn resolve_editor(
    flag: Option<String>,
    visual: Option<String>,
    editor_env: Option<String>,
) -> String {
    [flag, visual, editor_env]
        .into_iter()
        .find_map(|candidate| candidate.filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| "nvim".to_string())
}

/// Runs the interactive TUI and, on quit, emits annotations per the
/// resolved [`QuitOutcome`].
fn run_tui(config: &RunConfig) -> anyhow::Result<()> {
    let runner = GitRunner::discover()?;
    let target = config.diff_target();
    let snapshot = build_review(&runner, &target)?;

    let mut app = App::with_git(snapshot, target, Box::new(runner.clone()));
    app.set_repo_root(runner.root().to_path_buf());
    // Config loads exactly once, here, before the first render — there is no
    // reload path (docs/specs/07-spec-config-layer). Warnings (missing file
    // is silent and yields none; a syntax error or an invalid entry each
    // yield one) are handed to the app for its dismissible status-line
    // notice; never printed to stdout.
    let (loaded_config, config_warnings) = config::load();
    app.set_config(loaded_config, config_warnings);
    let visual = std::env::var_os("VISUAL").map(|s| s.to_string_lossy().into_owned());
    let editor_env = std::env::var_os("EDITOR").map(|s| s.to_string_lossy().into_owned());
    app.set_editor(resolve_editor(config.editor.clone(), visual, editor_env));
    let outcome = ui::run(&mut app)?;

    if let QuitOutcome::Emit = outcome {
        let markdown = render_markdown(&app.annotations);
        print!("{markdown}");
        if let Some(path) = &config.output {
            std::fs::write(path, &markdown)?;
        }
    }

    Ok(())
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = RunConfig::from(cli);

    if std::io::stderr().is_terminal() {
        run_tui(&config)
    } else {
        run(&config)
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_editor;

    #[test]
    fn flag_wins_over_everything() {
        assert_eq!(
            resolve_editor(
                Some("code --wait".to_string()),
                Some("emacs".to_string()),
                Some("vi".to_string())
            ),
            "code --wait"
        );
    }

    #[test]
    fn visual_wins_when_no_flag() {
        assert_eq!(
            resolve_editor(None, Some("emacs".to_string()), Some("vi".to_string())),
            "emacs"
        );
    }

    #[test]
    fn editor_env_wins_when_no_flag_or_visual() {
        assert_eq!(resolve_editor(None, None, Some("vi".to_string())), "vi");
    }

    #[test]
    fn nvim_is_the_final_fallback() {
        assert_eq!(resolve_editor(None, None, None), "nvim");
    }

    #[test]
    fn empty_flag_falls_through_to_visual() {
        assert_eq!(
            resolve_editor(
                Some("   ".to_string()),
                Some("emacs".to_string()),
                Some("vi".to_string())
            ),
            "emacs"
        );
    }

    #[test]
    fn empty_visual_falls_through_to_editor_env() {
        assert_eq!(
            resolve_editor(None, Some("".to_string()), Some("vi".to_string())),
            "vi"
        );
    }

    #[test]
    fn empty_editor_env_falls_through_to_nvim() {
        assert_eq!(resolve_editor(None, None, Some("  \t".to_string())), "nvim");
    }
}
