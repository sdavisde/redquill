//! The `space` staging gesture: resolving the cursor row (or a Visual
//! selection) into a staging granularity and applying it through the
//! existing [`StageOps`] trait seam. Kept out of [`super::App`] so the
//! coordinator stays thin; these functions read the view/patches/target off
//! `App` and drive git only through `StageOps`.

use std::collections::{HashMap, HashSet};

use crate::diff::LineOrigin;
use crate::git::{StagingMode, build_hunk_patch, build_line_patch};

use super::App;
use super::Mode;
use super::diff_view_state::DiffViewState;
use super::list_filter::ListFilter;
use super::rows::Row;
use super::stage_ops::{staged_from_status, staged_states_from_status};

/// The staging granularity a `space` gesture resolved to.
enum StageGesture {
    /// The whole file (file-header/binary rows, and every gesture on a
    /// synthetic untracked file).
    WholeFile,
    /// One hunk, by index into the selected file's hunks.
    Hunk(usize),
    /// Selected body-line indices within one hunk (Visual mode).
    Lines(usize, Vec<usize>),
}

/// Applies the `space` staging gesture. Direction depends on the diff
/// target: working tree stages, staged unstages, range is read-only.
/// Granularity depends on the cursor row (Normal: hunk on line/hunk rows,
/// whole file on file-header/binary rows) or the Visual selection (the
/// selected `+`/`-` lines of a single hunk). Synthetic untracked files
/// always stage whole-file — there is no index blob to apply hunk/line
/// patches against. Failures and no-op cases set a footer message and leave
/// state unchanged.
pub(super) fn toggle_stage(app: &mut App) {
    if !matches!(app.mode, Mode::Normal | Mode::Visual { .. }) {
        return;
    }
    toggle_stage_at_cursor(app, false);
}

/// The git panel's whole-file `space` gesture: same capability checks and
/// stage/unstage direction as [`toggle_stage`], but always the whole
/// highlighted file — the diff cursor's row kind is follow-sync
/// bookkeeping here, not a user gesture, so it must never pick the
/// granularity.
pub(super) fn toggle_stage_whole_file(app: &mut App) {
    toggle_stage_at_cursor(app, true);
}

/// The mode-independent core [`toggle_stage`] wraps: capability checks,
/// gesture resolution against the cursor row, and the apply/refresh
/// epilogue. Split from the mode guard so other entry points can route
/// here legitimately instead of spoofing a mode. `force_whole_file`
/// bypasses cursor-row resolution entirely.
fn toggle_stage_at_cursor(app: &mut App, force_whole_file: bool) {
    if app.target.staging_mode() == StagingMode::ReadOnly {
        app.set_status_message("read-only diff target");
        return;
    }
    if app.stage_ops().is_none() {
        app.set_status_message("staging unavailable (no git backend)");
        return;
    }
    let Some(file) = app.view.files.get(app.view.file_of_cursor()) else {
        return;
    };
    let path = file.path.clone();
    let staging = app.target.staging_mode() == StagingMode::Stage;
    let verb = if staging { "staged" } else { "unstaged" };

    let synthetic = app
        .patches
        .get(app.view.file_of_cursor())
        .is_none_or(|p| p.is_none());
    let gesture = if synthetic || force_whole_file {
        StageGesture::WholeFile
    } else {
        match app.mode {
            Mode::Visual { anchor } => match visual_stage_selection(&app.view, anchor) {
                Ok((hunk_index, lines)) => StageGesture::Lines(hunk_index, lines),
                Err(message) => {
                    app.set_status_message(message);
                    return;
                }
            },
            _ => match app.view.rows.get(app.view.cursor) {
                Some(Row::Line(line)) => StageGesture::Hunk(line.hunk_index),
                Some(Row::HunkHeader { hunk_index, .. }) => StageGesture::Hunk(*hunk_index),
                Some(Row::FileHeader { .. }) | Some(Row::Binary) => StageGesture::WholeFile,
                _ => return,
            },
        }
    };

    let result = run_stage_gesture(app, &gesture, &path, staging, verb);
    match result {
        Ok(message) => {
            if matches!(app.mode, Mode::Visual { .. }) {
                app.mode = Mode::Normal;
            }
            app.set_status_message(message);
            app.refresh();
        }
        Err(message) => app.set_status_message(message),
    }
}

