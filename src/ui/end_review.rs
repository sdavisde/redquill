//! The end-review modal's state transitions ([`super::app::Mode::EndReview`],
//! spec 08 Unit 2): opening it (capturing where `q` was pressed from),
//! cancelling back to that exact mode, and finishing a review (removing its
//! managed worktree and deleting the persisted state — Unit 4 wires the
//! latter half). Split out of `app.rs` alongside this state, mirroring
//! [`super::switcher`]'s own state-plus-handlers split.
//!
//! Pausing has no dedicated method here: it's exactly the pre-existing quit
//! path (`Flow::Quit(QuitOutcome::Discard)`, handled by
//! [`super::modes::handle_end_review_key`]'s `end_review_choice`) — amended
//! 2026-07-16, spec 08 Unit 6, reversing this module's original "pause
//! emits" note: pause now discards the stdout side effect, since
//! annotations are already durable (save-on-change, task 7.2) by the time
//! `p` is pressed. The "pause" contract is now keep the worktree, keep the
//! state, keep every annotation on disk, emit nothing, quit.

use super::QuitOutcome;
use super::app::{App, EndReviewOrigin, Mode};
use super::modal_keys::EndReviewAction;

impl App {
    /// Opens the end-review modal (spec 08 Unit 2), capturing the mode `q`
    /// was pressed from so [`App::cancel_end_review`] can restore it
    /// exactly. Called only when [`App::in_review_session`] is true (see
    /// [`super::quit_action`]).
    pub(super) fn open_end_review_modal(&mut self) {
        let origin = match self.mode {
            Mode::Visual { anchor } => EndReviewOrigin::Visual { anchor },
            Mode::Panel { cursor, tab } => EndReviewOrigin::Panel { cursor, tab },
            _ => EndReviewOrigin::Normal,
        };
        self.mode = Mode::EndReview { origin, cursor: 0 };
    }

