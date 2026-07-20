//! GitLab provider: `glab` argv construction and JSON parsing for MR
//! listing and MR detail (`diff_refs`). Mirrors `github.rs`'s shape (fixed
//! argv, fixture-tested pure parsers, a thin spawn-and-parse wrapper left
//! deliberately untested) but follows `glab`'s own CLI conventions where
//! they differ from `gh`'s:
//!
//! - `gh pr list --json <fields>` selects fields; `glab` has no field
//!   selector — `-F json` (`--output json`) dumps the full MR resource per
//!   row, so [`RawMr`] just declares the subset this model needs and lets
//!   serde ignore the rest, same as [`super::github::RawPr`] does structurally
//!   even though the two commands differ.
//! - MR detail goes through `glab api projects/:id/merge_requests/<iid>`
//!   rather than `mr view -F json` porcelain, because `api` returns the
//!   GitLab REST response verbatim — the only shape confirmed to carry
//!   `diff_refs`. `:id` is `glab api`'s own placeholder for the current
//!   repo's project id (mirroring `gh api`'s `{owner}/{repo}` substitution
//!   in `github.rs`), so the MR number (`u64` end-to-end) stays the only
//!   variable part of the path — no argv is ever string-assembled from
//!   caller input.
//!
//! **Unverified against a real `glab`**: this machine has no `glab` on
//! `PATH`, so every argv shape and field name here is built from documented
//! GitLab API / `glab` CLI behavior, not confirmed by a live run. The pure
//! parsers are what carry the test coverage (fixture JSON matching the
//! documented shapes); the thin spawn-and-parse functions are exercised only
//! by user dogfood, same as `github.rs`'s equivalents.

use std::process::Command;
use std::time::Duration;

use serde::Deserialize;

use super::process::{harden_glab, run_captured_with_timeout};
use super::{ForgeError, PullRequest};

/// How long a `glab` read invocation (list, detail) may run before it's
/// treated as failed and killed. Same budget `github.rs` uses for its
/// network reads.
const READ_TIMEOUT: Duration = Duration::from_secs(15);

// -- MR listing ---------------------------------------------------------------

/// Builds the fixed argv for `glab mr list -F json`. `mr list` defaults to
/// open MRs (`--state opened`), matching the "open PRs" scope this listing
/// wants, so no explicit `-s`/`--state` flag is added. `--per-page 100`
/// raises the page size since, unlike `gh pr list --json`, `glab mr list`
/// has no documented automatic multi-page JSON fetch.
pub fn mr_list_command() -> Command {
    let mut cmd = Command::new("glab");
    cmd.args(["mr", "list", "-F", "json", "--per-page", "100"]);
    harden_glab(&mut cmd);
    cmd
}

