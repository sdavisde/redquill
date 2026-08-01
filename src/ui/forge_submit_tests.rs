use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use crate::annotate::{Classification, Side, Target};
use crate::diff::FileDiff;
use crate::forge::{
    SubmitAttempt, SubmitReport, Thread, ThreadAnchor, ThreadComment, ThreadOverlayStore, Verdict,
};
use crate::git::{DiffTarget, RawFilePatch};
use crate::review::store::{ForgeMetadata, ForgeProviderKind};

use super::super::app::{App, Mode};
use super::super::modes::handle_submit_forge_key;
use super::*;

// -- fixtures ----------------------------------------------------------------

fn file(path: &str) -> FileDiff {
    let raw = format!(
        "diff --git a/{path} b/{path}\nindex 1..2 100644\n--- a/{path}\n+++ b/{path}\n@@ -1,2 +1,2 @@\n fn f() {{\n-    old();\n+    new();\n"
    );
    FileDiff::from_patch(&RawFilePatch {
        path: path.to_string(),
        old_path: None,
        raw,
        is_binary: false,
    })
    .unwrap()
}

/// A GitHub PR review session with the given files. `review_forge` set so the
/// submit action is live.
fn github_review_app(paths: &[&str]) -> App {
    let mut app = App::new(paths.iter().map(|p| file(p)).collect());
    app.target = DiffTarget::Review {
        base: "main".to_string(),
        branch: "redquill/pr/25".to_string(),
    };
    app.review_forge = Some(ForgeMetadata {
        provider: ForgeProviderKind::GitHub,
        host: "github.com".to_string(),
        number: 25,
        title: String::new(),
        last_head_sha: "deadbeef".to_string(),
        diff_refs: None,
    });
    app
}

/// The open modal's summary as one string.
fn summary_text(app: &App) -> String {
    app.submit_forge
        .as_ref()
        .expect("modal open")
        .summary
        .text()
}

/// Types `text` into the open submit modal through its real keymap: `Ctrl-j`
/// for each newline, a plain char for everything else.
fn type_into_summary(app: &mut App, text: &str) {
    for c in text.chars() {
        let key = if c == '\n' {
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL)
        } else {
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
        };
        handle_submit_forge_key(app, key);
    }
}

/// A GitHub review with its submit modal open on `text` as the summary.
fn app_with_summary(text: &str) -> App {
    let mut app = github_review_app(&["src/a.rs"]);
    app.open_submit_forge();
    type_into_summary(&mut app, text);
    app
}

// -- capability-driven verdict picker (FR-17) --------------------------------

#[test]
fn github_offers_all_three_verdicts() {
    let caps = capabilities_for(ForgeProviderKind::GitHub);
    assert_eq!(
        verdicts_for(caps),
        vec![Verdict::Comment, Verdict::Approve, Verdict::RequestChanges]
    );
}

#[test]
fn gitlab_offers_comment_and_approve_only() {
    let caps = capabilities_for(ForgeProviderKind::GitLab);
    assert_eq!(verdicts_for(caps), vec![Verdict::Comment, Verdict::Approve]);
}

/// A GitLab MR review session, mirroring [`github_review_app`].
fn gitlab_review_app(paths: &[&str]) -> App {
    let mut app = App::new(paths.iter().map(|p| file(p)).collect());
    app.target = DiffTarget::Review {
        base: "main".to_string(),
        branch: "redquill/pr/7".to_string(),
    };
    app.review_forge = Some(ForgeMetadata {
        provider: ForgeProviderKind::GitLab,
        host: "gitlab.com".to_string(),
        number: 7,
        title: String::new(),
        last_head_sha: "deadbeef".to_string(),
        diff_refs: None,
    });
    app
}

#[test]
fn gitlab_discloses_the_draft_submit_shape_without_naming_a_version() {
    // The disclosure is capability-driven (near_atomic_submit) and, per the
    // Open Question 4 copy decision, names no version number.
    let github = submit_disclosure(capabilities_for(ForgeProviderKind::GitHub));
    assert!(
        github.is_none(),
        "GitHub's single visible POST needs no caveat"
    );

    let gitlab = submit_disclosure(capabilities_for(ForgeProviderKind::GitLab))
        .expect("GitLab discloses its draft/visible split");
    assert!(gitlab.to_lowercase().contains("draft"));
    assert!(
        !gitlab.chars().any(|c| c.is_ascii_digit()),
        "the disclosure must name no version number: {gitlab}"
    );
}

// -- grouped preview + labels (FR-17) ----------------------------------------

