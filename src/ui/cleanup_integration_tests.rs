//! Real-git integration tests for the finished-review cleanup flow (spec 13
//! Unit 5, FR-22..FR-24), driven through [`App::open_cleanup_reviews`] +
//! [`App::confirm_cleanup_reviews`] against a `file://` bare "origin" that
//! advertises GitHub-style `refs/pull/<n>/head` refs. A PR is checked out into
//! a real worktree-backed session, the review-state is persisted, then a fresh
//! `App` rooted at the origin (outside the managed worktree) cross-references
//! the managed branches against a listing that no longer includes the PR and
//! cleans it up — verifying the worktree, branch, and state entry are gone via
//! real `git worktree list` / `git branch` / the state file. Everything is a
//! local bare repo under our own control: no network, no real forge, no `gh`.
//!
//! In-crate (not `tests/*.rs`) because the cleanup entry points are
//! `pub(super)`. Every fixture is built with `tempfile`; the shared isolation
//! guard runs before mutating calls, per the 2026-07-16 tempdir-leak incident.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use tempfile::TempDir;

use super::app::{App, Mode, ModeOrigin};
use super::review_launcher::LauncherTab;
use super::stage_ops::{PrFetchOutcome, build_review};
use crate::annotate::{Classification, PersistedAnnotation, Side, Source, Target};
use crate::forge::PullRequest;
use crate::git::{DiffTarget, GitRunner};
use crate::review::store::{self, ForgeProviderKind, PersistedReview};

