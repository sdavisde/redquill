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
    // The graph rail draws a commit dot (●) per row and a connector bar (│)
    // running between them; the first `ahead` (1) row's dot is the bright
    // (unpushed) variant, the rest dim.
    assert!(content.contains("\u{25cf}")); // commit dot
    assert!(content.contains("\u{2502}")); // graph connector bar
    assert!(content.contains("Jane Dev"));
    assert!(content.contains("abc1234")); // right-aligned short sha
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

// -- Panel file actions: stage/unstage/accept/defer the highlighted row ------

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::cell::RefCell;
use std::rc::Rc;

/// A modified tracked file's porcelain status: fully staged or fully
/// unstaged, the two states the panel-action tests flip between.
fn modified_status(path: &str, staged: bool) -> crate::git::FileStatus {
    use crate::git::{ChangeKind, StatusCode};
    let (staged_code, unstaged_code) = if staged {
        (StatusCode::Modified, StatusCode::Unmodified)
    } else {
        (StatusCode::Unmodified, StatusCode::Modified)
    };
    crate::git::FileStatus {
        kind: ChangeKind::Ordinary,
        staged: staged_code,
        unstaged: unstaged_code,
        path: path.to_string(),
        orig_path: None,
    }
}

/// A recording `StageOps` fake whose `stage_file`/`unstage_file` also flip
/// the path's entry in the shared status vec, so a post-gesture
/// `App::refresh` observes the same index change a real `git add` would
/// produce and the staged markers update through the real path.
struct PanelOps {
    calls: Rc<RefCell<Vec<String>>>,
    diff: Vec<RawFilePatch>,
    status: Rc<RefCell<Vec<crate::git::FileStatus>>>,
}

impl super::super::stage_ops::StageOps for PanelOps {
    fn diff(
        &self,
        _target: &crate::git::DiffTarget,
    ) -> Result<Vec<RawFilePatch>, crate::git::GitError> {
        Ok(self.diff.clone())
    }
    fn status(&self) -> Result<Vec<crate::git::FileStatus>, crate::git::GitError> {
        Ok(self.status.borrow().clone())
    }
    fn stage_file(&self, path: &str) -> Result<(), crate::git::GitError> {
        self.calls.borrow_mut().push(format!("stage-file {path}"));
        let mut status = self.status.borrow_mut();
        if let Some(entry) = status.iter_mut().find(|s| s.path == path) {
            *entry = modified_status(path, true);
        }
        Ok(())
    }
    fn unstage_file(&self, path: &str) -> Result<(), crate::git::GitError> {
        self.calls.borrow_mut().push(format!("unstage-file {path}"));
        let mut status = self.status.borrow_mut();
        if let Some(entry) = status.iter_mut().find(|s| s.path == path) {
            *entry = modified_status(path, false);
        }
        Ok(())
    }
    fn apply_cached(&self, _patch: &str) -> Result<(), crate::git::GitError> {
        self.calls.borrow_mut().push("apply-hunk".to_string());
        Ok(())
    }
    fn unapply_cached(&self, _patch: &str) -> Result<(), crate::git::GitError> {
        self.calls.borrow_mut().push("unapply-hunk".to_string());
        Ok(())
    }
    fn read_worktree_file(&self, _path: &str) -> Option<Vec<u8>> {
        None
    }
    fn show_file(&self, _spec: &str) -> Option<String> {
        None
    }
}

fn raw_file(path: &str) -> RawFilePatch {
    RawFilePatch {
        path: path.to_string(),
        old_path: None,
        raw: format!(
            "diff --git a/{path} b/{path}\n\
             index 111..222 100644\n\
             --- a/{path}\n\
             +++ b/{path}\n\
             @@ -1,1 +1,1 @@\n\
             -old\n\
             +new\n"
        ),
        is_binary: false,
    }
}

