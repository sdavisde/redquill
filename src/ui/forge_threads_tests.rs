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

#[test]
fn reaching_for_threads_after_a_failed_fetch_re_surfaces_the_unavailable_notice() {
    // A failed fetch leaves `threads_unavailable` set and no overlay. Any
    // later reach for threads (jump or open) re-explains why nothing is
    // there, so the reviewer is never silently blind to feedback that may
    // exist on the PR — the persistent-on-demand half of FR-13's notice.
    let mut app = review_app(&["a.rs"]);
    app.apply_thread_fetch(Err("boom".to_string()));
    assert!(app.threads_unavailable);

    app.status_message = None;
    app.next_thread();
    assert_eq!(
        app.status_message.as_deref(),
        Some("comments unavailable \u{2014} reviewing without them")
    );

    app.status_message = None;
    app.open_thread_view();
    assert_eq!(
        app.status_message.as_deref(),
        Some("comments unavailable \u{2014} reviewing without them")
    );
}

#[test]
fn reaching_for_threads_with_none_and_no_failure_gives_the_plain_hint() {
    let mut app = review_app(&["a.rs"]);
    app.rebuild_rows();
    app.next_thread();
    assert_eq!(app.status_message.as_deref(), Some("no comment threads"));
}

// -- published-copy dedupe (FR-15) -------------------------------------------

/// Helper: the ids of annotations whose body rows are actually rendered in
/// the diff buffer (a `Row::Annotation` carries its annotation's id).
fn rendered_annotation_ids(app: &App) -> std::collections::HashSet<usize> {
    app.view
        .rows
        .iter()
        .filter_map(|r| match r {
            Row::Annotation { id, .. } => Some(*id),
            _ => None,
        })
        .collect()
}

#[test]
fn a_published_annotation_covered_by_a_fetched_thread_is_not_drawn_as_a_local_row() {
    use crate::annotate::{Classification, Target};

    let mut app = review_app(&["a.rs"]);
    // A published annotation on new-side line 1, and a fetched thread anchored
    // at the very same position — the forge copy is authoritative on screen.
    let id = app
        .annotations
        .add(
            Target::line("a.rs", 1, Side::New),
            Classification::Issue,
            "already posted this",
        )
        .unwrap();
    app.annotations.set_published(id, true).unwrap();
    app.thread_overlay
        .replace(vec![positioned_thread(1, "a.rs", Side::New, 1, 0)]);
    app.rebuild_rows();

    assert!(
        !rendered_annotation_ids(&app).contains(&id),
        "a published annotation whose forge copy is shown must not be re-drawn locally"
    );
    // It still lives in the store: editable, listed, and — crucially — still
    // serialized to stdout (the stdout contract includes every annotation
    // regardless of published state).
    assert_eq!(app.annotations.len(), 1);
    assert!(
        crate::annotate::render_markdown(&app.annotations).contains("already posted this"),
        "the published annotation must still reach stdout"
    );
}

#[test]
fn an_unpublished_annotation_at_a_thread_anchor_is_still_drawn() {
    use crate::annotate::{Classification, Target};

    let mut app = review_app(&["a.rs"]);
    let id = app
        .annotations
        .add(
            Target::line("a.rs", 1, Side::New),
            Classification::Issue,
            "not yet posted",
        )
        .unwrap();
    // Left unpublished: even with a thread at the same anchor, a local draft
    // the reviewer hasn't published is theirs to see.
    app.thread_overlay
        .replace(vec![positioned_thread(1, "a.rs", Side::New, 1, 0)]);
    app.rebuild_rows();

    assert!(
        rendered_annotation_ids(&app).contains(&id),
        "an unpublished annotation must render even at a threaded anchor"
    );
}

#[test]
fn a_published_annotation_with_no_matching_thread_is_still_drawn() {
    use crate::annotate::{Classification, Target};

    let mut app = review_app(&["a.rs"]);
    let id = app
        .annotations
        .add(
            Target::line("a.rs", 1, Side::New),
            Classification::Issue,
            "posted, but no thread here",
        )
        .unwrap();
    app.annotations.set_published(id, true).unwrap();
    // A thread exists, but on the *old* side — a different anchor — so the
    // published annotation has no forge copy on screen and must still render.
    app.thread_overlay
        .replace(vec![positioned_thread(1, "a.rs", Side::Old, 1, 0)]);
    app.rebuild_rows();

    assert!(
        rendered_annotation_ids(&app).contains(&id),
        "dedupe must require an actual anchor match, not just any published flag"
    );
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

// -- Journey transcript: 5-and-5 conversation + drafted reply (FR-12/FR-14) --

/// Flattens a full-screen render of `render_fn` into newline-joined rows,
/// trailing spaces trimmed, for the human-readable transcript below.
fn flatten<F: FnOnce(&mut ratatui::Frame)>(width: u16, height: u16, render_fn: F) -> String {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| render_fn(frame)).unwrap();
    let buf = terminal.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..height {
        let mut row = String::new();
        for x in 0..width {
            row.push_str(buf[(x, y)].symbol());
        }
        out.push_str(row.trim_end());
        out.push('\n');
    }
    out
}

/// The index at which `needle` first appears in `haystack`, or a large
/// sentinel when absent (so an out-of-order or missing line fails the
/// monotonic-order assertion loudly).
fn order_of(haystack: &str, needle: &str) -> usize {
    haystack.find(needle).unwrap_or(usize::MAX)
}

