//! Error types for the git module.

use thiserror::Error;

/// Errors produced while running or parsing `git`.
#[derive(Debug, Error)]
pub enum GitError {
    /// The `git` executable could not be found on `PATH`.
    #[error("git executable not found on PATH")]
    GitNotFound,

    /// Spawning `git` failed for a reason other than it being missing.
    #[error("failed to run git: {0}")]
    Spawn(#[source] std::io::Error),

    /// A `git` invocation exited with a non-zero status.
    #[error("git {command} exited with status {code}: {stderr}")]
    Command {
        /// The git subcommand and arguments that were run.
        command: String,
        /// The exit code (or `signal` if terminated by a signal).
        code: String,
        /// Captured stderr, trimmed of trailing whitespace.
        stderr: String,
    },

    /// The starting directory is not inside a git working tree.
    #[error("not a git repository: {0}")]
    NotARepo(String),

    /// `git` produced output that was not valid UTF-8.
    #[error("git produced non-UTF-8 output")]
    Utf8(#[source] std::string::FromUtf8Error),

    /// `git` output did not match the expected porcelain format.
    #[error("failed to parse git output: {0}")]
    Parse(String),

    /// A filesystem operation needed to resolve a git path (e.g.
    /// canonicalizing `--git-common-dir`'s output) failed.
    #[error("filesystem error: {0}")]
    Io(#[source] std::io::Error),

    /// No default base ref could be resolved for `--review`: `origin/HEAD`
    /// is unset, and neither a local `main` nor `master` branch exists.
    /// Named in `Display` so the CLI's own error message tells the user
    /// exactly which flag closes the gap.
    #[error(
        "could not resolve a default base branch (tried origin/HEAD, main, master); pass --base <ref>"
    )]
    NoDefaultBase,
}

/// Maps a spawn `io::Error` to a `GitNotFound` when git is absent, else `Spawn`.
pub(crate) fn map_spawn_err(e: std::io::Error) -> GitError {
    if e.kind() == std::io::ErrorKind::NotFound {
        GitError::GitNotFound
    } else {
        GitError::Spawn(e)
    }
}

/// Builds a [`GitError::Command`] from a non-zero exit status: joins `args`
/// into the reported command string, maps a missing exit code (signal
/// termination) to `"signal"`, and trims `stderr`.
pub(crate) fn command_error(
    args: &[&str],
    status: &std::process::ExitStatus,
    stderr: &[u8],
) -> GitError {
    GitError::Command {
        command: args.join(" "),
        code: status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "signal".to_string()),
        stderr: String::from_utf8_lossy(stderr).trim().to_string(),
    }
}
