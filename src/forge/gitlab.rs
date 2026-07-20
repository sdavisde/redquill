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

use serde::{Deserialize, Serialize};

use crate::annotate::Side;

use super::process::{harden_glab, run_captured_with_timeout, run_with_input_and_timeout};
use super::submit::SubmitReport;
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
    /// The discussion's own string id — GitLab's reply target (a reply is a
    /// note appended to *this* discussion, not to the root note's id).
    id: String,
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
        // Carry GitLab's discussion string id so a drafted reply can target
        // the discussion (not the root note id) at submit time.
        discussion_id: Some(raw.id),
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

// -- Submit: position hashes, draft-notes sequence, visible fallback ----------

/// How long any one `glab api`/`glab mr approve` submit write may run before
/// it's treated as failed and killed. Same budget the reads use.
const SUBMIT_TIMEOUT: Duration = Duration::from_secs(15);

/// A GitLab diff-note position, built from an annotation's side/line data and
/// the MR's [`DiffRefs`] — the reverse of the [`anchor_for`] import mapping.
/// A `"text"` position carries exactly one of `new_line`/`old_line` (the side
/// the annotation is on); a `"file"` position carries neither. `new_path` and
/// `old_path` both hold the annotation's path: annotations don't track a
/// rename's old path, and GitLab requires both for a text position on a file
/// that isn't renamed. Serialized straight into a draft-note / discussion body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NotePosition {
    pub base_sha: String,
    pub start_sha: String,
    pub head_sha: String,
    pub position_type: &'static str,
    pub new_path: String,
    pub old_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_line: Option<u32>,
}

/// What an annotation anchors to, as a position-hash input: a specific line on
/// one side of the diff, or a whole file. A multi-line span (`Range`/`Hunk`)
/// collapses to its end line on the same side before reaching here, mirroring
/// how the GitHub review payload anchors a span at its `line`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoteTarget {
    Line { path: String, side: Side, line: u32 },
    File { path: String },
}

/// Builds the [`NotePosition`] for one annotation against the MR's `diff_refs`:
/// an added/context (`New`-side) line fills `new_line`, a removed (`Old`-side)
/// line fills `old_line`, and a file target is a `"file"` position with no
/// line — the reverse of the discussions-import mapping.
pub fn build_note_position(diff_refs: &DiffRefs, target: &NoteTarget) -> NotePosition {
    let base = |position_type, path: &str, new_line, old_line| NotePosition {
        base_sha: diff_refs.base_sha.clone(),
        start_sha: diff_refs.start_sha.clone(),
        head_sha: diff_refs.head_sha.clone(),
        position_type,
        new_path: path.to_string(),
        old_path: path.to_string(),
        new_line,
        old_line,
    };
    match target {
        NoteTarget::Line {
            path,
            side: Side::New,
            line,
        } => base("text", path, Some(*line), None),
        NoteTarget::Line {
            path,
            side: Side::Old,
            line,
        } => base("text", path, None, Some(*line)),
        NoteTarget::File { path } => base("file", path, None, None),
    }
}

/// One positioned note to publish: the originating annotation's store id (so a
/// success marks it published), its body, and its diff position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitlabNote {
    pub annotation_id: usize,
    pub body: String,
    pub position: NotePosition,
}

/// One drafted reply to publish: its store id, the discussion string id it
/// answers (GitLab replies target the *discussion*, not the root note), and
/// its body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitlabReply {
    pub reply_id: usize,
    pub discussion_id: String,
    pub body: String,
}

/// The full batch one GitLab submit run publishes: an optional review summary
/// (posted as a non-positioned note), the positioned annotation notes, the
/// drafted replies, and whether to approve after publishing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GitlabSubmitBatch {
    pub summary: Option<String>,
    pub notes: Vec<GitlabNote>,
    pub replies: Vec<GitlabReply>,
    pub approve: bool,
}

