use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::diff::FileDiff;
use crate::git::{DiffTarget, FileStatus, GitError, RawFilePatch};
use crate::review::store::{ForgeMetadata, ForgeProviderKind};

use super::super::app::App;
use super::super::keymap::Action;
use super::super::stage_ops::{AsyncPrWebOpener, StageOps};

/// A `StageOps` fake whose only real behavior is a browser opener that
/// records the PR number it was handed instead of launching anything.
#[derive(Clone, Default)]
struct OpenerFake {
    opened: Arc<Mutex<Vec<(ForgeProviderKind, u64)>>>,
}

impl StageOps for OpenerFake {
    fn diff(&self, _target: &DiffTarget) -> Result<Vec<RawFilePatch>, GitError> {
        Ok(Vec::new())
    }
    fn status(&self) -> Result<Vec<FileStatus>, GitError> {
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
    fn async_pr_web_opener(&self, provider: ForgeProviderKind) -> Option<AsyncPrWebOpener> {
        let opened = Arc::clone(&self.opened);
        Some(Box::new(move |number| {
            opened.lock().unwrap().push((provider, number));
            Ok(())
        }))
    }
}

/// A one-hunk file, enough to build an `App` around.
fn file(path: &str) -> FileDiff {
    let raw = format!(
        "diff --git a/{path} b/{path}\nindex 1..2 100644\n--- a/{path}\n+++ b/{path}\n@@ -1,1 +1,1 @@\n-old\n+new\n"
    );
    FileDiff::from_patch(&RawFilePatch {
        path: path.to_string(),
        old_path: None,
        raw,
        is_binary: false,
    })
    .unwrap()
}

/// An app in a review session on `branch`, with `forge` metadata attached
/// (or none, for a local-branch review).
fn review_app(branch: &str, forge: Option<ForgeMetadata>) -> App {
    let mut app = App::new(vec![file("a.rs")]);
    app.target = DiffTarget::Review {
        base: "main".to_string(),
        branch: branch.to_string(),
    };
    app.review_forge = forge;
    app
}

fn forge_meta(provider: ForgeProviderKind, number: u64, title: &str) -> ForgeMetadata {
    ForgeMetadata {
        provider,
        host: "github.com".to_string(),
        number,
        title: title.to_string(),
        last_head_sha: "abc123".to_string(),
        diff_refs: None,
    }
}

/// Drives `poll_pr_web_open` until the in-flight open drains or a deadline
/// passes (the opener runs on its own thread).
fn drain(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.pr_web_in_flight.is_some() && Instant::now() < deadline {
        app.poll_pr_web_open();
        std::thread::sleep(Duration::from_millis(2));
    }
}

// -- the banner label --------------------------------------------------------

#[test]
fn banner_label_names_the_pr_not_the_managed_branch() {
    let app = review_app(
        "redquill/pr/7",
        Some(forge_meta(
            ForgeProviderKind::GitHub,
            7,
            "Fix the flaky test",
        )),
    );
    assert_eq!(app.review_banner_label(), "#7 Fix the flaky test");
}

#[test]
fn banner_label_falls_back_to_the_number_when_a_persisted_review_has_no_title() {
    // `ForgeMetadata::title` is omitted from the JSON when empty (state
    // persisted before the field existed), so the label must not render a
    // dangling "#7 ".
    let app = review_app(
        "redquill/pr/7",
        Some(forge_meta(ForgeProviderKind::GitHub, 7, "")),
    );
    assert_eq!(app.review_banner_label(), "#7");
}

#[test]
fn banner_label_is_the_branch_name_for_a_local_branch_review() {
    let app = review_app("feature/thing", None);
    assert_eq!(app.review_banner_label(), "feature/thing");
}

// -- `gx` --------------------------------------------------------------------

#[test]
fn opens_the_pr_under_review_through_the_sessions_own_provider() {
    let fake = OpenerFake::default();
    let mut app = review_app(
        "redquill/pr/7",
        Some(forge_meta(ForgeProviderKind::GitLab, 7, "Fix it")),
    );
    app.stage_ops = Some(Box::new(fake.clone()));

    app.apply(Action::OpenPrInBrowser);
    drain(&mut app);

    assert_eq!(
        *fake.opened.lock().unwrap(),
        vec![(ForgeProviderKind::GitLab, 7)],
        "the open must target the session's own number and provider"
    );
    assert_eq!(
        app.status_message.as_deref(),
        Some("opened #7 in your browser")
    );
}

#[test]
fn a_second_open_while_one_is_running_is_ignored() {
    let fake = OpenerFake::default();
    let mut app = review_app(
        "redquill/pr/7",
        Some(forge_meta(ForgeProviderKind::GitHub, 7, "Fix it")),
    );
    app.stage_ops = Some(Box::new(fake.clone()));

    app.apply(Action::OpenPrInBrowser);
    // Without draining, so the first open is still in flight.
    app.apply(Action::OpenPrInBrowser);
    drain(&mut app);

    assert_eq!(
        fake.opened.lock().unwrap().len(),
        1,
        "holding gx must not open a second browser tab per keypress"
    );
}

#[test]
fn open_pr_in_browser_outside_a_forge_review_spawns_nothing_and_says_so() {
    let fake = OpenerFake::default();
    let mut app = review_app("feature/thing", None);
    app.stage_ops = Some(Box::new(fake.clone()));

    app.apply(Action::OpenPrInBrowser);

    assert!(
        app.pr_web_in_flight.is_none(),
        "a local-branch review has no PR number to open"
    );
    assert!(fake.opened.lock().unwrap().is_empty());
    assert_eq!(
        app.status_message.as_deref(),
        Some("no PR under review \u{2014} nothing to open")
    );
}
