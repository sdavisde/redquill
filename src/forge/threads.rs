//! The imported PR-thread model: root comment + ordered replies, resolved/
//! outdated state, and where a thread anchors in the diff.
//!
//! Built from GitHub's `GET .../pulls/{number}/comments` REST shape (the
//! same endpoint [`super::github::fetch_review_threads`] calls). Two things
//! about that shape drive the design here:
//!
//! - Every comment in a thread — root and replies alike — carries its own
//!   copy of `path`/`side`/`line`, but a reply's `in_reply_to_id` always
//!   points at the thread's root id (GitHub threads are flat, not a tree),
//!   so grouping is a single bucket-by-root pass, not a tree walk.
//! - `line` (and, historically, the deprecated `position`) is `null` on a
//!   comment whose diff position could no longer be resolved — that's the
//!   documented "outdated" signal, and it's the only one this endpoint
//!   gives us. GitHub's REST API has **no thread-resolution field** at all
//!   (`resolved`/`is_resolved` only exists on `PullRequestReviewThread` via
//!   the GraphQL API); every thread built here therefore parses with
//!   `resolved: false`. The field stays on [`Thread`] so a later change can
//!   populate it from a separate signal without reshaping this type.
//!
//! [`Side`] is reused from `crate::annotate` rather than redefined here:
//! it's the same "which side of the diff" concept a `Target::Line`
//! already carries, and a thread's anchor is meant to line up with that
//! vocabulary once the UI overlay (a later unit) renders threads alongside
//! annotations. This is deliberate cross-layer coupling, not an accident.

use std::collections::HashMap;

use serde::Deserialize;

use crate::annotate::Side;

use super::ForgeError;

/// One message in a thread: the root comment or one reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadComment {
    /// The provider's comment id.
    pub id: u64,
    /// The commenting user's login.
    pub author: String,
    /// Raw provider timestamp (RFC 3339), verbatim — relative-time
    /// rendering is a UI concern, not this layer's.
    pub created_at: String,
    pub body: String,
}

/// Where a thread attaches in the diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadAnchor {
    /// A specific line still present in the diff.
    Position { path: String, side: Side, line: u32 },
    /// The position no longer maps (outdated) — falls back to the whole
    /// file rather than being dropped.
    File { path: String },
}

impl ThreadAnchor {
    /// The path this anchor is attached to, regardless of variant.
    pub fn path(&self) -> &str {
        match self {
            ThreadAnchor::Position { path, .. } | ThreadAnchor::File { path } => path,
        }
    }
}

/// One review conversation: a root comment plus every reply, in
/// conversation order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thread {
    /// The root comment's provider id — also the id every reply's
    /// `in_reply_to` links to, and what a locally drafted reply targets.
    pub id: u64,
    pub anchor: ThreadAnchor,
    pub root: ThreadComment,
    /// Ordered oldest-first; a 5-reply back-and-forth reads top-to-bottom.
    pub replies: Vec<ThreadComment>,
    /// Always `false` from JSON construction — see the module doc for why
    /// this endpoint can't supply it.
    pub resolved: bool,
    /// Derived from the root's diff position being unmappable; mirrors
    /// `anchor` being [`ThreadAnchor::File`] but named separately since
    /// "outdated" is the cause and file-level attachment is the effect.
    pub outdated: bool,
}

/// The raw shape of one entry in the review-comments JSON array. Only the
/// fields this model needs are declared; the real payload carries many more
/// (`diff_hunk`, `commit_id`, `_links`, ...) that serde ignores by default.
#[derive(Debug, Deserialize)]
struct RawComment {
    id: u64,
    #[serde(default)]
    in_reply_to_id: Option<u64>,
    path: String,
    #[serde(default)]
    side: Option<String>,
    #[serde(default)]
    line: Option<u32>,
    user: RawUser,
    created_at: String,
    body: String,
}

#[derive(Debug, Deserialize)]
struct RawUser {
    login: String,
}

/// Parses `gh api .../pulls/{n}/comments`'s stdout into ordered threads.
/// Pure — no process involved — so it's exercised entirely by fixture
/// tests.
pub fn parse_review_comments_json(json: &str) -> Result<Vec<Thread>, ForgeError> {
    let raw: Vec<RawComment> = serde_json::from_str(json).map_err(|e| ForgeError::Parse {
        cli: "gh",
        message: e.to_string(),
    })?;
    Ok(build_threads(raw))
}

