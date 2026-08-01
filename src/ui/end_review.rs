//! The end-review modal's state transitions ([`super::app::Mode::EndReview`]):
//! opening it (capturing where `q` was pressed from), cancelling back to
//! that exact mode, and finishing a review (removing its managed worktree
//! and deleting the persisted state). Split out of `app.rs` alongside this
//! state, mirroring [`super::switcher`]'s own state-plus-handlers split.
//!
//! Ending a review does not end the process. Pause, finish, and the
//! swap-to-another-review path all converge on [`App::leave_review_session`],
//! which re-roots the app back onto the origin checkout's working tree — the
//! view redquill opens in by default. Only `Q`/Ctrl-C, and `q` from outside a
//! review, still quit.
//!
//! The three reasons differ only in what they leave *on disk* (see
//! [`LeaveReason`]) and in the status line's wording: pause and a swap keep
//! the worktree and the persisted entry, so the review resumes exactly where
//! it stopped; finish removes both first ([`App::finish_review`]).
//!
//! **Leaving a review never emits.** A review's annotations have their own
//! destinations — `review-state.json` while it is open, and for a PR/MR the
//! forge submit flow (`super::forge_submit`) — so none of the three exits
//! hands anything to the clipboard/stdout presentation `main` runs on quit.
//! That path belongs to non-review sessions, whose `q` still emits exactly as
//! it always has. The session's in-memory annotations are simply dropped as
//! it unwinds, which is also what keeps them out of the working tree's list
//! panel and out of the *next* review's persisted state.

use super::app::{App, Mode, ModeOrigin};
use super::modal_keys::EndReviewAction;
use crate::git::{DiffTarget, GitRunner};

/// Why a review session is being left. All three land on the origin working
/// tree via [`App::leave_review_session`] and clear the same in-memory state;
/// what they leave on disk was already settled by the caller before it got
/// here, so this only picks the status line's wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LeaveReason {
    /// `p` in the end-review modal, or `Esc` from the diff view. The
    /// worktree, the persisted entry, and every annotation in it stay on
    /// disk, so reopening this review resumes mid-review.
    Pause,
    /// `f` in the end-review modal, after [`App::finish_review`] has already
    /// removed the worktree and deleted the persisted entry — annotations
    /// included, since one entry holds both.
    Finish,
    /// Confirming a different branch or PR in the review launcher while a
    /// review is already open. Identical to [`LeaveReason::Pause`] in effect;
    /// distinct only so the status line can say the old review was paused
    /// rather than claim the user asked to pause it.
    Switch,
}

/// What [`App::pause_review_for_switch`] found and did — the three cases a
/// launcher confirm has to tell apart before it starts a new review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SwitchPause {
    /// No review was open; start the new one directly.
    Nothing,
    /// The named review was paused and the app is back on the origin
    /// checkout, so `stage_ops`/`repo_root` are the origin's again. The
    /// payload is the paused review's banner label, for the caller's
    /// "paused X — starting Y" status line.
    Paused(String),
    /// A review was open and could not be left — the footer already says why.
    /// The new review must not start: it would run rooted inside the outgoing
    /// review's worktree.
    Blocked,
}

impl SwitchPause {
    /// The `"paused <label> \u{2014} "` prefix a launcher confirm puts in
    /// front of what it is starting, so one footer line reports both halves
    /// of the swap. Empty when nothing was paused.
    pub(super) fn status_prefix(&self) -> String {
        match self {
            SwitchPause::Paused(label) => format!("paused {label} \u{2014} "),
            SwitchPause::Nothing | SwitchPause::Blocked => String::new(),
        }
    }
}

impl App {
    /// Opens the end-review modal, capturing the mode `q` was pressed from
    /// so [`App::cancel_end_review`] can restore it exactly. Called only
    /// when [`App::in_review_session`] is true (see [`super::quit_action`]).
    pub(super) fn open_end_review_modal(&mut self) {
        let origin = ModeOrigin::capture(self.mode);
        self.mode = Mode::EndReview { origin, cursor: 0 };
    }