#[test]
fn preview_groups_annotations_by_file_and_lists_replies_separately() {
    let mut app = github_review_app(&["src/a.rs"]);
    // Two annotations in a.rs, one whole-file comment in a.rs, one
    // worktree-anchored (local-only) note in b.rs.
    app.annotations
        .add(
            Target::line("src/a.rs", 2, Side::New),
            Classification::Issue,
            "fix line",
        )
        .unwrap();
    app.annotations
        .add(
            Target::file("src/a.rs"),
            Classification::Praise,
            "nice file",
        )
        .unwrap();
    app.annotations
        .add(
            Target::worktree_line("src/b.rs", 3),
            Classification::Nit,
            "local note",
        )
        .unwrap();
    app.replies.add(100, "agreed").unwrap();
    app.replies.add(200, "why?").unwrap();

    let preview = build_preview(
        app.annotations.unpublished(),
        app.replies
            .unpublished()
            .map(|r| (r.thread_id, r.body.as_str())),
        &ThreadOverlayStore::new(),
    );

    // a.rs group has the line comment then the file comment; b.rs has the
    // local-only note.
    assert_eq!(preview.groups.len(), 2);
    let a = &preview.groups[0];
    assert_eq!(a.path, "src/a.rs");
    assert_eq!(a.items.len(), 2);
    assert_eq!(a.items[0].note, PreviewNote::LineComment);
    assert_eq!(a.items[0].note.label(), None);
    assert_eq!(a.items[1].note, PreviewNote::FileComment);
    assert_eq!(
        a.items[1].note.label(),
        Some("posts as a separate file comment")
    );
    let b = &preview.groups[1];
    assert_eq!(b.path, "src/b.rs");
    assert_eq!(b.items[0].note, PreviewNote::LocalOnly);
    assert_eq!(
        b.items[0].note.label(),
        Some("local-only \u{2014} will not publish")
    );

    // Draft replies are listed apart from the per-file groups.
    assert_eq!(preview.replies.len(), 2);
    assert_eq!(preview.replies[0].thread_id, 100);
    assert_eq!(preview.replies[0].summary, "agreed");
}

/// A positioned thread rooted by `author`, for reply-preview target
/// resolution (mirrors `forge_threads_tests::positioned_thread`).
fn positioned_thread(id: u64, author: &str, path: &str, line: u32) -> Thread {
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
            body: "root comment".to_string(),
        },
        replies: Vec::new(),
        resolved: false,
        outdated: false,
        discussion_id: None,
    }
}

#[test]
fn reply_preview_names_the_root_author_and_anchor_when_the_thread_is_known() {
    let mut app = github_review_app(&["src/a.rs"]);
    app.replies.add(100, "agreed").unwrap();
    let mut threads = ThreadOverlayStore::new();
    threads.replace(vec![positioned_thread(100, "alice", "src/a.rs", 12)]);

    let preview = build_preview(
        app.annotations.unpublished(),
        app.replies
            .unpublished()
            .map(|r| (r.thread_id, r.body.as_str())),
        &threads,
    );

    let target = preview.replies[0]
        .target
        .as_ref()
        .expect("the thread is present in the overlay");
    assert_eq!(target.author, "alice");
    assert_eq!(target.anchor, "src/a.rs:12");
}

#[test]
fn reply_preview_falls_back_to_the_thread_id_when_the_thread_is_missing() {
    // Simulates a failed refresh dropping the thread from the overlay: the
    // reply is still drafted, but its author/anchor are no longer known.
    let mut app = github_review_app(&["src/a.rs"]);
    app.replies.add(100, "agreed").unwrap();
    let threads = ThreadOverlayStore::new();

    let preview = build_preview(
        app.annotations.unpublished(),
        app.replies
            .unpublished()
            .map(|r| (r.thread_id, r.body.as_str())),
        &threads,
    );

    assert!(
        preview.replies[0].target.is_none(),
        "no target once the thread is gone from the overlay"
    );
}

// -- open / not-a-forge-session no-op ----------------------------------------

/// Off a PR, `U` routes to the clipboard copy instead of the modal — there
/// is no verdict to pick and no forge to send to. Driven with an empty
/// annotation set on purpose: that path reports and returns *before* touching
/// the clipboard, so this proves the routing without `cargo test` writing to
/// the developer's real clipboard. What the copy actually hands over is
/// covered in `annotation_export`, against an injected fake.
#[test]
fn open_submit_forge_copies_to_the_clipboard_outside_a_forge_session() {
    let mut app = App::new(vec![file("src/a.rs")]);
    // A plain diff, no review_forge.
    app.open_submit_forge();
    assert_eq!(app.mode, Mode::Normal, "no modal opens outside a PR review");
    assert!(app.submit_forge.is_none());
    assert!(
        app.status_message
            .as_deref()
            .is_some_and(|m| m.contains("copy")),
        "the copy path must report itself, got {:?}",
        app.status_message
    );
}

// -- verdict cycling + summary editing ---------------------------------------

