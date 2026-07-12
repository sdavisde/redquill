use super::*;
use crate::annotate::Classification;
use crate::git::RawFilePatch;
use crate::ui::compose::TextBuffer;
use crate::ui::rows::StagedMarker;
use crate::ui::stage_ops::{build_review, staged_from_status, staged_states_from_status};

fn file(path: &str, hunk_count: usize) -> FileDiff {
    let mut raw =
        format!("diff --git a/{path} b/{path}\nindex 1..2 100644\n--- a/{path}\n+++ b/{path}\n");
    for h in 0..hunk_count {
        let start = 1 + h * 10;
        raw.push_str(&format!("@@ -{start},1 +{start},1 @@\n-old{h}\n+new{h}\n"));
    }
    FileDiff::from_patch(&RawFilePatch {
        path: path.to_string(),
        old_path: None,
        raw,
        is_binary: false,
    })
    .unwrap()
}

fn file_with_raw(path: &str, raw: &str) -> FileDiff {
    FileDiff::from_patch(&RawFilePatch {
        path: path.to_string(),
        old_path: None,
        raw: raw.to_string(),
        is_binary: false,
    })
    .unwrap()
}

#[test]
fn cursor_down_clamps_at_last_row() {
    let mut app = App::new(vec![file("a.rs", 1)]);
    let last = app.view.rows.len() - 1;
    for _ in 0..20 {
        app.apply(Action::CursorDown);
    }
    assert_eq!(app.view.cursor, last);
}

#[test]
fn cursor_up_clamps_at_zero() {
    let mut app = App::new(vec![file("a.rs", 1)]);
    app.apply(Action::CursorUp);
    assert_eq!(app.view.cursor, 0);
}

#[test]
fn cursor_motion_on_empty_diff_stays_at_zero() {
    let mut app = App::new(vec![]);
    app.apply(Action::CursorDown);
    assert_eq!(app.view.cursor, 0);
    app.apply(Action::HalfPageDown);
    assert_eq!(app.view.cursor, 0);
}

#[test]
fn half_page_motion_uses_last_known_viewport_height() {
    // 5 hunks -> 1 + 5*3 = 16 rows, plenty of headroom for a
    // half-page-of-10 step in either direction.
    let mut app = App::new(vec![file("a.rs", 5)]);
    app.view.set_viewport_height(10);
    app.apply(Action::HalfPageDown);
    assert_eq!(app.view.cursor, 5);
    app.apply(Action::HalfPageUp);
    assert_eq!(app.view.cursor, 0);
}

#[test]
fn half_page_never_steps_by_zero_on_tiny_viewport() {
    let mut app = App::new(vec![file("a.rs", 1)]);
    app.view.set_viewport_height(1);
    app.apply(Action::HalfPageDown);
    assert_eq!(app.view.cursor, 1);
}

#[test]
fn ensure_visible_scrolls_down_to_follow_cursor() {
    let mut app = App::new(vec![file("a.rs", 3)]);
    app.view.set_viewport_height(3);
    for _ in 0..6 {
        app.apply(Action::CursorDown);
    }
    assert_eq!(app.view.cursor, 6);
    assert!(app.view.scroll <= app.view.cursor);
    assert!(app.view.cursor < app.view.scroll + 3);
}

#[test]
fn ensure_visible_scrolls_up_to_follow_cursor() {
    let mut app = App::new(vec![file("a.rs", 3)]);
    app.view.set_viewport_height(3);
    for _ in 0..6 {
        app.apply(Action::CursorDown);
    }
    for _ in 0..6 {
        app.apply(Action::CursorUp);
    }
    assert_eq!(app.view.cursor, 0);
    assert_eq!(app.view.scroll, 0);
}

#[test]
fn next_hunk_jumps_within_file() {
    let mut app = App::new(vec![file("a.rs", 2)]);
    app.apply(Action::NextHunk);
    let Row::HunkHeader { hunk_index, .. } = &app.view.rows[app.view.cursor] else {
        panic!("expected hunk header at cursor");
    };
    assert_eq!(*hunk_index, 0);

    app.apply(Action::NextHunk);
    let Row::HunkHeader { hunk_index, .. } = &app.view.rows[app.view.cursor] else {
        panic!("expected hunk header at cursor");
    };
    assert_eq!(*hunk_index, 1);
}

#[test]
fn next_hunk_crosses_file_boundary() {
    let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
    // Cursor starts on file a's FileHeader row (0), first (only) hunk
    // header is row 1.
    app.apply(Action::NextHunk); // -> a's only hunk header
    app.apply(Action::NextHunk); // -> should cross into b.rs
    assert_eq!(app.view.selected_file, 1);
    assert!(matches!(
        app.view.rows[app.view.cursor],
        Row::HunkHeader { .. }
    ));
}

#[test]
fn next_hunk_at_last_file_last_hunk_is_no_op() {
    let mut app = App::new(vec![file("a.rs", 1)]);
    app.apply(Action::NextHunk);
    let cursor_before = app.view.cursor;
    let file_before = app.view.selected_file;
    app.apply(Action::NextHunk);
    assert_eq!(app.view.cursor, cursor_before);
    assert_eq!(app.view.selected_file, file_before);
}

#[test]
fn prev_hunk_crosses_file_boundary_backwards() {
    let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
    app.apply(Action::NextFile); // move to b.rs, cursor reset to top (FileHeader)
    assert_eq!(app.view.selected_file, 1);
    app.apply(Action::PrevHunk); // no hunk header before cursor in b.rs -> cross back
    assert_eq!(app.view.selected_file, 0);
    assert!(matches!(
        app.view.rows[app.view.cursor],
        Row::HunkHeader { .. }
    ));
}

#[test]
fn prev_hunk_at_first_file_before_first_hunk_is_no_op() {
    let mut app = App::new(vec![file("a.rs", 1)]);
    let cursor_before = app.view.cursor;
    app.apply(Action::PrevHunk);
    assert_eq!(app.view.cursor, cursor_before);
    assert_eq!(app.view.selected_file, 0);
}

#[test]
fn next_file_jumps_to_next_section_header() {
    let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
    app.apply(Action::CursorDown);
    app.apply(Action::NextFile);
    assert_eq!(app.view.selected_file, 1);
    // Cursor lands on b.rs's section header, not row 0.
    assert_eq!(app.view.cursor, app.view.header_row_of_file[1]);
    assert!(matches!(
        app.view.rows[app.view.cursor],
        Row::FileHeader { file_index: 1, .. }
    ));
}

#[test]
fn next_file_clamps_at_last_file() {
    let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
    app.apply(Action::NextFile);
    app.apply(Action::NextFile);
    assert_eq!(app.view.selected_file, 1);
}

#[test]
fn prev_file_clamps_at_first_file() {
    let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
    app.apply(Action::PrevFile);
    assert_eq!(app.view.selected_file, 0);
}

#[test]
fn prev_file_jumps_back_across_sections() {
    let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
    app.apply(Action::NextFile); // -> b.rs header
    assert_eq!(app.view.selected_file, 1);
    // From b.rs's header, prev-section jumps to a.rs's header.
    app.apply(Action::PrevFile);
    assert_eq!(app.view.selected_file, 0);
    assert_eq!(app.view.cursor, app.view.header_row_of_file[0]);
}

#[test]
fn toggle_collapse_collapses_and_expands_file_under_cursor() {
    let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
    let expanded_len = app.view.rows.len();
    // Cursor starts on a.rs's header.
    app.apply(Action::ToggleCollapse);
    assert!(app.view.is_collapsed("a.rs"));
    assert!(app.view.rows.len() < expanded_len);
    // a.rs now contributes exactly its (collapsed) header row.
    assert!(matches!(
        app.view.rows[app.view.header_row_of_file[0]],
        Row::FileHeader {
            collapsed: true,
            file_index: 0,
            ..
        }
    ));
    // Cursor stays on a.rs's header, still addressable.
    assert_eq!(app.view.cursor, app.view.header_row_of_file[0]);
    assert!(app.view.rows[app.view.cursor].is_addressable());

    app.apply(Action::ToggleCollapse);
    assert!(!app.view.is_collapsed("a.rs"));
    assert_eq!(app.view.rows.len(), expanded_len);
}

#[test]
fn toggle_collapse_targets_the_cursor_file_not_the_first() {
    let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
    app.apply(Action::NextFile); // cursor onto b.rs's header
    app.apply(Action::ToggleCollapse);
    assert!(app.view.is_collapsed("b.rs"));
    assert!(!app.view.is_collapsed("a.rs"));
}

#[test]
fn toggle_help_flips_state() {
    let mut app = App::new(vec![file("a.rs", 1)]);
    assert!(!app.help_open);
    app.apply(Action::ToggleHelp);
    assert!(app.help_open);
    app.apply(Action::ToggleHelp);
    assert!(!app.help_open);
}

#[test]
fn quit_actions_are_no_ops_on_state() {
    let mut app = App::new(vec![file("a.rs", 1)]);
    app.apply(Action::CursorDown);
    let cursor = app.view.cursor;
    app.apply(Action::Quit);
    app.apply(Action::QuitDiscard);
    assert_eq!(app.view.cursor, cursor);
}

// -- Visual mode ------------------------------------------------------

#[test]
fn enter_visual_on_line_row_sets_anchor() {
    let mut app = App::new(vec![file("a.rs", 1)]);
    app.apply(Action::CursorDown); // hunk header
    app.apply(Action::CursorDown); // onto a line row
    let cursor = app.view.cursor;
    assert!(matches!(app.view.rows[cursor], Row::Line(_)));
    app.apply(Action::EnterVisual);
    assert_eq!(app.mode, Mode::Visual { anchor: cursor });
}