/// The GitLab write operations the submit driver sequences, behind one seam so
/// the near-atomic draft path, the visible fallback, and the approve step are
/// all testable with a recording fake — no `glab` on PATH, no network (agents
/// never run the real writes; see the repo guardrails). Every method builds
/// its own `glab` argv from typed values.
pub trait GitlabSubmitExecutor {
    /// Creates one private draft note: positioned (annotation), a plain review
    /// summary (`position` and `reply` both `None`), or a reply to a discussion
    /// (`reply` = its string id). Invisible to others until [`Self::bulk_publish_drafts`].
    fn create_draft_note(
        &self,
        body: &str,
        position: Option<&NotePosition>,
        in_reply_to_discussion_id: Option<&str>,
    ) -> Result<(), ForgeError>;

    /// Publishes every draft note on the MR at once — the moment the whole
    /// batch becomes visible.
    fn bulk_publish_drafts(&self) -> Result<(), ForgeError>;

    /// The visible-fallback create for a positioned annotation or a plain
    /// summary (`position` `None`) — posts immediately, no draft staging.
    fn create_discussion(
        &self,
        body: &str,
        position: Option<&NotePosition>,
    ) -> Result<(), ForgeError>;

    /// The visible-fallback reply: a note appended to an existing discussion.
    fn create_discussion_reply(&self, discussion_id: &str, body: &str) -> Result<(), ForgeError>;

    /// Approves the MR (only ever called when the verdict is approve).
    fn approve(&self) -> Result<(), ForgeError>;
}

/// The one-line diagnostic a [`ForgeError`] contributes to a stopped run —
/// a `Command` error's first non-empty stderr line, else its `Display`.
/// Mirrors `super::submit::error_headline`.
fn error_headline(e: &ForgeError) -> String {
    match e {
        ForgeError::Command { stderr, .. } if !stderr.trim().is_empty() => stderr
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or(stderr)
            .trim()
            .to_string(),
        other => other.to_string(),
    }
}

/// Whether a draft-note create failed because the instance has no draft-notes
/// API (older GitLab) rather than a genuine error — the signal to fall back to
/// visible discussions. Recognized from a `404`-shaped `Command` error, the
/// documented response for the missing endpoint.
fn is_draft_notes_unavailable(e: &ForgeError) -> bool {
    match e {
        ForgeError::Command { code, stderr, .. } => {
            code == "404"
                || stderr.contains("404")
                || stderr.to_ascii_lowercase().contains("not found")
        }
        _ => false,
    }
}

/// A stopped run that published nothing — its diagnostic in `failure`. Used
/// when a draft create or the bulk-publish fails: drafts are author-only, so
/// no teammate ever saw a partial batch.
fn nothing_published(e: &ForgeError) -> SubmitReport {
    SubmitReport {
        failure: Some(error_headline(e)),
        ..SubmitReport::default()
    }
}

/// Whether the draft path even ran to completion, or bailed on the very first
/// create because the endpoint is unavailable (→ visible fallback).
enum DraftAttempt {
    Completed(SubmitReport),
    Unavailable,
}

/// Runs one GitLab submit pass over `batch` against `exec`. Prefers the
/// near-atomic draft-notes path — every note staged privately, then one
/// bulk-publish makes the whole batch visible at once, then `approve` when
/// asked — so a failure before bulk-publish leaves nothing on the MR. When the
/// instance has no draft-notes API (a `404` on the first create), it falls back
/// to sequential visible discussions with the Unit-4 per-item published
/// marking. A caller that rebuilds the next batch from only the
/// still-unpublished set re-sends nothing that already landed.
pub fn run_gitlab_submit_sequence(
    batch: &GitlabSubmitBatch,
    exec: &dyn GitlabSubmitExecutor,
) -> SubmitReport {
    match try_draft_submit(batch, exec) {
        DraftAttempt::Completed(report) => report,
        DraftAttempt::Unavailable => run_visible_fallback(batch, exec),
    }
}

