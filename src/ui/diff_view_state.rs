//! [`DiffViewState`]: the "one view over one diff" state and the pure
//! navigation logic over it — the file list, which file is selected, the
//! flattened row model for that file, the cursor (row and column), the
//! scroll offset, and the viewport height.
//!
//! This is the seam the multi-file collapsible diff buffer generalizes:
//! everything here is expressed in terms of "rows for the
//! selected file", and every motion/clamp/visibility operation is a pure
//! transform over that state. Row *building* (which needs syntax
//! highlighting, the annotation store, and the git backend) stays in
//! [`super::App`], which feeds freshly built rows into this component; this
//! keeps `git`/highlighting concerns out of the view state.

use std::collections::HashMap;

use crate::annotate::Target;
use crate::diff::FileDiff;

use super::diff_wrap::WrapLayout;
use super::rows::{MIN_GUTTER_WIDTH, Row, anchor_row_index};

/// A reasonable default viewport height, used until the first frame reports
/// the real one. Arbitrary but generous enough that half-page motion isn't
/// degenerate before the first draw.
const DEFAULT_VIEWPORT_HEIGHT: usize = 20;

/// The context margin (vim's `scrolloff`): line motions keep this many rows
/// visible beyond the cursor in the direction of travel, and structural
/// jumps place the target header this many rows from the top of the
/// viewport. Degrades toward zero on viewports too small to honor it (see
/// [`DiffViewState::scroll_margin`]).
const SCROLLOFF: usize = 3;

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
    /// The diff pane's last-known inner content width (columns available
    /// for a [`Row::Line`]'s content, after the gutter/marker prefix), used
    /// to build [`DiffViewState::layout`]. `0` until the render loop calls
    /// [`DiffViewState::set_content_width`] for the first time — the wrap
    /// layout stays unbuilt at that width (see [`super::diff_wrap::WrapLayout::build`]),
    /// so every scroll/render accessor falls back to its pre-wrap identity
    /// behavior (one visual row per logical row) until then.
    content_width: usize,
    /// The soft-wrap layout for `rows` at `content_width`: maps each
    /// logical row to a visual height and supports visual<->logical
    /// conversion. Rebuilt by [`DiffViewState::rebuild_layout`] whenever
    /// `rows` or `content_width` change. Every accessor that reads it is
    /// gated on [`super::diff_wrap::WrapLayout::is_built_for`], so a stale
    /// or never-built layout degrades to the identity mapping rather than
    /// misreporting.
    layout: WrapLayout,
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
            content_width: 0,
            layout: WrapLayout::default(),
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

    /// Records the diff pane's current inner content width and rebuilds the
    /// wrap layout if it changed. Called once per frame by the render loop,
    /// next to [`DiffViewState::set_viewport_height`]. A no-op (no rebuild)
    /// when the width is unchanged, so repeated calls at a stable terminal
    /// size don't re-wrap every frame.
    pub fn set_content_width(&mut self, w: usize) {
        if w != self.content_width {
            self.content_width = w;
            self.rebuild_layout();
        }
    }

    /// Rebuilds the wrap layout from the current `rows`/`gutter_width` at
    /// the current `content_width`. Called after [`DiffViewState::set_content_width`]
    /// changes, and by the owner ([`super::App`]) after every row rebuild
    /// (rows and gutter width can change independently of the pane's
    /// width).
    pub fn rebuild_layout(&mut self) {
        self.layout = WrapLayout::build(&self.rows, self.gutter_width, self.content_width);
    }

    /// Whether [`DiffViewState::layout`] is trustworthy for the current
    /// `rows` — the single gate every visual accessor below checks before
    /// reading it, so a stale layout (never built, or built before the last
    /// row rebuild) falls back to the identity mapping instead of
    /// misreporting.
    fn layout_is_current(&self) -> bool {
        self.layout.is_built_for(self.rows.len())
    }

    /// The visual row offset (from the top of the buffer) where logical row
    /// `logical` begins. Identity (`logical`) when the wrap layout isn't
    /// built for the current rows (see [`DiffViewState::layout_is_current`]).
    pub(super) fn row_visual_start(&self, logical: usize) -> usize {
        if self.layout_is_current() {
            self.layout.visual_start(logical)
        } else {
            logical
        }
    }

    /// The visual height (in terminal rows) of logical row `logical`. `1`
    /// when the wrap layout isn't built for the current rows.
    fn row_visual_height(&self, logical: usize) -> usize {
        if self.layout_is_current() {
            self.layout.row_height(logical)
        } else {
            1
        }
    }

    /// The total visual row count across the whole buffer. `rows.len()`
    /// when the wrap layout isn't built for the current rows.
    fn total_visual(&self) -> usize {
        if self.layout_is_current() {
            self.layout.total_visual()
        } else {
            self.rows.len()
        }
    }

    /// The visual row where the cursor's logical row begins.
    fn cursor_visual_start(&self) -> usize {
        self.row_visual_start(self.cursor)
    }

    /// One past the last visual row the cursor's logical row occupies
    /// (exclusive), i.e. `cursor_visual_start() + row_visual_height(cursor)`.
    fn cursor_visual_end(&self) -> usize {
        self.row_visual_start(self.cursor) + self.row_visual_height(self.cursor)
    }

    /// The logical row owning visual row `v` — the render walk's entry
    /// point into the wrap layout. Identity (clamped into range) when the
    /// wrap layout isn't built for the current rows.
    pub(super) fn logical_of_visual(&self, v: usize) -> usize {
        if self.layout_is_current() {
            self.layout.logical_of_visual(v)
        } else {
            v.min(self.rows.len().saturating_sub(1))
        }
    }

    /// The char-range partition for a wrapped `Row::Line` at `logical` (see
    /// [`super::diff_wrap::WrapLayout::ranges_of`]). `None` when the wrap
    /// layout isn't built for the current rows, so unwrapped/identity
    /// rendering is the default when width hasn't been fed yet.
    pub(super) fn ranges_of(&self, logical: usize) -> Option<&[(usize, usize)]> {
        if self.layout_is_current() {
            self.layout.ranges_of(logical)
        } else {
            None
        }
    }

    fn half_page(&self) -> usize {
        (self.viewport_height / 2).max(1)
    }

    /// Moves the cursor down one addressable row, then scrolls to follow it
    /// with a [`SCROLLOFF`] context margin.
    pub fn cursor_down(&mut self) {
        if !self.rows.is_empty() {
            let target = (self.cursor + 1).min(self.max_cursor());
            self.cursor = self.nearest_addressable(target, true);
        }
        self.ensure_visible_scrolloff();
    }

    /// Moves the cursor up one addressable row, then scrolls to follow it
    /// with a [`SCROLLOFF`] context margin.
    pub fn cursor_up(&mut self) {
        if !self.rows.is_empty() {
            let target = self.cursor.saturating_sub(1);
            self.cursor = self.nearest_addressable(target, false);
        }
        self.ensure_visible_scrolloff();
    }

    /// Moves the cursor down half a viewport, then scrolls to follow it
    /// with a [`SCROLLOFF`] context margin.
    pub fn half_page_down(&mut self) {
        if !self.rows.is_empty() {
            let step = self.half_page();
            let target = (self.cursor + step).min(self.max_cursor());
            self.cursor = self.nearest_addressable(target, true);
        }
        self.ensure_visible_scrolloff();
    }

    /// Moves the cursor up half a viewport, then scrolls to follow it
    /// with a [`SCROLLOFF`] context margin.
    pub fn half_page_up(&mut self) {
        if !self.rows.is_empty() {
            let step = self.half_page();
            let target = self.cursor.saturating_sub(step);
            self.cursor = self.nearest_addressable(target, false);
        }
        self.ensure_visible_scrolloff();
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
    /// viewport_height)` (a plain edge clamp, no context margin), and
    /// re-derives [`DiffViewState::selected_file`] from the cursor's owning
    /// file. Used by the buffer-extreme jumps and by owners that reposition
    /// the cursor directly (anchor jumps, refresh clamping); line motions
    /// use [`DiffViewState::ensure_visible_scrolloff`] and structural jumps
    /// use [`DiffViewState::reveal_at_top`] instead.
    pub fn ensure_visible(&mut self) {
        self.selected_file = self.file_of_cursor();
        if self.rows.is_empty() {
            self.scroll = 0;
            return;
        }
        let cvs = self.cursor_visual_start();
        let cve = self.cursor_visual_end();
        if cvs < self.scroll {
            self.scroll = cvs;
        } else if cve > self.scroll + self.viewport_height {
            let height = self.row_visual_height(self.cursor);
            self.scroll = if height <= self.viewport_height {
                cve - self.viewport_height
            } else {
                cvs
            };
        }
    }

    /// The effective context margin: [`SCROLLOFF`], shrunk so that twice the
    /// margin still fits strictly inside the viewport. This keeps the
    /// top/bottom follow conditions disjoint (no oscillation) and degrades
    /// to a plain edge clamp on tiny viewports.
    fn scroll_margin(&self) -> usize {
        SCROLLOFF.min(self.viewport_height.saturating_sub(1) / 2)
    }

    /// The largest useful scroll offset: the one that puts the last row on
    /// the bottom line of the viewport. Zero when everything fits.
    fn max_scroll(&self) -> usize {
        self.total_visual().saturating_sub(self.viewport_height)
    }

    /// Like [`DiffViewState::ensure_visible`], but keeps a
    /// [`SCROLLOFF`]-sized context margin between the cursor and the
    /// viewport edge in the direction of travel, degrading gracefully at
    /// the buffer's edges (scroll never underflows past the top or runs
    /// past [`DiffViewState::max_scroll`]).
    pub fn ensure_visible_scrolloff(&mut self) {
        self.selected_file = self.file_of_cursor();
        if self.rows.is_empty() {
            self.scroll = 0;
            return;
        }
        let margin = self.scroll_margin();
        let cvs = self.cursor_visual_start();
        let cve = self.cursor_visual_end();
        if cvs < self.scroll + margin {
            self.scroll = cvs.saturating_sub(margin);
        } else if cve + margin > self.scroll + self.viewport_height {
            self.scroll = (cve + margin)
                .saturating_sub(self.viewport_height)
                .min(self.max_scroll());
        }
    }

    /// The reveal policy for structural jumps (hunk/section): the cursor
    /// sits on the target's header row and `span_end` is the exclusive end
    /// of that hunk/section. If the header row and the first few body rows
    /// below it — `min(remaining body rows, viewport_height / 2)` — are
    /// already fully visible, the scroll is left alone; otherwise the view
    /// scrolls so the header sits [`SCROLLOFF`] rows from the top of the
    /// viewport (clamped at the buffer's edges). Also re-derives
    /// [`DiffViewState::selected_file`], like every other motion clamp.
    fn reveal_at_top(&mut self, span_end: usize) {
        self.selected_file = self.file_of_cursor();
        if self.rows.is_empty() {
            self.scroll = 0;
            return;
        }
        let cvs = self.cursor_visual_start();
        let cve = self.cursor_visual_end();
        let body_visual = self.row_visual_start(span_end).saturating_sub(cve);
        let context = body_visual.min(self.viewport_height / 2);
        let already_fine = cvs >= self.scroll && cvs + context < self.scroll + self.viewport_height;
        if already_fine {
            return;
        }
        self.scroll = cvs
            .saturating_sub(self.scroll_margin())
            .min(self.max_scroll());
    }

    /// Row indices of every `HunkHeader` in `rows`.
    fn hunk_header_rows(rows: &[Row]) -> Vec<usize> {
        rows.iter()
            .enumerate()
            .filter_map(|(i, r)| matches!(r, Row::HunkHeader { .. }).then_some(i))
            .collect()
    }

    /// The exclusive end row of the hunk whose header row is `header`: the
    /// next hunk header in the same file section (`next_header`, if any —
    /// callers pass the following entry of the already-built header list),
    /// capped by the owning section's end so a hunk never bleeds into the
    /// next file.
    fn hunk_span_end(&self, header: usize, next_header: Option<usize>) -> usize {
        let file = self.file_of_row.get(header).copied().unwrap_or(0);
        let (_, section_end) = self.section_span(file);
        next_header.unwrap_or(usize::MAX).min(section_end)
    }

    /// Jumps the cursor to the next hunk header after the cursor anywhere in
    /// the buffer — crossing into neighboring expanded files' hunks
    /// automatically, since the whole buffer's rows are already built (a
    /// collapsed file contributes no hunk headers, so it's skipped). A no-op
    /// if there is no hunk after the cursor. The view reveals the target
    /// hunk near the top of the viewport (see
    /// [`DiffViewState::reveal_at_top`]).
    pub fn next_hunk(&mut self) {
        let headers = Self::hunk_header_rows(&self.rows);
        if let Some(pos) = headers.iter().position(|&i| i > self.cursor) {
            self.cursor = headers[pos];
            let end = self.hunk_span_end(headers[pos], headers.get(pos + 1).copied());
            self.reveal_at_top(end);
        }
    }

    /// Jumps the cursor to the previous hunk header before the cursor
    /// anywhere in the buffer, crossing file boundaries backward. A no-op if
    /// there is no hunk before the cursor. Same reveal policy as
    /// [`DiffViewState::next_hunk`] — landing near the top with the hunk
    /// body visible below is right for backward jumps too.
    pub fn prev_hunk(&mut self) {
        let headers = Self::hunk_header_rows(&self.rows);
        if let Some(pos) = headers.iter().rposition(|&i| i < self.cursor) {
            self.cursor = headers[pos];
            let end = self.hunk_span_end(headers[pos], headers.get(pos + 1).copied());
            self.reveal_at_top(end);
        }
    }

    /// Jumps the cursor to the next file's section header after the cursor.
    /// A no-op at the last section. Repurposes `Tab`'s old next-file
    /// meaning. Reveals the target section near the top of the viewport
    /// (see [`DiffViewState::reveal_at_top`]).
    pub fn next_section(&mut self) {
        if let Some(&next) = self.header_row_of_file.iter().find(|&&h| h > self.cursor) {
            self.cursor = next;
            self.cursor_col = 0;
            let (_, end) = self.section_span(self.file_of_cursor());
            self.reveal_at_top(end);
        }
    }

    /// Jumps the cursor to the previous section header before the cursor
    /// (the current file's own header first, then earlier files). A no-op
    /// before the first section. Repurposes `Shift-Tab`'s old meaning. Same
    /// reveal policy as [`DiffViewState::next_section`].
    pub fn prev_section(&mut self) {
        if let Some(&prev) = self
            .header_row_of_file
            .iter()
            .rev()
            .find(|&&h| h < self.cursor)
        {
            self.cursor = prev;
            self.cursor_col = 0;
            let (_, end) = self.section_span(self.file_of_cursor());
            self.reveal_at_top(end);
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

    /// Scrolls so the cursor sits at the vertical center of the viewport (the
    /// `zz` gesture), clamped so the buffer's last row never scrolls above
    /// the bottom of the viewport. A no-op on an empty diff.
    pub fn recenter_cursor(&mut self) {
        self.selected_file = self.file_of_cursor();
        if self.rows.is_empty() {
            self.scroll = 0;
            return;
        }
        let half = self.viewport_height / 2;
        let cvs = self.cursor_visual_start();
        self.scroll = cvs.saturating_sub(half).min(self.max_scroll());
    }

    /// Scrolls so the cursor sits near the top of the viewport (the `zt`
    /// gesture), keeping the same [`SCROLLOFF`]-derived margin above it that
    /// [`DiffViewState::reveal_at_top`] uses, and degrading the same way at
    /// the buffer's edges. A no-op on an empty diff.
    pub fn scroll_cursor_top(&mut self) {
        self.selected_file = self.file_of_cursor();
        if self.rows.is_empty() {
            self.scroll = 0;
            return;
        }
        let cvs = self.cursor_visual_start();
        self.scroll = cvs
            .saturating_sub(self.scroll_margin())
            .min(self.max_scroll());
    }

    /// Scrolls so the cursor sits near the bottom of the viewport (the `zb`
    /// gesture), keeping the same margin below it. A no-op on an empty diff.
    pub fn scroll_cursor_bottom(&mut self) {
        self.selected_file = self.file_of_cursor();
        if self.rows.is_empty() {
            self.scroll = 0;
            return;
        }
        let cve = self.cursor_visual_end();
        self.scroll = (cve + self.scroll_margin())
            .saturating_sub(self.viewport_height)
            .min(self.max_scroll());
    }

    /// Moves the column cursor to the start of the cursor row's content (the
    /// `0` motion). A no-op on non-[`Row::Line`] rows.
    pub fn move_column_to_line_start(&mut self) {
        if self.cursor_line_content().is_some() {
            self.cursor_col = 0;
        }
    }

    /// Moves the column cursor to the last character of the cursor row's
    /// content (the `$` motion). A no-op on non-[`Row::Line`] rows or an
    /// empty line.
    pub fn move_column_to_line_end(&mut self) {
        let Some(content) = self.cursor_line_content() else {
            return;
        };
        let len = content.chars().count();
        if len == 0 {
            return;
        }
        self.cursor_col = len - 1;
    }

    /// Moves the cursor down a full viewport (the `Ctrl-f` gesture), then
    /// scrolls to follow it with a [`SCROLLOFF`] context margin. Mirrors
    /// [`DiffViewState::half_page_down`] at double the step.
    pub fn full_page_down(&mut self) {
        if !self.rows.is_empty() {
            let step = self.viewport_height.max(1);
            let target = (self.cursor + step).min(self.max_cursor());
            self.cursor = self.nearest_addressable(target, true);
        }
        self.ensure_visible_scrolloff();
    }

    /// Moves the cursor up a full viewport (the `Ctrl-b` gesture). Mirrors
    /// [`DiffViewState::half_page_up`] at double the step.
    pub fn full_page_up(&mut self) {
        if !self.rows.is_empty() {
            let step = self.viewport_height.max(1);
            let target = self.cursor.saturating_sub(step);
            self.cursor = self.nearest_addressable(target, false);
        }
        self.ensure_visible_scrolloff();
    }

    /// The word (alphanumeric/underscore run) containing the column cursor,
    /// for the `*`/`#` gestures. `None` if the cursor isn't on a
    /// [`Row::Line`] row, or the char at the column cursor isn't a word char
    /// — deliberately narrower than vim's "nearest word forward", which this
    /// doesn't implement.
    pub fn word_at_cursor(&self) -> Option<String> {
        let content = self.cursor_line_content()?;
        let chars: Vec<char> = content.chars().collect();
        let col = self.cursor_col.min(chars.len().saturating_sub(1));
        let c = *chars.get(col)?;
        if !is_word_char(c) {
            return None;
        }
        let start = (0..=col)
            .rev()
            .take_while(|&i| is_word_char(chars[i]))
            .last()?;
        let end = (col..chars.len())
            .take_while(|&i| is_word_char(chars[i]))
            .last()?;
        Some(chars[start..=end].iter().collect())
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
    use crate::ui::rows::{ReviewMarker, StagedMarker, SyntaxSpans, build_multibuffer};

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
        let review_markers = vec![ReviewMarker::None; view.files.len()];
        let syntax = vec![SyntaxSpans::default(); view.files.len()];
        let mb = build_multibuffer(
            &view.files,
            collapsed,
            &markers,
            &review_markers,
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

    /// A single-file patch whose one hunk carries `n` context lines,
    /// producing a long scrollable section: row 0 is the file header, row 1
    /// the hunk header, rows `2..2 + n` the lines.
    fn long_raw(n: usize) -> String {
        let mut s = format!(
            "diff --git a/big.rs b/big.rs\nindex 1..2 100644\n--- a/big.rs\n+++ b/big.rs\n@@ -1,{n} +1,{n} @@\n"
        );
        for i in 0..n {
            s.push_str(&format!(" line{i}\n"));
        }
        s
    }

    /// One file, two hunks with room between them: rows 0 file header,
    /// 1 hunk-1 header, 2..=6 hunk-1 lines, 7 hunk-2 header, 8..=19 hunk-2
    /// lines (20 rows total).
    fn two_big_hunk_raw() -> String {
        let mut s = String::from(
            "diff --git a/f.rs b/f.rs\nindex 1..2 100644\n--- a/f.rs\n+++ b/f.rs\n@@ -1,5 +1,5 @@\n",
        );
        for i in 0..5 {
            s.push_str(&format!(" a{i}\n"));
        }
        s.push_str("@@ -50,12 +50,12 @@\n");
        for i in 0..12 {
            s.push_str(&format!(" b{i}\n"));
        }
        s
    }

    #[test]
    fn cursor_down_keeps_scrolloff_margin_below_the_cursor() {
        let mut view = view_with_raw("big.rs", &long_raw(30)); // 32 rows
        view.set_viewport_height(10);
        // Near the top the margin is satisfied without scrolling.
        for _ in 0..3 {
            view.cursor_down();
        }
        assert_eq!(view.cursor, 3);
        assert_eq!(view.scroll, 0);
        // Once the cursor nears the bottom edge, the view scrolls so 3 rows
        // stay visible below it: cursor sits at scroll + viewport - 1 - 3.
        for _ in 0..7 {
            view.cursor_down();
        }
        assert_eq!(view.cursor, 10);
        assert_eq!(view.scroll, 4);
    }

    #[test]
    fn cursor_up_keeps_scrolloff_margin_above_the_cursor() {
        let mut view = view_with_raw("big.rs", &long_raw(30)); // 32 rows
        view.set_viewport_height(10);
        view.jump_to_bottom(); // plain clamp: cursor 31, scroll 22
        assert_eq!(view.cursor, 31);
        assert_eq!(view.scroll, 22);
        for _ in 0..8 {
            view.cursor_up();
        }
        // Mid-buffer, moving up keeps 3 rows of context above the cursor.
        assert_eq!(view.cursor, 23);
        assert_eq!(view.scroll, view.cursor - 3);
        // All the way to the top: margin degrades, nothing underflows.
        for _ in 0..40 {
            view.cursor_up();
        }
        assert_eq!(view.cursor, 0);
        assert_eq!(view.scroll, 0);
    }

    #[test]
    fn scrolloff_stops_at_the_buffer_bottom_edge() {
        let mut view = view_with_raw("big.rs", &long_raw(30)); // 32 rows
        view.set_viewport_height(10);
        for _ in 0..40 {
            view.cursor_down();
        }
        // The margin degrades at the end: the last row sits on the bottom
        // line rather than scrolling past the buffer.
        assert_eq!(view.cursor, 31);
        assert_eq!(view.scroll, 22);
    }

    #[test]
    fn scrolloff_degrades_to_edge_clamp_on_a_tiny_viewport() {
        let mut view = view_with_raw("big.rs", &long_raw(30));
        view.set_viewport_height(2); // < 2 * SCROLLOFF + 1 -> margin 0
        for step in 0..20 {
            view.cursor_down();
            assert!(view.scroll <= view.cursor, "step {step}: cursor above view");
            assert!(
                view.cursor < view.scroll + 2,
                "step {step}: cursor below view"
            );
        }
        for step in 0..20 {
            view.cursor_up();
            assert!(view.scroll <= view.cursor, "step {step}: cursor above view");
            assert!(
                view.cursor < view.scroll + 2,
                "step {step}: cursor below view"
            );
        }
    }

    #[test]
    fn scrolloff_margin_shrinks_to_fit_a_small_viewport() {
        let mut view = view_with_raw("big.rs", &long_raw(30));
        view.set_viewport_height(5); // margin shrinks to (5 - 1) / 2 = 2
        for _ in 0..10 {
            view.cursor_down();
        }
        assert_eq!(view.cursor, 10);
        assert_eq!(view.cursor - view.scroll, 5 - 1 - 2);
    }

    #[test]
    fn forward_hunk_jump_reveals_the_hunk_near_the_top() {
        let mut view = view_with_raw("f.rs", &two_big_hunk_raw());
        view.set_viewport_height(8);
        view.next_hunk(); // hunk 1 header (row 1): header + body visible -> stay
        assert_eq!(view.cursor, 1);
        assert_eq!(view.scroll, 0);
        view.next_hunk(); // hunk 2 header (row 7): body off-screen -> reveal
        assert_eq!(view.cursor, 7);
        assert_eq!(view.scroll, 4, "header sits SCROLLOFF rows from the top");
    }

    #[test]
    fn hunk_jump_to_an_already_visible_hunk_leaves_scroll_alone() {
        let mut view = view_with_raw("f.rs", &two_big_hunk_raw());
        view.set_viewport_height(8);
        // Cursor a few rows into hunk 2's body, header + 4 body rows all on
        // screen (rows 4..12 visible).
        view.cursor = 10;
        view.scroll = 4;
        view.prev_hunk();
        assert_eq!(view.cursor, 7);
        assert_eq!(view.scroll, 4, "already-visible target: no scroll change");
    }

    #[test]
    fn backward_hunk_jump_from_below_reveals_the_hunk_near_the_top() {
        let mut view = view_with_raw("f.rs", &two_big_hunk_raw());
        view.set_viewport_height(8);
        view.jump_to_bottom(); // cursor 19, scroll 12: hunk 2 header off-screen above
        view.prev_hunk();
        assert_eq!(view.cursor, 7);
        assert_eq!(view.scroll, 4, "header sits SCROLLOFF rows from the top");
    }

    #[test]
    fn section_jump_reveals_the_file_near_the_top() {
        let files = vec![
            file_with_raw("a.rs", &long_raw(12)),
            file_with_raw("b.rs", &long_raw(12)),
        ];
        let mut view = multibuffer_view(files, &[false, false]);
        view.set_viewport_height(8);
        assert_eq!(view.header_row_of_file[1], 14);
        view.next_section(); // b.rs header (row 14), far below the viewport
        assert_eq!(view.cursor, 14);
        assert_eq!(view.scroll, 11, "header sits SCROLLOFF rows from the top");
        view.prev_section(); // a.rs header (row 0): clamps at the buffer top
        assert_eq!(view.cursor, 0);
        assert_eq!(view.scroll, 0);
    }

    #[test]
    fn reveal_clamps_scroll_at_the_buffer_end() {
        // b.rs is short (4 rows) and last: revealing its header at
        // SCROLLOFF from the top would scroll past the end of the buffer,
        // so the reveal clamps to the max useful scroll instead.
        let files = vec![
            file_with_raw("a.rs", &long_raw(12)),
            file_with_raw("b.rs", &one_hunk_raw("b.rs")),
        ];
        let mut view = multibuffer_view(files, &[false, false]);
        view.set_viewport_height(8);
        view.next_section(); // b.rs header (row 14); 18 rows total
        assert_eq!(view.cursor, 14);
        assert_eq!(view.scroll, 10, "clamped so the last row fills the bottom");
    }

    #[test]
    fn hunk_jump_on_a_tiny_viewport_keeps_the_header_visible() {
        let mut view = view_with_raw("f.rs", &two_big_hunk_raw());
        view.set_viewport_height(2); // margin degrades to 0
        view.next_hunk();
        view.next_hunk(); // hunk 2 header (row 7) -> revealed at the very top
        assert_eq!(view.cursor, 7);
        assert_eq!(view.scroll, 7);
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

    // -- zz/zt/zb: viewport recenter ------------------------------------------

    #[test]
    fn recenter_cursor_centers_the_viewport_on_the_cursor() {
        let mut view = view_with_raw("big.rs", &long_raw(30)); // 32 rows
        view.set_viewport_height(10);
        view.cursor = 20;
        view.recenter_cursor();
        assert_eq!(view.scroll, 15); // 20 - 10/2
    }

    #[test]
    fn recenter_cursor_clamps_at_both_buffer_edges() {
        let mut view = view_with_raw("big.rs", &long_raw(30)); // 32 rows
        view.set_viewport_height(10);
        view.cursor = 2;
        view.recenter_cursor();
        assert_eq!(view.scroll, 0, "near the top: scroll never underflows");
        view.jump_to_bottom(); // cursor 31
        view.recenter_cursor();
        assert_eq!(
            view.scroll,
            view.max_scroll(),
            "near the bottom: scroll clamps at the max useful offset"
        );
    }

    #[test]
    fn recenter_cursor_is_a_noop_on_empty_diff() {
        let mut view = DiffViewState::new(vec![]);
        view.recenter_cursor();
        assert_eq!(view.scroll, 0);
    }

    #[test]
    fn scroll_cursor_top_places_the_cursor_a_margin_below_the_top() {
        let mut view = view_with_raw("big.rs", &long_raw(30)); // 32 rows
        view.set_viewport_height(10);
        view.cursor = 20;
        view.scroll_cursor_top();
        assert_eq!(view.scroll, 20 - SCROLLOFF);
    }

    #[test]
    fn scroll_cursor_top_clamps_at_both_buffer_edges() {
        let mut view = view_with_raw("big.rs", &long_raw(30));
        view.set_viewport_height(10);
        view.cursor = 1;
        view.scroll_cursor_top();
        assert_eq!(view.scroll, 0);
        view.jump_to_bottom();
        view.scroll_cursor_top();
        assert_eq!(view.scroll, view.max_scroll());
    }

    #[test]
    fn scroll_cursor_bottom_places_the_cursor_a_margin_above_the_bottom() {
        let mut view = view_with_raw("big.rs", &long_raw(30)); // 32 rows
        view.set_viewport_height(10);
        view.cursor = 20;
        view.scroll_cursor_bottom();
        assert_eq!(view.scroll, 20 + SCROLLOFF + 1 - 10);
    }

    #[test]
    fn scroll_cursor_bottom_clamps_at_both_buffer_edges() {
        let mut view = view_with_raw("big.rs", &long_raw(30));
        view.set_viewport_height(10);
        view.cursor = 0;
        view.scroll_cursor_bottom();
        assert_eq!(view.scroll, 0);
        view.jump_to_bottom();
        view.scroll_cursor_bottom();
        assert_eq!(view.scroll, view.max_scroll());
    }

    // -- 0/$: line-start/line-end column motion -------------------------------

    #[test]
    fn line_start_and_end_jump_the_column_cursor_to_the_line_edges() {
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
        view.move_column_right();
        view.move_column_right();
        assert_eq!(view.effective_column(), Some(2));
        view.move_column_to_line_end();
        assert_eq!(view.effective_column(), Some(4));
        view.move_column_to_line_start();
        assert_eq!(view.effective_column(), Some(0));
    }

    #[test]
    fn line_start_and_end_are_noops_off_a_line_row() {
        let mut view = view_with_raw("f.rs", sample_raw());
        assert!(matches!(view.rows[view.cursor], Row::FileHeader { .. }));
        view.move_column_to_line_end();
        view.move_column_to_line_start();
        assert_eq!(view.cursor_col, 0);
    }

    // -- Ctrl-f/Ctrl-b: full-page scroll ---------------------------------------

    #[test]
    fn full_page_down_moves_a_full_viewport_then_clamps() {
        let mut view = view_with_raw("big.rs", &long_raw(30)); // 32 rows
        view.set_viewport_height(10);
        view.full_page_down();
        assert_eq!(view.cursor, 10);
        view.full_page_down();
        assert_eq!(view.cursor, 20);
        view.full_page_down();
        assert_eq!(view.cursor, 30);
        view.full_page_down();
        assert_eq!(view.cursor, 31, "clamps at the last addressable row");
    }

    #[test]
    fn full_page_up_moves_a_full_viewport_then_clamps_at_zero() {
        let mut view = view_with_raw("big.rs", &long_raw(30)); // 32 rows
        view.set_viewport_height(10);
        view.jump_to_bottom(); // cursor 31
        view.full_page_up();
        assert_eq!(view.cursor, 21);
        view.full_page_up();
        assert_eq!(view.cursor, 11);
        view.full_page_up();
        assert_eq!(view.cursor, 1);
        view.full_page_up();
        assert_eq!(view.cursor, 0);
    }

    #[test]
    fn full_page_motions_are_noops_on_empty_diff() {
        let mut view = DiffViewState::new(vec![]);
        view.full_page_down();
        assert_eq!(view.cursor, 0);
        view.full_page_up();
        assert_eq!(view.cursor, 0);
    }

    // -- */#: word-under-cursor extraction ------------------------------------

    #[test]
    fn word_at_cursor_returns_the_word_char_run_containing_the_column_cursor() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,1 +1,1 @@
 hello_world foo-bar
";
        let mut view = view_with_raw("f.rs", raw);
        view.cursor_down(); // hunk header
        view.cursor_down(); // "hello_world foo-bar"
        view.cursor_col = 3; // inside "hello_world"
        assert_eq!(view.word_at_cursor().as_deref(), Some("hello_world"));
        view.cursor_col = 16; // inside "bar"
        assert_eq!(view.word_at_cursor().as_deref(), Some("bar"));
    }

    #[test]
    fn word_at_cursor_is_none_on_punctuation_or_off_a_line_row() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,1 +1,1 @@
 foo-bar
";
        let mut view = view_with_raw("f.rs", raw);
        view.cursor_down();
        view.cursor_down(); // "foo-bar"
        view.cursor_col = 3; // the '-'
        assert_eq!(view.word_at_cursor(), None);

        view.cursor = 0; // file header row: not a Line row
        assert_eq!(view.word_at_cursor(), None);
    }

    // -- Soft wrap layout: scroll math over wrapped visual rows --------------

    /// A single-hunk file with exactly one content line — a context line
    /// `len` chars long, with no whitespace so it hard-splits at the wrap
    /// width deterministically. Rows: FileHeader(0), HunkHeader(1),
    /// Line(2) — the only addressable content row.
    fn raw_with_long_line(len: usize) -> String {
        let content = "x".repeat(len);
        format!(
            "diff --git a/f.rs b/f.rs\nindex 1..2 100644\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n {content}\n"
        )
    }

    #[test]
    fn set_content_width_wraps_a_long_line_and_grows_total_visual_rows() {
        let mut view = view_with_raw("f.rs", &raw_with_long_line(200));
        let rows_len = view.rows.len();
        view.set_viewport_height(10);
        // gutter_width is MIN_GUTTER_WIDTH (3) for this small file, so
        // content_col_offset(3) == 11; width 30 -> wrap width 19 for a
        // 200-char line -> several wrapped rows.
        view.set_content_width(30);
        assert!(view.row_visual_height(2) > 1);
        assert!(view.total_visual() > rows_len);
    }

    #[test]
    fn wrapped_cursor_line_stays_fully_visible_when_it_fits_the_viewport() {
        let mut view = view_with_raw("f.rs", &raw_with_long_line(60));
        view.set_content_width(30);
        view.set_viewport_height(20); // generous: the wrapped line's whole span fits
        view.jump_to_bottom(); // cursor lands on the long line (only content row)
        assert_eq!(view.cursor, 2);
        let cvs = view.cursor_visual_start();
        let cve = view.cursor_visual_end();
        assert!(
            cvs >= view.scroll,
            "cursor's top must not scroll above view"
        );
        assert!(
            cve <= view.scroll + view.viewport_height(),
            "cursor's whole span must fit in view"
        );
    }

    #[test]
    fn wrapped_cursor_line_taller_than_viewport_aligns_to_its_top() {
        let mut view = view_with_raw("f.rs", &raw_with_long_line(400));
        // wrap width 19 -> ceil(400/19) = 22 visual rows, taller than an
        // 8-row viewport.
        view.set_content_width(30);
        view.set_viewport_height(8);
        view.jump_to_bottom();
        assert_eq!(view.cursor, 2);
        assert!(view.row_visual_height(2) > view.viewport_height());
        let cvs = view.cursor_visual_start();
        assert_eq!(
            view.scroll, cvs,
            "a row taller than the viewport aligns to its top rather than \
             centering or bottom-anchoring"
        );
    }

    #[test]
    fn zero_content_width_never_wraps_identity_path() {
        // The render loop hasn't called `set_content_width` yet (or a code
        // path that never does, e.g. a test `App`): rows stay height 1.
        let view = view_with_raw("f.rs", &raw_with_long_line(200));
        assert_eq!(view.row_visual_height(2), 1);
        assert_eq!(view.total_visual(), view.rows.len());
    }
}
