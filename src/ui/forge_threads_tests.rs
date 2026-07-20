use super::*;

use crate::annotate::Side;
use crate::diff::FileDiff;
use crate::forge::{Thread, ThreadAnchor, ThreadComment};
use crate::git::{DiffTarget, RawFilePatch};

use super::super::app::{App, Mode};
use super::super::rows::LineRow;

// -- fixtures ----------------------------------------------------------------

/// A one-hunk file whose sole added line is new-side line 1 (and whose sole
/// removed line is old-side line 1) — enough to anchor a thread on.
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

fn review_app(paths: &[&str]) -> App {
    let mut app = App::new(paths.iter().map(|p| file(p)).collect());
    app.target = DiffTarget::Review {
        base: "main".to_string(),
        branch: "redquill/pr/1".to_string(),
    };
    app
}

fn comment(id: u64, author: &str, created_at: &str, body: &str) -> ThreadComment {
    ThreadComment {
        id,
        author: author.to_string(),
        created_at: created_at.to_string(),
        body: body.to_string(),
    }
}

/// A positioned thread anchored at `(path, side, line)`, with `replies`
/// nested under a root.
fn positioned_thread(id: u64, path: &str, side: Side, line: u32, replies: usize) -> Thread {
    Thread {
        id,
        anchor: ThreadAnchor::Position {
            path: path.to_string(),
            side,
            line,
        },
        root: comment(id, "author", "2026-07-01T10:00:00Z", "root comment"),
        replies: (1..=replies as u64)
            .map(|i| comment(id * 100 + i, "reviewer", "2026-07-01T10:0{i}:00Z", "reply"))
            .collect(),
        resolved: false,
        outdated: false,
    }
}

// -- RFC3339 parsing ---------------------------------------------------------

#[test]
fn parse_rfc3339_reads_a_z_suffixed_utc_timestamp() {
    // 2026-07-01T00:00:00Z. Cross-check against the module's own inverse.
    let ts = parse_rfc3339_to_unix("2026-07-01T00:00:00Z").unwrap();
    assert_eq!(ts, days_from_civil(2026, 7, 1) * 86_400);
}

#[test]
fn parse_rfc3339_reads_hours_minutes_seconds() {
    let base = days_from_civil(2026, 7, 1) * 86_400;
    let ts = parse_rfc3339_to_unix("2026-07-01T01:02:03Z").unwrap();
    assert_eq!(ts, base + 3_600 + 120 + 3);
}

#[test]
fn parse_rfc3339_tolerates_a_fractional_and_offset_suffix() {
    assert!(parse_rfc3339_to_unix("2026-07-01T10:00:00.123+00:00").is_some());
}

#[test]
fn parse_rfc3339_rejects_a_non_timestamp() {
    assert_eq!(parse_rfc3339_to_unix("not a date"), None);
}

#[test]
fn days_from_civil_matches_the_epoch() {
    assert_eq!(days_from_civil(1970, 1, 1), 0);
    assert_eq!(days_from_civil(1970, 1, 2), 1);
}

// -- gutter marker decoration ------------------------------------------------

#[test]
fn decorate_marks_the_line_a_positioned_thread_anchors_on() {
    let mut app = review_app(&["a.rs"]);
    app.thread_overlay
        .replace(vec![positioned_thread(1, "a.rs", Side::New, 1, 0)]);
    app.rebuild_rows();

    let marked: Vec<&LineRow> = app
        .view
        .rows
        .iter()
        .filter_map(|r| match r {
            Row::Line(l) if l.thread => Some(l),
            _ => None,
        })
        .collect();
    assert_eq!(
        marked.len(),
        1,
        "exactly the anchored new-side line is marked"
    );
    assert_eq!(marked[0].new_line, Some(1));
}

#[test]
fn decorate_is_a_no_op_without_threads() {
    let mut app = review_app(&["a.rs"]);
    app.rebuild_rows();
    assert!(
        app.view
            .rows
            .iter()
            .all(|r| !matches!(r, Row::Line(l) if l.thread)),
        "no line is thread-marked when the overlay is empty"
    );
}