#[test]
fn verdict_cycles_forward_and_backward_within_the_supported_set() {
    let mut app = github_review_app(&["src/a.rs"]);
    app.open_submit_forge();
    assert_eq!(app.mode, Mode::SubmitForge);
    // The target line names the PR, host, and slug (the slug falls back to the
    // host with no backend attached here).
    assert!(
        app.submit_forge
            .as_ref()
            .unwrap()
            .target_line
            .starts_with("#25 on github.com/")
    );
    assert_eq!(
        app.submit_forge.as_ref().unwrap().verdict(),
        Verdict::Comment
    );
    app.submit_forge_verdict_next();
    assert_eq!(
        app.submit_forge.as_ref().unwrap().verdict(),
        Verdict::Approve
    );
    app.submit_forge_verdict_prev();
    assert_eq!(
        app.submit_forge.as_ref().unwrap().verdict(),
        Verdict::Comment
    );
    // Wrapping backward from the first lands on the last.
    app.submit_forge_verdict_prev();
    assert_eq!(
        app.submit_forge.as_ref().unwrap().verdict(),
        Verdict::RequestChanges
    );
}

// -- cancel sends nothing -----------------------------------------------------

#[test]
fn cancel_closes_the_modal_and_publishes_nothing() {
    let mut app = github_review_app(&["src/a.rs"]);
    app.annotations
        .add(
            Target::line("src/a.rs", 2, Side::New),
            Classification::Issue,
            "fix",
        )
        .unwrap();
    app.open_submit_forge();
    app.close_submit_forge();
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.submit_forge.is_none());
    // The annotation is still unpublished — nothing was sent.
    assert_eq!(app.annotations.unpublished().count(), 1);
    assert!(app.forge_submit_in_flight.is_none());
}

// -- build_submit_batch: resume excludes published, gates the review post ----

#[test]
fn build_submit_batch_includes_review_post_on_a_fresh_submit() {
    let mut app = github_review_app(&["src/a.rs"]);
    app.annotations
        .add(
            Target::line("src/a.rs", 2, Side::New),
            Classification::Issue,
            "fix",
        )
        .unwrap();
    let batch = app.build_submit_batch(Verdict::Comment, Some("looks good"));
    assert!(batch.include_review_post);
    assert_eq!(batch.plan.payload.comments.len(), 1);
    assert_eq!(batch.plan.payload.body, "looks good");
}

#[test]
fn build_submit_batch_skips_the_review_post_and_published_items_on_resume() {
    let mut app = github_review_app(&["src/a.rs"]);
    let id = app
        .annotations
        .add(
            Target::line("src/a.rs", 2, Side::New),
            Classification::Issue,
            "fix",
        )
        .unwrap();
    let file_id = app
        .annotations
        .add(Target::file("src/a.rs"), Classification::Praise, "nice")
        .unwrap();
    // Simulate a prior successful review POST (line comment published, review
    // delivered) with the file comment still pending.
    app.annotations.set_published(id, true).unwrap();
    app.forge_review_submitted = true;

    let batch = app.build_submit_batch(Verdict::Comment, None);
    assert!(
        !batch.include_review_post,
        "a resume must not re-post the review"
    );
    assert!(
        batch.plan.payload.comments.is_empty(),
        "the already-published line comment must not be re-sent"
    );
    assert_eq!(
        batch.plan.file_comment_follow_ups.len(),
        1,
        "the still-unpublished file comment remains"
    );
    assert_eq!(batch.plan.file_comment_follow_ups[0].annotation_id, file_id);
}

// -- apply_submit_outcome: per-item marking + split reporting ----------------

#[test]
fn apply_outcome_marks_published_items_and_reports_a_clean_success() {
    let mut app = github_review_app(&["src/a.rs"]);
    let a0 = app
        .annotations
        .add(
            Target::line("src/a.rs", 2, Side::New),
            Classification::Issue,
            "fix",
        )
        .unwrap();
    let r0 = app.replies.add(100, "agreed").unwrap();

    app.apply_submit_outcome(SubmitReport {
        published_annotation_ids: vec![a0],
        published_reply_ids: vec![r0],
        review_submitted: true,
        failure: None,
        draft_annotation_ids: vec![],
        draft_reply_ids: vec![],
        attempt: SubmitAttempt::default(),
        summary_draft_created: false,
    });

    assert!(app.annotations.unpublished().next().is_none());
    assert!(app.replies.unpublished().next().is_none());
    assert!(app.forge_review_submitted);
    assert!(
        app.status_message
            .as_deref()
            .is_some_and(|m| m.contains("review submitted"))
    );
}