#[test]
fn enter_visual_on_header_row_is_a_no_op() {
    let mut app = App::new(vec![file("a.rs", 1)]);
    assert!(matches!(app.view.rows[0], Row::FileHeader { .. }));
    app.apply(Action::EnterVisual);
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn v_again_cancels_visual_mode() {
    let mut app = App::new(vec![file("a.rs", 1)]);
    app.apply(Action::CursorDown); // hunk header
    app.apply(Action::CursorDown); // line row
    app.apply(Action::EnterVisual);
    assert!(matches!(app.mode, Mode::Visual { .. }));
    app.apply(Action::EnterVisual);
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn visual_mode_disables_hunk_and_file_navigation() {
    let mut app = App::new(vec![file("a.rs", 2)]);
    app.apply(Action::CursorDown); // hunk header
    app.apply(Action::CursorDown); // line row
    app.apply(Action::EnterVisual);
    let cursor_before = app.view.cursor;
    app.apply(Action::NextHunk);
    app.apply(Action::NextFile);
    app.apply(Action::HalfPageDown);
    assert_eq!(app.view.cursor, cursor_before);
    assert!(matches!(app.mode, Mode::Visual { .. }));
}

#[test]
fn visual_mode_j_k_extend_selection() {
    let mut app = App::new(vec![file("a.rs", 1)]);
    app.apply(Action::CursorDown); // hunk header
    app.apply(Action::CursorDown); // first line row
    let anchor = app.view.cursor;
    app.apply(Action::EnterVisual);
    app.apply(Action::CursorDown);
    assert_eq!(app.mode, Mode::Visual { anchor });
    assert!(app.view.cursor > anchor);
}

// -- Target derivation --------------------------------------------------

#[test]
fn target_for_cursor_on_removed_line_uses_old_side() {
    let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,2 +1,1 @@
-removed
 kept
";
    let mut app = App::new(vec![file_with_raw("f.rs", raw)]);
    app.apply(Action::CursorDown); // hunk header -> row 1
    app.apply(Action::CursorDown); // removed line -> row 2
    let target = app.target_for_cursor().unwrap();
    assert_eq!(target, Target::line("f.rs", 1, Side::Old));
}

#[test]
fn target_for_cursor_on_added_line_uses_new_side() {
    let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,1 +1,2 @@
 kept
+added
";
    let mut app = App::new(vec![file_with_raw("f.rs", raw)]);
    app.apply(Action::CursorDown); // hunk header
    app.apply(Action::CursorDown); // context "kept" -> new side too
    app.apply(Action::CursorDown); // added line
    let target = app.target_for_cursor().unwrap();
    assert_eq!(target, Target::line("f.rs", 2, Side::New));
}

#[test]
fn target_for_cursor_on_context_line_uses_new_side() {
    let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,1 +1,1 @@
 kept
";
    let mut app = App::new(vec![file_with_raw("f.rs", raw)]);
    app.apply(Action::CursorDown); // hunk header
    app.apply(Action::CursorDown); // context line
    let target = app.target_for_cursor().unwrap();
    assert_eq!(target, Target::line("f.rs", 1, Side::New));
}

#[test]
fn target_for_cursor_on_hunk_header_spans_new_side() {
    let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,1 +1,3 @@
 a
+b
+c
";
    let mut app = App::new(vec![file_with_raw("f.rs", raw)]);
    app.apply(Action::CursorDown); // hunk header row
    let target = app.target_for_cursor().unwrap();
    assert_eq!(target, Target::hunk("f.rs", 1, 3).unwrap());
}

#[test]
fn target_for_cursor_on_hunk_header_falls_back_to_old_side_when_new_count_zero() {
    let raw = "\
diff --git a/gone.rs b/gone.rs
deleted file mode 100644
index 111..000
--- a/gone.rs
+++ /dev/null
@@ -1,3 +0,0 @@
-a
-b
-c
";
    let mut app = App::new(vec![file_with_raw("gone.rs", raw)]);
    app.apply(Action::CursorDown); // hunk header row
    let target = app.target_for_cursor().unwrap();
    assert_eq!(target, Target::hunk("gone.rs", 1, 3).unwrap());
}

#[test]
fn target_for_cursor_on_file_header_is_file_target() {
    let app = App::new(vec![file("a.rs", 1)]);
    let target = app.target_for_cursor().unwrap();
    assert_eq!(target, Target::file("a.rs"));
}

#[test]
fn target_for_cursor_on_binary_row_is_file_target() {
    let raw = "\
diff --git a/img.png b/img.png
index 1..2 100644
Binary files a/img.png and b/img.png differ
";
    let mut app = App::new(vec![
        FileDiff::from_patch(&RawFilePatch {
            path: "img.png".to_string(),
            old_path: None,
            raw: raw.to_string(),
            is_binary: true,
        })
        .unwrap(),
    ]);
    app.apply(Action::CursorDown); // Binary row
    let target = app.target_for_cursor().unwrap();
    assert_eq!(target, Target::file("img.png"));
}

#[test]
fn target_for_visual_removed_only_uses_old_side() {
    let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,3 +0,0 @@
-a
-b
-c
";
    let mut app = App::new(vec![file_with_raw("f.rs", raw)]);
    app.apply(Action::CursorDown); // hunk header
    app.apply(Action::CursorDown); // line a
    let anchor = app.view.cursor;
    app.apply(Action::EnterVisual);
    app.apply(Action::CursorDown); // line b
    app.apply(Action::CursorDown); // line c
    let target = app.target_for_visual(anchor).unwrap();
    assert_eq!(target, Target::range("f.rs", 1, 3, Side::Old).unwrap());
}

#[test]
fn target_for_visual_mixed_uses_new_side_of_non_removed_rows() {
    let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,2 +1,3 @@
-old
+new1
+new2
 ctx
";
    let mut app = App::new(vec![file_with_raw("f.rs", raw)]);
    app.apply(Action::CursorDown); // hunk header
    app.apply(Action::CursorDown); // removed "old"
    let anchor = app.view.cursor;
    app.apply(Action::EnterVisual);
    app.apply(Action::CursorDown); // new1
    app.apply(Action::CursorDown); // new2
    app.apply(Action::CursorDown); // ctx
    let target = app.target_for_visual(anchor).unwrap();
    // new1=1, new2=2, ctx=3 -> spans 1..3 on the new side.
    assert_eq!(target, Target::range("f.rs", 1, 3, Side::New).unwrap());
}

#[test]
fn target_for_visual_with_no_line_rows_is_none() {
    let app = App::new(vec![file("a.rs", 1)]);
    // Cursor and anchor both on the FileHeader row (0).
    let target = app.target_for_visual(0);
    assert_eq!(target, None);
}

// -- Compose -----------------------------------------------------------

#[test]
fn compose_action_in_normal_opens_compose_with_cursor_target() {
    let mut app = App::new(vec![file("a.rs", 1)]);
    app.apply(Action::Compose);
    assert_eq!(app.mode, Mode::Compose);
    let compose = app.compose.as_ref().unwrap();
    assert_eq!(compose.target, Target::file("a.rs"));
    assert_eq!(compose.editing_id, None);
}

#[test]
fn compose_action_with_no_target_is_a_no_op() {
    let mut app = App::new(vec![]);
    app.apply(Action::Compose);
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.compose.is_none());
}

#[test]
fn compose_action_in_visual_uses_range_target() {
    let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,2 +1,2 @@
 a
+b
";
    let mut app = App::new(vec![file_with_raw("f.rs", raw)]);
    app.apply(Action::CursorDown); // hunk header
    app.apply(Action::CursorDown); // line a
    app.apply(Action::EnterVisual);
    app.apply(Action::CursorDown); // line b
    app.apply(Action::Compose);
    assert_eq!(app.mode, Mode::Compose);
    let compose = app.compose.as_ref().unwrap();
    assert_eq!(
        compose.target,
        Target::range("f.rs", 1, 2, Side::New).unwrap()
    );
}

#[test]
fn cancel_compose_discards_draft() {
    let mut app = App::new(vec![file("a.rs", 1)]);
    app.apply(Action::Compose);
    app.compose.as_mut().unwrap().buffer.insert_char('x');
    app.cancel_compose();
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.compose.is_none());
    assert!(app.annotations.is_empty());
}

#[test]
fn submit_compose_with_body_adds_annotation_and_refreshes_rows() {
    let mut app = App::new(vec![file("a.rs", 1)]);
    app.apply(Action::Compose);
    for c in "looks good".chars() {
        app.compose.as_mut().unwrap().buffer.insert_char(c);
    }
    app.submit_compose();
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.annotations.len(), 1);
    assert_eq!(app.annotations.iter().next().unwrap().body, "looks good");
    // Row model was rebuilt: the FileHeader row is now flagged annotated.
    assert!(matches!(
        app.view.rows[0],
        Row::FileHeader {
            annotated: true,
            ..
        }
    ));
}

#[test]
fn submit_compose_with_empty_body_cancels_without_error() {
    let mut app = App::new(vec![file("a.rs", 1)]);
    app.apply(Action::Compose);
    app.compose.as_mut().unwrap().buffer.insert_char(' ');
    app.submit_compose();
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.annotations.is_empty());
}

#[test]
fn submit_compose_while_editing_updates_body_and_classification() {
    let mut app = App::new(vec![file("a.rs", 1)]);
    let id = app
        .annotations
        .add(Target::file("a.rs"), Classification::Nit, "old body")
        .unwrap();
    app.edit_focused_annotation(); // list_cursor defaults to 0
    app.compose.as_mut().unwrap().buffer = TextBuffer::new();
    for c in "new body".chars() {
        app.compose.as_mut().unwrap().buffer.insert_char(c);
    }
    app.compose.as_mut().unwrap().classification = Classification::Praise;
    app.submit_compose();
    let annotation = app.annotations.iter().find(|a| a.id == id).unwrap();
    assert_eq!(annotation.body, "new body");
    assert_eq!(annotation.classification, Classification::Praise);
}

// -- Annotation list panel ---------------------------------------------

#[test]
fn toggle_list_opens_and_closes() {
    let mut app = App::new(vec![file("a.rs", 1)]);
    app.apply(Action::ToggleList);
    assert_eq!(app.mode, Mode::List);
    app.apply(Action::ToggleList);
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn list_move_down_and_up_clamp() {
    let mut app = App::new(vec![file("a.rs", 1)]);
    app.annotations
        .add(Target::file("a.rs"), Classification::Nit, "one")
        .unwrap();
    app.annotations
        .add(Target::file("a.rs"), Classification::Issue, "two")
        .unwrap();
    app.list_move_down();
    assert_eq!(app.list_cursor, 1);
    app.list_move_down();
    assert_eq!(app.list_cursor, 1); // clamped at last
    app.list_move_up();
    assert_eq!(app.list_cursor, 0);
    app.list_move_up();
    assert_eq!(app.list_cursor, 0); // clamped at first
}

#[test]
fn jump_to_focused_annotation_switches_file_and_places_cursor() {
    let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
    app.annotations
        .add(
            Target::line("b.rs", 1, Side::Old),
            Classification::Issue,
            "note",
        )
        .unwrap();
    app.list_cursor = 0;
    app.jump_to_focused_annotation();
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.view.selected_file, 1);
    let Row::Line(line) = &app.view.rows[app.view.cursor] else {
        panic!("expected cursor on a line row");
    };
    assert_eq!(line.old_line, Some(1));
}

#[test]
fn jump_to_annotation_expands_a_collapsed_target_section() {
    // Jumping to an annotation whose file is collapsed must re-expand
    // that section so the line/hunk anchor is reachable, then land on it.
    let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
    app.annotations
        .add(
            Target::line("b.rs", 1, Side::New),
            Classification::Issue,
            "note",
        )
        .unwrap();
    app.view.set_collapsed("b.rs", true);
    app.rebuild_rows();
    assert!(app.view.is_collapsed("b.rs"));

    app.list_cursor = 0;
    app.jump_to_focused_annotation();

    assert!(!app.view.is_collapsed("b.rs")); // re-expanded
    assert_eq!(app.view.selected_file, 1);
    let Row::Line(line) = &app.view.rows[app.view.cursor] else {
        panic!("expected cursor on a line row");
    };
    assert_eq!(line.new_line, Some(1));
}

// -- Markdown-on-quit output is unchanged by the multibuffer -------------

#[test]
fn multi_section_annotations_emit_unchanged_markdown() {
    // The stdout markdown is a public API keyed purely off the annotation
    // store's insertion order — the multibuffer never touches it. Compose
    // annotations across several sections and assert byte-for-byte output.
    use crate::annotate::render_markdown;
    let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1), file("c.rs", 1)]);
    app.annotations
        .add(Target::file("a.rs"), Classification::Praise, "clean module")
        .unwrap();
    app.annotations
        .add(
            Target::line("b.rs", 1, Side::New),
            Classification::Question,
            "why new0?",
        )
        .unwrap();
    app.annotations
        .add(
            Target::hunk("c.rs", 1, 1).unwrap(),
            Classification::Nit,
            "tidy this hunk",
        )
        .unwrap();
    app.rebuild_rows();

    let expected = "\
## a.rs\n\n[praise] clean module\n\n\
## b.rs:1 (+)\n\n[question] why new0?\n\n\
## c.rs:1-1 (+)\n\n[nit] tidy this hunk\n";
    assert_eq!(render_markdown(&app.annotations), expected);
}

#[test]
fn edit_focused_annotation_prefills_compose() {
    let mut app = App::new(vec![file("a.rs", 1)]);
    app.annotations
        .add(Target::file("a.rs"), Classification::Question, "why?")
        .unwrap();
    app.list_cursor = 0;
    app.edit_focused_annotation();
    assert_eq!(app.mode, Mode::Compose);
    let compose = app.compose.as_ref().unwrap();
    assert_eq!(compose.buffer.text(), "why?");
    assert_eq!(compose.classification, Classification::Question);
    assert_eq!(compose.editing_id, Some(0));
}

#[test]
fn delete_focused_annotation_removes_and_refreshes() {
    let mut app = App::new(vec![file("a.rs", 1)]);
    app.annotations
        .add(Target::file("a.rs"), Classification::Nit, "note")
        .unwrap();
    app.list_cursor = 0;
    app.delete_focused_annotation();
    assert!(app.annotations.is_empty());
    assert!(matches!(
        app.view.rows[0],
        Row::FileHeader {
            annotated: false,
            ..
        }
    ));
}

#[test]
fn list_actions_on_empty_store_are_no_ops() {
    let mut app = App::new(vec![file("a.rs", 1)]);
    app.list_move_down();
    assert_eq!(app.list_cursor, 0);
    app.delete_focused_annotation();
    assert!(app.annotations.is_empty());
    app.edit_focused_annotation();
    assert!(app.compose.is_none());
}

// -- Staging -------------------------------------------------------------

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use crate::git::{ChangeKind, FileStatus, GitError, StatusCode};

/// One recorded call against the fake staging backend.
#[derive(Debug, Clone, PartialEq)]
enum StageCall {
    StageFile(String),
    UnstageFile(String),
    Apply(String),
    Unapply(String),
}

/// A recording [`StageOps`] fake: staging calls are appended to a
/// shared log; `diff`/`status` return fixed data (what refresh will
/// see after an operation); `fail_ops` makes every staging call error.
#[derive(Default)]
struct FakeGit {
    calls: Rc<RefCell<Vec<StageCall>>>,
    diff: Vec<RawFilePatch>,
    status: Vec<FileStatus>,
    // Interior-mutable overrides: when set, `diff`/`status` read through
    // these instead, so a test can mutate the refresh result mid-flow
    // (e.g. to simulate an external edit landing between operations).
    diff_override: Option<Rc<RefCell<Vec<RawFilePatch>>>>,
    status_override: Option<Rc<RefCell<Vec<FileStatus>>>>,
    untracked_content: std::collections::HashMap<String, Vec<u8>>,
    fail_ops: bool,
    show_calls: Rc<RefCell<usize>>,
    show_content: Option<String>,
    branch: Option<BranchStatus>,
    stashes: Vec<StashEntry>,
}

impl FakeGit {
    fn op_result(&self) -> Result<(), GitError> {
        if self.fail_ops {
            Err(GitError::Parse("simulated git failure".to_string()))
        } else {
            Ok(())
        }
    }
}

impl StageOps for FakeGit {
    fn diff(&self, _target: &DiffTarget) -> Result<Vec<RawFilePatch>, GitError> {
        match &self.diff_override {
            Some(h) => Ok(h.borrow().clone()),
            None => Ok(self.diff.clone()),
        }
    }

    fn status(&self) -> Result<Vec<FileStatus>, GitError> {
        match &self.status_override {
            Some(h) => Ok(h.borrow().clone()),
            None => Ok(self.status.clone()),
        }
    }

    fn stage_file(&self, path: &str) -> Result<(), GitError> {
        self.calls
            .borrow_mut()
            .push(StageCall::StageFile(path.to_string()));
        self.op_result()
    }

