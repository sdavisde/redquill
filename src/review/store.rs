//! `review-state.json` schema, atomic persistence, and garbage collection.
//! Pure serialization/parsing plus plain filesystem I/O — no TUI types, no
//! git subprocess calls; the presentation layer supplies every git-derived
//! value (blob SHAs, branch existence) as plain data.
//!
//! **Silent-degradation contract:** [`load`] never errors. A missing file
//! is treated as an empty [`ReviewStateFile`]. A file that exists but fails
//! to parse is best-effort renamed aside to `<path>.corrupt` so the bad
//! bytes aren't silently discarded, a one-line diagnostic is printed to
//! **stderr** (never stdout — reserved for the annotation markdown emitted
//! on quit), and the load proceeds as empty either way.
//! [`save`]/[`save_review`]/[`delete_review`], by contrast, return typed
//! [`StoreError`]s: a write is something the caller chose to do right now
//! and can meaningfully react to.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The schema version written to every state file. An unreadable or
/// version-mismatched file degrades to empty per this module's
/// silent-degradation contract, the same as any other corrupt file.
///
/// v2 -> v3 added [`PersistedReview::forge`] and
/// [`PersistedReview::replies`] (both optional; absent for a plain
/// local-branch review). The migration is silent and additive: a v2 file
/// has neither key anywhere, which `#[serde(default)]` reads as `None`/empty
/// exactly like any other missing-field default in this schema, so no
/// explicit version-branching parse logic exists or is needed. A v2-era
/// review's JSON round-trips byte-identically through this schema version
/// except for the top-level `version` bump — see
/// `v2_era_review_serializes_byte_identically_except_the_version_bump`.
pub const SCHEMA_VERSION: u32 = 3;

/// A file's persisted review status. Only the two statuses a user gesture
/// can *durably* choose are represented here — `Unreviewed` (the default;
/// mirrored by simply having no entry, exactly like `App::review_states`'
/// own "missing = Unreviewed" convention) and `ChangedSinceAccepted` (always
/// *derived* at load time by [`super::reconcile::reconcile`], never itself
/// persisted — a stale `Accepted` entry reconciles back into
/// `ChangedSinceAccepted` on every load until the user re-accepts it) never
/// appear on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersistedStatus {
    Accepted,
    Deferred,
}

/// One file's persisted entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedFile {
    pub status: PersistedStatus,
    /// The file's blob SHA on the branch head at the moment of acceptance
    /// (`git rev-parse <branch>:<path>`, full SHA), used to detect staleness
    /// on the next load. `None` when the path didn't exist on the branch at
    /// acceptance time (an accepted deletion has no blob to record) or when
    /// `status` is `Deferred` (deferred files carry over unconditionally, so
    /// no blob SHA is ever meaningful for them). Omitted from the JSON
    /// entirely when absent, rather than written as `null`, so a plain
    /// `Deferred` entry reads as `{"status":"deferred"}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob_sha: Option<String>,
}

/// One branch's persisted review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedReview {
    /// The base ref this review's `base...branch` diff used.
    pub base: String,
    /// The managed worktree's path at the time of the last save.
    pub worktree_path: PathBuf,
    /// Per-path entries, keyed by repo-relative path. A [`BTreeMap`] (not a
    /// `HashMap`) so serialization order is deterministic — required for
    /// [`round_trip_is_byte_exact`]'s literal-string assertion, and a nice
    /// side effect for anyone reading the file by hand.
    #[serde(default)]
    pub files: BTreeMap<String, PersistedFile>,
    /// This review's annotations, in insertion order. The same entry
    /// annotations and file statuses live in together, so deleting or
    /// GC'ing a branch's [`PersistedReview`] ([`delete_review`]/[`gc`])
    /// removes both in one write. Snapshotted via [`crate::annotate::snapshot`]
    /// on every save-on-change and replayed via [`crate::annotate::restore_all`]
    /// at session start. Omitted from the JSON entirely when empty, mirroring
    /// [`PersistedFile::blob_sha`]'s "omit rather than write empty" convention.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<crate::annotate::PersistedAnnotation>,
    /// This review's locally drafted replies to imported forge threads, in
    /// insertion order — each a `(thread_id, body)` pair (see
    /// [`PersistedReply`]). Held separate from `annotations` because a reply
    /// answers a teammate's existing thread rather than anchoring to a diff
    /// line, and — unlike an annotation — never reaches the stdout markdown
    /// stream. Omitted from the JSON entirely when empty, matching
    /// `annotations`' convention, so a review with no drafted replies is
    /// on-disk-identical to one from before this field existed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replies: Vec<PersistedReply>,
    /// Which forge PR/MR this review targets, when it is one — `None` for a
    /// plain local-branch review (spec 08's original flow). Omitted from
    /// the JSON entirely when absent, mirroring
    /// [`PersistedFile::blob_sha`]'s "omit rather than write null"
    /// convention, so a non-forge review's on-disk shape is completely
    /// unaffected by this field's existence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forge: Option<ForgeMetadata>,
}