/// Groups a flat comment list into threads, root-first in order of each
/// root's first appearance in `raw`, replies sorted within a thread by
/// `(created_at, id)`.
fn build_threads(raw: Vec<RawComment>) -> Vec<Thread> {
    let by_id: HashMap<u64, &RawComment> = raw.iter().map(|c| (c.id, c)).collect();

    let mut root_order: Vec<u64> = Vec::new();
    let mut buckets: HashMap<u64, Vec<&RawComment>> = HashMap::new();
    for comment in &raw {
        let root_id = root_id_of(comment, &by_id);
        buckets
            .entry(root_id)
            .or_insert_with(|| {
                root_order.push(root_id);
                Vec::new()
            })
            .push(comment);
    }

    let mut threads = Vec::with_capacity(root_order.len());
    for root_id in root_order {
        // The root id always came from an entry in `by_id` (either a
        // comment's own id or a resolved `in_reply_to_id` chain), so this
        // lookup can't miss in practice; skip defensively rather than
        // panic if it ever does.
        let Some(root_raw) = by_id.get(&root_id) else {
            continue;
        };
        let mut members = buckets.remove(&root_id).unwrap_or_default();
        members.sort_by(|a, b| (a.created_at.as_str(), a.id).cmp(&(b.created_at.as_str(), b.id)));

        let replies = members
            .iter()
            .filter(|c| c.id != root_id)
            .map(|c| to_thread_comment(c))
            .collect();

        threads.push(Thread {
            id: root_id,
            anchor: anchor_for(root_raw),
            root: to_thread_comment(root_raw),
            replies,
            resolved: false,
            outdated: root_raw.line.is_none(),
        });
    }
    threads
}

/// Resolves `comment`'s thread-root id by following `in_reply_to_id` until
/// reaching a comment with none. GitHub always points every reply directly
/// at the root, so this loop runs at most once in practice; the cycle guard
/// and dangling-reference fallback exist only so malformed input degrades
/// (treat the malformed comment as its own root) rather than looping
/// forever or panicking.
fn root_id_of(comment: &RawComment, by_id: &HashMap<u64, &RawComment>) -> u64 {
    let mut current = comment;
    let mut visited = std::collections::HashSet::new();
    while let Some(parent_id) = current.in_reply_to_id {
        if !visited.insert(current.id) {
            break;
        }
        match by_id.get(&parent_id) {
            Some(parent) => current = parent,
            None => break,
        }
    }
    current.id
}

fn to_thread_comment(raw: &RawComment) -> ThreadComment {
    ThreadComment {
        id: raw.id,
        author: raw.user.login.clone(),
        created_at: raw.created_at.clone(),
        body: raw.body.clone(),
    }
}

fn anchor_for(raw: &RawComment) -> ThreadAnchor {
    match raw.line {
        Some(line) => ThreadAnchor::Position {
            path: raw.path.clone(),
            side: parse_side(raw.side.as_deref()),
            line,
        },
        None => ThreadAnchor::File {
            path: raw.path.clone(),
        },
    }
}

/// Maps GitHub's `"LEFT"`/`"RIGHT"` side strings onto [`Side`]. Anything
/// else (missing, or an unrecognized value) defaults to `New` — a comment
/// with a resolved `line` but no readable `side` still needs *some* anchor
/// side, and `New` is the common case (added/context lines).
fn parse_side(side: Option<&str>) -> Side {
    match side {
        Some("LEFT") => Side::Old,
        _ => Side::New,
    }
}

/// A read-only container for one PR's fetched comment threads, held
/// entirely separate from `crate::annotate::AnnotationStore`: teammates'
/// existing comments are shown for context, not treated as this reviewer's
/// own annotations. Enforced structurally, not just by convention — neither
/// `Thread` nor this store derives `Serialize`, and the only mutator is
/// [`ThreadOverlayStore::replace`], a wholesale live-fetch snapshot, never
/// an edit.
#[derive(Debug, Clone, Default)]
pub struct ThreadOverlayStore {
    threads: Vec<Thread>,
}

impl ThreadOverlayStore {
    pub fn new() -> ThreadOverlayStore {
        ThreadOverlayStore::default()
    }

    /// Replaces the store's contents wholesale with a fresh fetch. A PR
    /// review's threads are always shown as a full live snapshot, never
    /// patched incrementally.
    pub fn replace(&mut self, threads: Vec<Thread>) {
        self.threads = threads;
    }

    pub fn iter(&self) -> impl Iterator<Item = &Thread> {
        self.threads.iter()
    }

    pub fn len(&self) -> usize {
        self.threads.len()
    }

    pub fn is_empty(&self) -> bool {
        self.threads.is_empty()
    }

    /// Threads anchored at `path`, in fetch order — e.g. for a gutter
    /// marker pass over one file's rendered lines.
    pub fn for_path<'a>(&'a self, path: &'a str) -> impl Iterator<Item = &'a Thread> {
        self.threads.iter().filter(move |t| t.anchor.path() == path)
    }

    /// Looks up a thread by its root id (the id a drafted reply targets).
    pub fn find(&self, id: u64) -> Option<&Thread> {
        self.threads.iter().find(|t| t.id == id)
    }
}

#[cfg(test)]
#[path = "threads_tests.rs"]
mod tests;
