//! Real-git integration tests for the PR-checkout flow (spec 13 Unit 2,
//! FR-7..FR-11), driven through [`App::spawn_pr_checkout`] +
//! [`App::poll_pr_checkout`] against a `file://` bare "origin" that
//! advertises GitHub-style `refs/pull/<n>/head` refs — the same hermetic
//! fixture shape `tests/git_pr_fetch_integration.rs` uses, but exercising the
//! whole checkout-into-a-review-session path (fetch → worktree → reconcile →
//! reroot) rather than just the git layer. Everything is a local bare repo
//! under our own control: no network, no real forge, no `gh`.
//!
//! These live in-crate (not `tests/*.rs`) for the same reason
//! `review_launcher_integration_tests.rs` documents: the checkout entry
//! points are `pub(super)`, invisible to an out-of-crate test binary. Every
//! fixture is built with `tempfile`; the shared isolation guard runs before
//! every redquill mutating call, per the 2026-07-16 tempdir-leak incident.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use tempfile::TempDir;

use super::app::{App, Mode, ModeOrigin};
use super::keymap::Action;
use super::review_launcher::LauncherTab;
use super::stage_ops::{PrFetchOutcome, build_review};
use crate::forge::PullRequest;
use crate::git::{DiffTarget, GitRunner};
use crate::review::store::ForgeProviderKind;
use crate::review::{ReviewStatus, store};

// -- Fixtures ---------------------------------------------------------------

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

fn canon(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|e| panic!("canonicalize {path:?}: {e}"))
}

/// Fails loudly if a discovered repo root ever resolved outside its own
/// tempdir (local copy per this repo's one-copy-per-file rule).
fn assert_repo_root_inside_tempdir(runner: &GitRunner, tmp: &TempDir) {
    let root = canon(runner.root());
    let tmp_root = canon(tmp.path());
    assert!(
        root.starts_with(&tmp_root),
        "refusing to run a mutating git call outside the tempdir: {root:?} is not under {tmp_root:?}"
    );
}

/// Builds a bare "origin" with a single committed file on `main`.
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

/// Clones `bare` into a fresh tempdir with a local identity configured.
fn clone_of(bare: &Path) -> TempDir {
    let dest = TempDir::new().unwrap();
    git(dest.path(), &["clone", "-q", &file_url(bare), "."]);
    configure_identity(dest.path());
    dest
}

/// Pushes a branch off `main` (uniquely modifying `base.txt`) to
/// `refs/pull/<number>/head` — a GitHub-style special ref with **no**
/// matching `refs/heads/<branch>` on origin, exactly like a fork PR. Returns
/// the head SHA. Leaves `contributor` back on `main`.
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

/// An `App` with a real `GitRunner` rooted at `dir`, wired like `main.rs`.
fn app_rooted_at(dir: &Path) -> App {
    let runner = GitRunner::discover_in(dir).expect("discover repo");
    let snapshot = build_review(&runner, &DiffTarget::WorkingTree).expect("build review");
    let mut app = App::with_git(snapshot, DiffTarget::WorkingTree, Box::new(runner.clone()));
    app.set_repo_root(runner.root().to_path_buf());
    app
}

/// Drains the background PR checkout until it lands (or a 10s deadline —
/// generous for a `file://` fetch plus a local worktree add).
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

/// Drains pending review-state saves (blob-SHA persistence runs on a
/// background thread).
fn wait_for_review_save(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.review_saves_pending > 0 && Instant::now() < deadline {
        app.poll_review_save();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.review_saves_pending, 0, "review save did not complete");
}

// -- Happy path (FR-7) ------------------------------------------------------

