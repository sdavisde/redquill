//! [`DiffViewState`]: the "one view over one diff" state and the pure
//! navigation logic over it — the file list, which file is selected, the
//! flattened row model for that file, the cursor (row and column), the
//! unified and side-by-side scroll offsets, the viewport height, and the
//! layout choice.
//!
//! This is the seam the multi-file collapsible diff buffer (spec 03) will
//! generalize: everything here is expressed in terms of "rows for the
//! selected file", and every motion/clamp/visibility operation is a pure
//! transform over that state. Row *building* (which needs syntax
//! highlighting, the annotation store, and the git backend) stays in
//! [`super::App`], which feeds freshly built rows into this component; this
//! keeps `git`/highlighting concerns out of the view state.

use crate::annotate::AnnotationStore;
use crate::diff::FileDiff;

use super::rows::{Row, SbsRow, SyntaxSpans, build_rows};

/// A reasonable default viewport height, used until the first frame reports
/// the real one. Arbitrary but generous enough that half-page motion isn't
/// degenerate before the first draw.
const DEFAULT_VIEWPORT_HEIGHT: usize = 20;

/// Which layout the diff pane renders: one column of unified hunks, or two
/// columns (old left, new right) built as a rendering-time view over the
/// same source [`Row`]s (see [`super::rows::build_sbs_rows`]). Toggled with
/// `t`; the choice is preserved across file switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    /// One column, old/new lines interleaved in patch order.
    #[default]
    Unified,
    /// Two columns, old and new lines side by side.
    SideBySide,
}

/// The per-view state: the diffed files, which one is selected, the
/// flattened row model for that file, cursor and scroll positions, and the
/// layout choice. Owned by [`super::App`] as a single field; `App` delegates
/// every navigation gesture here and feeds rebuilt rows back in.
pub struct DiffViewState {
    /// Every file in the diff being reviewed.
    pub files: Vec<FileDiff>,
    /// Index into `files` of the currently selected file.
    pub selected_file: usize,
    /// The flattened row model for `files[selected_file]`.
    pub rows: Vec<Row>,
    /// The side-by-side visual row model over `rows` (see
    /// [`super::rows::build_sbs_rows`]), rebuilt alongside it. Only consulted
    /// when `layout == ViewMode::SideBySide`.
    pub sbs_rows: Vec<SbsRow>,
    /// `rows` index -> `sbs_rows` index, sized to `rows.len()`, rebuilt
    /// alongside both. Used to keep [`DiffViewState::sbs_scroll`] following
    /// the source-row cursor in visual-row space.
    pub sbs_visual_of: Vec<usize>,
    /// The cursor's row index into `rows` — a LINE the user moves with
    /// j/k, Zed-style. Anchors future annotation/staging commands. Stays a
    /// source-row index in both view modes.
    pub cursor: usize,
    /// The first visible row index into `rows` (the unified viewport
    /// follows the cursor). Meaningless in `ViewMode::SideBySide` — see
    /// `sbs_scroll`.
    pub scroll: usize,
    /// The first visible row index into `sbs_rows` (the side-by-side
    /// viewport follows the cursor's paired visual row). Kept in sync with
    /// `cursor`/`viewport_height` by [`DiffViewState::ensure_visible`]
    /// alongside `scroll`, regardless of which view is active, so toggling
    /// `t` never needs a scroll-position fixup.
    pub sbs_scroll: usize,
    /// Which layout the diff pane renders. Preserved across file switches.
    pub layout: ViewMode,
    /// The diff pane's last-known content height, used to size half-page
    /// motion. Updated once per frame by the render loop.
    viewport_height: usize,
    /// The column cursor: a 0-based char index into the cursor row's
    /// content, meaningful only on [`Row::Line`] rows. Clamped wherever
    /// it's read (see [`DiffViewState::effective_column`]) rather than
    /// proactively on every vertical motion — a simple clamp, not vim's
    /// "desired column" memory.
    pub cursor_col: usize,
}

impl DiffViewState {
    /// Builds a fresh view state over `files`, with the first file selected
    /// and empty rows. The owner ([`super::App`]) populates the row model
    /// immediately afterward via its highlighting-aware rebuild.
    pub fn new(files: Vec<FileDiff>) -> DiffViewState {
        DiffViewState {
            files,
            selected_file: 0,
            rows: Vec::new(),
            sbs_rows: Vec::new(),
            sbs_visual_of: Vec::new(),
            cursor: 0,
            scroll: 0,
            sbs_scroll: 0,
            layout: ViewMode::default(),
            viewport_height: DEFAULT_VIEWPORT_HEIGHT,
            cursor_col: 0,
        }
    }

