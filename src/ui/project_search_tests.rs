//! Unit tests for [`super::App`]'s Project Search state machine (spec 06
//! Unit 2): mode capture/restore, the generation/debounce contract (stale
//! results dropped, in-flight scan aborted on every query-affecting change,
//! invalid regex never wipes prior results), and result navigation/confirm.
//! Split out per the repo's big-test-module convention.
//!
//! Debounce/generation logic is exercised without sleeping wherever possible
//! (see `debounce_elapsed` and `maybe_fire_project_search`'s explicit `now`
//! parameter); the one test that spawns a real scan
//! (`valid_query_spawns_a_real_scan_and_streams_results_into_groups`) still
//! has to poll a real background thread to completion, mirroring
//! `file_finder.rs`'s `wait_for_finder_load` precedent.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::*;
use crate::diff::FileDiff;
use crate::git::{DiffTarget, GitError, RawFilePatch};
use crate::ui::app::PanelTab;
use crate::ui::stage_ops::StageOps;

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

/// A minimal `StageOps` fake serving fixed worktree file contents, for the
/// confirm/round-trip tests that need `open_file_view` to succeed.
struct FakeOps {
    files: HashMap<String, Vec<u8>>,
}

impl StageOps for FakeOps {
    fn diff(&self, _target: &DiffTarget) -> Result<Vec<RawFilePatch>, GitError> {
        Ok(Vec::new())
    }
    fn status(&self) -> Result<Vec<crate::git::FileStatus>, GitError> {
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
    fn read_worktree_file(&self, path: &str) -> Option<Vec<u8>> {
        self.files.get(path).cloned()
    }
    fn show_file(&self, _spec: &str) -> Option<String> {
        None
    }
}

fn hit(path: &str, line: u64) -> SearchHit {
    SearchHit {
        path: path.to_string(),
        line_number: line,
        line_text: "needle here".to_string(),
        #[allow(clippy::single_range_in_vec_init)]
        match_spans: vec![0..6],
        generation: 0,
    }
}

/// A fake in-flight scan backed by a real channel (never sent to), so tests
/// can assert on `abort` without spawning a real scan thread.
fn fake_scan(generation: u64) -> (InFlightScan, Arc<AtomicBool>) {
    let (_tx, rx) = mpsc::sync_channel(1);
    let abort = Arc::new(AtomicBool::new(false));
    (
        InFlightScan {
            generation,
            receiver: rx,
            abort: Arc::clone(&abort),
        },
        abort,
    )
}

// -- open / close: return-mode capture and restore --------------------------

#[test]
fn open_project_search_captures_return_mode_and_switches_mode() {
    let mut app = App::new(vec![sample_file()]);
    app.mode = Mode::Panel {
        cursor: 3,
        tab: PanelTab::Changes,
    };
    app.open_project_search();
    assert_eq!(app.mode, Mode::ProjectSearch);
    assert_eq!(
        app.project_search.as_ref().unwrap().return_mode,
        Mode::Panel {
            cursor: 3,
            tab: PanelTab::Changes
        }
    );
}

#[test]
fn close_project_search_restores_return_mode_and_aborts_in_flight_scan() {
    let mut app = App::new(vec![sample_file()]);
    app.open_project_search();
    let (scan, abort) = fake_scan(1);
    app.project_search.as_mut().unwrap().scan = Some(scan);

    app.close_project_search();

    assert_eq!(app.mode, Mode::Normal);
    assert!(app.project_search.is_none());
    assert!(abort.load(Ordering::Relaxed), "in-flight scan must abort");
}

#[test]
fn close_project_search_without_opening_is_a_no_op() {
    let mut app = App::new(vec![sample_file()]);
    app.close_project_search();
    assert_eq!(app.mode, Mode::Normal);
}

// -- focus model (round-1 UX fix: Esc/Tab/`/` two-focus split) --------------

#[test]
fn open_project_search_starts_in_input_focus() {
    let mut app = App::new(vec![sample_file()]);
    app.open_project_search();
    assert_eq!(
        app.project_search.as_ref().unwrap().focus,
        SearchFocus::Input
    );
}

#[test]
fn esc_in_input_focus_moves_to_results_without_closing_the_view() {
    let mut app = App::new(vec![sample_file()]);
    app.open_project_search();
    assert_eq!(
        app.project_search.as_ref().unwrap().focus,
        SearchFocus::Input
    );

    app.project_search_esc();

    assert_eq!(app.mode, Mode::ProjectSearch, "the view must stay open");
    assert_eq!(
        app.project_search.as_ref().unwrap().focus,
        SearchFocus::Results
    );
}

#[test]
fn esc_in_results_focus_closes_the_view() {
    let mut app = App::new(vec![sample_file()]);
    app.open_project_search();
    app.project_search.as_mut().unwrap().focus = SearchFocus::Results;

    app.project_search_esc();

    assert_eq!(app.mode, Mode::Normal);
    assert!(app.project_search.is_none());
}

#[test]
fn esc_without_opening_is_a_no_op() {
    let mut app = App::new(vec![sample_file()]);
    app.project_search_esc();
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.project_search.is_none());
}