#[test]
fn apply_outcome_on_mid_failure_reports_the_published_unpublished_split() {
    let mut app = github_review_app(&["src/a.rs"]);
    let a0 = app
        .annotations
        .add(
            Target::line("src/a.rs", 2, Side::New),
            Classification::Issue,
            "fix",
        )
        .unwrap();
    // A second, file-target annotation that did NOT publish.
    app.annotations
        .add(Target::file("src/a.rs"), Classification::Praise, "nice")
        .unwrap();

    app.apply_submit_outcome(SubmitReport {
        published_annotation_ids: vec![a0],
        published_reply_ids: vec![],
        review_submitted: true,
        failure: Some("file boom".to_string()),
        draft_annotation_ids: vec![],
        draft_reply_ids: vec![],
        attempt: SubmitAttempt::default(),
        summary_draft_created: false,
    });

    // One published, one still unpublished; the flag is set so a resume skips
    // the review POST.
    assert_eq!(app.annotations.unpublished().count(), 1);
    assert!(app.forge_review_submitted);
    let msg = app.status_message.as_deref().unwrap();
    assert!(msg.contains("1 published"), "status: {msg}");
    assert!(msg.contains("1 unpublished"), "status: {msg}");
    assert!(msg.contains("file boom"), "status: {msg}");
}

#[test]
fn apply_outcome_with_pending_drafts_reports_them_instead_of_calling_them_failed() {
    let mut app = gitlab_review_app(&["src/a.rs"]);
    let a0 = app
        .annotations
        .add(
            Target::line("src/a.rs", 2, Side::New),
            Classification::Issue,
            "fix",
        )
        .unwrap();
    let a1 = app
        .annotations
        .add(Target::file("src/a.rs"), Classification::Praise, "nice")
        .unwrap();
    let _ = a1;

    // a0's draft was created before the stop; a1 was never attempted.
    app.apply_submit_outcome(SubmitReport {
        published_annotation_ids: vec![],
        published_reply_ids: vec![],
        review_submitted: false,
        failure: Some("boom".to_string()),
        draft_annotation_ids: vec![a0],
        draft_reply_ids: vec![],
        attempt: SubmitAttempt::default(),
        summary_draft_created: false,
    });

    let msg = app.status_message.as_deref().unwrap();
    assert!(msg.contains("0 published"), "status: {msg}");
    assert!(msg.contains("1 pending draft"), "status: {msg}");
    assert!(msg.contains("submit again to publish"), "status: {msg}");
    assert!(msg.contains("1 not sent"), "status: {msg}");
    assert!(msg.contains("boom"), "status: {msg}");
}

#[test]
fn apply_outcome_records_pending_drafts_and_the_resubmit_batch_skips_them() {
    let mut app = gitlab_review_app(&["src/a.rs"]);
    let a0 = app
        .annotations
        .add(
            Target::line("src/a.rs", 2, Side::New),
            Classification::Issue,
            "fix",
        )
        .unwrap();
    let a1 = app
        .annotations
        .add(Target::file("src/a.rs"), Classification::Praise, "nice")
        .unwrap();
    let r0 = app.replies.add(100, "agreed").unwrap();

    // A stopped GitLab run created drafts for a0, the summary, and r0, but
    // published nothing.
    app.apply_submit_outcome(SubmitReport {
        published_annotation_ids: vec![],
        published_reply_ids: vec![],
        review_submitted: false,
        failure: Some("boom".to_string()),
        draft_annotation_ids: vec![a0],
        draft_reply_ids: vec![r0],
        attempt: SubmitAttempt::default(),
        summary_draft_created: true,
    });

    assert!(
        app.annotations
            .iter()
            .find(|a| a.id == a0)
            .unwrap()
            .draft_created
    );
    assert!(
        !app.annotations
            .iter()
            .find(|a| a.id == a1)
            .unwrap()
            .draft_created
    );
    assert!(app.replies.get(r0).unwrap().draft_created);
    assert!(app.forge_summary_draft_created);
    assert!(!app.forge_review_submitted);

    // The next batch still contains every unpublished item but flags the
    // existing drafts so the sequence creates only what's missing.
    let batch = app.build_submit_batch(Verdict::Comment, Some("overall"));
    assert_eq!(batch.draft_created_annotation_ids, vec![a0]);
    assert_eq!(batch.draft_created_reply_ids, vec![r0]);
    assert!(batch.summary_draft_created);
    assert_eq!(batch.plan.comment_annotation_ids, vec![a0]);
    assert_eq!(batch.plan.file_comment_follow_ups.len(), 1);
}

