//! Tests for the finished-review cleanup modal's open/cancel state
//! transitions; the confirmed deletion sequence is covered by the real-git
//! tempdir integration tests.

use std::path::PathBuf;

use crate::diff::FileDiff;
use crate::git::RawFilePatch;
use crate::review::FinishedReview;
use crate::review::store::ForgeProviderKind;

use super::super::app::{App, Mode, ModeOrigin};
use super::super::review_launcher::LauncherTab;

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