/// The draft-notes attempt: create the summary note, then each positioned
/// note, then each reply as private drafts; a `404` on the *first* create means
/// the endpoint is unavailable (→ [`DraftAttempt::Unavailable`]); any other
/// create failure, or a bulk-publish failure, stops with nothing published
/// (drafts are invisible). On bulk-publish success every item is marked
/// published together; a later `approve` failure leaves the (already published)
/// comments in place with the diagnostic surfaced.
fn try_draft_submit(batch: &GitlabSubmitBatch, exec: &dyn GitlabSubmitExecutor) -> DraftAttempt {
    let mut created = 0usize;

    if let Some(summary) = &batch.summary {
        match exec.create_draft_note(summary, None, None) {
            Ok(()) => created += 1,
            Err(e) if is_draft_notes_unavailable(&e) => return DraftAttempt::Unavailable,
            Err(e) => return DraftAttempt::Completed(nothing_published(&e)),
        }
    }
    for note in &batch.notes {
        match exec.create_draft_note(&note.body, Some(&note.position), None) {
            Ok(()) => created += 1,
            Err(e) if created == 0 && is_draft_notes_unavailable(&e) => {
                return DraftAttempt::Unavailable;
            }
            Err(e) => return DraftAttempt::Completed(nothing_published(&e)),
        }
    }
    for reply in &batch.replies {
        match exec.create_draft_note(&reply.body, None, Some(&reply.discussion_id)) {
            Ok(()) => created += 1,
            Err(e) if created == 0 && is_draft_notes_unavailable(&e) => {
                return DraftAttempt::Unavailable;
            }
            Err(e) => return DraftAttempt::Completed(nothing_published(&e)),
        }
    }

    if created > 0
        && let Err(e) = exec.bulk_publish_drafts()
    {
        return DraftAttempt::Completed(nothing_published(&e));
    }

    let mut report = SubmitReport {
        published_annotation_ids: batch.notes.iter().map(|n| n.annotation_id).collect(),
        published_reply_ids: batch.replies.iter().map(|r| r.reply_id).collect(),
        review_submitted: true,
        failure: None,
    };
    if batch.approve
        && let Err(e) = exec.approve()
    {
        report.failure = Some(error_headline(&e));
    }
    DraftAttempt::Completed(report)
}

/// The visible-discussions fallback (no draft-notes API): post the summary,
/// then each positioned note, then each reply, one at a time — marking each
/// item published as its own write lands and stopping at the first failure, so
/// a resume re-sends only the remainder (the Unit-4 discipline).
fn run_visible_fallback(
    batch: &GitlabSubmitBatch,
    exec: &dyn GitlabSubmitExecutor,
) -> SubmitReport {
    let mut report = SubmitReport::default();

    if let Some(summary) = &batch.summary
        && let Err(e) = exec.create_discussion(summary, None)
    {
        report.failure = Some(error_headline(&e));
        return report;
    }
    // The review-level content (the summary) is now visible; mark it so a
    // resume after a later per-item failure skips re-posting the summary (a
    // summary discussion carries no id to exclude it by).
    report.review_submitted = true;
    for note in &batch.notes {
        if let Err(e) = exec.create_discussion(&note.body, Some(&note.position)) {
            report.failure = Some(error_headline(&e));
            return report;
        }
        report.published_annotation_ids.push(note.annotation_id);
    }
    for reply in &batch.replies {
        if let Err(e) = exec.create_discussion_reply(&reply.discussion_id, &reply.body) {
            report.failure = Some(error_headline(&e));
            return report;
        }
        report.published_reply_ids.push(reply.reply_id);
    }
    if batch.approve
        && let Err(e) = exec.approve()
    {
        report.failure = Some(error_headline(&e));
    }
    report
}

// -- Submit argv builders (the draft-notes / discussions / approve endpoints) --

