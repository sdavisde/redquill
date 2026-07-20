//! GitHub provider: `gh` argv construction and JSON parsing for the PR
//! listing. `gh pr list` infers the repository from the current working
//! directory exactly as `git` itself does elsewhere in this codebase, so no
//! repo argument is ever built from user input — the argv is entirely
//! fixed.

use std::collections::HashMap;
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::annotate::{Annotation, Classification, Side, Target};

use super::process::{harden, run_captured_with_timeout, run_with_input_and_timeout};
use super::submit::ForgeSubmitExecutor;
use super::threads::{
    Thread, apply_resolved_states, parse_resolved_thread_states, parse_review_comments_json,
};
use super::{ForgeError, PullRequest, Verdict};

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

// -- Review payload construction (submit flow) -------------------------------

/// GitHub's `side` values for a review comment: `Side::Old` is the diff's
/// left (removed) column, `Side::New` the right (added/context) column — the
/// inverse of `threads::parse_side`'s `"LEFT"`/`"RIGHT"` -> `Side` mapping.
fn side_str(side: Side) -> &'static str {
    match side {
        Side::Old => "LEFT",
        Side::New => "RIGHT",
    }
}

/// The verdict's `event` value on the reviews endpoint.
fn event_str(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Comment => "COMMENT",
        Verdict::Approve => "APPROVE",
        Verdict::RequestChanges => "REQUEST_CHANGES",
    }
}

/// Prefixes `body`'s first line with the annotation's classification tag
/// (`"[issue] "`, etc.), leaving every other line untouched — the same
/// convention `crate::annotate::markdown::render_one` uses for stdout, kept
/// in sync deliberately so a comment reads identically whether it lands on
/// the forge or in the stdout markdown.
fn classification_prefixed_body(classification: Classification, body: &str) -> String {
    let mut lines = body.lines();
    let mut out = String::new();
    if let Some(first) = lines.next() {
        out.push_str(&format!("[{}] ", classification.label()));
        out.push_str(first);
    }
    for line in lines {
        out.push('\n');
        out.push_str(line);
    }
    out
}

/// One entry in the reviews endpoint's `comments` array. `start_line`/
/// `start_side` are only present for a multi-line span (`Range`/`Hunk`); a
/// single-line comment (`Line`) carries only `line`/`side`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewCommentPayload {
    pub path: String,
    pub body: String,
    pub line: u32,
    pub side: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_side: Option<&'static str>,
}

/// The exact JSON body for one POST to
/// `/repos/{owner}/{repo}/pulls/{n}/reviews`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewPayload {
    pub body: String,
    pub event: &'static str,
    pub comments: Vec<ReviewCommentPayload>,
}

/// A file-target annotation the reviews endpoint's `comments` array cannot
/// carry (file-level positions aren't accepted there) and that must post
/// afterward via the single-comment endpoint with `subject_type: file`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCommentFollowUp {
    /// The originating annotation's store id, so the submit-sequence driver
    /// (a later unit) can mark it published individually on success.
    pub annotation_id: usize,
    pub path: String,
    pub body: String,
}

/// One review's full submission plan: the single reviews-endpoint payload
/// plus the file-target annotations that must post separately afterward.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewSubmissionPlan {
    pub payload: ReviewPayload,
    /// The store ids of the annotations that became `payload.comments`
    /// entries, in the same order — so the submit-sequence driver can mark
    /// each published once the (atomic) reviews-endpoint POST succeeds. The
    /// `comments` array itself carries no id (GitHub doesn't want one), so
    /// this parallel list is how a success is attributed back to the local
    /// annotations.
    pub comment_annotation_ids: Vec<usize>,
    pub file_comment_follow_ups: Vec<FileCommentFollowUp>,
}