    /// Closes the end-review modal without ending the session, restoring the
    /// mode it was opened from. A no-op (falls back to `Mode::Normal`, never
    /// panicking) if called while the modal isn't open — defensive rather
    /// than relied upon; every caller only invokes this from
    /// `Mode::EndReview`.
    pub(super) fn cancel_end_review(&mut self) {
        self.mode = match self.mode {
            Mode::EndReview { origin, .. } => origin.restore(),
            other => other,
        };
    }

    /// The end-review modal's currently highlighted option (0 = Pause, 1 =
    /// Finish, 2 = Cancel), if it's open — the one place
    /// [`super::modes::handle_end_review_key`]'s `Enter`/`Confirm` dispatch
    /// and [`super::end_review_modal::render`]'s highlight both read the
    /// cursor from, per the "predicates asked in more than one place get one
    /// named helper" rule.
    pub(super) fn end_review_cursor(&self) -> Option<usize> {
        match self.mode {
            Mode::EndReview { cursor, .. } => Some(cursor),
            _ => None,
        }
    }

    /// Moves the end-review modal's highlighted option down one row, clamped
    /// at the last (Cancel, index 2). A no-op outside `Mode::EndReview`.
    pub(super) fn end_review_move_down(&mut self) {
        if let Mode::EndReview { origin, cursor } = self.mode {
            self.mode = Mode::EndReview {
                origin,
                cursor: (cursor + 1).min(EndReviewAction::LAST_CURSOR),
            };
        }
    }

    /// Moves the end-review modal's highlighted option up one row, clamped
    /// at the first (Pause, index 0). A no-op outside `Mode::EndReview`.
    pub(super) fn end_review_move_up(&mut self) {
        if let Mode::EndReview { origin, cursor } = self.mode {
            self.mode = Mode::EndReview {
                origin,
                cursor: cursor.saturating_sub(1),
            };
        }
    }

    /// The `f` (finish) gesture: removes the managed review worktree through
    /// [`App::review_origin_ops`] (never `stage_ops`, which is rooted
    /// *inside* the worktree being removed — see that field's doc). On
    /// success it also prunes stale worktree admin records and deletes this
    /// branch's persisted review entry — statuses and annotations live in
    /// one [`crate::review::store::PersistedReview`], so one
    /// [`crate::review::store::delete_review`] call removes both, and a
    /// later fresh `--review` of the same branch starts clean. Returns `true`
    /// on success, so the caller can hand off to
    /// [`App::leave_review_session`] and land back on the working tree.
    /// Nothing is emitted on the way — see this module's doc. On failure
    /// (e.g. a dirty worktree, or no origin backend/review session attached)
    /// the git message surfaces as a status message, the modal closes back to
    /// its origin mode, the session stays open, and the persisted state entry
    /// is left untouched for retry.
    pub(super) fn finish_review(&mut self) -> bool {
        let Some(ops) = self.review_origin_ops.as_deref() else {
            self.set_status_message("finish unavailable (no origin git backend)");
            self.cancel_end_review();
            return false;
        };
        let Some(path) = self.repo_root.clone() else {
            self.set_status_message("finish unavailable (no review worktree path)");
            self.cancel_end_review();
            return false;
        };
        match ops.worktree_remove(&path) {
            Ok(()) => {
                // Best-effort: a prune failure doesn't undo the removal that
                // already succeeded, and has nothing useful to surface to
                // the user (stale admin records are harmless clutter, not a
                // correctness issue).
                let _ = ops.worktree_prune();
                // Same best-effort treatment: the worktree is already gone,
                // so a failure to also delete the (much less consequential)
                // state entry isn't worth surfacing over — the next launch's
                // GC would clean up a leftover entry once the branch itself
                // is gone, and while the branch still exists a stale entry
                // just means the next `--review` of it resumes old progress
                // instead of starting fresh, not a crash or data-loss risk.
                if let (Some(state_path), Some(branch)) =
                    (self.review_state_path.clone(), self.review_branch())
                {
                    let _ = crate::review::store::delete_review(&state_path, branch);
                }
                true
            }
            Err(e) => {
                self.set_status_message(format!("finish failed: {e}"));
                self.cancel_end_review();
                false
            }
        }
    }

