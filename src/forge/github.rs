//! GitHub provider: `gh` argv construction and JSON parsing for the PR
//! listing. `gh pr list` infers the repository from the current working
//! directory exactly as `git` itself does elsewhere in this codebase, so no
//! repo argument is ever built from user input — the argv is entirely
//! fixed.

use std::collections::HashMap;
use std::process::Command;
use std::time::Duration;

use serde::Deserialize;

use super::process::{harden, run_captured_with_timeout};
use super::threads::{
    Thread, apply_resolved_states, parse_resolved_thread_states, parse_review_comments_json,
};
use super::{ForgeError, PullRequest};

/// The exact `--json` field list `gh pr list` is asked for — fixed at the
/// listing's field set, never composed from caller input.
pub const PR_LIST_JSON_FIELDS: &str =
    "number,title,author,headRefName,baseRefName,isDraft,updatedAt";

/// How long a `gh pr list` invocation may run before it's treated as
/// failed and killed. Generous relative to the credential-check timeout
/// since this is a real network round trip, not a local auth-store read.
const LIST_TIMEOUT: Duration = Duration::from_secs(15);

/// Builds the fixed argv for `gh pr list --json <fields>`, with prompts
/// disabled and color stripped from the (JSON, machine-read) output.
pub fn pr_list_command() -> Command {
    let mut cmd = Command::new("gh");
    cmd.args(["pr", "list", "--json", PR_LIST_JSON_FIELDS]);
    harden(&mut cmd);
    cmd
}

