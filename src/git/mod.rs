//! Runs `git` commands on PATH and parses porcelain/diff output into typed
//! structs. Owns all interaction with the git CLI, respecting the user's
//! git config. No TUI types leak in here.
//!
//! - [`GitRunner`] discovers the repo root and runs commands against it.
//! - [`status`] parses `git status --porcelain=v2 -z` into [`FileStatus`].
//! - [`diff`] splits `git diff` output into raw per-file [`RawFilePatch`]es
//!   (no hunk parsing — that belongs to a later diff-model task).

mod diff;
mod error;
mod runner;
mod status;

pub use diff::{DiffTarget, RawFilePatch, split_patches};
pub use error::GitError;
pub use runner::GitRunner;
pub use status::{ChangeKind, FileStatus, StatusCode, parse_porcelain_v2};
