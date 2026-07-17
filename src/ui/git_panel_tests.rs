use super::super::stage_ops::{StagedFile, StagedState};
use super::*;
use crate::diff::FileDiff;
use crate::git::{RawFilePatch, StashEntry};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use std::collections::HashMap;

fn sample_file(path: &str) -> FileDiff {
    let raw = format!(
        "diff --git a/{path} b/{path}\n\
         index 111..222 100644\n\
         --- a/{path}\n\
         +++ b/{path}\n\
         @@ -1,1 +1,1 @@\n\
         -old\n\
         +new\n"
    );
    FileDiff::from_patch(&RawFilePatch {
        path: path.to_string(),
        old_path: None,
        raw,
        is_binary: false,
    })
    .unwrap()
}

/// Renders `app`'s panel to a 32x24 `TestBackend` and returns the flat
/// buffer text.
fn render_panel(app: &App) -> String {
    render_panel_with_keymap(app, &Keymap::default_map())
}

/// [`render_panel`], but over an explicit keymap — used to prove the
/// remote-op hint line resolves keys dynamically rather than hardcoding
/// `f`/`p`/`P`.
fn render_panel_with_keymap(app: &App, keymap: &Keymap) -> String {
    let backend = TestBackend::new(32, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let area = Rect::new(0, 0, 32, 24);
    terminal
        .draw(|frame| render(frame, area, app, keymap))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect()
}

fn branch(name: &str, upstream: Option<&str>, ab: Option<(u32, u32)>) -> BranchStatus {
    BranchStatus {
        name: name.to_string(),
        detached: false,
        upstream: upstream.map(|s| s.to_string()),
        ahead_behind: ab,
    }
}

#[test]
fn header_shows_branch_name_and_ahead_behind_with_upstream() {
    let mut app = App::new(vec![sample_file("session.rs")]);
    app.branch = Some(branch("main", Some("origin/main"), Some((2, 1))));
    let content = render_panel(&app);
    assert!(content.contains("git: main"));
    assert!(content.contains("\u{2191}2\u{2193}1"));
}

#[test]
fn header_detached_head_shows_short_oid_without_arrows() {
    let mut app = App::new(vec![sample_file("session.rs")]);
    app.branch = Some(BranchStatus {
        name: "85d7cc5".to_string(),
        detached: true,
        upstream: None,
        ahead_behind: None,
    });
    let content = render_panel(&app);
    assert!(content.contains("git: 85d7cc5"));
    assert!(!content.contains("\u{2191}"));
    assert!(!content.contains("\u{2193}"));
}

#[test]
fn header_no_upstream_shows_no_arrows() {
    let mut app = App::new(vec![sample_file("session.rs")]);
    app.branch = Some(branch("feature", None, None));
    let content = render_panel(&app);
    assert!(content.contains("git: feature"));
    assert!(!content.contains("\u{2191}"));
    assert!(!content.contains("\u{2193}"));
}

#[test]
fn zero_ahead_behind_shows_no_arrows() {
    let mut app = App::new(vec![sample_file("session.rs")]);
    app.branch = Some(branch("main", Some("origin/main"), Some((0, 0))));
    let content = render_panel(&app);
    assert!(content.contains("git: main"));
    assert!(!content.contains("\u{2191}"));
    assert!(!content.contains("\u{2193}"));
}

// -- File tree rows -------------------------------------------------------

#[test]
fn file_row_shows_staged_marker_name_and_change_letter() {
    let mut app = App::new(vec![sample_file("session.rs")]);
    app.branch = Some(branch("main", Some("origin/main"), Some((2, 1))));
    app.staged = vec![StagedFile {
        path: "session.rs".to_string(),
        letter: 'M',
    }];
    app.staged_states = HashMap::from([("session.rs".to_string(), StagedState::Full)]);
    let content = render_panel(&app);
    assert!(content.contains("session.rs"));
    assert!(content.contains("\u{25cf}")); // staged dot preserved
    assert!(content.contains('M')); // change-kind letter, right-aligned
}

#[test]
fn accepted_file_renders_the_staged_dot() {
    // Renders as the staged ● — see theme.rs's staged_indicator rationale.
    let mut app = App::new(vec![sample_file("session.rs")]);
    app.target = crate::git::DiffTarget::Review {
        base: "main".to_string(),
        branch: "feature".to_string(),
    };
    app.apply(Action::ToggleAccept);
    let content = render_panel(&app);
    assert!(content.contains("\u{25cf}")); // the reused staged dot
    assert!(!content.contains('~'));
}

#[test]
fn deferred_file_renders_a_distinct_marker() {
    let mut app = App::new(vec![sample_file("session.rs")]);
    app.target = crate::git::DiffTarget::Review {
        base: "main".to_string(),
        branch: "feature".to_string(),
    };
    app.apply(Action::ToggleDefer);
    let content = render_panel(&app);
    assert!(content.contains('~'));
}

#[test]
fn tracked_and_untracked_files_share_one_tree_without_section_headers() {
    let mut app = App::new(vec![sample_file("session.rs"), sample_file("notes.md")]);
    app.untracked_paths = vec!["notes.md".to_string()];
    let content = render_panel(&app);
    // Both files appear, and the old CHANGES/UNTRACKED section headers are gone.
    assert!(content.contains("session.rs"));
    assert!(content.contains("notes.md"));
    assert!(!content.contains("CHANGES"));
    assert!(!content.contains("UNTRACKED"));
    // The untracked file carries the `?` change letter.
    assert!(content.contains('?'));
}

#[test]
fn files_nest_under_a_directory_row() {
    let app = App::new(vec![sample_file("src/session.rs")]);
    let content = render_panel(&app);
    assert!(content.contains("src"));
    assert!(content.contains("session.rs"));
    // The directory row and the file row are distinct navigable rows.
    assert_eq!(
        navigable_rows(&app),
        vec![PanelRow::Dir("src".to_string()), PanelRow::File(0)]
    );
    // Nested rows draw box-drawing tree guides rather than blank indentation.
    assert!(content.contains('\u{251c}') || content.contains('\u{2514}')); // ├ or └
}

/// The guide helper draws vertical bars under continuing ancestors and a
/// `├`/`└` connector per row: a directory whose sibling still follows keeps a
/// `│` running down its column, and the last child gets a `└`.
#[test]
fn tree_guides_connect_ancestors_and_mark_last_children() {
    let app = App::new(vec![
        sample_file("src/a.rs"),
        sample_file("src/b.rs"),
        sample_file("z.rs"),
    ]);
    let rows = super::panel_tree_rows(&app);
    let guides = super::tree_guides(&rows);
    // Rows: src(dir), src/a.rs, src/b.rs, z.rs.
    assert_eq!(guides[0], "\u{251c} "); // src: not last (z.rs follows) -> ├
    assert_eq!(guides[1], "\u{2502} \u{251c} "); // a.rs: under src (│), not last -> ├
    assert_eq!(guides[2], "\u{2502} \u{2514} "); // b.rs: under src (│), last -> └
    assert_eq!(guides[3], "\u{2514} "); // z.rs: last root row -> └
}

// -- Stashes (bottom-pinned, passive) -------------------------------------

fn app_with_stashes() -> App {
    let mut app = App::new(vec![sample_file("session.rs")]);
    app.stashes = vec![
        StashEntry {
            stash_ref: "stash@{0}".to_string(),
            branch: Some("main".to_string()),
            message: "wip: parser".to_string(),
        },
        StashEntry {
            stash_ref: "stash@{1}".to_string(),
            branch: Some("main".to_string()),
            message: "spike: tabs".to_string(),
        },
    ];
    app
}

#[test]
fn stashes_render_with_a_counted_header_and_indexed_rows() {
    let content = render_panel(&app_with_stashes());
    assert!(content.contains("STASHES (2)"));
    assert!(content.contains("0 wip: parser"));
    assert!(content.contains("1 spike: tabs"));
}

/// Stashes are no longer part of the navigable set — the panel cursor only
/// visits directory and file rows.
#[test]
fn stashes_are_not_navigable() {
    let app = app_with_stashes();
    let rows = navigable_rows(&app);
    assert_eq!(rows, vec![PanelRow::File(0)]);
}

#[test]
fn empty_stashes_hide_the_stashes_section() {
    let app = App::new(vec![sample_file("session.rs")]);
    let content = render_panel(&app);
    assert!(!content.contains("STASHES"));
}

#[test]
fn footer_shows_file_and_staged_counts() {
    let mut app = App::new(vec![sample_file("session.rs")]);
    app.staged = vec![StagedFile {
        path: "session.rs".to_string(),
        letter: 'M',
    }];
    let content = render_panel(&app);
    assert!(content.contains("[1 files]"));
    assert!(content.contains("[1 staged]"));
}

// -- Bottom section: last commit + remote keybind hints ----------------

#[test]
fn bottom_section_shows_last_commit_hash_and_subject() {
    let mut app = App::new(vec![sample_file("session.rs")]);
    app.last_commit = Some(CommitSummary {
        short_hash: "a1b2c3d".to_string(),
        subject: "fix: parser".to_string(),
    });
    let content = render_panel(&app);
    assert!(content.contains("a1b2c3d"));
    assert!(content.contains("fix: parser"));
}

#[test]
fn bottom_section_shows_no_commits_yet_without_a_last_commit() {
    let app = App::new(vec![sample_file("session.rs")]);
    let content = render_panel(&app);
    assert!(content.contains("no commits yet"));
}

#[test]
fn bottom_section_shows_fetch_pull_push_keybind_hints() {
    let mut app = App::new(vec![sample_file("session.rs")]);
    app.branch = Some(branch("main", Some("origin/main"), Some((0, 0))));
    let content = render_panel(&app);
    assert!(content.contains("f fetch"));
    assert!(content.contains("p pull"));
    assert!(content.contains("P push"));
    assert!(!content.contains("P publish"));
}

/// On a branch with no upstream, `P` publishes (see
/// `App::remote_push_op`), so the keybind line must say so.
#[test]
fn bottom_section_relabels_push_to_publish_on_an_unpublished_branch() {
    let mut app = App::new(vec![sample_file("session.rs")]);
    app.branch = Some(branch("feature", None, None));
    let content = render_panel(&app);
    assert!(content.contains("P publish"));
    assert!(!content.contains("P push"));
}

/// The remote-op hint line resolves its keys from the keymap rather than
/// hardcoding `f`/`p`/`P`: a `[keys.panel]` remap of `remote-fetch` must
/// show up here with no code change, and an unbind must drop that
/// segment entirely rather than showing a stale key.
#[test]
fn remote_keys_line_reflects_a_remapped_and_an_unbound_action() {
    let mut app = App::new(vec![sample_file("session.rs")]);
    app.branch = Some(branch("main", Some("origin/main"), Some((0, 0))));

    let mut keys = crate::config::KeysConfig::default();
    keys.panel.insert(
        "remote-fetch".to_string(),
        vec![crate::config::keys::KeySeqSpec::One(
            crate::config::keys::ChordSpec {
                code: crossterm::event::KeyCode::Char('F'),
                mods: crossterm::event::KeyModifiers::NONE,
            },
        )],
    );
    keys.panel.insert("remote-pull".to_string(), Vec::new());
    let (keymap, warnings) = super::super::keymap_config::effective_keymap(&keys);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");

    let content = render_panel_with_keymap(&app, &keymap);
    assert!(content.contains("F fetch"), "must show the remapped key");
    assert!(
        !content.contains("f fetch"),
        "the stale default must be gone"
    );
    assert!(
        !content.contains("pull"),
        "an unbound action's segment must be omitted entirely"
    );
    assert!(content.contains("P push"), "untouched action is unaffected");
}

// -- Empty states ------------------------------------------------------

#[test]
fn single_root_file_shows_without_any_directory_row() {
    let app = App::new(vec![sample_file("session.rs")]);
    let content = render_panel(&app);
    assert!(content.contains("session.rs"));
    assert_eq!(navigable_rows(&app), vec![PanelRow::File(0)]);
}

// -- Cursor model (tree flattening + clamping) -------------------------

/// An app with two files under `src/` plus a root-level untracked file — the
/// fixture the directory-tree tests share.
fn tree_app() -> App {
    let mut app = App::new(vec![
        sample_file("src/a.rs"),
        sample_file("src/b.rs"),
        sample_file("notes.md"),
    ]);
    app.untracked_paths = vec!["notes.md".to_string()];
    app
}

/// A flat app of root-level files (no directories), for the auto-follow
/// tests where row 0 must be a file.
fn flat_app() -> App {
    let mut app = App::new(vec![
        sample_file("a.rs"),
        sample_file("b.rs"),
        sample_file("notes.md"),
    ]);
    app.untracked_paths = vec!["notes.md".to_string()];
    app
}

/// The tree flattens into a `src` directory row, its two files, then the
/// root-level untracked file — directories before files, alphabetical.
#[test]
fn navigable_rows_are_tree_ordered() {
    let app = tree_app();
    assert_eq!(
        navigable_rows(&app),
        vec![
            PanelRow::Dir("src".to_string()),
            PanelRow::File(0), // src/a.rs
            PanelRow::File(1), // src/b.rs
            PanelRow::File(2), // notes.md
        ]
    );
}

/// Collapsing a directory drops its file rows from the navigable set.
#[test]
fn collapsing_a_directory_hides_its_file_rows() {
    let mut app = tree_app();
    app.panel_toggle_dir("src");
    assert_eq!(
        navigable_rows(&app),
        vec![PanelRow::Dir("src".to_string()), PanelRow::File(2)]
    );
}

#[test]
fn moved_cursor_clamps_at_the_top() {
    assert_eq!(moved_cursor(0, 5, false), 0);
}

#[test]
fn moved_cursor_clamps_at_the_bottom() {
    assert_eq!(moved_cursor(4, 5, true), 4);
}

#[test]
fn moved_cursor_crosses_dir_and_file_rows() {
    // Stepping down from the `src` directory row (index 0) lands on its first
    // file (index 1), then its second (index 2) — the flat list makes the
    // dir/file boundary invisible to motion.
    let app = tree_app();
    let len = navigable_rows(&app).len();
    assert_eq!(moved_cursor(0, len, true), 1);
    assert_eq!(moved_cursor(1, len, true), 2);
}

#[test]
fn moved_cursor_on_empty_list_stays_at_zero() {
    let app = App::new(vec![]);
    let len = navigable_rows(&app).len();
    assert_eq!(len, 0);
    assert_eq!(moved_cursor(0, len, true), 0);
    assert_eq!(moved_cursor(0, len, false), 0);
}

// -- Auto-follow --------------------------------------------------------

/// Moving the panel cursor onto a file row scrolls the diff to that
/// file without leaving `Mode::Panel` — follow, don't focus-jump.
#[test]
fn panel_cursor_motion_follows_file_rows() {
    let mut app = flat_app();
    app.mode = Mode::Panel {
        cursor: 0,
        tab: PanelTab::Changes,
    }; // a.rs, already selected (selected_file starts at 0)
    app.panel_move_down(); // -> b.rs
    assert_eq!(app.panel_cursor(), 1);
    assert_eq!(app.view.selected_file, 1);
    assert!(matches!(app.mode, Mode::Panel { .. }));
}

/// Moving onto a directory row leaves the diff's file selection where it
/// last followed to — directory rows have nothing to follow to.
#[test]
fn panel_cursor_on_dir_row_leaves_diff_selection() {
    let mut app = tree_app();
    app.mode = Mode::Panel {
        cursor: 1,
        tab: PanelTab::Changes,
    };
    app.panel_follow(); // -> src/a.rs (index 0) selected
    assert_eq!(app.view.selected_file, 0);
    // Move up onto the `src` directory row; the selection stays put.
    app.panel_move_up();
    assert_eq!(app.panel_cursor(), 0);
    assert_eq!(app.view.selected_file, 0);
}

/// An empty panel (no files) is a no-op, not a panic.
#[test]
fn panel_follow_on_empty_panel_is_noop() {
    let mut app = App::new(vec![]);
    app.mode = Mode::Panel {
        cursor: 0,
        tab: PanelTab::Changes,
    };
    app.panel_follow();
    assert_eq!(app.panel_cursor(), 0);
    assert_eq!(app.view.selected_file, 0);
}

/// Focusing the panel (`` ` ``) resets the cursor to the top row and
/// follows it, so the diff snaps back to the first file even if it had
/// scrolled elsewhere while the panel was unfocused.
#[test]
fn focusing_panel_follows_to_first_file() {
    let mut app = flat_app();
    assert!(app.select_file_by_path("b.rs"));
    assert_eq!(app.view.selected_file, 1);
    app.toggle_git_panel();
    assert!(matches!(app.mode, Mode::Panel { .. }));
    assert_eq!(app.panel_cursor(), 0);
    assert_eq!(app.view.selected_file, 0); // followed back to a.rs
}

/// Following onto a collapsed file's row expands its diff section — a
/// collapsed section has nothing to follow to otherwise.
#[test]
fn panel_follow_expands_collapsed_target() {
    let mut app = flat_app();
    app.view.set_collapsed("b.rs", true);
    app.rebuild_rows();
    assert!(app.view.is_collapsed("b.rs"));
    app.mode = Mode::Panel {
        cursor: 0,
        tab: PanelTab::Changes,
    };
    app.panel_move_down(); // onto b.rs's row
    assert_eq!(app.panel_cursor(), 1);
    assert_eq!(app.view.selected_file, 1);
    assert!(!app.view.is_collapsed("b.rs"));
}

/// Enter on a file row follows to it and returns focus to the diff.
#[test]
fn enter_on_file_row_returns_focus_with_file_selected() {
    let mut app = flat_app();
    app.mode = Mode::Panel {
        cursor: 1,
        tab: PanelTab::Changes,
    }; // b.rs
    app.panel_select();
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.view.selected_file, 1);
}