// -- Fixtures (local copies, per the one-copy-per-file convention) -----------

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("failed to spawn git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_out(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("failed to spawn git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn write(dir: &Path, rel: &str, contents: &str) {
    std::fs::write(dir.join(rel), contents).unwrap();
}

fn configure_identity(dir: &Path) {
    git(dir, &["config", "user.email", "test@redquill.invalid"]);
    git(dir, &["config", "user.name", "redquill test"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
}

fn file_url(bare: &Path) -> String {
    format!("file://{}", bare.display())
}

fn setup_bare_origin() -> TempDir {
    let bare = TempDir::new().unwrap();
    git(bare.path(), &["init", "-q", "--bare", "-b", "main"]);

    let seed = TempDir::new().unwrap();
    git(seed.path(), &["init", "-q", "-b", "main"]);
    configure_identity(seed.path());
    write(seed.path(), "base.txt", "line one\n");
    git(seed.path(), &["add", "."]);
    git(seed.path(), &["commit", "-q", "-m", "initial"]);
    git(
        seed.path(),
        &["remote", "add", "origin", &file_url(bare.path())],
    );
    git(seed.path(), &["push", "-q", "-u", "origin", "main"]);
    bare
}

fn clone_of(bare: &Path) -> TempDir {
    let dest = TempDir::new().unwrap();
    git(dest.path(), &["clone", "-q", &file_url(bare), "."]);
    configure_identity(dest.path());
    dest
}

fn push_pr_special_ref(contributor: &Path, branch: &str, number: u64, line: &str) -> String {
    git(contributor, &["checkout", "-qb", branch, "main"]);
    write(contributor, "base.txt", &format!("line one\n{line}\n"));
    git(contributor, &["commit", "-aqm", branch]);
    let sha = git_out(contributor, &["rev-parse", "HEAD"]);
    git(
        contributor,
        &[
            "push",
            "-q",
            "origin",
            &format!("{branch}:refs/pull/{number}/head"),
        ],
    );
    git(contributor, &["checkout", "-q", "main"]);
    sha
}

fn app_rooted_at(dir: &Path) -> App {
    let runner = GitRunner::discover_in(dir).expect("discover repo");
    let snapshot = build_review(&runner, &DiffTarget::WorkingTree).expect("build review");
    let mut app = App::with_git(snapshot, DiffTarget::WorkingTree, Box::new(runner.clone()));
    app.set_repo_root(runner.root().to_path_buf());
    app
}

fn drain_pr_checkout(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.pr_checkout_in_flight.is_some() && Instant::now() < deadline {
        app.poll_pr_checkout();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        app.pr_checkout_in_flight.is_none(),
        "PR checkout did not complete in time"
    );
}

fn wait_for_review_save(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.review_saves_pending > 0 && Instant::now() < deadline {
        app.poll_review_save();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.review_saves_pending, 0, "review save did not complete");
}

/// Checks out PR `number` into a real managed worktree + persisted state
/// entry, using a throwaway session app (dropped afterwards). Leaves the
/// worktree, branch, and state on disk for a later origin-rooted app to clean
/// up.
fn create_pr_checkout(reviewer: &Path, number: u64) {
    let mut app = app_rooted_at(reviewer);
    app.spawn_pr_checkout(
        number,
        "main".to_string(),
        "github.com".to_string(),
        format!("PR {number}"),
        ForgeProviderKind::GitHub,
        false,
    );
    drain_pr_checkout(&mut app);
    wait_for_review_save(&mut app);
    assert!(
        app.in_review_session(),
        "checkout {number} must land a session"
    );
}

/// A fresh `App` rooted at the origin (outside every managed worktree, as when
/// the launcher is opened at the origin repo), on the Pull Requests tab, with
/// `open_numbers` seeded as the current open listing and the finished set
/// recomputed against the managed branches + persisted state.
fn origin_app_with_open_listing(reviewer: &Path, open_numbers: &[u64]) -> App {
    let mut app = app_rooted_at(reviewer);
    app.mode = Mode::ReviewLauncher {
        tab: LauncherTab::PullRequests,
        cursor: 0,
        origin: ModeOrigin::Normal,
    };
    let prs = open_numbers
        .iter()
        .map(|&n| PullRequest {
            number: n,
            title: format!("PR {n}"),
            author: "octocat".to_string(),
            head_ref: format!("feature-{n}"),
            base_ref: "main".to_string(),
            is_draft: false,
            updated_at: "2026-07-19T00:00:00Z".to_string(),
        })
        .collect();
    app.launcher_prs = Some(PrFetchOutcome::Loaded {
        repo_label: "sdavisde/redquill".to_string(),
        prs,
    });
    app.recompute_launcher_finished_reviews();
    app
}

fn state_path_of(reviewer: &Path) -> PathBuf {
    let runner = GitRunner::discover_in(reviewer).unwrap();
    runner
        .git_common_dir()
        .unwrap()
        .join("redquill")
        .join("review-state.json")
}

fn branch_exists(reviewer: &Path, branch: &str) -> bool {
    Command::new("git")
        .current_dir(reviewer)
        .args(["rev-parse", "--verify", "-q", branch])
        .output()
        .unwrap()
        .status
        .success()
}

fn worktree_list(reviewer: &Path) -> String {
    git_out(reviewer, &["worktree", "list"])
}

// -- Confirm path (FR-23) ----------------------------------------------------

#[test]
fn confirm_deletes_the_worktree_branch_and_state_entry() {
    let bare = setup_bare_origin();
    let contributor = clone_of(bare.path());
    push_pr_special_ref(contributor.path(), "feature", 1, "the change");

    let reviewer = clone_of(bare.path());
    create_pr_checkout(reviewer.path(), 1);

    // Before: the managed worktree, branch, and state entry all exist.
    assert!(branch_exists(reviewer.path(), "redquill/pr/1"));
    assert!(worktree_list(reviewer.path()).contains("redquill/pr/1"));
    assert!(
        store::load(&state_path_of(reviewer.path()))
            .reviews
            .contains_key("redquill/pr/1")
    );

    // PR #1 is no longer open (empty listing), so it is a finished review.
    let mut app = origin_app_with_open_listing(reviewer.path(), &[]);
    assert_eq!(app.launcher_finished_reviews.len(), 1);
    assert_eq!(app.launcher_finished_reviews[0].number, 1);

    app.open_cleanup_reviews();
    assert!(matches!(app.mode, Mode::CleanupReviews { .. }));
    app.confirm_cleanup_reviews();

    // After: worktree, branch, and state entry are gone; the finished count
    // is back to zero.
    assert!(
        !branch_exists(reviewer.path(), "redquill/pr/1"),
        "the managed branch must be deleted"
    );
    assert!(
        !worktree_list(reviewer.path()).contains("redquill/pr/1"),
        "the managed worktree must be removed"
    );
    assert!(
        !store::load(&state_path_of(reviewer.path()))
            .reviews
            .contains_key("redquill/pr/1"),
        "the state entry must be deleted"
    );
    assert!(app.launcher_finished_reviews.is_empty());
    assert!(
        app.status_message
            .as_deref()
            .is_some_and(|m| m.contains("cleaned up 1")),
        "summary must report the cleanup: {:?}",
        app.status_message
    );
    drop(bare);
}

// -- Decline path (FR-23) ----------------------------------------------------

#[test]
fn declining_leaves_everything_intact() {
    let bare = setup_bare_origin();
    let contributor = clone_of(bare.path());
    push_pr_special_ref(contributor.path(), "feature", 1, "the change");

    let reviewer = clone_of(bare.path());
    create_pr_checkout(reviewer.path(), 1);

    let mut app = origin_app_with_open_listing(reviewer.path(), &[]);
    app.open_cleanup_reviews();
    app.cancel_cleanup_reviews();

    assert!(
        branch_exists(reviewer.path(), "redquill/pr/1"),
        "declining must not delete the branch"
    );
    assert!(worktree_list(reviewer.path()).contains("redquill/pr/1"));
    assert!(
        store::load(&state_path_of(reviewer.path()))
            .reviews
            .contains_key("redquill/pr/1")
    );
    drop(bare);
}

// -- Unpublished-annotation warning (FR-23) ----------------------------------

#[test]
fn an_unpublished_annotation_is_surfaced_in_the_finished_review() {
    let bare = setup_bare_origin();
    let contributor = clone_of(bare.path());
    push_pr_special_ref(contributor.path(), "feature", 1, "the change");

    let reviewer = clone_of(bare.path());
    create_pr_checkout(reviewer.path(), 1);

    // Rewrite the persisted entry to carry one unpublished annotation.
    let state_path = state_path_of(reviewer.path());
    let existing = store::load(&state_path)
        .reviews
        .get("redquill/pr/1")
        .cloned()
        .unwrap();
    store::save_review(
        &state_path,
        "redquill/pr/1",
        PersistedReview {
            annotations: vec![PersistedAnnotation {
                target: Target::line("base.txt", 2, Side::New),
                classification: Classification::Issue,
                body: "not yet submitted".to_string(),
                source: Source::WorkingTree,
                published: false,
            }],
            files: BTreeMap::new(),
            ..existing
        },
    )
    .unwrap();

    let app = origin_app_with_open_listing(reviewer.path(), &[]);
    assert_eq!(app.launcher_finished_reviews.len(), 1);
    assert_eq!(
        app.launcher_finished_reviews[0].unpublished_count, 1,
        "the persisted unpublished annotation must be counted"
    );
    drop(bare);
}

// -- Per-entry failure continuation (FR-24) ----------------------------------

#[test]
fn a_dirty_worktree_fails_that_entry_and_the_run_continues() {
    let bare = setup_bare_origin();
    let contributor = clone_of(bare.path());
    push_pr_special_ref(contributor.path(), "feat1", 1, "one");
    push_pr_special_ref(contributor.path(), "feat2", 2, "two");

    let reviewer = clone_of(bare.path());
    create_pr_checkout(reviewer.path(), 1);
    create_pr_checkout(reviewer.path(), 2);

    // Dirty PR #1's worktree so `git worktree remove` (never --force) refuses.
    let wt1 = store::load(&state_path_of(reviewer.path()))
        .reviews
        .get("redquill/pr/1")
        .unwrap()
        .worktree_path
        .clone();
    std::fs::write(wt1.join("base.txt"), "locally modified, uncommitted\n").unwrap();

    let mut app = origin_app_with_open_listing(reviewer.path(), &[]);
    assert_eq!(app.launcher_finished_reviews.len(), 2);
    app.open_cleanup_reviews();
    app.confirm_cleanup_reviews();

    // #1 failed: its worktree, branch, and state entry survive.
    assert!(
        branch_exists(reviewer.path(), "redquill/pr/1"),
        "the dirty entry's branch must survive"
    );
    assert!(
        store::load(&state_path_of(reviewer.path()))
            .reviews
            .contains_key("redquill/pr/1")
    );
    // #2 succeeded: fully cleaned up.
    assert!(
        !branch_exists(reviewer.path(), "redquill/pr/2"),
        "the clean entry must still be cleaned up despite #1 failing"
    );
    assert!(
        !store::load(&state_path_of(reviewer.path()))
            .reviews
            .contains_key("redquill/pr/2")
    );
    assert!(
        app.status_message
            .as_deref()
            .is_some_and(|m| m.contains("1 cleaned") && m.contains("1 failed")),
        "summary must report the per-entry split: {:?}",
        app.status_message
    );
    drop(bare);
}

// -- Journey transcript (spec 13 task 5.0 proof) -----------------------------

/// Journey generator for spec 13 task 5.0: on a real scratch repo, checks out
/// a PR into a managed worktree, then — from a fresh origin-rooted launcher
/// whose listing no longer includes that PR — cleans it up via the confirm
/// modal, capturing `git worktree list`, `git branch`, and review-state.json
/// before and after. Captured with `RQ_JOURNEY_DUMP=1 cargo test --lib
/// finished_review_cleanup_journey_transcript -- --nocapture`.
#[test]
fn finished_review_cleanup_journey_transcript() {
    let mut log = String::new();
    let mut step = |title: &str, body: &str| {
        log.push_str(&format!("\n=== {title} ===\n{body}\n"));
    };

    let bare = setup_bare_origin();
    let contributor = clone_of(bare.path());
    push_pr_special_ref(contributor.path(), "feature", 1, "the change");
    let reviewer = clone_of(bare.path());
    create_pr_checkout(reviewer.path(), 1);

    let state_path = state_path_of(reviewer.path());
    step(
        "before: a reviewed PR whose worktree, branch, and state all exist",
        &format!(
            "git worktree list:\n{}\n\ngit branch:\n{}\n\nreview-state.json:\n{}",
            worktree_list(reviewer.path()),
            git_out(reviewer.path(), &["branch"]),
            std::fs::read_to_string(&state_path).unwrap(),
        ),
    );

    // The PR has since merged/closed: the launcher's listing no longer holds
    // it, so it surfaces as a finished review.
    let mut app = origin_app_with_open_listing(reviewer.path(), &[]);
    step(
        "R -> Pull Requests tab: #1 is no longer open",
        &format!(
            "finished reviews: {}",
            app.launcher_finished_reviews
                .iter()
                .map(|f| format!("#{} ({})", f.number, f.branch))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    );

    // X -> confirm modal -> confirm the deletion.
    app.open_cleanup_reviews();
    app.confirm_cleanup_reviews();
    step(
        "X -> confirm -> delete: the finished review is cleaned up",
        &format!("summary: {:?}", app.status_message),
    );

    step(
        "after: worktree, branch, and state entry are gone",
        &format!(
            "git worktree list:\n{}\n\ngit branch:\n{}\n\nreview-state.json:\n{}",
            worktree_list(reviewer.path()),
            git_out(reviewer.path(), &["branch"]),
            std::fs::read_to_string(&state_path).unwrap(),
        ),
    );

    assert!(!branch_exists(reviewer.path(), "redquill/pr/1"));
    assert!(app.launcher_finished_reviews.is_empty());

    if std::env::var("RQ_JOURNEY_DUMP").is_ok() {
        eprintln!("{log}");
    }
    drop(bare);
}
