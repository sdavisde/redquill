use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::diff::FileDiff;
use crate::git::{DiffTarget, FileStatus, GitError, RawFilePatch};
use crate::review::store::{ForgeMetadata, ForgeProviderKind};

use super::super::app::App;
use super::super::keymap::Action;
use super::super::stage_ops::{AsyncWebOpener, StageOps};
use super::{WebTarget, WebTargetKind};

/// A `StageOps` fake whose only real behavior is a browser opener that
/// records what it was handed instead of launching anything.
#[derive(Clone, Default)]
struct OpenerFake {
    opened: Arc<Mutex<Vec<(ForgeProviderKind, WebTarget)>>>,
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
    fn async_web_opener(
        &self,
        provider: ForgeProviderKind,
        target: WebTarget,
    ) -> Option<AsyncWebOpener> {
        let opened = Arc::clone(&self.opened);
        Some(Box::new(move || {
            opened.lock().unwrap().push((provider, target.clone()));
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

fn app_on(target: DiffTarget, forge: Option<ForgeMetadata>) -> App {
    let mut app = App::new(vec![file("a.rs")]);
    app.target = target;
    app.review_forge = forge;
    app
}

fn review(branch: &str) -> DiffTarget {
    DiffTarget::Review {
        base: "main".to_string(),
        branch: branch.to_string(),
    }
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
    let app = app_on(
        review("redquill/pr/7"),
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
    let app = app_on(
        review("redquill/pr/7"),
        Some(forge_meta(ForgeProviderKind::GitHub, 7, "")),
    );
    assert_eq!(app.review_banner_label(), "#7");
}

#[test]
fn banner_label_is_the_branch_name_for_a_local_branch_review() {
    let app = app_on(review("feature/thing"), None);
    assert_eq!(app.review_banner_label(), "feature/thing");
}

// -- which view resolves to which target -------------------------------------

#[test]
fn web_target_follows_the_view() {
    let cases: Vec<(DiffTarget, Option<ForgeMetadata>, Option<WebTarget>)> = vec![
        (
            review("redquill/pr/7"),
            Some(forge_meta(ForgeProviderKind::GitHub, 7, "t")),
            Some(WebTarget::Pr(7)),
        ),
        (
            review("feature/thing"),
            None,
            Some(WebTarget::Branch("feature/thing".to_string())),
        ),
        (
            DiffTarget::Commit("abc123".to_string()),
            None,
            Some(WebTarget::Commit("abc123".to_string())),
        ),
        (DiffTarget::WorkingTree, None, None),
        (DiffTarget::Staged, None, None),
        (DiffTarget::Range("a..b".to_string()), None, None),
        (DiffTarget::File("a.rs".to_string()), None, None),
    ];
    for (target, forge, expected) in cases {
        let app = app_on(target.clone(), forge);
        assert_eq!(app.web_target(), expected, "target {target:?}");
    }
}

// -- `gx` --------------------------------------------------------------------

#[test]
fn opens_each_targets_own_value_through_the_sessions_provider() {
    let cases = [
        (
            review("redquill/pr/7"),
            Some(forge_meta(ForgeProviderKind::GitLab, 7, "t")),
            ForgeProviderKind::GitLab,
            WebTarget::Pr(7),
            "opened the PR in your browser",
        ),
        (
            review("feature/thing"),
            None,
            ForgeProviderKind::GitHub,
            WebTarget::Branch("feature/thing".to_string()),
            "opened the branch in your browser",
        ),
        (
            DiffTarget::Commit("abc123".to_string()),
            None,
            ForgeProviderKind::GitHub,
            WebTarget::Commit("abc123".to_string()),
            "opened the commit in your browser",
        ),
    ];
    for (target, forge, provider, expected_target, expected_message) in cases {
        let fake = OpenerFake::default();
        let mut app = app_on(target, forge);
        app.stage_ops = Some(Box::new(fake.clone()));

        app.apply(Action::OpenInBrowser);
        drain(&mut app);

        assert_eq!(
            *fake.opened.lock().unwrap(),
            vec![(provider, expected_target.clone())],
            "must open exactly {expected_target:?}"
        );
        assert_eq!(app.status_message.as_deref(), Some(expected_message));
    }
}

#[test]
fn a_second_open_while_one_is_running_is_ignored() {
    let fake = OpenerFake::default();
    let mut app = app_on(
        review("redquill/pr/7"),
        Some(forge_meta(ForgeProviderKind::GitHub, 7, "Fix it")),
    );
    app.stage_ops = Some(Box::new(fake.clone()));

    app.apply(Action::OpenInBrowser);
    // Without draining, so the first open is still in flight.
    app.apply(Action::OpenInBrowser);
    drain(&mut app);

    assert_eq!(
        fake.opened.lock().unwrap().len(),
        1,
        "holding gx must not open a second browser tab per keypress"
    );
}

#[test]
fn open_in_a_view_with_no_forge_counterpart_spawns_nothing_and_says_so() {
    let fake = OpenerFake::default();
    let mut app = app_on(DiffTarget::WorkingTree, None);
    app.stage_ops = Some(Box::new(fake.clone()));

    app.apply(Action::OpenInBrowser);

    assert!(app.pr_web_in_flight.is_none());
    assert!(fake.opened.lock().unwrap().is_empty());
    assert_eq!(
        app.status_message.as_deref(),
        Some("nothing here to open on the forge")
    );
}

/// The completion message names the target captured at spawn, not whatever
/// the view happens to be when the result lands — a refresh can reroot the
/// app mid-open, and "opened the PR" for a commit open would be a lie.
#[test]
fn the_completion_message_names_the_target_that_was_opened() {
    let fake = OpenerFake::default();
    let mut app = app_on(DiffTarget::Commit("abc123".to_string()), None);
    app.stage_ops = Some(Box::new(fake.clone()));

    app.apply(Action::OpenInBrowser);
    app.target = DiffTarget::WorkingTree;
    drain(&mut app);

    assert_eq!(
        app.status_message.as_deref(),
        Some("opened the commit in your browser")
    );
}

// -- labels ------------------------------------------------------------------

#[test]
fn each_target_kind_has_its_own_footer_label() {
    assert_eq!(WebTargetKind::Pr.label(), "open PR");
    assert_eq!(WebTargetKind::Branch.label(), "open branch");
    assert_eq!(WebTargetKind::Commit.label(), "open commit");
}
