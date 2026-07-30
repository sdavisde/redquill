use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use crate::annotate::{Classification, Side, Target};
use crate::diff::FileDiff;
use crate::forge::{
    ForgeError, ForgeSubmitExecutor, ReviewPayload, SubmitAttempt, SubmitReport, Thread,
    ThreadAnchor, ThreadComment, Verdict,
};
use crate::git::{DiffTarget, RawFilePatch};
use crate::review::store::{ForgeMetadata, ForgeProviderKind};

use super::super::app::{App, Mode, ModeOrigin, PanelTab};
use super::super::modes::handle_submit_result_key;
use super::*;

// -- fixtures ----------------------------------------------------------------

fn file(path: &str) -> FileDiff {
    let raw = format!(
        "diff --git a/{path} b/{path}\nindex 1..2 100644\n--- a/{path}\n+++ b/{path}\n@@ -1,3 +1,3 @@\n fn f() {{\n-    old();\n+    new();\n"
    );
    FileDiff::from_patch(&RawFilePatch {
        path: path.to_string(),
        old_path: None,
        raw,
        is_binary: false,
    })
    .unwrap()
}

/// A GitHub PR review session, so `submit-forge-review` is live.
fn review_app(paths: &[&str]) -> App {
    let mut app = App::new(paths.iter().map(|p| file(p)).collect());
    app.target = DiffTarget::Review {
        base: "main".to_string(),
        branch: "redquill/pr/34".to_string(),
    };
    app.review_forge = Some(ForgeMetadata {
        provider: ForgeProviderKind::GitHub,
        host: "github.com".to_string(),
        number: 34,
        title: String::new(),
        last_head_sha: "deadbeef".to_string(),
        diff_refs: None,
    });
    app
}

fn thread(id: u64, author: &str, path: &str, line: u32) -> Thread {
    Thread {
        id,
        anchor: ThreadAnchor::Position {
            path: path.to_string(),
            side: Side::New,
            line,
        },
        root: ThreadComment {
            id,
            author: author.to_string(),
            created_at: "2026-07-01T10:00:00Z".to_string(),
            body: "root".to_string(),
        },
        replies: Vec::new(),
        resolved: false,
        outdated: false,
        discussion_id: None,
    }
}

fn boom() -> ForgeError {
    ForgeError::Command {
        cli: "gh",
        command: "api".to_string(),
        code: "403".to_string(),
        stderr: "HTTP 403: Resource not accessible".to_string(),
    }
}

/// A fake [`ForgeSubmitExecutor`] (no `gh`/`glab` is ever run) that fails a
/// chosen phase of the sequence, so a real [`SubmitReport`] can be produced for
/// a genuinely partial run.
struct PartialSubmitter {
    review_ok: bool,
    file_comments_ok: bool,
    replies_ok: bool,
}

impl ForgeSubmitExecutor for PartialSubmitter {
    fn submit_review(&self, _payload: &ReviewPayload) -> Result<(), ForgeError> {
        if self.review_ok { Ok(()) } else { Err(boom()) }
    }

    fn post_file_comment(&self, _path: &str, _body: &str) -> Result<(), ForgeError> {
        if self.file_comments_ok {
            Ok(())
        } else {
            Err(boom())
        }
    }

    fn post_reply(&self, _thread_id: u64, _body: &str) -> Result<(), ForgeError> {
        if self.replies_ok { Ok(()) } else { Err(boom()) }
    }
}

/// A review with a line comment (rides the atomic review POST), a file comment
/// (a follow-up), and a drafted reply — one item per submit phase, so a failure
/// in any phase leaves a genuinely mixed outcome.
fn app_with_one_item_per_phase() -> App {
    let mut app = review_app(&["src/a.rs"]);
    app.annotations
        .add(
            Target::line("src/a.rs", 2, Side::New),
            Classification::Issue,
            "fix the line",
        )
        .unwrap();
    app.annotations
        .add(
            Target::file("src/a.rs"),
            Classification::Praise,
            "nice file",
        )
        .unwrap();
    app.replies.add(100, "agreed").unwrap();
    app.thread_overlay
        .replace(vec![thread(100, "alice", "src/a.rs", 12)]);
    app
}