/// One locally drafted reply to an imported forge thread: the thread it
/// answers (its root comment id) and the reply body. Local until the review
/// is submitted; persisted so a resumed session keeps queued replies. Never
/// serialized to stdout — replies are not annotations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedReply {
    /// The root comment id of the thread this reply targets.
    pub thread_id: u64,
    /// The reply body, guaranteed non-empty after trimming by the drafting
    /// path that produced it.
    pub body: String,
}

/// A PR/MR review's forge identity and fetch bookkeeping: which provider
/// and host it lives on, its number, and the head commit SHA last fetched
/// (compared against a fresh fetch to detect the author pushing new
/// commits — see spec 13's head-move reconciliation). Carried on
/// [`PersistedReview::forge`]; entirely absent for a local-branch review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForgeMetadata {
    pub provider: ForgeProviderKind,
    /// The forge's hostname (e.g. `github.com`, or a self-managed GitLab
    /// host), verbatim from provider resolution.
    pub host: String,
    /// The PR (GitHub) or MR (GitLab) number.
    pub number: u64,
    /// The head commit SHA as of the last successful fetch, for detecting
    /// the author pushing new commits on the next open/refresh.
    pub last_head_sha: String,
}

/// Which forge a [`ForgeMetadata`] block targets. Renamed explicitly
/// (rather than `rename_all = "snake_case"`, which would split `GitHub`
/// into the odd `git_hub`) so the on-disk value reads as the provider's own
/// name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ForgeProviderKind {
    #[serde(rename = "github")]
    GitHub,
    #[serde(rename = "gitlab")]
    GitLab,
}

/// The whole `review-state.json` file: one entry per review, keyed by
/// branch name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewStateFile {
    pub version: u32,
    /// A [`BTreeMap`] for the same determinism reason as
    /// [`PersistedReview::files`].
    #[serde(default)]
    pub reviews: BTreeMap<String, PersistedReview>,
}

/// Errors produced while writing the state file. [`load`] never produces
/// one — see the module doc's silent-degradation contract.
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("failed to serialize review state: {0}")]
    Serialize(#[source] serde_json::Error),
    #[error("failed to write review state: {0}")]
    Io(#[source] std::io::Error),
}

/// The `<path>.corrupt` sibling [`load`] renames an unreadable file to.
fn corrupt_sibling_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".corrupt");
    PathBuf::from(name)
}

/// Per-process counter so concurrent saves within one process never
/// collide on the temp filename. `Relaxed` ordering is enough: the only
/// property needed is that two concurrent `fetch_add` calls never observe
/// the same value.
static SAVE_SEQ: AtomicU64 = AtomicU64::new(0);

/// Writes `state` to `path` atomically: serialize to a temp file in the
/// same directory, then `rename` over the final path, so a crash or a
/// concurrent read never observes a half-written file. Creates `path`'s
/// parent directory if it doesn't exist yet. The temp file name includes
/// both the process id (two redquill processes saving concurrently never
/// collide) and [`SAVE_SEQ`] (two background threads within the same
/// process never collide either).
pub fn save(path: &Path, state: &ReviewStateFile) -> Result<(), StoreError> {
    let json = serde_json::to_string_pretty(state).map_err(StoreError::Serialize)?;
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir).map_err(StoreError::Io)?;
    let seq = SAVE_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp_path = dir.join(format!(
        ".review-state.json.tmp-{}-{seq}",
        std::process::id()
    ));
    std::fs::write(&tmp_path, json.as_bytes()).map_err(StoreError::Io)?;
    std::fs::rename(&tmp_path, path).map_err(StoreError::Io)?;
    Ok(())
}