/// An app over two modified files under `src/`, wired to a recording
/// [`PanelOps`] and refreshed through the real path so patches, staged
/// states, and the panel tree all come from the fake backend.
fn panel_actions_app() -> (App, Rc<RefCell<Vec<String>>>) {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let status = Rc::new(RefCell::new(vec![
        modified_status("src/a.rs", false),
        modified_status("src/b.rs", false),
    ]));
    let mut app = App::new(Vec::new());
    app.stage_ops = Some(Box::new(PanelOps {
        calls: calls.clone(),
        diff: vec![raw_file("src/a.rs"), raw_file("src/b.rs")],
        status,
    }));
    app.refresh();
    assert_eq!(app.view.files.len(), 2, "fixture must load both files");
    (app, calls)
}

/// Presses one key through the real dispatch path (the same entry point the
/// event loop uses), so panel-scope resolution and routing are exercised
/// end to end.
fn press(app: &mut App, code: KeyCode) {
    let keymap = Keymap::default_map();
    let mut pending = None;
    let mut pending_count = None;
    let _ = super::super::dispatch_key(
        app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(code, KeyModifiers::NONE),
    );
}

#[test]
fn panel_space_stages_the_highlighted_file_as_a_whole_file_gesture() {
    let (mut app, calls) = panel_actions_app();
    app.apply(Action::FocusGitPanel);
    press(&mut app, KeyCode::Char('j')); // Dir("src") -> File(src/a.rs)
    // Park the diff cursor on a body line of the followed file: a
    // cursor-derived gesture would stage a hunk here, so the assertion
    // below proves the panel forces the whole-file gesture.
    app.view.cursor += 2;
    press(&mut app, KeyCode::Char(' '));
    assert_eq!(
        calls.borrow().as_slice(),
        ["stage-file src/a.rs"],
        "panel Space must stage the whole highlighted file, never a hunk"
    );
    assert_eq!(
        app.staged_states.get("src/a.rs"),
        Some(&StagedState::Full),
        "the staged state must update through the existing refresh path"
    );
    assert!(
        matches!(app.mode, Mode::Panel { .. }),
        "focus stays in the panel"
    );
    assert!(
        render_panel(&app).contains('\u{25cf}'),
        "the staged dot must appear on the panel row"
    );
}

#[test]
fn panel_shift_s_stages_and_a_second_press_unstages() {
    let (mut app, calls) = panel_actions_app();
    app.apply(Action::FocusGitPanel);
    press(&mut app, KeyCode::Char('j'));
    press(&mut app, KeyCode::Char('S'));
    assert_eq!(calls.borrow().as_slice(), ["stage-file src/a.rs"]);
    assert_eq!(app.staged_states.get("src/a.rs"), Some(&StagedState::Full));
    press(&mut app, KeyCode::Char('S'));
    assert_eq!(
        calls.borrow().as_slice(),
        ["stage-file src/a.rs", "unstage-file src/a.rs"],
        "S on a fully staged file must unstage it, matching the diff view"
    );
    assert_eq!(
        app.staged_states.get("src/a.rs"),
        None,
        "the staged marker state clears after the unstage"
    );
}

#[test]
fn panel_space_on_a_directory_row_is_a_no_op_with_a_hint() {
    let (mut app, calls) = panel_actions_app();
    app.apply(Action::FocusGitPanel); // cursor 0 = Dir("src")
    press(&mut app, KeyCode::Char(' '));
    assert!(calls.borrow().is_empty(), "no staging call on a directory");
    assert!(
        app.status_message.is_some(),
        "a directory row must hint instead of acting"
    );
}

#[test]
fn panel_file_keys_are_inert_on_the_history_tab() {
    let (mut app, calls) = panel_actions_app();
    app.apply(Action::FocusGitPanel);
    press(&mut app, KeyCode::Tab); // -> History tab
    assert_eq!(app.panel_tab(), PanelTab::History);
    for key in [KeyCode::Char(' '), KeyCode::Char('S'), KeyCode::Char('d')] {
        press(&mut app, key);
    }
    assert!(
        calls.borrow().is_empty(),
        "History rows take no file actions"
    );
    assert!(app.status_message.is_none(), "inert means no hint either");
}

