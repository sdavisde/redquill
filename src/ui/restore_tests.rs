//! Tests for the restore confirm modal's state transitions
//! (`src/ui/restore.rs`).
//!
//! The contract under test is narrow but strict: `d` must never reach git on
//! its own, a confirm must run exactly the operation the modal's own wording
//! promised, and every exit must land back where `d` was pressed.

use std::cell::RefCell;
use std::rc::Rc;

use super::*;
use crate::diff::FileDiff;
use crate::git::{DiffTarget, GitError, RawFilePatch, RestoreScope};

use crate::ui::app::PanelTab;
use crate::ui::stage_ops::StageOps;

/// Every restore-shaped call a confirm made, in order, tagged by kind — so a
/// test can assert not just "git was touched" but *which* of the two
/// destructive operations ran.
#[derive(Default)]
struct RestoreLog {
    /// Each restore call's pathspec, joined with `+` so a test can assert
    /// both halves of a rename landed in ONE invocation (two calls would
    /// leave a window where the file exists at neither path).
    restored: Vec<String>,
    discarded: Vec<String>,
    scopes: Vec<RestoreScope>,
}

/// A [`StageOps`] fake that records the restore calls it receives and can be
/// told to fail, for the error-path tests.
struct RecordingOps {
    log: Rc<RefCell<RestoreLog>>,
    fail: bool,
}

impl RecordingOps {
    fn new(log: Rc<RefCell<RestoreLog>>) -> RecordingOps {
        RecordingOps { log, fail: false }
    }

    fn failing(log: Rc<RefCell<RestoreLog>>) -> RecordingOps {
        RecordingOps { log, fail: true }
    }
}

impl StageOps for RecordingOps {
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
    fn restore_paths(&self, paths: &[&str], scope: RestoreScope) -> Result<(), GitError> {
        self.log.borrow_mut().restored.push(paths.join("+"));
        self.log.borrow_mut().scopes.push(scope);
        if self.fail {
            return Err(GitError::Parse("restore blew up".into()));
        }
        Ok(())
    }
    fn discard_untracked_file(&self, path: &str) -> Result<(), GitError> {
        self.log.borrow_mut().discarded.push(path.to_string());
        if self.fail {
            return Err(GitError::Parse("delete blew up".into()));
        }
        Ok(())
    }
}

fn file(path: &str) -> FileDiff {
    let raw = format!(
        "diff --git a/{path} b/{path}\n\
         index 111..222 100644\n\
         --- a/{path}\n\
         +++ b/{path}\n\
         @@ -1,2 +1,2 @@\n\
         \x20context\n\
         -old\n\
         +new\n"
    );
    FileDiff::from_patch(&RawFilePatch {
        path: path.to_string(),
        old_path: None,
        raw,
        is_binary: false,
    })
    .unwrap()
}

/// A renamed file: `old` -> `path`, as the diff model represents a rename.
fn renamed_file(old: &str, path: &str) -> FileDiff {
    let raw = format!(
        "diff --git a/{old} b/{path}\n\
         similarity index 90%\n\
         rename from {old}\n\
         rename to {path}\n\
         --- a/{old}\n\
         +++ b/{path}\n\
         @@ -1,2 +1,2 @@\n\
         \x20context\n\
         -old\n\
         +new\n"
    );
    FileDiff::from_patch(&RawFilePatch {
        path: path.to_string(),
        old_path: Some(old.to_string()),
        raw,
        is_binary: false,
    })
    .unwrap()
}

/// An app over one working-tree file with a recording backend attached.
fn app_with_ops(ops: RecordingOps) -> App {
    let mut app = App::new(vec![file("src/main.rs")]);
    app.target = DiffTarget::WorkingTree;
    app.stage_ops = Some(Box::new(ops));
    app
}

fn logged_app() -> (App, Rc<RefCell<RestoreLog>>) {
    let log = Rc::new(RefCell::new(RestoreLog::default()));
    (app_with_ops(RecordingOps::new(log.clone())), log)
}

// -- Opening ---------------------------------------------------------------

