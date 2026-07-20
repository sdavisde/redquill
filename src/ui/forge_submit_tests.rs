use crate::annotate::{Classification, Side, Target};
use crate::diff::FileDiff;
use crate::forge::{SubmitReport, Verdict};
use crate::git::{DiffTarget, RawFilePatch};
use crate::review::store::{ForgeMetadata, ForgeProviderKind};

use super::super::app::{App, Mode};
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

#[test]
fn opening_the_modal_on_gitlab_sets_the_disclosure_and_two_verdicts() {
    let mut app = gitlab_review_app(&["src/a.rs"]);
    app.open_submit_forge();
    let state = app.submit_forge.as_ref().expect("modal opened");
    assert_eq!(state.verdicts, vec![Verdict::Comment, Verdict::Approve]);
    assert!(
        state.disclosure.is_some(),
        "GitLab modal must disclose its submit shape"
    );
}

#[test]
fn opening_the_modal_on_github_sets_no_disclosure() {
    let mut app = github_review_app(&["src/a.rs"]);
    app.open_submit_forge();
    let state = app.submit_forge.as_ref().expect("modal opened");
    assert!(state.disclosure.is_none());
}

// -- grouped preview + labels (FR-17) ----------------------------------------

#[test]
fn preview_groups_annotations_by_file_and_labels_local_only_and_file_comments() {
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

    let preview = build_preview(
        app.annotations.unpublished(),
        app.replies
            .unpublished()
            .map(|r| (r.thread_id, r.body.as_str())),
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
}

#[test]
fn preview_lists_draft_replies_separately_from_file_groups() {
    let mut app = github_review_app(&["src/a.rs"]);
    app.replies.add(100, "agreed").unwrap();
    app.replies.add(200, "why?").unwrap();

    let preview = build_preview(
        app.annotations.unpublished(),
        app.replies
            .unpublished()
            .map(|r| (r.thread_id, r.body.as_str())),
    );
    assert!(preview.groups.is_empty());
    assert_eq!(preview.replies.len(), 2);
    assert_eq!(preview.replies[0].thread_id, 100);
    assert_eq!(preview.replies[0].summary, "agreed");
}

// -- open / not-a-forge-session no-op ----------------------------------------

#[test]
fn open_submit_forge_opens_the_modal_in_a_github_pr_session() {
    let mut app = github_review_app(&["src/a.rs"]);
    app.open_submit_forge();
    assert_eq!(app.mode, Mode::SubmitForge);
    let state = app.submit_forge.as_ref().unwrap();
    assert_eq!(
        state.verdicts,
        vec![Verdict::Comment, Verdict::Approve, Verdict::RequestChanges]
    );
    // Target line names the PR, host, and slug (slug falls back to host with
    // no backend attached here).
    assert!(state.target_line.starts_with("#25 on github.com/"));
}

#[test]
fn open_submit_forge_is_a_no_op_hint_outside_a_forge_session() {
    let mut app = App::new(vec![file("src/a.rs")]);
    // A plain diff, no review_forge.
    app.open_submit_forge();
    assert_eq!(app.mode, Mode::Normal, "no modal opens outside a PR review");
    assert!(app.submit_forge.is_none());
    assert!(
        app.status_message
            .as_deref()
            .is_some_and(|m| m.contains("not a PR review"))
    );
}

// -- verdict cycling + summary editing ---------------------------------------

#[test]
fn verdict_cycles_forward_and_backward_within_the_supported_set() {
    let mut app = github_review_app(&["src/a.rs"]);
    app.open_submit_forge();
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

#[test]
fn summary_typing_and_backspace_edit_the_field() {
    let mut app = github_review_app(&["src/a.rs"]);
    app.open_submit_forge();
    app.submit_forge_insert_char('h');
    app.submit_forge_insert_char('i');
    assert_eq!(app.submit_forge.as_ref().unwrap().summary, "hi");
    app.submit_forge_delete_char();
    assert_eq!(app.submit_forge.as_ref().unwrap().summary, "h");
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
    // Typing clears the hint.
    app.submit_forge_insert_char('x');
    assert!(app.submit_forge.as_ref().unwrap().hint.is_none());
    // Now the confirm proceeds (closes the modal; no live backend so nothing
    // is actually sent).
    app.submit_forge_confirm();
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.submit_forge.is_none());
}

#[test]
fn cycling_the_verdict_clears_a_prior_hint() {
    let mut app = github_review_app(&["src/a.rs"]);
    app.open_submit_forge();
    app.submit_forge_verdict_next();
    app.submit_forge_verdict_next();
    app.submit_forge_confirm();
    assert!(app.submit_forge.as_ref().unwrap().hint.is_some());
    app.submit_forge_verdict_prev();
    assert!(app.submit_forge.as_ref().unwrap().hint.is_none());
}

// -- confirm on the fake path sends nothing (no live backend) ----------------

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