    /// Pauses whatever review session is open so a different one can start in
    /// its place — the review launcher's "switch reviews" step, and the only
    /// caller of [`LeaveReason::Switch`]. Outside a review session this does
    /// nothing and reports [`SwitchPause::Nothing`], so a launcher confirm can
    /// call it unconditionally.
    ///
    /// A swap is a pause, not a finish: the outgoing review's worktree,
    /// persisted progress, and annotations all stay exactly where they are, so
    /// reopening it later resumes mid-review. What the swap really buys the
    /// caller is the re-root — after this returns [`SwitchPause::Paused`],
    /// `stage_ops` and `repo_root` point at the origin checkout again, which
    /// is what the incoming review's worktree/fetch machinery expects to read
    /// them as.
    pub(super) fn pause_review_for_switch(&mut self) -> SwitchPause {
        if !self.in_review_session() {
            return SwitchPause::Nothing;
        }
        let label = self.review_banner_label();
        if self.leave_review_session(LeaveReason::Switch) {
            SwitchPause::Paused(label)
        } else {
            SwitchPause::Blocked
        }
    }

    /// Whether the review session in flight is a forge review of PR/MR
    /// `number` — the check a launcher confirm makes before treating `Enter`
    /// as a swap, since "switching" to the PR already on screen would only
    /// tear down and rebuild the session it is already showing.
    pub(super) fn reviewing_pr(&self, number: u64) -> bool {
        self.in_review_session()
            && self
                .review_forge
                .as_ref()
                .is_some_and(|forge| forge.number == number)
    }

