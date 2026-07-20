//! Tests for the finished-review cleanup modal's state transitions and its
//! confirmed deletion sequence (fakes only; the real-git worktree/branch flow
//! is covered by the tempdir integration tests).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::diff::FileDiff;
use crate::git::{DiffTarget, GitError, LocalBranch, RawFilePatch};
use crate::review::FinishedReview;
use crate::review::store::{ForgeMetadata, ForgeProviderKind, PersistedReview, load, save_review};

use super::super::app::{App, Mode, ModeOrigin};
use super::super::review_launcher::LauncherTab;
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

fn finished(number: u64, worktree: &str, unpublished: usize) -> FinishedReview {
    FinishedReview {
        branch: format!("redquill/pr/{number}"),
        number,
        title: format!("PR {number}"),
        provider: ForgeProviderKind::GitHub,
        host: "github.com".to_string(),
        worktree_path: PathBuf::from(worktree),
        unpublished_count: unpublished,
    }
}

/// A recording backend for the cleanup deletion sequence: tracks the order of
/// worktree removes, prunes, and branch deletes, resolves the state path
/// through a tempdir git-common-dir, and can fail a specific worktree remove.
struct CleanupFake {
    common_dir: PathBuf,
    remove_calls: Rc<RefCell<Vec<PathBuf>>>,
    prune_calls: Rc<RefCell<usize>>,
    branch_deletes: Rc<RefCell<Vec<u64>>>,
    /// Worktree paths whose removal fails (a locked/dirty worktree).
    fail_removes: Vec<PathBuf>,
}

impl StageOps for CleanupFake {
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
    fn git_common_dir(&self) -> Result<PathBuf, GitError> {
        Ok(self.common_dir.clone())
    }
    fn managed_pr_branches(&self) -> Result<Vec<LocalBranch>, GitError> {
        // After a delete the caller re-reads via `recompute`, but these tests
        // drive `run_cleanup_deletions` and assert on the store directly, so a
        // static list is enough.
        Ok(Vec::new())
    }
    fn worktree_remove(&self, path: &Path) -> Result<(), GitError> {
        self.remove_calls.borrow_mut().push(path.to_path_buf());
        if self.fail_removes.iter().any(|p| p == path) {
            return Err(GitError::Command {
                command: format!("worktree remove {}", path.display()),
                code: "1".to_string(),
                stderr: "fatal: 'wt' contains modified or untracked files".to_string(),
            });
        }
        Ok(())
    }
    fn worktree_prune(&self) -> Result<(), GitError> {
        *self.prune_calls.borrow_mut() += 1;
        Ok(())
    }
    fn delete_managed_pr_branch(&self, number: u64) -> Result<(), GitError> {
        self.branch_deletes.borrow_mut().push(number);
        Ok(())
    }
}

fn prs_launcher_app() -> App {
    let mut app = App::new(vec![sample_file()]);
    app.mode = Mode::ReviewLauncher {
        tab: LauncherTab::PullRequests,
        cursor: 0,
        origin: ModeOrigin::Normal,
    };
    app
}

// -- open / cancel -----------------------------------------------------------

#[test]
fn open_from_prs_tab_with_finished_reviews_enters_cleanup_mode() {
    let mut app = prs_launcher_app();
    app.launcher_finished_reviews = vec![finished(1, "/tmp/wt1", 0)];

    app.open_cleanup_reviews();

    assert_eq!(
        app.mode,
        Mode::CleanupReviews {
            origin: ModeOrigin::Normal
        }
    );
    assert_eq!(
        app.cleanup_reviews.len(),
        1,
        "the snapshot is frozen at open"
    );
}

#[test]
fn open_with_no_finished_reviews_stays_in_launcher_with_a_message() {
    let mut app = prs_launcher_app();
    app.open_cleanup_reviews();
    assert!(matches!(app.mode, Mode::ReviewLauncher { .. }));
    assert!(app.status_message.is_some());
}

#[test]
fn open_is_a_no_op_off_the_prs_tab() {
    let mut app = App::new(vec![sample_file()]);
    app.mode = Mode::ReviewLauncher {
        tab: LauncherTab::Branches,
        cursor: 0,
        origin: ModeOrigin::Normal,
    };
    app.launcher_finished_reviews = vec![finished(1, "/tmp/wt1", 0)];
    app.open_cleanup_reviews();
    assert!(matches!(
        app.mode,
        Mode::ReviewLauncher {
            tab: LauncherTab::Branches,
            ..
        }
    ));
}

#[test]
fn cancel_returns_to_the_prs_tab_and_deletes_nothing() {
    let mut app = prs_launcher_app();
    app.launcher_finished_reviews = vec![finished(1, "/tmp/wt1", 0)];
    app.open_cleanup_reviews();

    app.cancel_cleanup_reviews();

    assert!(matches!(
        app.mode,
        Mode::ReviewLauncher {
            tab: LauncherTab::PullRequests,
            ..
        }
    ));
    assert!(app.cleanup_reviews.is_empty());
}

// -- confirmed deletion sequence ---------------------------------------------