    fn unstage_file(&self, path: &str) -> Result<(), GitError> {
        self.calls
            .borrow_mut()
            .push(StageCall::UnstageFile(path.to_string()));
        self.op_result()
    }

    fn apply_cached(&self, patch: &str) -> Result<(), GitError> {
        self.calls
            .borrow_mut()
            .push(StageCall::Apply(patch.to_string()));
        self.op_result()
    }

    fn unapply_cached(&self, patch: &str) -> Result<(), GitError> {
        self.calls
            .borrow_mut()
            .push(StageCall::Unapply(patch.to_string()));
        self.op_result()
    }

    fn read_worktree_file(&self, path: &str) -> Option<Vec<u8>> {
        self.untracked_content.get(path).cloned()
    }

    fn show_file(&self, _spec: &str) -> Option<String> {
        *self.show_calls.borrow_mut() += 1;
        self.show_content.clone()
    }

    fn branch_status(&self) -> Result<BranchStatus, GitError> {
        self.branch
            .clone()
            .ok_or_else(|| GitError::Parse("no branch".into()))
    }

    fn stash_list(&self) -> Result<Vec<StashEntry>, GitError> {
        Ok(self.stashes.clone())
    }
}

fn raw_patch(path: &str, hunk_count: usize) -> RawFilePatch {
    let mut raw =
        format!("diff --git a/{path} b/{path}\nindex 1..2 100644\n--- a/{path}\n+++ b/{path}\n");
    for h in 0..hunk_count {
        let start = 1 + h * 10;
        raw.push_str(&format!("@@ -{start},1 +{start},1 @@\n-old{h}\n+new{h}\n"));
    }
    RawFilePatch {
        path: path.to_string(),
        old_path: None,
        raw,
        is_binary: false,
    }
}

/// A porcelain status entry with staged (index-side) changes only
/// (`M.` → fully staged).
fn staged_entry(path: &str) -> FileStatus {
    FileStatus {
        kind: ChangeKind::Ordinary,
        staged: StatusCode::Modified,
        unstaged: StatusCode::Unmodified,
        path: path.to_string(),
        orig_path: None,
    }
}

/// A porcelain status entry with both staged and unstaged changes
/// (`MM` → partially staged).
fn partial_entry(path: &str) -> FileStatus {
    FileStatus {
        kind: ChangeKind::Ordinary,
        staged: StatusCode::Modified,
        unstaged: StatusCode::Modified,
        path: path.to_string(),
        orig_path: None,
    }
}

/// A porcelain status entry with working-tree-only changes (`.M` →
/// unstaged).
fn unstaged_entry(path: &str) -> FileStatus {
    FileStatus {
        kind: ChangeKind::Ordinary,
        staged: StatusCode::Unmodified,
        unstaged: StatusCode::Modified,
        path: path.to_string(),
        orig_path: None,
    }
}

/// Builds an `App` over `patches` with a recording fake whose refresh
/// diff returns `refresh_diff` and refresh status returns `status`.
/// Returns the app plus the shared call log.
fn app_with_fake(
    patches: Vec<RawFilePatch>,
    target: DiffTarget,
    refresh_diff: Vec<RawFilePatch>,
    status: Vec<FileStatus>,
) -> (App, Rc<RefCell<Vec<StageCall>>>) {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let fake = FakeGit {
        calls: Rc::clone(&calls),
        diff: refresh_diff,
        status,
        ..FakeGit::default()
    };
    let files = patches
        .iter()
        .map(|p| FileDiff::from_patch(p).unwrap())
        .collect();
    let snapshot = ReviewSnapshot {
        files,
        patches: patches.into_iter().map(Some).collect(),
        staged: Vec::new(),
        staged_states: HashMap::new(),
    };
    (App::with_git(snapshot, target, Box::new(fake)), calls)
}

/// The single call in the log, panicking if there are zero or many.
fn single_call(calls: &Rc<RefCell<Vec<StageCall>>>) -> StageCall {
    let calls = calls.borrow();
    assert_eq!(calls.len(), 1, "expected exactly one call, got {calls:?}");
    calls[0].clone()
}

#[test]
fn space_on_hunk_header_stages_that_hunk() {
    let p = raw_patch("a.rs", 2);
    let (mut app, calls) = app_with_fake(vec![p.clone()], DiffTarget::WorkingTree, vec![p], vec![]);
    app.apply(Action::NextHunk);
    app.apply(Action::NextHunk); // second hunk's header
    app.apply(Action::ToggleStage);
    let StageCall::Apply(patch) = single_call(&calls) else {
        panic!("expected apply_cached");
    };
    assert!(patch.contains("@@ -11,1 +11,1 @@"));
    assert!(patch.contains("-old1"));
    assert!(!patch.contains("old0"));
    assert_eq!(app.status_message.as_deref(), Some("staged hunk"));
}

#[test]
fn space_on_line_row_stages_enclosing_hunk() {
    let p = raw_patch("a.rs", 1);
    let (mut app, calls) = app_with_fake(vec![p.clone()], DiffTarget::WorkingTree, vec![p], vec![]);
    app.apply(Action::CursorDown); // hunk header
    app.apply(Action::CursorDown); // line row
    assert!(matches!(app.view.rows[app.view.cursor], Row::Line(_)));
    app.apply(Action::ToggleStage);
    let StageCall::Apply(patch) = single_call(&calls) else {
        panic!("expected apply_cached");
    };
    assert!(patch.contains("-old0"));
    assert!(patch.contains("+new0"));
}

#[test]
fn space_on_file_header_stages_whole_file() {
    let p = raw_patch("a.rs", 1);
    let (mut app, calls) = app_with_fake(vec![p.clone()], DiffTarget::WorkingTree, vec![p], vec![]);
    assert!(matches!(
        app.view.rows[app.view.cursor],
        Row::FileHeader { .. }
    ));
    app.apply(Action::ToggleStage);
    assert_eq!(
        single_call(&calls),
        StageCall::StageFile("a.rs".to_string())
    );
    assert_eq!(app.status_message.as_deref(), Some("staged a.rs"));
}

#[test]
fn space_on_binary_row_stages_whole_file() {
    let raw = "\
diff --git a/img.png b/img.png
index 1..2 100644
Binary files a/img.png and b/img.png differ
";
    let p = RawFilePatch {
        path: "img.png".to_string(),
        old_path: None,
        raw: raw.to_string(),
        is_binary: true,
    };
    let (mut app, calls) = app_with_fake(vec![p.clone()], DiffTarget::WorkingTree, vec![p], vec![]);
    app.apply(Action::CursorDown); // Binary row
    assert!(matches!(app.view.rows[app.view.cursor], Row::Binary));
    app.apply(Action::ToggleStage);
    assert_eq!(
        single_call(&calls),
        StageCall::StageFile("img.png".to_string())
    );
}

#[test]
fn space_on_untracked_file_falls_back_to_stage_file_at_any_granularity() {
    // A synthetic untracked file has no raw patch (`patches[i]` is
    // `None`); even a line-row cursor must stage the whole file.
    let calls = Rc::new(RefCell::new(Vec::new()));
    let fake = FakeGit {
        calls: Rc::clone(&calls),
        ..FakeGit::default()
    };
    let snapshot = ReviewSnapshot {
        files: vec![FileDiff::synthetic_added("new.rs".to_string(), "x\ny\n")],
        patches: vec![None],
        staged: Vec::new(),
        staged_states: HashMap::new(),
    };
    let mut app = App::with_git(snapshot, DiffTarget::WorkingTree, Box::new(fake));
    app.apply(Action::CursorDown); // hunk header
    app.apply(Action::CursorDown); // line row
    assert!(matches!(app.view.rows[app.view.cursor], Row::Line(_)));
    app.apply(Action::ToggleStage);
    assert_eq!(
        single_call(&calls),
        StageCall::StageFile("new.rs".to_string())
    );
}

#[test]
fn untracked_visual_selection_falls_back_to_stage_file() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let fake = FakeGit {
        calls: Rc::clone(&calls),
        ..FakeGit::default()
    };
    let snapshot = ReviewSnapshot {
        files: vec![FileDiff::synthetic_added("new.rs".to_string(), "x\ny\n")],
        patches: vec![None],
        staged: Vec::new(),
        staged_states: HashMap::new(),
    };
    let mut app = App::with_git(snapshot, DiffTarget::WorkingTree, Box::new(fake));
    app.apply(Action::CursorDown);
    app.apply(Action::CursorDown);
    app.apply(Action::EnterVisual);
    app.apply(Action::CursorDown);
    app.apply(Action::ToggleStage);
    assert_eq!(
        single_call(&calls),
        StageCall::StageFile("new.rs".to_string())
    );
}

#[test]
fn staged_target_space_unstages_hunk() {
    let p = raw_patch("a.rs", 1);
    let (mut app, calls) = app_with_fake(vec![p.clone()], DiffTarget::Staged, vec![p], vec![]);
    app.apply(Action::CursorDown); // hunk header
    app.apply(Action::ToggleStage);
    assert!(matches!(single_call(&calls), StageCall::Unapply(_)));
    assert_eq!(app.status_message.as_deref(), Some("unstaged hunk"));
}

#[test]
fn staged_target_file_header_unstages_file() {
    let p = raw_patch("a.rs", 1);
    let (mut app, calls) = app_with_fake(vec![p.clone()], DiffTarget::Staged, vec![p], vec![]);
    app.apply(Action::ToggleStage);
    assert_eq!(
        single_call(&calls),
        StageCall::UnstageFile("a.rs".to_string())
    );
}

#[test]
fn range_target_space_is_noop_with_message() {
    let p = raw_patch("a.rs", 1);
    let (mut app, calls) = app_with_fake(
        vec![p.clone()],
        DiffTarget::Range("main..HEAD".to_string()),
        vec![p],
        vec![],
    );
    let files_before = app.view.files.len();
    app.apply(Action::ToggleStage);
    assert!(calls.borrow().is_empty());
    assert_eq!(app.status_message.as_deref(), Some("read-only diff target"));
    assert_eq!(app.view.files.len(), files_before);
}

#[test]
fn visual_selection_stages_only_selected_lines() {
    let p = raw_patch("a.rs", 1);
    let (mut app, calls) = app_with_fake(vec![p.clone()], DiffTarget::WorkingTree, vec![p], vec![]);
    // rows: FileHeader(0) HunkHeader(1) -old0(2) +new0(3)
    app.apply(Action::CursorDown);
    app.apply(Action::CursorDown);
    app.apply(Action::CursorDown); // +new0
    app.apply(Action::EnterVisual); // anchor on +new0 only
    app.apply(Action::ToggleStage);
    let StageCall::Apply(patch) = single_call(&calls) else {
        panic!("expected apply_cached");
    };
    // Selected addition kept; unselected removal downgraded to context.
    assert!(patch.contains("+new0\n"));
    assert!(patch.contains(" old0\n"));
    assert!(!patch.contains("-old0"));
    assert_eq!(app.status_message.as_deref(), Some("staged 1 line"));
    assert_eq!(app.mode, Mode::Normal); // visual exits on success
}

#[test]
fn visual_selection_on_staged_target_unstages_lines() {
    let p = raw_patch("a.rs", 1);
    let (mut app, calls) = app_with_fake(vec![p.clone()], DiffTarget::Staged, vec![p], vec![]);
    app.apply(Action::CursorDown);
    app.apply(Action::CursorDown); // -old0
    app.apply(Action::EnterVisual);
    app.apply(Action::CursorDown); // extend to +new0
    app.apply(Action::ToggleStage);
    let StageCall::Unapply(patch) = single_call(&calls) else {
        panic!("expected unapply_cached");
    };
    assert!(patch.contains("-old0"));
    assert!(patch.contains("+new0"));
    assert_eq!(app.status_message.as_deref(), Some("unstaged 2 lines"));
}

#[test]
fn visual_selection_spanning_multiple_hunks_is_rejected() {
    let p = raw_patch("a.rs", 2);
    let (mut app, calls) = app_with_fake(vec![p.clone()], DiffTarget::WorkingTree, vec![p], vec![]);
    // rows: FH(0) HH0(1) -old0(2) +new0(3) HH1(4) -old1(5) +new1(6)
    app.apply(Action::CursorDown);
    app.apply(Action::CursorDown); // -old0
    app.apply(Action::EnterVisual);
    for _ in 0..3 {
        app.apply(Action::CursorDown); // through HH1 into -old1
    }
    app.apply(Action::ToggleStage);
    assert!(calls.borrow().is_empty());
    assert_eq!(
        app.status_message.as_deref(),
        Some("selection spans multiple hunks")
    );
    assert!(matches!(app.mode, Mode::Visual { .. })); // selection kept
}

#[test]
fn visual_selection_with_no_changed_lines_is_rejected() {
    let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,2 +1,3 @@
 ctx1
+added
 ctx2
";
    let p = RawFilePatch {
        path: "f.rs".to_string(),
        old_path: None,
        raw: raw.to_string(),
        is_binary: false,
    };
    let (mut app, calls) = app_with_fake(vec![p.clone()], DiffTarget::WorkingTree, vec![p], vec![]);
    app.apply(Action::CursorDown);
    app.apply(Action::CursorDown); // ctx1
    app.apply(Action::EnterVisual); // select just the context line
    app.apply(Action::ToggleStage);
    assert!(calls.borrow().is_empty());
    assert_eq!(
        app.status_message.as_deref(),
        Some("no changed lines in selection")
    );
}

