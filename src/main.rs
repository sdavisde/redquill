//! CLI entry point: argument parsing and wiring for redquill.
//!
//! Owns the working-tree default, `--staged`, ref-range, and `-o file`
//! flags described in the README, and wires parsed args into the rest of
//! the crate. No behavior beyond argument parsing lives here yet.

use std::path::PathBuf;

use clap::Parser;

use redquill::diff;
use redquill::git::{ChangeKind, DiffTarget, GitRunner};

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

/// Prints a single aggregate summary line for the resolved diff target,
/// built from the parsed diff model rather than a per-file listing.
fn run(config: &Config) -> anyhow::Result<()> {
    let runner = GitRunner::discover()?;
    let target = config.diff_target();
    let patches = runner.diff(&target)?;

    // FR-diff-wire-1: parse the raw patches into the typed diff model and
    // derive aggregate counts from it — this replaces the Task-2 per-file
    // placeholder loop.
    let files = diff::parse_patches(&patches);
    let mut summary = diff::summarize(&files);

    // `git diff` never surfaces untracked files (see `git/` contract), so a
    // working-tree summary would otherwise silently omit them. They carry no
    // diff body (no hunks, no added/removed lines), so folding their count
    // into `files` — rather than preserving a separate per-file `?` listing —
    // is the summary-shaped choice: the operator still sees a complete file
    // count, without reintroducing the per-file loop this task removes.
    if matches!(target, DiffTarget::WorkingTree) {
        let untracked = runner
            .status()?
            .into_iter()
            .filter(|entry| entry.kind == ChangeKind::Untracked)
            .count();
        summary.files += untracked;
    }

    // FR-diff-wire-1: single summary line, e.g. `3 files, 7 hunks, +42 -18`.
    println!(
        "{} files, {} hunks, +{} -{}",
        summary.files, summary.hunks, summary.added, summary.removed
    );
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = Config::from(cli);
    run(&config)
}
