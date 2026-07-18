use std::path::PathBuf;

use super::*;
use crate::annotate::{Classification, Target};
use crate::config::SidebarSide;
use crate::diff::FileDiff;
use crate::git::{CommitLogEntry, DiffTarget, RawFilePatch, RemoteOp};
use crate::highlight::TokenKind;
use crate::lsp::SourceLocation;
use crate::review::ReviewStatus;
use crate::ui::app::{ModeOrigin, PanelTab};
use crate::ui::review_launcher::LauncherTab;
use crossterm::event::KeyModifiers;
use ratatui::backend::TestBackend;

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

/// Unconfigured (`None`), `sidebar_width` is 30% of the containing area,
/// clamped to `[40, 72]` — pure arithmetic, no terminal involved. Narrow
/// terminals (`80` included) sit on the floor of 40; the rest walk the
/// proportional band and both clamps, including the boundary just below/at
/// the point where 30% first exceeds the floor (`136` -> `40` still
/// clamped, `137` -> `41` unclamped) and the point where it first reaches
/// the cap (`239` -> `71`, `240` -> `72`). This is the "unset preserves
/// today's formula exactly" contract — identical table, identical inputs,
/// to the pre-config behavior.
#[test]
fn sidebar_width_matches_ratified_table_when_unconfigured() {
    let cases: &[(u16, u16)] = &[
        (0, 40),
        (80, 40),
        (120, 40),
        (136, 40),
        (137, 41),
        (160, 48),
        (200, 60),
        (239, 71),
        (240, 72),
        (300, 72),
        (65535, 72),
    ];
    for &(total, expected) in cases {
        assert_eq!(
            sidebar_width(total, None),
            expected,
            "sidebar_width({total}, None) should be {expected}"
        );
    }
}

/// A configured width overrides the formula entirely, at any terminal size
/// that has room for it.
#[test]
fn sidebar_width_configured_overrides_the_formula() {
    assert_eq!(sidebar_width(200, Some(55)), 55);
    assert_eq!(sidebar_width(80, Some(20)), 20);
}

/// A configured width wider than the terminal is clamped to the terminal's
/// actual width at render time (the FR's "clamped to available space"),
/// rather than overflowing the split.
#[test]
fn sidebar_width_configured_clamps_to_available_terminal_width() {
    assert_eq!(sidebar_width(50, Some(72)), 50);
    assert_eq!(sidebar_width(0, Some(40)), 0);
}

/// Renders a small `App` to a `TestBackend` and asserts the diff pane shows
/// expected content. No real terminal is touched. The git panel sidebar is
/// hidden by default (see `sidebar_hidden_in_normal_mode_shown_when_panel_focused`
/// below), so it has nothing to assert here.
#[test]
fn renders_diff_pane_content() {
    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let app = App::new(vec![sample_file()]);
    let keymap = Keymap::default_map();

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();

    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    assert!(content.contains("src/main.rs"));
    assert!(content.contains("old()"));
    assert!(content.contains("new()"));
}

/// The git panel sidebar is hidden by default (`Mode::Normal`) — the diff
/// pane gets the full width and none of the sidebar's content (its
/// `[N files]` footer) renders — and appears only once the panel is
/// focused (`Mode::Panel`, entered via the backtick toggle).
#[test]
fn sidebar_hidden_in_normal_mode_shown_when_panel_focused() {
    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    let keymap = Keymap::default_map();

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
    assert!(!content.contains("[1 files]"));

    app.apply(Action::FocusGitPanel);
    assert!(matches!(app.mode, Mode::Panel { .. }));

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
    assert!(content.contains("[1 files]"));
}

fn multi_file(path: &str) -> FileDiff {
    let raw = format!(
        "diff --git a/{path} b/{path}\nindex 111..222 100644\n--- a/{path}\n+++ b/{path}\n@@ -1,1 +1,1 @@\n-old\n+new\n"
    );
    FileDiff::from_patch(&RawFilePatch {
        path: path.to_string(),
        old_path: None,
        raw,
        is_binary: false,
    })
    .unwrap()
}

/// The multibuffer renders every file's section header (expanded, ▾
/// indicator) with its kind letter and path, all in one buffer.
#[test]
fn multibuffer_renders_all_section_headers() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let app = App::new(vec![multi_file("a.rs"), multi_file("b.rs")]);
    let keymap = Keymap::default_map();

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    // Both section headers present, each with the expanded indicator ▾
    // and the change-kind letter M, and both files' bodies visible.
    assert!(content.contains("\u{25be}")); // ▾ expanded indicator
    assert!(content.contains("M a.rs"));
    assert!(content.contains("M b.rs"));
    assert!(content.contains("old"));
}

/// A collapsed section renders exactly one line: its header with the
/// collapsed indicator ▸, and none of its body rows (the `old`/`new`
/// diff lines are hidden).
#[test]
fn collapsed_section_renders_header_only_with_collapsed_indicator() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![multi_file("a.rs")]);
    app.view.set_collapsed("a.rs", true);
    app.rebuild_rows();
    let keymap = Keymap::default_map();

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    // Scoped to the area above the footer strip: the context-sensitive
    // footer (see `footer.rs`) legitimately shows a "fold" hint (`za`), whose
    // text incidentally contains the substring "old" — unrelated to this
    // test's `old`/`new` diff-body fixture, so the whole-screen buffer isn't
    // the right thing to scan here.
    let footer_h = footer::footer_height(buffer.area.width, &app, &keymap, None);
    let content_h = buffer.area.height.saturating_sub(footer_h);
    let content: String = (0..content_h)
        .flat_map(|y| (0..buffer.area.width).map(move |x| (x, y)))
        .filter_map(|(x, y)| buffer.cell((x, y)))
        .map(|cell| cell.symbol())
        .collect();

    assert!(content.contains("\u{25b8}")); // ▸ collapsed indicator
    assert!(content.contains("M a.rs"));
    // Body rows are gone while collapsed.
    assert!(!content.contains("old"));
    assert!(!content.contains("new"));
}

/// A fully-staged file renders the `●` marker slot in its section
/// header; a partially-staged one renders `±`.
#[test]
fn staged_file_section_header_shows_marker() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![multi_file("a.rs")]);
    app.staged_states
        .insert("a.rs".to_string(), stage_ops::StagedState::Full);
    app.rebuild_rows();
    let keymap = Keymap::default_map();

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    assert!(content.contains("M a.rs"));
    assert!(content.contains("\u{25cf}")); // ● staged marker
}

/// A partially-staged file renders the `±` marker in its section header.
#[test]
fn partial_file_section_header_shows_partial_marker() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![multi_file("a.rs")]);
    app.staged_states
        .insert("a.rs".to_string(), stage_ops::StagedState::Partial);
    app.rebuild_rows();
    let keymap = Keymap::default_map();

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    assert!(content.contains("M a.rs"));
    assert!(content.contains("\u{00b1}")); // ± partial-staged marker
}

// -- Empty-diff welcome state -------------------------------------------------

/// Renders `app` and returns the frame's content as one flattened string, the
/// way every render test in this module inspects a `TestBackend` buffer.
fn rendered_content(app: &App, keymap: &Keymap) -> String {
    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| draw(frame, app, keymap, None))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    buffer.content().iter().map(|cell| cell.symbol()).collect()
}

/// An empty working-tree target (the "agent already committed" dead end this
/// spec targets) shows the welcome state: the situation line plus its keyed
/// hints, sourced from the shared keymap table — not the old bare
/// "no changes" placeholder.
#[test]
fn empty_working_tree_target_shows_welcome_state() {
    let app = App::new(vec![]);
    let keymap = Keymap::default_map();
    let content = rendered_content(&app, &keymap);

    assert!(content.contains("No uncommitted changes"));
    // Hints come from the table: FocusGitPanel is bound to `` ` `` in
    // Scope::Diff, ToggleHelp to `?` (resolved through the Global fallback).
    assert!(content.contains("open the git panel"));
    assert!(content.contains("switch to the History tab"));
    assert!(content.contains("open help"));
    assert!(
        !content.contains("no changes"),
        "old placeholder must be gone"
    );
}

/// Every non-working-tree target gets its own situational wording, not the
/// working-tree phrase reused verbatim.
#[test]
fn welcome_state_uses_target_appropriate_wording_per_target() {
    let keymap = Keymap::default_map();

    let mut staged_app = App::new(vec![]);
    staged_app.target = DiffTarget::Staged;
    assert!(rendered_content(&staged_app, &keymap).contains("Nothing staged"));

    let mut range_app = App::new(vec![]);
    range_app.target = DiffTarget::Range("main..HEAD".to_string());
    assert!(
        rendered_content(&range_app, &keymap).contains("Empty diff for main..HEAD"),
        "range wording must name the range as typed"
    );
}

/// The welcome state disappears the moment content arrives — here via the
/// same `apply_snapshot` path auto-refresh uses to fold a fresh
/// `ReviewSnapshot` back into the view (see `refresh.rs`).
#[test]
fn welcome_state_clears_once_a_snapshot_delivers_content() {
    let mut app = App::new(vec![]);
    let keymap = Keymap::default_map();
    assert!(rendered_content(&app, &keymap).contains("No uncommitted changes"));

    app.apply_snapshot(ReviewSnapshot {
        files: vec![sample_file()],
        patches: vec![None],
        staged: Vec::new(),
        staged_states: std::collections::HashMap::new(),
    });

    let content = rendered_content(&app, &keymap);
    assert!(
        !content.contains("No uncommitted changes"),
        "welcome text must clear once the target has content"
    );
    assert!(
        content.contains("src/main.rs"),
        "the delivered file must render"
    );
}

/// Regenerates `05-proofs/05-task-05-welcome-buffer.txt`: a rendered-buffer
/// text capture of the empty working-tree welcome block, standing in for the
/// interactive screenshot proof this sandbox has no controlling TTY to take
/// (see the task's proof artifact for the TTY-deferred manual steps).
/// `cargo test capture_task_05_welcome_buffer -- --ignored`.
#[test]
#[ignore = "writes the task-05 welcome-buffer proof artifact; run explicitly"]
fn capture_task_05_welcome_buffer() {
    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let app = App::new(vec![]);
    let keymap = Keymap::default_map();
    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();

    let buffer = terminal.backend().buffer().clone();
    let mut out = String::new();
    out.push_str(
        "Task 5.0 proof — empty working-tree welcome state, TestBackend(80x20)\n\
         Rendered via the real `draw()` the blocking event loop calls; the\n\
         only difference from a live terminal is the backend (no controlling\n\
         TTY in this sandbox — see 05-task-05-proofs.md's TTY-deferred section).\n\n",
    );
    for y in 0..buffer.area().height {
        let row: String = (0..buffer.area().width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        out.push_str(row.trim_end());
        out.push('\n');
    }

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("docs/specs/05-spec-diff-sources/05-proofs/05-task-05-welcome-buffer.txt");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, out).unwrap();
}

/// The multibuffer renders for a ref-range target exactly as for the
/// working tree — every file's section header and body appear.
#[test]
fn multibuffer_renders_for_a_range_target() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![multi_file("a.rs"), multi_file("b.rs")]);
    app.target = crate::git::DiffTarget::Range("main..HEAD".to_string());
    let keymap = Keymap::default_map();

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    assert!(content.contains("\u{25be}")); // ▾ expanded indicator
    assert!(content.contains("M a.rs"));
    assert!(content.contains("M b.rs"));
    assert!(content.contains("old"));
}

/// On a read-only range target the help overlay omits the inert
/// file/hunk staging gestures, but keeps the still-working staging-panel
/// toggle.
#[test]
fn help_overlay_hides_staging_rows_on_a_range_target() {
    let backend = TestBackend::new(100, 55);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.help.open = true;
    app.target = crate::git::DiffTarget::Range("main..HEAD".to_string());
    let keymap = Keymap::default_map();

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    assert!(content.contains("keybinds"));
    assert!(!content.contains("Stage/unstage file under cursor"));
    assert!(!content.contains("Stage/unstage hunk"));
    // The staging panel toggle still works on any target, so it stays.
    assert!(content.contains("Toggle staging panel"));
}

/// On the working-tree target every staging gesture is listed. A tall
/// terminal avoids the overlay clipping its lower sections (the launcher's
/// own section pushed "Toggle staging panel" out of a shorter viewport).
#[test]
fn help_overlay_shows_staging_rows_on_the_working_tree_target() {
    let backend = TestBackend::new(100, 300);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.help.open = true; // target defaults to WorkingTree
    let keymap = Keymap::default_map();

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    assert!(content.contains("Stage/unstage file under cursor"));
    assert!(content.contains("Stage/unstage hunk"));
    assert!(content.contains("Toggle staging panel"));
}

