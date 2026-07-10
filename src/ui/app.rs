//! [`App`]: the TUI's state and the pure state transitions every [`Action`]
//! performs. No rendering or terminal I/O lives here — these are plain
//! methods, unit-tested without a terminal.

use crate::annotate::AnnotationStore;
use crate::diff::FileDiff;

use super::keymap::Action;
use super::rows::{Row, build_rows};

/// A reasonable default viewport height, used until the first frame reports
/// the real one. Arbitrary but generous enough that half-page motion isn't
/// degenerate before the first draw.
const DEFAULT_VIEWPORT_HEIGHT: usize = 20;

/// The TUI's full state: the diffed files, which one is selected, the
/// flattened row model for that file, cursor and scroll position, help
/// overlay visibility, and the annotation store the session accumulates
/// into (emitted to stdout on quit).
pub struct App {
    /// Every file in the diff being reviewed.
    pub files: Vec<FileDiff>,
    /// Index into `files` of the currently selected file.
    pub selected_file: usize,
    /// The flattened row model for `files[selected_file]`.
    pub rows: Vec<Row>,
    /// The cursor's row index into `rows` — a LINE the user moves with
    /// j/k, Zed-style. Anchors future annotation/staging commands.
    pub cursor: usize,
    /// The first visible row index (the viewport follows the cursor).
    pub scroll: usize,
    /// Whether the help overlay is open.
    pub help_open: bool,
    /// Annotations accumulated this session.
    pub annotations: AnnotationStore,
    /// The diff pane's last-known content height, used to size half-page
    /// motion. Updated once per frame by the render loop.
    viewport_height: usize,
}

impl App {
    /// Builds a fresh `App` over `files`, with the first file selected.
    pub fn new(files: Vec<FileDiff>) -> App {
        let rows = files.first().map(build_rows).unwrap_or_default();
        App {
            files,
            selected_file: 0,
            rows,
            cursor: 0,
            scroll: 0,
            help_open: false,
            annotations: AnnotationStore::new(),
            viewport_height: DEFAULT_VIEWPORT_HEIGHT,
        }
    }

    /// Records the diff pane's current content height, for half-page
    /// motion. Called once per frame by the render loop.
    pub fn set_viewport_height(&mut self, height: usize) {
        self.viewport_height = height.max(1);
    }

    /// The last-known viewport height (see [`App::set_viewport_height`]).
    pub fn viewport_height(&self) -> usize {
        self.viewport_height
    }

    /// Applies one [`Action`] as a state transition.
    ///
    /// `Quit` and `QuitDiscard` are no-ops here — the event loop intercepts
    /// them before they reach `apply` and ends the session instead.
    pub fn apply(&mut self, action: Action) {
        match action {
            Action::CursorDown => {
                self.cursor = (self.cursor + 1).min(self.max_cursor());
                self.ensure_visible();
            }
            Action::CursorUp => {
                self.cursor = self.cursor.saturating_sub(1);
                self.ensure_visible();
            }
            Action::HalfPageDown => {
                let step = self.half_page();
                self.cursor = (self.cursor + step).min(self.max_cursor());
                self.ensure_visible();
            }
            Action::HalfPageUp => {
                let step = self.half_page();
                self.cursor = self.cursor.saturating_sub(step);
                self.ensure_visible();
            }
            Action::NextHunk => self.next_hunk(),
            Action::PrevHunk => self.prev_hunk(),
            Action::NextFile => self.switch_file(self.selected_file + 1),
            Action::PrevFile => {
                if let Some(prev) = self.selected_file.checked_sub(1) {
                    self.switch_file(prev);
                }
            }
            Action::ToggleHelp => self.help_open = !self.help_open,
            Action::Quit | Action::QuitDiscard => {}
        }
    }

    fn half_page(&self) -> usize {
        (self.viewport_height / 2).max(1)
    }

    fn max_cursor(&self) -> usize {
        self.rows.len().saturating_sub(1)
    }