#[test]
fn checkout_lands_in_a_pr_review_session_against_a_fork_style_head() {
    let bare = setup_bare_origin();
    let contributor = clone_of(bare.path());
    let feature_sha = push_pr_special_ref(contributor.path(), "feature", 1, "from the pr");

    let reviewer = clone_of(bare.path());
    let runner = GitRunner::discover_in(reviewer.path()).unwrap();
    assert_repo_root_inside_tempdir(&runner, &reviewer);
    // Sanity: this is genuinely fork-style — origin advertises no branch for
    // the PR head, only the magic ref.
    let branches = git_out(reviewer.path(), &["branch", "-r"]);
    assert!(
        !branches.contains("origin/feature"),
        "fixture must be fork-style (no origin/feature branch): {branches:?}"
    );

    let mut app = app_rooted_at(reviewer.path());
    app.spawn_pr_checkout(
        1,
        "main".to_string(),
        "github.com".to_string(),
        "add a feature".to_string(),
        ForgeProviderKind::GitHub,
        false,
    );
    drain_pr_checkout(&mut app);

    // The review session is live on the managed branch, diffed against
    // origin/main — exactly the spec-08 shape, reached without the user
    // touching git.
    assert_eq!(
        app.target,
        DiffTarget::Review {
            base: "origin/main".to_string(),
            branch: "redquill/pr/1".to_string(),
        }
    );
    assert!(app.in_review_session());

    // The managed branch checked out to the PR head exactly.
    let managed_sha = git_out(reviewer.path(), &["rev-parse", "redquill/pr/1"]);
    assert_eq!(managed_sha, feature_sha);

    // The forge block is stamped and persisted with the fetched head SHA.
    let forge = app.review_forge.as_ref().expect("forge metadata stamped");
    assert_eq!(forge.number, 1);
    assert_eq!(forge.host, "github.com");
    assert_eq!(forge.last_head_sha, feature_sha);

    wait_for_review_save(&mut app);
    let state_path = app.review_state_path.clone().unwrap();
    let persisted = store::load(&state_path);
    let review = persisted
        .reviews
        .get("redquill/pr/1")
        .expect("PR review persisted under its managed branch key");
    let pf = review.forge.as_ref().expect("persisted forge block");
    assert_eq!(pf.number, 1);
    assert_eq!(pf.last_head_sha, feature_sha);
    assert_eq!(pf.provider, ForgeProviderKind::GitHub);
}

/// The GitLab analog of [`push_pr_special_ref`]: pushes a branch off `main`
/// to `refs/merge-requests/<iid>/head` — a GitLab-style magic ref with no
/// matching `refs/heads/<branch>` on origin. Returns the head SHA.
fn push_mr_special_ref(contributor: &Path, branch: &str, iid: u64, line: &str) -> String {
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
            &format!("{branch}:refs/merge-requests/{iid}/head"),
        ],
    );
    git(contributor, &["checkout", "-q", "main"]);
    sha
}

// -- GitLab provider path (FR-25, Units 1-2 unchanged) ----------------------

#[test]
fn gitlab_mr_checkout_lands_in_a_review_session_via_the_merge_requests_ref() {
    // The exact Unit-2 flow, but the provider is GitLab: the head fetch must
    // resolve `refs/merge-requests/<iid>/head` and land the same
    // worktree-backed session on `redquill/pr/<iid>`, proving the provider is
    // the only variable.
    let bare = setup_bare_origin();
    let contributor = clone_of(bare.path());
    let mr_sha = push_mr_special_ref(contributor.path(), "feature", 7, "from the mr");

    let reviewer = clone_of(bare.path());
    let runner = GitRunner::discover_in(reviewer.path()).unwrap();
    assert_repo_root_inside_tempdir(&runner, &reviewer);

    let mut app = app_rooted_at(reviewer.path());
    app.spawn_pr_checkout(
        7,
        "main".to_string(),
        "gitlab.com".to_string(),
        "add a feature".to_string(),
        ForgeProviderKind::GitLab,
        false,
    );
    drain_pr_checkout(&mut app);

    assert_eq!(
        app.target,
        DiffTarget::Review {
            base: "origin/main".to_string(),
            branch: "redquill/pr/7".to_string(),
        }
    );
    assert!(app.in_review_session());
    let managed_sha = git_out(reviewer.path(), &["rev-parse", "redquill/pr/7"]);
    assert_eq!(managed_sha, mr_sha);

    let forge = app.review_forge.as_ref().expect("forge metadata stamped");
    assert_eq!(forge.provider, ForgeProviderKind::GitLab);
    assert_eq!(forge.number, 7);
    assert_eq!(forge.host, "gitlab.com");
    assert_eq!(forge.last_head_sha, mr_sha);

    wait_for_review_save(&mut app);
    let state_path = app.review_state_path.clone().unwrap();
    let persisted = store::load(&state_path);
    let pf = persisted
        .reviews
        .get("redquill/pr/7")
        .and_then(|r| r.forge.as_ref())
        .expect("persisted GitLab forge block");
    assert_eq!(pf.provider, ForgeProviderKind::GitLab);
    assert_eq!(pf.last_head_sha, mr_sha);
}