/// The dual `?` overlay proof: during a review
/// session, the accept/defer rows appear with their review-specific
/// descriptions and the (inapplicable) staging rows are gone — the mirror
/// image of `help_overlay_shows_staging_rows_on_the_working_tree_target`
/// above. Full real render via `draw()`, not a synthetic table check, so
/// this also proves the "Review" group actually reaches the screen.
#[test]
fn help_overlay_shows_review_rows_and_hides_staging_rows_during_a_review_session() {
    let backend = TestBackend::new(100, 65);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.help.open = true;
    app.target = DiffTarget::Review {
        base: "main".to_string(),
        branch: "feature".to_string(),
    };
    let keymap = Keymap::default_map();

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
    if std::env::var_os("REDQUILL_PROOF_DUMP").is_some() {
        let w = buffer.area.width as usize;
        let symbols: Vec<&str> = buffer.content().iter().map(|c| c.symbol()).collect();
        for row in symbols.chunks(w) {
            eprintln!("{}", row.concat());
        }
    }

    assert!(content.contains("keybinds"));
    assert!(content.contains("Review"), "the Review group must render");
    assert!(content.contains("Accept/un-accept file under cursor"));
    assert!(content.contains("Accept file under cursor"));
    assert!(content.contains("Defer/un-defer file under cursor"));
    // Read-only during a review (staging_mode() == ReadOnly), so the
    // staging-specific rows must be gone — mirrors
    // `help_overlay_hides_staging_rows_on_a_range_target`.
    assert!(!content.contains("Stage/unstage file under cursor"));
    assert!(!content.contains("Stage/unstage hunk"));
    // Still works regardless of target, so it stays.
    assert!(content.contains("Toggle staging panel"));
}

/// The mirror image: outside a review session the accept/defer rows are
/// absent entirely (not just inert) — the working-tree overlay from
/// `help_overlay_shows_staging_rows_on_the_working_tree_target` never
/// mentions them.
#[test]
fn help_overlay_hides_review_rows_outside_a_review_session() {
    let backend = TestBackend::new(100, 55);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.help.open = true; // target defaults to WorkingTree
    let keymap = Keymap::default_map();

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    assert!(!content.contains("Accept/un-accept file under cursor"));
    assert!(!content.contains("Accept file under cursor (review sessions)"));
    assert!(!content.contains("Defer/un-defer file under cursor"));
}

#[test]
fn help_overlay_renders_bindings_when_open() {
    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.help.open = true;
    let keymap = Keymap::default_map();

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();

    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
    assert!(content.contains("keybinds"));
    assert!(content.contains("Move cursor down"));
}

/// The help overlay documents the new remote-op and command-log bindings
/// in their scope groups (no hidden features). A tall terminal avoids the
/// overlay clipping its lower sections.
#[test]
fn help_overlay_lists_remote_and_command_log_bindings() {
    let backend = TestBackend::new(100, 300);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.help.open = true;
    // Panel-scope/modal-section content lives on the All keys tab now
    // (This context, the new default, only shows the origin's own scope
    // plus Works everywhere — see `help::this_context_sections`).
    app.help.tab = help::HelpTab::AllKeys;
    let keymap = Keymap::default_map();

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let content: String = terminal
        .backend()
        .buffer()
        .clone()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();

    // Command-log toggle ("Works everywhere" section, Scope::Global).
    assert!(content.contains("Toggle command log pane"));
    // Remote ops (Git panel focused section, panel scope).
    assert!(content.contains("Fetch from remote"));
    assert!(content.contains("Pull from remote"));
    assert!(content.contains("Push to remote"));
    // Switcher-open binding (Git panel focused section, panel scope) and
    // its modal hint section.
    assert!(content.contains("Open branch/worktree switcher"));
    assert!(content.contains("Branch/worktree switcher"));
    assert!(content.contains("Switch to the selected branch/worktree"));
}

/// Every `Scope::Global` binding renders exactly once, under its own
/// "Works everywhere" section, ahead of the per-scope sections — not
/// duplicated once per scope the way it rendered before.
#[test]
fn help_overlay_lists_global_bindings_once_in_a_works_everywhere_section() {
    let backend = TestBackend::new(100, 300);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.help.open = true;
    // This test is about the full reference's fixed section order (Works
    // everywhere first), which is the All keys tab's contract (FR-3); This
    // context (the new default) intentionally orders Works everywhere last.
    app.help.tab = help::HelpTab::AllKeys;
    let keymap = Keymap::default_map();

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|c| c.symbol()).collect();
    if std::env::var_os("REDQUILL_PROOF_DUMP").is_some() {
        let w = buffer.area.width as usize;
        let symbols: Vec<&str> = buffer.content().iter().map(|c| c.symbol()).collect();
        for row in symbols.chunks(w) {
            eprintln!("{}", row.concat());
        }
    }

    assert!(content.contains("Works everywhere"));
    let works_idx = content.find("Works everywhere").unwrap();
    let panels_idx = content.find("Panels").expect("Panels section must render");
    assert!(
        works_idx < panels_idx,
        "the Works everywhere section must render before the per-scope sections"
    );

    // One line per `Scope::Global` binding, in default_map(): `?`/`@`/`!`/
    // `q` each once, `Quit and discard annotations` twice (`Q` and Ctrl-C).
    for (description, expected_count) in [
        ("Toggle help", 1),
        ("Toggle command log pane", 1),
        ("Dismiss config warning notice", 1),
        ("Quit and emit annotations", 1),
        ("Quit and discard annotations", 2),
    ] {
        assert_eq!(
            content.matches(description).count(),
            expected_count,
            "unexpected occurrence count for {description:?}"
        );
    }
}

/// On a terminal too short for the whole binding list, the help overlay
/// caps its height and scrolls: the top frame shows the first sections
/// only, and driving it to the bottom (End, through the real key path)
/// reveals the last section (Project search — the last entry in
/// `help::modal_sections`) while scrolling the first off-screen.
#[test]
fn help_overlay_scrolls_to_reveal_lower_sections() {
    let backend = TestBackend::new(100, 22);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.help.open = true;
    // The modal sections (Project search, last in `help::modal_sections`)
    // only render on the All keys tab; This context (the new default) never
    // includes them.
    app.help.tab = help::HelpTab::AllKeys;
    let keymap = Keymap::default_map();

    // First frame renders the top of the list (and records the viewport
    // height the pager needs). The last section (Project search) is far
    // below.
    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let top: String = terminal
        .backend()
        .buffer()
        .clone()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(top.contains("Move cursor down"));
    assert!(!top.contains("Toggle regex / literal matching"));

    // Jump to the bottom through the real dispatch path, then redraw.
    let mut pending = None;
    let mut pending_count: Option<usize> = None;
    let end = KeyEvent::new(KeyCode::End, KeyModifiers::NONE);
    let _ = dispatch_key(&mut app, &keymap, &mut pending, &mut pending_count, end);
    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let bottom: String = terminal
        .backend()
        .buffer()
        .clone()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(bottom.contains("Toggle regex / literal matching"));
    assert!(!bottom.contains("Move cursor down"));
}

/// The full lazygit-style filter lifecycle, driven through the real
/// dispatch path: `/` starts editing with an empty query, typing extends it,
/// `Enter` locks it in (handing control back to the scroll keys without
/// closing the overlay), a first `Esc` clears the locked filter (still
/// without closing), and only a second `Esc` (now with no filter left)
/// closes help.
#[test]
fn help_filter_enter_locks_and_two_escapes_close() {
    let mut app = App::new(vec![sample_file()]);
    app.help.open = true;
    let keymap = Keymap::default_map();
    let mut pending = None;
    let mut pending_count: Option<usize> = None;

    let slash = KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE);
    dispatch_key(&mut app, &keymap, &mut pending, &mut pending_count, slash);
    assert_eq!(app.help.search, Some((String::new(), true)));

    for c in ['q', 'u', 'i', 't'] {
        let key = KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
        dispatch_key(&mut app, &keymap, &mut pending, &mut pending_count, key);
    }
    assert_eq!(app.help.search, Some(("quit".to_string(), true)));

    let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    dispatch_key(&mut app, &keymap, &mut pending, &mut pending_count, enter);
    assert_eq!(app.help.search, Some(("quit".to_string(), false)));
    assert!(
        app.help.open,
        "locking the filter must not close the overlay"
    );

    let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
    dispatch_key(&mut app, &keymap, &mut pending, &mut pending_count, esc);
    assert_eq!(app.help.search, None, "first Esc clears the locked filter");
    assert!(
        app.help.open,
        "clearing the filter must not close the overlay"
    );

    dispatch_key(&mut app, &keymap, &mut pending, &mut pending_count, esc);
    assert!(
        !app.help.open,
        "second Esc, with no filter left, closes the overlay"
    );
}

// -- Count-prefix dispatch (3j, 10j, 0, 3gg, ...) ---------------------------

/// `sample_file` has 5 addressable rows (0..=4): FileHeader, HunkHeader,
/// context, removed, added.
fn press_digits(
    app: &mut App,
    keymap: &Keymap,
    pending: &mut Option<KeyEvent>,
    pending_count: &mut Option<usize>,
    digits: &str,
) {
    for c in digits.chars() {
        dispatch_key(
            app,
            keymap,
            pending,
            pending_count,
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
        );
    }
}

#[test]
fn count_prefix_repeats_a_motion_n_times() {
    let mut app = App::new(vec![sample_file()]);
    let keymap = Keymap::default_map();
    let mut pending = None;
    let mut pending_count: Option<usize> = None;

    press_digits(&mut app, &keymap, &mut pending, &mut pending_count, "3");
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
    );
    assert_eq!(app.view.cursor, 3, "3j must move the cursor down 3 rows");
    assert_eq!(
        pending_count, None,
        "the count must reset once the motion applies"
    );
}

#[test]
fn count_prefix_accumulates_across_multiple_digits() {
    let mut app = App::new(vec![sample_file()]);
    let keymap = Keymap::default_map();
    let mut pending = None;
    let mut pending_count: Option<usize> = None;

    // "10j" clamps at the last addressable row (4), but proves "1" then "0"
    // combined into ten rather than acting as two separate counts.
    press_digits(&mut app, &keymap, &mut pending, &mut pending_count, "1");
    assert_eq!(pending_count, Some(1));
    press_digits(&mut app, &keymap, &mut pending, &mut pending_count, "0");
    assert_eq!(
        pending_count,
        Some(10),
        "a `0` after a digit continues the count rather than acting as CursorLineStart"
    );
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
    );
    assert_eq!(app.view.cursor, app.view.max_cursor());
}

#[test]
fn bare_zero_moves_column_cursor_to_line_start() {
    let mut app = App::new(vec![sample_file()]);
    let keymap = Keymap::default_map();
    let mut pending = None;
    let mut pending_count: Option<usize> = None;

    // Land on the added line ("    new();"), move right twice, then `0`.
    for _ in 0..2 {
        dispatch_key(
            &mut app,
            &keymap,
            &mut pending,
            &mut pending_count,
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        );
    }
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
    );
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
    );
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
    );
    assert_eq!(app.view.effective_column(), Some(2));

    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char('0'), KeyModifiers::NONE),
    );
    assert_eq!(app.view.effective_column(), Some(0));
    assert_eq!(pending_count, None);
}

#[test]
fn count_is_silently_dropped_for_a_non_repeatable_action() {
    // `gg` (JumpToTop) has no "repeat" meaning; a count typed before it must
    // not panic, double-apply, or leak into the next keypress.
    let mut app = App::new(vec![sample_file()]);
    let keymap = Keymap::default_map();
    let mut pending = None;
    let mut pending_count: Option<usize> = None;

    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
    );
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
    );
    assert_eq!(app.view.cursor, 2);

    press_digits(&mut app, &keymap, &mut pending, &mut pending_count, "3");
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE),
    );
    assert_eq!(
        pending_count,
        Some(3),
        "the count must survive the pending `g` prefix, not be dropped early"
    );
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE),
    );
    assert_eq!(app.view.cursor, 0, "gg still just jumps to the top");
    assert_eq!(pending_count, None, "the count must not leak past gg");

    // The count must not silently reapply to the next keystroke either.
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
    );
    assert_eq!(app.view.cursor, 1, "a single j, not a leaked 3j");
}

#[test]
fn a_non_repeatable_action_applies_exactly_once_despite_a_count() {
    let mut app = App::new(vec![sample_file()]);
    let keymap = Keymap::default_map();
    let mut pending = None;
    let mut pending_count: Option<usize> = None;

    press_digits(&mut app, &keymap, &mut pending, &mut pending_count, "3");
    // Space (ToggleStage) is a toggle, not a motion: applying it 3 times
    // would just flip staged state back and forth, so the count must be
    // ignored (applied exactly once) rather than repeated.
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
    );
    assert!(
        app.status_message.is_some(),
        "Space must still act exactly once (no git backend -> footer message)"
    );
}

#[test]
fn esc_mid_count_cancels_it() {
    let mut app = App::new(vec![sample_file()]);
    let keymap = Keymap::default_map();
    let mut pending = None;
    let mut pending_count: Option<usize> = None;

    press_digits(&mut app, &keymap, &mut pending, &mut pending_count, "5");
    assert_eq!(pending_count, Some(5));
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    );
    assert_eq!(pending_count, None, "Esc must cancel an in-progress count");

    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
    );
    assert_eq!(app.view.cursor, 1, "a plain j, not a leaked 5j");
}