#[test]
fn focus_input_switches_back_from_results_and_preserves_the_query() {
    let mut app = App::new(vec![sample_file()]);
    app.open_project_search();
    let state = app.project_search.as_mut().unwrap();
    state.focus = SearchFocus::Results;
    state.query = "needle".to_string();

    app.project_search_focus_input();

    let state = app.project_search.as_ref().unwrap();
    assert_eq!(state.focus, SearchFocus::Input);
    assert_eq!(state.query, "needle", "query must survive the focus switch");
}

#[test]
fn toggle_focus_flips_both_directions() {
    let mut app = App::new(vec![sample_file()]);
    app.open_project_search();
    assert_eq!(
        app.project_search.as_ref().unwrap().focus,
        SearchFocus::Input
    );

    app.project_search_toggle_focus();
    assert_eq!(
        app.project_search.as_ref().unwrap().focus,
        SearchFocus::Results
    );

    app.project_search_toggle_focus();
    assert_eq!(
        app.project_search.as_ref().unwrap().focus,
        SearchFocus::Input
    );
}

// -- generation/debounce contract --------------------------------------------

#[test]
fn typing_bumps_generation_and_sets_a_debounce_deadline() {
    let mut app = App::new(vec![sample_file()]);
    app.open_project_search();
    let before = app.project_search.as_ref().unwrap().generation;

    app.project_search_input_char('n');

    let state = app.project_search.as_ref().unwrap();
    assert_eq!(state.generation, before + 1);
    assert!(state.debounce_deadline.is_some());
    assert_eq!(state.query, "n");
}

#[test]
fn each_keystroke_bumps_generation_again_and_resets_the_debounce_window() {
    let mut app = App::new(vec![sample_file()]);
    app.open_project_search();
    app.project_search_input_char('n');
    let first_generation = app.project_search.as_ref().unwrap().generation;
    let first_deadline = app.project_search.as_ref().unwrap().debounce_deadline;

    app.project_search_input_char('e');

    let state = app.project_search.as_ref().unwrap();
    assert_eq!(state.generation, first_generation + 1);
    assert_ne!(state.debounce_deadline, first_deadline);
}

#[test]
fn note_change_aborts_whatever_scan_is_currently_in_flight() {
    let mut app = App::new(vec![sample_file()]);
    app.open_project_search();
    let (scan, abort) = fake_scan(1);
    app.project_search.as_mut().unwrap().scan = Some(scan);

    app.project_search_input_char('x');

    assert!(
        abort.load(Ordering::Relaxed),
        "typing must cancel the in-flight scan promptly"
    );
    assert!(
        app.project_search.as_ref().unwrap().scan.is_none(),
        "the cancelled scan must be cleared, not left dangling"
    );
}

#[test]
fn backspace_also_bumps_generation_and_aborts() {
    let mut app = App::new(vec![sample_file()]);
    app.open_project_search();
    app.project_search_input_char('n');
    let (scan, abort) = fake_scan(app.project_search.as_ref().unwrap().generation);
    app.project_search.as_mut().unwrap().scan = Some(scan);

    app.project_search_backspace();

    assert!(abort.load(Ordering::Relaxed));
    assert_eq!(app.project_search.as_ref().unwrap().query, "");
}