/// Executes one resolved [`StageGesture`] against the git backend,
/// returning a success echo or a displayable error. Reads the backend and
/// patches off `app` but does not mutate it.
fn run_stage_gesture(
    app: &App,
    gesture: &StageGesture,
    path: &str,
    staging: bool,
    verb: &str,
) -> Result<String, String> {
    let Some(ops) = app.stage_ops() else {
        return Err("staging unavailable (no git backend)".to_string());
    };
    match gesture {
        StageGesture::WholeFile => {
            let result = if staging {
                ops.stage_file(path)
            } else {
                ops.unstage_file(path)
            };
            result
                .map(|_| format!("{verb} {path}"))
                .map_err(|e| e.to_string())
        }
        StageGesture::Hunk(hunk_index) => {
            let Some(Some(raw)) = app.patches.get(app.view.file_of_cursor()) else {
                return Err("no patch available for this file".to_string());
            };
            let patch = build_hunk_patch(raw, *hunk_index).map_err(|e| e.to_string())?;
            let result = if staging {
                ops.apply_cached(&patch)
            } else {
                ops.unapply_cached(&patch)
            };
            result
                .map(|_| format!("{verb} hunk"))
                .map_err(|e| e.to_string())
        }
        StageGesture::Lines(hunk_index, lines) => {
            let Some(Some(raw)) = app.patches.get(app.view.file_of_cursor()) else {
                return Err("no patch available for this file".to_string());
            };
            let patch = build_line_patch(raw, *hunk_index, lines).map_err(|e| e.to_string())?;
            let result = if staging {
                ops.apply_cached(&patch)
            } else {
                ops.unapply_cached(&patch)
            };
            let plural = if lines.len() == 1 { "line" } else { "lines" };
            result
                .map(|_| format!("{verb} {} {plural}", lines.len()))
                .map_err(|e| e.to_string())
        }
    }
}

/// Resolves a Visual selection (`anchor`..cursor, order-independent) into
/// `(hunk_index, body-line indices)` for [`build_line_patch`]: the indices
/// count every body line of the hunk from 0, and only the selected `+`/`-`
/// lines are included (context is always kept by the patch builder anyway).
/// Errors if the selection's line rows span more than one *file* section
/// (`hunk_index` is per-file, so a cross-section span would misattribute
/// hunks), more than one hunk, or contain no changed lines at all.
fn visual_stage_selection(
    view: &DiffViewState,
    anchor: usize,
) -> Result<(usize, Vec<usize>), &'static str> {
    let (lo, hi) = if anchor <= view.cursor {
        (anchor, view.cursor)
    } else {
        (view.cursor, anchor)
    };

    // A visual span may cross section boundaries freely while navigating,
    // but staging one requires a single owning file: `hunk_index` is only
    // meaningful within a file, so a cross-file span could stage the wrong
    // hunk. Reject it before anything else.
    let files_in_span: HashSet<usize> = view.rows[lo..=hi]
        .iter()
        .enumerate()
        .filter(|(_, r)| matches!(r, Row::Line(_)))
        .filter_map(|(offset, _)| view.file_of_row.get(lo + offset).copied())
        .collect();
    if files_in_span.len() > 1 {
        return Err("selection spans multiple files");
    }

    // Body-line indices are per-hunk positions counted over Row::Line rows
    // only (annotation display rows are interleaved in `rows` but are not
    // hunk body lines). Since `hunk_index` is per-file, the count is scoped
    // to the selected file's section so a second file's hunk 0 can't shift a
    // first file's indices.
    let mut body_counters: HashMap<usize, usize> = HashMap::new();
    let mut hunks_in_span: HashSet<usize> = HashSet::new();
    let mut selected_hunk: Option<usize> = None;
    let mut selected_lines: Vec<usize> = Vec::new();

    if let Some((fstart, fend)) = files_in_span.iter().next().map(|&f| view.section_span(f)) {
        for i in fstart..fend {
            if i > hi {
                break;
            }
            let Row::Line(line) = &view.rows[i] else {
                continue;
            };
            let counter = body_counters.entry(line.hunk_index).or_insert(0);
            let body_index = *counter;
            *counter += 1;
            if i < lo {
                continue;
            }
            hunks_in_span.insert(line.hunk_index);
            if line.origin != LineOrigin::Context {
                selected_hunk = Some(line.hunk_index);
                selected_lines.push(body_index);
            }
        }
    }

    if hunks_in_span.len() > 1 {
        return Err("selection spans multiple hunks");
    }
    let Some(hunk_index) = selected_hunk else {
        return Err("no changed lines in selection");
    };
    Ok((hunk_index, selected_lines))
}