    /// Scrolls just enough to keep the cursor inside `[scroll, scroll +
    /// viewport_height)`.
    fn ensure_visible(&mut self) {
        if self.rows.is_empty() {
            self.scroll = 0;
            return;
        }
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + self.viewport_height {
            self.scroll = self.cursor + 1 - self.viewport_height;
        }
    }

    /// Switches to file `index`, resetting cursor and scroll to the top.
    /// Out-of-range indices are a no-op (this is how `NextFile`/`PrevFile`
    /// clamp at the first/last file rather than wrapping).
    fn switch_file(&mut self, index: usize) {
        if index >= self.files.len() {
            return;
        }
        self.selected_file = index;
        self.rows = build_rows(&self.files[index]);
        self.cursor = 0;
        self.scroll = 0;
    }

    /// Row indices of every `HunkHeader` in `rows`.
    fn hunk_header_rows(rows: &[Row]) -> Vec<usize> {
        rows.iter()
            .enumerate()
            .filter_map(|(i, r)| matches!(r, Row::HunkHeader { .. }).then_some(i))
            .collect()
    }

    /// Jumps the cursor to the next hunk header after the cursor, crossing
    /// into the next file (at its first hunk) if the current file has none
    /// left. A no-op if there is no next hunk anywhere.
    fn next_hunk(&mut self) {
        if let Some(&next) = Self::hunk_header_rows(&self.rows)
            .iter()
            .find(|&&i| i > self.cursor)
        {
            self.cursor = next;
            self.ensure_visible();
            return;
        }

        for index in (self.selected_file + 1)..self.files.len() {
            let rows = build_rows(&self.files[index]);
            if let Some(&first) = Self::hunk_header_rows(&rows).first() {
                self.selected_file = index;
                self.rows = rows;
                self.cursor = first;
                self.scroll = 0;
                self.ensure_visible();
                return;
            }
        }
    }