#[test]
fn unbound_key_mid_count_cancels_it() {
    let mut app = App::new(vec![sample_file()]);
    let keymap = Keymap::default_map();
    let mut pending = None;
    let mut pending_count: Option<usize> = None;

    press_digits(&mut app, &keymap, &mut pending, &mut pending_count, "4");
    // A capital `Z`-with-shift-only key that isn't bound anywhere resolves
    // to no action and must drop the count.
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::F(12), KeyModifiers::NONE),
    );
    assert_eq!(pending_count, None, "an unbound key must cancel the count");

    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
    );
    assert_eq!(app.view.cursor, 1, "a plain j, not a leaked 4j");
}

/// Direct unit coverage of `repeat_count` itself: the cap, and that a
/// non-repeatable action always collapses to exactly 1 regardless of count.
#[test]
fn repeat_count_caps_and_ignores_non_repeatable_actions() {
    assert_eq!(repeat_count(Action::CursorDown, Some(5)), 5);
    assert_eq!(repeat_count(Action::CursorDown, None), 1);
    assert_eq!(
        repeat_count(Action::CursorDown, Some(50_000)),
        MAX_COUNT,
        "a count is clamped by the digit-accumulation cap before it ever reaches repeat_count, \
         but repeat_count itself must not blow past MAX_COUNT either"
    );
    assert_eq!(repeat_count(Action::JumpToTop, Some(5)), 1);
    assert_eq!(repeat_count(Action::ToggleStage, Some(5)), 1);
}

/// Closing the help overlay (either `?` or the overlay's own Close action)
/// always resets an in-progress or locked filter, so reopening starts clean.
#[test]
fn closing_help_resets_the_filter() {
    let mut app = App::new(vec![sample_file()]);
    app.apply(Action::ToggleHelp);
    assert!(app.help.open);
    app.help.search = Some(("foo".to_string(), true));

    app.apply(Action::ToggleHelp);
    assert!(!app.help.open);
    assert_eq!(app.help.search, None, "closing help must clear the filter");

    app.apply(Action::ToggleHelp);
    assert_eq!(
        app.help.search, None,
        "reopening help must start with no filter"
    );
}

/// A locked filter narrows the rendered list to rows whose key label or
/// description match the query, dropping section headers (e.g.
/// "Navigation") whose section ends up with no matching rows.
#[test]
fn help_filter_narrows_rendered_bindings_to_matching_rows() {
    let backend = TestBackend::new(120, 50);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.help.open = true;
    app.help.search = Some(("quit".to_string(), false));
    let keymap = Keymap::default_map();

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let content: String = terminal
        .backend()
        .buffer()
        .clone()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(content.contains("Quit and emit annotations"));
    assert!(
        !content.contains("Navigation"),
        "a section with no matching rows must be dropped entirely"
    );
    assert!(!content.contains("Move cursor down"));
}

/// A query with no matches anywhere in the overlay shows a "no matches"
/// line instead of an empty list.
#[test]
fn help_filter_shows_no_matches_message_when_nothing_matches() {
    let backend = TestBackend::new(120, 50);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.help.open = true;
    app.help.search = Some(("zzznomatchzzz".to_string(), false));
    let keymap = Keymap::default_map();

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let content: String = terminal
        .backend()
        .buffer()
        .clone()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(content.contains("no matches for"));
    assert!(content.contains("zzznomatchzzz"));
}

// -- Tabbed help overlay: This context (default) / All keys -----------------

/// `?` always opens on the This context tab, showing only the origin's own
/// scope (Diff, from the diff view) plus Works everywhere — panel-scope
/// content (e.g. the branch/worktree switcher opener) is absent until the
/// user switches tabs.
#[test]
fn help_opens_on_this_context_tab_showing_only_the_origin_scope() {
    let backend = TestBackend::new(100, 300);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.apply(Action::ToggleHelp);
    assert_eq!(app.help.tab, help::HelpTab::ThisContext);
    let keymap = Keymap::default_map();

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let content: String = terminal
        .backend()
        .buffer()
        .clone()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(content.contains("This context"));
    assert!(content.contains("All keys"));
    assert!(content.contains("Move cursor down"), "Diff scope must show");
    assert!(content.contains("Works everywhere"));
    assert!(
        !content.contains("Open branch/worktree switcher"),
        "Panel-scope rows must not show on This context from the diff view"
    );
}

/// From the focused git panel, This context shows the Panel-scope bindings
/// (and Works everywhere) but no Diff-scope rows — the mirror image of
/// `help_opens_on_this_context_tab_showing_only_the_origin_scope`.
#[test]
fn help_this_context_from_the_panel_shows_only_panel_scope() {
    let backend = TestBackend::new(100, 300);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.mode = Mode::Panel {
        cursor: 0,
        tab: PanelTab::Changes,
    };
    app.apply(Action::ToggleHelp);
    assert_eq!(
        app.help.origin,
        ModeOrigin::Panel {
            cursor: 0,
            tab: PanelTab::Changes,
        }
    );
    let keymap = Keymap::default_map();

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let content: String = terminal
        .backend()
        .buffer()
        .clone()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(content.contains("Open branch/worktree switcher"));
    assert!(content.contains("Works everywhere"));
    assert!(
        !content.contains("Move cursor down"),
        "Diff-scope rows must not show on This context from the git panel"
    );
}

/// Switching tabs resets a locked filter and the scroll position (FR-4): a
/// filter that narrows This context to a handful of rows is gone once `Tab`
/// switches to All keys, which then renders complete (including
/// modal-section/panel-scope content This context never carries).
#[test]
fn help_filter_resets_and_other_tab_renders_complete_after_switching() {
    let backend = TestBackend::new(100, 300);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.apply(Action::ToggleHelp);
    app.help.scroll.set(7);
    app.help.search = Some(("stage".to_string(), false));
    let keymap = Keymap::default_map();

    // The locked filter narrows This context: matching rows show, unrelated
    // ones (and the panel-scope switcher opener, off-tab regardless) don't.
    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let narrowed: String = terminal
        .backend()
        .buffer()
        .clone()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(narrowed.contains("Stage/unstage file under cursor"));
    assert!(!narrowed.contains("Move cursor down"));

    let mut pending = None;
    let mut pending_count: Option<usize> = None;
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
    );
    assert_eq!(app.help.tab, help::HelpTab::AllKeys, "Tab must switch tabs");
    assert_eq!(
        app.help.search, None,
        "switching tabs must reset the filter"
    );
    assert_eq!(app.help.scroll.get(), 0, "switching tabs must reset scroll");

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let full: String = terminal
        .backend()
        .buffer()
        .clone()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(
        full.contains("Move cursor down"),
        "the new tab must render unfiltered/complete"
    );
    assert!(
        full.contains("Branch/worktree switcher"),
        "All keys' modal sections must be present now that the filter reset"
    );
}

/// An annotation present on the selected file renders both its inline
/// display row in the diff pane and its entry in the list panel when
/// toggled open — the two annotation UI surfaces this task adds.
#[test]
fn annotation_renders_inline_and_in_list_panel() {
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.annotations
        .add(
            Target::file("src/main.rs"),
            Classification::Question,
            "why swap old() for new()?",
        )
        .unwrap();
    // App::new built `rows` before this annotation existed; rebuild so
    // the inline display row/gutter marker reflect it (this is what
    // `App::submit_compose` does internally on a real compose flow).
    app.view.rows = build_rows(
        &app.view.files[0],
        &app.annotations,
        rows::SyntaxSpans::default(),
    );
    app.mode = Mode::List;

    let keymap = Keymap::default_map();
    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();

    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    // Inline display row in the diff pane.
    assert!(content.contains("question"));
    assert!(content.contains("why swap old() for new()?"));
    // List panel entry (mode is List, so the panel is rendered). The
    // `[1 notes]` count lives in the git panel sidebar, which is hidden
    // here since List mode isn't Mode::Panel — see
    // `sidebar_hidden_in_normal_mode_shown_when_panel_focused`.
    assert!(content.contains("src/main.rs"));
}

/// With a staged file present and the staging panel open, one frame shows
/// both staging surfaces: the staging panel entry and the transient
/// status-footer message. The git panel sidebar's staged `●` indicator and
/// `[N staged]` footer count are covered separately (Staging mode isn't
/// Mode::Panel, so the sidebar is hidden here) — see
/// `sidebar_staged_indicator_renders_when_panel_focused`.
#[test]
fn staging_panel_indicator_and_footer_render() {
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.staged = vec![StagedFile {
        path: "src/main.rs".to_string(),
        letter: 'M',
    }];
    app.staged_states
        .insert("src/main.rs".to_string(), stage_ops::StagedState::Full);
    app.mode = Mode::Staging;
    app.set_status_message("staged hunk");
    let keymap = Keymap::default_map();

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();

    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    assert!(content.contains("staged")); // staging panel title
    assert!(content.contains("M src/main.rs")); // panel entry
    assert!(content.contains("staged hunk")); // status footer message
}

/// The sidebar's staged `●` indicator and `[N staged]` footer count render
/// once the git panel is focused (`Mode::Panel`) — the sidebar's only
/// visibility condition post-hide-by-default.
#[test]
fn sidebar_staged_indicator_renders_when_panel_focused() {
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.staged = vec![StagedFile {
        path: "src/main.rs".to_string(),
        letter: 'M',
    }];
    app.staged_states
        .insert("src/main.rs".to_string(), stage_ops::StagedState::Full);
    app.apply(Action::FocusGitPanel);
    assert!(matches!(app.mode, Mode::Panel { .. }));
    let keymap = Keymap::default_map();

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();

    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    assert!(content.contains("\u{25cf}")); // sidebar staged indicator
    assert!(content.contains("[1 staged]")); // sidebar footer count
}

#[test]
fn empty_staging_panel_shows_hint() {
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.mode = Mode::Staging;
    let keymap = Keymap::default_map();

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();

    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
    assert!(content.contains("nothing staged yet"));
}

/// The staging panel's empty hint resolves its key from the effective
/// keymap rather than a hardcoded literal: a `[keys.diff] toggle-stage`
/// remap must show up here with no code change.
#[test]
fn empty_staging_panel_hint_reflects_a_remapped_toggle_stage_key() {
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.mode = Mode::Staging;

    let mut keys = crate::config::KeysConfig::default();
    keys.diff.insert(
        "toggle-stage".to_string(),
        vec![crate::config::keys::KeySeqSpec::One(
            crate::config::keys::ChordSpec {
                code: crossterm::event::KeyCode::Char('x'),
                mods: KeyModifiers::NONE,
            },
        )],
    );
    let (keymap, warnings) = keymap_config::effective_keymap(&keys);
    assert!(warnings.is_empty());

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();

    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
    assert!(
        content.contains("press x on a hunk to stage it"),
        "staging hint must show the remapped key, not the stale default"
    );
}

/// The annotation list panel's empty hint likewise resolves its key from the
/// effective keymap: a `[keys.diff] compose` remap must show up here too.
#[test]
fn empty_list_panel_hint_reflects_a_remapped_compose_key() {
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.mode = Mode::List;

    let mut keys = crate::config::KeysConfig::default();
    keys.diff.insert(
        "compose".to_string(),
        vec![crate::config::keys::KeySeqSpec::One(
            crate::config::keys::ChordSpec {
                code: crossterm::event::KeyCode::Char('t'),
                mods: KeyModifiers::NONE,
            },
        )],
    );
    let (keymap, warnings) = keymap_config::effective_keymap(&keys);
    assert!(warnings.is_empty());

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();

    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
    assert!(
        content.contains("press t to add one"),
        "list hint must show the remapped key, not the stale default"
    );
}

// -- Syntax highlighting (rendering layer) -------------------------------

/// A row carrying a syntax-highlight span renders that span's text with
/// the theme's token color — asserted via actual buffer cell styles,
/// not just text content, so this exercises the diff pane's rendering
/// (not the tree-sitter engine, which has its own tests).
#[test]
fn syntax_highlighted_span_renders_with_token_color() {
    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    let theme = app.theme;

    let Some(Row::Line(line)) = app.view.rows.get_mut(2) else {
        panic!("expected a line row at index 2");
    };
    assert_eq!(line.content, "fn main() {");
    line.syntax_spans = Some(vec![(0..2, TokenKind::Keyword)]);

    let keymap = Keymap::default_map();
    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();

    let buffer = terminal.backend().buffer().clone();
    let has_keyword_cell = buffer
        .content()
        .iter()
        .any(|cell| cell.symbol() == "f" && cell.fg == theme.keyword);
    assert!(
        has_keyword_cell,
        "expected a cell styled with the keyword token color"
    );
}

// -- Search ---------------------------------------------------------------

/// An active search shows the `/pattern` footer prompt while typing,
/// and — once confirmed — the matched row's text renders with the
/// search-match background.
#[test]
fn search_mode_renders_prompt_and_confirmed_match_is_highlighted() {
    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    let theme = app.theme;

    app.mode = Mode::Search;
    app.search_input = "n".to_string();
    let keymap = Keymap::default_map();
    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
    assert!(content.contains("/n"));

    // "n" matches both "fn main() {" (row 2) and "new();" (row 4);
    // confirming jumps the cursor to the first match (row 2), leaving
    // the second match (row 4) highlighted but not selected — so its
    // background is unambiguously the search-match tint, not the
    // cursor-row tint.
    app.confirm_search();
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.search.matches.len(), 2);
    assert_ne!(app.view.cursor, app.search.matches[1]);

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let has_match_bg = buffer
        .content()
        .iter()
        .any(|cell| cell.bg == theme.search_match_bg);
    assert!(
        has_match_bg,
        "expected the unselected matched row to carry the search-match background"
    );
}