/// The staging panel's handlers: opening/closing the panel, moving its focus,
/// and unstaging the focused file. Split out of `app.rs` alongside the `space`
/// gesture above so all staging-panel logic lives in one module.
impl App {
    /// Toggles the staging panel: opens it from Normal/Visual, closes it
    /// from Staging. Opening refreshes its list first, so it's current even
    /// if nothing changed this session — from `git status` in a plain
    /// session, or from `review_states` (the accepted-files panel) during a
    /// review session, via [`App::refresh_accepted_list`].
    /// A no-op while Compose or the annotation list is open. The filter is
    /// transient per-open (spec 12 Non-Goal 5): closing always drops it.
    pub(super) fn toggle_staging_panel(&mut self) {
        match self.mode {
            Mode::Staging => {
                self.mode = Mode::Normal;
                self.staging_filter = None;
            }
            Mode::Compose
            | Mode::List
            | Mode::Panel { .. }
            | Mode::Search
            | Mode::Peek
            | Mode::Switcher
            | Mode::ReviewLauncher { .. }
            | Mode::CommitMessage
            | Mode::Finder
            | Mode::ProjectSearch
            | Mode::EndReview { .. }
            | Mode::ConfirmRemoteOp { .. }
            | Mode::ThreadView
            | Mode::SubmitForge
            | Mode::CleanupReviews { .. } => {}
            Mode::Normal | Mode::Visual { .. } => {
                if self.in_review_session() {
                    self.refresh_accepted_list();
                } else {
                    self.refresh_staged_list();
                }
                self.staging_cursor = self.staging_cursor.min(self.staged.len().saturating_sub(1));
                self.mode = Mode::Staging;
                self.motion_count = None;
            }
        }
    }

    /// Closes the staging panel, returning to [`Mode::Normal`] and dropping
    /// any active filter (transient per-open).
    pub fn close_staging(&mut self) {
        self.mode = Mode::Normal;
        self.staging_filter = None;
    }

    /// Builds the staging/accepted panel's `/`-filterable labels (each
    /// entry's path), in the same order `staged`/`staging_cursor` already
    /// index over.
    fn staging_filter_labels(&self) -> Vec<String> {
        self.staged.iter().map(|e| e.path.clone()).collect()
    }

    /// The staging/accepted panel's effective row count: the active
    /// filter's filtered view when one is set, the full `staged` count
    /// otherwise — every motion clamps against this (spec 12's
    /// filtered-view design constraint).
    fn staging_effective_len(&self) -> usize {
        self.staging_filter
            .as_ref()
            .map_or(self.staged.len(), ListFilter::len)
    }

    /// Translates `staging_cursor` (a filtered position while a filter is
    /// active, a raw index otherwise) into a real `staged` index — the one
    /// point every verb (unstage/un-accept) routes through.
    pub(super) fn staging_real_index(&self) -> Option<usize> {
        match &self.staging_filter {
            Some(f) => f.real_index(self.staging_cursor),
            None => (self.staging_cursor < self.staged.len()).then_some(self.staging_cursor),
        }
    }

    /// Enters filter mode (`/`): a no-op if already active.
    pub(super) fn staging_enter_filter(&mut self) {
        if self.staging_filter.is_none() {
            let labels = self.staging_filter_labels();
            self.staging_filter = Some(ListFilter::open(&labels));
        }
    }

    /// Resumes editing a locked filter (`/` while locked).
    pub(super) fn staging_resume_filter_editing(&mut self) {
        if let Some(f) = self.staging_filter.as_mut() {
            f.resume_editing();
        }
    }

    /// Locks the active filter (`Enter` while editing).
    pub(super) fn staging_lock_filter(&mut self) {
        if let Some(f) = self.staging_filter.as_mut() {
            f.lock();
        }
    }

    /// Clears the active filter entirely (`Esc`).
    pub(super) fn staging_clear_filter(&mut self) {
        self.staging_filter = None;
        self.staging_cursor = self.staging_cursor.min(self.staged.len().saturating_sub(1));
    }

    /// Appends `c` to the active filter's query and re-clamps the cursor.
    pub(super) fn staging_filter_push_char(&mut self, c: char) {
        let labels = self.staging_filter_labels();
        if let Some(f) = self.staging_filter.as_mut() {
            f.push_char(c, &labels);
        }
        self.staging_clamp_cursor_to_filter();
    }

