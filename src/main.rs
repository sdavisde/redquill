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
use redquill::ui::{
    self, App, EditorConfigTier, EditorLaunch, QuitOutcome, build_review,
    resolve_editor_config_tier,
};

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

/// Resolves the editor `g<Space>` opens, in five-tier precedence order: the
/// `--editor` flag, then `[editor]` config (`config_tier`, already resolved
/// from `crate::config::EditorConfig` by
/// `ui::resolve_editor_config_tier` — an `EditorConfigTier::UnknownPreset`
/// is *not* a config-tier hit here; `run_tui` reports it as a warning and
/// passes `EditorConfigTier::Absent` in its place instead, so this function
/// only ever sees a real hit or a miss), then `$VISUAL`, then `$EDITOR`,
/// then `"nvim"`. Takes every tier as an explicit arg (rather than reading
/// `std::env::var`/resolving config itself) so precedence is unit-testable
/// without mutating process-global env state or touching real config paths;
/// `run_tui` reads the real env vars and config at the one call site. Empty
/// or whitespace-only strings at the flag/`$VISUAL`/`$EDITOR` tiers are
/// treated as unset and fall through to the next — an exported `EDITOR=""`
/// shouldn't silently break `g<Space>` (unchanged from before this spec). A
/// config-tier template always wins over `$VISUAL`/`$EDITOR`/`"nvim"` and
/// carries no such "empty is unset" allowance: it's already been validated
/// non-empty by `crate::config::EditorConfig::from_value`.
fn resolve_editor(
    flag: Option<String>,
    config_tier: EditorConfigTier,
    visual: Option<String>,
    editor_env: Option<String>,
) -> EditorLaunch {
    if let Some(cmd) = flag.filter(|s| !s.trim().is_empty()) {
        return EditorLaunch::Command(cmd);
    }
    if let EditorConfigTier::Template(template) = config_tier {
        return EditorLaunch::Template(template);
    }
    [visual, editor_env]
        .into_iter()
        .find_map(|candidate| candidate.filter(|s| !s.trim().is_empty()))
        .map(EditorLaunch::Command)
        .unwrap_or_else(|| EditorLaunch::Command("nvim".to_string()))
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
    let (loaded_config, mut config_warnings) = config::load();
    // The `[editor]` config tier is resolved *before* `loaded_config` moves
    // into `app.set_config` below. An unknown preset name is folded into
    // the same warning collection here (`ui::editor` owns the preset table
    // that names its validity against — see that module's doc for why
    // `crate::config` can't do this check itself) and then treated exactly
    // like an absent config tier for precedence purposes.
    let config_tier = match resolve_editor_config_tier(&loaded_config.editor) {
        EditorConfigTier::UnknownPreset(name) => {
            config_warnings.push(config::ConfigWarning::InvalidValue {
                section: "editor".to_string(),
                key: "preset".to_string(),
                message: format!("unknown preset \"{name}\""),
            });
            EditorConfigTier::Absent
        }
        resolved => resolved,
    };
    app.set_config(loaded_config, config_warnings);
    let visual = std::env::var_os("VISUAL").map(|s| s.to_string_lossy().into_owned());
    let editor_env = std::env::var_os("EDITOR").map(|s| s.to_string_lossy().into_owned());
    app.set_editor(resolve_editor(
        config.editor.clone(),
        config_tier,
        visual,
        editor_env,
    ));
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
    use redquill::ui::{EditorConfigTier, EditorLaunch};

    #[test]
    fn flag_wins_over_everything() {
        assert_eq!(
            resolve_editor(
                Some("code --wait".to_string()),
                EditorConfigTier::Template("zed {{filename}}:{{line}}".to_string()),
                Some("emacs".to_string()),
                Some("vi".to_string())
            ),
            EditorLaunch::Command("code --wait".to_string())
        );
    }

    #[test]
    fn config_template_wins_when_no_flag() {
        assert_eq!(
            resolve_editor(
                None,
                EditorConfigTier::Template("zed {{filename}}:{{line}}".to_string()),
                Some("emacs".to_string()),
                Some("vi".to_string())
            ),
            EditorLaunch::Template("zed {{filename}}:{{line}}".to_string())
        );
    }

    #[test]
    fn visual_wins_when_no_flag_or_config_tier() {
        assert_eq!(
            resolve_editor(
                None,
                EditorConfigTier::Absent,
                Some("emacs".to_string()),
                Some("vi".to_string())
            ),
            EditorLaunch::Command("emacs".to_string())
        );
    }

    #[test]
    fn editor_env_wins_when_no_flag_config_tier_or_visual() {
        assert_eq!(
            resolve_editor(None, EditorConfigTier::Absent, None, Some("vi".to_string())),
            EditorLaunch::Command("vi".to_string())
        );
    }

    #[test]
    fn nvim_is_the_final_fallback() {
        assert_eq!(
            resolve_editor(None, EditorConfigTier::Absent, None, None),
            EditorLaunch::Command("nvim".to_string())
        );
    }

    #[test]
    fn empty_flag_falls_through_to_config_tier() {
        assert_eq!(
            resolve_editor(
                Some("   ".to_string()),
                EditorConfigTier::Template("zed {{filename}}:{{line}}".to_string()),
                Some("emacs".to_string()),
                Some("vi".to_string())
            ),
            EditorLaunch::Template("zed {{filename}}:{{line}}".to_string())
        );
    }

    #[test]
    fn empty_flag_and_absent_config_tier_falls_through_to_visual() {
        assert_eq!(
            resolve_editor(
                Some("   ".to_string()),
                EditorConfigTier::Absent,
                Some("emacs".to_string()),
                Some("vi".to_string())
            ),
            EditorLaunch::Command("emacs".to_string())
        );
    }

    #[test]
    fn empty_visual_falls_through_to_editor_env() {
        assert_eq!(
            resolve_editor(
                None,
                EditorConfigTier::Absent,
                Some("".to_string()),
                Some("vi".to_string())
            ),
            EditorLaunch::Command("vi".to_string())
        );
    }

    #[test]
    fn empty_editor_env_falls_through_to_nvim() {
        assert_eq!(
            resolve_editor(
                None,
                EditorConfigTier::Absent,
                None,
                Some("  \t".to_string())
            ),
            EditorLaunch::Command("nvim".to_string())
        );
    }
}
