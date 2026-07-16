//! `review-state.json` schema, atomic persistence, and garbage collection
//! (spec 08 Unit 4). Pure serialization/parsing plus plain filesystem I/O —
//! no TUI types, no git subprocess calls (the presentation layer,
//! `src/ui/review_ops.rs`/`main.rs`, supplies every git-derived value —
//! blob SHAs via [`crate::git::GitRunner::blob_sha`], branch existence via
//! [`crate::git::GitRunner::branch_list`] — as plain data).
//!
//! **Silent-degradation contract** (this module's documented policy, per
//! `docs/rust-best-practices.md`'s "decide deliberately what degrades
//! silently" rule): [`load`] never returns an error. A missing file behaves
//! as an empty [`ReviewStateFile`] (nothing has ever been persisted yet — an
//! entirely ordinary first run). A file that exists but fails to parse
//! (corrupt — hand-edited, truncated by a crash mid-write before atomic
//! rename was in place, etc.) is best-effort renamed aside to
//! `<path>.corrupt` so the bad bytes aren't silently discarded, a one-line
//! diagnostic is printed to **stderr** (never stdout — reserved for the
//! annotation markdown emitted on quit), and the load still proceeds as
//! empty. Every review then degrades to fully `Unreviewed` rather than the
//! session crashing or refusing to start — a lost cache is an inconvenience,
//! not a correctness problem, and the next successful save rebuilds it from
//! whatever the user re-marks. [`save`]/[`save_review`]/[`delete_review`],
//! by contrast, return typed [`StoreError`]s: a write is something the
//! caller chose to do right now and can meaningfully react to (e.g. a
//! status message), unlike a load at startup with no interactive context to
//! surface into yet.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The schema version written to every state file. Bumped whenever the
/// on-disk shape changes incompatibly; [`load`]'s corrupt-file handling
/// covers an unreadable *future* version the same way it covers hand-edited
/// garbage — both fail to deserialize into the current [`ReviewStateFile`]
/// shape and degrade to empty, per this module's silent-degradation
/// contract. There is no migration path (spec 08 doesn't need one yet): a
/// version bump only needs to happen if a later spec changes the schema in
/// a way `#[serde(default)]` fields can't absorb.
pub const SCHEMA_VERSION: u32 = 1;

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
    /// `status` is `Deferred` (deferred files carry over unconditionally —
    /// spec 08 Unit 4 — so no blob SHA is ever meaningful for them).
    /// Omitted from the JSON entirely when absent, rather than written as
    /// `null`, so a plain `Deferred` entry reads as `{"status":"deferred"}`.
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

/// Writes `state` to `path` atomically: serialize to a temp file in the
/// same directory, then `rename` over the final path (rename is atomic
/// within one filesystem, so a crash or a concurrent read never observes a
/// half-written file — spec 08 Unit 4's "a crash or `Q` loses nothing"
/// requirement). Creates `path`'s parent directory if it doesn't exist yet
/// (the first save of a session). The temp file name includes the process
/// id so two redquill processes saving concurrently (reviewing different
/// branches) never collide on the same temp path.
pub fn save(path: &Path, state: &ReviewStateFile) -> Result<(), StoreError> {
    let json = serde_json::to_string_pretty(state).map_err(StoreError::Serialize)?;
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir).map_err(StoreError::Io)?;
    let tmp_path = dir.join(format!(".review-state.json.tmp-{}", std::process::id()));
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
/// saves atomically — the read-modify-write [`super::App`] drives after
/// every status change (spec 08 Unit 4) and finish's cleanup both use. A
/// corrupt file self-heals here too (`load`'s recovery runs first, so the
/// save that follows starts from an empty file rather than propagating the
/// corruption).
pub fn save_review(path: &Path, branch: &str, review: PersistedReview) -> Result<(), StoreError> {
    let mut state = load(path);
    state.version = SCHEMA_VERSION;
    state.reviews.insert(branch.to_string(), review);
    save(path, &state)
}

/// Loads the full state file, removes `branch`'s entry (a no-op if absent),
/// and saves atomically — used by finish (spec 08 Unit 2/4) to delete a
/// completed review's persisted state.
pub fn delete_review(path: &Path, branch: &str) -> Result<(), StoreError> {
    let mut state = load(path);
    if state.reviews.remove(branch).is_none() {
        return Ok(());
    }
    save(path, &state)
}

/// Removes every entry whose branch is not in `existing_branches` (spec 08
/// Unit 4's launch-time GC — "drop entries whose branch no longer exists...
/// GC shall never touch entries for existing branches"). Pure: takes the
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
  "version": 1,
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
    }

    // -- Atomic write (task 4.1) -----------------------------------------------

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

    // -- load: corrupt file recovery (task 4.5) --------------------------------

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

    // -- gc (task 4.5) ----------------------------------------------------------

    #[test]
    fn gc_drops_entries_for_branches_that_no_longer_exist() {
        let mut state = sample_state();
        state.reviews.insert(
            "deleted-branch".to_string(),
            PersistedReview {
                base: "main".to_string(),
                worktree_path: PathBuf::from("/tmp/wt-deleted"),
                files: BTreeMap::new(),
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