#[test]
fn panel_file_keys_are_inert_with_a_message_on_a_read_only_target() {
    let (mut app, calls) = panel_actions_app();
    app.target = crate::git::DiffTarget::Range("main..feature".to_string());
    app.apply(Action::FocusGitPanel);
    press(&mut app, KeyCode::Char('j'));
    press(&mut app, KeyCode::Char(' '));
    assert!(calls.borrow().is_empty());
    assert_eq!(
        app.status_message.as_deref(),
        Some("read-only diff target"),
        "the diff view's read-only hint is reused, not duplicated"
    );
}

/// A review-session app over two files under `src/`; accept/defer state is
/// in-memory, so no git backend is needed.
fn review_panel_app() -> App {
    let mut app = App::new(vec![sample_file("src/a.rs"), sample_file("src/b.rs")]);
    app.target = crate::git::DiffTarget::Review {
        base: "main".to_string(),
        branch: "feature".to_string(),
    };
    app
}

#[test]
fn panel_space_toggle_accepts_the_highlighted_file_in_a_review_session() {
    let mut app = review_panel_app();
    app.apply(Action::FocusGitPanel);
    press(&mut app, KeyCode::Char('j')); // Dir("src") -> File(src/a.rs)
    press(&mut app, KeyCode::Char(' '));
    assert_eq!(
        app.review_status("src/a.rs"),
        ReviewStatus::Accepted,
        "panel Space must translate to accept during a review session"
    );
    assert!(
        render_panel(&app).contains('\u{25cf}'),
        "the accepted marker must update immediately"
    );
    press(&mut app, KeyCode::Char(' '));
    assert_eq!(
        app.review_status("src/a.rs"),
        ReviewStatus::Unreviewed,
        "a second Space un-accepts, mirroring the diff view's toggle"
    );
}

#[test]
fn panel_shift_s_accepts_in_a_review_session() {
    let mut app = review_panel_app();
    app.apply(Action::FocusGitPanel);
    press(&mut app, KeyCode::Char('j'));
    press(&mut app, KeyCode::Char('S'));
    assert_eq!(app.review_status("src/a.rs"), ReviewStatus::Accepted);
}

#[test]
fn panel_d_toggle_defers_in_a_review_session() {
    let mut app = review_panel_app();
    app.apply(Action::FocusGitPanel);
    press(&mut app, KeyCode::Char('j'));
    press(&mut app, KeyCode::Char('d'));
    assert_eq!(app.review_status("src/a.rs"), ReviewStatus::Deferred);
    assert!(
        render_panel(&app).contains('~'),
        "the deferred marker must update immediately"
    );
    press(&mut app, KeyCode::Char('d'));
    assert_eq!(app.review_status("src/a.rs"), ReviewStatus::Unreviewed);
}

#[test]
fn panel_d_outside_a_review_session_is_a_total_no_op() {
    let (mut app, calls) = panel_actions_app();
    app.apply(Action::FocusGitPanel);
    press(&mut app, KeyCode::Char('j'));
    press(&mut app, KeyCode::Char('d'));
    assert!(calls.borrow().is_empty());
    assert!(app.status_message.is_none());
    assert!(app.review_states.is_empty());
}

// -- Journey: panel file actions over a real scratch repo --------------------