    /// Records the diff pane's current content height, for half-page
    /// motion. Called once per frame by the render loop.
    pub fn set_viewport_height(&mut self, height: usize) {
        self.viewport_height = height.max(1);
    }

    /// The last-known viewport height (see [`DiffViewState::set_viewport_height`]).
    pub fn viewport_height(&self) -> usize {
        self.viewport_height
    }

    /// Flips between unified and side-by-side layout. The cursor stays a
    /// source-row index, so nothing else needs to change — `scroll` and
    /// `sbs_scroll` are already kept in sync by
    /// [`DiffViewState::ensure_visible`] regardless of which view is active.
    pub fn toggle_view(&mut self) {
        self.layout = match self.layout {
            ViewMode::Unified => ViewMode::SideBySide,
            ViewMode::SideBySide => ViewMode::Unified,
        };
    }

    fn half_page(&self) -> usize {
        (self.viewport_height / 2).max(1)
    }

    /// Moves the cursor down one addressable row, then scrolls to follow it.
    pub fn cursor_down(&mut self) {
        if !self.rows.is_empty() {
            let target = (self.cursor + 1).min(self.max_cursor());
            self.cursor = self.nearest_addressable(target, true);
        }
        self.ensure_visible();
    }

    /// Moves the cursor up one addressable row, then scrolls to follow it.
    pub fn cursor_up(&mut self) {
        if !self.rows.is_empty() {
            let target = self.cursor.saturating_sub(1);
            self.cursor = self.nearest_addressable(target, false);
        }
        self.ensure_visible();
    }

    /// Moves the cursor down half a viewport, then scrolls to follow it.
    pub fn half_page_down(&mut self) {
        if !self.rows.is_empty() {
            let step = self.half_page();
            let target = (self.cursor + step).min(self.max_cursor());
            self.cursor = self.nearest_addressable(target, true);
        }
        self.ensure_visible();
    }

    /// Moves the cursor up half a viewport, then scrolls to follow it.
    pub fn half_page_up(&mut self) {
        if !self.rows.is_empty() {
            let step = self.half_page();
            let target = self.cursor.saturating_sub(step);
            self.cursor = self.nearest_addressable(target, false);
        }
        self.ensure_visible();
    }

    /// The last addressable row index (skipping trailing
    /// [`Row::Annotation`] display rows).
    pub fn max_cursor(&self) -> usize {
        self.rows.iter().rposition(Row::is_addressable).unwrap_or(0)
    }

    /// The nearest addressable row to `idx`, preferring the direction of
    /// travel (`prefer_forward` for downward motion, backward for upward
    /// motion) so runs of [`Row::Annotation`] display rows are skipped in
    /// one hop rather than landing on the first non-addressable row.
    pub fn nearest_addressable(&self, idx: usize, prefer_forward: bool) -> usize {
        if self.rows.is_empty() {
            return 0;
        }
        let idx = idx.min(self.rows.len() - 1);
        if self.rows[idx].is_addressable() {
            return idx;
        }
        let forward = (idx..self.rows.len()).find(|&i| self.rows[i].is_addressable());
        let backward = (0..=idx).rev().find(|&i| self.rows[i].is_addressable());
        if prefer_forward {
            forward.or(backward).unwrap_or(0)
        } else {
            backward.or(forward).unwrap_or(0)
        }
    }

    /// Scrolls just enough to keep the cursor inside `[scroll, scroll +
    /// viewport_height)`.
    pub fn ensure_visible(&mut self) {
        if self.rows.is_empty() {
            self.scroll = 0;
            self.sbs_scroll = 0;
            return;
        }
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + self.viewport_height {
            self.scroll = self.cursor + 1 - self.viewport_height;
        }

        // Side-by-side scrolls in visual-row space (a paired removed/added
        // line occupies one visual row, not two), kept in sync here
        // unconditionally so toggling `t` never needs a fixup.
        let visual_cursor = self.sbs_visual_of.get(self.cursor).copied().unwrap_or(0);
        if visual_cursor < self.sbs_scroll {
            self.sbs_scroll = visual_cursor;
        } else if visual_cursor >= self.sbs_scroll + self.viewport_height {
            self.sbs_scroll = visual_cursor + 1 - self.viewport_height;
        }
    }