    /// Jumps the cursor to the previous hunk header before the cursor,
    /// crossing into the previous file (at its last hunk) if the current
    /// file has none before the cursor. A no-op if there is no previous
    /// hunk anywhere.
    fn prev_hunk(&mut self) {
        if let Some(&prev) = Self::hunk_header_rows(&self.rows)
            .iter()
            .rev()
            .find(|&&i| i < self.cursor)
        {
            self.cursor = prev;
            self.ensure_visible();
            return;
        }

        for index in (0..self.selected_file).rev() {
            let rows = build_rows(&self.files[index]);
            if let Some(&last) = Self::hunk_header_rows(&rows).last() {
                self.selected_file = index;
                self.rows = rows;
                self.cursor = last;
                self.scroll = 0;
                self.ensure_visible();
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::RawFilePatch;

    fn file(path: &str, hunk_count: usize) -> FileDiff {
        let mut raw = format!(
            "diff --git a/{path} b/{path}\nindex 1..2 100644\n--- a/{path}\n+++ b/{path}\n"
        );
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

    #[test]
    fn cursor_down_clamps_at_last_row() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        let last = app.rows.len() - 1;
        for _ in 0..20 {
            app.apply(Action::CursorDown);
        }
        assert_eq!(app.cursor, last);
    }

    #[test]
    fn cursor_up_clamps_at_zero() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.apply(Action::CursorUp);
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn cursor_motion_on_empty_diff_stays_at_zero() {
        let mut app = App::new(vec![]);
        app.apply(Action::CursorDown);
        assert_eq!(app.cursor, 0);
        app.apply(Action::HalfPageDown);
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn half_page_motion_uses_last_known_viewport_height() {
        // 5 hunks -> 1 + 5*3 = 16 rows, plenty of headroom for a
        // half-page-of-10 step in either direction.
        let mut app = App::new(vec![file("a.rs", 5)]);
        app.set_viewport_height(10);
        app.apply(Action::HalfPageDown);
        assert_eq!(app.cursor, 5);
        app.apply(Action::HalfPageUp);
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn half_page_never_steps_by_zero_on_tiny_viewport() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.set_viewport_height(1);
        app.apply(Action::HalfPageDown);
        assert_eq!(app.cursor, 1);
    }

    #[test]
    fn ensure_visible_scrolls_down_to_follow_cursor() {
        let mut app = App::new(vec![file("a.rs", 3)]);
        app.set_viewport_height(3);
        for _ in 0..6 {
            app.apply(Action::CursorDown);
        }
        assert_eq!(app.cursor, 6);
        assert!(app.scroll <= app.cursor);
        assert!(app.cursor < app.scroll + 3);
    }

    #[test]
    fn ensure_visible_scrolls_up_to_follow_cursor() {
        let mut app = App::new(vec![file("a.rs", 3)]);
        app.set_viewport_height(3);
        for _ in 0..6 {
            app.apply(Action::CursorDown);
        }
        for _ in 0..6 {
            app.apply(Action::CursorUp);
        }
        assert_eq!(app.cursor, 0);
        assert_eq!(app.scroll, 0);
    }

    #[test]
    fn next_hunk_jumps_within_file() {
        let mut app = App::new(vec![file("a.rs", 2)]);
        app.apply(Action::NextHunk);
        let Row::HunkHeader { hunk_index, .. } = &app.rows[app.cursor] else {
            panic!("expected hunk header at cursor");
        };
        assert_eq!(*hunk_index, 0);

        app.apply(Action::NextHunk);
        let Row::HunkHeader { hunk_index, .. } = &app.rows[app.cursor] else {
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
        assert_eq!(app.selected_file, 1);
        assert!(matches!(app.rows[app.cursor], Row::HunkHeader { .. }));
    }

    #[test]
    fn next_hunk_at_last_file_last_hunk_is_no_op() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.apply(Action::NextHunk);
        let cursor_before = app.cursor;
        let file_before = app.selected_file;
        app.apply(Action::NextHunk);
        assert_eq!(app.cursor, cursor_before);
        assert_eq!(app.selected_file, file_before);
    }

    #[test]
    fn prev_hunk_crosses_file_boundary_backwards() {
        let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
        app.apply(Action::NextFile); // move to b.rs, cursor reset to top (FileHeader)
        assert_eq!(app.selected_file, 1);
        app.apply(Action::PrevHunk); // no hunk header before cursor in b.rs -> cross back
        assert_eq!(app.selected_file, 0);
        assert!(matches!(app.rows[app.cursor], Row::HunkHeader { .. }));
    }

    #[test]
    fn prev_hunk_at_first_file_before_first_hunk_is_no_op() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        let cursor_before = app.cursor;
        app.apply(Action::PrevHunk);
        assert_eq!(app.cursor, cursor_before);
        assert_eq!(app.selected_file, 0);
    }

    #[test]
    fn next_file_switches_and_resets_cursor() {
        let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
        app.apply(Action::CursorDown);
        app.apply(Action::NextFile);
        assert_eq!(app.selected_file, 1);
        assert_eq!(app.cursor, 0);
        assert_eq!(app.scroll, 0);
    }

    #[test]
    fn next_file_clamps_at_last_file() {
        let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
        app.apply(Action::NextFile);
        app.apply(Action::NextFile);
        assert_eq!(app.selected_file, 1);
    }

    #[test]
    fn prev_file_clamps_at_first_file() {
        let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
        app.apply(Action::PrevFile);
        assert_eq!(app.selected_file, 0);
    }

    #[test]
    fn prev_file_switches_and_resets_cursor() {
        let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
        app.apply(Action::NextFile);
        app.apply(Action::CursorDown);
        app.apply(Action::PrevFile);
        assert_eq!(app.selected_file, 0);
        assert_eq!(app.cursor, 0);
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
        let cursor = app.cursor;
        app.apply(Action::Quit);
        app.apply(Action::QuitDiscard);
        assert_eq!(app.cursor, cursor);
    }
}
