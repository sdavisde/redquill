use super::*;
use crate::git::{DiffTarget, FileStatus, GitError, RawFilePatch};
use crate::ui::background::TaskId;
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

// -- Single-flight (mirrors refresh's guard) --------------------------------

/// While a fetch is already in flight, `request_history_page` is a no-op —
/// it never spawns a second concurrent fetch.
#[test]
fn request_history_page_is_single_flight() {
    let mut app = App::new(Vec::new());
    let id = app.history_tasks.spawn(|| Some(vec![commit("a", "one")]));
    app.history_in_flight = Some(id);
    app.stage_ops = Some(Box::new(SyncHistoryFake {
        entries: vec![commit("b", "two")],
    }));

    app.request_history_page();

    // Still the original in-flight task; the synchronous fake's page was
    // never applied (a second fetch never started).
    assert_eq!(app.history_in_flight, Some(id));
    assert!(app.history.is_empty());
}

/// A result whose task id isn't the in-flight fetch's is dropped, not
/// applied — the guard that replaces the removed generation counter.
#[test]
fn a_foreign_history_result_is_dropped_not_applied() {
    let mut app = App::new(Vec::new());
    let foreign_page = vec![commit("foreign", "should never appear")];
    app.history_tasks.spawn(move || Some(foreign_page));
    // A different task is what the app believes is in flight.
    app.history_in_flight = Some(TaskId(u64::MAX));

    // Poll until the background thread's result has been drained (it always
    // completes quickly — the closure does no real work).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while app.history.is_empty() && std::time::Instant::now() < deadline {
        app.poll_history();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    assert_eq!(
        app.history_in_flight,
        Some(TaskId(u64::MAX)),
        "a foreign result must not clear the real in-flight marker"
    );
    assert!(
        app.history.is_empty(),
        "a foreign history page must never be applied"
    );
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