/// A stopped run over `n` line comments, none of which published — enough rows
/// to overflow any sensibly sized modal.
fn app_with_a_long_result(n: usize) -> App {
    let mut app = review_app(&["src/a.rs"]);
    let mut ids = Vec::new();
    for i in 0..n {
        ids.push(
            app.annotations
                .add(
                    Target::line("src/a.rs", 2, Side::New),
                    Classification::Issue,
                    format!("comment number {i}"),
                )
                .unwrap(),
        );
    }
    app.apply_submit_outcome(SubmitReport {
        failure: Some("HTTP 403".to_string()),
        attempt: SubmitAttempt {
            annotation_ids: ids,
            reply_ids: Vec::new(),
            review_post: true,
        },
        ..SubmitReport::default()
    });
    app
}

/// Renders the modal over a `width` x `height` terminal and returns its cell
/// symbols as one string.
fn render_modal(app: &App, width: u16, height: u16) -> String {
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

/// Runs the real submit sequence over the app's batch against a fake that
/// fails the named phase, then applies the report exactly as the poller does.
fn submit_with(app: &mut App, exec: &PartialSubmitter) {
    let batch = app.build_submit_batch(Verdict::Comment, Some("overall"));
    let report = crate::forge::run_submit_sequence(&batch, exec);
    app.apply_submit_outcome(report);
}

// -- when the result view opens at all ---------------------------------------

#[test]
fn a_clean_submit_reports_in_the_status_line_and_opens_no_modal() {
    let mut app = app_with_one_item_per_phase();
    submit_with(
        &mut app,
        &PartialSubmitter {
            review_ok: true,
            file_comments_ok: true,
            replies_ok: true,
        },
    );

    assert!(
        app.submit_result.is_none(),
        "a fully published submit needs no per-item accounting"
    );
    assert_eq!(app.mode, Mode::Normal);
    assert!(
        app.status_message
            .as_deref()
            .is_some_and(|m| m.contains("review submitted")),
        "status: {:?}",
        app.status_message
    );
}

#[test]
fn a_partial_failure_opens_the_result_view_and_still_sets_the_status_line() {
    let mut app = app_with_one_item_per_phase();
    // The review POST lands (the line comment publishes), the file comment
    // follow-up fails, so the reply is never attempted.
    submit_with(
        &mut app,
        &PartialSubmitter {
            review_ok: true,
            file_comments_ok: false,
            replies_ok: true,
        },
    );

    let view = &app
        .submit_result
        .as_ref()
        .expect("a stopped run opens the result view")
        .view;
    assert_eq!(view.published.len(), 1, "{view:?}");
    assert!(view.published[0].contains("src/a.rs:2"), "{view:?}");
    assert!(view.pending_drafts.is_empty(), "{view:?}");
    // The unsent file comment and the never-attempted reply.
    assert_eq!(view.not_sent.len(), 2, "{view:?}");
    assert!(
        view.not_sent.iter().any(|r| r.contains("src/a.rs")),
        "{view:?}"
    );
    assert!(
        view.not_sent
            .iter()
            .any(|r| r.contains("to alice @ src/a.rs:12")),
        "the reply keeps the humanized label the preview gave it: {view:?}"
    );
    assert!(
        !view.not_sent.iter().any(|r| r.contains("review itself")),
        "the review POST did land, so it must not be listed unsent: {view:?}"
    );
    assert!(view.diagnostic.contains("403"), "{view:?}");
    assert!(
        app.status_message
            .as_deref()
            .is_some_and(|m| m.contains("submit stopped")),
        "the one-line status stays for after the modal is dismissed: {:?}",
        app.status_message
    );
}

#[test]
fn a_total_failure_lists_every_item_and_the_review_post_as_not_sent() {
    let mut app = app_with_one_item_per_phase();
    submit_with(
        &mut app,
        &PartialSubmitter {
            review_ok: false,
            file_comments_ok: true,
            replies_ok: true,
        },
    );

    let view = &app
        .submit_result
        .as_ref()
        .expect("a total failure opens the result view")
        .view;
    assert!(view.published.is_empty(), "{view:?}");
    assert!(view.pending_drafts.is_empty(), "{view:?}");
    // The review itself, then the two annotations, then the reply.
    assert_eq!(view.not_sent.len(), 4, "{view:?}");
    assert!(
        view.not_sent[0].contains("review itself"),
        "the failed review POST leads the group: {view:?}"
    );
}

// -- grouping across all three outcomes --------------------------------------

#[test]
fn published_pending_and_unsent_items_land_in_their_own_groups() {
    // A GitLab-shaped stop: one comment published, one left as a private
    // draft, one never attempted, one reply never attempted, and the review
    // itself not posted.
    let mut app = review_app(&["src/a.rs"]);
    let published = app
        .annotations
        .add(
            Target::line("src/a.rs", 2, Side::New),
            Classification::Issue,
            "published one",
        )
        .unwrap();
    let drafted = app
        .annotations
        .add(
            Target::range("src/a.rs", 3, 5, Side::New).unwrap(),
            Classification::Nit,
            "drafted one",
        )
        .unwrap();
    let unsent = app
        .annotations
        .add(
            Target::file("src/a.rs"),
            Classification::Question,
            "unsent one",
        )
        .unwrap();
    let reply = app.replies.add(100, "agreed").unwrap();

    let report = SubmitReport {
        published_annotation_ids: vec![published],
        draft_annotation_ids: vec![drafted],
        failure: Some("HTTP 403: forbidden (write blocked)".to_string()),
        attempt: SubmitAttempt {
            annotation_ids: vec![published, drafted, unsent],
            reply_ids: vec![reply],
            review_post: true,
        },
        ..SubmitReport::default()
    };
    let view = build_result(&report, &app.annotations, &app.replies, &app.thread_overlay);

    assert_eq!(view.published.len(), 1);
    assert!(view.published[0].contains("src/a.rs:2"), "{view:?}");
    assert!(view.published[0].contains("issue"), "{view:?}");
    assert_eq!(view.pending_drafts.len(), 1);
    assert!(
        view.pending_drafts[0].contains("src/a.rs:3-5"),
        "a range anchor keeps its span: {view:?}"
    );
    assert_eq!(view.not_sent.len(), 3, "{view:?}");
    assert!(view.not_sent[0].contains("review itself"), "{view:?}");
    assert!(view.not_sent[1].contains("unsent one"), "{view:?}");
    assert!(
        view.not_sent[2].contains("thread 100"),
        "with no thread in the overlay the id is the only honest label: {view:?}"
    );
    assert!(view.diagnostic.contains("write blocked"), "{view:?}");
}

#[test]
fn a_published_item_is_never_also_reported_as_a_pending_draft() {
    // The GitLab bulk publish flips a pre-existing draft: the same id appears
    // in both lists' inputs, and only "published" is true of it.
    let mut app = review_app(&["src/a.rs"]);
    let id = app
        .annotations
        .add(
            Target::line("src/a.rs", 2, Side::New),
            Classification::Issue,
            "fix",
        )
        .unwrap();
    let report = SubmitReport {
        published_annotation_ids: vec![id],
        draft_annotation_ids: vec![id],
        failure: Some("approve failed".to_string()),
        attempt: SubmitAttempt {
            annotation_ids: vec![id],
            ..SubmitAttempt::default()
        },
        ..SubmitReport::default()
    };
    let view = build_result(&report, &app.annotations, &app.replies, &app.thread_overlay);
    assert_eq!(view.published.len(), 1);
    assert!(view.pending_drafts.is_empty(), "{view:?}");
}

#[test]
fn a_reply_only_resume_never_reports_a_review_post_it_did_not_owe() {
    // `include_review_post: false` (a resume): the sequence owed no review
    // POST, so a later failure must not accuse it of skipping one.
    let mut app = review_app(&["src/a.rs"]);
    let reply = app.replies.add(100, "agreed").unwrap();
    app.forge_review_submitted = true;
    let batch = app.build_submit_batch(Verdict::Comment, None);
    assert!(!batch.include_review_post);

    let report = crate::forge::run_submit_sequence(
        &batch,
        &PartialSubmitter {
            review_ok: true,
            file_comments_ok: true,
            replies_ok: false,
        },
    );
    let view = build_result(&report, &app.annotations, &app.replies, &app.thread_overlay);
    assert!(
        !view.not_sent.iter().any(|r| r.contains("review itself")),
        "{view:?}"
    );
    assert_eq!(view.not_sent.len(), 1, "just the reply: {view:?}");
    let _ = reply;
}

// -- keys --------------------------------------------------------------------

fn press(app: &mut App, code: KeyCode) {
    handle_submit_result_key(app, KeyEvent::new(code, KeyModifiers::NONE));
}

#[test]
fn every_dismiss_key_restores_the_mode_the_report_interrupted() {
    for code in [KeyCode::Enter, KeyCode::Esc, KeyCode::Char('q')] {
        let mut app = app_with_one_item_per_phase();
        app.mode = Mode::Panel {
            cursor: 3,
            tab: PanelTab::Changes,
        };
        submit_with(
            &mut app,
            &PartialSubmitter {
                review_ok: false,
                file_comments_ok: true,
                replies_ok: true,
            },
        );
        assert_eq!(
            app.mode,
            Mode::SubmitResult {
                origin: ModeOrigin::Panel {
                    cursor: 3,
                    tab: PanelTab::Changes,
                },
            },
            "{code:?}"
        );

        press(&mut app, code);
        assert_eq!(
            app.mode,
            Mode::Panel {
                cursor: 3,
                tab: PanelTab::Changes
            },
            "{code:?} must restore the interrupted mode"
        );
        assert!(app.submit_result.is_none(), "{code:?}");
    }
}

#[test]
fn retry_reopens_the_submit_modal_with_only_what_did_not_land() {
    let mut app = app_with_one_item_per_phase();
    // The review POST lands (the line comment publishes); the file comment
    // fails, so it and the reply remain.
    submit_with(
        &mut app,
        &PartialSubmitter {
            review_ok: true,
            file_comments_ok: false,
            replies_ok: true,
        },
    );
    assert!(app.submit_result.is_some());

    press(&mut app, KeyCode::Char('U'));
    assert_eq!(app.mode, Mode::SubmitForge);
    assert!(app.submit_result.is_none(), "the result view is dismissed");

    // The rebuilt batch carries the remainder only, and skips the review POST
    // that already landed.
    let batch = app.build_submit_batch(Verdict::Comment, Some("overall"));
    assert!(
        !batch.include_review_post,
        "the verdict already landed; a retry must not re-deliver it"
    );
    assert!(
        batch.plan.comment_annotation_ids.is_empty(),
        "the published line comment is not re-sent"
    );
    assert_eq!(batch.plan.file_comment_follow_ups.len(), 1);
    assert_eq!(batch.replies.len(), 1);
}

#[test]
fn every_result_table_entry_drives_its_documented_action() {
    use super::super::modal_keys::{SUBMIT_RESULT_KEYS, SubmitResultAction};

    for binding in SUBMIT_RESULT_KEYS.iter() {
        for key in &binding.keys {
            let label = binding.key_label();
            let mut app = app_with_a_long_result(40);
            // A render fixes the viewport the page keys move by, and the
            // clamp the scroll assertions below are read against.
            let _ = render_modal(&app, 80, 16);
            let viewport = app.submit_result.as_ref().unwrap().viewport.get();

            match binding.action {
                SubmitResultAction::ScrollDown => {
                    handle_submit_result_key(&mut app, key.event());
                    assert_eq!(
                        app.submit_result.as_ref().unwrap().scroll.get(),
                        1,
                        "Submit result {label}: scroll-down advances one line"
                    );
                }
                SubmitResultAction::ScrollUp => {
                    app.submit_result.as_ref().unwrap().scroll.set(3);
                    handle_submit_result_key(&mut app, key.event());
                    assert_eq!(
                        app.submit_result.as_ref().unwrap().scroll.get(),
                        2,
                        "Submit result {label}: scroll-up retreats one line"
                    );
                }
                SubmitResultAction::PageDown => {
                    handle_submit_result_key(&mut app, key.event());
                    assert_eq!(
                        app.submit_result.as_ref().unwrap().scroll.get(),
                        viewport,
                        "Submit result {label}: page-down moves a full viewport"
                    );
                }
                SubmitResultAction::PageUp => {
                    app.submit_result.as_ref().unwrap().scroll.set(viewport + 1);
                    handle_submit_result_key(&mut app, key.event());
                    assert_eq!(
                        app.submit_result.as_ref().unwrap().scroll.get(),
                        1,
                        "Submit result {label}: page-up moves a full viewport"
                    );
                }
                SubmitResultAction::Dismiss => {
                    handle_submit_result_key(&mut app, key.event());
                    assert!(
                        app.submit_result.is_none(),
                        "Submit result {label}: dismiss closes the modal"
                    );
                    assert_eq!(app.mode, Mode::Normal, "Submit result {label}");
                }
                SubmitResultAction::Retry => {
                    handle_submit_result_key(&mut app, key.event());
                    assert_eq!(
                        app.mode,
                        Mode::SubmitForge,
                        "Submit result {label}: retry reopens the submit modal"
                    );
                    assert!(app.submit_result.is_none(), "Submit result {label}");
                }
            }
        }
    }
}

#[test]
fn a_report_with_no_recorded_attempt_says_so_instead_of_showing_nothing() {
    // The panicked-submit-task path: a failure with no attempt recorded. Three
    // empty groups would render as a blank box that reads like "nothing
    // happened", which is the one thing the run cannot promise.
    let mut app = review_app(&["src/a.rs"]);
    app.apply_submit_outcome(SubmitReport {
        failure: Some("submit task panicked".to_string()),
        ..SubmitReport::default()
    });

    assert!(app.submit_result.is_some());
    let rendered = render_modal(&app, 80, 16);
    assert!(
        rendered.contains("no per-item outcome"),
        "an itemless report must say so: {rendered}"
    );
    assert!(rendered.contains("panicked"), "{rendered}");
}

// -- scrolling ---------------------------------------------------------------

#[test]
fn the_result_list_scrolls_and_the_offset_clamps_to_the_content() {
    let mut app = app_with_a_long_result(40);
    // A render establishes the viewport and clamps whatever the keys asked for.
    let top = render_modal(&app, 80, 16);
    assert!(top.contains("more line"), "the overflow is marked: {top}");
    assert_eq!(app.submit_result.as_ref().unwrap().scroll.get(), 0);

    press(&mut app, KeyCode::Char('j'));
    press(&mut app, KeyCode::Char('j'));
    let _ = render_modal(&app, 80, 16);
    assert_eq!(app.submit_result.as_ref().unwrap().scroll.get(), 2);

    press(&mut app, KeyCode::Char('k'));
    let _ = render_modal(&app, 80, 16);
    assert_eq!(app.submit_result.as_ref().unwrap().scroll.get(), 1);

    // Paging far past the end lands on the last page rather than off it.
    for _ in 0..20 {
        press(&mut app, KeyCode::PageDown);
    }
    let bottom = render_modal(&app, 80, 16);
    let offset = app.submit_result.as_ref().unwrap().scroll.get();
    assert!(offset > 0, "a long list scrolls");
    assert!(
        !bottom.contains("to scroll"),
        "the bottom of the list is reached, so nothing is marked below: {bottom}"
    );
    // A second render at the clamped offset must not move it again.
    let _ = render_modal(&app, 80, 16);
    assert_eq!(app.submit_result.as_ref().unwrap().scroll.get(), offset);
}

#[test]
fn the_diagnostic_headline_is_rendered_below_the_groups() {
    let app = app_with_a_long_result(1);
    let rendered = render_modal(&app, 80, 16);
    assert!(rendered.contains("not sent"), "{rendered}");
    assert!(rendered.contains("HTTP 403"), "{rendered}");
}
