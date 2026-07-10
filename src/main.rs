//! CLI entry point: argument parsing and wiring for redquill.
//!
//! Owns the working-tree default, `--staged`, ref-range, and `-o file`
//! flags described in the README, and wires parsed args into the rest of
//! the crate. No behavior beyond argument parsing lives here yet.

use std::path::PathBuf;

use clap::Parser;

use redquill::git::{ChangeKind, DiffTarget, GitRunner, RawFilePatch};

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
    ///
    /// Not implemented yet.
    #[arg(short = 'o', long = "output", value_name = "FILE")]
    output: Option<PathBuf>,
}

/// Fully resolved configuration derived from parsed CLI arguments.
struct Config {
    /// Ref range to diff, if any; `None` means the working tree.
    range: Option<String>,
    /// Whether to review the staged index instead of the working tree.
    staged: bool,
    /// Optional file to additionally write annotations to.
    ///
    /// Not consumed yet; annotation output lands in a later task.
    #[allow(dead_code)]
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

/// Derives a single-letter change label for a patch from its raw text.
fn patch_letter(patch: &RawFilePatch) -> char {
    if patch.old_path.is_some() {
        'R'
    } else if patch.raw.contains("\nnew file mode ") {
        'A'
    } else if patch.raw.contains("\ndeleted file mode ") {
        'D'
    } else {
        'M'
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
        match &patch.old_path {
            Some(old) => println!("R  {old} -> {}{binary}", patch.path),
            None => println!("{}  {}{binary}", patch_letter(patch), patch.path),
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

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = Config::from(cli);
    run(&config)
}
