//! Forge (GitHub/GitLab) integration: all "talk to the code-hosting
//! service" logic lives behind [`ForgeProvider`], mirroring `crate::git`'s
//! "own all interaction with an external CLI, no TUI types leak in here"
//! shape. A layering-guard test (`mod_tests.rs`) walks this directory's
//! source files and fails if any of them mention a TUI crate.
//!
//! - [`ForgeProvider`] — the trait every provider (GitHub, GitLab)
//!   implements: list PRs, read PR detail, fetch comment threads, submit a
//!   review, and report capability flags. UI code talks only to this trait,
//!   never to `gh`/`glab` directly, so it's testable with fakes. The
//!   `gitlab` module supplies GitLab's reads (MR listing, detail with
//!   `diff_refs`, discussion-thread import into the same [`Thread`] model
//!   `github` uses) into the same typed rows `github` produces; it isn't
//!   wired behind the trait itself yet.
//! - [`PullRequest`] — one row of a PR/MR listing.
//! - [`PrDetail`] — a minimal stand-in for a richer shape later work
//!   fleshes out; present now only so the trait surface compiles.
//! - [`Thread`] — an imported PR review-comment thread (root + ordered
//!   replies, resolved/outdated state, diff anchor); see [`threads`] for
//!   construction from GitHub's JSON shape and the read-only
//!   `ThreadOverlayStore` fetched threads live in.
//! - [`ReviewSubmission`] — a minimal stand-in for the submit-flow payload
//!   later work fleshes out; present now only so the trait surface
//!   compiles.
//! - [`Verdict`]/[`Capabilities`] — the review verdict a submission carries,
//!   and which verdicts/submit shapes a given provider actually supports.
//! - [`ForgeError`] — the shared error type for every provider operation.

mod detect;
mod github;
mod gitlab;
mod process;
mod remote_url;
mod submit;
mod threads;

pub use detect::{
    CredentialChecker, GhCredentialChecker, GlabCredentialChecker, ProviderKind,
    ProviderResolution, ResolutionCache, UnresolvedReason, resolve_provider,
};
pub use github::{
    FileCommentFollowUp, GhSubmitExecutor, PR_LIST_JSON_FIELDS, ReviewCommentPayload,
    ReviewPayload, ReviewSubmissionPlan, build_review_payload, fetch_review_threads,
    file_comment_command, list_open_prs, parse_pr_list_json, pr_list_command, reply_command,
    review_comments_command, review_threads_resolved_command, submit_review_command,
};
pub use gitlab::{
    DiffRefs, MrDetail, discussions_command, fetch_discussions, list_open_mrs, mr_detail,
    mr_detail_command, mr_list_command, parse_discussions_json, parse_mr_detail_json,
    parse_mr_list_json,
};
pub use remote_url::{Hostname, RemoteUrlError, parse_origin_hostname, parse_origin_repo_slug};
pub use submit::{
    ForgeSubmitExecutor, SubmitBatch, SubmitReplyItem, SubmitReport, run_submit_sequence,
};
pub use threads::{
    Thread, ThreadAnchor, ThreadComment, ThreadOverlayStore, apply_resolved_states,
    parse_resolved_thread_states, parse_review_comments_json,
};

use thiserror::Error;

/// One PR/MR row as listed by a provider: enough to render a list and to
/// target a checkout, nothing more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    /// The PR/MR number (GitHub) or internal ID (GitLab).
    pub number: u64,
    pub title: String,
    /// The author's login/username.
    pub author: String,
    /// The source branch name (`headRefName` on GitHub).
    pub head_ref: String,
    /// The target/base branch name (`baseRefName` on GitHub) the PR merges
    /// into — the ref a review's `base...head` diff is taken against.
    pub base_ref: String,
    pub is_draft: bool,
    /// Provider-formatted timestamp string, verbatim — no local parsing or
    /// timezone conversion happens at this layer.
    pub updated_at: String,
}

/// One PR's full detail (base/head SHAs, diff refs, etc.). A minimal
/// stand-in until PR checkout needs the richer shape — only `number` is
/// populated today, but the method exists so the trait surface is real.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PrDetail {
    pub number: u64,
}

/// A batch of comments/replies/verdict ready to publish as one review. A
/// minimal stand-in until the submit flow lands.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReviewSubmission {
    pub verdict: Option<Verdict>,
    pub summary: String,
}

/// The outcome a reviewer chooses when submitting a review. Not every
/// provider supports every variant — see [`Capabilities`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Comment,
    Approve,
    RequestChanges,
}

/// Which verdicts and submit-shape behaviors a provider instance actually
/// supports, so UI code can render only the choices that will really work
/// (e.g. GitHub supports all three verdicts; GitLab v1 has no
/// request-changes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub can_approve: bool,
    pub can_request_changes: bool,
    /// Whether this provider can stage a batch privately before publishing
    /// it atomically (GitLab draft notes) as opposed to posting each item
    /// visibly as it goes.
    pub near_atomic_submit: bool,
}

/// Errors produced while running or parsing a forge CLI (`gh`/`glab`).
/// Mirrors `crate::git::GitError`'s shape; `cli` names which CLI was
/// involved since a single process may talk to either.
#[derive(Debug, Error)]
pub enum ForgeError {
    /// The named CLI executable could not be found on `PATH`.
    #[error("{cli} executable not found on PATH")]
    CliNotFound {
        /// The CLI binary name (e.g. `"gh"`, `"glab"`).
        cli: &'static str,
    },

    /// Spawning the CLI failed for a reason other than it being missing.
    #[error("failed to run {cli}: {source}")]
    Spawn {
        cli: &'static str,
        #[source]
        source: std::io::Error,
    },

    /// A CLI invocation exited with a non-zero status.
    #[error("{cli} {command} exited with status {code}: {stderr}")]
    Command {
        cli: &'static str,
        /// The subcommand and arguments that were run.
        command: String,
        /// The exit code (or `"signal"` if terminated by a signal).
        code: String,
        /// Captured stderr, trimmed of trailing whitespace.
        stderr: String,
    },

    /// A CLI invocation's output did not match the expected machine format.
    #[error("failed to parse {cli} output: {message}")]
    Parse { cli: &'static str, message: String },
}

/// Everything forge-specific behind one seam: the UI layer talks only to
/// this trait, never to `gh`/`glab` directly, so it's testable with fakes
/// and a second provider (GitLab) is a second impl, not a UI branch. All
/// CLI invocations behind implementations of this trait build argv from
/// closed/typed values (PR numbers as integers, hostnames validated
/// against a strict charset) — never a string-assembled command line.
pub trait ForgeProvider {
    /// Lists the repo's open PRs/MRs.
    fn list_open_prs(&self) -> Result<Vec<PullRequest>, ForgeError>;

    /// Reads one PR's full detail.
    fn pr_detail(&self, number: u64) -> Result<PrDetail, ForgeError>;

    /// Fetches a PR's existing comment threads, live — never cached to disk.
    fn fetch_threads(&self, number: u64) -> Result<Vec<Thread>, ForgeError>;

    /// Publishes one review (comments, replies, verdict) against a PR.
    fn submit_review(&self, number: u64, submission: ReviewSubmission) -> Result<(), ForgeError>;

    /// This provider's supported verdicts and submit-shape capabilities.
    fn capabilities(&self) -> Capabilities;
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
