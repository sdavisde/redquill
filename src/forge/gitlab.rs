//! GitLab provider: `glab` argv construction and JSON parsing for MR
//! listing, MR detail (`diff_refs`), and discussion-thread import. Mirrors
//! `github.rs`'s shape (fixed argv, fixture-tested pure parsers, a thin
//! spawn-and-parse wrapper left deliberately untested) but follows `glab`'s
//! own CLI conventions where they differ from `gh`'s:
//!
//! - `gh pr list --json <fields>` selects fields; `glab` has no field
//!   selector — `-F json` (`--output json`) dumps the full MR resource per
//!   row, so [`RawMr`] just declares the subset this model needs and lets
//!   serde ignore the rest, same as [`super::github::RawPr`] does structurally
//!   even though the two commands differ.
//! - MR detail and the discussions import both go through `glab api`
//!   (`projects/:id/merge_requests/<iid>[...]`) rather than `mr view`
//!   porcelain, because `api` returns the GitLab REST response verbatim —
//!   the only shape confirmed to carry `diff_refs`. `:id` is `glab api`'s
//!   own placeholder for the current repo's project id (mirroring `gh
//!   api`'s `{owner}/{repo}` substitution in `github.rs`), so the MR number
//!   (`u64` end-to-end) stays the only variable part of the path — no argv
//!   is ever string-assembled from caller input.
//!
//! **Unverified against a real `glab`**: this machine has no `glab` on
//! `PATH`, so every argv shape and field name here is built from documented
//! GitLab API / `glab` CLI behavior, not confirmed by a live run. The pure
//! parsers are what carry the test coverage (fixture JSON matching the
//! documented shapes); the thin spawn-and-parse functions are exercised only
//! by user dogfood, same as `github.rs`'s equivalents.
//!
//! **Discussion-thread id caveat**: [`super::threads::Thread::id`] is `u64`
//! (GitHub's comment-id convention), so a discussion's root note id fills
//! that slot here. GitLab's own reply target, though, is the *discussion*
//! id — a string hash, not a note id — so a later unit that drafts replies
//! against an imported GitLab thread will need that string id from
//! somewhere other than `Thread::id`. Import (this module's scope) only
//! needs to render the conversation, not reply to it, so that gap is left
//! for whichever unit wires GitLab replies.
//!
//! **Position-mapping table** ([`anchor_for`]): a diff note's `position`
//! carries `new_line`/`old_line`, each present or absent depending on what
//! kind of diff line the note is attached to.
//!
//! | position_type | new_line | old_line | anchor            |
//! |----------------|----------|----------|-------------------|
//! | `"file"`       | —        | —        | `File` (new/old path) |
//! | other/absent   | present  | absent   | `Position` on `New`, added line |
//! | other/absent   | absent   | present  | `Position` on `Old`, removed line |
//! | other/absent   | present  | present  | `Position` on `New` (context line — consistent with the diff model anchoring context on the new side) |
//! | other/absent   | absent   | absent   | `File` (outdated/unmappable, falls back rather than dropping) |
//!
//! **Non-diff discussions**: a general MR-level comment (`individual_note:
//! true`) has no file to attach to at all — [`super::threads::ThreadAnchor`]
//! always carries a path — so these are skipped entirely rather than
//! inventing a placeholder path. Only diff-anchored discussions
//! (`individual_note: false`) are imported.

use std::process::Command;
use std::time::Duration;

use serde::Deserialize;

use crate::annotate::Side;

use super::process::{harden_glab, run_captured_with_timeout};
use super::threads::{Thread, ThreadAnchor, ThreadComment};
use super::{ForgeError, PullRequest};

/// How long a `glab` read invocation (list, detail, discussions) may run
/// before it's treated as failed and killed. Same budget `github.rs` uses
/// for its network reads.
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

// -- Discussion-thread import ---------------------------------------------------

/// Builds the fixed argv for `glab api
/// projects/:id/merge_requests/<iid>/discussions`, mirroring
/// [`mr_detail_command`]'s `:id` placeholder use. `--paginate` follows every
/// page, matching `github.rs`'s `review_comments_command` — unverified
/// locally that `glab api` supports `--paginate` identically to `gh api`,
/// but it is `glab`'s own documented flag for the same purpose.
pub fn discussions_command(iid: u64) -> Command {
    let mut cmd = Command::new("glab");
    cmd.args([
        "api",
        &format!("projects/:id/merge_requests/{iid}/discussions"),
        "--paginate",
    ]);
    harden_glab(&mut cmd);
    cmd
}