    /// Deletes the last character of the active filter's query.
    pub(super) fn staging_filter_backspace(&mut self) {
        let labels = self.staging_filter_labels();
        if let Some(f) = self.staging_filter.as_mut() {
            f.backspace(&labels);
        }
        self.staging_clamp_cursor_to_filter();
    }

    fn staging_clamp_cursor_to_filter(&mut self) {
        if let Some(f) = self.staging_filter.as_ref() {
            self.staging_cursor = self.staging_cursor.min(f.len().saturating_sub(1));
        }
    }

    /// Re-ranks an active filter against the current `staged` list (a no-op
    /// without one) and re-clamps the cursor — called after unstage/accept
    /// shrinks the list, so a filtered session survives the mutation
    /// instead of going stale.
    pub(super) fn staging_refresh_filter(&mut self) {
        if let Some(f) = self.staging_filter.as_mut() {
            let labels: Vec<String> = self.staged.iter().map(|e| e.path.clone()).collect();
            f.refresh(&labels);
        }
        let len = self.staging_effective_len();
        self.staging_cursor = if len == 0 {
            0
        } else {
            self.staging_cursor.min(len - 1)
        };
    }

    /// Moves the staging panel's focus down one row, clamped at the last.
    pub fn staging_move_down(&mut self) {
        let len = self.staging_effective_len();
        if len > 0 {
            self.staging_cursor = (self.staging_cursor + 1).min(len - 1);
        }
    }

    /// Moves the staging panel's focus up one row, clamped at the first.
    pub fn staging_move_up(&mut self) {
        self.staging_cursor = self.staging_cursor.saturating_sub(1);
    }

    /// The staging/accepted panel's page-size proxy for half/full-page
    /// motions (see `git_panel::App::panel_viewport_proxy`'s identical
    /// rationale — neither panel tracks its own render height).
    fn staging_viewport_proxy(&self) -> usize {
        self.view.viewport_height()
    }

    /// Moves the staging/accepted panel's focus down half a viewport
    /// (`Ctrl-d`; shared motion set, see `super::motion`). Shared by both
    /// panels, like `staging_move_down`/`up`.
    pub fn staging_half_page_down(&mut self) {
        let step = super::motion::half_page(self.staging_viewport_proxy());
        self.staging_cursor = super::motion::step(
            self.staging_cursor,
            self.staging_effective_len(),
            step,
            true,
        );
    }

    /// Moves the staging/accepted panel's focus up half a viewport
    /// (`Ctrl-u`).
    pub fn staging_half_page_up(&mut self) {
        let step = super::motion::half_page(self.staging_viewport_proxy());
        self.staging_cursor = super::motion::step(
            self.staging_cursor,
            self.staging_effective_len(),
            step,
            false,
        );
    }

    /// Moves the staging/accepted panel's focus down a full viewport
    /// (`Ctrl-f`).
    pub fn staging_full_page_down(&mut self) {
        let step = super::motion::full_page(self.staging_viewport_proxy());
        self.staging_cursor = super::motion::step(
            self.staging_cursor,
            self.staging_effective_len(),
            step,
            true,
        );
    }

    /// Moves the staging/accepted panel's focus up a full viewport
    /// (`Ctrl-b`).
    pub fn staging_full_page_up(&mut self) {
        let step = super::motion::full_page(self.staging_viewport_proxy());
        self.staging_cursor = super::motion::step(
            self.staging_cursor,
            self.staging_effective_len(),
            step,
            false,
        );
    }

    /// Jumps the staging/accepted panel's focus to the first entry
    /// (`g`/`Home`).
    pub fn staging_jump_to_top(&mut self) {
        self.staging_cursor = super::motion::jump_top();
    }

    /// Jumps the staging/accepted panel's focus to the last entry
    /// (`G`/`End`).
    pub fn staging_jump_to_bottom(&mut self) {
        self.staging_cursor = super::motion::jump_bottom(self.staging_effective_len());
    }

