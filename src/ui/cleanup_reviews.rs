//! The finished-review cleanup modal's state transitions
//! ([`super::app::Mode::CleanupReviews`]): opening it from the Pull Requests
//! tab, cancelling back into the launcher, and — on confirm — deleting each
//! finished review's managed worktree, branch, and persisted state entry.
//!
//! Modeled on [`super::end_review`]'s finish path, which this mirrors for the
//! single-review case: the deletion runs synchronously on the render thread
//! (worktree removal is a bounded, user-confirmed action, never a hot path),
//! removes the managed worktree through the current backend (rooted at the
//! origin repo while the launcher is open, i.e. outside every managed
//! worktree), prunes, deletes the `redquill/pr/<n>` branch through the
//! prefix-confined helper, and deletes the review's state entry — all
//! per-entry, so one dirty or locked worktree fails just that entry and the
//! run continues to the next, ending in a one-line outcome summary.

use crate::review::FinishedReview;

use super::app::{App, Mode, ModeOrigin};
use super::review_launcher::LauncherTab;
use super::review_session::resolve_review_state_path;

impl App {
    /// Opens the cleanup confirm modal for the Pull Requests tab's finished
    /// reviews. A PRs-tab-only gesture: a no-op unless the launcher is on that
    /// tab. Degrades to a status line — never opens an empty modal — when
    /// there are no finished reviews, and refuses while a remote op or a PR
    /// checkout is in flight (deletion mutates the same worktrees/branches
    /// those touch). On success it snapshots the finished set into
    /// [`App::cleanup_reviews`] (frozen so a background list refresh can't
    /// shift the rows mid-confirmation) and switches to
    /// [`Mode::CleanupReviews`], carrying the launcher's own origin so
    /// cancel/confirm can reopen it exactly.
    pub(super) fn open_cleanup_reviews(&mut self) {
        let Mode::ReviewLauncher {
            tab: LauncherTab::PullRequests,
            origin,
            ..
        } = self.mode
        else {
            return;
        };
        if self.launcher_finished_reviews.is_empty() {
            self.set_status_message("no finished reviews to clean up");
            return;
        }
        if let Some(label) = self.running_op_label() {
            self.set_status_message(format!(
                "{label} is running \u{2014} wait before cleaning up"
            ));
            return;
        }
        if self.pr_checkout_in_flight.is_some() {
            self.set_status_message("a PR checkout is running \u{2014} wait before cleaning up");
            return;
        }
        self.cleanup_reviews = self.launcher_finished_reviews.clone();
        self.mode = Mode::CleanupReviews { origin };
    }

    /// Closes the cleanup modal without deleting anything, reopening the
    /// launcher on the Pull Requests tab exactly where cleanup was invoked
    /// from. Declining mutates nothing on disk. A no-op outside
    /// [`Mode::CleanupReviews`].
    pub(super) fn cancel_cleanup_reviews(&mut self) {
        let Mode::CleanupReviews { origin } = self.mode else {
            return;
        };
        self.cleanup_reviews.clear();
        self.reopen_launcher_after_cleanup(origin);
    }

    /// Confirms the cleanup: deletes every enumerated finished review's
    /// worktree, branch, and state entry (see [`App::run_cleanup_deletions`]),
    /// recomputes the finished set from the still-current listing (a cleanup
    /// never changes which PRs are open, so no re-fetch is needed), reopens
    /// the launcher, and surfaces the per-entry outcome summary. A no-op
    /// outside [`Mode::CleanupReviews`].
    pub(super) fn confirm_cleanup_reviews(&mut self) {
        let Mode::CleanupReviews { origin } = self.mode else {
            return;
        };
        let entries = std::mem::take(&mut self.cleanup_reviews);
        let summary = self.run_cleanup_deletions(&entries);
        // The open-PR set is unchanged by a cleanup, so recomputing against
        // the already-loaded listing (now with the deleted branches/state
        // entries gone) is enough — no network round-trip.
        self.recompute_launcher_finished_reviews();
        self.reopen_launcher_after_cleanup(origin);
        self.set_status_message(summary);
    }

    /// Reopens the Review launcher on the Pull Requests tab after a cleanup
    /// confirm/cancel, restoring the origin `R` was pressed from so a later
    /// `Esc` still returns there.
    fn reopen_launcher_after_cleanup(&mut self, origin: ModeOrigin) {
        self.mode = Mode::ReviewLauncher {
            tab: LauncherTab::PullRequests,
            cursor: 0,
            origin,
        };
    }

    /// Deletes each finished review in order — `worktree remove` → `prune` →
    /// managed-branch delete → state-entry removal — continuing past a
    /// per-entry failure (a locked or dirty worktree) after recording a
    /// one-line diagnostic, and returning the run's outcome summary. A worktree
    /// removal that fails leaves that entry's branch and state untouched (the
    /// branch is still checked out there); the prune, branch delete, and state
    /// delete are best-effort once the worktree is gone, exactly as
    /// [`App::finish_review`] treats them.
    fn run_cleanup_deletions(&mut self, entries: &[FinishedReview]) -> String {
        let Some(ops) = self.stage_ops.as_deref() else {
            return "cleanup unavailable (no git backend)".to_string();
        };
        let state_path = resolve_review_state_path(ops).ok();
        let mut cleaned = 0usize;
        let mut failures: Vec<String> = Vec::new();
        for entry in entries {
            match ops.worktree_remove(&entry.worktree_path) {
                Ok(()) => {
                    let _ = ops.worktree_prune();
                    let _ = ops.delete_managed_pr_branch(entry.number);
                    if let Some(path) = state_path.as_ref() {
                        let _ = crate::review::store::delete_review(path, &entry.branch);
                    }
                    cleaned += 1;
                }
                Err(e) => {
                    failures.push(format!(
                        "#{} ({})",
                        entry.number,
                        first_line(&e.to_string())
                    ));
                }
            }
        }
        cleanup_summary(cleaned, &failures)
    }
}

/// The first line of `s`, trimmed — git's own multi-line stderr collapses to
/// its headline for a compact per-entry diagnostic.
fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().to_string()
}

/// Builds the end-of-run outcome summary from the cleaned/failed counts.
fn cleanup_summary(cleaned: usize, failures: &[String]) -> String {
    match (cleaned, failures.len()) {
        (c, 0) => format!("cleaned up {c} finished review(s)"),
        (0, _) => format!("cleanup failed: {}", failures.join("; ")),
        (c, f) => format!("{c} cleaned, {f} failed: {}", failures.join("; ")),
    }
}

#[cfg(test)]
#[path = "cleanup_reviews_tests.rs"]
mod tests;