#[test]
fn decorate_does_not_mark_a_line_a_thread_does_not_anchor_on() {
    let mut app = review_app(&["a.rs"]);
    // Anchor on a line that isn't in the diff (line 99): nothing marked.
    app.thread_overlay
        .replace(vec![positioned_thread(1, "a.rs", Side::New, 99, 0)]);
    app.rebuild_rows();
    assert!(
        app.view
            .rows
            .iter()
            .all(|r| !matches!(r, Row::Line(l) if l.thread))
    );
}

// -- open thread view --------------------------------------------------------

#[test]
fn open_thread_view_opens_the_thread_on_the_cursor_line() {
    let mut app = review_app(&["a.rs"]);
    app.thread_overlay
        .replace(vec![positioned_thread(7, "a.rs", Side::New, 1, 2)]);
    app.rebuild_rows();
    // Put the cursor on the new-side line 1 row.
    let row = app
        .view
        .rows
        .iter()
        .position(|r| matches!(r, Row::Line(l) if l.new_line == Some(1)))
        .unwrap();
    app.view.cursor = row;

    app.open_thread_view();

    assert_eq!(app.mode, Mode::ThreadView);
    assert_eq!(app.thread_view.as_ref().unwrap().root_id, 7);
}

#[test]
fn open_thread_view_falls_back_to_the_files_first_thread_from_a_header_row() {
    let mut app = review_app(&["a.rs"]);
    app.thread_overlay
        .replace(vec![positioned_thread(9, "a.rs", Side::New, 1, 0)]);
    app.rebuild_rows();
    // Cursor on the file header (row 0): no exact line match, but the file
    // has a thread, so it opens the first one.
    app.view.cursor = 0;

    app.open_thread_view();

    assert_eq!(app.mode, Mode::ThreadView);
    assert_eq!(app.thread_view.as_ref().unwrap().root_id, 9);
}

#[test]
fn open_thread_view_is_a_no_op_with_a_hint_when_no_thread_here() {
    let mut app = review_app(&["a.rs"]);
    app.rebuild_rows();
    app.open_thread_view();
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.thread_view.is_none());
    assert!(app.status_message.is_some());
}

#[test]
fn close_thread_view_returns_to_normal() {
    let mut app = review_app(&["a.rs"]);
    app.thread_overlay
        .replace(vec![positioned_thread(1, "a.rs", Side::New, 1, 0)]);
    app.rebuild_rows();
    app.view.cursor = app
        .view
        .rows
        .iter()
        .position(|r| matches!(r, Row::Line(l) if l.new_line == Some(1)))
        .unwrap();
    app.open_thread_view();
    assert_eq!(app.mode, Mode::ThreadView);
    app.close_thread_view();
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.thread_view.is_none());
}

#[test]
fn thread_view_scroll_moves_and_clamps_at_zero() {
    let mut app = review_app(&["a.rs"]);
    app.thread_view = Some(ThreadViewState {
        root_id: 1,
        scroll: 0,
    });
    app.mode = Mode::ThreadView;
    app.thread_view_scroll_down();
    app.thread_view_scroll_down();
    assert_eq!(app.thread_view.as_ref().unwrap().scroll, 2);
    app.thread_view_scroll_up();
    assert_eq!(app.thread_view.as_ref().unwrap().scroll, 1);
    app.thread_view_scroll_up();
    app.thread_view_scroll_up();
    assert_eq!(
        app.thread_view.as_ref().unwrap().scroll,
        0,
        "scroll-up saturates at zero"
    );
}

// -- next/prev navigation ----------------------------------------------------