    /// Row indices of every `HunkHeader` in `rows`.
    fn hunk_header_rows(rows: &[Row]) -> Vec<usize> {
        rows.iter()
            .enumerate()
            .filter_map(|(i, r)| matches!(r, Row::HunkHeader { .. }).then_some(i))
            .collect()
    }

    /// Jumps the cursor to the next hunk header after the cursor within the
    /// current file, returning `true` if it moved. Returns `false` (leaving
    /// the cursor put) when the current file has no hunk after the cursor —
    /// the owner then probes subsequent files via
    /// [`DiffViewState::probe_first_hunk_row`].
    pub fn next_hunk_in_file(&mut self) -> bool {
        if let Some(&next) = Self::hunk_header_rows(&self.rows)
            .iter()
            .find(|&&i| i > self.cursor)
        {
            self.cursor = next;
            self.ensure_visible();
            true
        } else {
            false
        }
    }

    /// Jumps the cursor to the previous hunk header before the cursor within
    /// the current file, returning `true` if it moved. Returns `false` when
    /// the current file has no hunk before the cursor — the owner then
    /// probes earlier files via [`DiffViewState::probe_last_hunk_row`].
    pub fn prev_hunk_in_file(&mut self) -> bool {
        if let Some(&prev) = Self::hunk_header_rows(&self.rows)
            .iter()
            .rev()
            .find(|&&i| i < self.cursor)
        {
            self.cursor = prev;
            self.ensure_visible();
            true
        } else {
            false
        }
    }

    /// The row index of `files[index]`'s first hunk header, via a cheap
    /// unhighlighted probe (only the file actually landed on gets its rows
    /// rebuilt with real highlighting, by the owner). `None` if that file
    /// has no hunk at all.
    pub fn probe_first_hunk_row(
        &self,
        annotations: &AnnotationStore,
        index: usize,
    ) -> Option<usize> {
        let probe = build_rows(&self.files[index], annotations, SyntaxSpans::default());
        Self::hunk_header_rows(&probe).first().copied()
    }

    /// The row index of `files[index]`'s last hunk header, via a cheap
    /// unhighlighted probe. `None` if that file has no hunk at all.
    pub fn probe_last_hunk_row(
        &self,
        annotations: &AnnotationStore,
        index: usize,
    ) -> Option<usize> {
        let probe = build_rows(&self.files[index], annotations, SyntaxSpans::default());
        Self::hunk_header_rows(&probe).last().copied()
    }

    /// The cursor row's content, if it's a [`Row::Line`] (the only rows
    /// with a meaningful column).
    fn cursor_line_content(&self) -> Option<&str> {
        match self.rows.get(self.cursor) {
            Some(Row::Line(line)) => Some(line.content.as_str()),
            _ => None,
        }
    }

    /// The 0-based char column, clamped into the cursor row's content
    /// bounds. `None` if the cursor isn't on a [`Row::Line`] row, or that
    /// row's content is empty (nothing to highlight).
    pub fn effective_column(&self) -> Option<usize> {
        let content = self.cursor_line_content()?;
        let len = content.chars().count();
        if len == 0 {
            return None;
        }
        Some(self.cursor_col.min(len - 1))
    }

    pub fn move_column_left(&mut self) {
        let Some(col) = self.effective_column() else {
            return;
        };
        self.cursor_col = col.saturating_sub(1);
    }

    pub fn move_column_right(&mut self) {
        let Some(content) = self.cursor_line_content() else {
            return;
        };
        let len = content.chars().count();
        if len == 0 {
            return;
        }
        let col = self.cursor_col.min(len - 1);
        self.cursor_col = (col + 1).min(len - 1);
    }

    pub fn move_word_forward(&mut self) {
        let Some(content) = self.cursor_line_content() else {
            return;
        };
        let chars: Vec<char> = content.chars().collect();
        if chars.is_empty() {
            return;
        }
        let mut i = self.cursor_col.min(chars.len() - 1);
        if is_word_char(chars[i]) {
            while i < chars.len() && is_word_char(chars[i]) {
                i += 1;
            }
        }
        while i < chars.len() && !is_word_char(chars[i]) {
            i += 1;
        }
        self.cursor_col = i.min(chars.len() - 1);
    }

