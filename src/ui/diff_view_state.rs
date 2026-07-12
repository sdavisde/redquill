//! [`DiffViewState`]: the "one view over one diff" state and the pure
//! navigation logic over it — the file list, which file is selected, the
//! flattened row model for that file, the cursor (row and column), the
//! scroll offset, and the viewport height.
//!
//! This is the seam the multi-file collapsible diff buffer (spec 03) will
//! generalize: everything here is expressed in terms of "rows for the
//! selected file", and every motion/clamp/visibility operation is a pure
//! transform over that state. Row *building* (which needs syntax
//! highlighting, the annotation store, and the git backend) stays in
//! [`super::App`], which feeds freshly built rows into this component; this
//! keeps `git`/highlighting concerns out of the view state.

use std::collections::HashMap;

use crate::annotate::Target;
use crate::diff::FileDiff;

use super::rows::{MIN_GUTTER_WIDTH, Row, anchor_row_index};

/// A reasonable default viewport height, used until the first frame reports
/// the real one. Arbitrary but generous enough that half-page motion isn't
/// degenerate before the first draw.
const DEFAULT_VIEWPORT_HEIGHT: usize = 20;

/// The per-view state: the diffed files, which one is selected, the
/// flattened row model for that file, cursor and scroll positions, and the
/// layout choice. Owned by [`super::App`] as a single field; `App` delegates
/// every navigation gesture here and feeds rebuilt rows back in.
pub struct DiffViewState {
    /// Every file in the diff being reviewed.
    pub files: Vec<FileDiff>,
    /// The file whose section the cursor is in — a *derived* value kept in
    /// sync with [`DiffViewState::file_of_cursor`] on every motion, used for
    /// the sidebar highlight and the diff pane's title. Not the source of
    /// truth; the multibuffer is.
    pub selected_file: usize,
    /// The concatenated multi-file row buffer (all files' rows).
    pub rows: Vec<Row>,
    /// `file_of_row[i]` is the index into `files` of the file owning
    /// `rows[i]`.
    pub file_of_row: Vec<usize>,
    /// `header_row_of_file[f]` is the row index of file `f`'s section
    /// header.
    pub header_row_of_file: Vec<usize>,
    /// The gutter's digit width for `rows` — one value shared across every
    /// file's section so columns stay aligned, recomputed alongside `rows`
    /// on every rebuild (see [`super::rows::build_multibuffer`]). Starts at
    /// the minimum width until the first rebuild populates it.
    pub gutter_width: usize,
    /// Per-file collapse state, keyed by path so it survives refreshes that
    /// reorder or re-index files. An absent entry means expanded.
    collapsed: HashMap<String, bool>,
    /// The cursor's row index into `rows` — a LINE the user moves with
    /// j/k, Zed-style. Anchors annotation/staging/LSP commands.
    pub cursor: usize,
    /// The first visible row index into `rows` (the viewport follows the
    /// cursor).
    pub scroll: usize,
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
    /// Builds a fresh view state over `files`, with empty rows and every
    /// file expanded. The owner ([`super::App`]) populates the row model
    /// immediately afterward via its highlighting-aware rebuild.
    pub fn new(files: Vec<FileDiff>) -> DiffViewState {
        DiffViewState {
            files,
            selected_file: 0,
            rows: Vec::new(),
            file_of_row: Vec::new(),
            header_row_of_file: Vec::new(),
            gutter_width: MIN_GUTTER_WIDTH,
            collapsed: HashMap::new(),
            cursor: 0,
            scroll: 0,
            viewport_height: DEFAULT_VIEWPORT_HEIGHT,
            cursor_col: 0,
        }
    }

    /// The index (into `files`) of the file whose section the cursor is in,
    /// derived from the row-to-file map. `0` when the buffer is empty.
    pub fn file_of_cursor(&self) -> usize {
        self.file_of_row.get(self.cursor).copied().unwrap_or(0)
    }

    /// Whether `path`'s section is collapsed (header-only). Absent entries
    /// are expanded.
    pub fn is_collapsed(&self, path: &str) -> bool {
        self.collapsed.get(path).copied().unwrap_or(false)
    }

    /// Sets `path`'s collapse state (does not rebuild rows — the owner
    /// [`super::App`] rebuilds after mutating this, since rebuilding needs
    /// highlighting).
    pub fn set_collapsed(&mut self, path: &str, collapsed: bool) {
        self.collapsed.insert(path.to_string(), collapsed);
    }

    /// Drops collapse-map entries whose path fails `keep`, so files that
    /// left the review on a refresh don't leave stale collapse state behind.
    pub fn retain_collapsed(&mut self, keep: impl Fn(&str) -> bool) {
        self.collapsed.retain(|path, _| keep(path));
    }

