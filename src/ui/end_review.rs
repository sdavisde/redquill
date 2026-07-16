//! The end-review modal's state transitions ([`super::app::Mode::EndReview`],
//! spec 08 Unit 2): opening it (capturing where `q` was pressed from),
//! cancelling back to that exact mode, and finishing a review (removing its
//! managed worktree and deleting the persisted state — Unit 4 wires the
//! latter half). Split out of `app.rs` alongside this state, mirroring
//! [`super::switcher`]'s own state-plus-handlers split.
//!
//! Pausing has no dedicated method here: it's exactly the pre-existing quit
//! path (`Flow::Quit(QuitOutcome::Emit)`, handled by [`super::quit_action`]'s
//! caller), which already emits `app.annotations` to stdout on the way out —
//! the "pause" contract (emit, keep the worktree, quit) needs nothing extra.

use super::QuitOutcome;
use super::app::{App, EndReviewOrigin, Mode};

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
        self.mode = Mode::EndReview { origin };
    }

    /// Closes the end-review modal without ending the session, restoring the
    /// mode it was opened from. A no-op (falls back to `Mode::Normal`, never
    /// panicking) if called while the modal isn't open — defensive rather
    /// than relied upon; every caller only invokes this from
    /// `Mode::EndReview`.
    pub(super) fn cancel_end_review(&mut self) {
        self.mode = match self.mode {
            Mode::EndReview { origin } => match origin {
                EndReviewOrigin::Normal => Mode::Normal,
                EndReviewOrigin::Visual { anchor } => Mode::Visual { anchor },
                EndReviewOrigin::Panel { cursor, tab } => Mode::Panel { cursor, tab },
            },
            other => other,
        };
    }

    /// The `f` (finish) gesture: removes the managed review worktree through
    /// [`App::review_origin_ops`] (never `stage_ops`, which is rooted
    /// *inside* the worktree being removed — see that field's doc), then
    /// prunes stale worktree admin records. Returns `Some(QuitOutcome::Emit)`
    /// on success — the caller quits exactly like a plain `q`, so annotations
    /// emit through the existing on-quit path unchanged. On failure (e.g. a
    /// dirty worktree; or no origin backend/no review session attached, in a
    /// git-less test context) the git message is surfaced as a status
    /// message and the modal closes back to its origin mode — the review
    /// continues, nothing is removed.
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
                origin: EndReviewOrigin::Normal
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
                origin: EndReviewOrigin::Visual { anchor: 3 }
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
                }
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
