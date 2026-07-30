use std::time::{Duration, Instant};

use crate::diff::FileDiff;
use crate::forge::PullRequest;
use crate::git::{DiffTarget, FileStatus, GitError, RawFilePatch};
use crate::review::store::{ForgeMetadata, ForgeProviderKind};

use super::super::app::{App, Mode, ModeOrigin};
use super::super::review_launcher::LauncherTab;
use super::super::stage_ops::{AsyncPrDetailFetcher, PrFetchOutcome, StageOps};
use super::*;

// -- fixtures ----------------------------------------------------------------

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

/// How a fake's detail fetcher resolves.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FakeDetail {
    /// A successful read whose body names the PR number it was asked for.
    Ok,
    /// A failed read (the offline / unauthenticated / non-zero-exit class).
    Err,
    /// A backend with no fetcher at all (a git-less context).
    NoFetcher,
}

/// A `StageOps` fake whose only real behavior is the description fetcher.
#[derive(Clone, Copy)]
struct DetailFake(FakeDetail);

impl StageOps for DetailFake {
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
    fn async_pr_detail_fetcher(
        &self,
        _provider: ForgeProviderKind,
    ) -> Option<AsyncPrDetailFetcher> {
        match self.0 {
            FakeDetail::NoFetcher => None,
            FakeDetail::Err => Some(Box::new(|_| Err("offline".to_string()))),
            FakeDetail::Ok => Some(Box::new(|number| {
                Ok(PrDetail {
                    number,
                    title: format!("title {number}"),
                    author: "octocat".to_string(),
                    base_ref: "main".to_string(),
                    head_ref: "feature".to_string(),
                    body: format!("body of {number}"),
                    is_draft: false,
                    updated_at: "2026-07-18T12:34:56Z".to_string(),
                })
            })),
        }
    }
}

fn pr(number: u64) -> PullRequest {
    PullRequest {
        number,
        title: format!("title {number}"),
        author: "octocat".to_string(),
        head_ref: "feature".to_string(),
        base_ref: "main".to_string(),
        is_draft: false,
        updated_at: "2026-07-18T12:34:56Z".to_string(),
    }
}

/// An `App` sitting on the launcher's Pull Requests tab over `numbers`, with
/// `detail` deciding how a description read resolves.
fn launcher_app(numbers: &[u64], detail: FakeDetail) -> App {
    let mut app = App::new(vec![file("a.rs")]);
    app.stage_ops = Some(Box::new(DetailFake(detail)));
    app.launcher_prs = Some(PrFetchOutcome::Loaded {
        repo_label: "org/repo".to_string(),
        prs: numbers.iter().copied().map(pr).collect(),
    });
    app.mode = Mode::ReviewLauncher {
        tab: LauncherTab::PullRequests,
        cursor: 0,
        origin: ModeOrigin::Normal,
    };
    app
}

/// An `App` mid-PR-review of `number`.
fn session_app(number: u64, detail: FakeDetail) -> App {
    let mut app = App::new(vec![file("a.rs")]);
    app.stage_ops = Some(Box::new(DetailFake(detail)));
    app.target = DiffTarget::Review {
        base: "origin/main".to_string(),
        branch: format!("redquill/pr/{number}"),
    };
    app.review_forge = Some(ForgeMetadata {
        provider: ForgeProviderKind::GitHub,
        host: "github.com".to_string(),
        number,
        title: format!("title {number}"),
        last_head_sha: "abc".to_string(),
        diff_refs: None,
    });
    app
}

/// Polls until `number`'s detail lands in the cache (or a short deadline
/// passes), the same drain-until-landed loop the thread-fetch tests use.
fn drain_until_cached(app: &mut App, number: u64) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        app.poll_pr_detail();
        if app.pr_detail_for(number).is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

// -- opening from the launcher ------------------------------------------------

#[test]
fn launcher_details_opens_the_overlay_on_the_highlighted_pr() {
    let mut app = launcher_app(&[41, 42, 43], FakeDetail::Ok);
    app.review_launcher_move_down();
    app.open_pr_description_from_launcher();

    assert!(matches!(
        app.mode,
        Mode::PrDescription {
            ret: PrDescriptionReturn::Launcher {
                cursor: 1,
                origin: ModeOrigin::Normal
            }
        }
    ));
    assert_eq!(
        app.pr_description.as_ref().map(|s| s.number),
        Some(42),
        "the overlay must open on the highlighted row's PR, not the first row's"
    );
}