    /// Whether the collapse map holds an entry for `path` at all (regardless
    /// of its value). Distinguishes "known and expanded" from "absent",
    /// which [`DiffViewState::is_collapsed`] alone cannot.
    pub fn collapse_contains(&self, path: &str) -> bool {
        self.collapsed.contains_key(path)
    }

    /// Toggles the collapse state of the file under the cursor, returning
    /// its path so the owner can rebuild. `None` on an empty buffer.
    pub fn toggle_collapse_at_cursor(&mut self) -> Option<String> {
        let path = self.files.get(self.file_of_cursor())?.path.clone();
        let now = !self.is_collapsed(&path);
        self.collapsed.insert(path.clone(), now);
        Some(path)
    }

    /// The `[start, end)` row span of file `f`'s section.
    pub fn section_span(&self, f: usize) -> (usize, usize) {
        let start = self.header_row_of_file.get(f).copied().unwrap_or(0);
        let end = self
            .header_row_of_file
            .get(f + 1)
            .copied()
            .unwrap_or(self.rows.len());
        (start, end)
    }

    /// Resolves `target`'s anchor row in the whole multibuffer: a
    /// [`Target::File`] maps to its section-header row, and line/hunk/range
    /// targets resolve within the owning file's row span (so a line number
    /// that also appears in another file's section can never be matched).
    /// `None` if the target's file isn't in the buffer, or the specific
    /// hunk/line the target names no longer exists in that section. The
    /// caller is responsible for expanding a collapsed target section first
    /// (a collapsed file contributes only its header, so only a File target
    /// would resolve).
    pub fn anchor_row_in_buffer(&self, target: &Target) -> Option<usize> {
        let index = self.files.iter().position(|f| f.path == target.path())?;
        let (start, end) = self.section_span(index);
        let local = anchor_row_index(&self.files[index], &self.rows[start..end], target)?;
        Some(start + local)
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

    /// Jumps the cursor to the first addressable row (top of the buffer),
    /// then scrolls to follow it. A no-op on an empty diff.
    pub fn jump_to_top(&mut self) {
        if !self.rows.is_empty() {
            self.cursor = self.nearest_addressable(0, true);
        }
        self.ensure_visible();
    }

    /// Jumps the cursor to the last addressable row (bottom of the buffer),
    /// then scrolls to follow it. A no-op on an empty diff.
    pub fn jump_to_bottom(&mut self) {
        if !self.rows.is_empty() {
            self.cursor = self.max_cursor();
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
    /// viewport_height)`, and re-derives [`DiffViewState::selected_file`]
    /// from the cursor's owning file. Called at the end of every motion, so
    /// the sidebar highlight and pane title always follow the cursor across
    /// file boundaries.
    pub fn ensure_visible(&mut self) {
        self.selected_file = self.file_of_cursor();
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

    /// Row indices of every `HunkHeader` in `rows`.
    fn hunk_header_rows(rows: &[Row]) -> Vec<usize> {
        rows.iter()
            .enumerate()
            .filter_map(|(i, r)| matches!(r, Row::HunkHeader { .. }).then_some(i))
            .collect()
    }

    /// Jumps the cursor to the next hunk header after the cursor anywhere in
    /// the buffer — crossing into neighboring expanded files' hunks
    /// automatically, since the whole buffer's rows are already built (a
    /// collapsed file contributes no hunk headers, so it's skipped). A no-op
    /// if there is no hunk after the cursor.
    pub fn next_hunk(&mut self) {
        if let Some(&next) = Self::hunk_header_rows(&self.rows)
            .iter()
            .find(|&&i| i > self.cursor)
        {
            self.cursor = next;
            self.ensure_visible();
        }
    }

    /// Jumps the cursor to the previous hunk header before the cursor
    /// anywhere in the buffer, crossing file boundaries backward. A no-op if
    /// there is no hunk before the cursor.
    pub fn prev_hunk(&mut self) {
        if let Some(&prev) = Self::hunk_header_rows(&self.rows)
            .iter()
            .rev()
            .find(|&&i| i < self.cursor)
        {
            self.cursor = prev;
            self.ensure_visible();
        }
    }

    /// Jumps the cursor to the next file's section header after the cursor.
    /// A no-op at the last section. Repurposes `Tab`'s old next-file meaning.
    pub fn next_section(&mut self) {
        if let Some(&next) = self.header_row_of_file.iter().find(|&&h| h > self.cursor) {
            self.cursor = next;
            self.cursor_col = 0;
            self.ensure_visible();
        }
    }

    /// Jumps the cursor to the previous section header before the cursor
    /// (the current file's own header first, then earlier files). A no-op
    /// before the first section. Repurposes `Shift-Tab`'s old meaning.
    pub fn prev_section(&mut self) {
        if let Some(&prev) = self
            .header_row_of_file
            .iter()
            .rev()
            .find(|&&h| h < self.cursor)
        {
            self.cursor = prev;
            self.cursor_col = 0;
            self.ensure_visible();
        }
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
    use crate::annotate::{AnnotationStore, Side, Target};
    use crate::git::RawFilePatch;
    use crate::ui::rows::{StagedMarker, SyntaxSpans, build_multibuffer};

    fn file_with_raw(path: &str, raw: &str) -> FileDiff {
        FileDiff::from_patch(&RawFilePatch {
            path: path.to_string(),
            old_path: None,
            raw: raw.to_string(),
            is_binary: false,
        })
        .unwrap()
    }

    /// Builds a `DiffViewState` over `files` (with the given per-file
    /// collapse flags), its multibuffer populated the way `App` would after
    /// a rebuild.
    fn multibuffer_view(files: Vec<FileDiff>, collapsed: &[bool]) -> DiffViewState {
        let mut view = DiffViewState::new(files);
        let paths: Vec<(String, bool)> = view
            .files
            .iter()
            .zip(collapsed)
            .map(|(f, &c)| (f.path.clone(), c))
            .collect();
        for (path, c) in paths {
            view.set_collapsed(&path, c);
        }
        let markers = vec![StagedMarker::None; view.files.len()];
        let syntax = vec![SyntaxSpans::default(); view.files.len()];
        let mb = build_multibuffer(
            &view.files,
            collapsed,
            &markers,
            &AnnotationStore::new(),
            &syntax,
        );
        view.rows = mb.rows;
        view.file_of_row = mb.file_of_row;
        view.header_row_of_file = mb.header_row_of_file;
        view.gutter_width = mb.gutter_width;
        view.selected_file = view.file_of_cursor();
        view
    }

    /// Builds a `DiffViewState` over one expanded file.
    fn view_with_raw(path: &str, raw: &str) -> DiffViewState {
        multibuffer_view(vec![file_with_raw(path, raw)], &[false])
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
    fn jump_to_bottom_then_top_hits_the_buffer_extremes() {
        let mut view = view_with_raw("f.rs", sample_raw());
        let last = view.max_cursor();
        assert_eq!(view.cursor, 0);
        view.jump_to_bottom();
        assert_eq!(view.cursor, last, "G lands on the last addressable row");
        view.jump_to_top();
        assert_eq!(
            view.cursor,
            view.nearest_addressable(0, true),
            "gg lands on the first addressable row"
        );
    }

    #[test]
    fn jump_extremes_are_no_ops_on_empty_diff() {
        let mut view = DiffViewState::new(vec![]);
        view.jump_to_bottom();
        assert_eq!(view.cursor, 0);
        view.jump_to_top();
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

    fn two_hunk_raw() -> &'static str {
        "\
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
"
    }

    #[test]
    fn next_hunk_advances_then_stops_at_last() {
        let mut view = view_with_raw("f.rs", two_hunk_raw());
        assert_eq!(view.cursor, 0);
        view.next_hunk(); // -> first hunk header
        let first = view.cursor;
        assert!(matches!(view.rows[first], Row::HunkHeader { .. }));
        view.next_hunk(); // -> second hunk header
        assert!(view.cursor > first);
        let last = view.cursor;
        view.next_hunk(); // no more hunks
        assert_eq!(view.cursor, last);
    }

    fn one_hunk_raw(path: &str) -> String {
        format!(
            "diff --git a/{path} b/{path}\nindex 1..2 100644\n--- a/{path}\n+++ b/{path}\n@@ -1,1 +1,1 @@\n-old\n+new\n"
        )
    }

    #[test]
    fn cursor_down_crosses_into_the_next_file_section() {
        let files = vec![
            file_with_raw("a.rs", &one_hunk_raw("a.rs")),
            file_with_raw("b.rs", &one_hunk_raw("b.rs")),
        ];
        let mut view = multibuffer_view(files, &[false, false]);
        // a.rs occupies rows 0..4; b.rs header is row 4.
        for _ in 0..4 {
            view.cursor_down();
        }
        assert_eq!(view.cursor, 4);
        assert_eq!(view.file_of_cursor(), 1);
        assert_eq!(view.selected_file, 1);
    }

    #[test]
    fn next_hunk_crosses_the_file_boundary() {
        let files = vec![
            file_with_raw("a.rs", &one_hunk_raw("a.rs")),
            file_with_raw("b.rs", &one_hunk_raw("b.rs")),
        ];
        let mut view = multibuffer_view(files, &[false, false]);
        view.next_hunk(); // a.rs hunk (row 1)
        assert_eq!(view.file_of_cursor(), 0);
        view.next_hunk(); // crosses into b.rs hunk (row 5)
        assert_eq!(view.file_of_cursor(), 1);
        assert!(matches!(view.rows[view.cursor], Row::HunkHeader { .. }));
    }

    #[test]
    fn next_hunk_skips_a_collapsed_section() {
        // b.rs collapsed contributes only its header (no hunk), so the next
        // hunk after a.rs's is c.rs's.
        let files = vec![
            file_with_raw("a.rs", &one_hunk_raw("a.rs")),
            file_with_raw("b.rs", &one_hunk_raw("b.rs")),
            file_with_raw("c.rs", &one_hunk_raw("c.rs")),
        ];
        let mut view = multibuffer_view(files, &[false, true, false]);
        view.next_hunk(); // a.rs hunk
        assert_eq!(view.file_of_cursor(), 0);
        view.next_hunk(); // -> c.rs hunk, skipping collapsed b.rs
        assert_eq!(view.file_of_cursor(), 2);
    }

    #[test]
    fn next_and_prev_section_jump_between_headers() {
        let files = vec![
            file_with_raw("a.rs", &one_hunk_raw("a.rs")),
            file_with_raw("b.rs", &one_hunk_raw("b.rs")),
        ];
        let mut view = multibuffer_view(files, &[false, false]);
        view.next_section(); // -> b.rs header (row 4)
        assert_eq!(view.cursor, view.header_row_of_file[1]);
        assert_eq!(view.file_of_cursor(), 1);
        view.next_section(); // no next section -> stays
        assert_eq!(view.cursor, view.header_row_of_file[1]);
        view.prev_section(); // -> a.rs header (row 0)
        assert_eq!(view.cursor, view.header_row_of_file[0]);
        view.prev_section(); // stays
        assert_eq!(view.cursor, 0);
    }

    #[test]
    fn toggle_collapse_at_cursor_flips_state_for_the_cursor_file() {
        let files = vec![
            file_with_raw("a.rs", &one_hunk_raw("a.rs")),
            file_with_raw("b.rs", &one_hunk_raw("b.rs")),
        ];
        let mut view = multibuffer_view(files, &[false, false]);
        view.next_section(); // cursor onto b.rs
        assert!(!view.is_collapsed("b.rs"));
        let path = view.toggle_collapse_at_cursor().unwrap();
        assert_eq!(path, "b.rs");
        assert!(view.is_collapsed("b.rs"));
        assert!(!view.is_collapsed("a.rs"));
    }

    #[test]
    fn anchor_row_in_buffer_resolves_targets_within_owning_section() {
        let files = vec![
            file_with_raw("a.rs", &one_hunk_raw("a.rs")),
            file_with_raw("b.rs", &one_hunk_raw("b.rs")),
        ];
        let view = multibuffer_view(files, &[false, false]);

        // A File target maps to that file's section-header row.
        assert_eq!(
            view.anchor_row_in_buffer(&Target::file("b.rs")),
            Some(view.header_row_of_file[1])
        );

        // A Line target resolves within the owning file's span — b.rs's new
        // line 1, never a.rs's identically-numbered line.
        let (b_start, _) = view.section_span(1);
        let expected = view.rows[b_start..]
            .iter()
            .position(|r| matches!(r, Row::Line(l) if l.new_line == Some(1)))
            .map(|i| b_start + i)
            .unwrap();
        assert_eq!(
            view.anchor_row_in_buffer(&Target::line("b.rs", 1, Side::New)),
            Some(expected)
        );
        assert!(expected >= view.header_row_of_file[1]);

        // A Hunk target resolves to b.rs's hunk header.
        let hunk = view
            .anchor_row_in_buffer(&Target::hunk("b.rs", 1, 1).unwrap())
            .unwrap();
        assert!(matches!(view.rows[hunk], Row::HunkHeader { .. }));
        assert_eq!(view.file_of_row[hunk], 1);

        // An unknown path resolves to nothing.
        assert_eq!(view.anchor_row_in_buffer(&Target::file("missing.rs")), None);
    }

    #[test]
    fn cursor_clamps_into_range_after_a_smaller_rebuild() {
        // Cursor deep in a two-file buffer, then the buffer shrinks to a
        // single short file: the clamp helpers keep the cursor addressable.
        let files = vec![
            file_with_raw("a.rs", two_hunk_raw()),
            file_with_raw("b.rs", &one_hunk_raw("b.rs")),
        ];
        let mut view = multibuffer_view(files, &[false, false]);
        for _ in 0..20 {
            view.cursor_down();
        }
        // Rebuild over a single small file.
        let small = multibuffer_view(vec![file_with_raw("a.rs", &one_hunk_raw("a.rs"))], &[false]);
        view.rows = small.rows;
        view.file_of_row = small.file_of_row;
        view.header_row_of_file = small.header_row_of_file;
        view.gutter_width = small.gutter_width;
        view.cursor = view.nearest_addressable(view.cursor.min(view.max_cursor()), false);
        view.ensure_visible();
        assert!(view.cursor < view.rows.len());
        assert!(view.rows[view.cursor].is_addressable());
    }
}