#[test]
fn open_captures_the_cursor_file_and_switches_mode() {
    let (mut app, log) = logged_app();
    app.open_confirm_restore();

    assert!(matches!(app.mode, Mode::ConfirmRestore { .. }));
    assert_eq!(
        app.restore_request,
        Some(RestoreRequest {
            path: "src/main.rs".to_string(),
            old_path: None,
            untracked: false,
            scope: RestoreScope::IndexAndWorktree,
        })
    );
    // The keypress itself must never reach git.
    assert!(log.borrow().restored.is_empty());
    assert!(log.borrow().discarded.is_empty());
}

#[test]
fn open_marks_an_untracked_file_as_untracked() {
    let (mut app, _log) = logged_app();
    app.untracked_paths = vec!["src/main.rs".to_string()];
    app.open_confirm_restore();

    assert_eq!(
        app.restore_request.as_ref().map(|r| r.untracked),
        Some(true)
    );
}

#[test]
fn open_remembers_the_panel_cursor_and_tab_it_was_pressed_from() {
    let (mut app, _log) = logged_app();
    app.mode = Mode::Panel {
        cursor: 3,
        tab: PanelTab::Changes,
    };
    app.open_confirm_restore();
    app.cancel_confirm_restore();

    assert_eq!(
        app.mode,
        Mode::Panel {
            cursor: 3,
            tab: PanelTab::Changes,
        }
    );
}

#[test]
fn open_is_refused_on_a_read_only_target() {
    let (mut app, _log) = logged_app();
    app.target = DiffTarget::Commit("abc123".to_string());
    app.open_confirm_restore();

    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.restore_request, None);
    assert_eq!(app.status_message.as_deref(), Some("read-only diff target"));
}

#[test]
fn open_is_refused_during_a_review_session() {
    let (mut app, _log) = logged_app();
    app.target = DiffTarget::Review {
        base: "main".to_string(),
        branch: "feature".to_string(),
    };
    app.open_confirm_restore();

    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.restore_request, None);
}

#[test]
fn open_is_refused_with_no_git_backend() {
    let mut app = App::new(vec![file("src/main.rs")]);
    app.target = DiffTarget::WorkingTree;
    app.stage_ops = None;
    app.open_confirm_restore();

    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.restore_request, None);
    assert_eq!(
        app.status_message.as_deref(),
        Some("restore unavailable (no git backend)")
    );
}

#[test]
fn open_on_an_empty_diff_is_a_no_op() {
    let log = Rc::new(RefCell::new(RestoreLog::default()));
    let mut app = App::new(Vec::new());
    app.target = DiffTarget::WorkingTree;
    app.stage_ops = Some(Box::new(RecordingOps::new(log)));
    app.open_confirm_restore();

    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.restore_request, None);
}

// -- Cancelling ------------------------------------------------------------

#[test]
fn cancel_runs_nothing_and_clears_the_request() {
    let (mut app, log) = logged_app();
    app.open_confirm_restore();
    app.cancel_confirm_restore();

    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.restore_request, None);
    assert!(log.borrow().restored.is_empty());
    assert!(log.borrow().discarded.is_empty());
}

#[test]
fn cancel_outside_the_modal_is_a_no_op() {
    let (mut app, _log) = logged_app();
    app.cancel_confirm_restore();
    assert_eq!(app.mode, Mode::Normal);
}

// -- Confirming ------------------------------------------------------------

#[test]
fn confirm_restores_the_tracked_file_and_closes() {
    let (mut app, log) = logged_app();
    app.open_confirm_restore();
    app.confirm_confirm_restore();

    assert_eq!(log.borrow().restored, vec!["src/main.rs".to_string()]);
    assert!(log.borrow().discarded.is_empty());
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.restore_request, None);
    assert_eq!(app.status_message.as_deref(), Some("restored src/main.rs"));
}

#[test]
fn confirm_deletes_an_untracked_file_instead_of_restoring_it() {
    let (mut app, log) = logged_app();
    app.untracked_paths = vec!["src/main.rs".to_string()];
    app.open_confirm_restore();
    app.confirm_confirm_restore();

    assert_eq!(log.borrow().discarded, vec!["src/main.rs".to_string()]);
    assert!(log.borrow().restored.is_empty());
    assert_eq!(app.status_message.as_deref(), Some("deleted src/main.rs"));
}

#[test]
fn confirm_acts_on_the_path_frozen_at_open_not_the_live_cursor() {
    let (mut app, log) = logged_app();
    app.open_confirm_restore();
    // A background refresh reshuffles the diff under the open modal.
    app.view.files = vec![file("src/other.rs")];
    app.confirm_confirm_restore();

    assert_eq!(log.borrow().restored, vec!["src/main.rs".to_string()]);
}