#[test]
fn apply_outcome_publishing_clears_draft_state() {
    let mut app = gitlab_review_app(&["src/a.rs"]);
    let a0 = app
        .annotations
        .add(
            Target::line("src/a.rs", 2, Side::New),
            Classification::Issue,
            "fix",
        )
        .unwrap();
    let r0 = app.replies.add(100, "agreed").unwrap();
    let _ = app.annotations.set_draft_created(a0, true);
    app.replies.set_draft_created(r0, true);
    app.forge_summary_draft_created = true;

    app.apply_submit_outcome(SubmitReport {
        published_annotation_ids: vec![a0],
        published_reply_ids: vec![r0],
        review_submitted: true,
        failure: None,
        draft_annotation_ids: vec![],
        draft_reply_ids: vec![],
        attempt: SubmitAttempt::default(),
        summary_draft_created: false,
    });

    let a = app.annotations.iter().find(|a| a.id == a0).unwrap();
    assert!(a.published);
    assert!(!a.draft_created, "publishing consumes the pending draft");
    let r = app.replies.get(r0).unwrap();
    assert!(r.published);
    assert!(!r.draft_created);
    assert!(!app.forge_summary_draft_created);
}

// -- request-changes requires a summary (blocked confirm) --------------------

#[test]
fn confirm_request_changes_with_no_summary_is_blocked_with_a_hint() {
    let mut app = github_review_app(&["src/a.rs"]);
    app.open_submit_forge();
    // Select request-changes (Comment -> Approve -> RequestChanges).
    app.submit_forge_verdict_next();
    app.submit_forge_verdict_next();
    assert_eq!(
        app.submit_forge.as_ref().unwrap().verdict(),
        Verdict::RequestChanges
    );
    app.submit_forge_confirm();
    // Modal stays open, nothing spawned, and a hint is set.
    assert_eq!(app.mode, Mode::SubmitForge);
    let state = app.submit_forge.as_ref().expect("modal still open");
    assert!(
        state.hint.as_deref().is_some_and(|h| h.contains("summary")),
        "a request-changes-needs-summary hint must be shown"
    );
    assert!(app.forge_submit_in_flight.is_none());
}

#[test]
fn typing_a_summary_clears_the_hint_and_lets_request_changes_confirm() {
    let mut app = github_review_app(&["src/a.rs"]);
    app.open_submit_forge();
    app.submit_forge_verdict_next();
    app.submit_forge_verdict_next();
    app.submit_forge_confirm();
    assert!(app.submit_forge.as_ref().unwrap().hint.is_some());
    // Typing clears the hint, and backspace edits the same field back down.
    app.submit_forge_insert_char('x');
    app.submit_forge_insert_char('y');
    assert_eq!(summary_text(&app), "xy");
    handle_submit_forge_key(
        &mut app,
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
    );
    assert_eq!(summary_text(&app), "x");
    assert!(app.submit_forge.as_ref().unwrap().hint.is_none());
    // Now the confirm proceeds (closes the modal; no live backend so nothing
    // is actually sent).
    app.submit_forge_confirm();
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.submit_forge.is_none());
}

/// The accepted trade for making `[keys.submit-forge]` remappable: the table
/// is consulted before the char-insert fallback, so a control action bound to
/// a bare printable key takes that character away from summary typing. Pins
/// the ordering — flipping it would silently un-remap every letter-keyed
/// submit-forge override.
#[test]
fn a_submit_forge_action_remapped_onto_a_letter_shadows_summary_typing() {
    let mut keys = crate::config::KeysConfig::default();
    let mut table = std::collections::BTreeMap::new();
    table.insert(
        "cancel".to_string(),
        vec![crate::config::keys::KeySeqSpec::One(
            crate::config::keys::ChordSpec {
                code: KeyCode::Char('q'),
                mods: KeyModifiers::NONE,
            },
        )],
    );
    keys.modal.insert("submit-forge".to_string(), table);
    let (modal_keys, warnings) = crate::ui::modal_keys_config::effective_modal_keys(&keys);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");

    let mut app = github_review_app(&["src/a.rs"]);
    app.modal_keys = modal_keys;
    app.open_submit_forge();
    handle_submit_forge_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
    );
    assert_eq!(app.mode, Mode::Normal, "`q` must cancel, not type");
    assert!(app.submit_forge.is_none());
}

// -- confirm on the fake path sends nothing (no live backend) ----------------

// -- scrollable preview + overflow markers -----------------------------------

/// Renders the modal over a `width` x `height` terminal and returns its cell
/// symbols as one string (the `cleanup_reviews_modal` test idiom).
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

// -- reply preview target labels -----------------------------------

#[test]
fn a_reply_preview_renders_the_root_author_and_anchor_not_the_raw_id() {
    let mut app = github_review_app(&["src/a.rs"]);
    app.replies.add(100, "agreed").unwrap();
    app.thread_overlay
        .replace(vec![positioned_thread(100, "alice", "src/a.rs", 12)]);
    app.open_submit_forge();

    let rendered = render_modal(&app, 90, 24);
    assert!(
        rendered.contains("to alice @ src/a.rs:12"),
        "the reply must name the root author and anchor: {rendered}"
    );
    assert!(
        !rendered.contains("thread 100:"),
        "a resolved thread must not fall back to the raw id: {rendered}"
    );
}