/// `glab api --method POST -H "Content-Type: application/json" projects/:id/merge_requests/<iid>/draft_notes --input -`.
/// The JSON body (note text + optional position/reply target) streams on stdin,
/// so the argv is fixed but for the `u64` iid. The explicit header is required
/// because `--input` streams raw bytes without `glab` inferring a content type
/// (unlike its `-f`/`-F` field style) — GitLab returns 415 without it.
pub fn draft_note_command(iid: u64) -> Command {
    let mut cmd = Command::new("glab");
    cmd.args([
        "api",
        "--method",
        "POST",
        "-H",
        "Content-Type: application/json",
        &format!("projects/:id/merge_requests/{iid}/draft_notes"),
        "--input",
        "-",
    ]);
    harden_glab(&mut cmd);
    cmd
}

/// `glab api --method POST -H "Content-Type: application/json" projects/:id/merge_requests/<iid>/draft_notes/bulk_publish`.
/// Sends no body, but still needs the header: without it `glab` sends no
/// Content-Type and GitLab 415s; with it, `glab` sends an empty `{}` body,
/// which GitLab accepts.
pub fn bulk_publish_command(iid: u64) -> Command {
    let mut cmd = Command::new("glab");
    cmd.args([
        "api",
        "--method",
        "POST",
        "-H",
        "Content-Type: application/json",
        &format!("projects/:id/merge_requests/{iid}/draft_notes/bulk_publish"),
    ]);
    harden_glab(&mut cmd);
    cmd
}

/// `glab api --method POST -H "Content-Type: application/json" projects/:id/merge_requests/<iid>/discussions --input -`
/// (the visible-fallback create). Body (`body` + optional `position`) on stdin.
pub fn discussion_create_command(iid: u64) -> Command {
    let mut cmd = Command::new("glab");
    cmd.args([
        "api",
        "--method",
        "POST",
        "-H",
        "Content-Type: application/json",
        &format!("projects/:id/merge_requests/{iid}/discussions"),
        "--input",
        "-",
    ]);
    harden_glab(&mut cmd);
    cmd
}

/// `glab api --method POST -H "Content-Type: application/json" projects/:id/merge_requests/<iid>/discussions/<did>/notes --input -`
/// (the visible-fallback reply). `did` is the discussion's own string id, from
/// the discussions read — a hex hash, placed as a single path segment; the body
/// (`body`) streams on stdin.
pub fn discussion_reply_command(iid: u64, discussion_id: &str) -> Command {
    let mut cmd = Command::new("glab");
    cmd.args([
        "api",
        "--method",
        "POST",
        "-H",
        "Content-Type: application/json",
        &format!("projects/:id/merge_requests/{iid}/discussions/{discussion_id}/notes"),
        "--input",
        "-",
    ]);
    harden_glab(&mut cmd);
    cmd
}

/// `glab mr approve <iid>` — the approve verdict's own porcelain command.
pub fn approve_command(iid: u64) -> Command {
    let mut cmd = Command::new("glab");
    cmd.args(["mr", "approve", &iid.to_string()]);
    harden_glab(&mut cmd);
    cmd
}

/// A draft note's JSON body: the text, plus at most one of a diff position
/// (annotation) or an `in_reply_to_discussion_id` (reply). A bare `note`
/// carries the review summary.
#[derive(Serialize)]
struct DraftNoteBody<'a> {
    note: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    position: Option<&'a NotePosition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    in_reply_to_discussion_id: Option<&'a str>,
}

/// A visible discussion's JSON body: the text plus an optional diff position.
#[derive(Serialize)]
struct DiscussionBody<'a> {
    body: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    position: Option<&'a NotePosition>,
}

/// A visible discussion reply's JSON body — just the text.
#[derive(Serialize)]
struct DiscussionReplyBody<'a> {
    body: &'a str,
}

/// The live GitLab submit executor: holds the MR iid and runs each `glab api`
/// POST by streaming a machine-serialized JSON body on stdin. Constructed only
/// on the human dogfood path (agents never build one); the pure sequencing over
/// the [`GitlabSubmitExecutor`] seam carries the fixture coverage, so this thin
/// runner is deliberately untested (exercising it needs a real `glab` and a
/// live MR).
pub struct GlabSubmitExecutor {
    number: u64,
}