/// Loads the state file at `path`. Never fails — see the module doc's
/// silent-degradation contract: a missing file is `Ok`-shaped empty, and a
/// corrupt one is moved aside (best-effort — if even the rename fails, the
/// original file is left in place but still treated as empty for this run)
/// with one stderr diagnostic line, also treated as empty.
pub fn load(path: &Path) -> ReviewStateFile {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return ReviewStateFile::default(),
        Err(e) => {
            eprintln!(
                "redquill: could not read review state at {} ({e}) — starting empty",
                path.display()
            );
            return ReviewStateFile::default();
        }
    };
    match serde_json::from_slice::<ReviewStateFile>(&bytes) {
        Ok(state) => state,
        Err(parse_err) => {
            let corrupt_path = corrupt_sibling_path(path);
            match std::fs::rename(path, &corrupt_path) {
                Ok(()) => eprintln!(
                    "redquill: review state at {} is corrupt ({parse_err}); moved aside to {} — starting empty",
                    path.display(),
                    corrupt_path.display()
                ),
                Err(rename_err) => eprintln!(
                    "redquill: review state at {} is corrupt ({parse_err}); could not move it aside ({rename_err}) — starting empty",
                    path.display()
                ),
            }
            ReviewStateFile::default()
        }
    }
}

/// Loads the full state file, upserts `branch`'s entry to `review`, and
/// saves atomically — the read-modify-write pattern every status change and
/// finish's cleanup use. A corrupt file self-heals here too (`load`'s
/// recovery runs first, so the save that follows starts from an empty file
/// rather than propagating the corruption).
pub fn save_review(path: &Path, branch: &str, review: PersistedReview) -> Result<(), StoreError> {
    let mut state = load(path);
    state.version = SCHEMA_VERSION;
    state.reviews.insert(branch.to_string(), review);
    save(path, &state)
}

/// Loads the full state file, removes `branch`'s entry (a no-op if absent),
/// and saves atomically — used by finish to delete a completed review's
/// persisted state.
pub fn delete_review(path: &Path, branch: &str) -> Result<(), StoreError> {
    let mut state = load(path);
    if state.reviews.remove(branch).is_none() {
        return Ok(());
    }
    save(path, &state)
}