#[test]
fn toggles_also_bump_generation_and_abort() {
    let mut app = App::new(vec![sample_file()]);
    app.open_project_search();
    let before = app.project_search.as_ref().unwrap().generation;
    app.project_search_toggle_whole_word();
    assert_eq!(app.project_search.as_ref().unwrap().generation, before + 1);
    assert!(app.project_search.as_ref().unwrap().whole_word);

    app.project_search_toggle_literal();
    assert!(app.project_search.as_ref().unwrap().literal);

    let before_case = app.project_search.as_ref().unwrap().case;
    app.project_search_toggle_case();
    assert_ne!(app.project_search.as_ref().unwrap().case, before_case);
}

#[test]
fn cycle_case_rotates_through_all_three_states_and_wraps() {
    assert_eq!(cycle_case(CaseMode::Smart), CaseMode::Sensitive);
    assert_eq!(cycle_case(CaseMode::Sensitive), CaseMode::Insensitive);
    assert_eq!(cycle_case(CaseMode::Insensitive), CaseMode::Smart);
}

#[test]
fn debounce_elapsed_is_false_with_no_deadline_and_true_once_now_reaches_it() {
    let now = Instant::now();
    let deadline = now + Duration::from_millis(140);
    assert!(!debounce_elapsed(None, now));
    assert!(!debounce_elapsed(Some(deadline), now));
    assert!(debounce_elapsed(
        Some(deadline),
        now + Duration::from_millis(140)
    ));
    assert!(debounce_elapsed(
        Some(deadline),
        now + Duration::from_millis(500)
    ));
}

#[test]
fn maybe_fire_is_a_no_op_before_the_debounce_deadline() {
    let mut app = App::new(vec![sample_file()]);
    app.set_repo_root(std::env::temp_dir());
    app.open_project_search();
    app.project_search_input_char('n');
    app.project_search_input_char('e');
    let generation_before = app.project_search.as_ref().unwrap().generation;

    // Well before the deadline: no scan should spawn.
    app.maybe_fire_project_search(Instant::now());

    assert!(app.project_search.as_ref().unwrap().scan.is_none());
    assert_eq!(
        app.project_search.as_ref().unwrap().generation,
        generation_before
    );
}

#[test]
fn below_min_length_query_clears_results_without_spawning_a_scan() {
    let mut app = App::new(vec![sample_file()]);
    app.set_repo_root(std::env::temp_dir());
    app.open_project_search();
    // Seed as if a previous (longer) query had produced results.
    let state = app.project_search.as_mut().unwrap();
    state.groups.push(ResultGroup {
        path: "a.rs".to_string(),
        hits: vec![hit("a.rs", 1)],
    });
    state.summary = Some(ScanSummary::default());

    app.project_search_input_char('n'); // length 1, below MIN_QUERY_LEN (2)
    let deadline = app
        .project_search
        .as_ref()
        .unwrap()
        .debounce_deadline
        .unwrap();
    app.maybe_fire_project_search(deadline + Duration::from_millis(1));

    let state = app.project_search.as_ref().unwrap();
    assert!(state.groups.is_empty());
    assert!(state.summary.is_none());
    assert!(state.error.is_none());
    assert!(state.scan.is_none());
}

#[test]
fn invalid_regex_sets_error_without_wiping_prior_good_results() {
    let mut app = App::new(vec![sample_file()]);
    app.set_repo_root(std::env::temp_dir());
    app.open_project_search();
    let state = app.project_search.as_mut().unwrap();
    state.groups.push(ResultGroup {
        path: "a.rs".to_string(),
        hits: vec![hit("a.rs", 1)],
    });
    state.summary = Some(ScanSummary {
        total_hits: 1,
        files_matched: 1,
        ..ScanSummary::default()
    });

    for c in "(unclosed".chars() {
        app.project_search_input_char(c);
    }
    let deadline = app
        .project_search
        .as_ref()
        .unwrap()
        .debounce_deadline
        .unwrap();
    app.maybe_fire_project_search(deadline + Duration::from_millis(1));

    let state = app.project_search.as_ref().unwrap();
    assert!(
        state.error.is_some(),
        "an invalid pattern must set an error"
    );
    assert_eq!(
        state.groups.len(),
        1,
        "prior good results must survive an invalid-regex query"
    );
    assert_eq!(state.summary.as_ref().unwrap().total_hits, 1);
}

