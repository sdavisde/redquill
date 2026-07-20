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

// -- Panel coherence: Esc leaves, s and / reach through (spec 11 Unit 2) -----

/// `Esc` closes the focused panel back to `Normal`, without touching any
/// staging state — the highlighted file's follow-sync is left exactly as
/// `panel_follow` set it, and no git op runs.
#[test]
fn panel_esc_closes_the_panel_without_touching_staging_state() {
    let (mut app, calls) = panel_actions_app();
    app.apply(Action::FocusGitPanel);
    press(&mut app, KeyCode::Char('j')); // Dir("src") -> File(src/a.rs)
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.mode, Mode::Normal, "Esc closes the panel to Normal");
    assert!(calls.borrow().is_empty(), "Esc must not run any staging op");
}

/// `s` from the focused panel reaches the staging panel — closing the git
/// panel first, exactly as if the user had pressed `` ` `` then `s`, rather
/// than no-oping because `toggle_staging_panel` guards against `Mode::Panel`.
#[test]
fn panel_s_reaches_the_staging_panel_with_the_index_it_would_show_from_normal() {
    let (mut app, _calls) = panel_actions_app();
    app.apply(Action::FocusGitPanel);
    press(&mut app, KeyCode::Char('j'));
    press(&mut app, KeyCode::Char('S')); // stage src/a.rs first, so the staging panel isn't empty
    press(&mut app, KeyCode::Char('s'));
    assert_eq!(app.mode, Mode::Staging, "s must open the staging panel");
    assert_eq!(
        app.staged
            .iter()
            .map(|f| f.path.as_str())
            .collect::<Vec<_>>(),
        vec!["src/a.rs"],
        "the staging panel shows the same index a Normal-mode s would"
    );
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

// -- Journey: panel coherence (Esc/s// reach through) over a scratch repo ----

/// Journey driver for spec 11 Unit 2: `` ` `` opens the panel, `Esc` backs
/// out to `Normal`; `` ` `` again, `/` opens search, typing a query and
/// confirming lands the cursor on a real match; `` ` `` again, `s` reaches
/// the staging panel with the index it would show from `Normal`. Every
/// logged step is asserted against a real scratch tempdir repo and the real
/// `GitRunner` backend, driven through the real `dispatch_key` path
/// (`RQ_JOURNEY_DUMP=1 cargo test --lib panel_coherence_journey_transcript
/// -- --nocapture` captures the persisted proof).
#[test]
fn panel_coherence_journey_transcript() {
    let mut log = String::new();
    let mut step = |title: &str, body: &str| {
        log.push_str(&format!("\n=== {title} ===\n{body}\n"));
    };

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
        "launch on the scratch working tree",
        &format!("{} files modified", app.view.files.len()),
    );

    // -- `Esc` backs the panel out to Normal --------------------------------
    press(&mut app, KeyCode::Char('`'));
    assert!(matches!(app.mode, Mode::Panel { .. }));
    step("press `: panel focused", &panel_frame(&app));

    press(&mut app, KeyCode::Esc);
    assert_eq!(app.mode, Mode::Normal, "Esc must close the panel to Normal");
    step(
        "press Esc: panel closes, back to Normal",
        &format!("mode: {:?}", app.mode),
    );

    // -- `/` reaches search, landing on a real match -------------------------
    press(&mut app, KeyCode::Char('`'));
    assert!(matches!(app.mode, Mode::Panel { .. }));
    press(&mut app, KeyCode::Char('/'));
    assert_eq!(
        app.mode,
        Mode::Search,
        "/ must open search, not no-op inside the panel"
    );
    step(
        "press ` then /: panel closed first, search input active",
        &format!("mode: {:?}", app.mode),
    );

    for ch in "changed".chars() {
        press(&mut app, KeyCode::Char(ch));
    }
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.mode,
        Mode::Normal,
        "confirming search returns to Normal"
    );
    assert_eq!(app.search.pattern.as_deref(), Some("changed"));
    assert!(
        !app.search.matches.is_empty(),
        "the typed query must match real diff content"
    );
    step(
        "type changed, Enter: confirmed, cursor lands on a real match",
        &format!(
            "pattern: {:?}, matches: {}, status: {:?}",
            app.search.pattern,
            app.search.matches.len(),
            app.status_message.as_deref()
        ),
    );

    // -- `s` reaches the staging panel ---------------------------------------
    press(&mut app, KeyCode::Char('`'));
    assert!(matches!(app.mode, Mode::Panel { .. }));
    press(&mut app, KeyCode::Char('j')); // Dir("src") -> File(src/a.rs)
    press(&mut app, KeyCode::Char('S')); // stage src/a.rs, so the panel isn't empty
    press(&mut app, KeyCode::Char('s'));
    assert_eq!(
        app.mode,
        Mode::Staging,
        "s must open the staging panel, not no-op inside the panel"
    );
    step(
        "press ` j S then s: staged src/a.rs, then s opens the staging panel",
        &format!(
            "mode: {:?}, staged: {:?}",
            app.mode,
            app.staged
                .iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>()
        ),
    );

    if std::env::var("RQ_JOURNEY_DUMP").is_ok() {
        eprintln!("{log}");
    }
}