    /// Unstages the staging panel's focused file (the filtered selection,
    /// while a filter is active), then refreshes. The panel stays open and
    /// its cursor is clamped to the shrunken list. A no-op on an empty
    /// list; failures set a footer message and change nothing.
    pub fn unstage_focused_file(&mut self) {
        let Some(index) = self.staging_real_index() else {
            return;
        };
        let Some(entry) = self.staged.get(index) else {
            return;
        };
        let path = entry.path.clone();
        let result = {
            let Some(ops) = self.stage_ops() else {
                self.set_status_message("staging unavailable (no git backend)");
                return;
            };
            ops.unstage_file(&path)
        };
        match result {
            Ok(()) => {
                self.set_status_message(format!("unstaged {path}"));
                self.refresh();
                self.staging_refresh_filter();
            }
            Err(e) => self.set_status_message(e.to_string()),
        }
    }

    /// Best-effort re-read of the staged-file list (and per-path staged
    /// states) from `git status`, keeping the previous values on any failure.
    fn refresh_staged_list(&mut self) {
        let (staged, states) = {
            let Some(ops) = self.stage_ops() else {
                return;
            };
            match ops.status() {
                Ok(status) => (
                    staged_from_status(&status),
                    staged_states_from_status(&status),
                ),
                Err(_) => return,
            }
        };
        self.staged = staged;
        self.staged_states = states;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotate::AnnotationStore;
    use crate::diff::FileDiff;
    use crate::git::RawFilePatch;
    use crate::ui::rows::{ReviewMarker, StagedMarker, SyntaxSpans, build_multibuffer};
    use crate::ui::stage_ops::StageOps;

    fn file_from_raw(path: &str, raw: &str) -> FileDiff {
        FileDiff::from_patch(&RawFilePatch {
            path: path.to_string(),
            old_path: None,
            raw: raw.to_string(),
            is_binary: false,
        })
        .unwrap()
    }

    /// A `DiffViewState` over `files` with its multibuffer populated the way
    /// `App` would after a rebuild (unhighlighted, all expanded).
    fn multi_view(files: Vec<FileDiff>) -> DiffViewState {
        let mut view = DiffViewState::new(files);
        let n = view.files.len();
        let mb = build_multibuffer(
            &view.files,
            &vec![false; n],
            &vec![StagedMarker::None; n],
            &vec![ReviewMarker::None; n],
            &AnnotationStore::new(),
            &vec![SyntaxSpans::default(); n],
        );
        view.rows = mb.rows;
        view.file_of_row = mb.file_of_row;
        view.header_row_of_file = mb.header_row_of_file;
        view.gutter_width = mb.gutter_width;
        view
    }

    /// A `DiffViewState` over one file, its multibuffer populated.
    fn view_with_raw(raw: &str) -> DiffViewState {
        multi_view(vec![file_from_raw("f.rs", raw)])
    }

    #[test]
    fn visual_stage_selection_collects_changed_body_lines() {
        // rows: FileHeader(0) HunkHeader(1) -old0(2) +new0(3)
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,1 +1,1 @@
-old0
+new0
";
        let mut view = view_with_raw(raw);
        view.cursor = 3; // +new0
        let (hunk_index, lines) = visual_stage_selection(&view, 3).unwrap();
        assert_eq!(hunk_index, 0);
        assert_eq!(lines, vec![1]); // second body line (the addition)
    }

    #[test]
    fn visual_stage_selection_rejects_multi_hunk_span() {
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
        let mut view = view_with_raw(raw);
        // rows: FH(0) HH0(1) -old0(2) +new0(3) HH1(4) -old1(5) +new1(6)
        view.cursor = 5;
        let err = visual_stage_selection(&view, 2).unwrap_err();
        assert_eq!(err, "selection spans multiple hunks");
    }

    #[test]
    fn visual_stage_selection_rejects_cross_section_span() {
        // Two single-hunk files: a.rs occupies rows 0..4, b.rs rows 4..8.
        // A span from a.rs's addition (row 3) into b.rs's removal (row 6)
        // crosses the section boundary and must be rejected before the
        // per-file hunk index is trusted.
        let one = "\
diff --git a/x b/x
index 1..2 100644
--- a/x
+++ b/x
@@ -1,1 +1,1 @@
-old
+new
";
        let mut view = multi_view(vec![file_from_raw("a.rs", one), file_from_raw("b.rs", one)]);
        // rows: FH_a(0) HH_a(1) -old(2) +new(3) FH_b(4) HH_b(5) -old(6) +new(7)
        view.cursor = 6;
        let err = visual_stage_selection(&view, 3).unwrap_err();
        assert_eq!(err, "selection spans multiple files");
    }

    #[test]
    fn visual_stage_selection_scopes_body_index_to_second_file_section() {
        // A selection wholly inside the second file's hunk must yield that
        // file's own body index (0-based within its hunk), not one offset by
        // the first file's identically-indexed hunk 0.
        let one = "\
diff --git a/x b/x
index 1..2 100644
--- a/x
+++ b/x
@@ -1,1 +1,1 @@
-old
+new
";
        let mut view = multi_view(vec![file_from_raw("a.rs", one), file_from_raw("b.rs", one)]);
        // b.rs: FH_b(4) HH_b(5) -old(6) +new(7). Select the addition (row 7).
        view.cursor = 7;
        let (hunk_index, lines) = visual_stage_selection(&view, 7).unwrap();
        assert_eq!(hunk_index, 0);
        assert_eq!(lines, vec![1]); // second body line of b.rs's hunk, not 3
    }

    #[test]
    fn visual_stage_selection_rejects_context_only_span() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,2 +1,3 @@
 ctx1
+added
 ctx2
";
        let mut view = view_with_raw(raw);
        view.cursor = 2; // ctx1
        let err = visual_stage_selection(&view, 2).unwrap_err();
        assert_eq!(err, "no changed lines in selection");
    }