/// Enter on a directory row toggles its collapse and keeps the panel
/// focused; a second Enter expands it again.
#[test]
fn enter_on_dir_row_toggles_collapse_and_keeps_focus() {
    let mut app = tree_app();
    app.mode = Mode::Panel {
        cursor: 0,
        tab: PanelTab::Changes,
    }; // the `src` directory row
    app.panel_select();
    assert!(app.panel_collapsed_dirs.contains("src"));
    assert!(matches!(app.mode, Mode::Panel { .. }));
    app.panel_select();
    assert!(!app.panel_collapsed_dirs.contains("src"));
    assert!(matches!(app.mode, Mode::Panel { .. }));
}

// -- History tab ---------------------------------------------------------
//
// TestBackend buffer assertions (no real TTY in CI).

use super::super::background::TaskId;
use super::super::history::InFlightHistory;
use crate::git::CommitLogEntry;

fn commit(sha: &str, subject: &str, author: &str, ts: i64) -> CommitLogEntry {
    CommitLogEntry {
        sha: sha.to_string(),
        short_sha: sha[..sha.len().min(7)].to_string(),
        subject: subject.to_string(),
        author_name: author.to_string(),
        timestamp: ts,
    }
}

fn app_on_history_tab() -> App {
    let mut app = App::new(vec![sample_file("session.rs")]);
    app.mode = Mode::Panel {
        cursor: 0,
        tab: PanelTab::History,
    };
    app
}