// -- Column cursor ---------------------------------------------------------

/// The column cursor renders as a distinct background on the cursor
/// row's char cell, and only on the cursor row.
#[test]
fn column_cursor_renders_distinct_background_on_the_cursor_cell() {
    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    let theme = app.theme;
    app.apply(Action::CursorDown); // hunk header
    app.apply(Action::CursorDown); // "fn main() {"
    app.apply(Action::CursorRight);
    app.apply(Action::CursorRight); // column 2, the second 'n' of "fn"
    let keymap = Keymap::default_map();

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let has_cursor_bg = buffer
        .content()
        .iter()
        .any(|cell| cell.bg == theme.column_cursor_bg);
    assert!(
        has_cursor_bg,
        "expected exactly one cell styled with the column-cursor background"
    );
}

// -- LSP peek overlay --------------------------------------------------------

/// Canned References results plus a preloaded preview cache render both
/// the location list and the syntax-free preview text, without ever
/// touching a real LSP server.
#[test]
fn peek_overlay_renders_canned_references_and_preview() {
    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);

    let loc_path = PathBuf::from("/tmp/repo/src/main.rs");
    let mut peek = super::peek::PeekState::locations(
        super::peek::PeekKind::References,
        vec![SourceLocation {
            path: loc_path.clone(),
            line: 0,
            character: 0,
        }],
    );
    peek.preview_cache.insert(
        loc_path,
        super::peek::CachedPreview {
            lines: vec!["fn main() {".to_string(), "    old();".to_string()],
            spans: Vec::new(),
        },
    );
    app.peek = Some(peek);
    app.mode = Mode::Peek;
    let keymap = Keymap::default_map();

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    assert!(content.contains("references: 1 results"));
    assert!(content.contains("main.rs"));
    assert!(content.contains("fn main() {"));
}

/// A Hover overlay renders its text body in the same overlay chrome.
#[test]
fn peek_overlay_renders_hover_text() {
    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.peek = Some(super::peek::PeekState::hover(
        "this function does nothing interesting".to_string(),
    ));
    app.mode = Mode::Peek;
    let keymap = Keymap::default_map();

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    assert!(content.contains("hover"));
    assert!(content.contains("this function does nothing interesting"));
}

// -- Git panel focus -----------------------------------------------------

/// A diff file with several rows (so `j` visibly scrolls the cursor),
/// parametrized by path so the panel can list more than one entry.
fn named_file(path: &str) -> FileDiff {
    let raw = format!(
        "diff --git a/{path} b/{path}\n\
             index 111..222 100644\n\
             --- a/{path}\n\
             +++ b/{path}\n\
             @@ -1,2 +1,2 @@\n\
             -old\n\
             +new\n\
             \x20ctx\n"
    );
    FileDiff::from_patch(&RawFilePatch {
        path: path.to_string(),
        old_path: None,
        raw,
        is_binary: false,
    })
    .unwrap()
}

// -- Performance -----------------------------------------------------------

/// A file whose single hunk carries `pairs` removed/added line pairs
/// (`2 * pairs` changed lines), each a realistic Rust statement so the
/// word-diff pairing runs on non-trivial content.
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

/// An `App` populated like a real review session with panel state:
/// two tracked files, one untracked, a branch header, and two stashes.
fn panel_smoke_app() -> App {
    let mut app = App::new(vec![
        named_file("src/a.rs"),
        named_file("src/b.rs"),
        named_file("notes.md"),
    ]);
    app.untracked_paths = vec!["notes.md".to_string()];
    app.branch = Some(crate::git::BranchStatus {
        name: "main".to_string(),
        detached: false,
        upstream: Some("origin/main".to_string()),
        ahead_behind: Some((2, 1)),
    });
    app.stashes = vec![
        crate::git::StashEntry {
            stash_ref: "stash@{0}".to_string(),
            branch: Some("main".to_string()),
            message: "wip: parser".to_string(),
        },
        crate::git::StashEntry {
            stash_ref: "stash@{1}".to_string(),
            branch: Some("main".to_string()),
            message: "spike: tabs".to_string(),
        },
    ];
    app
}

/// Scans the top border row (y = 0) for cells painted with the
/// focused-border color, returning `(diff_side_hot, panel_side_hot)`. The
/// diff pane occupies `x < panel_start`; the panel occupies the rest.
fn top_border_hot(
    terminal: &Terminal<TestBackend>,
    focus: ratatui::style::Color,
    width: usize,
    panel_start: usize,
) -> (bool, bool) {
    let binding = terminal.backend().buffer().clone();
    let content = binding.content();
    let mut diff_hot = false;
    let mut panel_hot = false;
    for (x, cell) in content.iter().enumerate().take(width) {
        if cell.fg == focus {
            if x < panel_start {
                diff_hot = true;
            } else {
                panel_hot = true;
            }
        }
    }
    (diff_hot, panel_hot)
}

/// The focused pane's border is emphasized, and the toggle moves that
/// emphasis from the diff pane to the git panel and back. Since the panel
/// is hidden unless focused (see `split_layout`), this also exercises the
/// sidebar appearing/disappearing across the same toggle.
#[test]
fn focused_pane_border_emphasis_follows_the_toggle() {
    let width = 80usize;
    // Derived, not a literal: the sidebar's left edge tracks `sidebar_width`
    // so this test can't silently drift when the ratio/clamps change.
    let panel_start = width - sidebar_width(width as u16, None) as usize;
    let backend = TestBackend::new(width as u16, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = panel_smoke_app();
    let focus = app.theme.focused_border;
    let keymap = Keymap::default_map();

    // Diff focused (Normal): the sidebar is hidden entirely, so the diff
    // pane spans the full width with its border emphasized.
    terminal.draw(|f| draw(f, &app, &keymap, None)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|c| c.symbol()).collect();
    assert!(
        !content.contains("git: main"),
        "sidebar should be hidden when diff focused"
    );
    let (diff_hot, _) = top_border_hot(&terminal, focus, width, panel_start);
    assert!(
        diff_hot,
        "diff border should be emphasized when diff focused"
    );

    // Panel focused: the sidebar reappears at its fixed width, and emphasis
    // moves to the panel border.
    app.apply(Action::FocusGitPanel);
    assert!(matches!(app.mode, Mode::Panel { .. }));
    terminal.draw(|f| draw(f, &app, &keymap, None)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|c| c.symbol()).collect();
    assert!(
        content.contains("git: main"),
        "sidebar should render when panel focused"
    );
    let (diff_hot, panel_hot) = top_border_hot(&terminal, focus, width, panel_start);
    assert!(
        panel_hot,
        "panel border should be emphasized when panel focused"
    );
    assert!(!diff_hot, "diff border should be plain when panel focused");

    // Toggling back to the diff hides the sidebar again.
    app.apply(Action::FocusGitPanel);
    assert!(matches!(app.mode, Mode::Normal));
    terminal.draw(|f| draw(f, &app, &keymap, None)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|c| c.symbol()).collect();
    assert!(
        !content.contains("git: main"),
        "sidebar should hide again once focus returns to the diff"
    );
}

/// The sidebar rect widens/narrows with the terminal per `sidebar_width`'s
/// clamped-30% rule, and that's reflected both in `split_layout` itself and
/// in what actually gets rendered onto the buffer. Uses the tip commit's
/// subject line (`commit_line`, deliberately unwrapped/unclipped-by-code —
/// see its doc comment, ratatui clips it at the rect edge) as the "does more
/// room show more text" probe: the subject is long enough to be clipped by a
/// 40-wide sidebar but to fit whole inside a 72-wide one.
#[test]
fn sidebar_rect_and_render_scale_with_terminal_width_when_panel_focused() {
    let subject = "feat: add sidebar proportional width layout support";
    let cases: &[(u16, u16)] = &[(80, 40), (160, 48), (240, 72)];

    for &(width, expected_sidebar) in cases {
        // Pure layout: `split_layout` itself hands back the clamped width.
        let area = Rect::new(0, 0, width, 30);
        let (sidebar_rect, diff_rect) = split_layout(area, true, SidebarSide::Right, None);
        let sidebar_rect = sidebar_rect.expect("sidebar shown when panel focused");
        assert_eq!(
            sidebar_rect.width, expected_sidebar,
            "split_layout sidebar width at terminal width {width}"
        );
        assert_eq!(
            diff_rect.width,
            width - expected_sidebar,
            "split_layout diff-pane width at terminal width {width}"
        );

        // Grounded in a real render: the git panel's branch title only ever
        // appears at or past the sidebar's left edge, never inside the diff
        // pane's columns, proving the sidebar actually renders at the width
        // `split_layout` computed above (not just on paper).
        let backend = TestBackend::new(width, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = panel_smoke_app();
        app.last_commit = Some(crate::git::CommitSummary {
            short_hash: "a1b2c3d".to_string(),
            subject: subject.to_string(),
        });
        app.apply(Action::FocusGitPanel);
        let keymap = Keymap::default_map();
        terminal.draw(|f| draw(f, &app, &keymap, None)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        let panel_start = width - expected_sidebar;
        let mut diff_side = String::new();
        let mut whole = String::new();
        for y in 0..buffer.area.height {
            for x in 0..width {
                let symbol = buffer.cell((x, y)).map(|c| c.symbol()).unwrap_or(" ");
                whole.push_str(symbol);
                if x < panel_start {
                    diff_side.push_str(symbol);
                }
            }
        }
        assert!(
            !diff_side.contains("git: main"),
            "diff pane columns should never show the sidebar's branch title at width {width}"
        );
        assert!(
            whole.contains("git: main"),
            "sidebar should render its branch title somewhere at width {width}"
        );

        // The un-truncated-at-wide/truncated-at-narrow probe: only asserted
        // at the two ends of the sweep (40 clips, 72 fits whole); 48 sits in
        // between and isn't a specified boundary.
        if expected_sidebar == 72 {
            assert!(
                whole.contains(subject),
                "a {expected_sidebar}-wide sidebar should show the full commit subject un-truncated"
            );
        } else if expected_sidebar == 40 {
            assert!(
                !whole.contains(subject),
                "a {expected_sidebar}-wide sidebar should clip the long commit subject"
            );
        }
    }
}

/// `[layout] sidebar_side = "left"` moves the sidebar to the left edge; the
/// diff pane gets the remaining width on the right.
#[test]
fn split_layout_left_side_puts_sidebar_at_the_left_edge() {
    let area = Rect::new(0, 0, 100, 30);
    let (sidebar_rect, diff_rect) = split_layout(area, true, SidebarSide::Left, None);
    let sidebar_rect = sidebar_rect.expect("sidebar shown when panel focused");
    assert_eq!(sidebar_rect.x, 0);
    assert_eq!(diff_rect.x, sidebar_rect.width);
    assert_eq!(diff_rect.width, 100 - sidebar_rect.width);
}

/// `[layout] sidebar_width` (already range-validated at load time) picks the
/// exact width, overriding the proportional formula, on both sides.
#[test]
fn split_layout_configured_width_overrides_the_formula_on_both_sides() {
    let area = Rect::new(0, 0, 100, 30);

    let (right_sidebar, _) = split_layout(area, true, SidebarSide::Right, Some(55));
    assert_eq!(right_sidebar.expect("shown").width, 55);

    let (left_sidebar, _) = split_layout(area, true, SidebarSide::Left, Some(55));
    assert_eq!(left_sidebar.expect("shown").width, 55);
}

/// A hidden sidebar (`show_sidebar: false`) ignores `side`/`configured_width`
/// entirely and hands the whole area to the diff pane, exactly as before
/// config existed.
#[test]
fn split_layout_hidden_sidebar_ignores_side_and_width() {
    let area = Rect::new(0, 0, 100, 30);
    let (sidebar, diff) = split_layout(area, false, SidebarSide::Left, Some(55));
    assert!(sidebar.is_none());
    assert_eq!(diff.width, area.width);
}

/// Drives real `KeyEvent`s through `dispatch_key` — the exact path the
/// blocking event loop uses — proving the focus toggle, panel `j`/`k`
/// traversal across all three sections (with the diff auto-following file
/// rows as the cursor moves), Enter-on-file, and that the diff-scope keys
/// still dispatch identically while the panel is unfocused. tmux is
/// unavailable on this host, so this headless driver stands in for the
/// manual smoke transcript (see 02-task-03-smoke.txt).
#[test]
fn panel_focus_key_dispatch_smoke() {
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut pending_count: Option<usize> = None;
    let mut app = panel_smoke_app();
    let mut press = |app: &mut App, pending: &mut Option<KeyEvent>, code: KeyCode| {
        let _ = dispatch_key(
            app,
            &keymap,
            pending,
            &mut pending_count,
            KeyEvent::new(code, KeyModifiers::NONE),
        );
    };

    // Focus the panel: cursor resets to the top, which is the `src`
    // directory row. Following a directory row does nothing, so the diff
    // stays on src/a.rs (`selected_file` starts at 0).
    assert_eq!(app.mode, Mode::Normal);
    press(&mut app, &mut pending, KeyCode::Char('`'));
    assert!(matches!(app.mode, Mode::Panel { .. }));
    assert_eq!(app.panel_cursor(), 0); // the src/ directory row
    assert_eq!(app.view.selected_file, 0); // src/a.rs

    // Walk the tree with `j`: src/ -> src/a.rs -> src/b.rs -> notes.md,
    // clamping at the last file. The diff follows each file row; the
    // directory row leaves the last-followed file alone.
    press(&mut app, &mut pending, KeyCode::Char('j'));
    assert_eq!(app.panel_cursor(), 1); // src/a.rs
    assert_eq!(app.view.selected_file, 0); // followed to src/a.rs
    press(&mut app, &mut pending, KeyCode::Char('j'));
    assert_eq!(app.panel_cursor(), 2); // src/b.rs
    assert_eq!(app.view.selected_file, 1); // followed to src/b.rs
    press(&mut app, &mut pending, KeyCode::Char('j'));
    assert_eq!(app.panel_cursor(), 3); // notes.md (root-level untracked)
    assert_eq!(app.view.selected_file, 2); // followed to notes.md
    press(&mut app, &mut pending, KeyCode::Char('j'));
    assert_eq!(app.panel_cursor(), 3); // clamped at the bottom
    press(&mut app, &mut pending, KeyCode::Char('k'));
    assert_eq!(app.panel_cursor(), 2); // back up onto src/b.rs
    assert_eq!(app.view.selected_file, 1);

    // Enter on the directory row folds it, hiding its files, and keeps the
    // panel focused.
    press(&mut app, &mut pending, KeyCode::Char('k')); // -> src/a.rs
    press(&mut app, &mut pending, KeyCode::Char('k')); // -> src/ directory
    assert_eq!(app.panel_cursor(), 0);
    press(&mut app, &mut pending, KeyCode::Enter);
    assert!(matches!(app.mode, Mode::Panel { .. }));
    assert!(app.panel_collapsed_dirs.contains("src"));
    // Now only two rows remain: src/ and notes.md.
    press(&mut app, &mut pending, KeyCode::Enter); // unfold again
    assert!(!app.panel_collapsed_dirs.contains("src"));

    // Move onto src/b.rs (following along the way) and Enter: focus
    // returns to the diff, already on src/b.rs from the follow.
    press(&mut app, &mut pending, KeyCode::Char('j')); // -> src/a.rs (1)
    press(&mut app, &mut pending, KeyCode::Char('j')); // -> src/b.rs (2)
    assert_eq!(app.panel_cursor(), 2);
    assert_eq!(app.view.selected_file, 1); // followed to src/b.rs
    press(&mut app, &mut pending, KeyCode::Enter);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.view.selected_file, 1); // src/b.rs

    // With the panel unfocused, the diff-scope keys dispatch as before.
    let cursor_before = app.view.cursor;
    press(&mut app, &mut pending, KeyCode::Char('j')); // CursorDown
    assert_eq!(app.view.cursor, cursor_before + 1);
    press(&mut app, &mut pending, KeyCode::Char('k')); // CursorUp
    assert_eq!(app.view.cursor, cursor_before);
    press(&mut app, &mut pending, KeyCode::Char('s')); // staging panel
    assert_eq!(app.mode, Mode::Staging);
    press(&mut app, &mut pending, KeyCode::Char('s')); // close it
    assert_eq!(app.mode, Mode::Normal);
    // `space` (ToggleStage) and `gd` still dispatch (no git/LSP backend
    // here, so they degrade to a footer message rather than acting) —
    // the point is they resolve and run without panicking, unchanged.
    press(&mut app, &mut pending, KeyCode::Char(' '));
    assert_eq!(app.mode, Mode::Normal);
    press(&mut app, &mut pending, KeyCode::Char('g'));
    press(&mut app, &mut pending, KeyCode::Char('d'));
    assert_eq!(app.mode, Mode::Normal);
}

/// The focused git panel is a first-class view, so the quit family ends
/// the session from it just as from the diff view: `q` emits, `Q`/Ctrl-C
/// discard. Driven through the real `dispatch_key` path.
#[test]
fn quit_family_quits_from_focused_panel() {
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut pending_count: Option<usize> = None;
    let cases = [
        (
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
            QuitOutcome::Emit,
        ),
        (
            KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::NONE),
            QuitOutcome::Discard,
        ),
        (
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            QuitOutcome::Discard,
        ),
    ];
    for (ev, want) in cases {
        let mut app = panel_smoke_app();
        app.apply(Action::FocusGitPanel);
        assert!(matches!(app.mode, Mode::Panel { .. }));
        match dispatch_key(&mut app, &keymap, &mut pending, &mut pending_count, ev) {
            Flow::Quit(outcome) => assert_eq!(outcome, want, "wrong quit outcome for {ev:?}"),
            Flow::Continue => panic!("{ev:?} should quit from the focused panel"),
            Flow::OpenEditor { .. } => panic!("{ev:?} should quit, not open an editor"),
        }
    }
}

// -- Review-session banner layout --------------------------------------------

/// `split_banner` reserves exactly one row at the top when shown, and is a
/// pure passthrough when not — pinning the pure half of the split chain
/// `draw` and the event loop's viewport-measurement mirror both run through.
#[test]
fn split_banner_reserves_exactly_one_row_when_shown() {
    let area = Rect::new(0, 0, 100, 40);
    let (banner, rest) = split_banner(area, true);
    let banner = banner.expect("banner must render when shown");
    assert_eq!(banner.height, 1);
    assert_eq!(banner.y, 0);
    assert_eq!(rest.y, 1);
    assert_eq!(rest.height, 39);

    let (banner_hidden, rest_hidden) = split_banner(area, false);
    assert!(banner_hidden.is_none());
    assert_eq!(rest_hidden, area);
}

/// `diff_pane_rect` — the shared function [`event_loop`]'s viewport
/// measurement and `draw`'s own `debug_assert_eq!` both depend on — must
/// shrink the diff pane by exactly the banner's one row during a review
/// session, and leave it untouched otherwise. A wide fixed area keeps the
/// footer strip at its 1-row floor for both targets, isolating the banner's
/// effect from any unrelated width-driven footer wrapping.
#[test]
fn diff_pane_rect_shrinks_by_exactly_the_banner_row_during_a_review_session() {
    let keymap = Keymap::default_map();
    let full_area = Rect::new(0, 0, 200, 40);

    let mut plain = App::new(vec![sample_file()]);
    plain.target = DiffTarget::WorkingTree;
    let plain_area = diff_pane_rect(full_area, &plain, &keymap, None);

    let mut review = App::new(vec![sample_file()]);
    review.target = DiffTarget::Review {
        base: "main".to_string(),
        branch: "feature".to_string(),
    };
    let review_area = diff_pane_rect(full_area, &review, &keymap, None);

    assert_eq!(
        review_area.height + 1,
        plain_area.height,
        "a review session's banner row must be subtracted from the diff pane's height"
    );
    assert_eq!(review_area.y, plain_area.y + 1);
    assert_eq!(review_area.x, plain_area.x);
    assert_eq!(review_area.width, plain_area.width);
}

// -- `q`/`Q` review-mode lifecycle -------------------------------------------

/// Outside a review session, `q`/`Q` are byte-for-byte unchanged: `q` still
/// quits emitting, `Q` still quits discarding — pinned as an explicit
/// regression test against `quit_action`'s review-session branch.
#[test]
fn q_and_shift_q_are_unchanged_outside_a_review_session() {
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut pending_count: Option<usize> = None;

    let mut app = App::new(vec![sample_file()]);
    match dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
    ) {
        Flow::Quit(QuitOutcome::Emit) => {}
        other => panic!("q outside a review session must quit emitting, got {other:?}"),
    }

    let mut app = App::new(vec![sample_file()]);
    match dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::NONE),
    ) {
        Flow::Quit(QuitOutcome::Discard) => {}
        other => panic!("Q outside a review session must quit discarding, got {other:?}"),
    }
}

