//! The submit-sequence driver: the ordered "one review POST, then the
//! sequential follow-ups" walk that publishes a previewed batch, marking each
//! item published only as its own write succeeds and stopping the moment one
//! fails. Pure orchestration over a [`ForgeSubmitExecutor`] seam — the real
//! executor (`super::github`) runs `gh api`, a fake records calls — so the
//! ordering, per-item marking, mid-sequence stop, and duplicate-free resume
//! are all testable without a network or a `gh` on PATH (agents never run the
//! real writes; see the repo guardrails).
//!
//! The sequence is GitHub's shape (FR-18/FR-19): the reviews endpoint carries
//! every `Line`/`Range`/`Hunk` comment plus the verdict and summary in one
//! atomic POST, then file-level comments (the reviews array can't hold them)
//! and thread replies post one at a time afterward. A GitLab equivalent
//! (draft notes + bulk-publish) is a later unit; this driver stays
//! GitHub-specific.

use super::diagnose::submit_error_headline;
use super::{ForgeError, ReviewPayload, ReviewSubmissionPlan};

/// One drafted reply queued for the post-review follow-up phase: the reply's
/// local store id (so a success marks the right draft published), the thread
/// root it answers, and its body. Plain data so the whole batch crosses the
/// render-loop/background boundary alongside the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitReplyItem {
    pub reply_id: usize,
    pub thread_id: u64,
    pub body: String,
}

/// The full batch one submit run publishes: the reviews-endpoint plan (line
/// comments + verdict + summary, plus the file-comment follow-ups it routes
/// out), the drafted replies, and whether the reviews POST itself still needs
/// to happen. `include_review_post` is `false` only on a *resume* after a
/// prior run's review POST already landed (its verdict and line comments are
/// on the forge) but a later follow-up failed — re-posting the review would
/// double the verdict, so the resume skips straight to the remaining
/// follow-ups. On a fresh submit it is always `true`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitBatch {
    pub plan: ReviewSubmissionPlan,
    pub replies: Vec<SubmitReplyItem>,
    pub include_review_post: bool,
}

/// What one submit run accomplished: which annotations and replies are now
/// published (to mark locally and persist), whether the reviews POST is now
/// done (so a resume skips it), and the one-line diagnostic when the run
/// stopped early. `failure: None` means every item in the batch published.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SubmitReport {
    /// Store ids of annotations published this run — the line/range/hunk
    /// comments (all at once, when the review POST succeeds) and any file
    /// comments that posted before a stop.
    pub published_annotation_ids: Vec<usize>,
    /// Store ids of replies published this run, in send order.
    pub published_reply_ids: Vec<usize>,
    /// Whether the reviews-endpoint POST has now succeeded (either this run,
    /// or a prior run when `include_review_post` was `false`) — the signal a
    /// resume reads to decide whether to skip the review POST.
    pub review_submitted: bool,
    /// The one-line diagnostic that stopped the run, or `None` when the whole
    /// batch published.
    pub failure: Option<String>,
}

/// The three positioned GitHub write operations the driver sequences, behind
/// one seam so the walk is testable with a recording fake. Every method
/// builds its own `gh api` argv from typed values and streams a
/// machine-serialized JSON body on stdin (see `super::github`); none takes a
/// string-assembled command line. Errors are the shared [`ForgeError`], whose
/// first stderr line the caller surfaces (plus a next-step hint when the
/// failure is HTTP-401/403-shaped — see
/// [`super::diagnose::submit_error_headline`]).
pub trait ForgeSubmitExecutor {
    /// Publishes the whole review (line comments + verdict + summary) in one
    /// atomic reviews-endpoint POST.
    fn submit_review(&self, payload: &ReviewPayload) -> Result<(), ForgeError>;

    /// Posts one file-level comment (the reviews array can't carry these).
    fn post_file_comment(&self, path: &str, body: &str) -> Result<(), ForgeError>;

    /// Posts one reply against a thread root (`thread_id`).
    fn post_reply(&self, thread_id: u64, body: &str) -> Result<(), ForgeError>;
}

/// Runs one submit pass over `batch` against `exec`, returning what published
/// before either the whole batch went out or one write failed.
///
/// Order (FR-18/FR-19): the atomic reviews POST first (unless the batch is a
/// resume that already delivered it), then file comments, then replies —
/// each marked published on its own success, the walk stopping at the first
/// failure with everything after it left unpublished. A caller that rebuilds
/// the next batch from only the still-unpublished set therefore re-sends
/// nothing that already landed.
pub fn run_submit_sequence(batch: &SubmitBatch, exec: &dyn ForgeSubmitExecutor) -> SubmitReport {
    let mut report = SubmitReport {
        review_submitted: !batch.include_review_post,
        ..SubmitReport::default()
    };

    // Skip the reviews POST when it would carry nothing GitHub accepts — an
    // empty-body `COMMENT` review with no comments 422s. A reply-only batch
    // (verdict Comment, no summary, no unpublished comments) goes straight to
    // the follow-ups, leaving `review_submitted` false so a later batch that
    // does carry a verdict or comment still posts the review.
    if batch.include_review_post && batch.plan.payload.carries_content() {
        if let Err(e) = exec.submit_review(&batch.plan.payload) {
            report.failure = Some(submit_error_headline(&e));
            return report;
        }
        report.review_submitted = true;
        // The reviews POST is atomic: every line/range/hunk comment landed
        // together, so all their annotations are published at once.
        report
            .published_annotation_ids
            .extend(batch.plan.comment_annotation_ids.iter().copied());
    }

    for follow_up in &batch.plan.file_comment_follow_ups {
        if let Err(e) = exec.post_file_comment(&follow_up.path, &follow_up.body) {
            report.failure = Some(submit_error_headline(&e));
            return report;
        }
        report
            .published_annotation_ids
            .push(follow_up.annotation_id);
    }

    for reply in &batch.replies {
        if let Err(e) = exec.post_reply(reply.thread_id, &reply.body) {
            report.failure = Some(submit_error_headline(&e));
            return report;
        }
        report.published_reply_ids.push(reply.reply_id);
    }

    report
}

#[cfg(test)]
#[path = "submit_tests.rs"]
mod tests;