/// The gesture belongs to the Pull Requests tab: pressing it on Branches (or
/// Commits) must leave the launcher exactly as it was, not open an overlay on
/// whatever PR happens to be cached.
#[test]
fn launcher_details_is_inert_off_the_pull_requests_tab() {
    for tab in [LauncherTab::Branches, LauncherTab::Commits] {
        let mut app = launcher_app(&[42], FakeDetail::Ok);
        app.mode = Mode::ReviewLauncher {
            tab,
            cursor: 0,
            origin: ModeOrigin::Normal,
        };
        app.open_pr_description_from_launcher();
        assert!(
            matches!(app.mode, Mode::ReviewLauncher { .. }),
            "{tab:?}: must stay in the launcher"
        );
        assert!(app.pr_description.is_none(), "{tab:?}");
    }
}

#[test]
fn launcher_details_is_inert_on_an_empty_listing() {
    let mut app = launcher_app(&[], FakeDetail::Ok);
    app.open_pr_description_from_launcher();
    assert!(matches!(app.mode, Mode::ReviewLauncher { .. }));
    assert!(app.pr_description.is_none());
}

#[test]
fn closing_from_the_launcher_returns_to_the_pull_requests_tab_on_the_same_row() {
    let mut app = launcher_app(&[41, 42], FakeDetail::Ok);
    app.review_launcher_move_down();
    app.open_pr_description_from_launcher();
    app.close_pr_description();

    assert_eq!(
        app.mode,
        Mode::ReviewLauncher {
            tab: LauncherTab::PullRequests,
            cursor: 1,
            origin: ModeOrigin::Normal,
        }
    );
    assert!(app.pr_description.is_none());
}

// -- opening from a review session --------------------------------------------

#[test]
fn session_chord_opens_the_overlay_on_the_pr_under_review() {
    let mut app = session_app(34, FakeDetail::Ok);
    app.apply(crate::ui::keymap::Action::OpenPrDescription);

    assert!(matches!(
        app.mode,
        Mode::PrDescription {
            ret: PrDescriptionReturn::Session
        }
    ));
    assert_eq!(app.pr_description.as_ref().map(|s| s.number), Some(34));
}

/// A local-branch review has no PR behind it: the chord must degrade to a
/// status hint rather than opening an overlay with nothing to show.
#[test]
fn session_chord_on_a_branch_review_hints_instead_of_opening() {
    let mut app = session_app(34, FakeDetail::Ok);
    app.review_forge = None;
    app.apply(crate::ui::keymap::Action::OpenPrDescription);

    assert_eq!(app.mode, Mode::Normal);
    assert!(app.pr_description.is_none());
    assert!(
        app.status_message
            .as_deref()
            .is_some_and(|m| m.contains("no PR under review"))
    );
}

#[test]
fn closing_from_a_session_returns_to_the_diff() {
    let mut app = session_app(34, FakeDetail::Ok);
    app.open_pr_description_in_session();
    app.close_pr_description();
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.pr_description.is_none());
}

// -- Enter: starts the review from the launcher only ---------------------------

/// `Enter` in a session-opened overlay must do nothing at all — the PR is
/// already under review, and re-running the launcher's confirm from here would
/// try to start a nested one.
#[test]
fn enter_is_a_no_op_when_the_overlay_was_opened_from_a_session() {
    let mut app = session_app(34, FakeDetail::Ok);
    app.open_pr_description_in_session();
    app.pr_description_confirm();

    assert!(matches!(
        app.mode,
        Mode::PrDescription {
            ret: PrDescriptionReturn::Session
        }
    ));
    assert_eq!(app.pr_description.as_ref().map(|s| s.number), Some(34));
}

/// `Enter` from a launcher-opened overlay routes back through the launcher's
/// own confirm — including its guards. With a review already in progress that
/// guard refuses and says so, which is the observable proof the confirm ran
/// (rather than the keypress being swallowed by the overlay).
#[test]
fn enter_from_the_launcher_runs_the_tabs_own_confirm_guards() {
    let mut app = launcher_app(&[42], FakeDetail::Ok);
    app.target = DiffTarget::Review {
        base: "origin/main".to_string(),
        branch: "feature".to_string(),
    };
    app.open_pr_description_from_launcher();
    app.pr_description_confirm();

    assert!(
        matches!(app.mode, Mode::ReviewLauncher { .. }),
        "the launcher must be back on screen, not the overlay"
    );
    assert!(app.pr_description.is_none());
    assert!(
        app.status_message
            .as_deref()
            .is_some_and(|m| m.contains("already reviewing")),
        "the launcher's in-session guard must have run: {:?}",
        app.status_message
    );
}

// -- the fetch: caching, degradation, staleness --------------------------------