// -- Head-move on reopen (FR-9) ---------------------------------------------

#[test]
fn author_push_on_reopen_recreates_the_worktree_and_demotes_accepted_files() {
    let bare = setup_bare_origin();
    let contributor = clone_of(bare.path());
    push_pr_special_ref(contributor.path(), "feature", 1, "first version");

    let reviewer = clone_of(bare.path());
    let runner = GitRunner::discover_in(reviewer.path()).unwrap();
    assert_repo_root_inside_tempdir(&runner, &reviewer);

    let mut app = app_rooted_at(reviewer.path());
    app.spawn_pr_checkout(
        1,
        "main".to_string(),
        "github.com".to_string(),
        "the feature".to_string(),
        ForgeProviderKind::GitHub,
        false,
    );
    drain_pr_checkout(&mut app);

    // Accept the one changed file (base.txt), persisting its blob SHA.
    app.select_file_by_path("base.txt");
    app.apply(Action::ToggleAccept);
    assert_eq!(app.review_status("base.txt"), ReviewStatus::Accepted);
    wait_for_review_save(&mut app);

    // The author pushes a new commit that further changes base.txt.
    git(contributor.path(), &["checkout", "-q", "feature"]);
    write(
        contributor.path(),
        "base.txt",
        "line one\nfirst version\nsecond version\n",
    );
    git(contributor.path(), &["commit", "-aqm", "author push"]);
    let second_sha = git_out(contributor.path(), &["rev-parse", "HEAD"]);
    git(
        contributor.path(),
        &["push", "-qf", "origin", "feature:refs/pull/1/head"],
    );

    // Reopen mid-session via the refresh action (FR-9's fetch-on-open).
    app.manual_refresh();
    drain_pr_checkout(&mut app);

    // The managed branch moved to the new head, and the accepted file whose
    // blob changed dropped back to ChangedSinceAccepted.
    assert_eq!(
        git_out(reviewer.path(), &["rev-parse", "redquill/pr/1"]),
        second_sha
    );
    assert_eq!(
        app.review_status("base.txt"),
        ReviewStatus::ChangedSinceAccepted,
        "an accepted file whose blob changed must demote on reopen"
    );
    assert!(
        app.status_message
            .as_deref()
            .is_some_and(|m| m.contains("PR updated") && m.contains('1')),
        "the update line must report the demotion count, got {:?}",
        app.status_message
    );
    assert_eq!(app.review_forge.as_ref().unwrap().last_head_sha, second_sha);
    assert!(!app.review_stale, "a successful refresh is never stale");
}

#[test]
fn reopen_with_no_author_push_keeps_accepts_and_reports_no_change() {
    let bare = setup_bare_origin();
    let contributor = clone_of(bare.path());
    push_pr_special_ref(contributor.path(), "feature", 2, "only version");

    let reviewer = clone_of(bare.path());
    let mut app = app_rooted_at(reviewer.path());
    app.spawn_pr_checkout(
        2,
        "main".to_string(),
        "github.com".to_string(),
        "steady feature".to_string(),
        ForgeProviderKind::GitHub,
        false,
    );
    drain_pr_checkout(&mut app);
    app.select_file_by_path("base.txt");
    app.apply(Action::ToggleAccept);
    wait_for_review_save(&mut app);

    // Refresh with no upstream change: the accept survives, nothing demotes.
    app.manual_refresh();
    drain_pr_checkout(&mut app);

    assert_eq!(app.review_status("base.txt"), ReviewStatus::Accepted);
    assert!(!app.review_stale);
}