#[test]
fn a_reply_preview_falls_back_to_the_thread_id_once_the_thread_drops_out_of_the_overlay() {
    // The overlay is empty — e.g. a failed refresh dropped the thread — so
    // there is no author/anchor left to show.
    let mut app = github_review_app(&["src/a.rs"]);
    app.replies.add(100, "agreed").unwrap();
    app.open_submit_forge();

    let rendered = render_modal(&app, 90, 24);
    assert!(
        rendered.contains("thread 100: agreed"),
        "with no thread in the overlay, the id form is the only honest label: {rendered}"
    );
}

/// A review with `n` annotations spread over three files — enough rows to
/// overflow any sensibly sized modal.
fn app_with_many_annotations(n: usize) -> App {
    let paths = ["src/a.rs", "src/b.rs", "src/c.rs"];
    let mut app = github_review_app(&paths);
    for i in 0..n {
        app.annotations
            .add(
                Target::line(paths[i % paths.len()], 2, Side::New),
                Classification::Issue,
                format!("comment number {i}"),
            )
            .unwrap();
    }
    app
}

#[test]
fn scroll_geometry_clamps_to_the_content_and_counts_what_is_hidden() {
    // Content that fits keeps the whole box, never offsets, and hides nothing
    // — even when a stale offset asks for the bottom.
    let fits = resolve_scroll(6, 20, u16::MAX);
    assert_eq!(fits.offset, 0);
    assert_eq!(fits.body_height, 20);
    assert_eq!((fits.hidden_above, fits.hidden_below), (0, 0));

    // 40 lines in a 10-row box: two rows go to the markers, so 8 show at once.
    let top = resolve_scroll(40, 10, 0);
    assert_eq!(top.body_height, 8);
    assert_eq!((top.offset, top.hidden_above, top.hidden_below), (0, 0, 32));

    // A mid-scroll offset splits the hidden lines between the two directions.
    let mid = resolve_scroll(40, 10, 12);
    assert_eq!(
        (mid.offset, mid.hidden_above, mid.hidden_below),
        (12, 12, 20)
    );

    // An overshoot lands on the last page, with nothing left below.
    let bottom = resolve_scroll(40, 10, u16::MAX);
    assert_eq!(
        (bottom.offset, bottom.hidden_above, bottom.hidden_below),
        (32, 32, 0)
    );

    // A box too short to spend rows on markers still scrolls, using them all.
    let tiny = resolve_scroll(40, 2, u16::MAX);
    assert_eq!(tiny.body_height, 2);
    assert_eq!(tiny.offset, 38);
}

#[test]
fn overflow_markers_are_shown_only_for_the_clipped_direction() {
    assert_eq!(below_marker(0), None, "nothing below, no marker");
    assert_eq!(above_marker(0), None, "nothing above, no marker");
    let below = below_marker(7).expect("clipped below is marked");
    assert!(
        below.contains('7') && below.contains("more lines"),
        "{below}"
    );
    assert!(
        below.contains("PgDn"),
        "the marker must name the scroll key: {below}"
    );
    let above = above_marker(1).expect("clipped above is marked");
    assert!(
        above.contains("1 more line above"),
        "a single hidden line reads singular: {above}"
    );
}

#[test]
fn a_tall_batch_renders_the_overflow_marker_and_scrolls() {
    let mut app = app_with_many_annotations(40);
    app.open_submit_forge();

    // Fresh open: clipped below, nothing above.
    let first = render_modal(&app, 90, 24);
    assert!(
        first.contains("more lines"),
        "a clipped batch must say how much is hidden: {first}"
    );
    assert!(
        !first.contains("above"),
        "nothing is hidden above at the top of the batch: {first}"
    );

    // Scrolling down moves the window: the clamped offset advances and the
    // top marker appears.
    for _ in 0..5 {
        app.submit_forge_scroll_down();
    }
    let scrolled = render_modal(&app, 90, 24);
    assert_eq!(app.submit_forge.as_ref().unwrap().scroll.get(), 5);
    assert!(
        scrolled.contains("more lines above"),
        "scrolled down, the lines above must be marked: {scrolled}"
    );

    // An overshoot is clamped to the last page by the render, and the bottom
    // marker goes away because nothing is left below.
    app.submit_forge.as_ref().unwrap().scroll.set(u16::MAX);
    let bottom = render_modal(&app, 90, 24);
    let offset = app.submit_forge.as_ref().unwrap().scroll.get();
    assert!(offset > 0 && offset < u16::MAX, "clamped offset: {offset}");
    assert!(
        !bottom.contains("to scroll"),
        "at the bottom there is nothing more to scroll to: {bottom}"
    );
    // Re-rendering at the clamped offset is stable (the clamp is idempotent).
    render_modal(&app, 90, 24);
    assert_eq!(app.submit_forge.as_ref().unwrap().scroll.get(), offset);
}