/// Runs the discussions fetch and returns ordered threads. The only import
/// function here that spawns a process; [`parse_discussions_json`] carries
/// the fixture coverage.
pub fn fetch_discussions(iid: u64) -> Result<Vec<Thread>, ForgeError> {
    let mut cmd = discussions_command(iid);
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
            command: format!("api projects/:id/merge_requests/{iid}/discussions"),
            code: output
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".to_string()),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }

    let json = String::from_utf8_lossy(&output.stdout);
    parse_discussions_json(&json)
}

/// The raw shape of one entry in the discussions JSON array.
#[derive(Debug, Deserialize)]
struct RawDiscussion {
    #[serde(default)]
    individual_note: bool,
    #[serde(default)]
    notes: Vec<RawNote>,
}

#[derive(Debug, Deserialize)]
struct RawNote {
    id: u64,
    body: String,
    author: RawAuthor,
    created_at: String,
    #[serde(default)]
    resolved: bool,
    #[serde(default)]
    position: Option<RawPosition>,
}

#[derive(Debug, Deserialize)]
struct RawPosition {
    #[serde(default)]
    position_type: Option<String>,
    #[serde(default)]
    new_line: Option<u32>,
    #[serde(default)]
    old_line: Option<u32>,
    #[serde(default)]
    new_path: Option<String>,
    #[serde(default)]
    old_path: Option<String>,
}

/// Parses the discussions JSON array into ordered [`Thread`]s. Pure — no
/// process involved — so it's exercised entirely by fixture tests. See the
/// module doc's position-mapping table and non-diff-discussion note for the
/// exact rules applied here.
pub fn parse_discussions_json(json: &str) -> Result<Vec<Thread>, ForgeError> {
    let raw: Vec<RawDiscussion> = serde_json::from_str(json).map_err(|e| ForgeError::Parse {
        cli: "glab",
        message: e.to_string(),
    })?;
    Ok(raw.into_iter().filter_map(build_thread).collect())
}

/// Builds one imported [`Thread`] from a discussion, or `None` when it can't
/// be represented at all: a general (non-diff) comment (`individual_note:
/// true`) has no file to anchor to, and an empty `notes` array or a root
/// note with no path anywhere in its position leaves nothing to anchor
/// to either — skipped rather than invented a placeholder, per the module
/// doc.
fn build_thread(raw: RawDiscussion) -> Option<Thread> {
    if raw.individual_note {
        return None;
    }
    let mut notes = raw.notes.into_iter();
    let root_raw = notes.next()?;
    let anchor = anchor_for(&root_raw)?;
    let outdated = matches!(anchor, ThreadAnchor::File { .. });
    let resolved = root_raw.resolved;
    let id = root_raw.id;
    let root = to_thread_comment(&root_raw);
    let replies: Vec<ThreadComment> = notes.map(|n| to_thread_comment(&n)).collect();
    Some(Thread {
        id,
        anchor,
        root,
        replies,
        resolved,
        outdated,
    })
}

/// Maps one note's `position` onto a [`ThreadAnchor`] per the module doc's
/// table. Returns `None` only when there is truly no path to anchor to at
/// all (no `position`, or a `position` with neither `new_path` nor
/// `old_path`) — every other case, including an outdated/unmappable line,
/// falls back to a file-level anchor rather than dropping the thread.
fn anchor_for(root: &RawNote) -> Option<ThreadAnchor> {
    let position = root.position.as_ref()?;

    if position.position_type.as_deref() == Some("file") {
        let path = position
            .new_path
            .clone()
            .or_else(|| position.old_path.clone())?;
        return Some(ThreadAnchor::File { path });
    }

    match (position.new_line, position.old_line) {
        // Added line, or context (both sides present): anchor on the new
        // side, consistent with how the diff model anchors context lines.
        (Some(new_line), _) => {
            let path = position.new_path.clone()?;
            Some(ThreadAnchor::Position {
                path,
                side: Side::New,
                line: new_line,
            })
        }
        // Removed line.
        (None, Some(old_line)) => {
            let path = position.old_path.clone()?;
            Some(ThreadAnchor::Position {
                path,
                side: Side::Old,
                line: old_line,
            })
        }
        // Neither line present: the position no longer maps (outdated) —
        // file-level fallback rather than dropped.
        (None, None) => {
            let path = position
                .new_path
                .clone()
                .or_else(|| position.old_path.clone())?;
            Some(ThreadAnchor::File { path })
        }
    }
}

fn to_thread_comment(raw: &RawNote) -> ThreadComment {
    ThreadComment {
        id: raw.id,
        author: raw.author.username.clone(),
        created_at: raw.created_at.clone(),
        body: raw.body.clone(),
    }
}

#[cfg(test)]
#[path = "gitlab_tests.rs"]
mod tests;
