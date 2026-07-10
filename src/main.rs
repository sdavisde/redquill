//! CLI entry point: argument parsing and wiring for redquill.
//!
//! Owns the working-tree default, `--staged`, ref-range, and `-o file`
//! flags described in the README, and wires parsed args into the rest of
//! the crate. No behavior beyond argument parsing lives here yet.

mod annotate;
mod diff;
mod git;
mod lsp;
mod ui;

use std::path::PathBuf;

use clap::Parser;

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
///
/// Fields are only read via the derived `Debug` impl for now, which rustc's
/// dead-code analysis doesn't count as a use; `allow` is scoped to this
/// skeleton struct until the fields are consumed by real logic.
#[derive(Debug)]
#[allow(dead_code)]
struct Config {
    /// Ref range to diff, if any; `None` means the working tree.
    range: Option<String>,
    /// Whether to review the staged index instead of the working tree.
    staged: bool,
    /// Optional file to additionally write annotations to.
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

fn main() {
    let cli = Cli::parse();
    let config = Config::from(cli);

    println!("{config:#?}");
    println!("redquill: not implemented yet");
}