#[test]
fn a_short_batch_neither_scrolls_nor_marks_overflow() {
    let mut app = app_with_many_annotations(1);
    app.open_submit_forge();
    // A stale/overshooting offset must not scroll content that already fits.
    app.submit_forge.as_ref().unwrap().scroll.set(u16::MAX);
    let content = render_modal(&app, 90, 24);
    assert_eq!(app.submit_forge.as_ref().unwrap().scroll.get(), 0);
    assert!(
        !content.contains("more line"),
        "everything fits, so no overflow marker: {content}"
    );
    assert!(content.contains("comment number 0"));
}

#[test]
fn reopening_the_modal_starts_at_the_top() {
    let mut app = app_with_many_annotations(40);
    app.open_submit_forge();
    app.submit_forge_page_down();
    render_modal(&app, 90, 24);
    assert!(app.submit_forge.as_ref().unwrap().scroll.get() > 0);
    app.close_submit_forge();
    app.open_submit_forge();
    assert_eq!(
        app.submit_forge.as_ref().unwrap().scroll.get(),
        0,
        "a fresh open starts at the top of the batch"
    );
}

#[test]
fn scroll_keys_move_the_preview_while_printable_keys_still_type_the_summary() {
    let mut app = app_with_many_annotations(40);
    app.open_submit_forge();
    // Record a viewport so PageDown has a real page to move by.
    render_modal(&app, 90, 24);

    // Ctrl-Down scrolls and leaves the summary alone; a bare Down is the
    // summary cursor's, so it moves nothing here.
    handle_submit_forge_key(
        &mut app,
        KeyEvent::new(KeyCode::Down, KeyModifiers::CONTROL),
    );
    let state = app.submit_forge.as_ref().unwrap();
    assert_eq!(state.scroll.get(), 1);
    assert_eq!(
        summary_text(&app),
        "",
        "a scroll key must not type into the summary"
    );
    handle_submit_forge_key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(
        app.submit_forge.as_ref().unwrap().scroll.get(),
        1,
        "a bare arrow belongs to the summary cursor, not to the preview"
    );

    // `j`/`k` belong to the summary, not to the scroll — they must type.
    handle_submit_forge_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
    );
    handle_submit_forge_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
    );
    assert_eq!(summary_text(&app), "jk");
    assert_eq!(
        app.submit_forge.as_ref().unwrap().scroll.get(),
        1,
        "typing must not move the preview"
    );

    // PageDown pages by the recorded viewport, PageUp/Ctrl-Up step back.
    let page = app.submit_forge.as_ref().unwrap().viewport.get();
    assert!(page > 1, "the render must record a real viewport");
    handle_submit_forge_key(
        &mut app,
        KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
    );
    assert_eq!(app.submit_forge.as_ref().unwrap().scroll.get(), 1 + page);
    handle_submit_forge_key(&mut app, KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
    assert_eq!(app.submit_forge.as_ref().unwrap().scroll.get(), 1);
    handle_submit_forge_key(&mut app, KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL));
    assert_eq!(app.submit_forge.as_ref().unwrap().scroll.get(), 0);
}

/// The hint is pinned below the scrolling preview, so a batch too tall to fit
/// can't hide the reason a confirm did nothing.
#[test]
fn a_blocked_confirm_shows_its_hint_on_a_tall_batch() {
    let mut app = app_with_many_annotations(40);
    app.open_submit_forge();
    render_modal(&app, 90, 24);
    // Select request-changes and confirm with no summary: blocked, hinted.
    app.submit_forge_verdict_next();
    app.submit_forge_verdict_next();
    app.submit_forge_confirm();
    assert_eq!(app.mode, Mode::SubmitForge);

    let content = render_modal(&app, 90, 24);
    assert!(content.contains("needs a summary"), "hint not visible");
}

// -- the in-modal summary field ----------------------------------------------

