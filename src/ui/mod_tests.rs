use std::path::PathBuf;

use super::*;
use crate::annotate::{Classification, Target};
use crate::config::SidebarSide;
use crate::diff::FileDiff;
use crate::git::{CommitLogEntry, DiffTarget, RawFilePatch, RemoteOp};
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
        ..Default::default()
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

/// On a read-only range target the help overlay omits the inert
/// file/hunk staging gestures, but keeps the still-working staging-panel
/// toggle.
#[test]
fn help_overlay_hides_staging_rows_on_a_range_target() {
    // Tall enough that the overlay's ~3/5-of-screen cap still fits the whole
    // This context list (workflow header + Navigation..Quit groups).
    let backend = TestBackend::new(100, 74);
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
    // Tall enough that the overlay's ~3/5-of-screen cap still fits the whole
    // This context list (workflow header + Navigation..Quit groups) — every
    // row added to a diff-scope group needs ~5/3 more screen rows here.
    let backend = TestBackend::new(100, 92);
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
        motion::MAX_COUNT,
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
    // `y` is a diff-scope key no default binds (`x` is now `DeleteAnnotation`),
    // so remapping toggle-stage onto it stays collision-free.
    keys.diff.insert(
        "toggle-stage".to_string(),
        vec![crate::config::keys::KeySeqSpec::One(
            crate::config::keys::ChordSpec {
                code: crossterm::event::KeyCode::Char('y'),
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
        content.contains("press y on a hunk to stage it"),
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

// -- Search ---------------------------------------------------------------

// -- Column cursor ---------------------------------------------------------

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

    // Focus the panel: the cursor seats on the file the diff already shows
    // (`selected_file` starts at 0 — src/a.rs, row 1, past the `src`
    // directory row). Step back up onto the directory row to walk the whole
    // tree from the top.
    assert_eq!(app.mode, Mode::Normal);
    press(&mut app, &mut pending, KeyCode::Char('`'));
    assert!(matches!(app.mode, Mode::Panel { .. }));
    assert_eq!(app.panel_cursor(), 1); // src/a.rs, the selected file
    assert_eq!(app.view.selected_file, 0); // src/a.rs
    press(&mut app, &mut pending, KeyCode::Char('k'));
    assert_eq!(app.panel_cursor(), 0); // the src/ directory row

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

/// The shared motion layer's count prefix works in panel scope exactly like
/// diff scope: `3j` steps three rows in one gesture, `Ctrl-d`/`Ctrl-u` page,
/// `g`/`G` jump to the row extremes (single `g`, not the diff view's `gg` —
/// see `motion`'s module doc), and Esc mid-count cancels it rather than also
/// closing the panel.
#[test]
fn panel_motion_layer_supports_counts_and_jumps() {
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;
    let mut pending_count: Option<usize> = None;
    let mut app = App::new((0..30).map(|i| named_file(&format!("f{i}.rs"))).collect());
    let mut press = |app: &mut App, code: KeyCode| {
        let _ = dispatch_key(
            app,
            &keymap,
            &mut pending,
            &mut pending_count,
            KeyEvent::new(code, KeyModifiers::NONE),
        );
    };
    press(&mut app, KeyCode::Char('`')); // focus the panel
    assert!(matches!(app.mode, Mode::Panel { .. }));
    assert_eq!(app.panel_cursor(), 0);

    // `3j` steps three rows in one gesture.
    press(&mut app, KeyCode::Char('3'));
    assert_eq!(app.panel_cursor(), 0, "digits accumulate, no move yet");
    press(&mut app, KeyCode::Char('j'));
    assert_eq!(app.panel_cursor(), 3);

    // `G` jumps to the last row; `g` jumps back to the first.
    press(&mut app, KeyCode::Char('G'));
    assert_eq!(app.panel_cursor(), 29);
    press(&mut app, KeyCode::Char('g'));
    assert_eq!(app.panel_cursor(), 0);

    // Esc mid-count cancels the count without closing the panel.
    press(&mut app, KeyCode::Char('5'));
    press(&mut app, KeyCode::Esc);
    assert!(
        matches!(app.mode, Mode::Panel { .. }),
        "Esc must cancel the count, not close the panel"
    );
    press(&mut app, KeyCode::Char('j'));
    assert_eq!(app.panel_cursor(), 1, "the cancelled count must not apply");

    // A bare Esc (nothing pending) still closes the panel.
    press(&mut app, KeyCode::Esc);
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
fn shift_d_defers_the_cursor_file_in_a_review_session_via_dispatch_key() {
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
        KeyEvent::new(KeyCode::Char('D'), KeyModifiers::NONE),
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
fn shift_d_is_a_total_no_op_outside_a_review_session_via_dispatch_key() {
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
        KeyEvent::new(KeyCode::Char('D'), KeyModifiers::NONE),
    );

    assert_eq!(app.review_status("src/main.rs"), ReviewStatus::Unreviewed);
    assert!(!app.view.is_collapsed("src/main.rs"));
    assert!(app.status_message.is_none());
    assert_eq!(app.mode, Mode::Normal);
}

/// Plain `d` is the restore confirm, not defer: with no git backend wired it
/// declines with a footer hint and opens nothing, rather than silently
/// pretending to have restored something.
#[test]
fn d_without_a_git_backend_declines_instead_of_opening_the_restore_confirm() {
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

    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(
        app.status_message.as_deref(),
        Some("restore unavailable (no git backend)")
    );
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

// -- Accepted-files panel -----------------------------------------------------

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
    app.apply(Action::FocusGitPanel); // seats on src/a.rs (row 1)
    app.apply(Action::PanelCursorDown);
    app.apply(Action::PanelCursorDown);
    assert_eq!(app.panel_cursor(), 3);
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
        3,
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

// -- Config-warning notice ----------------------------------------------------

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

// -- Panel file actions: review-session translation through real dispatch ----

/// Presses one key through `dispatch_key` with no pending prefix/count.
fn press_one(app: &mut App, keymap: &Keymap, code: KeyCode) {
    let mut pending = None;
    let mut pending_count = None;
    let _ = dispatch_key(
        app,
        keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(code, KeyModifiers::NONE),
    );
}

/// From the focused git panel during a review session, `Space`/`S`
/// translate to the accept gestures and `d` toggle-defers — the same
/// translation the diff view's dispatch applies, now reachable without
/// leaving the panel.
#[test]
fn panel_keys_translate_to_accept_and_defer_in_a_review_session() {
    let mut app = App::new(vec![sample_file()]); // src/main.rs
    app.target = DiffTarget::Review {
        base: "main".to_string(),
        branch: "feature".to_string(),
    };
    let keymap = Keymap::default_map();
    press_one(&mut app, &keymap, KeyCode::Char('`'));
    assert!(matches!(app.mode, Mode::Panel { .. }));
    press_one(&mut app, &keymap, KeyCode::Char('j')); // Dir("src") -> File
    press_one(&mut app, &keymap, KeyCode::Char(' '));
    assert_eq!(app.review_status("src/main.rs"), ReviewStatus::Accepted);
    press_one(&mut app, &keymap, KeyCode::Char(' '));
    assert_eq!(app.review_status("src/main.rs"), ReviewStatus::Unreviewed);
    press_one(&mut app, &keymap, KeyCode::Char('S'));
    assert_eq!(app.review_status("src/main.rs"), ReviewStatus::Accepted);
    press_one(&mut app, &keymap, KeyCode::Char('D'));
    assert_eq!(
        app.review_status("src/main.rs"),
        ReviewStatus::Deferred,
        "D replaces the accepted status with deferred"
    );
    assert!(
        matches!(app.mode, Mode::Panel { .. }),
        "the panel keeps focus across every gesture"
    );
}

/// Outside a review session the panel's `D` stays a total no-op, exactly
/// like the diff view's unconditional `D` binding.
#[test]
fn panel_shift_d_stays_inert_outside_a_review_session_through_dispatch() {
    let mut app = App::new(vec![sample_file()]);
    let keymap = Keymap::default_map();
    press_one(&mut app, &keymap, KeyCode::Char('`'));
    press_one(&mut app, &keymap, KeyCode::Char('j'));
    press_one(&mut app, &keymap, KeyCode::Char('D'));
    assert!(app.review_states.is_empty());
    assert!(app.status_message.is_none());
}

/// The `?` overlay from the focused git panel documents the panel's
/// per-file keys with the same mutual exclusion as the diff view: a plain
/// session shows the stage rows and hides accept/defer; a review session
/// (read-only target) shows accept/defer and hides the stage rows.
#[test]
fn help_from_the_panel_swaps_stage_rows_for_accept_rows_in_a_review_session() {
    let keymap = Keymap::default_map();
    let render_help = |app: &App| -> String {
        let backend = TestBackend::new(100, 300);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| draw(frame, app, &keymap, None))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .clone()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    };

    let mut app = App::new(vec![sample_file()]);
    app.mode = Mode::Panel {
        cursor: 0,
        tab: PanelTab::Changes,
    };
    app.apply(Action::ToggleHelp);
    let plain = render_help(&app);
    assert!(plain.contains("Stage the highlighted file"));
    assert!(plain.contains("Stage/unstage the highlighted file"));
    assert!(!plain.contains("Accept/un-accept the highlighted file"));
    assert!(!plain.contains("Defer/un-defer the highlighted file"));

    let mut review = App::new(vec![sample_file()]);
    review.target = DiffTarget::Review {
        base: "main".to_string(),
        branch: "feature".to_string(),
    };
    review.mode = Mode::Panel {
        cursor: 0,
        tab: PanelTab::Changes,
    };
    review.apply(Action::ToggleHelp);
    let reviewing = render_help(&review);
    assert!(reviewing.contains("Accept/un-accept the highlighted file"));
    assert!(reviewing.contains("Accept the highlighted file"));
    assert!(reviewing.contains("Defer/un-defer the highlighted file"));
    assert!(!reviewing.contains("Stage the highlighted file"));
}

// -- Panel coherence: Esc leaves, s and / reach through (spec 11 Unit 2) -----

/// `Esc` from the focused git panel closes it, landing back in `Normal` —
/// the same destination `` ` `` already reaches, just via the app's
/// universal "back out" key instead of the panel-specific toggle.
#[test]
fn panel_esc_closes_the_panel_to_normal() {
    let mut app = App::new(vec![sample_file()]);
    let keymap = Keymap::default_map();
    press_one(&mut app, &keymap, KeyCode::Char('`'));
    assert!(matches!(app.mode, Mode::Panel { .. }));
    press_one(&mut app, &keymap, KeyCode::Esc);
    assert_eq!(app.mode, Mode::Normal, "Esc must close the panel to Normal");
}

/// An open help overlay shadows panel dispatch entirely (the existing
/// `dispatch_key` arm at the top of the `Mode::Panel` match), so `Esc` while
/// help is open over the panel closes help first, leaving the panel
/// focused underneath — a second `Esc` is then needed to leave the panel.
#[test]
fn panel_esc_is_shadowed_by_an_open_help_overlay() {
    let mut app = App::new(vec![sample_file()]);
    let keymap = Keymap::default_map();
    press_one(&mut app, &keymap, KeyCode::Char('`'));
    press_one(&mut app, &keymap, KeyCode::Char('?'));
    assert!(app.help.open, "? must open help over the focused panel");
    press_one(&mut app, &keymap, KeyCode::Esc);
    assert!(!app.help.open, "Esc closes the help overlay first");
    assert!(
        matches!(app.mode, Mode::Panel { .. }),
        "the panel stays focused underneath the help overlay Esc closed"
    );
    press_one(&mut app, &keymap, KeyCode::Esc);
    assert_eq!(
        app.mode,
        Mode::Normal,
        "a second Esc, with help already closed, now closes the panel"
    );
}

/// `s` from the focused panel behaves as if the panel were closed first: it
/// lands in the staging panel exactly like the diff view's own `s`, rather
/// than doing nothing (`toggle_staging_panel` no-ops while `Mode::Panel` is
/// active — see `staging.rs`).
#[test]
fn panel_s_reaches_the_staging_panel() {
    let mut app = App::new(vec![sample_file()]);
    let keymap = Keymap::default_map();
    press_one(&mut app, &keymap, KeyCode::Char('`'));
    press_one(&mut app, &keymap, KeyCode::Char('s'));
    assert_eq!(
        app.mode,
        Mode::Staging,
        "s must open the staging panel, not no-op inside the git panel"
    );
}

/// `/` from the focused panel behaves as if the panel were closed first: it
/// lands in `Mode::Search`, exactly where the diff view's own `/` lands, and
/// confirming a match returns focus to `Normal` — the panel is not restored.
#[test]
fn panel_slash_reaches_search_and_confirm_lands_in_normal() {
    let mut app = App::new(vec![sample_file()]);
    let keymap = Keymap::default_map();
    press_one(&mut app, &keymap, KeyCode::Char('`'));
    press_one(&mut app, &keymap, KeyCode::Char('/'));
    assert_eq!(
        app.mode,
        Mode::Search,
        "/ must open search, not no-op inside the git panel"
    );
    press_one(&mut app, &keymap, KeyCode::Char('n'));
    press_one(&mut app, &keymap, KeyCode::Char('e'));
    press_one(&mut app, &keymap, KeyCode::Char('w'));
    press_one(&mut app, &keymap, KeyCode::Enter);
    assert_eq!(
        app.mode,
        Mode::Normal,
        "confirming a search returns to Normal, not back to the panel"
    );
}