// -- Re-entry idempotency (draft duplication regression) --------------------

use crate::annotate::{Classification, Target};

/// Seeds one line annotation and one draft reply into the live session and
/// persists them, so a subsequent re-entry has a disk copy to (wrongly)
/// re-append.
fn seed_one_annotation_and_reply(app: &mut App) {
    app.annotations
        .add(
            Target::file("base.txt"),
            Classification::Issue,
            "please fix",
        )
        .unwrap();
    app.replies.add(42, "agreed, will fix");
    app.persist_review_state();
    wait_for_review_save(app);
}

/// A manual refresh with no author push must not duplicate the session's
/// draft annotations or replies — the live stores already hold them, so the
/// re-entry must leave counts and ids untouched (regression for the
/// state-duplication bug found dogfooding PR review).
#[test]
fn manual_refresh_without_a_head_move_does_not_duplicate_drafts() {
    let bare = setup_bare_origin();
    let contributor = clone_of(bare.path());
    push_pr_special_ref(contributor.path(), "feature", 1, "only version");

    let reviewer = clone_of(bare.path());
    let mut app = app_rooted_at(reviewer.path());
    app.spawn_pr_checkout(
        1,
        "main".to_string(),
        "github.com".to_string(),
        "feature".to_string(),
        ForgeProviderKind::GitHub,
        false,
    );
    drain_pr_checkout(&mut app);

    seed_one_annotation_and_reply(&mut app);
    let annotation_ids: Vec<usize> = app.annotations.iter().map(|a| a.id).collect();
    let reply_ids: Vec<usize> = app.replies.iter().map(|r| r.id).collect();
    assert_eq!(annotation_ids.len(), 1);
    assert_eq!(reply_ids.len(), 1);

    // Refresh with no upstream change: fetch-on-open re-enters the session.
    app.manual_refresh();
    drain_pr_checkout(&mut app);
    wait_for_review_save(&mut app);

    assert_eq!(
        app.annotations.iter().map(|a| a.id).collect::<Vec<_>>(),
        annotation_ids,
        "a no-op refresh must not duplicate annotations"
    );
    assert_eq!(
        app.replies.iter().map(|r| r.id).collect::<Vec<_>>(),
        reply_ids,
        "a no-op refresh must not duplicate replies"
    );

    // And the durable copy stays single, not doubled.
    let state_path = app.review_state_path.clone().unwrap();
    let review = store::load(&state_path)
        .reviews
        .remove("redquill/pr/1")
        .unwrap();
    assert_eq!(review.annotations.len(), 1, "disk must hold one annotation");
    assert_eq!(review.replies.len(), 1, "disk must hold one reply");
}

/// A refresh that recreates the worktree on an author push must demote the
/// changed file (head-move path still works) *and* leave the session's
/// drafts single, not doubled.
#[test]
fn head_move_refresh_demotes_but_does_not_duplicate_drafts() {
    let bare = setup_bare_origin();
    let contributor = clone_of(bare.path());
    push_pr_special_ref(contributor.path(), "feature", 1, "first version");

    let reviewer = clone_of(bare.path());
    let mut app = app_rooted_at(reviewer.path());
    app.spawn_pr_checkout(
        1,
        "main".to_string(),
        "github.com".to_string(),
        "feature".to_string(),
        ForgeProviderKind::GitHub,
        false,
    );
    drain_pr_checkout(&mut app);

    app.select_file_by_path("base.txt");
    app.apply(Action::ToggleAccept);
    seed_one_annotation_and_reply(&mut app);

    // The author pushes a new commit onto the PR head.
    git(contributor.path(), &["checkout", "-q", "feature"]);
    write(
        contributor.path(),
        "base.txt",
        "line one\nfirst version\nsecond version\n",
    );
    git(contributor.path(), &["commit", "-aqm", "author push"]);
    git(
        contributor.path(),
        &["push", "-qf", "origin", "feature:refs/pull/1/head"],
    );

    app.manual_refresh();
    drain_pr_checkout(&mut app);
    wait_for_review_save(&mut app);

    assert_eq!(
        app.review_status("base.txt"),
        ReviewStatus::ChangedSinceAccepted,
        "the head-move demotion must still fire"
    );
    assert_eq!(
        app.annotations.len(),
        1,
        "a worktree-recreate refresh must not duplicate annotations"
    );
    assert_eq!(
        app.replies.len(),
        1,
        "a worktree-recreate refresh must not duplicate replies"
    );
    let state_path = app.review_state_path.clone().unwrap();
    let review = store::load(&state_path)
        .reviews
        .remove("redquill/pr/1")
        .unwrap();
    assert_eq!(review.annotations.len(), 1);
    assert_eq!(review.replies.len(), 1);
}

