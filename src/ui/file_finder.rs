//! The fuzzy file finder overlay ([`super::app::Mode::Finder`]): candidates
//! loaded once per open through [`super::background::BackgroundTasks`]
//! (single-flight, generation-guarded — mirrors [`super::history`]'s
//! commit-log loader), then ranked by [`crate::search::rank`] and re-ranked on
//! every keystroke. Split out of `app.rs` alongside the switcher/history
//! modules, so all finder logic lives in one place.

use crate::search::{FileCandidate, FuzzyMatch, rank};

use super::app::{App, Mode};
use super::background::TaskId;

/// The fuzzy finder modal's state: the query buffer, the full candidate
/// list (empty while still loading), the current query's ranked matches, the
/// selection cursor, and the mode to restore on a losslessly-cancelled close
/// (`Esc`).
#[derive(Debug, Clone)]
pub(super) struct FinderState {
    /// The free-text query buffer.
    pub(super) query: String,
    /// Every candidate file, as loaded when the finder opened (empty until
    /// the background load lands).
    pub(super) candidates: Vec<FileCandidate>,
    /// The current query's ranked matches over `candidates`, recomputed by
    /// [`App::rerank_finder`] on every keystroke and whenever the candidate
    /// list itself changes.
    pub(super) matches: Vec<FuzzyMatch>,
    /// The selected row within `matches`.
    pub(super) cursor: usize,
    /// The mode to restore on `Esc` (the mode the finder was opened from) —
    /// `Enter` never uses this: opening a file always lands in
    /// [`Mode::Normal`] via [`App::open_file_view`], mirroring how a commit
    /// view always returns focus to the diff regardless of where it was
    /// opened from.
    pub(super) return_mode: Mode,
}

impl FinderState {
    fn new(return_mode: Mode) -> FinderState {
        FinderState {
            query: String::new(),
            candidates: Vec::new(),
            matches: Vec::new(),
            cursor: 0,
            return_mode,
        }
    }
}

/// A background file-candidate load awaiting completion. Mirrors
/// [`super::history::InFlightHistory`]'s shape exactly.
#[derive(Debug, Clone, Copy)]
pub(super) struct InFlightFinderLoad {
    /// The background task delivering this load's candidate list.
    pub(super) id: TaskId,
    /// The finder generation captured when this load was spawned.
    pub(super) generation: u64,
}

impl App {
    /// Opens the fuzzy file finder (`gp`, diff scope): captures the current
    /// mode as the close-without-opening restore point, switches to
    /// [`Mode::Finder`], and kicks off the candidate load. Bumping
    /// `finder_generation` and clearing any prior in-flight load here means
    /// a straggling load from a previous open (closed and reopened quickly)
    /// is dropped on arrival rather than applied to this session.
    pub(super) fn open_finder(&mut self) {
        let return_mode = self.mode;
        self.finder = Some(FinderState::new(return_mode));
        self.mode = Mode::Finder;
        self.finder_generation = self.finder_generation.wrapping_add(1);
        self.finder_in_flight = None;
        self.request_finder_candidates();
    }

    /// Requests the candidate list, single-flight: a no-op while a load is
    /// already in flight. Prefers the async path (off the render thread via
    /// [`super::stage_ops::StageOps::async_file_candidates_fetcher`]);
    /// falls back to a synchronous read for backends that can't cross a
    /// thread boundary (test fakes, git-less contexts) — the same fallback
    /// shape [`super::history::App::request_history_page`] uses.
    fn request_finder_candidates(&mut self) {
        if self.finder_in_flight.is_some() {
            return;
        }
        let Some(ops) = self.stage_ops() else {
            return;
        };
        if let Some(fetcher) = ops.async_file_candidates_fetcher() {
            let generation = self.finder_generation;
            let id = self.finder_tasks.spawn(move || fetcher().ok());
            self.finder_in_flight = Some(InFlightFinderLoad { id, generation });
        } else if let Ok(candidates) = ops.list_files() {
            self.apply_finder_candidates(candidates);
        }
    }

    /// Drains a completed background candidate load (once per event-loop
    /// tick, alongside the other pollers). Drops a stale result — spawned
    /// before `finder_generation` was last bumped, or delivered after the
    /// finder closed — or a foreign/task-panic result silently; applies a
    /// successful list otherwise.
    pub(super) fn poll_finder(&mut self) {
        for (id, result) in self.finder_tasks.poll() {
            let Some(in_flight) = self.finder_in_flight else {
                continue;
            };
            if in_flight.id != id {
                continue;
            }
            self.finder_in_flight = None;
            if in_flight.generation != self.finder_generation {
                continue;
            }
            let Ok(Some(candidates)) = result else {
                continue;
            };
            self.apply_finder_candidates(candidates);
        }
    }