#[test]
fn confirm_acts_on_the_kind_frozen_at_open_not_the_live_untracked_set() {
    let (mut app, log) = logged_app();
    app.untracked_paths = vec!["src/main.rs".to_string()];
    app.open_confirm_restore();
    // The question said "Delete"; a refresh must not turn it into a restore.
    app.untracked_paths.clear();
    app.confirm_confirm_restore();

    assert_eq!(log.borrow().discarded, vec!["src/main.rs".to_string()]);
    assert!(log.borrow().restored.is_empty());
}

#[test]
fn confirm_returns_to_the_panel_it_was_opened_from() {
    let (mut app, _log) = logged_app();
    app.mode = Mode::Panel {
        cursor: 2,
        tab: PanelTab::Changes,
    };
    app.open_confirm_restore();
    app.confirm_confirm_restore();

    // Focus lands back in the panel on the tab it left from. The cursor
    // index deliberately isn't asserted: the restored file leaves the diff,
    // so the post-refresh clamp is expected to move it.
    assert!(
        matches!(
            app.mode,
            Mode::Panel {
                tab: PanelTab::Changes,
                ..
            }
        ),
        "expected to land back in the panel, got {:?}",
        app.mode
    );
}

#[test]
fn confirm_surfaces_a_git_failure_and_closes_the_modal() {
    let log = Rc::new(RefCell::new(RestoreLog::default()));
    let mut app = app_with_ops(RecordingOps::failing(log.clone()));
    app.open_confirm_restore();
    app.confirm_confirm_restore();

    assert_eq!(log.borrow().restored, vec!["src/main.rs".to_string()]);
    // The modal doesn't outlive its question, even on failure.
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.restore_request, None);
    assert!(
        app.status_message
            .as_deref()
            .is_some_and(|m| m.contains("restore blew up")),
        "expected the git error in the footer, got {:?}",
        app.status_message
    );
}