/// Quit-and-relaunch idempotency: a fresh app reopening the same PR restores
/// exactly one copy of each persisted draft, and re-persisting leaves the
/// on-disk annotation/reply lists identical — no growth across the cycle.
#[test]
fn relaunch_and_reenter_is_idempotent_across_a_persist_cycle() {
    let bare = setup_bare_origin();
    let contributor = clone_of(bare.path());
    push_pr_special_ref(contributor.path(), "feature", 5, "only version");
    let reviewer = clone_of(bare.path());

    // First app: check out, draft, persist, then drop as if quitting.
    let state_path;
    let annotations_before;
    let replies_before;
    {
        let mut app = app_rooted_at(reviewer.path());
        app.spawn_pr_checkout(
            5,
            "main".to_string(),
            "github.com".to_string(),
            "feature".to_string(),
            ForgeProviderKind::GitHub,
            false,
        );
        drain_pr_checkout(&mut app);
        seed_one_annotation_and_reply(&mut app);
        state_path = app.review_state_path.clone().unwrap();
        let review = store::load(&state_path)
            .reviews
            .remove("redquill/pr/5")
            .unwrap();
        annotations_before = review.annotations;
        replies_before = review.replies;
        assert_eq!(annotations_before.len(), 1);
        assert_eq!(replies_before.len(), 1);
    }

    // Second app: relaunch and reopen the same PR from scratch.
    let mut app2 = app_rooted_at(reviewer.path());
    app2.spawn_pr_checkout(
        5,
        "main".to_string(),
        "github.com".to_string(),
        "feature".to_string(),
        ForgeProviderKind::GitHub,
        false,
    );
    drain_pr_checkout(&mut app2);
    assert_eq!(app2.annotations.len(), 1, "reopen restores one annotation");
    assert_eq!(app2.replies.len(), 1, "reopen restores one reply");
    wait_for_review_save(&mut app2);

    let review_after = store::load(&state_path)
        .reviews
        .remove("redquill/pr/5")
        .unwrap();
    assert_eq!(
        review_after.annotations, annotations_before,
        "the persisted annotations must be identical across a relaunch+persist cycle"
    );
    assert_eq!(
        review_after.replies, replies_before,
        "the persisted replies must be identical across a relaunch+persist cycle"
    );
}

// -- Fetch failure (FR-10) --------------------------------------------------

#[test]
fn fetch_failure_mid_session_labels_the_checkout_stale_and_touches_nothing() {
    let bare = setup_bare_origin();
    let contributor = clone_of(bare.path());
    push_pr_special_ref(contributor.path(), "feature", 1, "the version");

    let reviewer = clone_of(bare.path());
    let mut app = app_rooted_at(reviewer.path());
    app.spawn_pr_checkout(
        1,
        "main".to_string(),
        "github.com".to_string(),
        "feature".to_string(),
        ForgeProviderKind::GitHub,
        false,
    );
    drain_pr_checkout(&mut app);
    app.select_file_by_path("base.txt");
    app.apply(Action::ToggleAccept);
    wait_for_review_save(&mut app);
    let target_before = app.target.clone();
    let status_before = app.review_status("base.txt");

    // Origin goes away (offline): the reopen peek fails.
    drop(bare);

    app.manual_refresh();
    drain_pr_checkout(&mut app);

    assert!(
        app.review_stale,
        "a failed reopen fetch with a live worktree must flag the session stale"
    );
    assert_eq!(app.target, target_before, "target must be untouched");
    assert_eq!(
        app.review_status("base.txt"),
        status_before,
        "the accept must survive an offline reopen untouched"
    );
    assert!(
        app.status_message
            .as_deref()
            .is_some_and(|m| m.to_lowercase().contains("stale") || m.contains("failed")),
        "a diagnostic must surface, got {:?}",
        app.status_message
    );
}