/// In a review session, `q` opens the end-review modal instead of quitting;
/// `Q` keeps its global "quit immediately, emit nothing" meaning.
#[test]
fn q_opens_end_review_modal_and_shift_q_still_quits_instantly_in_a_review_session() {
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut pending_count: Option<usize> = None;

    let mut app = App::new(vec![sample_file()]);
    app.target = DiffTarget::Review {
        base: "main".to_string(),
        branch: "feature".to_string(),
    };
    match dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
    ) {
        Flow::Continue => {}
        other => panic!("q in a review session must not quit directly, got {other:?}"),
    }
    assert!(
        matches!(app.mode, Mode::EndReview { .. }),
        "q in a review session must open the end-review modal, got {:?}",
        app.mode
    );

    let mut app = App::new(vec![sample_file()]);
    app.target = DiffTarget::Review {
        base: "main".to_string(),
        branch: "feature".to_string(),
    };
    match dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::NONE),
    ) {
        Flow::Quit(QuitOutcome::Discard) => {}
        other => panic!("Q in a review session must still quit instantly, got {other:?}"),
    }
}

// -- Accept/defer keys --------------------------------------------------------

/// `Space` in a review session dispatched through the real `dispatch_key`
/// path (not `App::apply` directly): translates the resolved `ToggleStage`
/// into `ToggleAccept`, accepts the cursor file, collapses its section, and
/// the banner's `(accepted, total)` count reflects it immediately.
#[test]
fn space_accepts_the_cursor_file_in_a_review_session_via_dispatch_key() {
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut pending_count: Option<usize> = None;
    let mut app = App::new(vec![sample_file()]);
    app.target = DiffTarget::Review {
        base: "main".to_string(),
        branch: "feature".to_string(),
    };
    assert_eq!(app.review_progress(), (0, 1));

    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
    );

    assert_eq!(app.review_status("src/main.rs"), ReviewStatus::Accepted);
    assert!(app.view.is_collapsed("src/main.rs"));
    assert_eq!(
        app.review_progress(),
        (1, 1),
        "banner accepted/total must reflect the accept immediately"
    );

    // A second press un-accepts and expands, and the count drops back down.
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
    );
    assert_eq!(app.review_status("src/main.rs"), ReviewStatus::Unreviewed);
    assert!(!app.view.is_collapsed("src/main.rs"));
    assert_eq!(app.review_progress(), (0, 1));
}

/// `S` accepts unconditionally via the same dispatch-time translation.
#[test]
fn shift_s_accepts_the_cursor_file_in_a_review_session_via_dispatch_key() {
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut pending_count: Option<usize> = None;
    let mut app = App::new(vec![sample_file()]);
    app.target = DiffTarget::Review {
        base: "main".to_string(),
        branch: "feature".to_string(),
    };

    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char('S'), KeyModifiers::NONE),
    );

    assert_eq!(app.review_status("src/main.rs"), ReviewStatus::Accepted);
    assert_eq!(app.review_progress(), (1, 1));
}

/// `d` toggles defer, bound directly (no translation needed).
#[test]
fn d_defers_the_cursor_file_in_a_review_session_via_dispatch_key() {
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut pending_count: Option<usize> = None;
    let mut app = App::new(vec![sample_file()]);
    app.target = DiffTarget::Review {
        base: "main".to_string(),
        branch: "feature".to_string(),
    };

    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
    );

    assert_eq!(app.review_status("src/main.rs"), ReviewStatus::Deferred);
    assert!(app.view.is_collapsed("src/main.rs"));
    // A deferred file never counts as accepted.
    assert_eq!(app.review_progress(), (0, 1));
}

/// Outside a review session, `Space`/`S` keep staging's pre-existing
/// meaning byte-for-byte: no review state is ever produced, no matter the
/// target (a regression pin against `dispatch_key`'s review-session
/// translation firing unconditionally).
#[test]
fn space_and_shift_s_never_produce_review_state_outside_a_review_session() {
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut pending_count: Option<usize> = None;

    for target in [
        DiffTarget::WorkingTree,
        DiffTarget::Range("main..HEAD".to_string()),
    ] {
        let mut app = App::new(vec![sample_file()]);
        app.target = target.clone();
        dispatch_key(
            &mut app,
            &keymap,
            &mut pending,
            &mut pending_count,
            KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
        );
        dispatch_key(
            &mut app,
            &keymap,
            &mut pending,
            &mut pending_count,
            KeyEvent::new(KeyCode::Char('S'), KeyModifiers::NONE),
        );
        assert_eq!(
            app.review_status("src/main.rs"),
            ReviewStatus::Unreviewed,
            "target {target:?} must never produce review state from Space/S"
        );
    }
}

/// Outside a review session, `d` is a total no-op — byte-for-byte the same
/// as when the key was unbound: no state change, no status message, mode
/// untouched.
#[test]
fn d_is_a_total_no_op_outside_a_review_session_via_dispatch_key() {
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut pending_count: Option<usize> = None;
    let mut app = App::new(vec![sample_file()]);
    assert_eq!(app.target, DiffTarget::WorkingTree);

    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
    );

    assert_eq!(app.review_status("src/main.rs"), ReviewStatus::Unreviewed);
    assert!(!app.view.is_collapsed("src/main.rs"));
    assert!(app.status_message.is_none());
    assert_eq!(app.mode, Mode::Normal);
}

