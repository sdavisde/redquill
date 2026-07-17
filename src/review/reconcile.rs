//! Pure blob-SHA reconciliation (spec 08 Unit 4): turns a persisted
//! review's per-file entries into the in-memory [`ReviewStatus`] map
//! `App::review_states` uses, given the branch's *current* blob SHA for
//! each persisted path. Kept pure and git-free — the caller resolves each
//! path's current blob SHA (via [`crate::git::GitRunner::blob_sha`]) and
//! hands the results in as plain data — so the reconciliation rule itself
//! is unit-testable without a repository.

use std::collections::HashMap;

use super::ReviewStatus;
use super::store::{PersistedReview, PersistedStatus};

/// Reconciles `persisted`'s per-file entries against `current_blob_shas`
/// (one entry per persisted path; `None` means the path no longer exists on
/// the branch). The rule (spec 08 Unit 4 FR):
///
/// - `Deferred` carries over unconditionally — no staleness check ever
///   applies to a deferred file.
/// - `Accepted` stays `Accepted` when the persisted `blob_sha` still equals
///   the current one (including the "both `None`" case: a file accepted
///   while already deleted from the branch, still deleted now); any
///   mismatch — a real content change, the file coming back after being
///   deleted, or a path `current_blob_shas` has no entry for at all —
///   demotes it to `ChangedSinceAccepted`.
/// - A path with no persisted entry gets no entry in the returned map at
///   all, exactly mirroring `App::review_status`'s "missing = Unreviewed"
///   convention — "files new on the branch since last session are
///   Unreviewed" (spec 08 Unit 4) needs no special-casing here, it falls
///   out of simply never being visited.
pub fn reconcile(
    persisted: &PersistedReview,
    current_blob_shas: &HashMap<String, Option<String>>,
) -> HashMap<String, ReviewStatus> {
    persisted
        .files
        .iter()
        .map(|(path, entry)| {
            let status = match entry.status {
                PersistedStatus::Deferred => ReviewStatus::Deferred,
                PersistedStatus::Accepted => {
                    // A missing key (the caller couldn't resolve a current
                    // blob SHA for this path at all) collapses to `None`,
                    // the same as an explicit "path doesn't exist" answer —
                    // both are treated as a mismatch unless the persisted
                    // entry was *also* `None`.
                    let current = current_blob_shas.get(path).cloned().flatten();
                    if current == entry.blob_sha {
                        ReviewStatus::Accepted
                    } else {
                        ReviewStatus::ChangedSinceAccepted
                    }
                }
            };
            (path.clone(), status)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::store::PersistedFile;
    use std::path::PathBuf;

    fn review_with(files: Vec<(&str, PersistedStatus, Option<&str>)>) -> PersistedReview {
        let files = files
            .into_iter()
            .map(|(path, status, sha)| {
                (
                    path.to_string(),
                    PersistedFile {
                        status,
                        blob_sha: sha.map(str::to_string),
                    },
                )
            })
            .collect();
        PersistedReview {
            base: "main".to_string(),
            worktree_path: PathBuf::from("/tmp/wt"),
            files,
            annotations: Vec::new(),
        }
    }

    #[test]
    fn accepted_with_matching_sha_stays_accepted() {
        let persisted = review_with(vec![("a.rs", PersistedStatus::Accepted, Some("sha1"))]);
        let current: HashMap<String, Option<String>> =
            [("a.rs".to_string(), Some("sha1".to_string()))].into();
        let result = reconcile(&persisted, &current);
        assert_eq!(result.get("a.rs"), Some(&ReviewStatus::Accepted));
    }

    #[test]
    fn accepted_with_mismatched_sha_becomes_changed_since_accepted() {
        let persisted = review_with(vec![("a.rs", PersistedStatus::Accepted, Some("sha1"))]);
        let current: HashMap<String, Option<String>> =
            [("a.rs".to_string(), Some("sha2".to_string()))].into();
        let result = reconcile(&persisted, &current);
        assert_eq!(
            result.get("a.rs"),
            Some(&ReviewStatus::ChangedSinceAccepted)
        );
    }

    #[test]
    fn accepted_deletion_that_stays_deleted_remains_accepted() {
        // Accepted while already absent from the branch (`blob_sha: None`),
        // and it's still absent now — both sides `None`, no staleness.
        let persisted = review_with(vec![("a.rs", PersistedStatus::Accepted, None)]);
        let current: HashMap<String, Option<String>> = [("a.rs".to_string(), None)].into();
        let result = reconcile(&persisted, &current);
        assert_eq!(result.get("a.rs"), Some(&ReviewStatus::Accepted));
    }

    #[test]
    fn accepted_deletion_that_reappears_becomes_changed_since_accepted() {
        let persisted = review_with(vec![("a.rs", PersistedStatus::Accepted, None)]);
        let current: HashMap<String, Option<String>> =
            [("a.rs".to_string(), Some("sha-new".to_string()))].into();
        let result = reconcile(&persisted, &current);
        assert_eq!(
            result.get("a.rs"),
            Some(&ReviewStatus::ChangedSinceAccepted)
        );
    }

    #[test]
    fn accepted_file_that_becomes_deleted_becomes_changed_since_accepted() {
        let persisted = review_with(vec![("a.rs", PersistedStatus::Accepted, Some("sha1"))]);
        let current: HashMap<String, Option<String>> = [("a.rs".to_string(), None)].into();
        let result = reconcile(&persisted, &current);
        assert_eq!(
            result.get("a.rs"),
            Some(&ReviewStatus::ChangedSinceAccepted)
        );
    }

    #[test]
    fn accepted_path_missing_from_current_shas_entirely_is_treated_as_a_mismatch() {
        let persisted = review_with(vec![("a.rs", PersistedStatus::Accepted, Some("sha1"))]);
        let current: HashMap<String, Option<String>> = HashMap::new();
        let result = reconcile(&persisted, &current);
        assert_eq!(
            result.get("a.rs"),
            Some(&ReviewStatus::ChangedSinceAccepted)
        );
    }

    #[test]
    fn deferred_carries_over_regardless_of_blob_sha_state() {
        let persisted = review_with(vec![("a.rs", PersistedStatus::Deferred, None)]);
        // Even a mismatching (or entirely absent) current SHA changes nothing.
        let current: HashMap<String, Option<String>> =
            [("a.rs".to_string(), Some("whatever".to_string()))].into();
        let result = reconcile(&persisted, &current);
        assert_eq!(result.get("a.rs"), Some(&ReviewStatus::Deferred));
    }

    #[test]
    fn a_path_with_no_persisted_entry_is_absent_from_the_result() {
        let persisted = review_with(vec![("a.rs", PersistedStatus::Accepted, Some("sha1"))]);
        let current: HashMap<String, Option<String>> =
            [("a.rs".to_string(), Some("sha1".to_string()))].into();
        let result = reconcile(&persisted, &current);
        assert_eq!(
            result.get("b.rs"),
            None,
            "no entry means Unreviewed by default lookup"
        );
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn multiple_files_reconcile_independently() {
        let persisted = review_with(vec![
            ("a.rs", PersistedStatus::Accepted, Some("sha1")),
            ("b.rs", PersistedStatus::Accepted, Some("sha2")),
            ("c.rs", PersistedStatus::Deferred, None),
        ]);
        let current: HashMap<String, Option<String>> = [
            ("a.rs".to_string(), Some("sha1".to_string())), // unchanged
            ("b.rs".to_string(), Some("sha2-new".to_string())), // changed
            ("c.rs".to_string(), Some("irrelevant".to_string())),
        ]
        .into();
        let result = reconcile(&persisted, &current);
        assert_eq!(result.get("a.rs"), Some(&ReviewStatus::Accepted));
        assert_eq!(
            result.get("b.rs"),
            Some(&ReviewStatus::ChangedSinceAccepted)
        );
        assert_eq!(result.get("c.rs"), Some(&ReviewStatus::Deferred));
    }

    #[test]
    fn empty_persisted_review_reconciles_to_an_empty_map() {
        let persisted = review_with(vec![]);
        let result = reconcile(&persisted, &HashMap::new());
        assert!(result.is_empty());
    }
}