#[test]
fn next_thread_moves_the_cursor_to_the_next_thread_anchor() {
    let mut app = review_app(&["a.rs", "b.rs"]);
    app.thread_overlay.replace(vec![
        positioned_thread(1, "a.rs", Side::New, 1, 0),
        positioned_thread(2, "b.rs", Side::New, 1, 0),
    ]);
    app.rebuild_rows();
    app.view.cursor = 0;

    app.next_thread();
    let first = app.view.cursor;
    // Lands on a threaded row.
    assert!(matches!(app.view.rows.get(first), Some(Row::Line(l)) if l.thread));

    app.next_thread();
    let second = app.view.cursor;
    assert!(matches!(app.view.rows.get(second), Some(Row::Line(l)) if l.thread));
    assert_ne!(first, second, "advances to the second thread");
}

#[test]
fn prev_thread_wraps_to_the_last_thread() {
    let mut app = review_app(&["a.rs", "b.rs"]);
    app.thread_overlay.replace(vec![
        positioned_thread(1, "a.rs", Side::New, 1, 0),
        positioned_thread(2, "b.rs", Side::New, 1, 0),
    ]);
    app.rebuild_rows();
    app.view.cursor = 0;
    app.prev_thread();
    // From the top, prev wraps to the last (b.rs) thread's anchor.
    assert!(matches!(app.view.rows.get(app.view.cursor), Some(Row::Line(l)) if l.thread));
}

#[test]
fn next_thread_hints_when_no_threads_exist() {
    let mut app = review_app(&["a.rs"]);
    app.rebuild_rows();
    app.next_thread();
    assert!(app.status_message.is_some());
}

// -- fetch application + poll guard ------------------------------------------

#[test]
fn apply_thread_fetch_ok_swaps_the_overlay_and_clears_the_flag() {
    let mut app = review_app(&["a.rs"]);
    app.threads_unavailable = true;
    app.apply_thread_fetch(Ok(vec![positioned_thread(1, "a.rs", Side::New, 1, 0)]));
    assert_eq!(app.thread_overlay.len(), 1);
    assert!(!app.threads_unavailable);
}

#[test]
fn apply_thread_fetch_err_sets_the_notice_and_keeps_the_prior_overlay() {
    let mut app = review_app(&["a.rs"]);
    app.thread_overlay
        .replace(vec![positioned_thread(1, "a.rs", Side::New, 1, 0)]);
    app.apply_thread_fetch(Err("offline".to_string()));
    assert!(app.threads_unavailable);
    assert!(
        app.status_message
            .as_deref()
            .unwrap()
            .contains("comments unavailable")
    );
    assert_eq!(
        app.thread_overlay.len(),
        1,
        "a failed refresh keeps the last-seen threads"
    );
}

#[test]
fn poll_applies_a_current_generation_result() {
    let mut app = review_app(&["a.rs"]);
    let generation = app.thread_fetch_generation;
    let id = app
        .thread_fetch_tasks
        .spawn(move || Ok(vec![positioned_thread(1, "a.rs", Side::New, 1, 0)]));
    app.thread_fetch_in_flight = Some(InFlightThreadFetch { id, generation });

    // Drain until the background task lands.
    for _ in 0..200 {
        app.poll_thread_fetch();
        if app.thread_overlay.len() == 1 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(app.thread_overlay.len(), 1);
    assert!(app.thread_fetch_in_flight.is_none());
}

#[test]
fn poll_drops_a_stale_generation_result() {
    let mut app = review_app(&["a.rs"]);
    // Spawn a task tagged with an already-superseded generation.
    let stale_generation = app.thread_fetch_generation;
    let id = app
        .thread_fetch_tasks
        .spawn(move || Ok(vec![positioned_thread(1, "a.rs", Side::New, 1, 0)]));
    app.thread_fetch_in_flight = Some(InFlightThreadFetch {
        id,
        generation: stale_generation,
    });
    // A newer session/refresh bumped the generation past the in-flight one.
    app.thread_fetch_generation = app.thread_fetch_generation.wrapping_add(1);

    for _ in 0..200 {
        app.poll_thread_fetch();
        if app.thread_fetch_in_flight.is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(
        app.thread_overlay.is_empty(),
        "a stale-generation result must be dropped, not applied"
    );
}