fn file_named(path: &str) -> FileDiff {
    let raw = format!(
        "diff --git a/{path} b/{path}\nindex 111..222 100644\n--- a/{path}\n+++ b/{path}\n@@ -1,1 +1,1 @@\n-old\n+new\n"
    );
    FileDiff::from_patch(&RawFilePatch {
        path: path.to_string(),
        old_path: None,
        raw,
        is_binary: false,
    })
    .unwrap()
}

/// The reviewer-facing rendering proof: sidebar
/// and section-header markers for an accepted (`●`, reusing the staged
/// glyph/color) and a deferred (`~`) file, alongside the banner's
/// `accepted/total` progress count — the full real render, panel focused so
/// the sidebar is visible. Set `REDQUILL_PROOF_DUMP=1` to print the rendered
/// buffer to stderr (mirrors `perf_tests.rs`'s `REDQUILL_PERF_PRINT`
/// convention) for a text-render screenshot substitute.
#[test]
fn review_markers_render_on_sidebar_and_section_headers_with_banner_count() {
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![file_named("a.rs"), file_named("b.rs")]);
    app.target = DiffTarget::Review {
        base: "main".to_string(),
        branch: "feature".to_string(),
    };
    app.apply(Action::ToggleAccept); // a.rs (cursor starts on its header)
    app.select_file_by_path("b.rs");
    app.apply(Action::ToggleDefer); // b.rs
    // Move the cursor back to a.rs's header *without* going through
    // `select_file_by_path` (which un-collapses its target on selection —
    // correct for that gesture, but not what this proof wants): focusing
    // the panel resets its cursor to row 0 and `panel_follow` (existing,
    // pre-spec-08 behavior) re-syncs the diff to whatever file that row
    // names, expanding it if `view.selected_file` disagrees. Pre-syncing
    // `selected_file` to a.rs here (its collapsed header is still row 0)
    // keeps both files rendered in their true collapsed-by-review state.
    app.view.cursor = app.view.header_row_of_file[0];
    app.rebuild_rows();
    app.apply(Action::FocusGitPanel);
    assert!(matches!(app.mode, Mode::Panel { .. }));
    let keymap = Keymap::default_map();

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
    if std::env::var_os("REDQUILL_PROOF_DUMP").is_some() {
        let w = buffer.area.width as usize;
        let symbols: Vec<&str> = buffer.content().iter().map(|c| c.symbol()).collect();
        for row in symbols.chunks(w) {
            eprintln!("{}", row.concat());
        }
    }

    assert!(content.contains("REVIEWING feature"));
    assert!(
        content.contains("1/2"),
        "banner accepted/total: {content:?}"
    );
    // Both the sidebar and the section-header marker slot carry the glyphs;
    // this asserts presence of each marker character somewhere on screen —
    // the per-widget unit tests (`diff_view.rs`/`git_panel.rs`) pin the
    // exact slot/color, this proves the whole render composes them together.
    assert!(
        content.contains('\u{25cf}'),
        "accepted ● marker: {content:?}"
    );
    assert!(content.contains('~'), "deferred ~ marker: {content:?}");
}

// -- Accepted-files panel -----------------------------------------------------

/// `s` in a review session with nothing accepted yet shows the
/// review-appropriate empty-state hint — the mirror of
/// `empty_staging_panel_shows_hint`, resolving `Space`'s key from the
/// effective keymap (`Action::ToggleAccept`) rather than a hardcoded key,
/// same convention as the local panel's own hint.
#[test]
fn empty_accepted_panel_shows_review_appropriate_hint() {
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    app.target = DiffTarget::Review {
        base: "main".to_string(),
        branch: "feature".to_string(),
    };
    app.apply(Action::ToggleStagingPanel);
    assert_eq!(app.mode, Mode::Staging);
    let keymap = Keymap::default_map();

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    assert!(content.contains("no files accepted yet"));
    assert!(content.contains("Space"));
    // Never the local panel's own wording.
    assert!(!content.contains("nothing staged yet"));
}

/// The accepted-files panel lists accepted files (not deferred/unreviewed
/// ones) and un-accepting one via `Space` removes it from the list,
/// re-expands its diff section, and drops the banner's accepted count —
/// the full round trip through the real `dispatch_key` path (the
/// unstage-panel analogue).
#[test]
fn accepted_panel_lists_accepted_files_and_space_un_accepts() {
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![
        file_named("a.rs"),
        file_named("b.rs"),
        file_named("c.rs"),
    ]);
    app.target = DiffTarget::Review {
        base: "main".to_string(),
        branch: "feature".to_string(),
    };
    app.apply(Action::ToggleAccept); // a.rs accepted
    app.select_file_by_path("b.rs");
    app.apply(Action::ToggleDefer); // b.rs deferred, not accepted
    app.select_file_by_path("c.rs");
    app.apply(Action::ToggleAccept); // c.rs accepted
    assert_eq!(app.review_progress(), (2, 3));

    app.apply(Action::ToggleStagingPanel);
    assert_eq!(app.mode, Mode::Staging);
    // The panel's underlying list model (not a rendered-text scan, which
    // can't distinguish the panel's own list from the diff pane's section
    // headers rendered alongside it — every file's header shows there
    // regardless of accept/defer status): exactly the two accepted files,
    // in diff order, deferred `b.rs` excluded.
    let listed: Vec<&str> = app.staged.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        listed,
        vec!["a.rs", "c.rs"],
        "deferred file must not be listed"
    );

    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut pending_count: Option<usize> = None;

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
    if std::env::var_os("REDQUILL_PROOF_DUMP").is_some() {
        let w = buffer.area.width as usize;
        let symbols: Vec<&str> = buffer.content().iter().map(|c| c.symbol()).collect();
        for row in symbols.chunks(w) {
            eprintln!("{}", row.concat());
        }
    }
    assert!(content.contains("a.rs"));
    assert!(content.contains("c.rs"));

    // Un-accept the focused (first) entry via the real dispatch path.
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
    );
    assert_eq!(app.review_status("a.rs"), ReviewStatus::Unreviewed);
    assert!(!app.view.is_collapsed("a.rs"));
    assert_eq!(
        app.staged.len(),
        1,
        "the un-accepted file drops off the list"
    );
    assert_eq!(app.review_progress(), (1, 3));
}

/// Outside a review session, `s` still opens the ordinary staging panel —
/// byte-for-byte the pre-existing behavior (a regression pin against the
/// accepted-panel repurposing added this task).
#[test]
fn staging_panel_is_unchanged_outside_a_review_session() {
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = App::new(vec![sample_file()]);
    assert_eq!(app.target, DiffTarget::WorkingTree);
    app.apply(Action::ToggleStagingPanel);
    assert_eq!(app.mode, Mode::Staging);
    let keymap = Keymap::default_map();

    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    assert!(content.contains("nothing staged yet"));
    assert!(!content.contains("no files accepted yet"));
}

// -- Branch/worktree switcher modal ------------------------------------------

/// `b` resolves to `OpenSwitcher` only in panel scope, driven through the
/// real `dispatch_key` path. `panel_smoke_app` attaches no git backend, so
/// the switcher can't read branch/worktree lists and this degrades to a
/// footer message rather than opening — still proving `b` reaches
/// `App::open_switcher` from the focused panel rather than resolving to
/// nothing.
#[test]
fn b_in_panel_mode_opens_switcher_through_dispatch_key() {
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut pending_count: Option<usize> = None;
    let mut app = panel_smoke_app();
    app.apply(Action::FocusGitPanel);
    assert!(matches!(app.mode, Mode::Panel { .. }));
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE),
    );
    assert!(matches!(app.mode, Mode::Panel { .. }));
    assert!(
        app.status_message.is_some(),
        "b must act (no-backend footer message) from the focused panel"
    );
}

// -- Guarded panel writes during review ---------------------------------------

/// `p`/`P` in a review session open the confirm modal instead of running the
/// op immediately; `f` stays unprompted, running through the unchanged
/// direct path.
#[test]
fn p_and_shift_p_open_a_confirm_modal_in_a_review_session_f_stays_unprompted() {
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut pending_count: Option<usize> = None;

    let mut app = panel_smoke_app();
    app.target = DiffTarget::Review {
        base: "main".to_string(),
        branch: "feature".to_string(),
    };
    app.apply(Action::FocusGitPanel);
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
    );
    assert!(
        matches!(app.mode, Mode::ConfirmRemoteOp { op, .. } if op == RemoteOp::Pull),
        "p in a review session must open the pull confirm modal, got {:?}",
        app.mode
    );

    let mut app = panel_smoke_app();
    app.target = DiffTarget::Review {
        base: "main".to_string(),
        branch: "feature".to_string(),
    };
    app.apply(Action::FocusGitPanel);
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char('P'), KeyModifiers::NONE),
    );
    assert!(
        matches!(app.mode, Mode::ConfirmRemoteOp { op, .. } if op == RemoteOp::Push),
        "P in a review session must open the push confirm modal, got {:?}",
        app.mode
    );

    let mut app = panel_smoke_app();
    app.target = DiffTarget::Review {
        base: "main".to_string(),
        branch: "feature".to_string(),
    };
    app.apply(Action::FocusGitPanel);
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE),
    );
    assert!(
        matches!(app.mode, Mode::Panel { .. }),
        "f must stay unprompted (no confirm modal) even in a review session, got {:?}",
        app.mode
    );
    assert!(
        app.status_message.is_some(),
        "f must still act directly (no-backend footer message)"
    );
}

/// Outside a review session, `p`/`P`/`f` are byte-for-byte unchanged: none
/// of them ever opens the confirm modal (a regression pin against the
/// guard added this task).
#[test]
fn p_shift_p_and_f_never_open_a_confirm_modal_outside_a_review_session() {
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut pending_count: Option<usize> = None;

    for ch in ['p', 'P', 'f'] {
        let mut app = panel_smoke_app();
        assert_eq!(app.target, DiffTarget::WorkingTree);
        app.apply(Action::FocusGitPanel);
        dispatch_key(
            &mut app,
            &keymap,
            &mut pending,
            &mut pending_count,
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
        );
        assert!(
            matches!(app.mode, Mode::Panel { .. }),
            "{ch} outside a review session must never open the confirm modal, got {:?}",
            app.mode
        );
        assert!(
            app.status_message.is_some(),
            "{ch} must still act directly (no-backend footer message)"
        );
    }
}

/// `b` in Normal mode (diff scope) is unaffected by the panel-scope
/// binding: it stays `WordBackward`, never opening the switcher.
#[test]
fn b_in_normal_mode_still_word_jumps() {
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut pending_count: Option<usize> = None;
    let mut app = panel_smoke_app();
    assert_eq!(app.mode, Mode::Normal);
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE),
    );
    assert_eq!(
        app.mode,
        Mode::Normal,
        "b must not open the switcher outside the focused panel"
    );
    assert!(app.switcher.is_none());
}

/// `Esc` inside the switcher modal closes it and restores the git panel's
/// cursor to the row it had before the modal opened, not wherever the
/// panel cursor happens to sit afterward.
#[test]
fn esc_restores_panel_cursor_row() {
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut pending_count: Option<usize> = None;
    let mut app = panel_smoke_app();
    app.apply(Action::FocusGitPanel);
    app.apply(Action::PanelCursorDown);
    app.apply(Action::PanelCursorDown);
    assert_eq!(app.panel_cursor(), 2);
    app.switcher = Some(super::switcher::SwitcherState::new(
        vec![],
        vec![],
        None,
        app.panel_cursor(),
    ));
    app.mode = Mode::Switcher;
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    );
    assert!(matches!(app.mode, Mode::Panel { .. }));
    assert_eq!(
        app.panel_cursor(),
        2,
        "Esc must restore the pre-open panel cursor row"
    );
}

/// `q` is inert inside the switcher modal, per the existing overlay rule —
/// it must not quit the session.
#[test]
fn q_is_inert_inside_switcher() {
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut pending_count: Option<usize> = None;
    let mut app = panel_smoke_app();
    app.switcher = Some(super::switcher::SwitcherState::new(vec![], vec![], None, 0));
    app.mode = Mode::Switcher;
    match dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
    ) {
        Flow::Quit(_) => panic!("q must not quit from inside the switcher modal"),
        Flow::Continue => {}
        Flow::OpenEditor { .. } => panic!("q must not open an editor from inside the switcher"),
    }
    assert_eq!(
        app.mode,
        Mode::Switcher,
        "q must not close the switcher modal"
    );
}

/// An open overlay never quits the app: `q` is inert while the help
/// overlay is up, and `?` still toggles it closed. Driven through
/// `dispatch_key`.
#[test]
fn q_is_inert_while_help_overlay_open() {
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut pending_count: Option<usize> = None;
    let mut app = panel_smoke_app();
    app.help.open = true;
    let flow = dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
    );
    assert!(
        matches!(flow, Flow::Continue),
        "q must not quit while help is open"
    );
    assert!(app.help.open, "q must not close the help overlay");
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
    );
    assert!(!app.help.open, "? still closes the help overlay");
}

