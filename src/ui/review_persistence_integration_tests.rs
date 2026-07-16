//! Real-git, real-worktree integration tests for spec 08 Unit 4 (review
//! progress survives sessions and self-invalidates when files change),
//! task 4.6: a two-"session" scenario (resume → staleness → re-accept →
//! finish) plus a GC-of-a-deleted-branch scenario.
//!
//! Lives beside `commit_integration_tests.rs`/`review_guard_integration_tests.rs`
//! for the identical reason those files document: `dispatch_key`,
//! `open_end_review_modal`, and `finish_review` are crate-internal by
//! design, so a `tests/*.rs` binary could not drive them; living here keeps
//! the coverage genuinely end-to-end (a real managed worktree, real `git`
//! subprocesses through the real background poller, real key dispatch)
//! without widening the public API for a test's sake. "Session" here means
//! a fresh `App`/`GitRunner` built over the *same on-disk worktree and
//! state file* — the same thing a second `redquill --review <branch>`
//! process would see — not a literal second OS process.
//!
//! Every fixture is built with `tempfile`; every mutating git call is
//! preceded by `assert_inside_tempdir` (a local copy of the shared
//! isolation guard `tests/git_review_integration.rs` introduced in task
//! 1.5, duplicated here per this repo's established one-copy-per-file
//! convention).

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tempfile::TempDir;

use super::stage_ops::build_review;
use super::*;
use crate::git::{DiffTarget, GitRunner};
use crate::review::ReviewStatus;
use crate::review::store;

// -- Fixture helpers ----------------------------------------------------------

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

fn canon(path: &Path) -> std::path::PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|e| panic!("canonicalize {path:?}: {e}"))
}

/// The shared isolation guard every mutating git call in this file runs
/// before touching disk (task 1.5's convention).
fn assert_inside_tempdir(path: &Path, tmp: &TempDir) {
    let tmp_root = canon(tmp.path());
    let mut probe = path.to_path_buf();
    while !probe.exists() {
        match probe.parent() {
            Some(parent) => probe = parent.to_path_buf(),
            None => panic!("path {path:?} has no existing ancestor to canonicalize"),
        }
    }
    let probe_canon = canon(&probe);
    assert!(
        probe_canon.starts_with(&tmp_root),
        "refusing to run a mutating git call outside the tempdir: {path:?} (resolved ancestor {probe_canon:?}) is not under {tmp_root:?}"
    );
}

/// A repo on `main` with one commit, plus a `feature` branch (not checked
/// out — main stays the primary checkout's active branch throughout, the
/// "user's own checkout" spec 08 promises stays untouched) three commits
/// ahead, each touching one of `a.rs`/`b.rs`/`c.rs`.
fn repo_with_feature_branch_three_files() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    assert_inside_tempdir(dir, &tmp);
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.email", "test@redquill.invalid"]);
    git(dir, &["config", "user.name", "redquill test"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
    write(dir, "base.txt", "line one\n");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "initial"]);

    git(dir, &["branch", "feature"]);
    let wt = TempDir::new().unwrap();
    let wt_path = wt.path().join("seed");
    assert_inside_tempdir(&wt_path, &wt);
    git(
        dir,
        &[
            "worktree",
            "add",
            "-q",
            wt_path.to_str().unwrap(),
            "feature",
        ],
    );
    for (name, content) in [
        ("a.rs", "fn a() {}\n"),
        ("b.rs", "fn b() {}\n"),
        ("c.rs", "fn c() {}\n"),
    ] {
        write(&wt_path, name, content);
    }
    git(&wt_path, &["add", "."]);
    git(&wt_path, &["commit", "-qm", "add a.rs, b.rs, c.rs"]);
    git(dir, &["worktree", "remove", wt_path.to_str().unwrap()]);

    tmp
}

/// Builds a review-session `App` the same way `main.rs`'s bootstrap does:
/// `build_review` + `App::with_git` + `set_repo_root`, rooted at `runner`.
/// Does *not* set the state path or origin ops — callers wire those
/// themselves so each "session" controls exactly what it seeds.
fn app_for_worktree(runner: &GitRunner, target: DiffTarget) -> App {
    let snapshot = build_review(runner, &target).expect("build review");
    let mut app = App::with_git(snapshot, target, Box::new(runner.clone()));
    app.set_repo_root(runner.root().to_path_buf());
    app
}

fn press(app: &mut App, keymap: &Keymap, pending: &mut Option<KeyEvent>, code: KeyCode) {
    dispatch_key(
        app,
        keymap,
        pending,
        &mut None,
        KeyEvent::new(code, KeyModifiers::NONE),
    );
}