#[test]
fn history_tab_shows_a_loading_placeholder_before_the_first_page_lands() {
    let mut app = app_on_history_tab();
    // Simulate a fetch in flight without actually spawning a thread.
    app.history_in_flight = Some(InFlightHistory {
        id: TaskId(0),
        generation: app.history_generation,
    });
    let content = render_panel(&app);
    assert!(content.contains("loading"));
}

#[test]
fn history_tab_shows_no_commits_when_nothing_is_in_flight_and_history_is_empty() {
    let app = app_on_history_tab();
    let content = render_panel(&app);
    assert!(content.contains("no commits"));
    assert!(!content.contains("loading"));
}

#[test]
fn history_tab_renders_commit_rows_with_subject_meta_and_unpushed_marker() {
    let mut app = app_on_history_tab();
    app.branch = Some(branch("main", Some("origin/main"), Some((1, 0))));
    app.history = vec![
        commit("abc1234full", "feat: new thing", "Jane Dev", 1_700_000_000),
        commit("def5678full", "fix: old bug", "Jane Dev", 1_600_000_000),
    ];
    let content = render_panel(&app);
    assert!(content.contains("feat: new thing"));
    assert!(content.contains("fix: old bug"));
    // The unpushed marker (●) decorates only the first `ahead` (1) row.
    assert!(content.contains("\u{25cf}"));
    assert!(content.contains("Jane Dev"));
    assert!(content.contains("abc1234"));
}