#[test]
fn a_successful_read_caches_the_detail_under_its_own_number() {
    let mut app = launcher_app(&[42], FakeDetail::Ok);
    app.open_pr_description_from_launcher();
    drain_until_cached(&mut app, 42);

    let Some(PrDetailOutcome::Ready(detail)) = app.pr_detail_for(42) else {
        panic!("expected a cached detail, got {:?}", app.pr_detail_for(42));
    };
    assert_eq!(detail.number, 42);
    assert_eq!(detail.body, "body of 42");
    assert!(
        app.pr_detail_for(41).is_none(),
        "no other PR's number may gain an entry"
    );
}

/// A failed read (offline, unauthenticated, non-zero exit) must land as
/// `Unavailable` — never a status-line error, never a modal, and never a
/// permanently-loading overlay.
#[test]
fn a_failed_read_caches_unavailable_without_interrupting_the_reviewer() {
    let mut app = launcher_app(&[42], FakeDetail::Err);
    app.open_pr_description_from_launcher();
    drain_until_cached(&mut app, 42);

    assert_eq!(app.pr_detail_for(42), Some(&PrDetailOutcome::Unavailable));
    assert!(
        app.status_message.is_none(),
        "a failed description read must not surface a message: {:?}",
        app.status_message
    );
    assert!(matches!(app.mode, Mode::PrDescription { .. }));
}

/// A backend that can't produce a fetcher at all must resolve immediately
/// rather than leaving the overlay stuck on "loading…" forever.
#[test]
fn a_backend_with_no_fetcher_resolves_straight_to_unavailable() {
    let mut app = launcher_app(&[42], FakeDetail::NoFetcher);
    app.open_pr_description_from_launcher();
    assert_eq!(app.pr_detail_for(42), Some(&PrDetailOutcome::Unavailable));
    assert!(app.pr_detail_in_flight.is_none());
}

/// Reopening a PR whose detail is already cached must not spawn a second read.
#[test]
fn reopening_a_cached_pr_spawns_no_second_read() {
    let mut app = launcher_app(&[42], FakeDetail::Ok);
    app.open_pr_description_from_launcher();
    drain_until_cached(&mut app, 42);
    let generation = app.pr_detail_generation;

    app.close_pr_description();
    app.open_pr_description_from_launcher();

    assert_eq!(
        app.pr_detail_generation, generation,
        "a cached PR must not start another fetch"
    );
    assert!(app.pr_detail_in_flight.is_none());
}

/// A result tagged with a superseded generation (a straggler from an overlay
/// that has since been closed and reopened on another PR) must be dropped
/// rather than written into the cache.
#[test]
fn poll_drops_a_stale_generation_result() {
    let mut app = launcher_app(&[42], FakeDetail::Ok);
    let stale_generation = app.pr_detail_generation;
    let id = app.pr_detail_tasks.spawn(|| {
        Ok(PrDetail {
            number: 42,
            ..PrDetail::default()
        })
    });
    app.pr_detail_in_flight = Some(InFlightPrDetailFetch {
        id,
        generation: stale_generation,
        number: 42,
    });
    // A newer open bumped the generation past the in-flight one.
    app.pr_detail_generation = app.pr_detail_generation.wrapping_add(1);

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        app.poll_pr_detail();
        if app.pr_detail_in_flight.is_none() {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        app.pr_detail_for(42),
        None,
        "a stale-generation result must be dropped, not cached"
    );
}

/// A read that lands after the overlay closed must update nothing but the
/// cache: it may not reopen the overlay or move the mode.
#[test]
fn a_read_landing_after_the_overlay_closed_does_not_reopen_it() {
    let mut app = launcher_app(&[42], FakeDetail::Ok);
    app.open_pr_description_from_launcher();
    app.close_pr_description();
    let mode_before = app.mode;
    drain_until_cached(&mut app, 42);

    assert_eq!(
        app.mode, mode_before,
        "the landing read must not change mode"
    );
    assert!(app.pr_description.is_none());
    assert!(
        matches!(app.pr_detail_for(42), Some(PrDetailOutcome::Ready(_))),
        "the body still belongs in the cache for the next open"
    );
}

// -- scrolling ----------------------------------------------------------------

#[test]
fn scroll_down_advances_the_offset_and_scroll_up_retreats_it_no_further_than_zero() {
    let mut app = session_app(34, FakeDetail::Ok);
    app.open_pr_description_in_session();

    app.pr_description_scroll_down();
    app.pr_description_scroll_down();
    assert_eq!(app.pr_description.as_ref().map(|s| s.scroll.get()), Some(2));

    app.pr_description_scroll_up();
    app.pr_description_scroll_up();
    app.pr_description_scroll_up();
    assert_eq!(app.pr_description.as_ref().map(|s| s.scroll.get()), Some(0));
}