/// Builds the exact submission plan for one review: `annotations` (expected
/// to already be the caller-filtered *unpublished* set — see
/// `crate::annotate::AnnotationStore::unpublished`) become either an entry in
/// the reviews-endpoint `comments` array (`Line`/`Range`/`Hunk` targets) or a
/// follow-up file comment (`Target::File`); `Target::WorktreeLine`/
/// `Target::WorktreeRange` targets anchor to local worktree content with no
/// forge-side position and are excluded from both — local-only, never
/// publishable. Pure — no network, no process — so it's exhaustively
/// fixture-tested.
pub fn build_review_payload(
    annotations: &[Annotation],
    verdict: Verdict,
    summary: Option<&str>,
) -> ReviewSubmissionPlan {
    let mut comments = Vec::new();
    let mut comment_annotation_ids = Vec::new();
    let mut file_comment_follow_ups = Vec::new();

    for annotation in annotations {
        let body = classification_prefixed_body(annotation.classification, &annotation.body);
        match &annotation.target {
            Target::Line { path, line, side } => {
                comment_annotation_ids.push(annotation.id);
                comments.push(ReviewCommentPayload {
                    path: path.clone(),
                    body,
                    line: *line,
                    side: side_str(*side),
                    start_line: None,
                    start_side: None,
                });
            }
            Target::Range {
                path,
                start,
                end,
                side,
            } => {
                comment_annotation_ids.push(annotation.id);
                comments.push(ReviewCommentPayload {
                    path: path.clone(),
                    body,
                    line: *end,
                    side: side_str(*side),
                    start_line: Some(*start),
                    start_side: Some(side_str(*side)),
                });
            }
            Target::Hunk { path, start, end } => {
                comment_annotation_ids.push(annotation.id);
                comments.push(ReviewCommentPayload {
                    path: path.clone(),
                    body,
                    line: *end,
                    side: side_str(Side::New),
                    start_line: Some(*start),
                    start_side: Some(side_str(Side::New)),
                });
            }
            Target::File { path } => {
                file_comment_follow_ups.push(FileCommentFollowUp {
                    annotation_id: annotation.id,
                    path: path.clone(),
                    body,
                });
            }
            Target::WorktreeLine { .. } | Target::WorktreeRange { .. } => {
                // Local-only: no forge-side position exists to anchor to.
            }
        }
    }

    ReviewSubmissionPlan {
        payload: ReviewPayload {
            body: summary.unwrap_or_default().to_string(),
            event: event_str(verdict),
            comments,
        },
        comment_annotation_ids,
        file_comment_follow_ups,
    }
}

// -- Submit-sequence argv builders (the three write endpoints) ---------------

/// Builds the fixed argv for the reviews-endpoint POST
/// (`gh api --method POST repos/{owner}/{repo}/pulls/<n>/reviews --input -`).
/// The JSON body (a [`ReviewPayload`]) is streamed on stdin (`--input -`)
/// rather than assembled into argv, so a nested `comments` array stays
/// machine-serialized and the argv itself is entirely fixed but for the PR
/// number (`u64` end-to-end). The `{owner}`/`{repo}` placeholders are
/// substituted by `gh` from the current working directory's repo — the same
/// "no repo argument built from caller input" contract [`pr_list_command`]
/// follows.
pub fn submit_review_command(number: u64) -> Command {
    let mut cmd = Command::new("gh");
    cmd.args([
        "api",
        "--method",
        "POST",
        &format!("repos/{{owner}}/{{repo}}/pulls/{number}/reviews"),
        "--input",
        "-",
    ]);
    harden(&mut cmd);
    cmd
}

/// Builds the fixed argv for a single file-level comment POST
/// (`gh api --method POST repos/{owner}/{repo}/pulls/<n>/comments --input -`).
/// The body (`{body, commit_id, path, subject_type: "file"}`) rides on stdin;
/// only the PR number is variable, and it is a `u64`.
pub fn file_comment_command(number: u64) -> Command {
    let mut cmd = Command::new("gh");
    cmd.args([
        "api",
        "--method",
        "POST",
        &format!("repos/{{owner}}/{{repo}}/pulls/{number}/comments"),
        "--input",
        "-",
    ]);
    harden(&mut cmd);
    cmd
}