/// `@` toggles the command-log pane from *both* the diff view (Normal)
/// and the focused git panel, driven through the real `dispatch_key`
/// path; when open the pane renders in the bottom-panel slot, showing a
/// nonzero-exit entry with its stderr.
#[test]
fn at_toggles_command_log_from_both_scopes_and_renders_in_bottom_slot() {
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut pending_count: Option<usize> = None;
    let mut app = panel_smoke_app();
    app.command_log.push(super::command_log::CommandLogEntry {
        command_line: "git push".to_string(),
        success: false,
        code: Some(1),
        stdout: String::new(),
        stderr: "! [rejected] main -> main (non-fast-forward)".to_string(),
    });
    let at = KeyEvent::new(KeyCode::Char('@'), KeyModifiers::NONE);
    let backtick = KeyEvent::new(KeyCode::Char('`'), KeyModifiers::NONE);

    // Diff scope: `@` opens the log.
    assert!(!app.command_log_open);
    dispatch_key(&mut app, &keymap, &mut pending, &mut pending_count, at);
    assert!(app.command_log_open);

    // It renders in the bottom slot with the failed entry and its stderr.
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &app, &keymap, None)).unwrap();
    let content: String = terminal
        .backend()
        .buffer()
        .clone()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(content.contains("command log"));
    assert!(content.contains("git push"));
    assert!(content.contains("exit 1"));
    assert!(content.contains("non-fast-forward"));

    // `@` again closes it.
    dispatch_key(&mut app, &keymap, &mut pending, &mut pending_count, at);
    assert!(!app.command_log_open);

    // Panel scope toggles it too: focus the panel, then `@`.
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        backtick,
    );
    assert!(matches!(app.mode, Mode::Panel { .. }));
    dispatch_key(&mut app, &keymap, &mut pending, &mut pending_count, at);
    assert!(app.command_log_open);
    // Still focused on the panel — the log toggle is orthogonal to focus.
    assert!(matches!(app.mode, Mode::Panel { .. }));
}

/// The running indicator shows in the footer while a remote op is in
/// flight (here, a stalled background task the test controls).
#[test]
fn running_indicator_renders_while_a_remote_op_is_in_flight() {
    let keymap = Keymap::default_map();
    let mut app = panel_smoke_app();
    // Spawn a task that blocks on a gate we never release, so the op stays
    // "in flight" for the duration of the render.
    let (_gate_tx, gate_rx) = std::sync::mpsc::channel::<()>();
    let id = app.background.spawn(move || {
        let _ = gate_rx.recv();
        super::background::CommandOutcome {
            success: true,
            code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
        }
    });
    app.git_op = Some(super::app::InFlightGitOp {
        id,
        kind: super::app::GitOpKind::Remote(crate::git::RemoteOp::Fetch),
        command_line: "git fetch".to_string(),
    });

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &app, &keymap, None)).unwrap();
    let content: String = terminal
        .backend()
        .buffer()
        .clone()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(
        content.contains("fetch"),
        "footer should show the running fetch indicator"
    );
}

/// With no search input, remote-op indicator, or transient status message
/// active, the footer falls back to the context-sensitive hint strip (see
/// `footer.rs`) — the git panel's only discoverability surface now that it's
/// hidden by default (besides the `?` help overlay), plus the rest of the
/// curated Normal-mode strip.
#[test]
fn context_footer_strip_renders_when_nothing_else_occupies_the_footer() {
    let keymap = Keymap::default_map();
    let app = App::new(vec![sample_file()]);

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &app, &keymap, None)).unwrap();
    let content: String = terminal
        .backend()
        .buffer()
        .clone()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(
        content.contains("git panel"),
        "footer strip should hint at the backtick binding"
    );
    assert!(
        content.contains("help"),
        "footer strip should hint at the help overlay"
    );
    assert!(
        content.contains("move"),
        "footer strip should hint at j/k movement"
    );
    assert!(
        content.contains("stage hunk"),
        "footer strip should hint at Space staging a hunk"
    );
}

/// A transient status message takes priority over the context footer strip —
/// the strip only shows once the message clears.
#[test]
fn status_message_replaces_the_context_footer_strip() {
    let keymap = Keymap::default_map();
    let mut app = App::new(vec![sample_file()]);
    app.set_status_message("staged hunk!!");

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &app, &keymap, None)).unwrap();
    let content: String = terminal
        .backend()
        .buffer()
        .clone()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(content.contains("staged hunk!!"));
    assert!(
        !content.contains("git panel"),
        "status message should displace the footer strip"
    );
}

// -- Config-warning notice ----------------------------------------------------

/// A loaded warning renders in the footer, naming the problem, without
/// blocking the diff content above it — and is never printed to stdout (this
/// is a `TestBackend` render, not process stdout, so nothing here could ever
/// reach it either way).
#[test]
fn config_warning_notice_renders_in_the_footer_without_blocking_the_diff() {
    let keymap = Keymap::default_map();
    let mut app = App::new(vec![sample_file()]);
    app.set_config(
        crate::config::Config::default(),
        vec![crate::config::ConfigWarning::SyntaxError {
            path: "/tmp/config.toml".to_string(),
            message: "TOML parse error at line 3".to_string(),
        }],
    );

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &app, &keymap, None)).unwrap();
    let content: String = terminal
        .backend()
        .buffer()
        .clone()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(content.contains("/tmp/config.toml"));
    assert!(content.contains("TOML parse error at line 3"));
    // Diff content still renders (main.rs's own file content), unblocked.
    assert!(content.contains("fn main"));
}

/// Multiple warnings show the first plus an "(and N more)" summary, per the
/// FR ("first problem + and N more").
#[test]
fn config_warning_notice_summarizes_additional_warnings() {
    let mut app = App::new(vec![sample_file()]);
    app.set_config(
        crate::config::Config::default(),
        vec![
            crate::config::ConfigWarning::UnknownKey {
                section: "layout".to_string(),
                key: "bogus".to_string(),
            },
            crate::config::ConfigWarning::InvalidValue {
                section: "search".to_string(),
                key: "case".to_string(),
                message: "expected \"smart\"".to_string(),
            },
        ],
    );
    let notice = app.config_warning_notice().expect("warnings present");
    assert!(notice.contains("bogus"));
    assert!(notice.contains("and 1 more"));
}

/// `!` (`Action::DismissConfigWarning`) clears the notice for the session;
/// the footer strip (or whatever else would show) resumes underneath.
#[test]
fn dismiss_config_warning_hides_the_notice() {
    let keymap = Keymap::default_map();
    let mut app = App::new(vec![sample_file()]);
    app.set_config(
        crate::config::Config::default(),
        vec![crate::config::ConfigWarning::SyntaxError {
            path: "/tmp/config.toml".to_string(),
            message: "boom".to_string(),
        }],
    );
    assert!(app.config_warning_visible());

    app.apply(Action::DismissConfigWarning);

    assert!(!app.config_warning_visible());
    assert_eq!(app.config_warning_notice(), None);

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &app, &keymap, None)).unwrap();
    let content: String = terminal
        .backend()
        .buffer()
        .clone()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(!content.contains("/tmp/config.toml"));
}

/// With no warnings collected, the notice never renders at all — the
/// no-config invariant (zero visible difference from today).
#[test]
fn no_warnings_means_no_notice() {
    let app = App::new(vec![sample_file()]);
    assert!(!app.config_warning_visible());
    assert_eq!(app.config_warning_notice(), None);
}

/// The footer reserves its one-row slot whenever the notice is visible, the
/// same way it already does for a running op or a transient status message
/// (`footer::footer_height`'s single computation both `draw` and the event
/// loop's viewport mirror share).
#[test]
fn footer_height_reserves_a_row_for_a_visible_config_warning() {
    let keymap = Keymap::default_map();
    let mut app = App::new(vec![sample_file()]);
    app.set_config(
        crate::config::Config::default(),
        vec![crate::config::ConfigWarning::SyntaxError {
            path: "/tmp/config.toml".to_string(),
            message: "boom".to_string(),
        }],
    );
    assert_eq!(footer::footer_height(100, &app, &keymap, None), 1);
}

/// Regenerates `02-proofs/02-task-03-smoke.txt` from a real key-dispatch
/// run, rendering a `TestBackend` frame at each step and recording the
/// observed state. Ignored by default (it writes into the repo); run with
/// `cargo test capture_task_03_smoke_transcript -- --ignored` to refresh.
#[test]
#[ignore = "writes the smoke transcript proof artifact; run explicitly"]
fn capture_task_03_smoke_transcript() {
    use std::fmt::Write as _;

    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut pending_count: Option<usize> = None;
    let mut app = panel_smoke_app();
    let backend = TestBackend::new(60, 20);
    let mut terminal = Terminal::new(backend).unwrap();

    let mut out = String::new();
    out.push_str("Task 3.0 smoke transcript — git panel focus & navigation\n");
    out.push_str("Driver: real crossterm KeyEvents through ui::dispatch_key\n");
    out.push_str("(the same handler the blocking event loop calls).\n");
    out.push_str("tmux unavailable on this host; headless TestBackend fallback.\n\n");

    let mut step = 0;
    // One flat closure: optionally dispatch a key, render a frame, and
    // record observed state. All mutable state is passed in as params so
    // nothing aliases across calls.
    let mut do_step = |app: &mut App,
                       pending: &mut Option<KeyEvent>,
                       terminal: &mut Terminal<TestBackend>,
                       out: &mut String,
                       code: Option<KeyCode>,
                       label: &str| {
        if let Some(code) = code {
            let _ = dispatch_key(
                app,
                &keymap,
                pending,
                &mut pending_count,
                KeyEvent::new(code, KeyModifiers::NONE),
            );
        }
        terminal.draw(|f| draw(f, app, &keymap, None)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let text: String = buf.content().iter().map(|c| c.symbol()).collect();
        let pane = if app.git_panel_focused() {
            "git panel"
        } else {
            "diff view"
        };
        writeln!(out, "step {step}: {label}").unwrap();
        writeln!(
            out,
            "  mode={:?} focus={pane} panel_cursor={} selected_file={} diff_cursor={}",
            app.mode,
            app.panel_cursor(),
            app.view.selected_file,
            app.view.cursor
        )
        .unwrap();
        writeln!(
            out,
            "  frame shows: CHANGES={} UNTRACKED={} STASHES={} branch_hdr={}",
            text.contains("CHANGES"),
            text.contains("UNTRACKED"),
            text.contains("STASHES"),
            text.contains("git: main")
        )
        .unwrap();
        out.push('\n');
        step += 1;
    };

    let steps: &[(Option<KeyCode>, &str)] = &[
        (None, "initial (diff focused)"),
        (Some(KeyCode::Char('`')), "press ` — focus git panel"),
        (
            Some(KeyCode::Char('j')),
            "press j — CHANGES row 2 (src/b.rs)",
        ),
        (
            Some(KeyCode::Char('j')),
            "press j — cross into UNTRACKED (notes.md)",
        ),
        (
            Some(KeyCode::Char('j')),
            "press j — cross into STASHES (stash 0)",
        ),
        (Some(KeyCode::Char('j')), "press j — STASHES (stash 1)"),
        (
            Some(KeyCode::Enter),
            "press Enter on stash — no-op, stays focused",
        ),
        (Some(KeyCode::Char('k')), "press k — back up"),
        (
            Some(KeyCode::Char('k')),
            "press k — onto UNTRACKED (notes.md)",
        ),
        (
            Some(KeyCode::Char('k')),
            "press k — onto CHANGES (src/b.rs)",
        ),
        (
            Some(KeyCode::Enter),
            "press Enter on file — jump diff, return focus",
        ),
        (
            Some(KeyCode::Char('j')),
            "press j (unfocused) — diff cursor scrolls",
        ),
        (
            Some(KeyCode::Char('s')),
            "press s (unfocused) — staging panel opens",
        ),
        (Some(KeyCode::Char('s')), "press s — staging panel closes"),
    ];
    for (code, label) in steps {
        do_step(
            &mut app,
            &mut pending,
            &mut terminal,
            &mut out,
            *code,
            label,
        );
    }

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("docs/specs/02-spec-git-panel/02-proofs/02-task-03-smoke.txt");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, out).unwrap();
}