fn wait_for_review_save(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.review_saves_pending > 0 && Instant::now() < deadline {
        app.poll_review_save();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        app.review_saves_pending, 0,
        "review save did not complete in time"
    );
}

// -- Two-session scenario: resume -> staleness -> re-accept -> finish -------

#[test]
fn resume_staleness_re_accept_and_finish_round_trip_against_real_git() {
    let repo = repo_with_feature_branch_three_files();
    let discovered = GitRunner::discover_in(repo.path()).expect("discover repo");
    let common_dir = discovered.git_common_dir().unwrap();
    let worktree_path = common_dir
        .join("redquill")
        .join("worktrees")
        .join("feature-test");
    assert_inside_tempdir(&worktree_path, &repo);
    discovered.worktree_add(&worktree_path, "feature").unwrap();
    let state_path = common_dir.join("redquill").join("review-state.json");
    assert_inside_tempdir(&state_path, &repo);

    let target = DiffTarget::Review {
        base: "main".to_string(),
        branch: "feature".to_string(),
    };
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    // -- "Session 1": accept a.rs and b.rs, defer c.rs, then pause (drop). --
    {
        let session_runner = GitRunner::discover_in(&worktree_path).expect("discover worktree");
        let mut app = app_for_worktree(&session_runner, target.clone());
        app.set_review_state_path(state_path.clone());
        assert_eq!(app.view.files.len(), 3, "fixture must diff exactly 3 files");

        press(&mut app, &keymap, &mut pending, KeyCode::Char(' ')); // accept a.rs
        app.select_file_by_path("b.rs");
        press(&mut app, &keymap, &mut pending, KeyCode::Char(' ')); // accept b.rs
        app.select_file_by_path("c.rs");
        press(&mut app, &keymap, &mut pending, KeyCode::Char('d')); // defer c.rs
        wait_for_review_save(&mut app);
        assert_eq!(app.review_progress(), (2, 3));
        // Pause: `app` is simply dropped here (no `finish`), exactly like
        // `q`'s pause exit — the worktree and state entry both survive.
    }
    assert!(
        worktree_path.exists(),
        "pause must leave the worktree in place"
    );
    let saved = store::load(&state_path);
    assert!(saved.reviews.contains_key("feature"));

    // -- The "author" pushes a new commit touching the already-accepted b.rs. --
    write(&worktree_path, "b.rs", "fn b() { changed(); }\n");
    git(&worktree_path, &["commit", "-aqm", "change b.rs"]);

    // -- "Session 2": resume, observe staleness, re-accept, then finish. --
    let session_runner = GitRunner::discover_in(&worktree_path).expect("discover worktree");
    // Mirrors `main.rs`'s `load_reconciled_review_state` exactly (that
    // function is private to the binary crate, so this in-crate test
    // re-derives the same load-then-reconcile steps directly against the
    // public `crate::review` API it's built from).
    let (states, blob_shas) = {
        let persisted = store::load(&state_path);
        let review = persisted
            .reviews
            .get("feature")
            .expect("session 1 saved an entry");
        let mut current_shas = std::collections::HashMap::new();
        for path in review.files.keys() {
            current_shas.insert(
                path.clone(),
                session_runner.blob_sha("feature", path).unwrap(),
            );
        }
        let statuses = crate::review::reconcile(review, &current_shas);
        let mut blob_shas = std::collections::HashMap::new();
        for (path, status) in &statuses {
            if matches!(
                status,
                ReviewStatus::Accepted | ReviewStatus::ChangedSinceAccepted
            ) && let Some(entry) = review.files.get(path)
            {
                blob_shas.insert(path.clone(), entry.blob_sha.clone());
            }
        }
        (statuses, blob_shas)
    };
    let mut app = app_for_worktree(&session_runner, target.clone());
    app.set_review_states(states, blob_shas);
    app.set_review_state_path(state_path.clone());
    app.set_review_origin_ops(Box::new(GitRunner::discover_in(repo.path()).unwrap()));

    assert_eq!(
        app.review_status("a.rs"),
        ReviewStatus::Accepted,
        "an untouched accepted file must stay accepted across sessions"
    );
    assert!(app.view.is_collapsed("a.rs"));
    assert_eq!(
        app.review_status("b.rs"),
        ReviewStatus::ChangedSinceAccepted,
        "exactly the touched file must show changed-since-accepted"
    );
    assert!(
        !app.view.is_collapsed("b.rs"),
        "a changed-since-accepted file must render un-collapsed"
    );
    if std::env::var_os("REDQUILL_PROOF_DUMP").is_some() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| draw(frame, &app, &keymap, None))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let w = buffer.area.width as usize;
        let symbols: Vec<&str> = buffer.content().iter().map(|c| c.symbol()).collect();
        eprintln!("-- session 2, resumed and reconciled (before re-accepting b.rs) --");
        for row in symbols.chunks(w) {
            eprintln!("{}", row.concat());
        }
    }
    assert_eq!(
        app.review_status("c.rs"),
        ReviewStatus::Deferred,
        "deferred status must carry over as-is"
    );
    assert!(app.view.is_collapsed("c.rs"));
    assert_eq!(
        app.review_progress(),
        (1, 3),
        "only a.rs still counts as accepted until b.rs is re-accepted"
    );

    // One `Space` press re-accepts b.rs at the fresh SHA.
    app.select_file_by_path("b.rs");
    press(&mut app, &keymap, &mut pending, KeyCode::Char(' '));
    wait_for_review_save(&mut app);
    assert_eq!(app.review_status("b.rs"), ReviewStatus::Accepted);
    assert!(app.view.is_collapsed("b.rs"));
    assert_eq!(app.review_progress(), (2, 3));

    let fresh_b_sha = session_runner.blob_sha("feature", "b.rs").unwrap();
    let saved_after_reaccept = store::load(&state_path);
    let b_entry = saved_after_reaccept.reviews["feature"]
        .files
        .get("b.rs")
        .unwrap();
    assert_eq!(
        b_entry.blob_sha, fresh_b_sha,
        "re-accepting must persist the fresh SHA, not the stale one"
    );

    // -- Finish: worktree removed, admin records pruned, state entry gone. --
    app.open_end_review_modal();
    let outcome = app.finish_review();
    assert_eq!(outcome, Some(super::QuitOutcome::Emit));

    assert!(!worktree_path.exists(), "finish must remove the worktree");
    let worktree_list = git_out(repo.path(), &["worktree", "list", "--porcelain"]);
    assert!(
        !worktree_list.contains(worktree_path.to_str().unwrap()),
        "finish must prune the worktree's admin record: {worktree_list}"
    );
    let final_state = store::load(&state_path);
    assert!(
        !final_state.reviews.contains_key("feature"),
        "finish must delete this branch's persisted state entry"
    );

    // The user's own checkout is untouched throughout.
    assert_eq!(git_out(repo.path(), &["branch", "--show-current"]), "main");
    assert_eq!(git_out(repo.path(), &["status", "--porcelain"]), "");
}