/// Builds the fixed argv for a reply POST against a thread root
/// (`gh api --method POST repos/{owner}/{repo}/pulls/<n>/comments/<id>/replies --input -`).
/// Both the PR number and the root comment id are `u64` end-to-end; the reply
/// body (`{body}`) rides on stdin.
pub fn reply_command(number: u64, comment_id: u64) -> Command {
    let mut cmd = Command::new("gh");
    cmd.args([
        "api",
        "--method",
        "POST",
        &format!("repos/{{owner}}/{{repo}}/pulls/{number}/comments/{comment_id}/replies"),
        "--input",
        "-",
    ]);
    harden(&mut cmd);
    cmd
}

/// How long any one submit-sequence write (`gh api` POST) may run before it's
/// treated as failed and killed. Same budget as the reads — a real network
/// round trip, not a local store lookup.
const SUBMIT_TIMEOUT: Duration = Duration::from_secs(15);

/// The live GitHub submit executor: holds the PR number and the head commit
/// SHA the file-level comments must anchor to, and runs the three `gh api`
/// POSTs by streaming a machine-serialized JSON body on each call's stdin.
/// Constructed only on the human dogfood path (the sole place forge writes
/// run — agents never build one); the pure sequencing over the
/// [`ForgeSubmitExecutor`] seam is what carries the fixture coverage, so this
/// thin runner is deliberately untested (exercising it needs a real `gh` and
/// a live PR).
pub struct GhSubmitExecutor {
    number: u64,
    /// The PR head commit SHA, required by the single-comment endpoint for a
    /// file-level (`subject_type: file`) comment.
    head_sha: String,
}

impl GhSubmitExecutor {
    pub fn new(number: u64, head_sha: String) -> GhSubmitExecutor {
        GhSubmitExecutor { number, head_sha }
    }

    /// Runs one hardened `gh api` POST with `body` on stdin, mapping a missing
    /// CLI, a spawn failure, and a non-zero exit into the shared
    /// [`ForgeError`] shape (`command` names the endpoint for diagnostics).
    fn post(&self, mut cmd: Command, command: &str, body: Vec<u8>) -> Result<(), ForgeError> {
        let output =
            run_with_input_and_timeout(&mut cmd, body, SUBMIT_TIMEOUT).map_err(|source| {
                if source.kind() == std::io::ErrorKind::NotFound {
                    ForgeError::CliNotFound { cli: "gh" }
                } else {
                    ForgeError::Spawn { cli: "gh", source }
                }
            })?;
        if output.status.success() {
            Ok(())
        } else {
            Err(ForgeError::Command {
                cli: "gh",
                command: command.to_string(),
                code: output
                    .status
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "signal".to_string()),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            })
        }
    }
}

/// A file-level comment's JSON body (`subject_type: file`, anchored to the
/// head commit).
#[derive(Serialize)]
struct FileCommentBody<'a> {
    body: &'a str,
    commit_id: &'a str,
    path: &'a str,
    subject_type: &'a str,
}

/// A reply's JSON body — just the text.
#[derive(Serialize)]
struct ReplyBody<'a> {
    body: &'a str,
}

impl ForgeSubmitExecutor for GhSubmitExecutor {
    fn submit_review(&self, payload: &ReviewPayload) -> Result<(), ForgeError> {
        let body = serde_json::to_vec(payload).map_err(|e| ForgeError::Parse {
            cli: "gh",
            message: e.to_string(),
        })?;
        self.post(submit_review_command(self.number), "pulls reviews", body)
    }

    fn post_file_comment(&self, path: &str, body: &str) -> Result<(), ForgeError> {
        let json = serde_json::to_vec(&FileCommentBody {
            body,
            commit_id: &self.head_sha,
            path,
            subject_type: "file",
        })
        .map_err(|e| ForgeError::Parse {
            cli: "gh",
            message: e.to_string(),
        })?;
        self.post(file_comment_command(self.number), "pulls comments", json)
    }

    fn post_reply(&self, thread_id: u64, body: &str) -> Result<(), ForgeError> {
        let json = serde_json::to_vec(&ReplyBody { body }).map_err(|e| ForgeError::Parse {
            cli: "gh",
            message: e.to_string(),
        })?;
        self.post(reply_command(self.number, thread_id), "pulls replies", json)
    }
}

#[cfg(test)]
#[path = "github_tests.rs"]
mod tests;