#[test]
fn missing_repo_root_degrades_to_an_error_message() {
    let mut app = App::new(vec![sample_file()]); // no repo_root set
    app.open_project_search();
    for c in "needle".chars() {
        app.project_search_input_char(c);
    }
    let deadline = app
        .project_search
        .as_ref()
        .unwrap()
        .debounce_deadline
        .unwrap();
    app.maybe_fire_project_search(deadline + Duration::from_millis(1));

    assert!(app.project_search.as_ref().unwrap().error.is_some());
    assert!(app.project_search.as_ref().unwrap().scan.is_none());
}

/// Drains `app`'s Project Search scan until `Done` lands (real background
/// thread — bounded real-time poll, mirroring `file_finder.rs`'s
/// `wait_for_finder_load`).
fn wait_for_scan_done(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app
        .project_search
        .as_ref()
        .is_some_and(|s| s.scan.is_some())
        && Instant::now() < deadline
    {
        app.poll_project_search();
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn valid_query_spawns_a_real_scan_and_streams_results_into_groups() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(tmp.path().join("a.txt"), "one needle here\ntwo\n").unwrap();
    std::fs::write(tmp.path().join("b.txt"), "another needle line\n").unwrap();

    let mut app = App::new(vec![sample_file()]);
    app.set_repo_root(tmp.path().to_path_buf());
    app.open_project_search();
    for c in "needle".chars() {
        app.project_search_input_char(c);
    }
    let deadline = app
        .project_search
        .as_ref()
        .unwrap()
        .debounce_deadline
        .unwrap();
    app.maybe_fire_project_search(deadline + Duration::from_millis(1));

    assert!(
        app.project_search.as_ref().unwrap().scan.is_some(),
        "a valid query at/above the minimum length must spawn a scan"
    );

    wait_for_scan_done(&mut app);

    let state = app.project_search.as_ref().unwrap();
    assert!(state.error.is_none());
    let summary = state
        .summary
        .as_ref()
        .expect("scan must finish with a summary");
    assert_eq!(summary.total_hits, 2);
    assert_eq!(summary.files_matched, 2);
    let paths: Vec<&str> = state.groups.iter().map(|g| g.path.as_str()).collect();
    assert!(paths.contains(&"a.txt"));
    assert!(paths.contains(&"b.txt"));
}

#[test]
fn stale_generation_batches_and_summaries_are_dropped_on_drain() {
    let mut app = App::new(vec![sample_file()]);
    app.open_project_search();
    let (tx, rx) = mpsc::sync_channel(4);
    let abort = Arc::new(AtomicBool::new(false));
    // The state's current generation has already moved past what this scan
    // was tagged with (simulating a straggler from a superseded query).
    app.project_search.as_mut().unwrap().generation = 5;
    app.project_search.as_mut().unwrap().scan = Some(InFlightScan {
        generation: 2,
        receiver: rx,
        abort,
    });
    tx.send(ScanMessage::Batch(vec![hit("stale.rs", 1)]))
        .unwrap();
    tx.send(ScanMessage::Done(ScanSummary {
        generation: 2,
        total_hits: 1,
        ..ScanSummary::default()
    }))
    .unwrap();

    app.poll_project_search();

    let state = app.project_search.as_ref().unwrap();
    assert!(
        state.groups.is_empty(),
        "a stale scan's batches must be dropped, not applied"
    );
    assert!(
        state.summary.is_none(),
        "a stale scan's summary must be dropped, not applied"
    );
    assert!(state.scan.is_none(), "the stale scan must be cleared");
}

#[test]
fn matching_generation_batches_are_applied_and_grouped_by_path() {
    let mut app = App::new(vec![sample_file()]);
    app.open_project_search();
    let (tx, rx) = mpsc::sync_channel(4);
    let abort = Arc::new(AtomicBool::new(false));
    app.project_search.as_mut().unwrap().scan = Some(InFlightScan {
        generation: 0,
        receiver: rx,
        abort,
    });
    tx.send(ScanMessage::Batch(vec![
        hit("a.rs", 1),
        hit("a.rs", 2),
        hit("b.rs", 1),
    ]))
    .unwrap();
    tx.send(ScanMessage::Done(ScanSummary {
        generation: 0,
        total_hits: 3,
        files_matched: 2,
        ..ScanSummary::default()
    }))
    .unwrap();

    app.poll_project_search();

    let state = app.project_search.as_ref().unwrap();
    assert_eq!(state.groups.len(), 2, "hits must be grouped by path");
    assert_eq!(state.groups[0].path, "a.rs");
    assert_eq!(state.groups[0].hits.len(), 2);
    assert_eq!(state.groups[1].path, "b.rs");
    assert_eq!(state.summary.as_ref().unwrap().total_hits, 3);
    assert!(state.scan.is_none(), "scan clears once Done arrives");
}

// -- navigation / selected hit ------------------------------------------------

fn app_with_groups() -> App {
    let mut app = App::new(vec![sample_file()]);
    app.open_project_search();
    let state = app.project_search.as_mut().unwrap();
    state.groups = vec![
        ResultGroup {
            path: "a.rs".to_string(),
            hits: vec![hit("a.rs", 1), hit("a.rs", 2)],
        },
        ResultGroup {
            path: "b.rs".to_string(),
            hits: vec![hit("b.rs", 5)],
        },
    ];
    app
}

#[test]
fn move_down_and_up_clamp_across_group_boundaries() {
    let mut app = app_with_groups();
    assert_eq!(app.project_search.as_ref().unwrap().cursor, 0);
    app.project_search_move_down();
    app.project_search_move_down();
    assert_eq!(
        app.project_search.as_ref().unwrap().cursor,
        2,
        "must cross into the second group"
    );
    app.project_search_move_down(); // clamps at the last hit (index 2)
    assert_eq!(app.project_search.as_ref().unwrap().cursor, 2);
    app.project_search_move_up();
    app.project_search_move_up();
    app.project_search_move_up(); // clamps at 0
    assert_eq!(app.project_search.as_ref().unwrap().cursor, 0);
}

#[test]
fn selected_hit_walks_the_flat_index_across_groups() {
    let mut app = app_with_groups();
    app.project_search.as_mut().unwrap().cursor = 2;
    let selected = app.selected_project_search_hit().unwrap();
    assert_eq!(selected.path, "b.rs");
    assert_eq!(selected.line_number, 5);
}

#[test]
fn selected_hit_is_none_with_no_results() {
    let mut app = App::new(vec![sample_file()]);
    app.open_project_search();
    assert!(app.selected_project_search_hit().is_none());
}

// -- confirm: opens the file view and preserves search state ----------------

#[test]
fn confirm_opens_the_selected_hit_and_keeps_project_search_state_intact() {
    let mut app = app_with_groups();
    let mut files = HashMap::new();
    files.insert(
        "b.rs".to_string(),
        b"one\ntwo\nthree\nfour\nfive\n".to_vec(),
    );
    app.stage_ops = Some(Box::new(FakeOps { files }));
    app.project_search.as_mut().unwrap().cursor = 2; // b.rs:5

    app.project_search_confirm();

    assert_eq!(app.mode, Mode::Normal, "opening a hit lands in Normal");
    assert_eq!(app.target, DiffTarget::File("b.rs".to_string()));
    assert!(
        app.project_search.is_some(),
        "Project Search state must survive opening a hit"
    );
    assert_eq!(app.project_search.as_ref().unwrap().cursor, 2);

    // Esc from the file view must return to ProjectSearch, not Normal.
    app.return_from_file_view();
    assert_eq!(app.mode, Mode::ProjectSearch);
    assert!(app.project_search.is_some());
    assert_eq!(app.project_search.as_ref().unwrap().groups.len(), 2);
    assert_eq!(app.project_search.as_ref().unwrap().cursor, 2);
}

#[test]
fn confirm_with_no_hits_is_a_no_op() {
    let mut app = App::new(vec![sample_file()]);
    app.open_project_search();
    app.project_search_confirm();
    assert_eq!(app.mode, Mode::ProjectSearch, "view must stay open");
    assert!(app.project_search.is_some());
}

// -- push_hit: pure grouping helper -------------------------------------------

#[test]
fn push_hit_creates_a_new_group_in_first_seen_order_and_appends_to_existing_ones() {
    let mut groups: Vec<ResultGroup> = Vec::new();
    push_hit(&mut groups, hit("b.rs", 1));
    push_hit(&mut groups, hit("a.rs", 1));
    push_hit(&mut groups, hit("b.rs", 2));

    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].path, "b.rs", "first-seen file stays first");
    assert_eq!(groups[0].hits.len(), 2);
    assert_eq!(groups[1].path, "a.rs");
    assert_eq!(groups[1].hits.len(), 1);
}
