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
use redquill::diff::{FileChangeKind, FileDiff};
use redquill::git::{ChangeKind, DiffTarget, GitRunner};
use redquill::ui::{self, App, QuitOutcome};

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
}

/// Fully resolved configuration derived from parsed CLI arguments.
struct Config {
    /// Ref range to diff, if any; `None` means the working tree.
    range: Option<String>,
    /// Whether to review the staged index instead of the working tree.
    staged: bool,
    /// Optional file to additionally write annotations to, alongside stdout.
    output: Option<PathBuf>,
}

impl From<Cli> for Config {
    fn from(cli: Cli) -> Self {
        Config {
            range: cli.range,
            staged: cli.staged,
            output: cli.output,
        }
    }
}

impl Config {
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
fn run(config: &Config) -> anyhow::Result<()> {
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

/// Builds the full `Vec<FileDiff>` for `target`: parsed patches plus, for
/// the working tree, synthetic entries for untracked files (`git diff`
/// never surfaces those).
fn build_files(runner: &GitRunner, target: &DiffTarget) -> anyhow::Result<Vec<FileDiff>> {
    let patches = runner.diff(target)?;
    let mut files = Vec::with_capacity(patches.len());
    for patch in &patches {
        files.push(FileDiff::from_patch(patch)?);
    }

    if matches!(target, DiffTarget::WorkingTree) {
        for entry in runner.status()? {
            if entry.kind != ChangeKind::Untracked {
                continue;
            }
            let path = runner.root().join(&entry.path);
            let Ok(bytes) = std::fs::read(&path) else {
                // Unreadable (permissions, race with deletion, ...): skip
                // rather than fail the whole review session.
                continue;
            };
            let file = match String::from_utf8(bytes) {
                Ok(content) => FileDiff::synthetic_added(entry.path.clone(), &content),
                Err(_) => FileDiff {
                    path: entry.path.clone(),
                    old_path: None,
                    kind: FileChangeKind::Added,
                    is_binary: true,
                    hunks: Vec::new(),
                },
            };
            files.push(file);
        }
    }

    Ok(files)
}

/// Runs the interactive TUI and, on quit, emits annotations per the
/// resolved [`QuitOutcome`].
fn run_tui(config: &Config) -> anyhow::Result<()> {
    let runner = GitRunner::discover()?;
    let target = config.diff_target();
    let files = build_files(&runner, &target)?;

    let mut app = App::new(files);
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
    let config = Config::from(cli);

    if std::io::stderr().is_terminal() {
        run_tui(&config)
    } else {
        run(&config)
    }
}
