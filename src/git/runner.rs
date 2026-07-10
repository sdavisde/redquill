//! [`GitRunner`]: shells out to `git` on `PATH` against a discovered repo root.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::diff::{DiffTarget, RawFilePatch, split_patches};
use super::error::GitError;
use super::status::{FileStatus, parse_porcelain_v2};

/// Runs `git` commands against a single repository working tree.
///
/// Construct via [`GitRunner::discover`] (or [`GitRunner::discover_in`] for a
/// specific starting directory). All subsequent commands run with the repo root
/// as their working directory, so the user's git config is respected exactly as
/// `git` on their `PATH` would apply it.
#[derive(Debug, Clone)]
pub struct GitRunner {
    root: PathBuf,
}

impl GitRunner {
    /// Discovers the repository containing the current working directory.
    pub fn discover() -> Result<Self, GitError> {
        let cwd = std::env::current_dir().map_err(GitError::Spawn)?;
        Self::discover_in(cwd)
    }

    /// Discovers the repository containing `start`.
    pub fn discover_in(start: impl AsRef<Path>) -> Result<Self, GitError> {
        let output = Command::new("git")
            .current_dir(start.as_ref())
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .map_err(map_spawn_err)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(GitError::NotARepo(stderr));
        }

        let root = String::from_utf8(output.stdout)
            .map_err(GitError::Utf8)?
            .trim()
            .to_string();
        Ok(GitRunner {
            root: PathBuf::from(root),
        })
    }

    /// The absolute path to the repository root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Runs a git subcommand at the repo root, returning raw stdout bytes.
    fn run_raw(&self, args: &[&str]) -> Result<Vec<u8>, GitError> {
        let output = Command::new("git")
            .current_dir(&self.root)
            .args(args)
            .output()
            .map_err(map_spawn_err)?;

        if !output.status.success() {
            return Err(GitError::Command {
                command: args.join(" "),
                code: output
                    .status
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "signal".to_string()),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
        Ok(output.stdout)
    }

    /// Runs a git subcommand and decodes stdout as UTF-8.
    fn run_utf8(&self, args: &[&str]) -> Result<String, GitError> {
        String::from_utf8(self.run_raw(args)?).map_err(GitError::Utf8)
    }

    /// Returns the parsed working-tree/index status for every changed path.
    pub fn status(&self) -> Result<Vec<FileStatus>, GitError> {
        let out = self.run_utf8(&["status", "--porcelain=v2", "-z"])?;
        parse_porcelain_v2(&out)
    }

    /// Returns raw per-file patches for the given diff target.
    ///
    /// Rename detection (`-M`) is enabled and color/external-diff drivers are
    /// disabled so the output shape is stable regardless of user config.
    pub fn diff(&self, target: &DiffTarget) -> Result<Vec<RawFilePatch>, GitError> {
        let mut args = vec!["diff", "--no-color", "--no-ext-diff", "-M"];
        match target {
            DiffTarget::WorkingTree => {}
            DiffTarget::Staged => args.push("--staged"),
            DiffTarget::Range(range) => args.push(range.as_str()),
        }
        let out = self.run_utf8(&args)?;
        Ok(split_patches(&out))
    }
}

/// Maps a spawn `io::Error` to a `GitNotFound` when git is absent, else `Spawn`.
fn map_spawn_err(e: std::io::Error) -> GitError {
    if e.kind() == std::io::ErrorKind::NotFound {
        GitError::GitNotFound
    } else {
        GitError::Spawn(e)
    }
}