// -- GC of a deleted branch (task 4.5, exercised end-to-end here too) -------

/// A plain repo on `main` only — deliberately *without* a `feature`
/// branch, so a persisted entry naming one simulates a branch the author
/// has since deleted.
fn repo_on_main_only() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    assert_inside_tempdir(dir, &tmp);
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.email", "test@redquill.invalid"]);
    git(dir, &["config", "user.name", "redquill test"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
    write(dir, "base.txt", "line one\n");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "initial"]);
    tmp
}

#[test]
fn launching_after_the_reviewed_branch_is_deleted_gcs_its_entry() {
    let repo = repo_on_main_only();
    let discovered = GitRunner::discover_in(repo.path()).expect("discover repo");
    let common_dir = discovered.git_common_dir().unwrap();
    let state_path = common_dir.join("redquill").join("review-state.json");
    assert_inside_tempdir(&state_path, &repo);

    store::save_review(
        &state_path,
        "feature",
        store::PersistedReview {
            base: "main".to_string(),
            worktree_path: repo.path().join("wt"),
            files: Default::default(),
        },
    )
    .unwrap();
    store::save_review(
        &state_path,
        "still-here",
        store::PersistedReview {
            base: "main".to_string(),
            worktree_path: repo.path().join("wt2"),
            files: Default::default(),
        },
    )
    .unwrap();
    git(repo.path(), &["branch", "still-here"]);
    // `feature` itself is never created as a real branch here — simulating
    // the author having deleted it after this review finished or was
    // abandoned without going through `finish`.

    let existing: std::collections::HashSet<String> = discovered
        .branch_list()
        .unwrap()
        .into_iter()
        .map(|b| b.name)
        .collect();
    let mut state = store::load(&state_path);
    let changed = store::gc(&mut state, &existing);
    assert!(changed);
    store::save(&state_path, &state).unwrap();

    let reloaded = store::load(&state_path);
    assert!(!reloaded.reviews.contains_key("feature"));
    assert!(reloaded.reviews.contains_key("still-here"));
}
