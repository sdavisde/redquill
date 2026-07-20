//! Runs `git` commands on PATH and parses porcelain/diff output into typed
//! structs. Owns all interaction with the git CLI, respecting the user's
//! git config. No TUI types leak in here.
//!
//! - [`GitRunner`] discovers the repo root and runs commands against it.
//! - [`status`] parses `git status --porcelain=v2 --branch -z` into
//!   [`FileStatus`]es (and, via [`StatusSnapshot`], alongside [`BranchStatus`]).
//! - [`branch`] parses the `# branch.*` header fields of that same payload
//!   into [`BranchStatus`] (name/short-oid, upstream, ahead/behind), and
//!   `git for-each-ref refs/heads` into the full [`LocalBranch`] list.
//! - [`commit`] parses `git log -1 --format=<COMMIT_SUMMARY_FORMAT>` into a
//!   [`CommitSummary`] (abbreviated hash + subject) for the tip commit, and
//!   builds the write command (`commit -m <message>`, the message verbatim
//!   as one argv element, `GIT_TERMINAL_PROMPT=0`, never any flag beyond
//!   `-m`).
//! - [`stash`] parses `git stash list --format=<STASH_LIST_FORMAT>` into
//!   [`StashEntry`] records.
//! - [`worktree`] parses `git worktree list --porcelain` into
//!   [`WorktreeEntry`] records, and provides
//!   [`sanitize_branch_dir_name`] — a branch name to collision-safe
//!   worktree directory name mapping.
//! - [`diff`] splits `git diff` output into raw per-file [`RawFilePatch`]es
//!   (no hunk parsing — see [`crate::diff::parse_hunks`] for that); see
//!   `diff.rs` for the [`DiffTarget`] variants, including the branch-review
//!   [`DiffTarget::Review`] target.
//! - [`log`] parses `git log --format=<log::COMMIT_LOG_FORMAT>` into
//!   [`CommitLogEntry`] records (full/short SHA, subject, author, timestamp)
//!   for the git panel's commit-history read model; pagination (count/skip)
//!   is a parameter of [`GitRunner::commit_log`], not of the parser.
//! - [`remote`] builds the three sanctioned remote operations (fetch / pull /
//!   push) as fixed argument vectors with `GIT_TERMINAL_PROMPT=0`, never
//!   `--force` — the write/network ceiling. Also owns [`PrRef`]: the closed
//!   type behind PR/MR head-ref fetch, whose one permitted forced refspec
//!   (and managed-branch delete) is structurally confined to the
//!   `redquill/pr/` namespace.
//! - [`stage`] adds index staging: file-level (`stage_file`/`unstage_file`)
//!   and hunk/line-level via synthetic patches applied with `--cached`.
//! - [`ls_files`] parses `git ls-files -z` (tracked) and `git ls-files -z
//!   --others --exclude-standard` (untracked-but-unignored) into
//!   repo-relative path lists — the fuzzy file finder's candidate source;
//!   [`crate::search::files`] merges the two lists.

mod branch;
mod commit;
mod diff;
mod error;
mod log;
mod ls_files;
mod remote;
mod runner;
mod stage;
mod stash;
mod status;
mod worktree;

pub use branch::{
    BRANCH_LIST_FORMAT, BranchStatus, LocalBranch, parse_branch_headers, parse_branch_list,
};
pub use commit::{
    COMMIT_SUMMARY_FORMAT, CommitSummary, commit_command, commit_command_line, parse_commit_summary,
};
pub use diff::{DiffTarget, RawFilePatch, StagingMode, split_patches};
pub use error::GitError;
pub use log::{COMMIT_LOG_FORMAT, CommitLogEntry, CommitLogRange, parse_commit_log};
pub use ls_files::parse_ls_files_z;
pub use remote::{
    MANAGED_PR_BRANCH_PREFIX, PrRef, PrRefKind, RemoteOp, base_fetch_command,
    delete_managed_pr_branch_command, pr_fetch_command, pr_peek_fetch_command, remote_command,
};
pub use runner::GitRunner;
pub use stage::{build_hunk_patch, build_line_patch};
pub use stash::{STASH_LIST_FORMAT, StashEntry, parse_stash_list};
pub use status::{
    ChangeKind, FileStatus, StatusCode, StatusSnapshot, parse_porcelain_v2, parse_porcelain_v2_full,
};
pub use worktree::{WorktreeEntry, parse_worktree_list, sanitize_branch_dir_name};