// -- Journey: annotation round-trip (edit/delete from the diff view) ----------

/// Renders the diff row model (`app.view.rows`) as text for the annotation
/// round-trip journey log — the truest view of what splices inline, marking
/// the cursor row (`>`) and each annotation body line (`┃`). Directly shows an
/// annotation appearing and disappearing next to the line it targets.
fn diff_rows_text(app: &App) -> String {
    use crate::ui::Row;
    app.view
        .rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let cur = if i == app.view.cursor { ">" } else { " " };
            match row {
                Row::FileHeader { path, .. } => format!("{cur} ▾ {path}"),
                Row::HunkHeader { text, .. } => format!("{cur} {text}"),
                Row::Line(l) => {
                    let sign = match l.origin {
                        crate::diff::LineOrigin::Added => '+',
                        crate::diff::LineOrigin::Removed => '-',
                        crate::diff::LineOrigin::Context => ' ',
                    };
                    format!("{cur}   {sign}{}", l.content)
                }
                Row::Annotation {
                    text,
                    classification,
                    ..
                } => match classification {
                    Some(c) => format!("{cur}   ┃ ● [{}] {text}", c.label()),
                    None => format!("{cur}   ┃   {text}"),
                },
                Row::AnnotationBorder { .. } => format!("{cur}   ┃"),
                Row::Binary => format!("{cur}   <binary>"),
                Row::Thread(_) | Row::ThreadBorder { .. } => format!("{cur}   ≡"),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Drives `j` until the diff cursor sits on a new-side line (an added or
/// context line the annotation gestures anchor to), or gives up after a bound.
fn park_on_new_side_line(app: &mut App) {
    for _ in 0..64 {
        if matches!(
            app.target_for_cursor(),
            Some(crate::annotate::Target::Line {
                side: crate::annotate::Side::New,
                ..
            })
        ) {
            return;
        }
        press(app, KeyCode::Char('j'));
    }
    panic!("never landed on a new-side line");
}

/// Journey driver for spec 11 Unit 3: annotate a changed line with `c`, move
/// away and back, edit it in place with `e` (compose opens pre-filled), submit
/// the new text, then delete it with `x` (the inline row disappears); finally
/// `e` on a bare line leaves a no-op status hint. Every step is asserted
/// against a real scratch tempdir repo driven through the real `dispatch_key`
/// path (`RQ_JOURNEY_DUMP=1 cargo test --lib annotation_roundtrip_journey_transcript
/// -- --nocapture` captures the persisted proof).
#[test]
fn annotation_roundtrip_journey_transcript() {
    let mut log = String::new();
    let mut step = |title: &str, body: &str| {
        log.push_str(&format!("\n=== {title} ===\n{body}\n"));
    };

    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path();
    scratch_git(dir, &["init", "-q", "-b", "main"]);
    scratch_git(dir, &["config", "user.name", "redquill test"]);
    scratch_git(dir, &["config", "user.email", "test@redquill.invalid"]);
    scratch_write(dir, "src/a.rs", "fn a() {\n    one();\n    two();\n}\n");
    scratch_git(dir, &["add", "."]);
    scratch_git(dir, &["commit", "-q", "-m", "base"]);
    scratch_write(
        dir,
        "src/a.rs",
        "fn a() {\n    one();\n    changed();\n    two();\n}\n",
    );

    let runner = crate::git::GitRunner::discover_in(dir).expect("discover scratch repo");
    let mut app = App::new(Vec::new());
    app.stage_ops = Some(Box::new(runner));
    app.refresh();
    assert_eq!(app.view.files.len(), 1);
    step(
        "launch on the scratch working tree",
        &format!("{} file modified: src/a.rs", app.view.files.len()),
    );

    // -- Annotate the changed line with `c` --------------------------------
    park_on_new_side_line(&mut app);
    let annotated_target = app.target_for_cursor().expect("a line target");
    press(&mut app, KeyCode::Char('c'));
    assert_eq!(app.mode, Mode::Compose);
    for ch in "needs a test".chars() {
        press(&mut app, KeyCode::Char(ch));
    }
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.annotations.len(), 1);
    assert_eq!(app.annotations.iter().next().unwrap().body, "needs a test");
    step(
        "press c, type \"needs a test\", Enter: annotation renders inline",
        &diff_rows_text(&app),
    );

    // -- Move away (down), then back up to the annotated line --------------
    for _ in 0..3 {
        press(&mut app, KeyCode::Char('j'));
    }
    assert_ne!(
        app.target_for_cursor(),
        Some(annotated_target.clone()),
        "cursor moved off the annotated line"
    );
    while app.target_for_cursor() != Some(annotated_target.clone()) {
        press(&mut app, KeyCode::Char('k'));
    }
    step(
        "j×3 away then k back to the annotated line",
        &format!("cursor target: {:?}", app.target_for_cursor()),
    );

    // -- Edit in place with `e` --------------------------------------------
    press(&mut app, KeyCode::Char('e'));
    assert_eq!(app.mode, Mode::Compose);
    let compose = app.compose.as_ref().unwrap();
    assert_eq!(compose.buffer.text(), "needs a test", "pre-filled body");
    assert_eq!(compose.editing_id, Some(0), "edits in place");
    for ch in " (added)".chars() {
        press(&mut app, KeyCode::Char(ch));
    }
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.annotations.len(), 1, "still one annotation, edited");
    assert_eq!(
        app.annotations.iter().next().unwrap().body,
        "needs a test (added)"
    );
    step(
        "press e, append \" (added)\", Enter: edited text renders inline",
        &diff_rows_text(&app),
    );

    // -- Delete with `x` (cursor is still on the annotated line) -----------
    assert_eq!(app.target_for_cursor(), Some(annotated_target.clone()));
    press(&mut app, KeyCode::Char('x'));
    assert!(app.annotations.is_empty(), "annotation deleted");
    assert!(
        !app.view
            .rows
            .iter()
            .any(|r| matches!(r, crate::ui::Row::Annotation { .. })),
        "the inline annotation row is gone"
    );
    step(
        "press x: annotation deleted, inline row disappears",
        &diff_rows_text(&app),
    );

    // -- `e` on a bare line: no-op status hint -----------------------------
    park_on_new_side_line(&mut app);
    press(&mut app, KeyCode::Char('e'));
    assert_eq!(app.mode, Mode::Normal, "no modal opens");
    assert!(app.compose.is_none());
    assert!(app.status_message.is_some());
    step(
        "press e on a bare line: no-op with a status hint",
        &format!("status: {:?}", app.status_message.as_deref()),
    );

    if std::env::var("RQ_JOURNEY_DUMP").is_ok() {
        eprintln!("{log}");
    }
}

// -- Shared motion layer: panel half/full-page + jump (FR-3) ----------------

/// A flat app of `n` root-level files, for paging/jump tests that need more
/// rows than `flat_app`'s three.
fn many_files_app(n: usize) -> App {
    let files: Vec<FileDiff> = (0..n).map(|i| sample_file(&format!("f{i}.rs"))).collect();
    App::new(files)
}

fn panel_app(app: App, tab: PanelTab) -> App {
    let mut app = app;
    app.mode = Mode::Panel { cursor: 0, tab };
    app
}

#[test]
fn panel_half_and_full_page_step_and_clamp_on_changes_tab() {
    let mut app = panel_app(many_files_app(50), PanelTab::Changes);
    // Default viewport height (before any frame renders) is 20 rows, so
    // half-page steps 10 and full-page steps 20 — mirrors the diff view's
    // own DEFAULT_VIEWPORT_HEIGHT fallback.
    app.panel_half_page_down();
    assert_eq!(app.panel_cursor(), 10);
    app.panel_full_page_down();
    assert_eq!(app.panel_cursor(), 30);
    app.panel_full_page_down();
    assert_eq!(app.panel_cursor(), 49, "clamps at the last navigable row");
    app.panel_half_page_up();
    assert_eq!(app.panel_cursor(), 39);
    app.panel_full_page_up();
    app.panel_full_page_up();
    assert_eq!(app.panel_cursor(), 0, "clamps at the first row");
}

#[test]
fn panel_jump_to_top_and_bottom_hit_the_row_extremes() {
    let mut app = panel_app(many_files_app(50), PanelTab::Changes);
    app.panel_jump_to_bottom();
    assert_eq!(app.panel_cursor(), 49);
    app.panel_jump_to_top();
    assert_eq!(app.panel_cursor(), 0);
}

#[test]
fn panel_half_full_page_and_jump_are_noops_on_an_empty_panel() {
    let mut app = panel_app(App::new(vec![]), PanelTab::Changes);
    app.panel_half_page_down();
    app.panel_full_page_down();
    app.panel_jump_to_bottom();
    assert_eq!(app.panel_cursor(), 0);
    app.panel_jump_to_top();
    assert_eq!(app.panel_cursor(), 0);
}

/// A minimal `StageOps` fake serving a fixed commit list synchronously,
/// local to this test (mirrors `history_tests.rs`'s `SyncHistoryFake`, which
/// is private to its own module).
struct PanelHistoryFake {
    entries: Vec<CommitLogEntry>,
}

impl crate::ui::stage_ops::StageOps for PanelHistoryFake {
    fn diff(&self, _target: &DiffTarget) -> Result<Vec<RawFilePatch>, crate::git::GitError> {
        Ok(Vec::new())
    }
    fn status(&self) -> Result<Vec<crate::git::FileStatus>, crate::git::GitError> {
        Ok(Vec::new())
    }
    fn stage_file(&self, _path: &str) -> Result<(), crate::git::GitError> {
        Ok(())
    }
    fn unstage_file(&self, _path: &str) -> Result<(), crate::git::GitError> {
        Ok(())
    }
    fn apply_cached(&self, _patch: &str) -> Result<(), crate::git::GitError> {
        Ok(())
    }
    fn unapply_cached(&self, _patch: &str) -> Result<(), crate::git::GitError> {
        Ok(())
    }
    fn read_worktree_file(&self, _path: &str) -> Option<Vec<u8>> {
        None
    }
    fn show_file(&self, _spec: &str) -> Option<String> {
        None
    }
    fn commit_log(
        &self,
        count: u32,
        skip: u32,
    ) -> Result<Vec<CommitLogEntry>, crate::git::GitError> {
        let start = (skip as usize).min(self.entries.len());
        let end = (start + count as usize).min(self.entries.len());
        Ok(self.entries[start..end].to_vec())
    }
}

fn history_commit(sha: &str) -> CommitLogEntry {
    CommitLogEntry {
        sha: sha.to_string(),
        short_sha: sha.to_string(),
        subject: "subject".to_string(),
        author_name: "Dev".to_string(),
        timestamp: 1_700_000_000,
    }
}

/// A layer-driven jump on the History tab must trigger the same lazy
/// prefetch a plain `j` (`panel_move_down`) does — jumping to the bottom of
/// a not-yet-exhausted 100-row page immediately lands within
/// `HISTORY_PREFETCH_MARGIN` of the end, so it must request the next page.
#[test]
fn panel_jump_to_bottom_on_history_tab_triggers_prefetch_like_a_plain_move() {
    let entries: Vec<CommitLogEntry> = (0..100).map(|i| history_commit(&format!("c{i}"))).collect();
    let mut app = App::new(vec![]);
    app.stage_ops = Some(Box::new(PanelHistoryFake { entries }));
    app.ensure_history_loaded();
    assert_eq!(app.history.len(), 100);
    assert!(!app.history_exhausted);

    app.mode = Mode::Panel {
        cursor: 0,
        tab: PanelTab::History,
    };
    app.panel_jump_to_bottom();
    assert_eq!(app.panel_cursor(), 99);
    // The prefetch's synchronous fallback path applies its page immediately
    // (no background thread involved), so requesting past the fake's 100
    // total entries exhausting history is the observable proof the jump
    // actually fired `maybe_prefetch_history`.
    assert_eq!(app.history.len(), 100);
    assert!(
        app.history_exhausted,
        "requesting past the end of the fake's 100 entries exhausts history, \
         which only happens if the jump actually requested a page"
    );
}

/// Half/full-page paging on the History tab fires the same prefetch once
/// the cursor lands within the margin of the end, exactly like `j`.
#[test]
fn panel_full_page_down_on_history_tab_triggers_prefetch_near_the_end() {
    let entries: Vec<CommitLogEntry> = (0..100).map(|i| history_commit(&format!("c{i}"))).collect();
    let mut app = App::new(vec![]);
    app.stage_ops = Some(Box::new(PanelHistoryFake { entries }));
    app.ensure_history_loaded();
    app.mode = Mode::Panel {
        cursor: 85,
        tab: PanelTab::History,
    };
    // viewport defaults to 20 -> full page steps 20 -> cursor 99 (clamped),
    // within HISTORY_PREFETCH_MARGIN (10) of history.len() (100).
    app.panel_full_page_down();
    assert_eq!(app.panel_cursor(), 99);
    assert!(
        app.history_exhausted,
        "landing within the prefetch margin must have requested (and exhausted) the next page"
    );
}

// -- Journey A: a 200-file changeset, shared-motion-layer paging -----------

/// Journey driver for the motion-layer proof: a real scratch tempdir repo
/// with 200 modified files, driven key by key through the real dispatch
/// path (`Ctrl-d` pages, `G` jumps to bottom, `g` jumps to top, `3j` steps
/// three) in the git panel, then the identical gestures in the annotation
/// list. Every logged step is asserted, so this is a regression test as
/// well as the transcript generator
/// (`RQ_JOURNEY_DUMP=1 cargo test --lib big_changeset_motion_journey_transcript
/// -- --nocapture` captures the persisted `12-proofs/` transcript).
#[test]
fn big_changeset_motion_journey_transcript() {
    use crate::annotate::{Classification, Target};

    let mut log = String::new();
    let mut step = |title: &str, body: &str| {
        log.push_str(&format!("\n=== {title} ===\n{body}\n"));
    };

    // -- Scratch repo: 200 modified root-level files ------------------------
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path();
    scratch_git(dir, &["init", "-q", "-b", "main"]);
    scratch_git(dir, &["config", "user.name", "redquill test"]);
    scratch_git(dir, &["config", "user.email", "test@redquill.invalid"]);
    for i in 0..200 {
        scratch_write(dir, &format!("f{i:03}.rs"), &format!("fn f{i}() {{}}\n"));
    }
    scratch_git(dir, &["add", "."]);
    scratch_git(dir, &["commit", "-q", "-m", "base"]);
    for i in 0..200 {
        scratch_write(
            dir,
            &format!("f{i:03}.rs"),
            &format!("fn f{i}() {{ changed(); }}\n"),
        );
    }

    let runner = crate::git::GitRunner::discover_in(dir).expect("discover scratch repo");
    let mut app = App::new(Vec::new());
    app.stage_ops = Some(Box::new(runner));
    app.refresh();
    assert_eq!(app.view.files.len(), 200);

    // A stateful key-press closure (unlike this file's shared `press`
    // helper, which resets its pending-prefix/count on every call): the
    // count-prefix gesture below (`3j`) needs that state to survive across
    // presses, exactly like the real event loop.
    let keymap = Keymap::default_map();
    let mut pending = None;
    let mut pending_count = None;
    let mut press = |app: &mut App, code: KeyCode, mods: KeyModifiers| {
        let _ = super::super::dispatch_key(
            app,
            &keymap,
            &mut pending,
            &mut pending_count,
            KeyEvent::new(code, mods),
        );
    };
    let none = KeyModifiers::NONE;
    let ctrl = KeyModifiers::CONTROL;

    step(
        "journey A: launch on a 200-file scratch changeset",
        &format!("repo: {} files modified", app.view.files.len()),
    );

    press(&mut app, KeyCode::Char('`'), none);
    assert!(matches!(app.mode, Mode::Panel { .. }));
    assert_eq!(app.panel_cursor(), 0);
    step("press ` : panel focused, cursor at row 0", "cursor: 0");

    press(&mut app, KeyCode::Char('d'), ctrl);
    assert_eq!(app.panel_cursor(), 10);
    step(
        "press Ctrl-d: half page down (default viewport 20 -> step 10)",
        "cursor: 10",
    );

    press(&mut app, KeyCode::Char('G'), none);
    assert_eq!(app.panel_cursor(), 199);
    step("press G: jump to the last of 200 rows", "cursor: 199");

    press(&mut app, KeyCode::Char('g'), none);
    assert_eq!(app.panel_cursor(), 0);
    step("press g: jump back to the first row", "cursor: 0");

    press(&mut app, KeyCode::Char('3'), none);
    press(&mut app, KeyCode::Char('j'), none);
    assert_eq!(app.panel_cursor(), 3);
    step(
        "press 3 then j: count-prefixed step moves three rows in one gesture",
        "cursor: 3",
    );

    // -- Same gestures in the annotation list -------------------------------
    for i in 0..40 {
        app.annotations
            .add(
                Target::file(format!("f{i:03}.rs")),
                Classification::Question,
                format!("note {i}"),
            )
            .unwrap();
    }
    press(&mut app, KeyCode::Esc, none); // close the panel
    assert_eq!(app.mode, Mode::Normal);
    press(&mut app, KeyCode::Char('a'), none); // open the annotation list
    assert_eq!(app.mode, Mode::List);
    assert_eq!(app.list_cursor, 0);
    step(
        "press Esc then a: annotation list open over 40 annotations",
        "list_cursor: 0",
    );

    press(&mut app, KeyCode::Char('d'), ctrl);
    assert_eq!(app.list_cursor, 10);
    step(
        "press Ctrl-d in the annotation list: half page down (step 10)",
        "list_cursor: 10",
    );

    press(&mut app, KeyCode::Char('G'), none);
    assert_eq!(app.list_cursor, 39);
    step(
        "press G in the annotation list: jump to the last of 40 annotations",
        "list_cursor: 39",
    );

    press(&mut app, KeyCode::Char('g'), none);
    assert_eq!(app.list_cursor, 0);
    step(
        "press g in the annotation list: jump back to the first annotation",
        "list_cursor: 0",
    );

    press(&mut app, KeyCode::Char('3'), none);
    press(&mut app, KeyCode::Char('j'), none);
    assert_eq!(app.list_cursor, 3);
    step(
        "press 3 then j in the annotation list: steps three annotations",
        "list_cursor: 3",
    );

    if std::env::var("RQ_JOURNEY_DUMP").is_ok() {
        eprintln!("{log}");
    }
}