/// Runs `glab mr list -F json` and returns the typed rows. The only listing
/// function here that actually spawns a process — kept thin and
/// deliberately untested, same as [`super::github::list_open_prs`].
pub fn list_open_mrs() -> Result<Vec<PullRequest>, ForgeError> {
    let mut cmd = mr_list_command();
    let output = run_captured_with_timeout(&mut cmd, READ_TIMEOUT).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            ForgeError::CliNotFound { cli: "glab" }
        } else {
            ForgeError::Spawn {
                cli: "glab",
                source,
            }
        }
    })?;

    if !output.status.success() {
        return Err(ForgeError::Command {
            cli: "glab",
            command: "mr list".to_string(),
            code: output
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".to_string()),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }

    let json = String::from_utf8_lossy(&output.stdout);
    parse_mr_list_json(&json)
}

/// The raw shape of one entry in `glab mr list -F json`'s JSON array —
/// GitLab's full `MergeRequest` resource, trimmed to the fields this model
/// needs (serde ignores the rest by default, same convention
/// [`super::github::RawPr`] and [`super::threads::RawComment`] follow).
#[derive(Debug, Deserialize)]
struct RawMr {
    iid: u64,
    title: String,
    author: RawAuthor,
    source_branch: String,
    target_branch: String,
    /// GitLab's draft flag has gone by two field names across API versions
    /// (`draft` is the current one, `work_in_progress` the older alias for
    /// the same boolean) — both are read and combined so listing tolerates
    /// either server vintage rather than committing to one unverified name.
    #[serde(default)]
    draft: Option<bool>,
    #[serde(default)]
    work_in_progress: Option<bool>,
    updated_at: String,
}

#[derive(Debug, Deserialize)]
struct RawAuthor {
    username: String,
}

/// Parses `glab mr list -F json`'s stdout into the same typed [`PullRequest`]
/// rows `github.rs` produces: `number` <- `iid`, `author` <- `author.username`,
/// `head_ref`/`base_ref` <- `source_branch`/`target_branch`. Pure — no
/// process involved — so it's exercised entirely by fixture tests.
pub fn parse_mr_list_json(json: &str) -> Result<Vec<PullRequest>, ForgeError> {
    let raw: Vec<RawMr> = serde_json::from_str(json).map_err(|e| ForgeError::Parse {
        cli: "glab",
        message: e.to_string(),
    })?;
    Ok(raw
        .into_iter()
        .map(|r| PullRequest {
            number: r.iid,
            title: r.title,
            author: r.author.username,
            head_ref: r.source_branch,
            base_ref: r.target_branch,
            is_draft: r.draft.or(r.work_in_progress).unwrap_or(false),
            updated_at: r.updated_at,
        })
        .collect())
}

// -- MR detail (diff_refs) -----------------------------------------------------

/// `base_sha`/`start_sha`/`head_sha` for an MR's current diff — the three
/// SHAs GitLab positions notes and draft notes against. An MR's diff shifts
/// as new commits land, so a note's line position is only meaningful when
/// pinned to the `diff_refs` that were current when it was written.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DiffRefs {
    pub base_sha: String,
    pub start_sha: String,
    pub head_sha: String,
}

/// One MR's detail, trimmed to what checkout/position-hash construction
/// needs today.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MrDetail {
    pub number: u64,
    pub diff_refs: DiffRefs,
}

#[derive(Debug, Deserialize)]
struct RawMrDetail {
    iid: u64,
    diff_refs: DiffRefs,
}

/// Builds the fixed argv for `glab api projects/:id/merge_requests/<iid>`.
/// See the module doc for why `api` (not `mr view -F json`) was chosen.
pub fn mr_detail_command(iid: u64) -> Command {
    let mut cmd = Command::new("glab");
    cmd.args(["api", &format!("projects/:id/merge_requests/{iid}")]);
    harden_glab(&mut cmd);
    cmd
}

/// Parses `glab api projects/:id/merge_requests/<iid>`'s stdout into
/// [`MrDetail`]. Pure — fixture-tested.
pub fn parse_mr_detail_json(json: &str) -> Result<MrDetail, ForgeError> {
    let raw: RawMrDetail = serde_json::from_str(json).map_err(|e| ForgeError::Parse {
        cli: "glab",
        message: e.to_string(),
    })?;
    Ok(MrDetail {
        number: raw.iid,
        diff_refs: raw.diff_refs,
    })
}

/// Runs the MR detail fetch and returns the typed result. The only detail
/// function here that spawns a process — kept thin and deliberately
/// untested, same as [`list_open_mrs`].
pub fn mr_detail(iid: u64) -> Result<MrDetail, ForgeError> {
    let mut cmd = mr_detail_command(iid);
    let output = run_captured_with_timeout(&mut cmd, READ_TIMEOUT).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            ForgeError::CliNotFound { cli: "glab" }
        } else {
            ForgeError::Spawn {
                cli: "glab",
                source,
            }
        }
    })?;

    if !output.status.success() {
        return Err(ForgeError::Command {
            cli: "glab",
            command: format!("api projects/:id/merge_requests/{iid}"),
            code: output
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".to_string()),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }

    let json = String::from_utf8_lossy(&output.stdout);
    parse_mr_detail_json(&json)
}

#[cfg(test)]
#[path = "gitlab_tests.rs"]
mod tests;