    /// Folds a freshly loaded candidate list into the open finder (a no-op
    /// if the finder has since closed) and re-ranks against the current
    /// query.
    fn apply_finder_candidates(&mut self, candidates: Vec<FileCandidate>) {
        let Some(finder) = self.finder.as_mut() else {
            return;
        };
        finder.candidates = candidates;
        self.rerank_finder();
    }

    /// Re-ranks the open finder's candidates against its current query (see
    /// [`crate::search::rank`]), re-clamping the selection cursor into the
    /// new match count. A no-op if the finder isn't open.
    pub(super) fn rerank_finder(&mut self) {
        let Some(finder) = self.finder.as_mut() else {
            return;
        };
        finder.matches = rank(&finder.candidates, &finder.query);
        finder.cursor = if finder.matches.is_empty() {
            0
        } else {
            finder.cursor.min(finder.matches.len() - 1)
        };
    }

    /// Appends `c` to the query buffer and re-ranks. A no-op if the finder
    /// isn't open.
    pub(super) fn finder_input_char(&mut self, c: char) {
        if let Some(finder) = self.finder.as_mut() {
            finder.query.push(c);
        }
        self.rerank_finder();
    }

    /// Deletes the last character of the query buffer and re-ranks. A no-op
    /// if the finder isn't open (or the buffer is already empty).
    pub(super) fn finder_backspace(&mut self) {
        if let Some(finder) = self.finder.as_mut() {
            finder.query.pop();
        }
        self.rerank_finder();
    }

    /// Moves the finder's selection down one match, clamped at the last (or
    /// pinned at 0 on an empty result list). A no-op if the finder isn't
    /// open.
    pub(super) fn finder_move_down(&mut self) {
        if let Some(finder) = self.finder.as_mut()
            && !finder.matches.is_empty()
        {
            finder.cursor = (finder.cursor + 1).min(finder.matches.len() - 1);
        }
    }

    /// Moves the finder's selection up one match, clamped at the first. A
    /// no-op if the finder isn't open.
    pub(super) fn finder_move_up(&mut self) {
        if let Some(finder) = self.finder.as_mut() {
            finder.cursor = finder.cursor.saturating_sub(1);
        }
    }

    /// Closes the finder losslessly (`Esc`, no selection made): restores
    /// whichever mode it was opened from. A no-op if the finder isn't open.
    pub(super) fn close_finder(&mut self) {
        let Some(finder) = self.finder.take() else {
            return;
        };
        self.mode = finder.return_mode;
    }