    // -- Filter + motion + verb composition (spec 12 FR-8) -------------------

    /// A no-op `StageOps` fake recording every `unstage_file` call (via a
    /// shared `Rc<RefCell<_>>` so the test can inspect it after the fake is
    /// boxed into `App::stage_ops`) — enough to prove *which* path a
    /// filtered `Space` targeted, without needing a real git backend.
    struct RecordingOps {
        unstaged: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
    }

    impl StageOps for RecordingOps {
        fn diff(
            &self,
            _target: &crate::git::DiffTarget,
        ) -> Result<Vec<RawFilePatch>, crate::git::GitError> {
            Ok(Vec::new())
        }
        fn status(&self) -> Result<Vec<crate::git::FileStatus>, crate::git::GitError> {
            Ok(Vec::new())
        }
        fn stage_file(&self, _path: &str) -> Result<(), crate::git::GitError> {
            Ok(())
        }
        fn unstage_file(&self, path: &str) -> Result<(), crate::git::GitError> {
            self.unstaged.borrow_mut().push(path.to_string());
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
    }

    /// Three staged files, two of which ("apple.rs"/"apricot.rs") share an
    /// `ap` prefix a `/ap` query narrows to, leaving the third ("banana.rs")
    /// out — so the filtered view is genuinely narrower than the raw list,
    /// and stepping within it has two real rows to move between.
    fn app_with_three_staged_files(log: std::rc::Rc<std::cell::RefCell<Vec<String>>>) -> App {
        let mut app = App::new(vec![file_from_raw(
            "z.rs",
            "diff --git a/z.rs b/z.rs\nindex 1..2 100644\n--- a/z.rs\n+++ b/z.rs\n@@ -1,1 +1,1 @@\n-old\n+new\n",
        )]);
        app.staged = vec![
            crate::ui::StagedFile {
                path: "src/apple.rs".to_string(),
                letter: 'M',
            },
            crate::ui::StagedFile {
                path: "src/apricot.rs".to_string(),
                letter: 'M',
            },
            crate::ui::StagedFile {
                path: "src/banana.rs".to_string(),
                letter: 'M',
            },
        ];
        app.stage_ops = Some(Box::new(RecordingOps { unstaged: log }));
        app.mode = Mode::Staging;
        app
    }

    #[test]
    fn filter_narrows_j_then_space_unstages_the_correct_filtered_entry() {
        use crate::ui::modes::handle_staging_key;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut app = app_with_three_staged_files(log.clone());
        let key = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);

        handle_staging_key(&mut app, key('/'));
        for c in "ap".chars() {
            handle_staging_key(&mut app, key(c));
        }
        assert_eq!(
            app.staging_filter.as_ref().unwrap().len(),
            2,
            "banana.rs must be excluded"
        );
        handle_staging_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!app.staging_filter.as_ref().unwrap().is_editing());

        let first = app.staging_real_index().unwrap();
        handle_staging_key(&mut app, key('j'));
        let second = app.staging_real_index().unwrap();
        assert_ne!(first, second, "`j` must move within the filtered view");
        let target_path = app.staged[second].path.clone();

        handle_staging_key(&mut app, key(' '));
        assert_eq!(
            *log.borrow(),
            vec![target_path],
            "Space must unstage the filtered (not raw-list) selection"
        );
    }
}