    /// Closes the end-review modal without ending the session, restoring the
    /// mode it was opened from. A no-op (falls back to `Mode::Normal`, never
    /// panicking) if called while the modal isn't open — defensive rather
    /// than relied upon; every caller only invokes this from
    /// `Mode::EndReview`.
    pub(super) fn cancel_end_review(&mut self) {
        self.mode = match self.mode {
            Mode::EndReview { origin, .. } => match origin {
                EndReviewOrigin::Normal => Mode::Normal,
                EndReviewOrigin::Visual { anchor } => Mode::Visual { anchor },
                EndReviewOrigin::Panel { cursor, tab } => Mode::Panel { cursor, tab },
            },
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
    /// *inside* the worktree being removed — see that field's doc), then
    /// prunes stale worktree admin records and — spec 08 Unit 4, closing the
    /// loop with this unit — deletes this branch's persisted review-state
    /// entry (statuses *and* annotations together, spec 08 Unit 6 task 7.3:
    /// they live in one [`crate::review::store::PersistedReview`], so one
    /// [`crate::review::store::delete_review`] call removes both — "one
    /// lifecycle", no orphaned annotation data left behind), so a later
    /// fresh `--review` of the same branch starts clean rather than
    /// resuming stale progress. Returns `Some(QuitOutcome::Emit)` on
    /// success — the caller quits emitting `app.annotations`, which by this
    /// point holds the complete restored-plus-new set exactly once, in the
    /// unchanged markdown format. On failure (e.g. a dirty worktree; or no
    /// origin backend/no review session attached, in a git-less test
    /// context) the git message is surfaced as a status message and the
    /// modal closes back to its origin mode — the review continues, nothing
    /// is removed, and the persisted state entry (statuses and annotations
    /// alike) is left untouched (the worktree removal is the gate; the
    /// state entry is only ever deleted alongside a worktree that actually
    /// went away).
    pub(super) fn finish_review(&mut self) -> Option<QuitOutcome> {
        let Some(ops) = self.review_origin_ops.as_deref() else {
            self.set_status_message("finish unavailable (no origin git backend)");
            self.cancel_end_review();
            return None;
        };
        let Some(path) = self.repo_root.clone() else {
            self.set_status_message("finish unavailable (no review worktree path)");
            self.cancel_end_review();
            return None;
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
                // GC (spec 08 Unit 4 task 4.5) would clean up a leftover
                // entry anyway once the branch itself is gone, and while the
                // branch still exists a stale entry just means the next
                // `--review` of it resumes old progress instead of starting
                // fresh, not a crash or data-loss risk.
                if let (Some(state_path), Some(branch)) =
                    (self.review_state_path.clone(), self.review_branch())
                {
                    let _ = crate::review::store::delete_review(&state_path, branch);
                }
                Some(QuitOutcome::Emit)
            }
            Err(e) => {
                self.set_status_message(format!("finish failed: {e}"));
                self.cancel_end_review();
                None
            }
        }
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
    fn open_from_normal_and_cancel_restores_normal() {
        let mut app = review_app();
        assert_eq!(app.mode, Mode::Normal);
        app.open_end_review_modal();
        assert_eq!(
            app.mode,
            Mode::EndReview {
                origin: EndReviewOrigin::Normal,
                cursor: 0,
            }
        );
        app.cancel_end_review();
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn open_from_visual_and_cancel_restores_the_anchor() {
        let mut app = review_app();
        app.mode = Mode::Visual { anchor: 3 };
        app.open_end_review_modal();
        assert_eq!(
            app.mode,
            Mode::EndReview {
                origin: EndReviewOrigin::Visual { anchor: 3 },
                cursor: 0,
            }
        );
        app.cancel_end_review();
        assert_eq!(app.mode, Mode::Visual { anchor: 3 });
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
                origin: EndReviewOrigin::Panel {
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
    fn finish_removes_the_worktree_prunes_and_quits_emitting() {
        let mut app = review_app();
        app.set_repo_root(PathBuf::from("/tmp/review-worktree"));
        let remove_calls = Rc::new(RefCell::new(Vec::new()));
        let prune_calls = Rc::new(RefCell::new(0));
        app.set_review_origin_ops(Box::new(WorktreeFake {
            remove_calls: Rc::clone(&remove_calls),
            prune_calls: Rc::clone(&prune_calls),
            remove_error: None,
        }));
        app.open_end_review_modal();

        let outcome = app.finish_review();
        assert_eq!(outcome, Some(QuitOutcome::Emit));
        assert_eq!(
            remove_calls.borrow().as_slice(),
            [PathBuf::from("/tmp/review-worktree")]
        );
        assert_eq!(
            *prune_calls.borrow(),
            1,
            "prune must run after a successful remove"
        );
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
            },
        )
        .unwrap();

        let mut app = review_app();
        app.set_repo_root(PathBuf::from("/tmp/review-worktree"));
        app.set_review_state_path(state_path.clone());
        app.set_review_origin_ops(Box::new(WorktreeFake::default()));
        app.open_end_review_modal();

        let outcome = app.finish_review();

        assert_eq!(outcome, Some(QuitOutcome::Emit));
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

    /// Spec 08 Unit 6, task 7.3's "one lifecycle" requirement: finish
    /// deletes a branch's persisted *annotations* alongside its file
    /// statuses, in the same call — there is no separate annotation-only
    /// entry left orphaned behind.
    #[test]
    fn finish_deletes_persisted_annotations_alongside_the_state_entry() {
        use crate::annotate::{Classification, PersistedAnnotation, Side, Source, Target};

        let tmp = tempfile::TempDir::new().unwrap();
        let state_path = tmp.path().join("review-state.json");
        crate::review::store::save_review(
            &state_path,
            "feature",
            crate::review::store::PersistedReview {
                base: "main".to_string(),
                worktree_path: PathBuf::from("/tmp/review-worktree"),
                files: std::collections::BTreeMap::new(),
                annotations: vec![PersistedAnnotation {
                    target: Target::line("src/main.rs", 1, Side::New),
                    classification: Classification::Nit,
                    body: "note".to_string(),
                    source: Source::WorkingTree,
                }],
            },
        )
        .unwrap();
        assert!(
            !crate::review::store::load(&state_path).reviews["feature"]
                .annotations
                .is_empty(),
            "fixture must actually have a persisted annotation before finish"
        );

        let mut app = review_app();
        app.set_repo_root(PathBuf::from("/tmp/review-worktree"));
        app.set_review_state_path(state_path.clone());
        app.set_review_origin_ops(Box::new(WorktreeFake::default()));
        app.open_end_review_modal();

        let outcome = app.finish_review();

        assert_eq!(outcome, Some(QuitOutcome::Emit));
        let state = crate::review::store::load(&state_path);
        assert!(
            !state.reviews.contains_key("feature"),
            "finish deletes the whole entry, annotations included"
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

        assert_eq!(outcome, None);
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
        assert_eq!(outcome, None, "a failed finish must not quit");
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
        assert_eq!(outcome, None);
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.status_message.is_some());
    }
}