#[allow(clippy::type_complexity)]
fn cleanup_app_with_store(
    common: &Path,
    entries: Vec<FinishedReview>,
    fail_removes: Vec<PathBuf>,
) -> (
    App,
    Rc<RefCell<Vec<PathBuf>>>,
    Rc<RefCell<usize>>,
    Rc<RefCell<Vec<u64>>>,
    PathBuf,
) {
    let state_path = common.join("redquill").join("review-state.json");
    // Persist a state entry per finished review so the delete has something to
    // remove; also add an unrelated review that must survive.
    for entry in &entries {
        save_review(
            &state_path,
            &entry.branch,
            PersistedReview {
                base: "main".to_string(),
                worktree_path: entry.worktree_path.clone(),
                files: BTreeMap::new(),
                annotations: Vec::new(),
                replies: Vec::new(),
                forge: Some(ForgeMetadata {
                    provider: ForgeProviderKind::GitHub,
                    host: "github.com".to_string(),
                    number: entry.number,
                    title: entry.title.clone(),
                    last_head_sha: "abc".to_string(),
                }),
            },
        )
        .unwrap();
    }
    save_review(
        &state_path,
        "keep-me",
        PersistedReview {
            base: "main".to_string(),
            worktree_path: PathBuf::from("/tmp/keep"),
            files: BTreeMap::new(),
            annotations: Vec::new(),
            replies: Vec::new(),
            forge: None,
        },
    )
    .unwrap();

    let remove_calls = Rc::new(RefCell::new(Vec::new()));
    let prune_calls = Rc::new(RefCell::new(0));
    let branch_deletes = Rc::new(RefCell::new(Vec::new()));
    let fake = CleanupFake {
        common_dir: common.to_path_buf(),
        remove_calls: Rc::clone(&remove_calls),
        prune_calls: Rc::clone(&prune_calls),
        branch_deletes: Rc::clone(&branch_deletes),
        fail_removes,
    };

    let mut app = prs_launcher_app();
    app.stage_ops = Some(Box::new(fake));
    app.launcher_finished_reviews = entries;
    app.open_cleanup_reviews();

    (app, remove_calls, prune_calls, branch_deletes, state_path)
}

#[test]
fn confirm_deletes_worktree_branch_and_state_entry_per_review() {
    let tmp = tempfile::TempDir::new().unwrap();
    let entries = vec![finished(1, "/tmp/wt1", 0), finished(2, "/tmp/wt2", 0)];
    let (mut app, removes, prunes, branch_deletes, state_path) =
        cleanup_app_with_store(tmp.path(), entries, Vec::new());

    app.confirm_cleanup_reviews();

    assert_eq!(
        removes.borrow().as_slice(),
        [PathBuf::from("/tmp/wt1"), PathBuf::from("/tmp/wt2")],
        "each worktree removed, in order"
    );
    assert_eq!(
        *prunes.borrow(),
        2,
        "prune runs after each successful remove"
    );
    assert_eq!(branch_deletes.borrow().as_slice(), [1, 2]);

    let state = load(&state_path);
    assert!(!state.reviews.contains_key("redquill/pr/1"));
    assert!(!state.reviews.contains_key("redquill/pr/2"));
    assert!(
        state.reviews.contains_key("keep-me"),
        "an unrelated review must survive cleanup"
    );
    assert!(matches!(app.mode, Mode::ReviewLauncher { .. }));
    assert!(
        app.status_message
            .as_deref()
            .is_some_and(|m| m.contains("cleaned up 2")),
        "summary names how many were cleaned: {:?}",
        app.status_message
    );
}

#[test]
fn a_failed_worktree_remove_continues_to_the_next_entry_with_a_summary() {
    let tmp = tempfile::TempDir::new().unwrap();
    let entries = vec![finished(1, "/tmp/wt1", 0), finished(2, "/tmp/wt2", 0)];
    // The first entry's worktree is locked/dirty and fails.
    let (mut app, removes, _prunes, branch_deletes, state_path) =
        cleanup_app_with_store(tmp.path(), entries, vec![PathBuf::from("/tmp/wt1")]);

    app.confirm_cleanup_reviews();

    assert_eq!(
        removes.borrow().len(),
        2,
        "the run continues to the second entry after the first fails"
    );
    assert_eq!(
        branch_deletes.borrow().as_slice(),
        [2],
        "only the successfully-removed entry's branch is deleted"
    );
    let state = load(&state_path);
    assert!(
        state.reviews.contains_key("redquill/pr/1"),
        "a failed remove leaves that entry's state untouched"
    );
    assert!(!state.reviews.contains_key("redquill/pr/2"));
    assert!(
        app.status_message
            .as_deref()
            .is_some_and(|m| m.contains("1 cleaned") && m.contains("1 failed")),
        "summary reports the split: {:?}",
        app.status_message
    );
}

#[test]
fn decline_after_open_leaves_the_store_intact() {
    let tmp = tempfile::TempDir::new().unwrap();
    let entries = vec![finished(1, "/tmp/wt1", 0)];
    let (mut app, removes, _prunes, _branch_deletes, state_path) =
        cleanup_app_with_store(tmp.path(), entries, Vec::new());

    app.cancel_cleanup_reviews();

    assert!(removes.borrow().is_empty(), "decline runs no deletion");
    let state = load(&state_path);
    assert!(state.reviews.contains_key("redquill/pr/1"));
}