#[test]
fn refresh_keeps_selected_file_by_path_when_order_changes() {
    let a = raw_patch("a.rs", 1);
    let b = raw_patch("b.rs", 1);
    // After the operation the diff comes back reordered: [b, a].
    let (mut app, _calls) = app_with_fake(
        vec![a.clone(), b.clone()],
        DiffTarget::WorkingTree,
        vec![b, a],
        vec![],
    );
    app.apply(Action::NextFile); // select b.rs (index 1)
    app.apply(Action::ToggleStage); // stage b.rs whole-file, then refresh
    assert_eq!(app.view.files[app.view.selected_file].path, "b.rs");
    assert_eq!(app.view.selected_file, 0); // b.rs moved to index 0
}

#[test]
fn refresh_selects_nearest_file_when_selected_disappears() {
    let a = raw_patch("a.rs", 1);
    let b = raw_patch("b.rs", 1);
    // Staging all of b.rs removes it from the working-tree diff.
    let (mut app, _calls) =
        app_with_fake(vec![a.clone(), b], DiffTarget::WorkingTree, vec![a], vec![]);
    app.apply(Action::NextFile); // select b.rs (index 1)
    app.apply(Action::ToggleStage);
    assert_eq!(app.view.selected_file, 0);
    assert_eq!(app.view.files[app.view.selected_file].path, "a.rs");
    assert!(app.view.cursor <= app.view.rows.len().saturating_sub(1));
}

#[test]
fn refresh_clamps_cursor_when_file_shrinks() {
    let big = raw_patch("a.rs", 3); // 1 + 3*3 = 10 rows
    let small = raw_patch("a.rs", 1); // 4 rows
    let (mut app, _calls) = app_with_fake(vec![big], DiffTarget::WorkingTree, vec![small], vec![]);
    for _ in 0..9 {
        app.apply(Action::CursorDown);
    }
    assert_eq!(app.view.cursor, 9);
    app.apply(Action::ToggleStage); // hunk op + refresh to the small diff
    assert!(app.view.cursor < app.view.rows.len());
    assert_eq!(app.view.rows.len(), 4);
}

#[test]
fn refresh_after_empty_diff_resets_cursor_and_selection() {
    let p = raw_patch("a.rs", 1);
    let (mut app, _calls) = app_with_fake(vec![p], DiffTarget::WorkingTree, vec![], vec![]);
    app.apply(Action::CursorDown);
    app.apply(Action::ToggleStage);
    assert!(app.view.files.is_empty());
    assert_eq!(app.view.cursor, 0);
    assert_eq!(app.view.scroll, 0);
    assert_eq!(app.view.selected_file, 0);
}

#[test]
fn refresh_updates_staged_list_and_counts_from_status() {
    let p = raw_patch("a.rs", 1);
    let (mut app, _calls) = app_with_fake(
        vec![p.clone()],
        DiffTarget::WorkingTree,
        vec![p],
        vec![staged_entry("a.rs")],
    );
    assert!(app.staged.is_empty());
    app.apply(Action::ToggleStage); // whole file, then refresh
    assert_eq!(app.staged.len(), 1);
    assert_eq!(app.staged[0].path, "a.rs");
    assert_eq!(app.staged[0].letter, 'M');
}

#[test]
fn refresh_repopulates_branch_and_stash_and_preserves_staged_and_annotations() {
    let p = raw_patch("a.rs", 1);
    let calls = Rc::new(RefCell::new(Vec::new()));
    let fake = FakeGit {
        calls: Rc::clone(&calls),
        diff: vec![p.clone()],
        status: vec![staged_entry("a.rs")],
        branch: Some(BranchStatus {
            name: "main".into(),
            detached: false,
            upstream: Some("origin/main".into()),
            ahead_behind: Some((2, 1)),
        }),
        stashes: vec![StashEntry {
            stash_ref: "stash@{0}".into(),
            branch: Some("main".into()),
            message: "wip: parser".into(),
        }],
        ..FakeGit::default()
    };
    let snapshot = ReviewSnapshot {
        files: vec![FileDiff::from_patch(&p).unwrap()],
        patches: vec![Some(p)],
        staged: Vec::new(),
        staged_states: HashMap::new(),
    };
    let mut app = App::with_git(snapshot, DiffTarget::WorkingTree, Box::new(fake));
    // Startup read populated branch/stash state.
    assert_eq!(app.branch.as_ref().unwrap().name, "main");
    assert_eq!(app.stashes.len(), 1);
    // An annotation made this session must survive the refresh.
    app.annotations
        .add(Target::file("a.rs"), Classification::Nit, "look here")
        .unwrap();

    app.refresh();

    assert_eq!(app.branch.as_ref().unwrap().ahead_behind, Some((2, 1)));
    assert_eq!(app.stashes[0].message, "wip: parser");
    // Staged markers survive: the refresh status reports a.rs staged.
    assert_eq!(app.staged.len(), 1);
    assert_eq!(app.staged[0].path, "a.rs");
    // Annotations survive the refresh exactly as today.
    assert_eq!(app.annotations.len(), 1);
}

// -- Remote operations & command log (task 4.0) ------------------------

/// While a remote op is in flight, a second request is rejected with a
/// status message and spawns nothing — the guard is a message, not a queue.
#[test]
fn second_remote_request_while_one_in_flight_is_rejected_and_does_not_spawn() {
    let mut app = App::new(vec![file("a.rs", 1)]);
    // A repo root is present (so a request *could* spawn) and a fetch is
    // already recorded as in flight.
    app.repo_root = Some(std::path::PathBuf::from("/tmp"));
    app.remote_op = Some(InFlightRemote {
        id: TaskId(7),
        op: RemoteOp::Fetch,
    });

    app.request_remote_op(RemoteOp::Pull);

    // The in-flight op is untouched (still the fetch), the request was
    // rejected with a message, and nothing new was spawned.
    assert_eq!(app.remote_op.map(|o| o.op), Some(RemoteOp::Fetch));
    assert!(
        app.status_message
            .as_deref()
            .is_some_and(|m| m.contains("already running")),
        "got {:?}",
        app.status_message
    );
    assert!(
        app.background.poll().is_empty(),
        "the rejected request must not spawn a background task"
    );
}

/// Without a known repository root, a remote request degrades to a footer
/// message rather than panicking or spawning.
#[test]
fn remote_request_without_a_repo_root_is_a_message_only() {
    let mut app = App::new(vec![file("a.rs", 1)]);
    assert!(app.repo_root.is_none());
    app.request_remote_op(RemoteOp::Fetch);
    assert!(app.remote_op.is_none());
    assert_eq!(
        app.status_message.as_deref(),
        Some("remote operations unavailable (no repository)")
    );
}

/// The full spawn -> poll -> log pipeline, driven through the *real*
/// [`BackgroundTasks`] path with a benign successful command standing in
/// for git: on completion the command log gains an entry, the refresh
/// re-reads branch/stash state, and staged markers plus annotations
/// survive exactly as after any refresh.
#[cfg(unix)]
#[test]
fn completed_remote_op_logs_and_refreshes_preserving_staged_and_annotations() {
    use std::process::Command;
    use std::time::{Duration, Instant};

    let p = raw_patch("a.rs", 1);
    let calls = Rc::new(RefCell::new(Vec::new()));
    let fake = FakeGit {
        calls: Rc::clone(&calls),
        diff: vec![p.clone()],
        status: vec![staged_entry("a.rs")],
        branch: Some(BranchStatus {
            name: "main".into(),
            detached: false,
            upstream: Some("origin/main".into()),
            ahead_behind: Some((0, 0)),
        }),
        stashes: vec![StashEntry {
            stash_ref: "stash@{0}".into(),
            branch: Some("main".into()),
            message: "wip: parser".into(),
        }],
        ..FakeGit::default()
    };
    let snapshot = ReviewSnapshot {
        files: vec![FileDiff::from_patch(&p).unwrap()],
        patches: vec![Some(p)],
        staged: Vec::new(),
        staged_states: HashMap::new(),
    };
    let mut app = App::with_git(snapshot, DiffTarget::WorkingTree, Box::new(fake));
    app.annotations
        .add(Target::file("a.rs"), Classification::Nit, "look here")
        .unwrap();

    // Spawn a benign successful command through the real background poller
    // and mark it as the in-flight fetch (this is exactly what
    // `request_remote_op` does, minus running git itself).
    let id = app
        .background
        .spawn(|| run_command(&mut Command::new("true")));
    app.remote_op = Some(InFlightRemote {
        id,
        op: RemoteOp::Fetch,
    });

    // Drain until the op is logged (mirrors the event-loop tick).
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.command_log.is_empty() && Instant::now() < deadline {
        app.poll_remote();
        std::thread::sleep(Duration::from_millis(5));
    }

    // The command log gained exactly one entry for the fetch.
    assert_eq!(app.command_log.len(), 1);
    let entry = app.command_log.entries().next().unwrap();
    assert_eq!(entry.command_line, "git fetch");
    assert!(entry.success);
    // The guard is cleared, so a fresh op could start.
    assert!(app.remote_op.is_none());
    // Refresh ran: branch/stash re-read; staged markers + annotations survive.
    assert_eq!(app.branch.as_ref().unwrap().name, "main");
    assert_eq!(app.stashes.len(), 1);
    assert_eq!(app.staged.len(), 1);
    assert_eq!(app.staged[0].path, "a.rs");
    assert_eq!(app.annotations.len(), 1);
}

#[test]
fn stage_error_sets_message_and_leaves_state_unchanged() {
    let p = raw_patch("a.rs", 1);
    let calls = Rc::new(RefCell::new(Vec::new()));
    let fake = FakeGit {
        calls: Rc::clone(&calls),
        // If a refresh ran anyway, files would empty out — the
        // assertion below would catch it.
        diff: vec![],
        fail_ops: true,
        ..FakeGit::default()
    };
    let snapshot = ReviewSnapshot {
        files: vec![FileDiff::from_patch(&p).unwrap()],
        patches: vec![Some(p)],
        staged: Vec::new(),
        staged_states: HashMap::new(),
    };
    let mut app = App::with_git(snapshot, DiffTarget::WorkingTree, Box::new(fake));
    app.apply(Action::CursorDown); // hunk header
    let cursor_before = app.view.cursor;
    app.apply(Action::ToggleStage);
    assert_eq!(app.view.files.len(), 1);
    assert_eq!(app.view.cursor, cursor_before);
    assert!(
        app.status_message
            .as_deref()
            .is_some_and(|m| m.contains("simulated git failure"))
    );
}

#[test]
fn space_without_git_backend_sets_message_only() {
    let mut app = App::new(vec![file("a.rs", 1)]);
    app.apply(Action::ToggleStage);
    assert_eq!(
        app.status_message.as_deref(),
        Some("staging unavailable (no git backend)")
    );
}

#[test]
fn toggle_stage_in_list_and_compose_modes_is_a_no_op() {
    let p = raw_patch("a.rs", 1);
    let (mut app, calls) = app_with_fake(vec![p.clone()], DiffTarget::WorkingTree, vec![p], vec![]);
    app.mode = Mode::List;
    app.apply(Action::ToggleStage);
    app.mode = Mode::Compose;
    app.apply(Action::ToggleStage);
    assert!(calls.borrow().is_empty());
    assert!(app.status_message.is_none());
}

#[test]
fn status_message_set_and_clear() {
    let mut app = App::new(vec![file("a.rs", 1)]);
    app.set_status_message("staged hunk");
    assert_eq!(app.status_message.as_deref(), Some("staged hunk"));
    app.clear_status_message();
    assert!(app.status_message.is_none());
}

// -- Stage-and-collapse review flow (S) ----------------------------------

/// Builds an `App` whose fake reads its refresh diff/status through
/// mutable handles, so a test can change what the next `refresh` sees.
/// The snapshot is derived from `initial_status` (staged list + states)
/// over `initial_files`.
#[allow(clippy::type_complexity)]
fn app_with_mutable_fake(
    initial_files: Vec<RawFilePatch>,
    initial_status: Vec<FileStatus>,
    refresh_diff: Vec<RawFilePatch>,
    refresh_status: Vec<FileStatus>,
) -> (
    App,
    Rc<RefCell<Vec<StageCall>>>,
    Rc<RefCell<Vec<RawFilePatch>>>,
    Rc<RefCell<Vec<FileStatus>>>,
) {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let diff_h = Rc::new(RefCell::new(refresh_diff));
    let status_h = Rc::new(RefCell::new(refresh_status));
    let fake = FakeGit {
        calls: Rc::clone(&calls),
        diff_override: Some(Rc::clone(&diff_h)),
        status_override: Some(Rc::clone(&status_h)),
        ..FakeGit::default()
    };
    let files = initial_files
        .iter()
        .map(|p| FileDiff::from_patch(p).unwrap())
        .collect();
    let snapshot = ReviewSnapshot {
        files,
        patches: initial_files.into_iter().map(Some).collect(),
        staged: staged_from_status(&initial_status),
        staged_states: staged_states_from_status(&initial_status),
    };
    let app = App::with_git(snapshot, DiffTarget::WorkingTree, Box::new(fake));
    (app, calls, diff_h, status_h)
}