#[test]
fn panel_title_shows_both_tab_labels_regardless_of_which_is_active() {
    let mut app = App::new(vec![sample_file("session.rs")]);
    app.mode = Mode::Panel {
        cursor: 0,
        tab: PanelTab::Changes,
    };
    let content = render_panel(&app);
    assert!(content.contains("Changes"));
    assert!(content.contains("History"));
}

#[test]
fn moving_the_cursor_down_the_history_tab_stops_at_the_last_loaded_row() {
    let mut app = app_on_history_tab();
    app.history = vec![
        commit("a", "one", "Dev", 1_700_000_000),
        commit("b", "two", "Dev", 1_700_000_000),
    ];
    app.history_exhausted = true;
    app.panel_move_down();
    app.panel_move_down();
    app.panel_move_down(); // clamps at the last row
    assert_eq!(app.panel_cursor(), 1);
}

/// `Tab` switches tabs, resets the cursor, and remembers the tab for the
/// next time the panel is focused.
#[test]
fn toggle_panel_tab_switches_and_resets_cursor() {
    let mut app = tree_app();
    app.mode = Mode::Panel {
        cursor: 2,
        tab: PanelTab::Changes,
    };
    app.toggle_panel_tab();
    assert_eq!(app.panel_tab(), PanelTab::History);
    assert_eq!(app.panel_cursor(), 0);
    assert_eq!(app.last_panel_tab, PanelTab::History);

    app.toggle_panel_tab();
    assert_eq!(app.panel_tab(), PanelTab::Changes);
    assert_eq!(app.panel_cursor(), 0);
}

/// Re-focusing the panel lands on whichever tab was last active, not
/// always Changes.
#[test]
fn refocusing_the_panel_remembers_the_last_active_tab() {
    let mut app = tree_app();
    app.mode = Mode::Panel {
        cursor: 0,
        tab: PanelTab::Changes,
    };
    app.toggle_panel_tab(); // -> History
    app.toggle_git_panel(); // unfocus
    assert_eq!(app.mode, Mode::Normal);
    app.toggle_git_panel(); // refocus
    assert_eq!(app.panel_tab(), PanelTab::History);
}