/// Journey driver for spec 13's Unit 3 success metric (conversation
/// fidelity): a PR thread with a five-and-five back-and-forth renders
/// top-to-bottom in conversation order, and a reply drafted in the terminal
/// appears in the annotation panel with its reply marker. Every logged step
/// is asserted, so this is both a regression test and the transcript
/// generator (`RQ_JOURNEY_DUMP=1 cargo test --lib
/// thread_conversation_and_reply_journey_transcript -- --nocapture` captures
/// the persisted `13-proofs/` transcript).
#[test]
fn thread_conversation_and_reply_journey_transcript() {
    use super::super::keymap::Keymap;

    let mut log = String::new();
    let mut step = |title: &str, body: &str| {
        log.push_str(&format!("\n=== {title} ===\n{body}\n"));
    };

    let mut app = review_app(&["a.rs"]);

    // A single thread anchored on new-side line 1, five messages from each of
    // two participants (alice authored the root + four replies, bob five
    // replies), interleaved as a real back-and-forth.
    let conversation = Thread {
        id: 1,
        anchor: ThreadAnchor::Position {
            path: "a.rs".to_string(),
            side: Side::New,
            line: 1,
        },
        root: comment(
            1,
            "alice",
            "2026-07-19T10:00:00Z",
            "Should this handle the empty input?",
        ),
        replies: vec![
            comment(
                2,
                "bob",
                "2026-07-19T10:05:00Z",
                "Good catch — it panics today.",
            ),
            comment(3, "alice", "2026-07-19T10:10:00Z", "Right, let's guard it."),
            comment(
                4,
                "bob",
                "2026-07-19T10:15:00Z",
                "Guard added in the latest push.",
            ),
            comment(
                5,
                "alice",
                "2026-07-19T10:20:00Z",
                "Does a test cover it now?",
            ),
            comment(
                6,
                "bob",
                "2026-07-19T10:25:00Z",
                "Added a unit test for empty input.",
            ),
            comment(
                7,
                "alice",
                "2026-07-19T10:30:00Z",
                "One nit: name it explicitly.",
            ),
            comment(
                8,
                "bob",
                "2026-07-19T10:35:00Z",
                "Renamed to empty_input_is_rejected.",
            ),
            comment(9, "alice", "2026-07-19T10:40:00Z", "LGTM once CI is green."),
            comment(10, "bob", "2026-07-19T10:45:00Z", "CI passed — thanks!"),
        ],
        resolved: false,
        outdated: false,
    };

    // The fetch lands (as the background poller would apply it).
    app.apply_thread_fetch(Ok(vec![conversation]));
    assert_eq!(app.thread_overlay.len(), 1);
    let marked = app
        .view
        .rows
        .iter()
        .filter(|r| matches!(r, Row::Line(l) if l.thread))
        .count();
    step(
        "fetch lands: one thread, its anchored line gets a gutter marker",
        &format!(
            "threads: {}  marked lines: {marked}",
            app.thread_overlay.len()
        ),
    );

    // Open the thread overlay (T) on the anchored file and render it. A tall
    // frame so the whole ten-message conversation fits without scrolling.
    app.open_thread_view();
    assert_eq!(app.mode, Mode::ThreadView);
    let overlay = flatten(100, 60, |frame| super::render(frame, frame.area(), &app));

    // Conversation order: every message body appears, and each strictly after
    // the one before it — top-to-bottom, replies under the root.
    let ordered_bodies = [
        "Should this handle the empty input?",
        "Good catch",
        "let's guard it",
        "Guard added",
        "Does a test cover it",
        "Added a unit test",
        "One nit",
        "Renamed to empty_input_is_rejected",
        "LGTM once CI is green",
        "CI passed",
    ];
    let mut last = 0usize;
    for body in ordered_bodies {
        let at = order_of(&overlay, body);
        assert!(
            at != usize::MAX && at >= last,
            "conversation must render in order; {body:?} was out of place"
        );
        last = at;
    }
    step(
        "press T: the whole 5-and-5 conversation renders top-to-bottom",
        overlay.trim_end(),
    );

    // Draft a reply to the thread (r), type it, submit.
    app.open_reply_compose();
    assert_eq!(app.mode, Mode::Compose);
    assert_eq!(app.compose.as_ref().and_then(|c| c.thread_id), Some(1));
    if let Some(compose) = app.compose.as_mut() {
        for ch in "I'll take the empty-input guard.".chars() {
            compose.buffer.insert_char(ch);
        }
    }
    app.submit_compose();
    assert_eq!(app.replies.len(), 1);
    step(
        "press r, type, submit: the draft reply joins the local review",
        &format!(
            "drafted replies: {}  (targets thread {})",
            app.replies.len(),
            app.replies.iter().next().unwrap().thread_id
        ),
    );

    // Open the annotation panel; the reply shows with its ↳ marker.
    app.mode = Mode::List;
    let km = Keymap::default_map();
    let panel = flatten(80, 12, |frame| {
        super::super::list_panel::render(frame, frame.area(), &app, &km)
    });
    assert!(
        panel.contains('\u{21b3}') && panel.contains("I'll take the empty-input guard."),
        "the drafted reply must appear in the notes panel with its reply marker:\n{panel}"
    );
    step(
        "open the notes panel: the drafted reply is listed with its ↳ marker",
        panel.trim_end(),
    );

    if std::env::var("RQ_JOURNEY_DUMP").is_ok() {
        eprintln!("{log}");
    }
}