    pub fn move_word_backward(&mut self) {
        let Some(content) = self.cursor_line_content() else {
            return;
        };
        let chars: Vec<char> = content.chars().collect();
        if chars.is_empty() {
            return;
        }
        let mut i = self.cursor_col.min(chars.len() - 1);
        if i == 0 {
            self.cursor_col = 0;
            return;
        }
        i -= 1;
        while i > 0 && !is_word_char(chars[i]) {
            i -= 1;
        }
        while i > 0 && is_word_char(chars[i - 1]) {
            i -= 1;
        }
        self.cursor_col = i;
    }
}

/// Whether `c` is part of a "word" for `w`/`b` column motion: alphanumeric
/// or underscore.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::RawFilePatch;
    use crate::ui::rows::build_sbs_rows;

    fn file_with_raw(path: &str, raw: &str) -> FileDiff {
        FileDiff::from_patch(&RawFilePatch {
            path: path.to_string(),
            old_path: None,
            raw: raw.to_string(),
            is_binary: false,
        })
        .unwrap()
    }

    /// Builds a `DiffViewState` over one file with its rows populated
    /// (unhighlighted), the way `App` would after a rebuild.
    fn view_with_raw(path: &str, raw: &str) -> DiffViewState {
        let file = file_with_raw(path, raw);
        let mut view = DiffViewState::new(vec![file]);
        let rows = build_rows(
            &view.files[0],
            &AnnotationStore::new(),
            SyntaxSpans::default(),
        );
        let (sbs_rows, sbs_visual_of) = build_sbs_rows(&view.files[0], &rows);
        view.rows = rows;
        view.sbs_rows = sbs_rows;
        view.sbs_visual_of = sbs_visual_of;
        view
    }

    fn sample_raw() -> &'static str {
        "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,3 +1,3 @@
 abcde
-beta
+gamma
"
    }

    #[test]
    fn nearest_addressable_on_empty_rows_is_zero() {
        let view = DiffViewState::new(vec![]);
        assert_eq!(view.nearest_addressable(5, true), 0);
        assert_eq!(view.max_cursor(), 0);
    }

    #[test]
    fn cursor_down_clamps_at_last_addressable_row() {
        let mut view = view_with_raw("f.rs", sample_raw());
        let last = view.max_cursor();
        for _ in 0..20 {
            view.cursor_down();
        }
        assert_eq!(view.cursor, last);
    }

    #[test]
    fn cursor_up_clamps_at_zero() {
        let mut view = view_with_raw("f.rs", sample_raw());
        view.cursor_up();
        assert_eq!(view.cursor, 0);
    }

    #[test]
    fn ensure_visible_follows_cursor_within_viewport() {
        let mut view = view_with_raw("f.rs", sample_raw());
        view.set_viewport_height(2);
        for _ in 0..4 {
            view.cursor_down();
        }
        assert!(view.scroll <= view.cursor);
        assert!(view.cursor < view.scroll + 2);
    }

    #[test]
    fn toggle_view_round_trips() {
        let mut view = view_with_raw("f.rs", sample_raw());
        assert_eq!(view.layout, ViewMode::Unified);
        view.toggle_view();
        assert_eq!(view.layout, ViewMode::SideBySide);
        view.toggle_view();
        assert_eq!(view.layout, ViewMode::Unified);
    }

    #[test]
    fn column_motion_clamps_within_line() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,1 +1,1 @@
 abcde
";
        let mut view = view_with_raw("f.rs", raw);
        view.cursor_down(); // hunk header
        view.cursor_down(); // "abcde"
        assert_eq!(view.effective_column(), Some(0));
        view.move_column_right();
        view.move_column_right();
        assert_eq!(view.effective_column(), Some(2));
        for _ in 0..10 {
            view.move_column_right();
        }
        assert_eq!(view.effective_column(), Some(4));
        for _ in 0..10 {
            view.move_column_left();
        }
        assert_eq!(view.effective_column(), Some(0));
    }

    #[test]
    fn next_hunk_in_file_reports_whether_it_moved() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,1 +1,1 @@
-old0
+new0
@@ -11,1 +11,1 @@
-old1
+new1
";
        let mut view = view_with_raw("f.rs", raw);
        assert!(view.next_hunk_in_file()); // -> first hunk header
        assert!(view.next_hunk_in_file()); // -> second hunk header
        assert!(!view.next_hunk_in_file()); // no more hunks in this file
    }
}