/// Runs a git command inside the scratch tempdir, asserting success. Fixture
/// plumbing only — every invocation is pinned to `dir`, never the host repo.
fn scratch_git(dir: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("failed to spawn git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn scratch_write(dir: &std::path::Path, rel: &str, contents: &str) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

/// Renders the panel to a framed multi-line snapshot for the journey log.
fn panel_frame(app: &App) -> String {
    let width = 44u16;
    let backend = TestBackend::new(width, 16);
    let mut terminal = Terminal::new(backend).unwrap();
    let area = Rect::new(0, 0, width, 16);
    terminal
        .draw(|frame| render(frame, area, app, &Keymap::default_map()))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let symbols: Vec<&str> = buffer.content().iter().map(|c| c.symbol()).collect();
    symbols
        .chunks(width as usize)
        .map(|row| row.concat().trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Journey driver: a scratch tempdir repo, the real `GitRunner` backend, and
/// the real dispatch path, key by key — first the working-tree staging flow,
/// then a review-session triage. Every logged step is asserted, so this is a
/// regression test as well as the transcript generator
/// (`cargo test panel_actions_journey_transcript -- --nocapture` captures
/// the persisted proof).
#[test]
fn panel_actions_journey_transcript() {
    let mut log = String::new();
    let mut step = |title: &str, body: &str| {
        log.push_str(&format!("\n=== {title} ===\n{body}\n"));
    };

    // -- Scratch repo: two modified tracked files under src/ ---------------
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path();
    scratch_git(dir, &["init", "-q", "-b", "main"]);
    scratch_git(dir, &["config", "user.name", "redquill test"]);
    scratch_git(dir, &["config", "user.email", "test@redquill.invalid"]);
    scratch_write(dir, "src/a.rs", "fn a() {}\n");
    scratch_write(dir, "src/b.rs", "fn b() {}\n");
    scratch_git(dir, &["add", "."]);
    scratch_git(dir, &["commit", "-q", "-m", "base"]);
    scratch_write(dir, "src/a.rs", "fn a() { changed(); }\n");
    scratch_write(dir, "src/b.rs", "fn b() { changed(); }\n");

    let runner = crate::git::GitRunner::discover_in(dir).expect("discover scratch repo");
    let mut app = App::new(Vec::new());
    app.stage_ops = Some(Box::new(runner));
    app.refresh();
    assert_eq!(app.view.files.len(), 2);
    step(
        "journey A: launch on the scratch working tree",
        &format!(
            "repo: {} files modified, nothing staged\nstaged_states: {:?}",
            app.view.files.len(),
            app.staged_states
        ),
    );

    press(&mut app, KeyCode::Char('`'));
    assert!(matches!(app.mode, Mode::Panel { .. }));
    press(&mut app, KeyCode::Char('j')); // Dir("src") -> File(src/a.rs)
    step(
        "press ` then j: panel focused, src/a.rs highlighted",
        &panel_frame(&app),
    );

    press(&mut app, KeyCode::Char(' '));
    assert_eq!(app.staged_states.get("src/a.rs"), Some(&StagedState::Full));
    step(
        "press Space: src/a.rs stages, marker updates in place",
        &format!(
            "status: {:?}\n{}",
            app.status_message.as_deref(),
            panel_frame(&app)
        ),
    );

    press(&mut app, KeyCode::Char('S'));
    assert_eq!(app.staged_states.get("src/a.rs"), None);
    step(
        "press S: fully staged file unstages (toggle parity with the diff view)",
        &format!(
            "status: {:?}\n{}",
            app.status_message.as_deref(),
            panel_frame(&app)
        ),
    );

    press(&mut app, KeyCode::Char('S'));
    assert_eq!(app.staged_states.get("src/a.rs"), Some(&StagedState::Full));
    step(
        "press S again: src/a.rs stages back",
        &format!("staged_states: {:?}", app.staged_states),
    );

    press(&mut app, KeyCode::Char('k')); // back to Dir("src")
    press(&mut app, KeyCode::Char(' '));
    assert!(app.status_message.is_some());
    step(
        "press k then Space on the src/ directory row: hinted no-op",
        &format!("status: {:?}", app.status_message.as_deref()),
    );

    // The index write really happened in the scratch repo, nowhere else.
    let staged = std::process::Command::new("git")
        .current_dir(dir)
        .args(["diff", "--cached", "--name-only"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&staged.stdout).trim(),
        "src/a.rs",
        "the scratch repo's index holds exactly the staged file"
    );
    step(
        "scratch repo check: git diff --cached --name-only",
        String::from_utf8_lossy(&staged.stdout).trim(),
    );

    // -- Journey B: review-session triage over a feature branch ------------
    let tmp2 = tempfile::TempDir::new().unwrap();
    let dir2 = tmp2.path();
    scratch_git(dir2, &["init", "-q", "-b", "main"]);
    scratch_git(dir2, &["config", "user.name", "redquill test"]);
    scratch_git(dir2, &["config", "user.email", "test@redquill.invalid"]);
    scratch_write(dir2, "src/one.rs", "fn one() {}\n");
    scratch_write(dir2, "src/two.rs", "fn two() {}\n");
    scratch_write(dir2, "notes.md", "notes\n");
    scratch_git(dir2, &["add", "."]);
    scratch_git(dir2, &["commit", "-q", "-m", "base"]);
    scratch_git(dir2, &["switch", "-q", "-c", "feature"]);
    scratch_write(dir2, "src/one.rs", "fn one() { reviewed(); }\n");
    scratch_write(dir2, "src/two.rs", "fn two() { reviewed(); }\n");
    scratch_write(dir2, "notes.md", "notes\nmore notes\n");
    scratch_git(dir2, &["add", "."]);
    scratch_git(dir2, &["commit", "-q", "-m", "feature work"]);

    let runner2 = crate::git::GitRunner::discover_in(dir2).expect("discover review repo");
    let mut review = App::new(Vec::new());
    review.stage_ops = Some(Box::new(runner2));
    review.target = crate::git::DiffTarget::Review {
        base: "main".to_string(),
        branch: "feature".to_string(),
    };
    review.refresh();
    assert_eq!(review.view.files.len(), 3);
    step(
        "journey B: review session over feature (3 changed files vs main)",
        &format!("files: {:?}", {
            let mut paths: Vec<&str> = review.view.files.iter().map(|f| f.path.as_str()).collect();
            paths.sort();
            paths
        }),
    );

    press(&mut review, KeyCode::Char('`'));
    press(&mut review, KeyCode::Char('j')); // notes.md (root file sorts after src/? follow the tree)
    // Walk the panel until src/one.rs is highlighted, logging nothing —
    // tree order is directories first, so land explicitly.
    let rows = navigable_rows(&review);
    let one_index = rows
        .iter()
        .position(|r| matches!(r, PanelRow::File(i) if review.view.files[*i].path == "src/one.rs"))
        .expect("src/one.rs must be a panel row");
    while review.panel_cursor() < one_index {
        press(&mut review, KeyCode::Char('j'));
    }
    press(&mut review, KeyCode::Char(' '));
    assert_eq!(review.review_status("src/one.rs"), ReviewStatus::Accepted);
    step(
        "press Space on src/one.rs: accepted, ● appears",
        &panel_frame(&review),
    );

    press(&mut review, KeyCode::Char('j'));
    press(&mut review, KeyCode::Char('S'));
    assert_eq!(review.review_status("src/two.rs"), ReviewStatus::Accepted);
    step(
        "press j then S on src/two.rs: accepted, second ●",
        &panel_frame(&review),
    );

    let notes_index = navigable_rows(&review)
        .iter()
        .position(|r| matches!(r, PanelRow::File(i) if review.view.files[*i].path == "notes.md"))
        .expect("notes.md must be a panel row");
    while review.panel_cursor() != notes_index {
        let down = review.panel_cursor() < notes_index;
        press(
            &mut review,
            if down {
                KeyCode::Char('j')
            } else {
                KeyCode::Char('k')
            },
        );
    }
    press(&mut review, KeyCode::Char('d'));
    assert_eq!(review.review_status("notes.md"), ReviewStatus::Deferred);
    step(
        "press d on notes.md: deferred, ~ appears",
        &panel_frame(&review),
    );
    assert_eq!(review.review_progress(), (2, 3));
    step(
        "review progress",
        &format!("accepted {} of {} files, 1 deferred", 2, 3),
    );

    if std::env::var("RQ_JOURNEY_DUMP").is_ok() {
        eprintln!("{log}");
    }
}