#[test]
fn stage_file_stages_the_file_and_collapses_its_section() {
    let p = raw_patch("a.rs", 1);
    // After staging, a.rs is fully staged and gone from the working diff;
    // decision (A) keeps it as a header-only section from status.
    let (mut app, calls) = app_with_fake(
        vec![p],
        DiffTarget::WorkingTree,
        vec![], // nothing unstaged left
        vec![staged_entry("a.rs")],
    );
    assert!(!app.view.is_collapsed("a.rs"));
    app.apply(Action::StageFile);
    assert_eq!(
        single_call(&calls),
        StageCall::StageFile("a.rs".to_string())
    );
    assert!(app.view.is_collapsed("a.rs"));
    // The section persists (never hides), now marked fully staged.
    assert_eq!(app.view.files.len(), 1);
    assert_eq!(app.view.files[0].path, "a.rs");
    assert_eq!(
        app.staged_states.get("a.rs").copied(),
        Some(StagedState::Full)
    );
    assert_eq!(app.status_message.as_deref(), Some("staged a.rs"));
}

#[test]
fn stage_file_on_fully_staged_file_unstages_and_expands() {
    let p = raw_patch("a.rs", 1);
    // Start fully staged (collapsed at launch); after unstaging, a.rs is
    // back in the working diff with nothing staged.
    let (mut app, calls, _diff, _status) = app_with_mutable_fake(
        vec![p.clone()],
        vec![staged_entry("a.rs")],   // initial: M. -> Full
        vec![p],                      // refresh diff after unstage
        vec![unstaged_entry("a.rs")], // refresh status: unstaged only
    );
    assert!(app.view.is_collapsed("a.rs")); // fully staged starts collapsed
    // Cursor sits on the collapsed header.
    app.apply(Action::StageFile);
    assert_eq!(
        single_call(&calls),
        StageCall::UnstageFile("a.rs".to_string())
    );
    assert!(!app.view.is_collapsed("a.rs")); // auto-expanded
    assert_eq!(app.staged_states.get("a.rs").copied(), None); // no longer staged
    assert_eq!(app.status_message.as_deref(), Some("unstaged a.rs"));
}

#[test]
fn stage_file_records_only_stageops_methods() {
    // Both directions must go through `stage_file`/`unstage_file` — no
    // new git-layer gestures.
    let p = raw_patch("a.rs", 1);
    let (mut app, calls) = app_with_fake(
        vec![p],
        DiffTarget::WorkingTree,
        vec![],
        vec![staged_entry("a.rs")],
    );
    app.apply(Action::StageFile);
    for call in calls.borrow().iter() {
        assert!(
            matches!(call, StageCall::StageFile(_) | StageCall::UnstageFile(_)),
            "unexpected call {call:?}"
        );
    }
}

#[test]
fn stage_file_on_read_only_range_is_a_noop_with_message() {
    let p = raw_patch("a.rs", 1);
    let (mut app, calls) = app_with_fake(
        vec![p.clone()],
        DiffTarget::Range("main..HEAD".to_string()),
        vec![p],
        vec![],
    );
    app.apply(Action::StageFile);
    assert!(calls.borrow().is_empty());
    assert_eq!(app.status_message.as_deref(), Some("read-only diff target"));
}

#[test]
fn stage_file_error_sets_message_and_leaves_state_unchanged() {
    let p = raw_patch("a.rs", 1);
    let calls = Rc::new(RefCell::new(Vec::new()));
    let fake = FakeGit {
        calls: Rc::clone(&calls),
        diff: vec![],
        fail_ops: true,
        ..FakeGit::default()
    };
    let snapshot = ReviewSnapshot {
        files: vec![FileDiff::from_patch(&p).unwrap()],
        patches: vec![Some(p)],
        staged: Vec::new(),
        staged_states: HashMap::new(),
    };
    let mut app = App::with_git(snapshot, DiffTarget::WorkingTree, Box::new(fake));
    app.apply(Action::StageFile);
    // The stage was attempted but failed; nothing collapsed, file kept.
    assert_eq!(
        single_call(&calls),
        StageCall::StageFile("a.rs".to_string())
    );
    assert!(!app.view.is_collapsed("a.rs"));
    assert_eq!(app.view.files.len(), 1);
    assert!(
        app.status_message
            .as_deref()
            .is_some_and(|m| m.contains("simulated git failure"))
    );
}

#[test]
fn hunk_stage_marks_file_partial_and_keeps_it_expanded() {
    // Staging one of two hunks leaves the file partially staged: its
    // header marker becomes `±` and the section stays expanded (its
    // unstaged work is still visible).
    let p = raw_patch("a.rs", 2);
    let (mut app, _calls) = app_with_fake(
        vec![p.clone()],
        DiffTarget::WorkingTree,
        vec![p],                     // still has unstaged content
        vec![partial_entry("a.rs")], // MM -> Partial
    );
    app.apply(Action::NextHunk); // onto a hunk header
    app.apply(Action::ToggleStage); // stage that hunk, then refresh
    assert_eq!(
        app.staged_states.get("a.rs").copied(),
        Some(StagedState::Partial)
    );
    let header = app.view.header_row_of_file[0];
    let Row::FileHeader { staged_marker, .. } = &app.view.rows[header] else {
        panic!("expected file header");
    };
    assert_eq!(*staged_marker, StagedMarker::Partial);
    assert!(!app.view.is_collapsed("a.rs"));
}

// -- Refresh collapse-map rules (task 3.3) -------------------------------

#[test]
fn refresh_auto_expands_a_partially_staged_collapsed_file() {
    // The "nothing hides" guarantee: a fully-staged (collapsed) file that
    // gets edited again comes back partially staged and must re-expand.
    let p = raw_patch("a.rs", 1);
    let (mut app, _calls, _diff, _status) = app_with_mutable_fake(
        vec![p.clone()],
        vec![staged_entry("a.rs")],  // initial: Full -> collapsed
        vec![p],                     // external edit: unstaged diff present
        vec![partial_entry("a.rs")], // now MM -> Partial
    );
    assert!(app.view.is_collapsed("a.rs"));
    app.refresh(); // picks up the external edit
    assert!(!app.view.is_collapsed("a.rs")); // re-expanded
    assert_eq!(
        app.staged_states.get("a.rs").copied(),
        Some(StagedState::Partial)
    );
    let header = app.view.header_row_of_file[0];
    let Row::FileHeader { staged_marker, .. } = &app.view.rows[header] else {
        panic!("expected file header");
    };
    assert_eq!(*staged_marker, StagedMarker::Partial); // ±
}

#[test]
fn refresh_keeps_a_still_fully_staged_collapsed_file_collapsed() {
    let p = raw_patch("a.rs", 1);
    let (mut app, _calls, _diff, _status) = app_with_mutable_fake(
        vec![p],
        vec![staged_entry("a.rs")], // Full -> collapsed
        vec![],                     // still nothing unstaged
        vec![staged_entry("a.rs")], // still Full
    );
    assert!(app.view.is_collapsed("a.rs"));
    app.refresh();
    assert!(app.view.is_collapsed("a.rs")); // stays collapsed
}

#[test]
fn refresh_preserves_a_manually_collapsed_unstaged_file() {
    // A file the reviewer collapsed with `za` that has only unstaged
    // changes keeps its collapse state (it isn't partially staged).
    let a = raw_patch("a.rs", 1);
    let b = raw_patch("b.rs", 1);
    let (mut app, _calls, _diff, _status) = app_with_mutable_fake(
        vec![a.clone(), b.clone()],
        vec![],     // nothing staged
        vec![a, b], // refresh returns the same two files
        vec![],
    );
    app.view.set_collapsed("a.rs", true);
    app.rebuild_rows();
    app.refresh();
    assert!(app.view.is_collapsed("a.rs")); // survives refresh
    assert!(!app.view.is_collapsed("b.rs"));
}

#[test]
fn refresh_drops_collapse_entries_for_departed_files() {
    // b.rs leaves the review on refresh; its collapse-map entry must be
    // dropped rather than lingering as stale state.
    let a = raw_patch("a.rs", 1);
    let b = raw_patch("b.rs", 1);
    let (mut app, _calls, _diff, _status) = app_with_mutable_fake(
        vec![a.clone(), b.clone()],
        vec![],
        vec![a], // b.rs gone from the refresh diff
        vec![],
    );
    app.view.set_collapsed("b.rs", true);
    app.rebuild_rows();
    app.refresh();
    assert!(!app.view.collapse_contains("b.rs")); // entry cleaned up
}

// -- Auto-refresh (working-tree polling) --------------------------------

#[test]
fn auto_refresh_applies_an_external_edit_and_reports_the_change() {
    // The working tree gains a second hunk between polls (an agent edited
    // the file): auto_refresh picks it up and reports that it applied.
    let (mut app, _calls, diff_h, _status_h) = app_with_mutable_fake(
        vec![raw_patch("a.rs", 1)],
        vec![],
        vec![raw_patch("a.rs", 1)], // override starts identical to the view
        vec![],
    );
    assert_eq!(app.view.files[0].hunks.len(), 1);

    // Simulate the external edit landing between ticks.
    *diff_h.borrow_mut() = vec![raw_patch("a.rs", 2)];
    assert!(app.auto_refresh(), "changed tree should apply");
    assert_eq!(app.view.files[0].hunks.len(), 2);
}

#[test]
fn auto_refresh_is_a_noop_when_the_tree_is_unchanged() {
    // Nothing changed since the last refresh: auto_refresh must not rebuild
    // (it returns false) so idle polling never disturbs the view.
    let (mut app, _calls, _diff_h, _status_h) = app_with_mutable_fake(
        vec![raw_patch("a.rs", 1)],
        vec![],
        vec![raw_patch("a.rs", 1)], // identical to what's displayed
        vec![],
    );
    assert!(!app.auto_refresh(), "unchanged tree should be a no-op");
    assert_eq!(app.status_message, None);
}

#[test]
fn maybe_auto_refresh_skips_while_a_remote_op_is_in_flight() {
    // A remote op's own completion refreshes; the intermediate tree it
    // produces must not be picked up mid-flight.
    let (mut app, _calls, diff_h, _status_h) = app_with_mutable_fake(
        vec![raw_patch("a.rs", 1)],
        vec![],
        vec![raw_patch("a.rs", 1)],
        vec![],
    );
    *diff_h.borrow_mut() = vec![raw_patch("a.rs", 2)];
    app.remote_op = Some(InFlightRemote {
        id: TaskId(0),
        op: RemoteOp::Fetch,
    });
    app.maybe_auto_refresh();
    assert_eq!(app.view.files[0].hunks.len(), 1, "skipped during remote op");
    // With the op cleared, the same pending edit is picked up.
    app.remote_op = None;
    app.maybe_auto_refresh();
    assert_eq!(app.view.files[0].hunks.len(), 2);
}

#[test]
fn maybe_auto_refresh_skips_on_a_read_only_range_target() {
    // A fixed range never changes under the reviewer, so polling is a
    // no-op — even though the (contrived) fake would return a new diff.
    let (mut app, _calls) = app_with_fake(
        vec![raw_patch("a.rs", 1)],
        DiffTarget::Range("HEAD~1..HEAD".to_string()),
        vec![raw_patch("a.rs", 2)],
        vec![],
    );
    app.maybe_auto_refresh();
    assert_eq!(app.view.files[0].hunks.len(), 1, "range target not polled");
    // The guard is what skips it: a direct auto_refresh still applies.
    assert!(app.auto_refresh());
    assert_eq!(app.view.files[0].hunks.len(), 2);
}

#[test]
fn maybe_auto_refresh_skips_while_composing() {
    // Mid-input the diff must not shift under the user; once back in
    // Normal the pending edit is picked up.
    let (mut app, _calls, diff_h, _status_h) = app_with_mutable_fake(
        vec![raw_patch("a.rs", 1)],
        vec![],
        vec![raw_patch("a.rs", 1)],
        vec![],
    );
    *diff_h.borrow_mut() = vec![raw_patch("a.rs", 2)];
    app.mode = Mode::Compose;
    app.maybe_auto_refresh();
    assert_eq!(app.view.files[0].hunks.len(), 1, "skipped while composing");
    app.mode = Mode::Normal;
    app.maybe_auto_refresh();
    assert_eq!(app.view.files[0].hunks.len(), 2);
}

#[test]
fn manual_refresh_applies_the_edit_and_acknowledges_in_the_footer() {
    // `R` always confirms it ran (even mid-input, where auto-refresh is
    // suppressed) and picks up the external edit.
    let (mut app, _calls, diff_h, _status_h) = app_with_mutable_fake(
        vec![raw_patch("a.rs", 1)],
        vec![],
        vec![raw_patch("a.rs", 1)],
        vec![],
    );
    *diff_h.borrow_mut() = vec![raw_patch("a.rs", 2)];
    app.apply(Action::Refresh);
    assert_eq!(app.view.files[0].hunks.len(), 2);
    assert_eq!(app.status_message.as_deref(), Some("refreshed"));
}