impl GlabSubmitExecutor {
    pub fn new(number: u64) -> GlabSubmitExecutor {
        GlabSubmitExecutor { number }
    }

    /// Runs one hardened `glab api` POST with `body` on stdin, mapping a
    /// missing CLI, a spawn failure, and a non-zero exit into [`ForgeError`]
    /// (`command` names the endpoint for diagnostics).
    fn post(&self, mut cmd: Command, command: String, body: Vec<u8>) -> Result<(), ForgeError> {
        let output =
            run_with_input_and_timeout(&mut cmd, body, SUBMIT_TIMEOUT).map_err(|source| {
                if source.kind() == std::io::ErrorKind::NotFound {
                    ForgeError::CliNotFound { cli: "glab" }
                } else {
                    ForgeError::Spawn {
                        cli: "glab",
                        source,
                    }
                }
            })?;
        status_into_result(output, command)
    }
}

/// Maps a captured `glab` output into `Ok`/`ForgeError::Command`, shared by the
/// stdin-body POSTs and the no-input approve.
fn status_into_result(
    output: super::process::CapturedOutput,
    command: String,
) -> Result<(), ForgeError> {
    if output.status.success() {
        Ok(())
    } else {
        Err(ForgeError::Command {
            cli: "glab",
            command,
            code: output
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".to_string()),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}

impl GitlabSubmitExecutor for GlabSubmitExecutor {
    fn create_draft_note(
        &self,
        body: &str,
        position: Option<&NotePosition>,
        in_reply_to_discussion_id: Option<&str>,
    ) -> Result<(), ForgeError> {
        let json = serde_json::to_vec(&DraftNoteBody {
            note: body,
            position,
            in_reply_to_discussion_id,
        })
        .map_err(|e| ForgeError::Parse {
            cli: "glab",
            message: e.to_string(),
        })?;
        self.post(
            draft_note_command(self.number),
            "draft_notes".to_string(),
            json,
        )
    }

    fn bulk_publish_drafts(&self) -> Result<(), ForgeError> {
        let mut cmd = bulk_publish_command(self.number);
        let output = run_captured_with_timeout(&mut cmd, SUBMIT_TIMEOUT).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                ForgeError::CliNotFound { cli: "glab" }
            } else {
                ForgeError::Spawn {
                    cli: "glab",
                    source,
                }
            }
        })?;
        status_into_result(output, "draft_notes/bulk_publish".to_string())
    }

    fn create_discussion(
        &self,
        body: &str,
        position: Option<&NotePosition>,
    ) -> Result<(), ForgeError> {
        let json = serde_json::to_vec(&DiscussionBody { body, position }).map_err(|e| {
            ForgeError::Parse {
                cli: "glab",
                message: e.to_string(),
            }
        })?;
        self.post(
            discussion_create_command(self.number),
            "discussions".to_string(),
            json,
        )
    }

    fn create_discussion_reply(&self, discussion_id: &str, body: &str) -> Result<(), ForgeError> {
        let json =
            serde_json::to_vec(&DiscussionReplyBody { body }).map_err(|e| ForgeError::Parse {
                cli: "glab",
                message: e.to_string(),
            })?;
        self.post(
            discussion_reply_command(self.number, discussion_id),
            "discussions/notes".to_string(),
            json,
        )
    }

    fn approve(&self) -> Result<(), ForgeError> {
        let mut cmd = approve_command(self.number);
        let output = run_captured_with_timeout(&mut cmd, SUBMIT_TIMEOUT).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                ForgeError::CliNotFound { cli: "glab" }
            } else {
                ForgeError::Spawn {
                    cli: "glab",
                    source,
                }
            }
        })?;
        status_into_result(output, "mr approve".to_string())
    }
}

#[cfg(test)]
#[path = "gitlab_tests.rs"]
mod tests;
