//! Tests for the finished-review cleanup modal's open/cancel state
//! transitions and per-entry selection (cursor move, toggle, zero-selected
//! confirm); the confirmed deletion sequence — including a partial-selection
//! subset — is covered by the real-git tempdir integration tests.

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
            origin: ModeOrigin::Normal,
            cursor: 0,
        }
    );
    assert_eq!(
        app.cleanup_reviews.len(),
        1,
        "the snapshot is frozen at open"
    );
    assert_eq!(
        app.cleanup_reviews_selected,
        vec![true],
        "every entry starts selected"
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
    assert!(app.cleanup_reviews_selected.is_empty());
}

// -- Per-entry selection: cursor + toggle ------------------------------------

fn opened_cleanup_app(count: u64) -> App {
    let mut app = prs_launcher_app();
    app.launcher_finished_reviews = (1..=count)
        .map(|n| finished(n, &format!("/tmp/wt{n}"), 0))
        .collect();
    app.open_cleanup_reviews();
    app
}

#[test]
fn space_toggles_the_highlighted_entry_and_the_count_reflects_it() {
    let mut app = opened_cleanup_app(3);
    assert_eq!(
        app.cleanup_reviews_selected_count(),
        3,
        "all selected at open"
    );

    app.toggle_cleanup_review_selection();

    assert_eq!(app.cleanup_reviews_selected, vec![false, true, true]);
    assert_eq!(app.cleanup_reviews_selected_count(), 2);

    // Toggling again flips it back.
    app.toggle_cleanup_review_selection();
    assert_eq!(app.cleanup_reviews_selected, vec![true, true, true]);
}

#[test]
fn move_down_and_up_walk_the_cursor_and_toggle_follows_it() {
    let mut app = opened_cleanup_app(3);

    app.cleanup_reviews_move_down();
    app.toggle_cleanup_review_selection();

    assert_eq!(
        app.cleanup_reviews_selected,
        vec![true, false, true],
        "toggle must act on entry 1 (where the cursor moved to), not entry 0"
    );

    app.cleanup_reviews_move_up();
    app.toggle_cleanup_review_selection();
    assert_eq!(
        app.cleanup_reviews_selected,
        vec![false, false, true],
        "moving back up and toggling must act on entry 0 again"
    );
}

#[test]
fn cursor_movement_is_clamped_at_both_ends() {
    let mut app = opened_cleanup_app(2);
    assert_eq!(
        app.mode,
        Mode::CleanupReviews {
            origin: ModeOrigin::Normal,
            cursor: 0,
        }
    );

    app.cleanup_reviews_move_up();
    assert_eq!(
        app.mode,
        Mode::CleanupReviews {
            origin: ModeOrigin::Normal,
            cursor: 0,
        },
        "moving up from the first row stays pinned at 0"
    );

    app.cleanup_reviews_move_down();
    app.cleanup_reviews_move_down();
    app.cleanup_reviews_move_down();
    assert_eq!(
        app.mode,
        Mode::CleanupReviews {
            origin: ModeOrigin::Normal,
            cursor: 1,
        },
        "moving past the last row stays pinned at the last index"
    );
}

#[test]
fn zero_selected_confirm_is_a_no_op() {
    let mut app = opened_cleanup_app(1);
    app.toggle_cleanup_review_selection();
    assert_eq!(app.cleanup_reviews_selected_count(), 0);

    app.confirm_cleanup_reviews();

    assert!(
        matches!(app.mode, Mode::CleanupReviews { .. }),
        "the modal must stay open with nothing selected"
    );
    assert_eq!(
        app.cleanup_reviews.len(),
        1,
        "the snapshot must be untouched by a no-op confirm"
    );
    assert!(app.status_message.is_none(), "no summary for a no-op");
}