#[test]
fn maybe_auto_refresh_uses_the_sync_fallback_without_an_async_backend() {
    // `FakeGit` isn't `Send`, so it yields no async builder: the poll must
    // fall back to a synchronous rebuild (no task in flight) and apply the
    // edit directly.
    let (mut app, _calls, diff_h, _status_h) = app_with_mutable_fake(
        vec![raw_patch("a.rs", 1)],
        vec![],
        vec![raw_patch("a.rs", 1)],
        vec![],
    );
    *diff_h.borrow_mut() = vec![raw_patch("a.rs", 2)];
    app.maybe_auto_refresh();
    assert!(
        app.refresh_in_flight.is_none(),
        "a non-Send backend must not spawn an async task"
    );
    assert_eq!(app.view.files[0].hunks.len(), 2, "applied synchronously");
}

#[test]
fn a_foreground_refresh_bumps_the_refresh_generation() {
    // The staleness guard depends on every synchronous refresh advancing
    // the generation; a stage refreshes, so it advances too.
    let p = raw_patch("a.rs", 1);
    let (mut app, _calls) =
        app_with_fake(vec![p.clone()], DiffTarget::WorkingTree, vec![p], vec![]);
    let before = app.refresh_generation;
    app.refresh();
    assert!(
        app.refresh_generation > before,
        "refresh must bump the generation"
    );
    let after_refresh = app.refresh_generation;
    app.apply(Action::StageFile);
    assert!(
        app.refresh_generation > after_refresh,
        "a stage refreshes, so it bumps the generation too"
    );
}

/// The async working-tree poll, exercised on real background threads with a
/// `Send` fake (the `Rc`-based `FakeGit` can't cross a thread boundary).
mod async_refresh {
    use super::*;
    use crate::ui::stage_ops::AsyncReviewBuilder;
    use std::time::{Duration, Instant};

    /// A `Send` [`StageOps`] fake so the async refresh can run on a worker
    /// thread. `diff`/`status` read through shared handles a test mutates
    /// to simulate an external edit landing between polls.
    #[derive(Clone)]
    struct SendFake {
        diff: Arc<Mutex<Vec<RawFilePatch>>>,
        status: Arc<Mutex<Vec<FileStatus>>>,
    }

    impl StageOps for SendFake {
        fn diff(&self, _target: &DiffTarget) -> Result<Vec<RawFilePatch>, GitError> {
            Ok(self.diff.lock().unwrap().clone())
        }
        fn status(&self) -> Result<Vec<FileStatus>, GitError> {
            Ok(self.status.lock().unwrap().clone())
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
        fn async_review_builder(&self) -> Option<AsyncReviewBuilder> {
            let me = self.clone();
            Some(Box::new(move |target| build_review(&me, target)))
        }
    }

    fn snapshot_of(patches: &[RawFilePatch], status: &[FileStatus]) -> ReviewSnapshot {
        ReviewSnapshot {
            files: patches
                .iter()
                .map(|p| FileDiff::from_patch(p).unwrap())
                .collect(),
            patches: patches.iter().cloned().map(Some).collect(),
            staged: staged_from_status(status),
            staged_states: staged_states_from_status(status),
        }
    }

    /// An `App` backed by a `Send` fake, plus a clone of that fake sharing
    /// the same `diff`/`status` handles — a test mutates them to stage an
    /// external edit the background read then sees.
    fn app_with_send_fake(files: Vec<RawFilePatch>, status: Vec<FileStatus>) -> (App, SendFake) {
        let fake = SendFake {
            diff: Arc::new(Mutex::new(files.clone())),
            status: Arc::new(Mutex::new(status.clone())),
        };
        let snapshot = snapshot_of(&files, &status);
        let app = App::with_git(snapshot, DiffTarget::WorkingTree, Box::new(fake.clone()));
        (app, fake)
    }