/// The summary is edited in place with Compose's keymap: `Ctrl-j` opens a new
/// line, the motion and delete keys act on the buffer, and every line stays
/// visible — no second editor, and nothing lands in a store.
#[test]
fn the_summary_field_edits_a_multi_line_body_in_place() {
    let mut app = app_with_summary("one\ntwo\nthree");
    assert_eq!(app.mode, Mode::SubmitForge, "no editor is ever opened");
    assert!(app.compose.is_none());
    assert_eq!(summary_text(&app), "one\ntwo\nthree");

    // Compose's motion keys move this buffer's cursor: home, then a typed
    // char lands at the start of the last line.
    handle_submit_forge_key(&mut app, KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
    handle_submit_forge_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('*'), KeyModifiers::NONE),
    );
    assert_eq!(summary_text(&app), "one\ntwo\n*three");

    // Up moves off the last line, so a backspace there eats a character of
    // the middle line rather than the one just typed.
    handle_submit_forge_key(&mut app, KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    handle_submit_forge_key(&mut app, KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    handle_submit_forge_key(
        &mut app,
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
    );
    assert_eq!(summary_text(&app), "one\ntw\n*three");

    // A summary is neither an annotation nor a reply.
    assert_eq!(app.annotations.iter().count(), 0);
    assert!(app.replies.is_empty());
}

/// Every summary line is on screen, and the cursor sits in the field — the
/// affordance that says it's typeable at all.
#[test]
fn the_summary_field_shows_every_line_and_holds_the_cursor() {
    let app = app_with_summary("first\nsecond\nthird");
    let mut terminal = Terminal::new(TestBackend::new(90, 24)).unwrap();
    terminal
        .draw(|f| crate::ui::forge_submit::render(f, f.area(), &app))
        .unwrap();
    let content = terminal.backend().to_string();
    for line in ["first", "second", "third"] {
        assert!(content.contains(line), "{line} is not shown: {content}");
    }
    assert!(
        terminal.get_cursor_position().is_ok(),
        "the field must place a terminal cursor"
    );
}

#[test]
fn an_empty_summary_field_invites_typing() {
    let mut app = github_review_app(&["src/a.rs"]);
    app.open_submit_forge();
    let content = render_modal(&app, 90, 24);
    assert!(
        content.contains("Type your review summary here"),
        "an empty field must say it takes text: {content}"
    );
}

#[test]
fn request_changes_accepts_a_multi_line_summary() {
    let mut app = app_with_summary("needs work\n\n- fix the parser\n- add a test");
    app.submit_forge_verdict_next();
    app.submit_forge_verdict_next();
    assert_eq!(
        app.submit_forge.as_ref().unwrap().verdict(),
        Verdict::RequestChanges
    );

    app.submit_forge_confirm();
    assert_eq!(
        app.mode,
        Mode::Normal,
        "a multi-line body satisfies the needs-a-summary rule"
    );
    assert!(app.submit_forge.is_none(), "the confirm was not blocked");
}

/// A [`crate::forge::ForgeSubmitExecutor`] that records the review bodies it is
/// handed. A fake — no `gh`/`glab` is ever run.
#[derive(Default)]
struct RecordingSubmitter {
    bodies: std::cell::RefCell<Vec<String>>,
}

impl crate::forge::ForgeSubmitExecutor for RecordingSubmitter {
    fn submit_review(
        &self,
        payload: &crate::forge::ReviewPayload,
    ) -> Result<(), crate::forge::ForgeError> {
        self.bodies.borrow_mut().push(payload.body.clone());
        Ok(())
    }

    fn post_file_comment(&self, _path: &str, _body: &str) -> Result<(), crate::forge::ForgeError> {
        Ok(())
    }

    fn post_reply(&self, _thread_id: u64, _body: &str) -> Result<(), crate::forge::ForgeError> {
        Ok(())
    }
}

#[test]
fn the_outgoing_review_payload_carries_every_summary_line() {
    let summary = "needs work\n\n- fix the parser\n- add a test";
    let mut app = github_review_app(&["src/a.rs"]);
    app.annotations
        .add(
            Target::line("src/a.rs", 2, Side::New),
            Classification::Issue,
            "fix",
        )
        .unwrap();

    let batch = app.build_submit_batch(Verdict::RequestChanges, Some(summary));
    assert_eq!(
        batch.plan.payload.body, summary,
        "the summary crosses into the payload whole"
    );

    let fake = RecordingSubmitter::default();
    let report = crate::forge::run_submit_sequence(&batch, &fake);
    assert!(report.failure.is_none(), "{:?}", report.failure);
    assert_eq!(
        fake.bodies.borrow().as_slice(),
        &[summary.to_string()],
        "the submit sequence delivers every line, unsplit"
    );
}

#[test]
fn the_summary_lives_with_the_modal_and_a_fresh_open_starts_empty() {
    let mut app = app_with_summary("first\nsecond");
    app.close_submit_forge();
    app.open_submit_forge();
    assert_eq!(
        summary_text(&app),
        "",
        "the summary belongs to the modal that was cancelled, not to the session"
    );
}

#[test]
fn confirm_without_a_live_submitter_backend_sends_nothing() {
    let mut app = github_review_app(&["src/a.rs"]);
    app.annotations
        .add(
            Target::line("src/a.rs", 2, Side::New),
            Classification::Issue,
            "fix",
        )
        .unwrap();
    app.open_submit_forge();
    app.submit_forge_confirm();
    // No StageOps backend attached → no submitter → nothing spawned/published.
    assert!(app.forge_submit_in_flight.is_none());
    assert_eq!(app.annotations.unpublished().count(), 1);
    assert_eq!(app.mode, Mode::Normal);
}
