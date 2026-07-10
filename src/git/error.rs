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
}