    /// Drives `poll_refresh` until the in-flight read drains or a deadline
    /// passes (the worker runs on its own thread).
    fn drain_refresh(app: &mut App) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.refresh_in_flight.is_some() && Instant::now() < deadline {
            app.poll_refresh();
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn async_poll_applies_an_external_edit_off_thread() {
        let (mut app, fake) = app_with_send_fake(vec![raw_patch("a.rs", 1)], vec![]);
        assert_eq!(app.view.files[0].hunks.len(), 1);

        // An agent edits the file: it now has a second hunk.
        *fake.diff.lock().unwrap() = vec![raw_patch("a.rs", 2)];
        app.maybe_auto_refresh(); // spawns the background read
        assert!(
            app.refresh_in_flight.is_some(),
            "the poll should run on a worker, not inline"
        );
        drain_refresh(&mut app);
        assert!(app.refresh_in_flight.is_none());
        assert_eq!(app.view.files[0].hunks.len(), 2);
    }

    #[test]
    fn async_poll_is_single_flight() {
        let (mut app, fake) = app_with_send_fake(vec![raw_patch("a.rs", 1)], vec![]);
        *fake.diff.lock().unwrap() = vec![raw_patch("a.rs", 2)];

        app.maybe_auto_refresh();
        let first = app.refresh_in_flight.expect("first read in flight").id;
        // A second tick before the first drains must not stack another read.
        app.maybe_auto_refresh();
        let second = app.refresh_in_flight.expect("still the first read").id;
        assert_eq!(first, second, "must not spawn a second background read");

        drain_refresh(&mut app);
        assert_eq!(app.view.files[0].hunks.len(), 2);
    }

    #[test]
    fn async_poll_discards_a_snapshot_from_before_a_foreground_refresh() {
        // A background read (2-hunk edit) is in flight when a foreground
        // refresh lands (bumping the generation). The stale snapshot must be
        // dropped rather than clobber the newer state.
        let (mut app, _fake) = app_with_send_fake(vec![raw_patch("a.rs", 3)], vec![]);
        assert_eq!(app.view.files[0].hunks.len(), 3);

        let stale = snapshot_of(&[raw_patch("a.rs", 2)], &[]);
        let id = app.refresh_tasks.spawn(move || Some(stale));
        app.refresh_in_flight = Some(InFlightRefresh {
            id,
            generation: app.refresh_generation,
        });
        // Foreground refresh happens first: it advances the generation.
        app.refresh_generation = app.refresh_generation.wrapping_add(1);

        drain_refresh(&mut app);
        assert!(app.refresh_in_flight.is_none(), "stale read was consumed");
        assert_eq!(
            app.view.files[0].hunks.len(),
            3,
            "stale snapshot must not be applied over newer state"
        );
    }

    #[test]
    fn async_poll_does_not_rebuild_under_an_active_visual_selection() {
        // The read was spawned in Normal, but the user entered Visual before
        // it drained: applying it would rebuild rows under the selection's
        // anchor, so the drain must drop it instead.
        let (mut app, fake) = app_with_send_fake(vec![raw_patch("a.rs", 1)], vec![]);
        *fake.diff.lock().unwrap() = vec![raw_patch("a.rs", 2)];
        app.maybe_auto_refresh();
        assert!(app.refresh_in_flight.is_some());

        app.mode = Mode::Visual { anchor: 0 };
        drain_refresh(&mut app);
        assert!(app.refresh_in_flight.is_none(), "read was consumed");
        assert_eq!(
            app.view.files[0].hunks.len(),
            1,
            "must not rebuild while a Visual selection is active"
        );
    }
}

#[test]
fn nothing_hides_smoke_stage_two_then_edit_one_reexpands() {
    // Drives the Unit-2 smoke via the FakeGit harness: three files,
    // stage two (watch them collapse), then an external edit lands on a
    // staged file; the next refresh re-expands it with `±`.
    let a = raw_patch("a.rs", 1);
    let b = raw_patch("b.rs", 1);
    let c = raw_patch("c.rs", 1);
    let (mut app, calls, diff_h, status_h) = app_with_mutable_fake(
        vec![a.clone(), b.clone(), c.clone()],
        vec![], // nothing staged initially -> all expanded
        // The fake starts reflecting the state after a.rs is staged.
        vec![b.clone(), c.clone()],
        vec![staged_entry("a.rs")],
    );
    // All three expanded to start.
    assert!(!app.view.is_collapsed("a.rs"));
    assert!(!app.view.is_collapsed("b.rs"));
    assert!(!app.view.is_collapsed("c.rs"));

    // Stage a.rs (cursor on its header): it collapses.
    app.apply(Action::StageFile);
    assert!(app.view.is_collapsed("a.rs"));

    // Now stage b.rs. Update the fake to the post-(a,b)-stage state,
    // then move the cursor onto b.rs and stage it.
    *diff_h.borrow_mut() = vec![c.clone()];
    *status_h.borrow_mut() = vec![staged_entry("a.rs"), staged_entry("b.rs")];
    app.view.cursor = app.view.header_row_of_file[app
        .view
        .files
        .iter()
        .position(|f| f.path == "b.rs")
        .unwrap()];
    app.apply(Action::StageFile);
    assert!(app.view.is_collapsed("b.rs"));
    assert!(app.view.is_collapsed("a.rs"));
    // Two S gestures, both whole-file stages.
    assert_eq!(
        *calls.borrow(),
        vec![
            StageCall::StageFile("a.rs".to_string()),
            StageCall::StageFile("b.rs".to_string()),
        ]
    );

    // External edit lands on the staged a.rs: it is now partially staged
    // and reappears in the working diff. A refresh must re-expand it.
    *diff_h.borrow_mut() = vec![a.clone(), c.clone()];
    *status_h.borrow_mut() = vec![partial_entry("a.rs"), staged_entry("b.rs")];
    app.refresh();

    assert!(!app.view.is_collapsed("a.rs")); // re-expanded — nothing hides
    assert!(app.view.is_collapsed("b.rs")); // still fully staged, collapsed
    assert_eq!(
        app.staged_states.get("a.rs").copied(),
        Some(StagedState::Partial)
    );
    let header = app.view.header_row_of_file[app
        .view
        .files
        .iter()
        .position(|f| f.path == "a.rs")
        .unwrap()];
    let Row::FileHeader { staged_marker, .. } = &app.view.rows[header] else {
        panic!("expected file header");
    };
    assert_eq!(*staged_marker, StagedMarker::Partial); // ±
}

// -- Staging panel -------------------------------------------------------

#[test]
fn toggle_staging_panel_opens_with_fresh_status_and_closes() {
    let p = raw_patch("a.rs", 1);
    let (mut app, _calls) = app_with_fake(
        vec![p.clone()],
        DiffTarget::WorkingTree,
        vec![p],
        vec![staged_entry("other.rs")],
    );
    app.apply(Action::ToggleStagingPanel);
    assert_eq!(app.mode, Mode::Staging);
    assert_eq!(app.staged.len(), 1); // re-read from status on open
    app.close_staging();
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn staging_panel_navigation_clamps() {
    let mut app = App::new(vec![file("a.rs", 1)]);
    app.staged = vec![
        StagedFile {
            path: "a.rs".to_string(),
            letter: 'M',
        },
        StagedFile {
            path: "b.rs".to_string(),
            letter: 'A',
        },
    ];
    app.staging_move_down();
    assert_eq!(app.staging_cursor, 1);
    app.staging_move_down();
    assert_eq!(app.staging_cursor, 1); // clamped at last
    app.staging_move_up();
    assert_eq!(app.staging_cursor, 0);
    app.staging_move_up();
    assert_eq!(app.staging_cursor, 0); // clamped at first
}

#[test]
fn staging_panel_unstage_keeps_panel_open_and_clamps_cursor() {
    let p = raw_patch("a.rs", 1);
    // Post-refresh status: only one staged file remains.
    let (mut app, calls) = app_with_fake(
        vec![p.clone()],
        DiffTarget::WorkingTree,
        vec![p],
        vec![staged_entry("a.rs")],
    );
    app.staged = vec![staged_entry_file("a.rs"), staged_entry_file("b.rs")];
    app.mode = Mode::Staging;
    app.staging_cursor = 1; // focus b.rs
    app.unstage_focused_file();
    assert_eq!(
        single_call(&calls),
        StageCall::UnstageFile("b.rs".to_string())
    );
    assert_eq!(app.mode, Mode::Staging); // panel stays open
    assert_eq!(app.staged.len(), 1); // refreshed list
    assert_eq!(app.staging_cursor, 0); // clamped into range
    assert_eq!(app.status_message.as_deref(), Some("unstaged b.rs"));
}

fn staged_entry_file(path: &str) -> StagedFile {
    StagedFile {
        path: path.to_string(),
        letter: 'M',
    }
}

#[test]
fn staging_panel_unstage_on_empty_list_is_a_no_op() {
    let p = raw_patch("a.rs", 1);
    let (mut app, calls) = app_with_fake(vec![p.clone()], DiffTarget::WorkingTree, vec![p], vec![]);
    app.mode = Mode::Staging;
    app.unstage_focused_file();
    assert!(calls.borrow().is_empty());
    assert_eq!(app.mode, Mode::Staging);
}

#[test]
fn visual_space_allows_staging_but_navigation_stays_disabled() {
    let p = raw_patch("a.rs", 1);
    let (mut app, calls) = app_with_fake(vec![p.clone()], DiffTarget::WorkingTree, vec![p], vec![]);
    app.apply(Action::CursorDown);
    app.apply(Action::CursorDown); // line row
    app.apply(Action::EnterVisual);
    app.apply(Action::ToggleStage);
    assert_eq!(calls.borrow().len(), 1);
}

// -- Syntax highlight cache -----------------------------------------------

fn highlight_patch(path: &str) -> RawFilePatch {
    RawFilePatch {
        path: path.to_string(),
        old_path: None,
        raw: format!(
            "diff --git a/{path} b/{path}\nindex 1..2 100644\n--- a/{path}\n+++ b/{path}\n@@ -1,1 +1,1 @@\n-fn old() {{}}\n+fn new() {{}}\n"
        ),
        is_binary: false,
    }
}

/// The multibuffer highlights every *expanded* file's in-use sides once
/// at build time, and the `(path, side)`-keyed cache means later motions
/// (section jumps back and forth) re-fetch nothing.
#[test]
fn multibuffer_highlights_every_expanded_file_once() {
    let a = highlight_patch("a.rs");
    let b = highlight_patch("b.rs");
    let show_calls = Rc::new(RefCell::new(0));
    let fake = FakeGit {
        diff: vec![a.clone(), b.clone()],
        show_calls: Rc::clone(&show_calls),
        show_content: Some("fn old() {}\nfn new() {}\n".to_string()),
        ..FakeGit::default()
    };
    let snapshot = ReviewSnapshot {
        files: vec![
            FileDiff::from_patch(&a).unwrap(),
            FileDiff::from_patch(&b).unwrap(),
        ],
        patches: vec![Some(a), Some(b)],
        staged: Vec::new(),
        staged_states: HashMap::new(),
    };
    let mut app = App::with_git(snapshot, DiffTarget::Staged, Box::new(fake));
    // Both files expanded -> a.rs (2 sides) + b.rs (2 sides) = 4 fetches.
    assert_eq!(*show_calls.borrow(), 4);

    app.apply(Action::NextFile); // -> b.rs header: cache hit
    app.apply(Action::PrevFile); // -> a.rs header: cache hit
    app.apply(Action::NextFile); // -> b.rs header: cache hit
    assert_eq!(*show_calls.borrow(), 4);
}

/// A file that starts collapsed (fully staged at launch) is not
/// highlighted until it is expanded — the lazy-per-file population rule.
#[test]
fn collapsed_file_is_not_highlighted_until_expanded() {
    let a = highlight_patch("a.rs");
    let c = highlight_patch("c.rs");
    let show_calls = Rc::new(RefCell::new(0));
    let fake = FakeGit {
        diff: vec![a.clone(), c.clone()],
        show_calls: Rc::clone(&show_calls),
        show_content: Some("fn old() {}\nfn new() {}\n".to_string()),
        ..FakeGit::default()
    };
    let snapshot = ReviewSnapshot {
        files: vec![
            FileDiff::from_patch(&a).unwrap(),
            FileDiff::from_patch(&c).unwrap(),
        ],
        patches: vec![Some(a), Some(c)],
        // c.rs starts fully staged -> starts collapsed.
        staged: vec![StagedFile {
            path: "c.rs".to_string(),
            letter: 'M',
        }],
        staged_states: HashMap::from([("c.rs".to_string(), StagedState::Full)]),
    };
    let mut app = App::with_git(snapshot, DiffTarget::Staged, Box::new(fake));
    // Only a.rs (expanded) is highlighted; collapsed c.rs is skipped.
    assert_eq!(*show_calls.borrow(), 2);

    // Expanding c.rs highlights its two sides on the rebuild.
    app.view.set_collapsed("c.rs", false);
    app.rebuild_rows();
    assert_eq!(*show_calls.borrow(), 4);
}

/// Builds an `App` (Staged target) whose fake reads its refresh diff/status
/// through mutable handles and counts `show_file` fetches, so a refresh
/// test can change what the next `refresh` sees and observe whether the
/// highlight cache was reused or re-fetched. Returns the app, the diff
/// handle, and the show-call counter.
#[allow(clippy::type_complexity)]
fn app_with_counting_fake(
    initial: Vec<RawFilePatch>,
) -> (App, Rc<RefCell<Vec<RawFilePatch>>>, Rc<RefCell<usize>>) {
    let show_calls = Rc::new(RefCell::new(0));
    let diff_h = Rc::new(RefCell::new(initial.clone()));
    let status_h = Rc::new(RefCell::new(Vec::new()));
    let fake = FakeGit {
        diff_override: Some(Rc::clone(&diff_h)),
        status_override: Some(Rc::clone(&status_h)),
        show_calls: Rc::clone(&show_calls),
        show_content: Some("fn old() {}\nfn new() {}\n".to_string()),
        ..FakeGit::default()
    };
    let snapshot = ReviewSnapshot {
        files: initial
            .iter()
            .map(|p| FileDiff::from_patch(p).unwrap())
            .collect(),
        patches: initial.into_iter().map(Some).collect(),
        staged: Vec::new(),
        staged_states: HashMap::new(),
    };
    let app = App::with_git(snapshot, DiffTarget::Staged, Box::new(fake));
    (app, diff_h, show_calls)
}

#[test]
fn refresh_preserves_highlight_cache_for_unchanged_files() {
    let a = highlight_patch("a.rs");
    let (mut app, _diff, show_calls) = app_with_counting_fake(vec![a]);
    // a.rs expanded, both sides -> 2 fetches at build.
    assert_eq!(*show_calls.borrow(), 2);
    assert!(app.highlight_cache_contains("a.rs", Side::New));

    // The refresh sees byte-identical diff content -> the cache survives
    // and nothing is re-fetched.
    app.refresh();
    assert_eq!(*show_calls.borrow(), 2);
    assert!(app.highlight_cache_contains("a.rs", Side::New));
    assert!(app.highlight_cache_contains("a.rs", Side::Old));
}

#[test]
fn refresh_invalidates_highlight_cache_for_changed_files() {
    let a = highlight_patch("a.rs");
    let (mut app, diff, show_calls) = app_with_counting_fake(vec![a]);
    assert_eq!(*show_calls.borrow(), 2);

    // The file's diff content changes underneath us (an external edit):
    // its cache entry must be invalidated and both sides re-fetched.
    *diff.borrow_mut() = vec![raw_patch("a.rs", 1)];
    app.refresh();
    assert_eq!(*show_calls.borrow(), 4);
}

#[test]
fn refresh_drops_highlight_cache_entries_for_removed_files() {
    let a = highlight_patch("a.rs");
    let b = highlight_patch("b.rs");
    let (mut app, diff, show_calls) = app_with_counting_fake(vec![a, b]);
    // Two expanded files, two sides each -> 4 fetches.
    assert_eq!(*show_calls.borrow(), 4);
    assert!(app.highlight_cache_contains("b.rs", Side::New));

    // b.rs leaves the review; its cache entries must be dropped (no
    // unbounded growth), while a.rs (unchanged) keeps its cached spans.
    *diff.borrow_mut() = vec![highlight_patch("a.rs")];
    app.refresh();
    assert!(app.highlight_cache_contains("a.rs", Side::New));
    assert!(!app.highlight_cache_contains("b.rs", Side::New));
    assert!(!app.highlight_cache_contains("b.rs", Side::Old));
    // a.rs was a cache hit -> no further fetches.
    assert_eq!(*show_calls.borrow(), 4);
    assert_eq!(app.highlight_cache_len(), 2);
}

/// A file whose single hunk carries `pairs` removed/added line pairs
/// (`2 * pairs` changed lines) of realistic Rust, so the word-diff pairing
/// (the dominant per-rebuild cost) runs on non-trivial content.
fn perf_file(i: usize, pairs: usize) -> FileDiff {
    let path = format!("src/module_{i}.rs");
    let mut raw = format!(
        "diff --git a/{path} b/{path}\nindex 1..2 100644\n--- a/{path}\n+++ b/{path}\n@@ -1,{pairs} +1,{pairs} @@\n"
    );
    for k in 0..pairs {
        raw.push_str(&format!(
                "-    let value_{k} = compute_old({k}, factor);\n+    let value_{k} = compute_new({k}, factor);\n"
            ));
    }
    FileDiff::from_patch(&RawFilePatch {
        path,
        old_path: None,
        raw,
        is_binary: false,
    })
    .unwrap()
}

/// A full `rebuild_rows` over a ~5k-changed-line, 15-file multibuffer —
/// the exact work every stage/collapse gesture triggers — must be
/// imperceptible. Measured well under 10ms (recorded in the perf proof),
/// so incremental rebuild is unnecessary. The assertion uses a generous
/// CI-safe bound to stay non-flaky; run with `--nocapture` for the number.
#[test]
fn rebuild_rows_on_a_5k_line_multibuffer_is_fast() {
    let files: Vec<FileDiff> = (0..15).map(|i| perf_file(i, 168)).collect();
    let total_lines: usize = files
        .iter()
        .flat_map(|f| f.hunks.iter())
        .map(|h| h.lines.len())
        .sum();
    assert!(
        total_lines >= 5000,
        "fixture should be ~5k changed lines, got {total_lines}"
    );

    let mut app = App::new(files);
    let rows = app.view.rows.len();
    // Warm up once, then average a handful of full rebuilds.
    app.rebuild_rows();
    let iters = 20;
    let start = std::time::Instant::now();
    for _ in 0..iters {
        app.rebuild_rows();
    }
    let per = start.elapsed() / iters;
    println!(
        "rebuild_rows: {per:?} avg over {iters} rebuilds, {rows} rows ({total_lines} changed lines)"
    );
    assert!(
        per < std::time::Duration::from_millis(250),
        "rebuild_rows took {per:?} for {total_lines} lines / {rows} rows"
    );
}

// -- Search ----------------------------------------------------------------

fn search_file() -> FileDiff {
    let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,3 +1,3 @@ fn foo() {
 alpha
-beta
+gamma
 delta
";
    file_with_raw("f.rs", raw)
}

#[test]
fn slash_opens_search_mode_with_empty_buffer() {
    let mut app = App::new(vec![search_file()]);
    app.apply(Action::Search);
    assert_eq!(app.mode, Mode::Search);
    assert_eq!(app.search_input, "");
}

#[test]
fn slash_is_disabled_in_visual_mode() {
    let mut app = App::new(vec![search_file()]);
    app.apply(Action::CursorDown); // hunk header
    app.apply(Action::CursorDown); // line row
    app.apply(Action::EnterVisual);
    app.apply(Action::Search);
    assert!(matches!(app.mode, Mode::Visual { .. }));
}

#[test]
fn confirm_search_jumps_to_first_match_at_or_after_cursor() {
    let mut app = App::new(vec![search_file()]);
    app.apply(Action::Search);
    for c in "gamma".chars() {
        app.search_input.push(c);
    }
    app.confirm_search();
    assert_eq!(app.mode, Mode::Normal);
    let Row::Line(line) = &app.view.rows[app.view.cursor] else {
        panic!("expected cursor on a line row");
    };
    assert_eq!(line.content, "gamma");
    assert_eq!(app.status_message.as_deref(), Some("match 1/1"));
}

#[test]
fn confirm_search_with_no_matches_sets_message_but_keeps_pattern() {
    let mut app = App::new(vec![search_file()]);
    app.apply(Action::Search);
    for c in "zzz".chars() {
        app.search_input.push(c);
    }
    app.confirm_search();
    assert_eq!(app.status_message.as_deref(), Some("no matches"));
    assert_eq!(app.search.pattern.as_deref(), Some("zzz"));
}

#[test]
fn confirm_search_with_empty_buffer_clears_active_pattern() {
    let mut app = App::new(vec![search_file()]);
    app.search.pattern = Some("gamma".to_string());
    app.search.matches = vec![4];
    app.apply(Action::Search); // buffer starts empty
    app.confirm_search();
    assert_eq!(app.search.pattern, None);
    assert!(app.search.matches.is_empty());
}

#[test]
fn esc_with_nonempty_buffer_cancels_without_clearing_active_pattern() {
    let mut app = App::new(vec![search_file()]);
    app.apply(Action::Search);
    for c in "gamma".chars() {
        app.search_input.push(c);
    }
    app.confirm_search(); // active pattern is now "gamma"
    app.apply(Action::Search); // reopen with a fresh buffer
    app.search_input.push('x');
    app.cancel_search();
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.search.pattern.as_deref(), Some("gamma"));
}

#[test]
fn esc_with_empty_buffer_clears_active_pattern() {
    let mut app = App::new(vec![search_file()]);
    app.search.pattern = Some("gamma".to_string());
    app.search.matches = vec![4];
    app.apply(Action::Search);
    app.cancel_search();
    assert_eq!(app.search.pattern, None);
    assert!(app.search.matches.is_empty());
}

#[test]
fn search_next_and_prev_wrap_around_both_directions() {
    let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,3 +1,3 @@
 foo
 foo
 foo
";
    let mut app = App::new(vec![file_with_raw("f.rs", raw)]);
    app.apply(Action::Search);
    for c in "foo".chars() {
        app.search_input.push(c);
    }
    app.confirm_search();
    let first = app.view.cursor;

    app.apply(Action::SearchNext);
    let second = app.view.cursor;
    assert_ne!(first, second);

    app.apply(Action::SearchNext);
    let third = app.view.cursor;
    assert_ne!(second, third);

    app.apply(Action::SearchNext); // wraps forward back to the first match
    assert_eq!(app.view.cursor, first);

    app.apply(Action::SearchPrev); // wraps backward to the last match
    assert_eq!(app.view.cursor, third);
}

#[test]
fn search_next_without_active_pattern_sets_message() {
    let mut app = App::new(vec![search_file()]);
    app.apply(Action::SearchNext);
    assert_eq!(app.status_message.as_deref(), Some("no search pattern"));
}

#[test]
fn search_next_with_zero_matches_sets_message() {
    let mut app = App::new(vec![search_file()]);
    app.apply(Action::Search);
    for c in "zzz".chars() {
        app.search_input.push(c);
    }
    app.confirm_search();
    app.apply(Action::SearchNext);
    assert_eq!(app.status_message.as_deref(), Some("no matches"));
}

#[test]
fn search_matches_are_smartcase() {
    let mut app = App::new(vec![search_file()]);
    app.apply(Action::Search);
    for c in "GAMMA".chars() {
        app.search_input.push(c);
    }
    app.confirm_search();
    // "gamma" (lowercase in the file) does not match the uppercase,
    // case-sensitive pattern "GAMMA".
    assert_eq!(app.status_message.as_deref(), Some("no matches"));
}

#[test]
fn hunk_header_section_text_is_searchable() {
    let mut app = App::new(vec![search_file()]);
    app.apply(Action::Search);
    for c in "foo".chars() {
        app.search_input.push(c);
    }
    app.confirm_search();
    assert!(matches!(
        app.view.rows[app.view.cursor],
        Row::HunkHeader { .. }
    ));
}

#[test]
fn search_pattern_survives_row_rebuild() {
    let mut app = App::new(vec![search_file()]);
    app.apply(Action::Search);
    for c in "gamma".chars() {
        app.search_input.push(c);
    }
    app.confirm_search();
    assert_eq!(app.search.matches.len(), 1);

    // Adding an annotation triggers `refresh_rows` -> `rebuild_rows`,
    // which must recompute matches against the rebuilt rows rather
    // than dropping the active pattern.
    app.apply(Action::Compose);
    for c in "note".chars() {
        app.compose.as_mut().unwrap().buffer.insert_char(c);
    }
    app.submit_compose();

    assert_eq!(app.search.pattern.as_deref(), Some("gamma"));
    assert_eq!(app.search.matches.len(), 1);
}

// -- Search across the multibuffer (task 4.2) ---------------------------

/// A one-hunk file whose added line contains the pattern `needle`.
fn needle_file(path: &str) -> FileDiff {
    let raw = format!(
        "diff --git a/{path} b/{path}\nindex 1..2 100644\n--- a/{path}\n+++ b/{path}\n@@ -1,1 +1,1 @@\n-old\n+needle here\n"
    );
    file_with_raw(path, &raw)
}

fn confirm_needle_search(app: &mut App) {
    app.apply(Action::Search);
    for c in "needle".chars() {
        app.search_input.push(c);
    }
    app.confirm_search();
}

#[test]
fn search_matches_span_file_boundaries() {
    let mut app = App::new(vec![needle_file("a.rs"), needle_file("b.rs")]);
    confirm_needle_search(&mut app);
    // One match per file's section — the search spans the whole buffer.
    assert_eq!(app.search.matches.len(), 2);
    let f0 = app.view.file_of_row[app.search.matches[0]];
    let f1 = app.view.file_of_row[app.search.matches[1]];
    assert_ne!(f0, f1);
}

#[test]
fn collapsed_section_contributes_no_search_matches() {
    let mut app = App::new(vec![needle_file("a.rs"), needle_file("b.rs")]);
    // a.rs collapsed contributes only its header — its needle row is
    // absent from the buffer, so it cannot match.
    app.view.set_collapsed("a.rs", true);
    app.rebuild_rows();
    confirm_needle_search(&mut app);
    assert_eq!(app.search.matches.len(), 1);
    assert_eq!(app.view.file_of_row[app.search.matches[0]], 1);
}

#[test]
fn search_next_wraps_across_the_whole_buffer() {
    let mut app = App::new(vec![needle_file("a.rs"), needle_file("b.rs")]);
    confirm_needle_search(&mut app);
    let first = app.view.cursor;
    assert_eq!(app.view.file_of_cursor(), 0); // first match in a.rs

    app.apply(Action::SearchNext); // -> b.rs's match
    let second = app.view.cursor;
    assert_ne!(first, second);
    assert_eq!(app.view.file_of_cursor(), 1);

    app.apply(Action::SearchNext); // wraps back to a.rs's match
    assert_eq!(app.view.cursor, first);

    app.apply(Action::SearchPrev); // wraps backward to b.rs's match
    assert_eq!(app.view.cursor, second);
}

#[test]
fn toggling_collapse_recomputes_search_matches_without_stale_indices() {
    let mut app = App::new(vec![needle_file("a.rs"), needle_file("b.rs")]);
    confirm_needle_search(&mut app);
    assert_eq!(app.search.matches.len(), 2);

    // Collapse b.rs via the `za` action; its rows leave the buffer, so
    // the match set must recompute (no stale row indices survive).
    app.apply(Action::NextFile); // cursor onto b.rs's header
    assert_eq!(app.view.file_of_cursor(), 1);
    app.apply(Action::ToggleCollapse);
    assert!(app.view.is_collapsed("b.rs"));

    assert_eq!(app.search.matches.len(), 1);
    for &m in &app.search.matches {
        assert!(m < app.view.rows.len(), "stale match index {m}");
        let Row::Line(l) = &app.view.rows[m] else {
            panic!("match row is not a line row");
        };
        assert!(l.content.contains("needle"));
    }
}

// -- Select-by-path seam (task 4.4) -------------------------------------

#[test]
fn select_file_by_path_moves_cursor_to_section_header() {
    let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
    assert!(app.select_file_by_path("b.rs"));
    assert_eq!(app.view.cursor, app.view.header_row_of_file[1]);
    assert_eq!(app.view.selected_file, 1);
    assert!(matches!(
        app.view.rows[app.view.cursor],
        Row::FileHeader { file_index: 1, .. }
    ));
}

#[test]
fn select_file_by_path_expands_a_collapsed_target() {
    let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
    app.view.set_collapsed("b.rs", true);
    app.rebuild_rows();
    assert!(app.view.is_collapsed("b.rs"));

    assert!(app.select_file_by_path("b.rs"));
    assert!(!app.view.is_collapsed("b.rs")); // expanded on select
    assert_eq!(app.view.cursor, app.view.header_row_of_file[1]);
    assert_eq!(app.view.selected_file, 1);
}

#[test]
fn select_file_by_path_unknown_path_is_a_noop_returning_false() {
    let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
    app.apply(Action::CursorDown);
    let cursor_before = app.view.cursor;
    assert!(!app.select_file_by_path("missing.rs"));
    assert_eq!(app.view.cursor, cursor_before);
    assert_eq!(app.view.selected_file, 0);
}

// -- Column cursor ---------------------------------------------------------

#[test]
fn h_and_l_move_column_within_bounds() {
    let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,1 +1,1 @@
 abcde
";
    let mut app = App::new(vec![file_with_raw("f.rs", raw)]);
    app.apply(Action::CursorDown); // hunk header
    app.apply(Action::CursorDown); // line "abcde"
    assert_eq!(app.view.effective_column(), Some(0));
    app.apply(Action::CursorRight);
    app.apply(Action::CursorRight);
    assert_eq!(app.view.effective_column(), Some(2));
    for _ in 0..10 {
        app.apply(Action::CursorRight);
    }
    assert_eq!(app.view.effective_column(), Some(4)); // clamped at last char
    for _ in 0..10 {
        app.apply(Action::CursorLeft);
    }
    assert_eq!(app.view.effective_column(), Some(0));
}

#[test]
fn column_is_hidden_on_header_rows() {
    let app = App::new(vec![file("a.rs", 1)]);
    assert!(matches!(app.view.rows[0], Row::FileHeader { .. }));
    assert_eq!(app.view.effective_column(), None);
}

#[test]
fn w_and_b_jump_between_words() {
    let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,1 +1,1 @@
 foo bar_baz  qux
";
    let mut app = App::new(vec![file_with_raw("f.rs", raw)]);
    app.apply(Action::CursorDown);
    app.apply(Action::CursorDown);
    assert_eq!(app.view.effective_column(), Some(0)); // 'f' of foo
    app.apply(Action::WordForward);
    assert_eq!(app.view.effective_column(), Some(4)); // 'b' of bar_baz (word = alnum/_)
    app.apply(Action::WordForward);
    assert_eq!(app.view.effective_column(), Some(13)); // 'q' of qux
    app.apply(Action::WordBackward);
    assert_eq!(app.view.effective_column(), Some(4));
    app.apply(Action::WordBackward);
    assert_eq!(app.view.effective_column(), Some(0));
}

#[test]
fn column_clamps_when_switching_to_a_shorter_row() {
    let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,2 +1,2 @@
 a really long line here
 x
";
    let long_line_last_col = "a really long line here".chars().count() - 1;
    let mut app = App::new(vec![file_with_raw("f.rs", raw)]);
    app.apply(Action::CursorDown); // hunk header
    app.apply(Action::CursorDown); // long line
    for _ in 0..40 {
        app.apply(Action::CursorRight);
    }
    assert_eq!(app.view.effective_column(), Some(long_line_last_col));
    app.apply(Action::CursorDown); // short line "x"
    assert_eq!(app.view.effective_column(), Some(0));
}

#[test]
fn column_motion_is_a_noop_off_a_line_row() {
    let mut app = App::new(vec![file("a.rs", 1)]);
    app.apply(Action::CursorRight);
    app.apply(Action::WordForward);
    assert_eq!(app.view.effective_column(), None);
}

// -- Git panel focus & navigation --------------------------------------

/// Builds a panel fixture: two tracked files, one untracked, two stashes.
fn panel_app() -> App {
    let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1), file("notes.md", 1)]);
    app.untracked_paths = vec!["notes.md".to_string()];
    app.stashes = vec![
        StashEntry {
            stash_ref: "stash@{0}".to_string(),
            branch: Some("main".to_string()),
            message: "wip".to_string(),
        },
        StashEntry {
            stash_ref: "stash@{1}".to_string(),
            branch: Some("main".to_string()),
            message: "spike".to_string(),
        },
    ];
    app
}