/// Removes every entry whose branch is not in `existing_branches`, and
/// never touches an entry whose branch still exists. Pure: takes the
/// branch set as plain data rather than reading git itself, so it's
/// unit-testable without a repository; the caller resolves
/// `existing_branches` via [`crate::git::GitRunner::branch_list`]. Returns
/// whether anything was removed, so the caller can skip an unnecessary save
/// when GC was a no-op.
pub fn gc(state: &mut ReviewStateFile, existing_branches: &HashSet<String>) -> bool {
    let before = state.reviews.len();
    state
        .reviews
        .retain(|branch, _| existing_branches.contains(branch));
    state.reviews.len() != before
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_state() -> ReviewStateFile {
        let mut files = BTreeMap::new();
        files.insert(
            "a.rs".to_string(),
            PersistedFile {
                status: PersistedStatus::Accepted,
                blob_sha: Some("abc123def456".to_string()),
            },
        );
        files.insert(
            "b.rs".to_string(),
            PersistedFile {
                status: PersistedStatus::Deferred,
                blob_sha: None,
            },
        );
        let mut reviews = BTreeMap::new();
        reviews.insert(
            "feature".to_string(),
            PersistedReview {
                base: "main".to_string(),
                worktree_path: PathBuf::from("/tmp/redquill/worktrees/feature-1234"),
                files,
                annotations: Vec::new(),
                replies: Vec::new(),
                forge: None,
            },
        );
        ReviewStateFile {
            version: SCHEMA_VERSION,
            reviews,
        }
    }

    // -- Schema round-trip (TDD, byte-exact) ----------------------------------

    #[test]
    fn round_trip_is_byte_exact() {
        let state = sample_state();
        let json = serde_json::to_string_pretty(&state).unwrap();
        let expected = r#"{
  "version": 3,
  "reviews": {
    "feature": {
      "base": "main",
      "worktree_path": "/tmp/redquill/worktrees/feature-1234",
      "files": {
        "a.rs": {
          "status": "accepted",
          "blob_sha": "abc123def456"
        },
        "b.rs": {
          "status": "deferred"
        }
      }
    }
  }
}"#;
        assert_eq!(json, expected);

        let round_tripped: ReviewStateFile = serde_json::from_str(&json).unwrap();
        assert_eq!(round_tripped, state);
    }

    #[test]
    fn empty_state_round_trips() {
        let state = ReviewStateFile {
            version: SCHEMA_VERSION,
            reviews: BTreeMap::new(),
        };
        let json = serde_json::to_string_pretty(&state).unwrap();
        let round_tripped: ReviewStateFile = serde_json::from_str(&json).unwrap();
        assert_eq!(round_tripped, state);
    }

    #[test]
    fn missing_optional_fields_default_on_load() {
        // A hand-written minimal file (no `files` map at all for the
        // review, and a deferred entry with no `blob_sha` key) must still
        // parse — `#[serde(default)]` absorbing an older or hand-trimmed
        // file.
        let json = r#"{
            "version": 1,
            "reviews": {
                "feature": {
                    "base": "main",
                    "worktree_path": "/tmp/wt"
                }
            }
        }"#;
        let state: ReviewStateFile = serde_json::from_str(json).unwrap();
        let review = state.reviews.get("feature").unwrap();
        assert!(review.files.is_empty());
        assert!(review.annotations.is_empty());
    }

    // -- Schema v2: annotations field -------------------------------------------

    /// A v1 file (`"version": 1`, no `annotations` key anywhere) must load
    /// as a normal, non-corrupt review with an empty annotation list, going
    /// through the real [`load`] entry point (not just a bare
    /// `serde_json::from_str`) so the corrupt-file side path is proven
    /// *not* taken.
    #[test]
    fn v1_file_without_annotations_loads_as_empty_not_corrupt() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("review-state.json");
        let v1_json = r#"{
  "version": 1,
  "reviews": {
    "feature": {
      "base": "main",
      "worktree_path": "/tmp/redquill/worktrees/feature-1234",
      "files": {
        "a.rs": {
          "status": "accepted",
          "blob_sha": "abc123def456"
        }
      }
    }
  }
}"#;
        std::fs::write(&path, v1_json).unwrap();

        let state = load(&path);

        assert!(
            !tmp.path().join("review-state.json.corrupt").exists(),
            "a v1 file must never be moved aside as corrupt"
        );
        let review = state.reviews.get("feature").expect("v1 entry must load");
        assert_eq!(review.files.len(), 1);
        assert!(
            review.annotations.is_empty(),
            "a v1 file has no annotations key; it must default to empty, not fail to parse"
        );
    }

    /// A v1 file's own `version: 1` survives the read verbatim (`load`
    /// never rewrites it in place) — the *next* save is what upgrades it on
    /// disk, via [`save_review`]'s `state.version = SCHEMA_VERSION`
    /// assignment.
    #[test]
    fn v1_file_upgrades_to_v2_on_the_next_save() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("review-state.json");
        std::fs::write(
            &path,
            r#"{"version":1,"reviews":{"feature":{"base":"main","worktree_path":"/tmp/wt","files":{}}}}"#,
        )
        .unwrap();

        save_review(
            &path,
            "feature",
            PersistedReview {
                base: "main".to_string(),
                worktree_path: PathBuf::from("/tmp/wt"),
                files: BTreeMap::new(),
                annotations: Vec::new(),
                replies: Vec::new(),
                forge: None,
            },
        )
        .unwrap();

        let state = load(&path);
        assert_eq!(state.version, SCHEMA_VERSION);
    }

    /// Byte-exact round-trip for the `annotations` field, locking the exact
    /// shape a review's saved annotations take inside `review-state.json` —
    /// the array sits directly under the review entry, each element in
    /// [`crate::annotate::PersistedAnnotation`]'s own stable shape.
    #[test]
    fn annotations_field_round_trips_byte_exact() {
        use crate::annotate::{Classification, PersistedAnnotation, Side, Source, Target};

        let mut state = ReviewStateFile {
            version: SCHEMA_VERSION,
            reviews: BTreeMap::new(),
        };
        state.reviews.insert(
            "feature".to_string(),
            PersistedReview {
                base: "main".to_string(),
                worktree_path: PathBuf::from("/tmp/wt"),
                files: BTreeMap::new(),
                annotations: vec![PersistedAnnotation {
                    target: Target::line("src/lib.rs", 10, Side::New),
                    classification: Classification::Issue,
                    body: "fix this".to_string(),
                    source: Source::WorkingTree,
                }],
                replies: Vec::new(),
                forge: None,
            },
        );

        let json = serde_json::to_string_pretty(&state).unwrap();
        let expected = r#"{
  "version": 3,
  "reviews": {
    "feature": {
      "base": "main",
      "worktree_path": "/tmp/wt",
      "files": {},
      "annotations": [
        {
          "target": {
            "kind": "line",
            "path": "src/lib.rs",
            "line": 10,
            "side": "new"
          },
          "classification": "issue",
          "body": "fix this",
          "source": "working_tree"
        }
      ]
    }
  }
}"#;
        assert_eq!(json, expected);

        let round_tripped: ReviewStateFile = serde_json::from_str(&json).unwrap();
        assert_eq!(round_tripped, state);
    }

    /// An empty `files` map still serializes as `"files": {}` (no
    /// `skip_serializing_if` on that field) while an empty `annotations`
    /// list is omitted entirely — the two fields deliberately have
    /// different emptiness conventions. This test exists so a future edit
    /// that "fixes" one to match the other fails loudly instead of silently
    /// changing the on-disk shape.
    #[test]
    fn empty_annotations_are_omitted_but_empty_files_still_serialize_as_object() {
        let review = PersistedReview {
            base: "main".to_string(),
            worktree_path: PathBuf::from("/tmp/wt"),
            files: BTreeMap::new(),
            annotations: Vec::new(),
            replies: Vec::new(),
            forge: None,
        };
        let json = serde_json::to_string(&review).unwrap();
        assert!(json.contains("\"files\":{}"));
        assert!(!json.contains("annotations"));
        assert!(!json.contains("replies"));
        assert!(!json.contains("forge"));
    }

    // -- Schema v3: forge field -------------------------------------------------

    #[test]
    fn schema_version_is_3() {
        assert_eq!(SCHEMA_VERSION, 3);
    }

    /// Byte-exact round-trip for the `forge` field on a PR/MR review,
    /// locking its exact shape: a `provider`/`host`/`number`/`last_head_sha`
    /// object nested directly under the review entry, alongside (not
    /// replacing) `base`/`worktree_path`/`files`/`annotations`.
    #[test]
    fn forge_block_round_trips_byte_exact() {
        let mut state = ReviewStateFile {
            version: SCHEMA_VERSION,
            reviews: BTreeMap::new(),
        };
        state.reviews.insert(
            "redquill/pr/42".to_string(),
            PersistedReview {
                base: "main".to_string(),
                worktree_path: PathBuf::from("/tmp/wt"),
                files: BTreeMap::new(),
                annotations: Vec::new(),
                replies: Vec::new(),
                forge: Some(ForgeMetadata {
                    provider: ForgeProviderKind::GitHub,
                    host: "github.com".to_string(),
                    number: 42,
                    last_head_sha: "abc123def456".to_string(),
                }),
            },
        );

        let json = serde_json::to_string_pretty(&state).unwrap();
        let expected = r#"{
  "version": 3,
  "reviews": {
    "redquill/pr/42": {
      "base": "main",
      "worktree_path": "/tmp/wt",
      "files": {},
      "forge": {
        "provider": "github",
        "host": "github.com",
        "number": 42,
        "last_head_sha": "abc123def456"
      }
    }
  }
}"#;
        assert_eq!(json, expected);

        let round_tripped: ReviewStateFile = serde_json::from_str(&json).unwrap();
        assert_eq!(round_tripped, state);
    }

    // -- Schema v3: replies field -----------------------------------------------

    /// Byte-exact round-trip for the `replies` field on a forge review,
    /// locking its exact shape: a `[{thread_id, body}, ...]` array nested
    /// directly under the review entry, positioned between `annotations` and
    /// `forge`, each element a bare thread-id/body pair.
    #[test]
    fn replies_field_round_trips_byte_exact() {
        let mut state = ReviewStateFile {
            version: SCHEMA_VERSION,
            reviews: BTreeMap::new(),
        };
        state.reviews.insert(
            "redquill/pr/7".to_string(),
            PersistedReview {
                base: "main".to_string(),
                worktree_path: PathBuf::from("/tmp/wt"),
                files: BTreeMap::new(),
                annotations: Vec::new(),
                replies: vec![
                    PersistedReply {
                        thread_id: 100,
                        body: "agreed, will fix".to_string(),
                    },
                    PersistedReply {
                        thread_id: 200,
                        body: "why not use the helper here?".to_string(),
                    },
                ],
                forge: None,
            },
        );

        let json = serde_json::to_string_pretty(&state).unwrap();
        let expected = r#"{
  "version": 3,
  "reviews": {
    "redquill/pr/7": {
      "base": "main",
      "worktree_path": "/tmp/wt",
      "files": {},
      "replies": [
        {
          "thread_id": 100,
          "body": "agreed, will fix"
        },
        {
          "thread_id": 200,
          "body": "why not use the helper here?"
        }
      ]
    }
  }
}"#;
        assert_eq!(json, expected);

        let round_tripped: ReviewStateFile = serde_json::from_str(&json).unwrap();
        assert_eq!(round_tripped, state);
    }

    /// A v3 review that carries annotations but no replies loads with an
    /// empty `replies` list and no `replies` key on re-serialization — the
    /// "absent replies stays absent" counterpart to the annotations/forge
    /// omission tests, proving the new field never leaks into a review that
    /// never drafted a reply.
    #[test]
    fn v3_review_without_replies_loads_and_reserializes_clean() {
        use crate::annotate::{Classification, PersistedAnnotation, Side, Source, Target};

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("review-state.json");
        let json = r#"{
  "version": 3,
  "reviews": {
    "feature": {
      "base": "main",
      "worktree_path": "/tmp/wt",
      "files": {},
      "annotations": [
        {
          "target": {
            "kind": "line",
            "path": "src/lib.rs",
            "line": 10,
            "side": "new"
          },
          "classification": "issue",
          "body": "fix this",
          "source": "working_tree"
        }
      ]
    }
  }
}"#;
        std::fs::write(&path, json).unwrap();

        let state = load(&path);
        assert!(
            !tmp.path().join("review-state.json.corrupt").exists(),
            "a v3 review without replies must never be moved aside as corrupt"
        );
        let review = state.reviews.get("feature").expect("entry must load");
        assert!(
            review.replies.is_empty(),
            "a review with no replies key must default to an empty list"
        );
        assert_eq!(
            review.annotations,
            vec![PersistedAnnotation {
                target: Target::line("src/lib.rs", 10, Side::New),
                classification: Classification::Issue,
                body: "fix this".to_string(),
                source: Source::WorkingTree,
            }]
        );

        let reserialized = serde_json::to_string_pretty(&state).unwrap();
        assert_eq!(
            reserialized, json,
            "a review with no replies must re-serialize byte-identically — no replies key appears"
        );
    }

    /// A v2 file (`"version": 2`, no `forge` key anywhere) must load as a
    /// normal, non-corrupt review with an absent `forge` field, going
    /// through the real [`load`] entry point (not just a bare
    /// `serde_json::from_str`) so the corrupt-file side path is proven
    /// *not* taken — the v2 -> v3 counterpart of
    /// `v1_file_without_annotations_loads_as_empty_not_corrupt`.
    #[test]
    fn v2_file_without_forge_loads_silently_as_v3_with_absent_forge_field() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("review-state.json");
        let v2_json = r#"{
  "version": 2,
  "reviews": {
    "feature": {
      "base": "main",
      "worktree_path": "/tmp/redquill/worktrees/feature-1234",
      "files": {
        "a.rs": {
          "status": "accepted",
          "blob_sha": "abc123def456"
        }
      }
    }
  }
}"#;
        std::fs::write(&path, v2_json).unwrap();

        let state = load(&path);

        assert!(
            !tmp.path().join("review-state.json.corrupt").exists(),
            "a v2 file must never be moved aside as corrupt"
        );
        let review = state.reviews.get("feature").expect("v2 entry must load");
        assert_eq!(review.files.len(), 1);
        assert_eq!(
            review.forge, None,
            "a v2 file has no forge key; it must default to absent, not fail to parse"
        );
    }

    /// A v2 file's own `version: 2` survives the read verbatim (`load`
    /// never rewrites it in place) — the *next* save is what upgrades it on
    /// disk, mirroring `v1_file_upgrades_to_v2_on_the_next_save`.
    #[test]
    fn v2_file_upgrades_to_v3_on_the_next_save() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("review-state.json");
        std::fs::write(
            &path,
            r#"{"version":2,"reviews":{"feature":{"base":"main","worktree_path":"/tmp/wt","files":{}}}}"#,
        )
        .unwrap();

        save_review(
            &path,
            "feature",
            PersistedReview {
                base: "main".to_string(),
                worktree_path: PathBuf::from("/tmp/wt"),
                files: BTreeMap::new(),
                annotations: Vec::new(),
                replies: Vec::new(),
                forge: None,
            },
        )
        .unwrap();

        let state = load(&path);
        assert_eq!(state.version, SCHEMA_VERSION);
    }

    /// The byte-stability proof the spec calls for: a v2-era review's exact
    /// on-disk JSON, read through the real [`load`] entry point and
    /// re-serialized, must come back byte-identical except for the
    /// top-level `version` field bumping from 2 to 3. Nothing about a
    /// non-forge review's shape changes — no `forge` key appears, no field
    /// reorders, no whitespace shifts.
    ///
    /// [`load`] itself never rewrites the version in place (see
    /// `v2_file_upgrades_to_v3_on_the_next_save` and its v1 precedent) — the
    /// version bump only happens on the next write, exactly like
    /// [`save_review`]'s `state.version = SCHEMA_VERSION` assignment, which
    /// this test reproduces directly rather than going through a save.
    #[test]
    fn v2_era_review_serializes_byte_identically_except_the_version_bump() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("review-state.json");
        let v2_json = r#"{
  "version": 2,
  "reviews": {
    "feature": {
      "base": "main",
      "worktree_path": "/tmp/redquill/worktrees/feature-1234",
      "files": {
        "a.rs": {
          "status": "accepted",
          "blob_sha": "abc123def456"
        },
        "b.rs": {
          "status": "deferred"
        }
      }
    }
  }
}"#;
        std::fs::write(&path, v2_json).unwrap();

        let mut state = load(&path);
        state.version = SCHEMA_VERSION;
        let reserialized = serde_json::to_string_pretty(&state).unwrap();

        let expected = v2_json.replacen("\"version\": 2", "\"version\": 3", 1);
        assert_eq!(reserialized, expected);
    }

    // -- Atomic write -------------------------------------------------------

    #[test]
    fn save_then_load_round_trips_through_disk() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("review-state.json");
        let state = sample_state();
        save(&path, &state).unwrap();
        let loaded = load(&path);
        assert_eq!(loaded, state);
    }

    #[test]
    fn save_creates_missing_parent_directories() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("redquill").join("review-state.json");
        assert!(!path.parent().unwrap().exists());
        save(&path, &sample_state()).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn save_leaves_no_temp_file_behind() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("review-state.json");
        save(&path, &sample_state()).unwrap();
        let entries: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(
            entries,
            vec![std::ffi::OsString::from("review-state.json")],
            "only the final file must remain, no temp leftovers"
        );
    }

    #[test]
    fn save_overwrites_the_previous_content_wholesale() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("review-state.json");
        save(&path, &sample_state()).unwrap();
        let empty = ReviewStateFile {
            version: SCHEMA_VERSION,
            reviews: BTreeMap::new(),
        };
        save(&path, &empty).unwrap();
        assert_eq!(load(&path), empty);
    }

    // -- load: missing file behaves as empty -----------------------------------

    #[test]
    fn load_missing_file_is_empty() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("review-state.json");
        assert_eq!(load(&path), ReviewStateFile::default());
    }

    // -- load: corrupt file recovery ----------------------------------------

    #[test]
    fn load_corrupt_file_is_moved_aside_and_treated_as_empty() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("review-state.json");
        std::fs::write(&path, b"not valid json {{{").unwrap();

        let state = load(&path);

        assert_eq!(state, ReviewStateFile::default());
        assert!(!path.exists(), "the corrupt file must be moved aside");
        let corrupt_path = tmp.path().join("review-state.json.corrupt");
        assert!(corrupt_path.exists(), "a .corrupt sibling must exist");
        assert_eq!(
            std::fs::read(&corrupt_path).unwrap(),
            b"not valid json {{{",
            "the original bytes must survive the move"
        );
    }

    #[test]
    fn load_corrupt_file_self_heals_on_the_next_save() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("review-state.json");
        std::fs::write(&path, b"garbage").unwrap();
        let _ = load(&path); // moves the corrupt file aside

        save_review(
            &path,
            "feature",
            PersistedReview {
                base: "main".to_string(),
                worktree_path: PathBuf::from("/tmp/wt"),
                files: BTreeMap::new(),
                annotations: Vec::new(),
                replies: Vec::new(),
                forge: None,
            },
        )
        .unwrap();

        let state = load(&path);
        assert_eq!(state.reviews.len(), 1);
        assert!(tmp.path().join("review-state.json.corrupt").exists());
    }

    // -- save_review / delete_review (read-modify-write) -----------------------

    #[test]
    fn save_review_upserts_one_branch_without_disturbing_others() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("review-state.json");
        save(&path, &sample_state()).unwrap(); // has "feature"

        save_review(
            &path,
            "other-branch",
            PersistedReview {
                base: "main".to_string(),
                worktree_path: PathBuf::from("/tmp/wt2"),
                files: BTreeMap::new(),
                annotations: Vec::new(),
                replies: Vec::new(),
                forge: None,
            },
        )
        .unwrap();

        let state = load(&path);
        assert_eq!(state.reviews.len(), 2);
        assert!(state.reviews.contains_key("feature"));
        assert!(state.reviews.contains_key("other-branch"));
    }

    #[test]
    fn save_review_replaces_an_existing_branchs_entry_wholesale() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("review-state.json");
        save(&path, &sample_state()).unwrap();

        save_review(
            &path,
            "feature",
            PersistedReview {
                base: "main".to_string(),
                worktree_path: PathBuf::from("/tmp/wt"),
                files: BTreeMap::new(),
                annotations: Vec::new(),
                replies: Vec::new(),
                forge: None,
            },
        )
        .unwrap();

        let state = load(&path);
        assert!(state.reviews.get("feature").unwrap().files.is_empty());
    }

    #[test]
    fn delete_review_removes_only_the_named_branch() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("review-state.json");
        let mut state = sample_state();
        state.reviews.insert(
            "other".to_string(),
            PersistedReview {
                base: "main".to_string(),
                worktree_path: PathBuf::from("/tmp/wt2"),
                files: BTreeMap::new(),
                annotations: Vec::new(),
                replies: Vec::new(),
                forge: None,
            },
        );
        save(&path, &state).unwrap();

        delete_review(&path, "feature").unwrap();

        let loaded = load(&path);
        assert!(!loaded.reviews.contains_key("feature"));
        assert!(loaded.reviews.contains_key("other"));
    }

    #[test]
    fn delete_review_of_an_absent_branch_is_a_no_op() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("review-state.json");
        save(&path, &sample_state()).unwrap();

        delete_review(&path, "no-such-branch").unwrap();

        assert_eq!(load(&path), sample_state());
    }

    #[test]
    fn delete_review_on_a_missing_file_is_a_no_op_and_creates_nothing() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("review-state.json");
        delete_review(&path, "feature").unwrap();
        assert!(!path.exists());
    }

    // -- gc -------------------------------------------------------------------

    #[test]
    fn gc_drops_entries_for_branches_that_no_longer_exist() {
        let mut state = sample_state();
        state.reviews.insert(
            "deleted-branch".to_string(),
            PersistedReview {
                base: "main".to_string(),
                worktree_path: PathBuf::from("/tmp/wt-deleted"),
                files: BTreeMap::new(),
                annotations: Vec::new(),
                replies: Vec::new(),
                forge: None,
            },
        );
        let existing: HashSet<String> = ["feature".to_string()].into_iter().collect();

        let changed = gc(&mut state, &existing);

        assert!(changed);
        assert!(state.reviews.contains_key("feature"));
        assert!(!state.reviews.contains_key("deleted-branch"));
    }

    #[test]
    fn gc_never_touches_entries_for_existing_branches() {
        let mut state = sample_state();
        let existing: HashSet<String> = ["feature".to_string()].into_iter().collect();
        let before = state.clone();

        let changed = gc(&mut state, &existing);

        assert!(!changed);
        assert_eq!(state, before);
    }

    #[test]
    fn gc_of_an_empty_state_is_a_no_op() {
        let mut state = ReviewStateFile::default();
        let changed = gc(&mut state, &HashSet::new());
        assert!(!changed);
        assert!(state.reviews.is_empty());
    }
}
