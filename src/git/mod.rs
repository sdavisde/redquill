//! Runs `git` commands on PATH and parses porcelain/diff output into typed
//! structs. Owns all interaction with the git CLI, respecting the user's
//! git config. No TUI types leak in here.
//!
//! - [`GitRunner`] discovers the repo root and runs commands against it.
//! - [`status`] parses `git status --porcelain=v2 -z` into [`FileStatus`].
//! - [`diff`] splits `git diff` output into raw per-file [`RawFilePatch`]es
//!   (no hunk parsing — see [`crate::diff::parse_hunks`] for that).
//! - [`stage`] adds index staging: file-level (`stage_file`/`unstage_file`)
//!   and hunk/line-level via synthetic patches applied with `--cached`.

mod diff;
mod error;
mod runner;
mod stage;
mod status;

pub use diff::{DiffTarget, RawFilePatch, split_patches};
pub use error::GitError;
pub use runner::GitRunner;
pub use stage::{build_hunk_patch, build_line_patch};
pub use status::{ChangeKind, FileStatus, StatusCode, parse_porcelain_v2};