#[test]
fn focus_git_panel_toggles_mode_both_ways() {
    let mut app = panel_app();
    assert_eq!(app.mode, Mode::Normal);
    assert!(!app.git_panel_focused());
    app.apply(Action::FocusGitPanel);
    assert_eq!(app.mode, Mode::Panel { cursor: 0 });
    assert!(app.git_panel_focused());
    app.apply(Action::FocusGitPanel);
    assert_eq!(app.mode, Mode::Normal);
    assert!(!app.git_panel_focused());
}

#[test]
fn focusing_the_panel_resets_its_cursor_to_top() {
    let mut app = panel_app();
    // The cursor lives in `Mode::Panel`, so entering panel mode always starts
    // it at the top: focus, move it off the top, unfocus, then refocus.
    app.apply(Action::FocusGitPanel);
    app.apply(Action::PanelCursorDown);
    app.apply(Action::PanelCursorDown);
    assert_eq!(app.panel_cursor(), 2);
    app.apply(Action::FocusGitPanel); // unfocus
    app.apply(Action::FocusGitPanel); // refocus
    assert_eq!(app.panel_cursor(), 0);
}

#[test]
fn panel_cursor_moves_and_clamps_across_sections() {
    let mut app = panel_app();
    app.apply(Action::FocusGitPanel);
    // 5 navigable rows: a.rs, b.rs, notes.md, stash0, stash1.
    assert_eq!(app.panel_cursor(), 0);
    app.apply(Action::PanelCursorUp); // clamps at top
    assert_eq!(app.panel_cursor(), 0);
    for _ in 0..10 {
        app.apply(Action::PanelCursorDown);
    }
    assert_eq!(app.panel_cursor(), 4); // clamps at bottom (last stash)
    app.apply(Action::PanelCursorUp);
    assert_eq!(app.panel_cursor(), 3);
}

#[test]
fn panel_enter_on_file_selects_it_and_returns_focus_to_diff() {
    let mut app = panel_app();
    app.apply(Action::FocusGitPanel);
    app.apply(Action::PanelCursorDown); // -> b.rs (index 1)
    app.apply(Action::PanelSelect);
    assert_eq!(app.view.selected_file, 1);
    assert_eq!(app.mode, Mode::Normal); // focus returned to the diff
    // Selecting scrolls the multibuffer to that file's section header.
    assert_eq!(app.view.cursor, app.view.header_row_of_file[1]);
}

#[test]
fn panel_enter_on_untracked_file_selects_it() {
    let mut app = panel_app();
    app.apply(Action::FocusGitPanel);
    app.mode = Mode::Panel { cursor: 2 }; // notes.md, the lone UNTRACKED row
    app.apply(Action::PanelSelect);
    assert_eq!(app.view.selected_file, 2);
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn panel_enter_on_stash_is_a_no_op_and_stays_focused() {
    let mut app = panel_app();
    app.apply(Action::FocusGitPanel);
    app.mode = Mode::Panel { cursor: 3 }; // first stash row
    let selected_before = app.view.selected_file;
    app.apply(Action::PanelSelect);
    assert_eq!(app.view.selected_file, selected_before); // unchanged
    assert_eq!(app.mode, Mode::Panel { cursor: 3 }); // still focused on the panel
}

#[test]
fn focus_toggle_is_a_no_op_while_a_modal_owns_the_keyboard() {
    let mut app = panel_app();
    app.mode = Mode::Search;
    app.apply(Action::FocusGitPanel);
    assert_eq!(app.mode, Mode::Search); // unchanged: Search still owns keys
}
