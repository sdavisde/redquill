use ratatui::Terminal;
use ratatui::backend::TestBackend;

use crate::diff::FileDiff;
use crate::git::RawFilePatch;

use super::super::pr_description::{PrDescriptionReturn, PrDescriptionState};
use super::*;

// -- fixtures ----------------------------------------------------------------

fn file() -> FileDiff {
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

fn detail(body: &str) -> PrDetail {
    PrDetail {
        number: 34,
        title: "Add widget support".to_string(),
        author: "octocat".to_string(),
        base_ref: "main".to_string(),
        head_ref: "feature/widget".to_string(),
        body: body.to_string(),
        is_draft: false,
        updated_at: "2020-01-02T03:04:05Z".to_string(),
    }
}

/// An `App` with the description overlay open on PR 34, `outcome` deciding
/// what the cache holds for it (`None` = still loading).
fn overlay_app(outcome: Option<PrDetailOutcome>, ret: PrDescriptionReturn) -> App {
    let mut app = App::new(vec![file()]);
    app.pr_description = Some(PrDescriptionState::new(34));
    app.mode = Mode::PrDescription { ret };
    if let Some(outcome) = outcome {
        app.pr_details.insert(34, outcome);
    }
    app
}

/// Draws the overlay at `width`x`height` and returns the flattened buffer
/// symbols, the same render harness the other modal tests use.
fn render_at(app: &App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    let area = Rect::new(0, 0, width, height);
    terminal.draw(|frame| render(frame, area, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect()
}

fn render_overlay(app: &App) -> String {
    render_at(app, 100, 24)
}

// -- title / meta text --------------------------------------------------------

#[test]
fn title_names_the_number_and_title_and_marks_a_draft() {
    assert_eq!(title_text(&detail("x")), "#34 Add widget support");

    let mut draft = detail("x");
    draft.is_draft = true;
    assert_eq!(title_text(&draft), "#34 Add widget support (draft)");

    // A title-less PR still names its number rather than rendering a stray
    // separator.
    let mut untitled = detail("x");
    untitled.title = String::new();
    assert_eq!(title_text(&untitled), "#34");
}

#[test]
fn meta_line_reads_author_then_head_into_base_then_a_relative_time() {
    // A fixed `now` a day after the fixture's `updated_at`.
    let now = 1_577_934_245 + 86_400;
    assert_eq!(
        meta_text(&detail("x"), now),
        "octocat \u{b7} feature/widget \u{2192} main \u{b7} 1d ago"
    );
}

/// An unparseable provider timestamp must fall back to the raw string rather
/// than blanking the field — the same degrade the picker row takes.
#[test]
fn meta_line_falls_back_to_the_raw_timestamp_when_it_does_not_parse() {
    let mut odd = detail("x");
    odd.updated_at = "yesterday-ish".to_string();
    assert!(meta_text(&odd, 0).ends_with("yesterday-ish"));
}

// -- body wrapping ------------------------------------------------------------

/// Author line breaks are structure, not noise: an empty line between
/// paragraphs must survive as its own row, and a long line must soft-wrap
/// rather than being truncated.
#[test]
fn wrapping_preserves_line_breaks_and_soft_wraps_long_lines() {
    let rows = wrapped("one\n\ntwo three four", 8);
    assert_eq!(rows, vec!["one", "", "two ", "three ", "four"]);
}

// -- scroll clamp -------------------------------------------------------------

/// Content taller than the viewport clamps to its last full screen; content
/// that fits can't be scrolled at all. This is the guard against the
/// unbounded-counter bug: an offset that outruns the content scrolls the whole
/// description off the top of a viewport that had nothing left to show.
#[test]
fn scroll_clamps_to_the_last_screen_and_short_content_never_scrolls() {
    // 40 rows in a 10-row viewport: the last screen starts at row 30.
    assert_eq!(clamp_scroll(u16::MAX, 40, 10), 30);
    assert_eq!(clamp_scroll(5, 40, 10), 5);
    // Exactly-fitting and shorter-than-viewport content: max offset is 0.
    assert_eq!(clamp_scroll(u16::MAX, 10, 10), 0);
    assert_eq!(clamp_scroll(3, 4, 10), 0);
}

/// The render pass writes the clamped offset back into the caller's `Cell`
/// (the help overlay's contract), so the stored state and what's on screen
/// can never disagree after a `G`-style overshoot.
#[test]
fn render_writes_the_clamped_scroll_offset_back() {
    let body = (0..200)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let app = overlay_app(
        Some(PrDetailOutcome::Ready(detail(&body))),
        PrDescriptionReturn::Session,
    );
    if let Some(state) = app.pr_description.as_ref() {
        state.scroll.set(u16::MAX);
    }
    let _ = render_overlay(&app);

    let state = app.pr_description.as_ref().unwrap();
    let viewport = state.viewport.get();
    assert!(viewport > 0, "render must record the viewport height");
    let offset = state.scroll.get();
    assert!(
        offset > 0 && offset < u16::MAX,
        "an overshoot must be clamped to a real offset, got {offset}"
    );
    // The clamped offset must leave the region full: content height (202
    // rows: meta + blank + 200 body lines) minus the viewport.
    assert_eq!(offset, 202 - viewport);
}

/// A one-line description in a tall overlay must stay pinned at the top no
/// matter what offset the key handler accumulated.
#[test]
fn a_short_description_stays_pinned_at_offset_zero() {
    let app = overlay_app(
        Some(PrDetailOutcome::Ready(detail("short"))),
        PrDescriptionReturn::Session,
    );
    if let Some(state) = app.pr_description.as_ref() {
        state.scroll.set(99);
    }
    let content = render_overlay(&app);

    assert_eq!(app.pr_description.as_ref().unwrap().scroll.get(), 0);
    assert!(content.contains("short"), "{content}");
}

// -- the three body states ----------------------------------------------------

#[test]
fn body_states_each_render_their_own_line_never_a_blank_region() {
    let cases: [(Option<PrDetailOutcome>, &str); 4] = [
        (None, "loading"),
        (
            Some(PrDetailOutcome::Unavailable),
            "description unavailable",
        ),
        (Some(PrDetailOutcome::Ready(detail(""))), "no description"),
        (
            Some(PrDetailOutcome::Ready(detail("what this PR does"))),
            "what this PR does",
        ),
    ];
    for (outcome, expected) in cases {
        let app = overlay_app(outcome, PrDescriptionReturn::Session);
        let content = render_overlay(&app);
        assert!(
            content.contains(expected),
            "missing {expected:?} in:\n{content}"
        );
    }
}

/// A whitespace-only body is "no description" too — a body of blank lines
/// would otherwise render as an empty region indistinguishable from a bug.
#[test]
fn a_whitespace_only_body_reads_as_no_description() {
    let app = overlay_app(
        Some(PrDetailOutcome::Ready(detail("\n  \n"))),
        PrDescriptionReturn::Session,
    );
    assert!(render_overlay(&app).contains("no description"));
}

/// Before the read lands, the title shows only the number: a cached title
/// from another PR must never fill the gap.
#[test]
fn a_loading_overlay_titles_only_the_number() {
    let mut app = overlay_app(None, PrDescriptionReturn::Session);
    app.pr_details
        .insert(99, PrDetailOutcome::Ready(detail("other PR's body")));
    let content = render_overlay(&app);
    assert!(content.contains("#34"));
    assert!(
        !content.contains("Add widget support"),
        "another PR's cached title must not leak in:\n{content}"
    );
    assert!(!content.contains("other PR's body"));
}

#[test]
fn renders_nothing_outside_the_description_mode() {
    let app = App::new(vec![file()]);
    assert!(render_overlay(&app).trim().is_empty());
}

// The `start review`-only-from-the-launcher scoping now lives in the shared
// footer strip — asserted in `crate::ui::footer`'s tests
// (`pr_description_strip_drops_start_review_inside_a_review_session`).