#[test]
fn confirm_outside_the_modal_runs_nothing() {
    let (mut app, log) = logged_app();
    app.confirm_confirm_restore();

    assert!(log.borrow().restored.is_empty());
    assert!(log.borrow().discarded.is_empty());
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn confirm_with_a_missing_request_closes_without_running_anything() {
    let (mut app, log) = logged_app();
    app.open_confirm_restore();
    // Defensive: the two always move together, but a desynced pair must
    // close cleanly rather than act on a guessed path.
    app.restore_request = None;
    app.confirm_confirm_restore();

    assert!(log.borrow().restored.is_empty());
    assert_eq!(app.mode, Mode::Normal);
}

// -- Reachability ----------------------------------------------------------

#[test]
fn the_restore_file_action_opens_the_modal_rather_than_acting() {
    let (mut app, log) = logged_app();
    app.apply(crate::ui::Action::RestoreFile);

    assert!(matches!(app.mode, Mode::ConfirmRestore { .. }));
    assert!(log.borrow().restored.is_empty());
}

// -- Regression: renames (a restore of the new path alone loses the file) ---

#[test]
fn a_renamed_file_restores_both_paths_in_one_invocation() {
    let log = Rc::new(RefCell::new(RestoreLog::default()));
    let mut app = App::new(vec![renamed_file("src/old.rs", "src/new.rs")]);
    app.target = DiffTarget::WorkingTree;
    app.stage_ops = Some(Box::new(RecordingOps::new(log.clone())));

    app.open_confirm_restore();
    app.confirm_confirm_restore();

    // One call, both paths: restoring only the new path would delete it (it
    // isn't in HEAD) and strand the original staged-deleted.
    assert_eq!(
        log.borrow().restored,
        vec!["src/new.rs+src/old.rs".to_string()]
    );
}

#[test]
fn a_copied_file_does_not_drag_its_source_path_into_the_restore() {
    let log = Rc::new(RefCell::new(RestoreLog::default()));
    let mut copied = renamed_file("src/source.rs", "src/copy.rs");
    copied.kind = crate::diff::FileChangeKind::Copied;
    let mut app = App::new(vec![copied]);
    app.target = DiffTarget::WorkingTree;
    app.stage_ops = Some(Box::new(RecordingOps::new(log.clone())));

    app.open_confirm_restore();
    app.confirm_confirm_restore();

    // A copy's source is an untouched file and must stay untouched.
    assert_eq!(log.borrow().restored, vec!["src/copy.rs".to_string()]);
}

// -- Regression: the staged view must not touch the working tree -----------

#[test]
fn the_staged_view_restores_the_index_only() {
    let log = Rc::new(RefCell::new(RestoreLog::default()));
    let mut app = App::new(vec![file("src/main.rs")]);
    app.target = DiffTarget::Staged;
    app.stage_ops = Some(Box::new(RecordingOps::new(log.clone())));

    app.open_confirm_restore();
    assert!(
        matches!(app.mode, Mode::ConfirmRestore { .. }),
        "the staged view is writable"
    );
    app.confirm_confirm_restore();

    // `--worktree` here would discard edits the staged view never rendered.
    assert_eq!(log.borrow().scopes, vec![RestoreScope::IndexOnly]);
    assert_eq!(app.status_message.as_deref(), Some("unstaged src/main.rs"));
}

#[test]
fn the_working_tree_view_restores_both_index_and_worktree() {
    let (mut app, log) = logged_app();
    app.open_confirm_restore();
    app.confirm_confirm_restore();

    assert_eq!(log.borrow().scopes, vec![RestoreScope::IndexAndWorktree]);
}

// -- Regression: a Visual anchor must never survive the refresh ------------

#[test]
fn confirming_from_visual_mode_drops_the_selection() {
    let (mut app, _log) = logged_app();
    app.rebuild_rows();
    let anchor = app.view.rows.len() - 1;
    app.view.cursor = anchor;
    app.mode = Mode::Visual { anchor };

    app.open_confirm_restore();
    app.confirm_confirm_restore();

    // The refresh rebuilds `view.rows`; a surviving anchor would index past
    // the end of the new list, where `visual_stage_selection` slices raw.
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn cancelling_from_visual_mode_keeps_the_selection() {
    let (mut app, _log) = logged_app();
    app.rebuild_rows();
    let anchor = app.view.rows.len() - 1;
    app.view.cursor = anchor;
    app.mode = Mode::Visual { anchor };

    app.open_confirm_restore();
    app.cancel_confirm_restore();

    // Cancelling runs no refresh, so the anchor still indexes live rows.
    assert_eq!(app.mode, Mode::Visual { anchor });
}

// -- Regression: untracked must mean untracked, not "has no patch" ---------

#[test]
fn a_fully_staged_file_with_no_patch_is_not_treated_as_untracked() {
    use super::super::stage_ops::StagedState;

    let log = Rc::new(RefCell::new(RestoreLog::default()));
    let mut app = App::new(vec![file("assets/logo.png")]);
    app.target = DiffTarget::WorkingTree;
    app.stage_ops = Some(Box::new(RecordingOps::new(log.clone())));
    // A staged binary file / staged deletion: tracked, fully staged, and
    // carrying a header-only placeholder with no patch.
    app.patches = vec![None];
    app.staged_states
        .insert("assets/logo.png".to_string(), StagedState::Full);
    app.recompute_untracked();

    assert!(
        app.untracked_paths.is_empty(),
        "a tracked, fully-staged file is not untracked: {:?}",
        app.untracked_paths
    );

    app.open_confirm_restore();
    app.confirm_confirm_restore();

    // It must be restored from HEAD, never deleted off disk.
    assert_eq!(log.borrow().restored, vec!["assets/logo.png".to_string()]);
    assert!(log.borrow().discarded.is_empty());
}

#[test]
fn a_genuinely_untracked_file_is_still_classified_untracked() {
    let log = Rc::new(RefCell::new(RestoreLog::default()));
    let mut app = App::new(vec![file("src/fresh.rs")]);
    app.target = DiffTarget::WorkingTree;
    app.stage_ops = Some(Box::new(RecordingOps::new(log.clone())));
    // Synthetic untracked entry: no patch, and no staged changes at all.
    app.patches = vec![None];
    app.recompute_untracked();

    assert_eq!(app.untracked_paths, vec!["src/fresh.rs".to_string()]);

    app.open_confirm_restore();
    app.confirm_confirm_restore();

    assert_eq!(log.borrow().discarded, vec!["src/fresh.rs".to_string()]);
    assert!(log.borrow().restored.is_empty());
}
