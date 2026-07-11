//! Runs `git` commands on PATH and parses porcelain/diff output into typed
//! structs. Owns all interaction with the git CLI, respecting the user's
//! git config. No TUI types leak in here.
//!
//! - [`GitRunner`] discovers the repo root and runs commands against it.
//! - [`status`] parses `git status --porcelain=v2 --branch -z` into
//!   [`FileStatus`]es (and, via [`StatusSnapshot`], alongside [`BranchStatus`]).
//! - [`branch`] parses the `# branch.*` header fields of that same payload
//!   into [`BranchStatus`] (name/short-oid, upstream, ahead/behind).
//! - [`commit`] parses `git log -1 --format=<COMMIT_SUMMARY_FORMAT>` into a
//!   [`CommitSummary`] (abbreviated hash + subject) for the tip commit.
//! - [`stash`] parses `git stash list --format=<STASH_LIST_FORMAT>` into
//!   [`StashEntry`] records.
//! - [`diff`] splits `git diff` output into raw per-file [`RawFilePatch`]es
//!   (no hunk parsing — see [`crate::diff::parse_hunks`] for that).
//! - [`remote`] builds the three sanctioned remote operations (fetch / pull /
//!   push) as fixed argument vectors with `GIT_TERMINAL_PROMPT=0`, never
//!   `--force` — the write/network ceiling.
//! - [`stage`] adds index staging: file-level (`stage_file`/`unstage_file`)
//!   and hunk/line-level via synthetic patches applied with `--cached`.

mod branch;
mod commit;
mod diff;
mod error;
mod remote;
mod runner;
mod stage;
mod stash;
mod status;

pub use branch::{BranchStatus, parse_branch_headers};
pub use commit::{COMMIT_SUMMARY_FORMAT, CommitSummary, parse_commit_summary};
pub use diff::{DiffTarget, RawFilePatch, split_patches};
pub use error::GitError;
pub use remote::{RemoteOp, remote_command};
pub use runner::GitRunner;
pub use stage::{build_hunk_patch, build_line_patch};
pub use stash::{STASH_LIST_FORMAT, StashEntry, parse_stash_list};
pub use status::{
    ChangeKind, FileStatus, StatusCode, StatusSnapshot, parse_porcelain_v2, parse_porcelain_v2_full,
};
