//! Finished-review detection: the pure set-difference that tells the Pull
//! Requests tab which managed `redquill/pr/*` reviews belong to PRs that are
//! no longer open, so they can be cleaned up. Pure data in, pure data out —
//! no git calls, no filesystem, no TUI types; the presentation layer supplies
//! the managed-branch list, the persisted reviews, and the open-PR number set
//! (the latter already fetched by the tab's list call, so this adds no network
//! round-trip — see spec 13 FR-22).

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use super::store::{ForgeProviderKind, PersistedReview};

/// One managed PR/MR review whose PR is no longer open — a cleanup candidate.
/// Carries everything the confirm modal needs to describe it and everything
/// the deletion sequence needs to act on it, so neither has to re-derive
/// anything from git or the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishedReview {
    /// The managed branch short name (`redquill/pr/<n>`) — the deletion
    /// sequence's key into both `git branch -D` and the state store.
    pub branch: String,
    /// The PR/MR number, from the review's persisted forge block.
    pub number: u64,
    /// The PR/MR title as last checked out (may be empty for a review
    /// persisted before the title was retained).
    pub title: String,
    /// Which forge this review lives on.
    pub provider: ForgeProviderKind,
    /// The forge hostname.
    pub host: String,
    /// The managed worktree's path, for `git worktree remove`.
    pub worktree_path: PathBuf,
    /// How many of this review's annotations and drafted replies are still
    /// unpublished — surfaced as an explicit warning in the confirm modal
    /// when nonzero, so a reviewer never silently discards un-submitted work.
    pub unpublished_count: usize,
}

/// The managed reviews whose PR is no longer open, in the order their managed
/// branches were given (`for-each-ref`'s order at the call site).
///
/// A managed branch is a finished-review candidate only when it has a
/// persisted state entry carrying a forge block whose PR number is absent from
/// `open_pr_numbers`. A managed branch with no state entry (never reviewed
/// locally, or its state was lost) is excluded — there is nothing to clean up
/// but a bare branch, and the spec scopes this to reviews. A managed branch
/// whose PR is still open is likewise excluded.
pub fn finished_reviews(
    managed_branches: &[String],
    reviews: &BTreeMap<String, PersistedReview>,
    open_pr_numbers: &HashSet<u64>,
) -> Vec<FinishedReview> {
    managed_branches
        .iter()
        .filter_map(|branch| {
            let review = reviews.get(branch)?;
            let forge = review.forge.as_ref()?;
            if open_pr_numbers.contains(&forge.number) {
                return None;
            }
            Some(FinishedReview {
                branch: branch.clone(),
                number: forge.number,
                title: forge.title.clone(),
                provider: forge.provider,
                host: forge.host.clone(),
                worktree_path: review.worktree_path.clone(),
                unpublished_count: unpublished_count(review),
            })
        })
        .collect()
}

/// Counts a review's still-unpublished annotations and drafted replies — the
/// work a cleanup would discard, warned about in the confirm modal.
fn unpublished_count(review: &PersistedReview) -> usize {
    let annotations = review.annotations.iter().filter(|a| !a.published).count();
    let replies = review.replies.iter().filter(|r| !r.published).count();
    annotations + replies
}

#[cfg(test)]
#[path = "cleanup_tests.rs"]
mod tests;