#[test]
fn reopen_from_the_launcher_after_a_fetch_failure_enters_a_stale_session() {
    let bare = setup_bare_origin();
    let contributor = clone_of(bare.path());
    push_pr_special_ref(contributor.path(), "feature", 3, "the version");

    let reviewer = clone_of(bare.path());
    // First app: check out to create the managed worktree + persisted state.
    {
        let mut app = app_rooted_at(reviewer.path());
        app.spawn_pr_checkout(
            3,
            "main".to_string(),
            "github.com".to_string(),
            "feature".to_string(),
            ForgeProviderKind::GitHub,
            false,
        );
        drain_pr_checkout(&mut app);
        wait_for_review_save(&mut app);
        assert!(app.in_review_session());
    }

    // Origin goes away, then a fresh app reopens the same PR from the
    // launcher: the peek fails, but the prior worktree survives, so the
    // reviewer still enters — clearly labeled stale.
    drop(bare);
    let mut app2 = app_rooted_at(reviewer.path());
    app2.spawn_pr_checkout(
        3,
        "main".to_string(),
        "github.com".to_string(),
        "feature".to_string(),
        ForgeProviderKind::GitHub,
        false,
    );
    drain_pr_checkout(&mut app2);

    assert!(
        app2.in_review_session(),
        "the stale worktree is still reviewable"
    );
    assert!(app2.review_stale, "the stale entry must be labeled stale");
    assert_eq!(
        app2.target,
        DiffTarget::Review {
            base: "origin/main".to_string(),
            branch: "redquill/pr/3".to_string(),
        }
    );
}

// -- Journey transcript (spec 13 task 2.0 proof) ----------------------------