/// Regenerates `02-proofs/02-task-04-smoke.txt` from two real runs:
/// (a) a deliberately slow (2s) background command driven through the real
/// `BackgroundTasks` path while `j`/`k` scroll the diff via `dispatch_key`,
/// with timestamped observations proving the render loop never blocks; and
/// (b) a real non-fast-forward push rejection against a `file://` remote,
/// driven through the real spawn -> poll -> command-log pipeline, with the
/// command-log pane rendered showing git's stderr and nonzero exit.
///
/// tmux is unavailable on this host, so this headless driver stands in for
/// the manual transcript. Ignored by default (it writes into the repo, runs
/// a 2s sleep, and shells out to git); run with
/// `cargo test capture_task_04_smoke_transcript -- --ignored`.
#[test]
#[ignore = "writes the task-04 smoke transcript; 2s sleep + real git. run explicitly"]
fn capture_task_04_smoke_transcript() {
    use super::app::{GitOpKind, InFlightGitOp};
    use super::background::run_command;
    use crate::git::RemoteOp;
    use std::fmt::Write as _;
    use std::process::Command as PCommand;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    fn git(dir: &std::path::Path, args: &[&str]) {
        let out = PCommand::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .expect("spawn git");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let mut out = String::new();
    let wall = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    out.push_str("Task 4.0 smoke transcript — async remote ops & command log\n");
    out.push_str("Driver: real BackgroundTasks + ui::dispatch_key (the handlers\n");
    out.push_str("the blocking event loop calls). tmux unavailable; headless.\n");
    writeln!(out, "Wall-clock start (unix seconds): {wall}\n").unwrap();

    // -- Part (a): non-blocking proof ---------------------------------
    out.push_str("== Part (a): render loop stays responsive during a slow op ==\n");
    out.push_str("A 2s `sleep` runs on the real background poller while j/k drive\n");
    out.push_str("the diff cursor through dispatch_key. Each keypress is timed from\n");
    out.push_str("op-spawn; all land in single-digit ms, far under the 2000ms op.\n\n");

    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut pending_count: Option<usize> = None;
    let mut app = panel_smoke_app();

    let spawn_at = Instant::now();
    let id = app.background.spawn(|| {
        let mut cmd = PCommand::new("sh");
        cmd.args(["-c", "sleep 2"]);
        run_command(&mut cmd)
    });
    app.git_op = Some(InFlightGitOp {
        id,
        kind: GitOpKind::Remote(RemoteOp::Fetch),
        command_line: RemoteOp::Fetch.command_line(),
    });
    writeln!(
        out,
        "t=+{:>4}ms spawned slow op (git_op={:?}, running_label={:?})",
        spawn_at.elapsed().as_millis(),
        app.git_op.as_ref().map(|o| o.kind),
        app.running_op_label(),
    )
    .unwrap();

    let motions = [
        (KeyCode::Char('j'), "j"),
        (KeyCode::Char('j'), "j"),
        (KeyCode::Char('j'), "j"),
        (KeyCode::Char('k'), "k"),
    ];
    for (code, label) in motions {
        let before = app.view.cursor;
        dispatch_key(
            &mut app,
            &keymap,
            &mut pending,
            &mut pending_count,
            KeyEvent::new(code, KeyModifiers::NONE),
        );
        // Poll for completion the way the event loop does; the op is still
        // sleeping, so nothing drains — the guard stays set, log empty.
        app.poll_git_ops();
        let still_pending = app.git_op.is_some() && app.command_log.is_empty();
        assert!(still_pending, "op must still be in flight during scrolling");
        writeln!(
            out,
            "t=+{:>4}ms press {label}: diff_cursor {before}->{} | op still pending={} log_len={}",
            spawn_at.elapsed().as_millis(),
            app.view.cursor,
            still_pending,
            app.command_log.len(),
        )
        .unwrap();
    }
    assert!(
        spawn_at.elapsed() < Duration::from_millis(1500),
        "all scrolling completed well before the 2s op"
    );
    writeln!(
        out,
        "\nObservation: all {} keypresses processed by t=+{}ms while the\n\
             2000ms op was still pending — dispatch never blocked on it.",
        motions.len(),
        spawn_at.elapsed().as_millis()
    )
    .unwrap();

    // Let the op finish and drain it, recording when.
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.git_op.is_some() && Instant::now() < deadline {
        app.poll_git_ops();
        std::thread::sleep(Duration::from_millis(10));
    }
    writeln!(
        out,
        "t=+{:>4}ms op completed and drained (log_len={})\n",
        spawn_at.elapsed().as_millis(),
        app.command_log.len()
    )
    .unwrap();

    // -- Part (b): failure transparency -------------------------------
    out.push_str("== Part (b): a non-fast-forward push rejection is logged ==\n");
    out.push_str("A file:// bare remote is advanced by a second clone; the local\n");
    out.push_str("clone commits too, so `git push` is rejected non-fast-forward.\n");
    out.push_str("Driven through App::request_remote_op -> poll_git_ops (real spawn).\n\n");

    let bare = tempfile::TempDir::new().unwrap();
    // Branch must be explicit: the host's init.defaultBranch config must not
    // leak into the fixture. A bare repo's HEAD (e.g. `master` on a host
    // without init.defaultBranch set) that never has a matching branch
    // pushed to it leaves clones with a dangling HEAD and no working tree.
    git(bare.path(), &["init", "-q", "--bare", "-b", "main"]);
    let bare_url = format!("file://{}", bare.path().display());

    let repo = tempfile::TempDir::new().unwrap();
    git(repo.path(), &["init", "-q"]);
    git(repo.path(), &["config", "user.name", "redquill test"]);
    git(
        repo.path(),
        &["config", "user.email", "test@redquill.invalid"],
    );
    git(repo.path(), &["branch", "-M", "main"]);
    std::fs::write(repo.path().join("base.txt"), b"one\n").unwrap();
    git(repo.path(), &["add", "."]);
    git(repo.path(), &["commit", "-qm", "initial"]);
    git(repo.path(), &["remote", "add", "origin", &bare_url]);
    git(repo.path(), &["push", "-q", "-u", "origin", "main"]);

    // A second clone advances origin/main out from under `repo`.
    let parent = tempfile::TempDir::new().unwrap();
    git(parent.path(), &["clone", "-q", &bare_url, "clone2"]);
    let clone2 = parent.path().join("clone2");
    git(&clone2, &["config", "user.name", "redquill test"]);
    git(&clone2, &["config", "user.email", "test@redquill.invalid"]);
    std::fs::write(clone2.join("base.txt"), b"one\nremote two\n").unwrap();
    git(&clone2, &["commit", "-aqm", "remote commit"]);
    git(&clone2, &["push", "-q", "origin", "main"]);

    // The local clone commits its own divergent history.
    std::fs::write(repo.path().join("base.txt"), b"one\nlocal two\n").unwrap();
    git(repo.path(), &["commit", "-aqm", "local commit"]);

    let mut app2 = panel_smoke_app();
    app2.set_repo_root(repo.path().to_path_buf());
    app2.request_remote_op(RemoteOp::Push);
    writeln!(
        out,
        "spawned push (running_label={:?})",
        app2.running_op_label()
    )
    .unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    while app2.command_log.is_empty() && Instant::now() < deadline {
        app2.poll_git_ops();
        std::thread::sleep(Duration::from_millis(10));
    }
    let entry = app2
        .command_log
        .entries()
        .next()
        .expect("push should have been logged")
        .clone();
    assert!(!entry.success, "the non-ff push must be recorded as failed");
    writeln!(
        out,
        "logged: command_line={:?} exit_status={:?} success={}",
        entry.command_line,
        entry.exit_status(),
        entry.success
    )
    .unwrap();
    writeln!(out, "stderr (verbatim from git):").unwrap();
    for line in entry.stderr.lines() {
        writeln!(out, "    {line}").unwrap();
    }

    // Render the command-log pane and capture what the user would see.
    app2.command_log_open = true;
    let backend = TestBackend::new(100, 40);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &app2, &keymap, None)).unwrap();
    let frame: String = terminal
        .backend()
        .buffer()
        .clone()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(frame.contains("command log"), "pane must be visible");
    assert!(
        frame.contains("git push") && frame.contains("exit"),
        "pane must show the failed push and its exit status"
    );
    let shows_reject = frame.contains("rejected") || frame.contains("fast-forward");
    writeln!(
        out,
        "\ncommand-log pane visible: {} | shows rejection text: {}",
        frame.contains("command log"),
        shows_reject
    )
    .unwrap();
    out.push_str("Observation: the rejected push did not crash the tool; it landed\n");
    out.push_str("in the command log with its nonzero exit and git's own stderr.\n");

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("docs/specs/02-spec-git-panel/02-proofs/02-task-04-smoke.txt");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, out).unwrap();
}

/// Builds a ~5k-changed-line, 15-file multibuffer and scrolls it top to
/// bottom a half-page at a time through the real `draw` render path on a
/// `TestBackend`, reporting ms/frame. The spec's quantitative proxy for
/// "instant-feel scrolling" is ms/frame well under 16ms; the assertion
/// uses a generous CI-safe bound (real measured value, recorded in the
/// perf proof, is far lower). Run with `--nocapture` to see the numbers.
#[test]
fn scrolling_a_5k_line_multibuffer_renders_fast() {
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
    let total_rows = app.view.rows.len();
    let keymap = Keymap::default_map();
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).unwrap();
    app.view.set_viewport_height(38);

    let mut frames = 0u32;
    let start = std::time::Instant::now();
    loop {
        terminal
            .draw(|frame| draw(frame, &app, &keymap, None))
            .unwrap();
        frames += 1;
        if app.view.cursor >= app.view.max_cursor() || frames > 2000 {
            break;
        }
        app.apply(Action::HalfPageDown);
    }
    let per_frame = start.elapsed() / frames;
    println!(
        "scroll: {frames} frames over {total_rows} rows ({total_lines} changed lines), {per_frame:?}/frame"
    );
    assert!(
        per_frame < std::time::Duration::from_millis(50),
        "ms/frame {per_frame:?} too slow over {frames} frames / {total_rows} rows"
    );
}

// -- Review launcher journeys: `R` opens it from anywhere, `Esc` restores ---
//
// `R` from the diff view and from the git panel (cursor mid-list on the
// non-default History tab) both open the Review launcher; `Esc` restores the
// exact prior focus either way.

/// Prints a rendered frame's non-blank rows to stderr when
/// `REDQUILL_PROOF_DUMP` is set — the proof-capture convention this file's
/// other render tests already use inline, factored out once here since both
/// journey tests below need it.
fn dump_frame_if_requested(label: &str, app: &App, keymap: &Keymap) {
    if std::env::var_os("REDQUILL_PROOF_DUMP").is_none() {
        return;
    }
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| draw(frame, app, keymap, None))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let w = buffer.area.width as usize;
    let symbols: Vec<&str> = buffer.content().iter().map(|c| c.symbol()).collect();
    eprintln!("-- {label} --");
    for row in symbols.chunks(w) {
        let line = row.concat();
        if !line.trim().is_empty() {
            eprintln!("{line}");
        }
    }
}

/// Diff-view leg: from `Mode::Normal`, `R` opens the launcher
/// (landing on Branches, the default tab) and `Esc` restores the exact prior
/// mode. Driven through the real `dispatch_key` pipeline, the same handler
/// the blocking event loop calls.
#[test]
fn journey_r_from_diff_view_opens_launcher_and_esc_restores() {
    let mut app = App::new(vec![sample_file()]);
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut pending_count: Option<usize> = None;

    assert_eq!(app.mode, Mode::Normal);
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE),
    );
    assert_eq!(
        app.mode,
        Mode::ReviewLauncher {
            tab: LauncherTab::Branches,
            cursor: 0,
            origin: ModeOrigin::Normal,
        },
        "R from the diff view opens the launcher on the default tab"
    );
    dump_frame_if_requested(
        "R from the diff view opens the Review launcher",
        &app,
        &keymap,
    );

    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    );
    assert_eq!(app.mode, Mode::Normal, "Esc restores the exact prior focus");
    dump_frame_if_requested("Esc restores the diff view", &app, &keymap);
}

/// Git-panel leg: with the panel focused on the History tab and
/// its cursor mid-list (neither the top nor the last loaded row), `R` opens
/// the launcher and `Esc` restores the panel with its cursor and tab exactly
/// intact — the non-default-tab case.
#[test]
fn journey_r_from_panel_mid_list_history_tab_opens_launcher_and_esc_restores_cursor_and_tab() {
    let mut app = App::new(vec![sample_file()]);
    app.mode = Mode::Panel {
        cursor: 0,
        tab: PanelTab::History,
    };
    app.history = vec![
        CommitLogEntry {
            sha: "aaa1111full".to_string(),
            short_sha: "aaa1111".to_string(),
            subject: "third".to_string(),
            author_name: "Dev".to_string(),
            timestamp: 1_700_000_002,
        },
        CommitLogEntry {
            sha: "bbb2222full".to_string(),
            short_sha: "bbb2222".to_string(),
            subject: "second".to_string(),
            author_name: "Dev".to_string(),
            timestamp: 1_700_000_001,
        },
        CommitLogEntry {
            sha: "ccc3333full".to_string(),
            short_sha: "ccc3333".to_string(),
            subject: "first".to_string(),
            author_name: "Dev".to_string(),
            timestamp: 1_700_000_000,
        },
    ];
    app.history_exhausted = true;
    app.panel_move_down(); // cursor -> 1, mid-list
    assert_eq!(app.panel_cursor(), 1);

    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut pending_count: Option<usize> = None;

    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE),
    );
    assert_eq!(
        app.mode,
        Mode::ReviewLauncher {
            tab: LauncherTab::Branches,
            cursor: 0,
            origin: ModeOrigin::Panel {
                cursor: 1,
                tab: PanelTab::History,
            },
        },
        "R from the panel captures its cursor/tab as the restore origin"
    );
    dump_frame_if_requested(
        "R from the git panel (History tab, cursor mid-list) opens the Review launcher",
        &app,
        &keymap,
    );

    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    );
    assert_eq!(
        app.mode,
        Mode::Panel {
            cursor: 1,
            tab: PanelTab::History,
        },
        "Esc restores the panel with its cursor and tab exactly intact"
    );
    dump_frame_if_requested(
        "Esc restores the git panel, cursor and tab intact",
        &app,
        &keymap,
    );
}