    /// The finder's `Enter` gesture: opens the selected match's file in the
    /// read-only file view (see [`App::open_file_view`]) and closes the
    /// finder. A no-op (finder stays open) if nothing is selected — an empty
    /// query, or a query with no matches.
    pub(super) fn finder_confirm(&mut self) {
        let Some(finder) = self.finder.as_ref() else {
            return;
        };
        let Some(m) = finder.matches.get(finder.cursor) else {
            return;
        };
        let Some(candidate) = finder.candidates.get(m.index) else {
            return;
        };
        let path = candidate.path.clone();
        self.finder = None;
        self.open_file_view(path, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::FileDiff;
    use crate::git::{DiffTarget, FileStatus, GitError, RawFilePatch};
    use crate::ui::stage_ops::{AsyncFileCandidatesFetcher, StageOps};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

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

    /// A fake `StageOps` that serves a fixed candidate list, either
    /// synchronously or (when `async_capable`) through a real background
    /// thread — so tests can exercise both `request_finder_candidates`
    /// fallback paths.
    struct FakeOps {
        candidates: Vec<FileCandidate>,
        async_capable: bool,
        /// Blocks the async fetcher's closure until released, so a test can
        /// assert on the in-flight state before the load completes.
        gate: Option<Arc<Mutex<()>>>,
    }

    impl StageOps for FakeOps {
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
        fn list_files(&self) -> Result<Vec<FileCandidate>, GitError> {
            Ok(self.candidates.clone())
        }
        fn async_file_candidates_fetcher(&self) -> Option<AsyncFileCandidatesFetcher> {
            if !self.async_capable {
                return None;
            }
            let candidates = self.candidates.clone();
            let gate = self.gate.clone();
            Some(Box::new(move || {
                if let Some(gate) = &gate {
                    // Blocks until the test-held lock is released; the guard
                    // is dropped immediately since only the *acquisition*
                    // needs to block.
                    let _guard = gate.lock();
                }
                Ok(candidates.clone())
            }))
        }
    }

    fn candidate(path: &str) -> FileCandidate {
        FileCandidate {
            path: path.to_string(),
        }
    }

    fn app_with_candidates(paths: &[&str], async_capable: bool) -> App {
        let mut app = App::new(vec![sample_file()]);
        app.stage_ops = Some(Box::new(FakeOps {
            candidates: paths.iter().map(|p| candidate(p)).collect(),
            async_capable,
            gate: None,
        }));
        app
    }

    fn wait_for_finder_load(app: &mut App) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.finder_in_flight.is_some() && Instant::now() < deadline {
            app.poll_finder();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    // -- open_finder / close_finder ----------------------------------------

    #[test]
    fn open_finder_switches_mode_and_captures_return_mode() {
        let mut app = app_with_candidates(&["a.rs"], false);
        app.open_finder();
        assert_eq!(app.mode, Mode::Finder);
        assert_eq!(app.finder.as_ref().unwrap().return_mode, Mode::Normal);
    }

    #[test]
    fn close_finder_restores_the_captured_return_mode() {
        let mut app = app_with_candidates(&["a.rs"], false);
        app.mode = Mode::Panel {
            cursor: 3,
            tab: crate::ui::app::PanelTab::Changes,
        };
        app.open_finder();
        app.close_finder();
        assert_eq!(
            app.mode,
            Mode::Panel {
                cursor: 3,
                tab: crate::ui::app::PanelTab::Changes
            }
        );
        assert!(app.finder.is_none());
    }

    #[test]
    fn close_finder_without_ever_opening_is_a_no_op() {
        let mut app = App::new(vec![sample_file()]);
        app.close_finder();
        assert_eq!(app.mode, Mode::Normal);
    }

    // -- candidate loading: sync fallback + async single-flight -------------

    #[test]
    fn open_finder_loads_candidates_synchronously_without_an_async_backend() {
        let mut app = app_with_candidates(&["a.rs", "b.rs"], false);
        app.open_finder();
        assert_eq!(app.finder.as_ref().unwrap().candidates.len(), 2);
    }

    #[test]
    fn open_finder_loads_candidates_through_the_background_poller() {
        let mut app = app_with_candidates(&["a.rs", "b.rs", "c.rs"], true);
        app.open_finder();
        assert!(
            app.finder.as_ref().unwrap().candidates.is_empty(),
            "candidates must not be populated before the background load lands"
        );
        wait_for_finder_load(&mut app);
        assert_eq!(app.finder.as_ref().unwrap().candidates.len(), 3);
    }

    #[test]
    fn a_stale_load_spawned_before_reopening_is_dropped_on_arrival() {
        let gate = Arc::new(Mutex::new(()));
        let lock = gate.lock().unwrap();
        let mut app = App::new(vec![sample_file()]);
        app.stage_ops = Some(Box::new(FakeOps {
            candidates: vec![candidate("stale.rs")],
            async_capable: true,
            gate: Some(gate.clone()),
        }));

        app.open_finder(); // spawns a load gated on `lock`
        let stale_generation_task = app.finder_in_flight;
        assert!(stale_generation_task.is_some());

        // Close and reopen before the stale load can land: this bumps
        // `finder_generation`, and swaps in a backend with no gate so the
        // second load completes immediately.
        app.close_finder();
        app.stage_ops = Some(Box::new(FakeOps {
            candidates: vec![candidate("fresh.rs")],
            async_capable: true,
            gate: None,
        }));
        app.open_finder();
        wait_for_finder_load(&mut app);
        assert_eq!(
            app.finder.as_ref().unwrap().candidates,
            vec![candidate("fresh.rs")]
        );

        // Release the stale load and drain it — it must be dropped, not
        // clobber the fresh candidates already applied.
        drop(lock);
        std::thread::sleep(Duration::from_millis(50));
        app.poll_finder();
        assert_eq!(
            app.finder.as_ref().unwrap().candidates,
            vec![candidate("fresh.rs")],
            "the stale load must not overwrite the fresh candidates"
        );
    }

    // -- typing / re-ranking ------------------------------------------------

    #[test]
    fn typing_reranks_on_every_keystroke() {
        let mut app = app_with_candidates(&["src/main.rs", "README.md"], false);
        app.open_finder();
        for c in "main".chars() {
            app.finder_input_char(c);
        }
        let finder = app.finder.as_ref().unwrap();
        assert_eq!(finder.query, "main");
        assert_eq!(finder.matches.len(), 1);
        assert_eq!(
            finder.candidates[finder.matches[0].index].path,
            "src/main.rs"
        );
    }

    #[test]
    fn backspace_shrinks_the_query_and_reranks() {
        let mut app = app_with_candidates(&["src/main.rs", "README.md"], false);
        app.open_finder();
        for c in "mainx".chars() {
            app.finder_input_char(c);
        }
        assert!(app.finder.as_ref().unwrap().matches.is_empty());
        app.finder_backspace();
        assert_eq!(app.finder.as_ref().unwrap().query, "main");
        assert_eq!(app.finder.as_ref().unwrap().matches.len(), 1);
    }

    #[test]
    fn empty_query_shows_no_matches() {
        let mut app = app_with_candidates(&["a.rs"], false);
        app.open_finder();
        assert!(app.finder.as_ref().unwrap().matches.is_empty());
    }

    // -- navigation -----------------------------------------------------

    #[test]
    fn move_down_and_up_clamp_within_matches() {
        let mut app = app_with_candidates(&["a.rs", "ab.rs", "abc.rs"], false);
        app.open_finder();
        for c in "a".chars() {
            app.finder_input_char(c);
        }
        assert!(app.finder.as_ref().unwrap().matches.len() >= 2);
        app.finder_move_down();
        assert_eq!(app.finder.as_ref().unwrap().cursor, 1);
        app.finder_move_up();
        assert_eq!(app.finder.as_ref().unwrap().cursor, 0);
        app.finder_move_up(); // clamps at 0
        assert_eq!(app.finder.as_ref().unwrap().cursor, 0);
    }

    #[test]
    fn move_on_empty_matches_stays_at_zero() {
        let mut app = app_with_candidates(&["a.rs"], false);
        app.open_finder();
        app.finder_move_down();
        assert_eq!(app.finder.as_ref().unwrap().cursor, 0);
    }

    // -- confirm: opens the file view and closes the finder -----------------

    #[test]
    fn confirm_opens_the_selected_file_and_closes_the_finder() {
        let mut app = App::new(vec![sample_file()]);
        let mut files = std::collections::HashMap::new();
        files.insert("target.rs".to_string(), b"hello\n".to_vec());
        struct Ops {
            candidates: Vec<FileCandidate>,
            files: std::collections::HashMap<String, Vec<u8>>,
        }
        impl StageOps for Ops {
            fn diff(&self, _t: &DiffTarget) -> Result<Vec<RawFilePatch>, GitError> {
                Ok(Vec::new())
            }
            fn status(&self) -> Result<Vec<FileStatus>, GitError> {
                Ok(Vec::new())
            }
            fn stage_file(&self, _p: &str) -> Result<(), GitError> {
                Ok(())
            }
            fn unstage_file(&self, _p: &str) -> Result<(), GitError> {
                Ok(())
            }
            fn apply_cached(&self, _p: &str) -> Result<(), GitError> {
                Ok(())
            }
            fn unapply_cached(&self, _p: &str) -> Result<(), GitError> {
                Ok(())
            }
            fn read_worktree_file(&self, path: &str) -> Option<Vec<u8>> {
                self.files.get(path).cloned()
            }
            fn show_file(&self, _s: &str) -> Option<String> {
                None
            }
            fn list_files(&self) -> Result<Vec<FileCandidate>, GitError> {
                Ok(self.candidates.clone())
            }
        }
        app.stage_ops = Some(Box::new(Ops {
            candidates: vec![candidate("target.rs")],
            files,
        }));

        app.open_finder();
        app.finder_input_char('t');
        app.finder_confirm();

        assert!(app.finder.is_none(), "finder must close on confirm");
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.target, DiffTarget::File("target.rs".to_string()));
    }

    #[test]
    fn confirm_with_no_matches_is_a_no_op() {
        let mut app = app_with_candidates(&["a.rs"], false);
        app.open_finder();
        for c in "zzz".chars() {
            app.finder_input_char(c);
        }
        assert!(app.finder.as_ref().unwrap().matches.is_empty());
        app.finder_confirm();
        assert_eq!(app.mode, Mode::Finder, "finder must stay open");
        assert!(app.finder.is_some());
    }
}