/// Journey generator for spec 13 task 2.0: on a real scratch repo whose
/// `origin` advertises a GitHub-style `refs/pull/1/head` ref, opens the
/// launcher on the Pull Requests tab (the list itself is seeded, since no
/// real `gh` runs in-crate), presses `Enter` on the PR to land in a full
/// review session via the real checkout, accepts a file, then simulates the
/// author pushing a new commit and refreshes — showing the "PR updated — N
/// accepted file(s) changed" line and the demoted file. Captured with
/// `RQ_JOURNEY_DUMP=1 cargo test --lib pr_checkout_journey_transcript --
/// --nocapture`.
#[test]
fn pr_checkout_journey_transcript() {
    let mut log = String::new();
    let mut step = |title: &str, body: &str| {
        log.push_str(&format!("\n=== {title} ===\n{body}\n"));
    };

    let bare = setup_bare_origin();
    let contributor = clone_of(bare.path());
    let first_sha = push_pr_special_ref(contributor.path(), "feature", 1, "first version");
    step(
        "journey: a scratch origin advertising a GitHub-style PR head ref",
        &format!(
            "origin holds refs/pull/1/head at {first_sha} (no origin/feature branch — fork-style)"
        ),
    );

    let reviewer = clone_of(bare.path());
    let mut app = app_rooted_at(reviewer.path());

    // Open the launcher on the Pull Requests tab with the PR listed (the
    // listing is seeded — no real `gh` runs in-crate; the checkout below is
    // fully real).
    app.mode = Mode::ReviewLauncher {
        tab: LauncherTab::PullRequests,
        cursor: 0,
        origin: ModeOrigin::Normal,
    };
    app.launcher_prs = Some(PrFetchOutcome::Loaded {
        repo_label: "sdavisde/redquill".to_string(),
        prs: vec![PullRequest {
            number: 1,
            title: "Add a feature".to_string(),
            author: "octocat".to_string(),
            head_ref: "feature".to_string(),
            base_ref: "main".to_string(),
            is_draft: false,
            updated_at: "2026-07-19T00:00:00Z".to_string(),
        }],
    });
    step(
        "R -> Pull Requests tab: #1 Add a feature is listed",
        "launcher open on the Pull Requests tab, cursor on PR #1",
    );

    // Enter on the PR row: real fetch + worktree + reroot into a session.
    app.review_launcher_confirm();
    drain_pr_checkout(&mut app);
    assert!(app.in_review_session());
    step(
        "Enter on #1: landed in a worktree-backed review session",
        &format!(
            "target: {:?}\nforge: #{} on {} @ {}",
            app.target,
            app.review_forge.as_ref().unwrap().number,
            app.review_forge.as_ref().unwrap().host,
            &app.review_forge.as_ref().unwrap().last_head_sha[..12],
        ),
    );

    // Accept the changed file.
    app.select_file_by_path("base.txt");
    app.apply(Action::ToggleAccept);
    wait_for_review_save(&mut app);
    step(
        "accept base.txt",
        &format!("base.txt status: {:?}", app.review_status("base.txt")),
    );

    // The author pushes a new commit onto the PR head.
    git(contributor.path(), &["checkout", "-q", "feature"]);
    write(
        contributor.path(),
        "base.txt",
        "line one\nfirst version\nsecond version\n",
    );
    git(contributor.path(), &["commit", "-aqm", "author push"]);
    let second_sha = git_out(contributor.path(), &["rev-parse", "HEAD"]);
    git(
        contributor.path(),
        &["push", "-qf", "origin", "feature:refs/pull/1/head"],
    );
    step(
        "author pushes a new commit to the PR",
        &format!("refs/pull/1/head advanced {first_sha} -> {second_sha}"),
    );

    // Refresh mid-session: fetch-on-open detects the move and demotes.
    app.manual_refresh();
    drain_pr_checkout(&mut app);
    step(
        "refresh: fetch-on-open detects the author push",
        &format!(
            "status: {:?}\nbase.txt status: {:?}",
            app.status_message,
            app.review_status("base.txt"),
        ),
    );

    assert_eq!(
        app.review_status("base.txt"),
        ReviewStatus::ChangedSinceAccepted
    );
    assert!(
        app.status_message
            .as_deref()
            .is_some_and(|m| m.contains("PR updated"))
    );

    if std::env::var("RQ_JOURNEY_DUMP").is_ok() {
        eprintln!("{log}");
    }
    drop(bare);
}

#[test]
fn first_checkout_offline_leaves_no_session_and_surfaces_a_diagnostic() {
    let bare = setup_bare_origin();
    let contributor = clone_of(bare.path());
    push_pr_special_ref(contributor.path(), "feature", 9, "the version");

    let reviewer = clone_of(bare.path());
    let mut app = app_rooted_at(reviewer.path());
    // Origin unreachable before any checkout: a first-open fetch fails with
    // no prior worktree to fall back to.
    drop(bare);

    app.spawn_pr_checkout(
        9,
        "main".to_string(),
        "github.com".to_string(),
        "feature".to_string(),
        ForgeProviderKind::GitHub,
        false,
    );
    drain_pr_checkout(&mut app);

    assert!(
        !app.in_review_session(),
        "a failed first checkout starts no session"
    );
    assert!(app.review_forge.is_none());
    assert!(
        app.status_message
            .as_deref()
            .is_some_and(|m| m.contains("failed")),
        "a one-line diagnostic must surface, got {:?}",
        app.status_message
    );
    // Nothing was created locally.
    let verify = Command::new("git")
        .current_dir(reviewer.path())
        .args(["rev-parse", "--verify", "redquill/pr/9"])
        .output()
        .unwrap();
    assert!(
        !verify.status.success(),
        "no managed branch may exist after a failed first fetch"
    );
}
