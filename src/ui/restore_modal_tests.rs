//! Tests for the restore confirm modal's rendering
//! (`src/ui/restore_modal.rs`).
//!
//! The wording is the safety boundary here, so it's asserted rather than
//! eyeballed: the modal must name the exact file, and must say plainly which
//! of the two destructive outcomes confirming produces.

use super::*;
use crate::diff::FileDiff;
use crate::git::{DiffTarget, RawFilePatch, RestoreScope};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use crate::ui::app::ModeOrigin;
use crate::ui::restore::RestoreRequest;

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

fn render_modal(app: &App) -> String {
    let backend = TestBackend::new(70, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let area = Rect::new(0, 0, 70, 24);
    terminal.draw(|frame| render(frame, area, app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    buffer.content().iter().map(|c| c.symbol()).collect()
}

/// An app with the modal open on `path`, tracked or not.
fn modal_app(path: &str, untracked: bool) -> App {
    request_app(RestoreRequest {
        path: path.to_string(),
        old_path: None,
        untracked,
        scope: RestoreScope::IndexAndWorktree,
    })
}

/// An app with the modal open on an arbitrary pending request.
fn request_app(request: RestoreRequest) -> App {
    let mut app = App::new(vec![sample_file()]);
    app.target = DiffTarget::WorkingTree;
    app.restore_request = Some(request);
    app.mode = Mode::ConfirmRestore {
        origin: ModeOrigin::Normal,
    };
    app
}

#[test]
fn tracked_file_asks_to_restore_and_names_the_file() {
    let content = render_modal(&modal_app("src/main.rs", false));
    assert!(content.contains("Restore src/main.rs?"), "{content}");
}

#[test]
fn tracked_file_spells_out_that_both_halves_go_and_there_is_no_undo() {
    let content = render_modal(&modal_app("src/main.rs", false));
    assert!(content.contains("staged and unstaged"), "{content}");
    assert!(content.contains("No undo"), "{content}");
}

#[test]
fn untracked_file_says_delete_not_restore() {
    let content = render_modal(&modal_app("src/fresh.rs", true));
    assert!(content.contains("Delete src/fresh.rs?"), "{content}");
    assert!(!content.contains("Restore src/fresh.rs?"), "{content}");
    assert!(content.contains("removed, not restored"), "{content}");
}

#[test]
fn both_keys_are_offered() {
    let content = render_modal(&modal_app("src/main.rs", false));
    assert!(content.contains("restore"), "{content}");
    assert!(content.contains("cancel"), "{content}");
}

#[test]
fn untracked_confirm_hint_reads_delete() {
    let content = render_modal(&modal_app("src/fresh.rs", true));
    assert!(content.contains("delete"), "{content}");
}

#[test]
fn render_is_a_no_op_outside_the_modal() {
    let app = App::new(vec![sample_file()]);
    assert_eq!(app.mode, Mode::Normal);
    let content = render_modal(&app);
    assert!(!content.contains("No undo"));
}

#[test]
fn render_is_a_no_op_when_no_request_is_pending() {
    let mut app = modal_app("src/main.rs", false);
    app.restore_request = None;
    let content = render_modal(&app);
    assert!(!content.contains("No undo"));
}

#[test]
fn a_long_path_keeps_its_file_name_visible() {
    let long = "very/deeply/nested/directory/structure/that/keeps/going/target.rs";
    let content = render_modal(&modal_app(long, false));
    // The tail identifies the file being destroyed; it must survive elision.
    assert!(content.contains("target.rs?"), "{content}");
    assert!(content.contains('\u{2026}'), "{content}");
}

// -- `elide_path_left` -----------------------------------------------------

#[test]
fn elide_leaves_a_short_path_untouched() {
    assert_eq!(elide_path_left("src/main.rs", 40), "src/main.rs");
    // Exactly at the budget is still untouched.
    assert_eq!(elide_path_left("src/main.rs", 11), "src/main.rs");
}

#[test]
fn elide_cuts_at_a_component_boundary_when_one_fits() {
    // Budget 13 (14 minus the marker) fits "/ccc/dddd.rs" exactly.
    assert_eq!(
        elide_path_left("a/bb/ccc/dddd.rs", 14),
        "\u{2026}/ccc/dddd.rs"
    );
    // One column tighter drops to the next boundary rather than overflowing.
    assert_eq!(elide_path_left("a/bb/ccc/dddd.rs", 12), "\u{2026}/dddd.rs");
}

#[test]
fn elide_falls_back_to_a_raw_tail_when_no_component_fits() {
    // No `/` boundary leaves a short enough tail, so this cuts mid-component.
    let out = elide_path_left("aaaa/bbbbbbbbbbbbbbbb.rs", 8);
    assert_eq!(out.chars().count(), 8);
    assert!(out.starts_with('\u{2026}'));
    assert!("aaaa/bbbbbbbbbbbbbbbb.rs".ends_with(&out[out.len() - 7..]));
}

#[test]
fn elide_handles_a_zero_budget() {
    assert_eq!(elide_path_left("src/main.rs", 0), "");
}

#[test]
fn elide_never_exceeds_its_budget() {
    let path = "one/two/three/four/five/six/seven/eight/nine/ten.rs";
    for max in 0..=path.chars().count() + 5 {
        assert!(
            elide_path_left(path, max).chars().count() <= max,
            "budget {max} exceeded"
        );
    }
}

// -- Wording for the cases that could otherwise mislead --------------------

#[test]
fn a_rename_says_the_file_comes_back_under_its_old_name() {
    let content = render_modal(&request_app(RestoreRequest {
        path: "src/new.rs".to_string(),
        old_path: Some("src/old.rs".to_string()),
        untracked: false,
        scope: RestoreScope::IndexAndWorktree,
    }));
    assert!(content.contains("Restore src/new.rs?"), "{content}");
    // The reviewer must be told the file reappears under a different name
    // than the header shows — the one outcome they could not predict.
    assert!(content.contains("Renamed from src/old.rs"), "{content}");
    assert!(content.contains("the rename is undone too"), "{content}");
}

#[test]
fn the_staged_view_asks_to_unstage_and_promises_the_working_tree_is_kept() {
    let content = render_modal(&request_app(RestoreRequest {
        path: "src/main.rs".to_string(),
        old_path: None,
        untracked: false,
        scope: RestoreScope::IndexOnly,
    }));
    assert!(
        content.contains("Unstage all changes to src/main.rs?"),
        "{content}"
    );
    assert!(
        content.contains("Working-tree changes are kept"),
        "{content}"
    );
    // It must NOT claim to discard everything: it doesn't.
    assert!(!content.contains("No undo"), "{content}");
    assert!(content.contains("unstage"), "{content}");
}