/// Runs `gh pr list` and returns the typed rows. The only function here
/// that actually spawns a process — kept thin and deliberately untested;
/// [`parse_pr_list_json`] carries the fixture coverage, since exercising
/// this would require a real `gh` on PATH.
pub fn list_open_prs() -> Result<Vec<PullRequest>, ForgeError> {
    let mut cmd = pr_list_command();
    let output = run_captured_with_timeout(&mut cmd, LIST_TIMEOUT).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            ForgeError::CliNotFound { cli: "gh" }
        } else {
            ForgeError::Spawn { cli: "gh", source }
        }
    })?;

    if !output.status.success() {
        return Err(ForgeError::Command {
            cli: "gh",
            command: "pr list".to_string(),
            code: output
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".to_string()),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }

    let json = String::from_utf8_lossy(&output.stdout);
    parse_pr_list_json(&json)
}

/// The raw shape of one entry in `gh pr list --json ...`'s JSON array.
#[derive(Debug, Deserialize)]
struct RawPr {
    number: u64,
    title: String,
    author: RawAuthor,
    #[serde(rename = "headRefName")]
    head_ref_name: String,
    #[serde(rename = "baseRefName")]
    base_ref_name: String,
    #[serde(rename = "isDraft")]
    is_draft: bool,
    #[serde(rename = "updatedAt")]
    updated_at: String,
}

#[derive(Debug, Deserialize)]
struct RawAuthor {
    login: String,
}

/// Parses `gh pr list --json ...`'s stdout into typed rows. Pure — no
/// process involved — so it's exercised entirely by fixture tests built
/// from captured-shape `gh` output.
pub fn parse_pr_list_json(json: &str) -> Result<Vec<PullRequest>, ForgeError> {
    let raw: Vec<RawPr> = serde_json::from_str(json).map_err(|e| ForgeError::Parse {
        cli: "gh",
        message: e.to_string(),
    })?;
    Ok(raw
        .into_iter()
        .map(|r| PullRequest {
            number: r.number,
            title: r.title,
            author: r.author.login,
            head_ref: r.head_ref_name,
            base_ref: r.base_ref_name,
            is_draft: r.is_draft,
            updated_at: r.updated_at,
        })
        .collect())
}

/// How long a `gh api` review-comments invocation may run before it's
/// treated as failed and killed. Same budget as the PR list: a real network
/// round trip, not a local read.
const REVIEW_COMMENTS_TIMEOUT: Duration = Duration::from_secs(15);

/// Builds the fixed argv for `gh api repos/{owner}/{repo}/pulls/<n>/comments`.
/// The `{owner}`/`{repo}` placeholders are substituted by `gh` itself from
/// the repository of the current working directory — the same "no repo
/// argument built from caller input" contract [`pr_list_command`] follows —
/// so the PR number, typed as `u64` end-to-end, is the only variable part
/// of the endpoint path. `--paginate` follows every page so a PR with more
/// than the default 30 comments fetches in full (the endpoint returns a JSON
/// array, which `gh` combines across pages into one array `serde` still
/// parses whole).
pub fn review_comments_command(number: u64) -> Command {
    let mut cmd = Command::new("gh");
    cmd.args([
        "api",
        &format!("repos/{{owner}}/{{repo}}/pulls/{number}/comments"),
        "--paginate",
    ]);
    harden(&mut cmd);
    cmd
}

/// The GraphQL query fetching each review thread's `isResolved` plus its root
/// comment's `databaseId` (the join key back onto a REST-built [`Thread`]).
/// Read-only; the REST endpoint has no resolution field, so this second read
/// is the only way to know a thread is resolved.
const REVIEW_THREADS_RESOLVED_QUERY: &str = "query($owner: String!, $name: String!, $number: Int!) { repository(owner: $owner, name: $name) { pullRequest(number: $number) { reviewThreads(first: 100) { nodes { isResolved comments(first: 1) { nodes { databaseId } } } } } } }";

/// Builds the fixed argv for the read-only `gh api graphql` resolution query.
/// `owner`/`name` come from the user's own `origin` slug and `number` is a
/// `u64` end-to-end; all three ride as typed GraphQL variables (`-f`/`-F`
/// fields), never interpolated into the query text, so there is no
/// string-assembled command line.
pub fn review_threads_resolved_command(owner: &str, repo: &str, number: u64) -> Command {
    let mut cmd = Command::new("gh");
    cmd.args([
        "api".to_string(),
        "graphql".to_string(),
        "-f".to_string(),
        format!("owner={owner}"),
        "-f".to_string(),
        format!("name={repo}"),
        "-F".to_string(),
        format!("number={number}"),
        "-f".to_string(),
        format!("query={REVIEW_THREADS_RESOLVED_QUERY}"),
    ]);
    harden(&mut cmd);
    cmd
}

/// Runs the review-comments fetch and returns ordered threads, with each
/// thread's resolution state overlaid best-effort from a second, read-only
/// GraphQL query. `repo_slug` (`owner/repo`, from the `origin` URL) drives
/// that overlay; a `None` slug, or any failure of the GraphQL read, simply
/// leaves every thread `resolved: false` — the review continues either way.
/// Like [`list_open_prs`], the only function here that spawns a process;
/// [`parse_review_comments_json`]/[`parse_resolved_thread_states`] carry the
/// fixture coverage.
pub fn fetch_review_threads(
    repo_slug: Option<&str>,
    number: u64,
) -> Result<Vec<Thread>, ForgeError> {
    let mut cmd = review_comments_command(number);
    let output =
        run_captured_with_timeout(&mut cmd, REVIEW_COMMENTS_TIMEOUT).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                ForgeError::CliNotFound { cli: "gh" }
            } else {
                ForgeError::Spawn { cli: "gh", source }
            }
        })?;

    if !output.status.success() {
        return Err(ForgeError::Command {
            cli: "gh",
            command: format!("api repos/{{owner}}/{{repo}}/pulls/{number}/comments"),
            code: output
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".to_string()),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }

    let json = String::from_utf8_lossy(&output.stdout);
    let mut threads = parse_review_comments_json(&json)?;
    if let Some((owner, repo)) = repo_slug.and_then(|s| s.rsplit_once('/'))
        && let Some(states) = fetch_resolved_states(owner, repo, number)
    {
        apply_resolved_states(&mut threads, &states);
    }
    Ok(threads)
}

/// Best-effort read of the resolution overlay: spawns the GraphQL query and
/// parses it, returning `None` on any failure (spawn error, non-zero exit,
/// unparseable output) so the caller keeps unresolved threads rather than
/// surfacing an error. Never spawned on the render loop (see
/// [`fetch_review_threads`]'s callers).
fn fetch_resolved_states(owner: &str, repo: &str, number: u64) -> Option<HashMap<u64, bool>> {
    let mut cmd = review_threads_resolved_command(owner, repo, number);
    let output = run_captured_with_timeout(&mut cmd, REVIEW_COMMENTS_TIMEOUT).ok()?;
    if !output.status.success() {
        return None;
    }
    let json = String::from_utf8_lossy(&output.stdout);
    parse_resolved_thread_states(&json).ok()
}

#[cfg(test)]
#[path = "github_tests.rs"]
mod tests;