    /// Leaves the active review session and re-roots the app back onto the
    /// origin checkout's working tree ([`DiffTarget::WorkingTree`], the
    /// default view), returning `true` once it has landed there.
    ///
    /// This is the single unwind path for all three exits — the end-review
    /// modal's pause and finish, `Esc` from the diff view, and the review
    /// launcher's swap-to-another-review — so what a review leaves behind can
    /// never drift between them. `reason` picks the status line's wording and
    /// nothing else: every exit clears the same in-memory state, and what
    /// survives on disk was already settled before this was called (pause
    /// leaves the persisted entry alone, finish deleted it).
    ///
    /// Nothing is emitted here. A review's annotations reach their consumer
    /// through the persisted entry and, for a PR/MR, the forge — never the
    /// clipboard/stdout presentation, which belongs to a non-review session's
    /// `q`. See this module's doc.
    ///
    /// The re-root is attempted *before* any session state is cleared, so a
    /// failure — a missing origin root (never the case for a session started
    /// through any of the real entry points, all of which record it), or a
    /// repository that has since gone away — leaves the review fully intact
    /// with git's own message in the footer, exactly like
    /// [`App::confirm_worktree_switch`]'s failure path.
    ///
    /// Mode is deliberately untouched: the launcher stays open across a swap,
    /// the diff view stays put on `Esc`, and the end-review modal's callers
    /// restore their own origin mode first. Panel coherence is re-established
    /// afterwards for the one mode that needs it.
    pub(super) fn leave_review_session(&mut self, reason: LeaveReason) -> bool {
        if !self.in_review_session() {
            return false;
        }
        let Some(origin_root) = self.review_origin_root.clone() else {
            self.set_status_message(
                "can't return to the working tree (no origin repo recorded for this review)",
            );
            return false;
        };
        // Flush any status/annotation change that hasn't reached disk yet.
        // A no-op for pause after a quiet moment (save-on-change has already
        // written it) and for finish (whose entry is deleted below anyway,
        // making a redundant save harmless).
        if matches!(reason, LeaveReason::Pause | LeaveReason::Switch) {
            self.persist_review_state();
        }
        let left = self.review_banner_label();
        let runner = match GitRunner::discover_in(&origin_root) {
            Ok(runner) => runner,
            Err(e) => {
                self.set_status_message(format!("can't return to the working tree: {e}"));
                return false;
            }
        };
        if let Err(e) = self.reroot(runner, DiffTarget::WorkingTree) {
            self.set_status_message(format!("can't return to the working tree: {e}"));
            return false;
        }

        // Past the point of no return: `reroot` has swapped the backend, the
        // root, and the target, and cleared the forge/stale markers. Every
        // remaining scrap of per-session state goes with it, so the working
        // tree isn't left wearing the review's accept marks, annotations, or
        // imported comment threads.
        self.annotations = crate::annotate::AnnotationStore::new();
        self.replies = super::draft_reply::DraftReplyStore::new();
        self.review_states.clear();
        self.review_blob_shas.clear();
        self.review_origin_ops = None;
        self.review_origin_root = None;
        self.review_state_path = None;
        self.thread_overlay.replace(Vec::new());
        self.threads_unavailable = false;
        // A thread fetch spawned for the PR just left must not repopulate the
        // overlay once it lands (mirrors `spawn_thread_fetch`'s own bump).
        self.thread_fetch_generation = self.thread_fetch_generation.wrapping_add(1);
        self.list_cursor = 0;
        self.list_filter = None;
        self.rebuild_rows();
        self.after_panel_coherence();

        match reason {
            LeaveReason::Pause => {
                self.set_status_message(format!("paused {left} \u{2014} back on the working tree"));
            }
            LeaveReason::Finish => {
                self.set_status_message(format!(
                    "finished {left} \u{2014} back on the working tree"
                ));
            }
            // Deliberately silent: the launcher immediately overwrites the
            // footer with what it is starting, and folds the paused review's
            // name into that one line rather than flashing two.
            LeaveReason::Switch => {}
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::FileDiff;
    use crate::git::{DiffTarget, GitError, RawFilePatch};
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    use super::super::app::PanelTab;
    use super::super::stage_ops::StageOps;

    fn sample_file() -> FileDiff {
        let raw = "\
diff --git a/src/main.rs b/src/main.rs
index 111..222 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,2 +1,2 @@
 fn main() {
-    old();
+    new();
";
        FileDiff::from_patch(&RawFilePatch {
            path: "src/main.rs".to_string(),
            old_path: None,
            raw: raw.to_string(),
            is_binary: false,
        })
        .unwrap()
    }

    fn review_app() -> App {
        let mut app = App::new(vec![sample_file()]);
        app.target = DiffTarget::Review {
            base: "main".to_string(),
            branch: "feature".to_string(),
        };
        app
    }

    #[test]
    fn open_from_panel_and_cancel_restores_the_cursor_and_tab() {
        let mut app = review_app();
        app.mode = Mode::Panel {
            cursor: 2,
            tab: PanelTab::History,
        };
        app.open_end_review_modal();
        assert_eq!(
            app.mode,
            Mode::EndReview {
                origin: ModeOrigin::Panel {
                    cursor: 2,
                    tab: PanelTab::History,
                },
                cursor: 0,
            }
        );
        app.cancel_end_review();
        assert_eq!(
            app.mode,
            Mode::Panel {
                cursor: 2,
                tab: PanelTab::History,
            }
        );
    }

    /// A recording [`StageOps`] fake tracking `worktree_remove`/
    /// `worktree_prune` calls only — the rest of the trait is unused by
    /// `finish_review`.
    #[derive(Default)]
    struct WorktreeFake {
        remove_calls: Rc<RefCell<Vec<PathBuf>>>,
        prune_calls: Rc<RefCell<usize>>,
        remove_error: Option<String>,
    }

    impl StageOps for WorktreeFake {
        fn diff(&self, _target: &DiffTarget) -> Result<Vec<RawFilePatch>, GitError> {
            Ok(Vec::new())
        }
        fn status(&self) -> Result<Vec<crate::git::FileStatus>, GitError> {
            Ok(Vec::new())
        }
        fn stage_file(&self, _path: &str) -> Result<(), GitError> {
            Ok(())
        }
        fn unstage_file(&self, _path: &str) -> Result<(), GitError> {
            Ok(())
        }
        fn apply_cached(&self, _patch: &str) -> Result<(), GitError> {
            Ok(())
        }
        fn unapply_cached(&self, _patch: &str) -> Result<(), GitError> {
            Ok(())
        }
        fn read_worktree_file(&self, _path: &str) -> Option<Vec<u8>> {
            None
        }
        fn show_file(&self, _spec: &str) -> Option<String> {
            None
        }
        fn worktree_remove(&self, path: &Path) -> Result<(), GitError> {
            self.remove_calls.borrow_mut().push(path.to_path_buf());
            match &self.remove_error {
                None => Ok(()),
                Some(stderr) => Err(GitError::Command {
                    command: format!("worktree remove {}", path.display()),
                    code: "1".to_string(),
                    stderr: stderr.clone(),
                }),
            }
        }
        fn worktree_prune(&self) -> Result<(), GitError> {
            *self.prune_calls.borrow_mut() += 1;
            Ok(())
        }
    }

    #[test]
    fn finish_deletes_the_branchs_persisted_state_entry() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state_path = tmp.path().join("review-state.json");
        crate::review::store::save_review(
            &state_path,
            "feature",
            crate::review::store::PersistedReview {
                base: "main".to_string(),
                worktree_path: PathBuf::from("/tmp/review-worktree"),
                files: std::collections::BTreeMap::new(),
                annotations: Vec::new(),
                replies: Vec::new(),
                forge: None,
            },
        )
        .unwrap();
        // A different branch's entry must survive.
        crate::review::store::save_review(
            &state_path,
            "other-branch",
            crate::review::store::PersistedReview {
                base: "main".to_string(),
                worktree_path: PathBuf::from("/tmp/other-worktree"),
                files: std::collections::BTreeMap::new(),
                annotations: Vec::new(),
                replies: Vec::new(),
                forge: None,
            },
        )
        .unwrap();

        let mut app = review_app();
        app.set_repo_root(PathBuf::from("/tmp/review-worktree"));
        app.set_review_state_path(state_path.clone());
        app.set_review_origin_ops(Box::new(WorktreeFake::default()));
        app.open_end_review_modal();

        let outcome = app.finish_review();

        assert!(
            outcome,
            "a successful worktree removal reports finish succeeded"
        );
        let state = crate::review::store::load(&state_path);
        assert!(
            !state.reviews.contains_key("feature"),
            "finish must delete this review's own entry"
        );
        assert!(
            state.reviews.contains_key("other-branch"),
            "finish must never touch another branch's entry"
        );
    }

    #[test]
    fn finish_failure_leaves_the_persisted_state_entry_untouched() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state_path = tmp.path().join("review-state.json");
        crate::review::store::save_review(
            &state_path,
            "feature",
            crate::review::store::PersistedReview {
                base: "main".to_string(),
                worktree_path: PathBuf::from("/tmp/review-worktree"),
                files: std::collections::BTreeMap::new(),
                annotations: Vec::new(),
                replies: Vec::new(),
                forge: None,
            },
        )
        .unwrap();

        let mut app = review_app();
        app.set_repo_root(PathBuf::from("/tmp/review-worktree"));
        app.set_review_state_path(state_path.clone());
        app.set_review_origin_ops(Box::new(WorktreeFake {
            remove_error: Some("fatal: worktree is dirty".to_string()),
            ..Default::default()
        }));
        app.open_end_review_modal();

        let outcome = app.finish_review();

        assert!(!outcome);
        let state = crate::review::store::load(&state_path);
        assert!(
            state.reviews.contains_key("feature"),
            "a failed finish must leave the persisted entry in place"
        );
    }

    #[test]
    fn finish_failure_surfaces_the_message_and_keeps_the_session() {
        let mut app = review_app();
        app.set_repo_root(PathBuf::from("/tmp/review-worktree"));
        app.set_review_origin_ops(Box::new(WorktreeFake {
            remove_error: Some("fatal: worktree is dirty".to_string()),
            ..Default::default()
        }));
        app.open_end_review_modal();

        let outcome = app.finish_review();
        assert!(!outcome, "a failed finish must not report success");
        assert_eq!(
            app.mode,
            Mode::Normal,
            "the modal closes back to its origin"
        );
        assert!(
            app.status_message
                .as_deref()
                .is_some_and(|m| m.contains("worktree is dirty")),
            "git's own message must surface: {:?}",
            app.status_message
        );
    }

    #[test]
    fn finish_without_an_origin_backend_degrades_to_a_message() {
        let mut app = review_app();
        app.open_end_review_modal();
        let outcome = app.finish_review();
        assert!(!outcome);
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.status_message.is_some());
    }
}
