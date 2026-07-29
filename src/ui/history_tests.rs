use super::*;
use crate::git::{DiffTarget, FileStatus, GitError, RawFilePatch};
use crate::ui::stage_ops::StageOps;

/// A minimal `StageOps` fake serving a fixed, pre-built commit list
/// synchronously (no `async_commit_log_fetcher`, so `request_history_page`
/// takes the synchronous fallback path — exercising it without a real
/// background thread).
struct SyncHistoryFake {
    entries: Vec<CommitLogEntry>,
}

impl StageOps for SyncHistoryFake {
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
    fn commit_log(&self, count: u32, skip: u32) -> Result<Vec<CommitLogEntry>, GitError> {
        let start = (skip as usize).min(self.entries.len());
        let end = (start + count as usize).min(self.entries.len());
        Ok(self.entries[start..end].to_vec())
    }
}

fn commit(sha: &str, subject: &str) -> CommitLogEntry {
    CommitLogEntry {
        sha: sha.to_string(),
        short_sha: sha.to_string(),
        subject: subject.to_string(),
        author_name: "Dev".to_string(),
        timestamp: 1_700_000_000,
    }
}

fn app_with_history_fake(entries: Vec<CommitLogEntry>) -> App {
    let mut app = App::new(Vec::new());
    app.stage_ops = Some(Box::new(SyncHistoryFake { entries }));
    app
}

// -- Loading placeholder (3.2a) ---------------------------------------------

// -- Stale-generation drop (3.2b) --------------------------------------------

/// A history fetch completing after `history_generation` has advanced past
/// its spawn-time value must be dropped, not applied — mirrors
/// `stale_async_snapshot_discarded_after_generation_bump` in
/// `app_tests.rs` for the working-tree refresh.
#[test]
fn stale_generation_history_result_is_dropped_not_applied() {
    let mut app = App::new(Vec::new());
    let stale_page = vec![commit("stale", "should never appear")];
    let id = app.history_tasks.spawn(move || Some(stale_page));
    app.history_in_flight = Some(InFlightHistory {
        id,
        generation: app.history_generation,
    });

    // Something (e.g. a future invalidation point) bumps the generation
    // before this fetch lands.
    app.history_generation = app.history_generation.wrapping_add(1);

    // Poll until the background thread's result is drained (it always
    // completes quickly — the closure does no real work).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        app.poll_history();
        if app.history_in_flight.is_none() || std::time::Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    assert!(app.history_in_flight.is_none(), "stale fetch was consumed");
    assert!(
        app.history.is_empty(),
        "a stale-generation history page must never be applied"
    );
}

// -- Single-flight (bonus coverage, mirrors refresh's guard) -----------------

/// While a fetch is already in flight, `request_history_page` is a no-op —
/// it never spawns a second concurrent fetch.
#[test]
fn request_history_page_is_single_flight() {
    let mut app = App::new(Vec::new());
    let id = app.history_tasks.spawn(|| Some(vec![commit("a", "one")]));
    app.history_in_flight = Some(InFlightHistory {
        id,
        generation: app.history_generation,
    });
    app.stage_ops = Some(Box::new(SyncHistoryFake {
        entries: vec![commit("b", "two")],
    }));

    app.request_history_page();

    // Still the original in-flight task; the synchronous fake's page was
    // never applied (a second fetch never started).
    assert_eq!(app.history_in_flight.map(|f| f.id), Some(id));
    assert!(app.history.is_empty());
}

/// A page shorter than a full [`HISTORY_PAGE_SIZE`] request marks history
/// exhausted, so no further page is ever requested.
#[test]
fn a_short_page_marks_history_exhausted() {
    let mut app = app_with_history_fake(vec![commit("a", "one"), commit("b", "two")]);
    app.ensure_history_loaded();
    assert!(app.history_exhausted);
    assert_eq!(app.history.len(), 2);

    // A further request is a no-op: exhausted history never re-fetches.
    app.request_history_page();
    assert_eq!(app.history.len(), 2);
}
