//! [`App`]: the TUI's state and the pure state transitions every [`Action`]
//! performs. No rendering or terminal I/O lives here — these are plain
//! methods, unit-tested without a terminal.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::annotate::{AnnotationStore, Side, Target};
use crate::diff::{FileDiff, LineOrigin};
use crate::git::{DiffTarget, RawFilePatch, build_hunk_patch, build_line_patch};
use crate::highlight::{Highlighter, Lang};
use crate::lsp::{LspEvent, LspManager, RequestId, SourceLocation};

use super::compose::ComposeState;
use super::diff_view_state::DiffViewState;
use super::keymap::Action;
use super::lsp_ops::LspClient;
use super::peek::{CachedPreview, PeekKind, PeekState};
use super::rows::{
    LineRow, Row, SyntaxSpans, anchor_row_index, build_rows, build_sbs_rows, hunk_span,
};
use super::search::SearchState;
use super::stage_ops::{ReviewSnapshot, StageOps, StagedFile, build_review, staged_from_status};
use super::syntax::{self, HighlightCache};
use super::theme::Theme;

/// The interaction mode. Normal/Visual bindings dispatch through the
/// [`super::keymap::Keymap`] table; Compose, List, and Staging handle their
/// keys modally (see [`super::handle_compose_key`]/[`super::handle_list_key`]/
/// [`super::handle_staging_key`]), bypassing the table entirely so every
/// keystroke can be text/navigation rather than a bound action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Ordinary review/navigation.
    Normal,
    /// A line-range selection in progress. `anchor` is the row index where
    /// `v` was pressed; the cursor is the selection's other end.
    Visual { anchor: usize },
    /// The comment modal is open, composing or editing an annotation.
    Compose,
    /// The annotation list panel is open and focused.
    List,
    /// The staging panel is open and focused.
    Staging,
    /// The search input is open in the footer, composing a pattern.
    Search,
    /// The LSP peek overlay (`gd`/`gr`/`K` results) is open.
    Peek,
}

/// The TUI's full state: the per-view diff state (files, selection, rows,
/// cursor, scroll, layout — see [`DiffViewState`]), help overlay
/// visibility, and the annotation store the session accumulates into
/// (emitted to stdout on quit), plus the modal states and service glue.
pub struct App {
    /// The per-view diff state: the diffed files, which one is selected, the
    /// flattened row model for that file, cursor and scroll positions, the
    /// viewport height, and the layout choice. `App` delegates every
    /// navigation gesture here and feeds rebuilt rows back in.
    pub view: DiffViewState,
    /// Whether the help overlay is open.
    pub help_open: bool,
    /// Annotations accumulated this session.
    pub annotations: AnnotationStore,
    /// The current interaction mode.
    pub mode: Mode,
    /// The Compose modal's state, when `mode == Mode::Compose`.
    pub compose: Option<ComposeState>,
    /// The focused row index into `annotations` (insertion order) in the
    /// annotation list panel.
    pub list_cursor: usize,
    /// The raw patch each entry of `files` was parsed from, index-aligned.
    /// `None` for synthetic untracked entries (no real patch exists, so
    /// hunk/line staging falls back to whole-file).
    pub patches: Vec<Option<RawFilePatch>>,
    /// The diff target being reviewed; decides whether `space` stages
    /// (working tree), unstages (staged), or is read-only (range).
    pub target: DiffTarget,
    /// Files with staged changes, per the latest `git status` refresh.
    pub staged: Vec<StagedFile>,
    /// The focused row index into `staged` in the staging panel.
    pub staging_cursor: usize,
    /// A transient one-line message for the status footer (errors, no-op
    /// explanations, success echoes). Cleared on the next keypress.
    pub status_message: Option<String>,
    /// The git backend staging and refresh run through. `None` in
    /// git-less contexts (e.g. pure-navigation unit tests), where staging
    /// degrades to a footer message.
    stage_ops: Option<Box<dyn StageOps>>,
    /// The color palette every renderer routes through.
    pub theme: Theme,
    /// The tree-sitter highlighting engine. Owned here so its per-language
    /// config cache persists across selections.
    highlighter: Highlighter,
    /// Highlighted line spans, cached per `(path, side)` and cleared on
    /// every [`App::refresh`] (see [`syntax::HighlightCache`]).
    highlight_cache: HighlightCache,
    /// The active (or inactive) search session: confirmed pattern plus its
    /// match row indices against the current file's rows.
    pub search: SearchState,
    /// The in-progress pattern buffer while [`Mode::Search`] is active.
    pub search_input: String,
    /// The repo root LSP servers are spawned against (from the
    /// [`crate::git::GitRunner`]). `None` in git-less contexts, where
    /// `gd`/`gr`/`K` degrade to a footer message like everything else
    /// without a git backend.
    pub repo_root: Option<PathBuf>,
    /// The active or most recent [`Mode::Peek`] overlay's state. `None`
    /// when the overlay has never been opened, or after it's closed.
    pub peek: Option<PeekState>,
    /// The LSP client backing `gd`/`gr`/`K`, created lazily on first use
    /// against `repo_root`. `None` until then.
    lsp: Option<Box<dyn LspClient>>,
    /// The request id + kind `gd`/`gr`/`K` is currently awaiting a
    /// response for. A new request overwrites this (cancelling interest in
    /// whatever was pending before); an [`LspEvent`] whose id doesn't match
    /// is ignored.
    pending_lsp: Option<(RequestId, PeekKind)>,
}

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

impl App {
    /// Builds a fresh `App` over `files`, with the first file selected. No
    /// git backend is attached: staging gestures degrade to a footer
    /// message. Interactive sessions should use [`App::with_git`].
    pub fn new(files: Vec<FileDiff>) -> App {
        let annotations = AnnotationStore::new();
        let patches = files.iter().map(|_| None).collect();
        let mut app = App {
            view: DiffViewState::new(files),
            help_open: false,
            annotations,
            mode: Mode::Normal,
            compose: None,
            list_cursor: 0,
            patches,
            target: DiffTarget::WorkingTree,
            staged: Vec::new(),
            staging_cursor: 0,
            status_message: None,
            stage_ops: None,
            theme: Theme::default(),
            highlighter: Highlighter::new(),
            highlight_cache: HighlightCache::default(),
            search: SearchState::default(),
            search_input: String::new(),
            repo_root: None,
            peek: None,
            lsp: None,
            pending_lsp: None,
        };
        app.rebuild_rows();
        app
    }

    /// Builds an `App` over a [`ReviewSnapshot`] with a git backend
    /// attached, enabling staging and post-stage refresh.
    pub fn with_git(snapshot: ReviewSnapshot, target: DiffTarget, ops: Box<dyn StageOps>) -> App {
        let mut app = App::new(snapshot.files);
        app.patches = snapshot.patches;
        app.staged = snapshot.staged;
        app.target = target;
        app.stage_ops = Some(ops);
        app.highlight_cache.clear();
        app.rebuild_rows();
        app
    }

    /// Sets the workspace root `gd`/`gr`/`K` spawn LSP servers against
    /// (the GitRunner's repo root). Without this, code-intelligence
    /// requests degrade to a footer message.
    pub fn set_repo_root(&mut self, root: PathBuf) {
        self.repo_root = Some(root);
    }

    /// Takes the LSP client, if one was ever created, so the caller can
    /// shut it down after restoring the terminal. Leaves `None` in its
    /// place; a subsequent `gd`/`gr`/`K` would lazily create a fresh one.
    pub fn take_lsp_client(&mut self) -> Option<Box<dyn LspClient>> {
        self.lsp.take()
    }

    /// Test-only injection point for a fake [`LspClient`], bypassing lazy
    /// creation of the real [`LspManager`]. Also sets `repo_root` so
    /// `gd`/`gr`/`K` don't short-circuit on a missing root.
    #[cfg(test)]
    fn inject_lsp_client(&mut self, client: Box<dyn LspClient>, root: PathBuf) {
        self.lsp = Some(client);
        self.repo_root = Some(root);
    }

    /// Applies one [`Action`] as a state transition.
    ///
    /// `Quit` and `QuitDiscard` are no-ops here — the event loop intercepts
    /// them before they reach `apply` and ends the session instead. In
    /// [`Mode::Visual`], every action other than the ones
    /// [`visual_mode_allows`] passes through is a no-op (`]`/`[`/Tab/etc.
    /// stay disabled while selecting a range).
    pub fn apply(&mut self, action: Action) {
        if matches!(self.mode, Mode::Visual { .. }) && !visual_mode_allows(action) {
            return;
        }
        match action {
            Action::CursorDown => self.view.cursor_down(),
            Action::CursorUp => self.view.cursor_up(),
            Action::HalfPageDown => self.view.half_page_down(),
            Action::HalfPageUp => self.view.half_page_up(),
            Action::CursorLeft => self.view.move_column_left(),
            Action::CursorRight => self.view.move_column_right(),
            Action::WordForward => self.view.move_word_forward(),
            Action::WordBackward => self.view.move_word_backward(),
            Action::NextHunk => self.next_hunk(),
            Action::PrevHunk => self.prev_hunk(),
            Action::NextFile => self.switch_file(self.view.selected_file + 1),
            Action::PrevFile => {
                if let Some(prev) = self.view.selected_file.checked_sub(1) {
                    self.switch_file(prev);
                }
            }
            Action::ToggleHelp => self.help_open = !self.help_open,
            Action::ToggleView => self.view.toggle_view(),
            Action::EnterVisual => self.toggle_visual(),
            Action::Compose => self.open_compose(),
            Action::ToggleList => self.toggle_list(),
            Action::ToggleStage => self.toggle_stage(),
            Action::ToggleStagingPanel => self.toggle_staging_panel(),
            Action::Search => self.enter_search(),
            Action::SearchNext => self.search_advance(true),
            Action::SearchPrev => self.search_advance(false),
            Action::GotoDefinition => self.request_code_intel(PeekKind::Definition),
            Action::GotoReferences => self.request_code_intel(PeekKind::References),
            Action::Hover => self.request_code_intel(PeekKind::Hover),
            Action::Quit | Action::QuitDiscard => {}
        }
    }

    /// Switches to file `index`, resetting cursor and scroll to the top.
    /// Out-of-range indices are a no-op (this is how `NextFile`/`PrevFile`
    /// clamp at the first/last file rather than wrapping). Rebuilding rows
    /// (with highlighting) stays here; the view just holds the result.
    fn switch_file(&mut self, index: usize) {
        if index >= self.view.files.len() {
            return;
        }
        self.view.selected_file = index;
        self.rebuild_rows();
        self.view.cursor = 0;
        self.view.scroll = 0;
        self.view.sbs_scroll = 0;
    }

    /// Rebuilds `rows` for the currently selected file against the current
    /// `annotations`, then re-clamps the cursor. Called after any mutation
    /// to the annotation store so inline display/gutter markers stay in
    /// sync.
    fn refresh_rows(&mut self) {
        if self.view.files.get(self.view.selected_file).is_some() {
            self.rebuild_rows();
            self.view.cursor = self
                .view
                .nearest_addressable(self.view.cursor.min(self.view.max_cursor()), true);
            self.view.ensure_visible();
        }
    }

    /// Rebuilds the view's `rows` for the currently selected file: lazily
    /// populates the syntax-highlight cache for whichever side(s) this
    /// file's hunks actually use (a no-op on a cache hit — highlighting
    /// happens at most once per `(path, side)` between refreshes), then
    /// rebuilds the row model against the current annotations and those
    /// spans. Also recomputes the active search's match positions, since
    /// they're relative to `rows`. Sets `rows` to empty if `selected_file`
    /// is out of range (e.g. an empty diff). This is `App`'s side of the
    /// seam: highlighting and the git backend live here, and the freshly
    /// built rows are fed into [`DiffViewState`].
    fn rebuild_rows(&mut self) {
        let Some(file) = self.view.files.get(self.view.selected_file) else {
            self.view.rows = Vec::new();
            self.view.sbs_rows = Vec::new();
            self.view.sbs_visual_of = Vec::new();
            self.search.recompute(&self.view.rows);
            return;
        };
        let path = file.path.clone();
        let old_path = file.old_path.clone();
        let needs_new = syntax::side_in_use(file, Side::New);
        let needs_old = syntax::side_in_use(file, Side::Old);
        let synthetic = self
            .patches
            .get(self.view.selected_file)
            .is_none_or(|p| p.is_none());

        if needs_new {
            syntax::populate_cache(
                &mut self.highlight_cache,
                &mut self.highlighter,
                self.stage_ops.as_deref(),
                &self.target,
                &path,
                old_path.as_deref(),
                Side::New,
                synthetic,
            );
        }
        if needs_old {
            syntax::populate_cache(
                &mut self.highlight_cache,
                &mut self.highlighter,
                self.stage_ops.as_deref(),
                &self.target,
                &path,
                old_path.as_deref(),
                Side::Old,
                synthetic,
            );
        }

        let new_spans = self.highlight_cache.get(&path, Side::New);
        let old_spans = self.highlight_cache.get(&path, Side::Old);
        let file = &self.view.files[self.view.selected_file];
        let rows = build_rows(
            file,
            &self.annotations,
            SyntaxSpans {
                new: new_spans,
                old: old_spans,
            },
        );
        let (sbs_rows, sbs_visual_of) = build_sbs_rows(file, &rows);
        self.view.rows = rows;
        self.view.sbs_rows = sbs_rows;
        self.view.sbs_visual_of = sbs_visual_of;
        self.search.recompute(&self.view.rows);
    }

    /// Jumps the cursor to the next hunk header after the cursor, crossing
    /// into the next file (at its first hunk) if the current file has none
    /// left. A no-op if there is no next hunk anywhere. The in-file jump and
    /// the file probe live on [`DiffViewState`]; `App` orchestrates the
    /// highlighting rebuild between selecting a file and positioning on it.
    fn next_hunk(&mut self) {
        if self.view.next_hunk_in_file() {
            return;
        }
        for index in (self.view.selected_file + 1)..self.view.files.len() {
            if let Some(first) = self.view.probe_first_hunk_row(&self.annotations, index) {
                self.view.selected_file = index;
                self.rebuild_rows();
                self.view.cursor = first;
                self.view.scroll = 0;
                self.view.sbs_scroll = 0;
                self.view.ensure_visible();
                return;
            }
        }
    }

    /// Jumps the cursor to the previous hunk header before the cursor,
    /// crossing into the previous file (at its last hunk) if the current
    /// file has none before the cursor. A no-op if there is no previous
    /// hunk anywhere.
    fn prev_hunk(&mut self) {
        if self.view.prev_hunk_in_file() {
            return;
        }
        for index in (0..self.view.selected_file).rev() {
            if let Some(last) = self.view.probe_last_hunk_row(&self.annotations, index) {
                self.view.selected_file = index;
                self.rebuild_rows();
                self.view.cursor = last;
                self.view.scroll = 0;
                self.view.sbs_scroll = 0;
                self.view.ensure_visible();
                return;
            }
        }
    }

    // -- Visual mode -------------------------------------------------

    fn toggle_visual(&mut self) {
        match self.mode {
            Mode::Normal => {
                if matches!(self.view.rows.get(self.view.cursor), Some(Row::Line(_))) {
                    self.mode = Mode::Visual {
                        anchor: self.view.cursor,
                    };
                }
            }
            Mode::Visual { .. } => self.mode = Mode::Normal,
            _ => {}
        }
    }

    // -- Target derivation ---------------------------------------------

    /// The annotation target for the cursor's current row in [`Mode::Normal`]:
    /// a `Line` target for a diff line (side/number from the line's
    /// origin), a `Hunk` target for a hunk header, or a `File` target for
    /// the file header/binary placeholder. `None` on rows that carry no
    /// derivable target (currently only [`Row::Annotation`], which the
    /// cursor never addresses).
    pub fn target_for_cursor(&self) -> Option<Target> {
        let file = self.view.files.get(self.view.selected_file)?;
        match self.view.rows.get(self.view.cursor)? {
            Row::Line(line) => line_target(&file.path, line),
            Row::HunkHeader { hunk_index, .. } => self.hunk_target(*hunk_index),
            Row::FileHeader { .. } | Row::Binary => Some(Target::file(&file.path)),
            Row::Annotation { .. } => None,
        }
    }

    fn hunk_target(&self, hunk_index: usize) -> Option<Target> {
        let file = self.view.files.get(self.view.selected_file)?;
        let hunk = file.hunks.get(hunk_index)?;
        let (start, end) = hunk_span(hunk);
        Target::hunk(&file.path, start, end).ok()
    }

    /// The annotation target for a [`Mode::Visual`] selection between
    /// `anchor` and the cursor (inclusive, order-independent). Only
    /// `Row::Line` rows in the span count; selections spanning hunk/file
    /// headers clamp to the line rows within them. If every selected line
    /// is `Removed`, the target uses the old side and old-side line
    /// numbers; otherwise it uses the new side and the new-side line
    /// numbers of the non-removed rows the selection spans. `None` if the
    /// selection covers no line rows at all.
    pub fn target_for_visual(&self, anchor: usize) -> Option<Target> {
        let file = self.view.files.get(self.view.selected_file)?;
        let (lo, hi) = if anchor <= self.view.cursor {
            (anchor, self.view.cursor)
        } else {
            (self.view.cursor, anchor)
        };
        let lines: Vec<&LineRow> = self.view.rows[lo..=hi]
            .iter()
            .filter_map(|r| match r {
                Row::Line(l) => Some(l),
                _ => None,
            })
            .collect();
        if lines.is_empty() {
            return None;
        }

        if lines.iter().all(|l| l.origin == LineOrigin::Removed) {
            let nums: Vec<u32> = lines.iter().filter_map(|l| l.old_line).collect();
            let start = *nums.iter().min()?;
            let end = *nums.iter().max()?;
            Target::range(&file.path, start, end, Side::Old).ok()
        } else {
            let nums: Vec<u32> = lines
                .iter()
                .filter(|l| l.origin != LineOrigin::Removed)
                .filter_map(|l| l.new_line)
                .collect();
            let start = *nums.iter().min()?;
            let end = *nums.iter().max()?;
            Target::range(&file.path, start, end, Side::New).ok()
        }
    }

    // -- Compose ---------------------------------------------------------

    /// Opens the Compose modal for the current cursor row (Normal) or the
    /// current selection (Visual). A no-op (stays in the current mode) if
    /// no target can be derived (e.g. `c` on an empty diff, or a Visual
    /// selection with no line rows).
    fn open_compose(&mut self) {
        let target = match self.mode {
            Mode::Visual { anchor } => self.target_for_visual(anchor),
            _ => self.target_for_cursor(),
        };
        if let Some(target) = target {
            self.compose = Some(ComposeState::new(target));
            self.mode = Mode::Compose;
        }
    }

    /// Opens the Compose modal pre-filled with the given existing
    /// annotation, so submitting edits it in place instead of adding a new
    /// one.
    fn open_compose_for(&mut self, id: usize) {
        let Some(annotation) = self.annotations.iter().find(|a| a.id == id) else {
            return;
        };
        self.compose = Some(ComposeState::editing(
            annotation.id,
            annotation.target.clone(),
            annotation.classification,
            &annotation.body,
        ));
        self.mode = Mode::Compose;
    }

    /// Cancels Compose without saving, discarding the draft.
    pub fn cancel_compose(&mut self) {
        self.compose = None;
        self.mode = Mode::Normal;
    }

    /// Submits the Compose draft: adds a new annotation, or (when editing)
    /// updates the existing one's body and classification. An empty or
    /// whitespace-only body cancels instead — the store rejects empty
    /// bodies, and surfacing that as a hard error over "just cancel" would
    /// be needless friction for a body the reviewer clearly abandoned.
    pub fn submit_compose(&mut self) {
        let Some(compose) = self.compose.take() else {
            self.mode = Mode::Normal;
            return;
        };
        let body = compose.buffer.text();
        if body.trim().is_empty() {
            self.mode = Mode::Normal;
            return;
        }

        match compose.editing_id {
            Some(id) => {
                let _ = self.annotations.edit(id, &body);
                let _ = self
                    .annotations
                    .set_classification(id, compose.classification);
            }
            None => {
                let _ = self
                    .annotations
                    .add(compose.target, compose.classification, &body);
            }
        }
        self.mode = Mode::Normal;
        self.refresh_rows();
    }

    // -- Annotation list panel -------------------------------------------

    fn toggle_list(&mut self) {
        match self.mode {
            Mode::List => self.mode = Mode::Normal,
            Mode::Compose | Mode::Staging | Mode::Search | Mode::Peek => {}
            Mode::Normal | Mode::Visual { .. } => {
                if !self.annotations.is_empty() {
                    self.list_cursor = self.list_cursor.min(self.annotations.len() - 1);
                }
                self.mode = Mode::List;
            }
        }
    }

    /// Closes the annotation list panel, returning to [`Mode::Normal`].
    pub fn close_list(&mut self) {
        self.mode = Mode::Normal;
    }

    /// Moves the list panel's focus down one annotation, clamped at the
    /// last.
    pub fn list_move_down(&mut self) {
        if !self.annotations.is_empty() {
            self.list_cursor = (self.list_cursor + 1).min(self.annotations.len() - 1);
        }
    }

    /// Moves the list panel's focus up one annotation, clamped at the
    /// first.
    pub fn list_move_up(&mut self) {
        self.list_cursor = self.list_cursor.saturating_sub(1);
    }

    /// Switches to the focused annotation's file, places the cursor on its
    /// anchor row, and closes the list panel. A no-op if the store is
    /// empty or the annotation's file/anchor can no longer be found.
    pub fn jump_to_focused_annotation(&mut self) {
        let Some(id) = self.annotations.iter().nth(self.list_cursor).map(|a| a.id) else {
            self.mode = Mode::Normal;
            return;
        };
        self.jump_to_annotation(id);
    }

    fn jump_to_annotation(&mut self, id: usize) {
        let Some(annotation) = self.annotations.iter().find(|a| a.id == id) else {
            self.mode = Mode::Normal;
            return;
        };
        let target = annotation.target.clone();
        let path = target.path().to_string();
        if let Some(index) = self.view.files.iter().position(|f| f.path == path) {
            self.view.selected_file = index;
            self.rebuild_rows();
            self.view.cursor =
                anchor_row_index(&self.view.files[index], &self.view.rows, &target).unwrap_or(0);
            self.view.scroll = 0;
            self.view.sbs_scroll = 0;
            self.view.ensure_visible();
        }
        self.mode = Mode::Normal;
    }

    /// Opens Compose pre-filled with the focused annotation for editing.
    pub fn edit_focused_annotation(&mut self) {
        let Some(id) = self.annotations.iter().nth(self.list_cursor).map(|a| a.id) else {
            return;
        };
        self.open_compose_for(id);
    }

    /// Deletes the focused annotation. No confirmation — deletion is cheap
    /// to redo.
    pub fn delete_focused_annotation(&mut self) {
        let Some(id) = self.annotations.iter().nth(self.list_cursor).map(|a| a.id) else {
            return;
        };
        let _ = self.annotations.remove(id);
        if self.annotations.is_empty() {
            self.list_cursor = 0;
        } else {
            self.list_cursor = self.list_cursor.min(self.annotations.len() - 1);
        }
        self.refresh_rows();
    }

    // -- Staging -----------------------------------------------------------

    /// Sets the transient status-footer message (cleared by the event loop
    /// on the next keypress).
    pub fn set_status_message(&mut self, message: impl Into<String>) {
        self.status_message = Some(message.into());
    }

    /// Clears the transient status-footer message.
    pub fn clear_status_message(&mut self) {
        self.status_message = None;
    }

    /// Applies the `space` staging gesture. Direction depends on the diff
    /// target: working tree stages, staged unstages, range is read-only.
    /// Granularity depends on the cursor row (Normal: hunk on line/hunk
    /// rows, whole file on file-header/binary rows) or the Visual selection
    /// (the selected `+`/`-` lines of a single hunk). Synthetic untracked
    /// files always stage whole-file — there is no index blob to apply
    /// hunk/line patches against. Failures and no-op cases set a footer
    /// message and leave state unchanged.
    fn toggle_stage(&mut self) {
        if !matches!(self.mode, Mode::Normal | Mode::Visual { .. }) {
            return;
        }
        if matches!(self.target, DiffTarget::Range(_)) {
            self.set_status_message("read-only diff target");
            return;
        }
        if self.stage_ops.is_none() {
            self.set_status_message("staging unavailable (no git backend)");
            return;
        }
        let Some(file) = self.view.files.get(self.view.selected_file) else {
            return;
        };
        let path = file.path.clone();
        let staging = matches!(self.target, DiffTarget::WorkingTree);
        let verb = if staging { "staged" } else { "unstaged" };

        let synthetic = self
            .patches
            .get(self.view.selected_file)
            .is_none_or(|p| p.is_none());
        let gesture = if synthetic {
            StageGesture::WholeFile
        } else {
            match self.mode {
                Mode::Visual { anchor } => match self.visual_stage_selection(anchor) {
                    Ok((hunk_index, lines)) => StageGesture::Lines(hunk_index, lines),
                    Err(message) => {
                        self.set_status_message(message);
                        return;
                    }
                },
                _ => match self.view.rows.get(self.view.cursor) {
                    Some(Row::Line(line)) => StageGesture::Hunk(line.hunk_index),
                    Some(Row::HunkHeader { hunk_index, .. }) => StageGesture::Hunk(*hunk_index),
                    Some(Row::FileHeader { .. }) | Some(Row::Binary) => StageGesture::WholeFile,
                    _ => return,
                },
            }
        };

        let result = self.run_stage_gesture(&gesture, &path, staging, verb);
        match result {
            Ok(message) => {
                if matches!(self.mode, Mode::Visual { .. }) {
                    self.mode = Mode::Normal;
                }
                self.set_status_message(message);
                self.refresh();
            }
            Err(message) => self.set_status_message(message),
        }
    }

    /// Executes one resolved [`StageGesture`] against the git backend,
    /// returning a success echo or a displayable error. Does not mutate
    /// `self`.
    fn run_stage_gesture(
        &self,
        gesture: &StageGesture,
        path: &str,
        staging: bool,
        verb: &str,
    ) -> Result<String, String> {
        let Some(ops) = self.stage_ops.as_deref() else {
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
                let Some(Some(raw)) = self.patches.get(self.view.selected_file) else {
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
                let Some(Some(raw)) = self.patches.get(self.view.selected_file) else {
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

    /// Resolves a Visual selection (`anchor`..cursor, order-independent)
    /// into `(hunk_index, body-line indices)` for [`build_line_patch`]:
    /// the indices count every body line of the hunk from 0, and only the
    /// selected `+`/`-` lines are included (context is always kept by the
    /// patch builder anyway). Errors if the selection's line rows span more
    /// than one hunk, or contain no changed lines at all.
    fn visual_stage_selection(&self, anchor: usize) -> Result<(usize, Vec<usize>), &'static str> {
        let (lo, hi) = if anchor <= self.view.cursor {
            (anchor, self.view.cursor)
        } else {
            (self.view.cursor, anchor)
        };

        // Body-line indices are per-hunk positions counted over Row::Line
        // rows only (annotation display rows are interleaved in `rows` but
        // are not hunk body lines).
        let mut body_counters: HashMap<usize, usize> = HashMap::new();
        let mut hunks_in_span: HashSet<usize> = HashSet::new();
        let mut selected_hunk: Option<usize> = None;
        let mut selected_lines: Vec<usize> = Vec::new();

        for (i, row) in self.view.rows.iter().enumerate() {
            if i > hi {
                break;
            }
            let Row::Line(line) = row else {
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

        if hunks_in_span.len() > 1 {
            return Err("selection spans multiple hunks");
        }
        let Some(hunk_index) = selected_hunk else {
            return Err("no changed lines in selection");
        };
        Ok((hunk_index, selected_lines))
    }

    /// Re-runs the diff and status for the current target, rebuilds
    /// files/patches/rows and the staged list, then restores position: the
    /// previously selected file is kept by path if it still exists (else
    /// the index is clamped to the nearest remaining file), and cursor,
    /// scroll, and the staging-panel cursor are clamped into range. On any
    /// git/parse error the state is left unchanged and a footer message is
    /// set. A no-op without a git backend.
    fn refresh(&mut self) {
        let snapshot = {
            let Some(ops) = self.stage_ops.as_deref() else {
                return;
            };
            build_review(ops, &self.target)
        };
        let snapshot = match snapshot {
            Ok(snapshot) => snapshot,
            Err(e) => {
                self.set_status_message(format!("refresh failed: {e}"));
                return;
            }
        };

        let previous_path = self
            .view
            .files
            .get(self.view.selected_file)
            .map(|f| f.path.clone());
        let previous_index = self.view.selected_file;

        self.view.files = snapshot.files;
        self.patches = snapshot.patches;
        self.staged = snapshot.staged;

        self.view.selected_file = previous_path
            .and_then(|path| self.view.files.iter().position(|f| f.path == path))
            .unwrap_or_else(|| previous_index.min(self.view.files.len().saturating_sub(1)));
        // Content may have changed underneath every cached (path, side).
        self.highlight_cache.clear();
        self.rebuild_rows();
        if self.view.rows.is_empty() {
            self.view.cursor = 0;
            self.view.scroll = 0;
            self.view.sbs_scroll = 0;
        } else {
            self.view.cursor = self
                .view
                .nearest_addressable(self.view.cursor.min(self.view.max_cursor()), true);
            self.view.scroll = self.view.scroll.min(self.view.cursor);
            self.view.ensure_visible();
        }
        self.staging_cursor = self.staging_cursor.min(self.staged.len().saturating_sub(1));
    }

    // -- Staging panel -----------------------------------------------------

    /// Toggles the staging panel: opens it (refreshing the staged list from
    /// `git status` first, so it's current even if nothing was staged this
    /// session) from Normal/Visual, closes it from Staging. A no-op while
    /// Compose or the annotation list is open.
    fn toggle_staging_panel(&mut self) {
        match self.mode {
            Mode::Staging => self.mode = Mode::Normal,
            Mode::Compose | Mode::List | Mode::Search | Mode::Peek => {}
            Mode::Normal | Mode::Visual { .. } => {
                self.refresh_staged_list();
                self.staging_cursor = self.staging_cursor.min(self.staged.len().saturating_sub(1));
                self.mode = Mode::Staging;
            }
        }
    }

    /// Closes the staging panel, returning to [`Mode::Normal`].
    pub fn close_staging(&mut self) {
        self.mode = Mode::Normal;
    }

    /// Moves the staging panel's focus down one file, clamped at the last.
    pub fn staging_move_down(&mut self) {
        if !self.staged.is_empty() {
            self.staging_cursor = (self.staging_cursor + 1).min(self.staged.len() - 1);
        }
    }

    /// Moves the staging panel's focus up one file, clamped at the first.
    pub fn staging_move_up(&mut self) {
        self.staging_cursor = self.staging_cursor.saturating_sub(1);
    }

    /// Unstages the staging panel's focused file, then refreshes. The panel
    /// stays open and its cursor is clamped to the shrunken list. A no-op
    /// on an empty list; failures set a footer message and change nothing.
    pub fn unstage_focused_file(&mut self) {
        let Some(entry) = self.staged.get(self.staging_cursor) else {
            return;
        };
        let path = entry.path.clone();
        let result = {
            let Some(ops) = self.stage_ops.as_deref() else {
                self.set_status_message("staging unavailable (no git backend)");
                return;
            };
            ops.unstage_file(&path)
        };
        match result {
            Ok(()) => {
                self.set_status_message(format!("unstaged {path}"));
                self.refresh();
            }
            Err(e) => self.set_status_message(e.to_string()),
        }
    }

    /// Best-effort re-read of the staged-file list from `git status`,
    /// keeping the previous list on any failure.
    fn refresh_staged_list(&mut self) {
        let staged = {
            let Some(ops) = self.stage_ops.as_deref() else {
                return;
            };
            match ops.status() {
                Ok(status) => staged_from_status(&status),
                Err(_) => return,
            }
        };
        self.staged = staged;
    }

    // -- Search --------------------------------------------------------------

    /// Opens the search input (`/`), starting from an empty pattern buffer
    /// regardless of any already-active search.
    fn enter_search(&mut self) {
        self.search_input.clear();
        self.mode = Mode::Search;
    }

    /// Cancels the in-progress search edit, returning to [`Mode::Normal`].
    /// If the buffer was left empty, this also clears any already-active
    /// search pattern (matching the spec's "Esc-cleared empty pattern"
    /// behavior); a non-empty, uncommitted buffer is discarded without
    /// touching the previously active pattern.
    pub fn cancel_search(&mut self) {
        if self.search_input.is_empty() {
            self.search.pattern = None;
            self.search.matches.clear();
        }
        self.search_input.clear();
        self.mode = Mode::Normal;
    }

    /// Confirms the in-progress search pattern: recomputes matches against
    /// the current file's rows, jumps the cursor to the first match at or
    /// after the cursor (wrapping if none), and echoes `match k/N` (or
    /// `no matches`) in the footer. An empty buffer clears the active
    /// pattern instead, same as an empty-buffer `Esc`.
    pub fn confirm_search(&mut self) {
        let pattern = std::mem::take(&mut self.search_input);
        self.mode = Mode::Normal;
        if pattern.is_empty() {
            self.search.pattern = None;
            self.search.matches.clear();
            return;
        }
        self.search.pattern = Some(pattern);
        self.search.recompute(&self.view.rows);
        match self.search.next_from(self.view.cursor) {
            Some(row) => {
                self.view.cursor = row;
                self.view.ensure_visible();
                let k = self.search.position_of(row).unwrap_or(1);
                self.set_status_message(format!("match {k}/{}", self.search.matches.len()));
            }
            None => self.set_status_message("no matches"),
        }
    }

    /// Applies the `n`/`N` gesture: jumps to the next (`forward = true`) or
    /// previous match relative to the cursor, wrapping around either end.
    /// Sets a transient footer message: `match k/N` on success, `no
    /// matches` if the pattern has zero matches, or `no search pattern` if
    /// no search is active at all.
    fn search_advance(&mut self, forward: bool) {
        if self.search.pattern.is_none() {
            self.set_status_message("no search pattern");
            return;
        }
        if self.search.matches.is_empty() {
            self.set_status_message("no matches");
            return;
        }
        let next = if forward {
            self.search.advance_from(self.view.cursor)
        } else {
            self.search.retreat_from(self.view.cursor)
        };
        if let Some(row) = next {
            self.view.cursor = row;
            self.view.ensure_visible();
            let k = self.search.position_of(row).unwrap_or(1);
            self.set_status_message(format!("match {k}/{}", self.search.matches.len()));
        }
    }

    // -- LSP: request dispatch and event routing ----------------------------

    /// Derives the `(repo-relative path, 0-based line, UTF-16 character)`
    /// position `gd`/`gr`/`K` would request for the cursor's current
    /// position. Valid only on [`Row::Line`] rows with a `new_line`
    /// (Added/Context — a `Removed` line has no position in the file as it
    /// exists on disk). `None` on any other row.
    fn code_intel_position(&self) -> Option<(String, u32, u32)> {
        let file = self.view.files.get(self.view.selected_file)?;
        let Row::Line(line) = self.view.rows.get(self.view.cursor)? else {
            return None;
        };
        if !matches!(line.origin, LineOrigin::Added | LineOrigin::Context) {
            return None;
        }
        let new_line = line.new_line?;
        let col = self.view.effective_column().unwrap_or(0);
        let character = utf16_offset(&line.content, col);
        Some((file.path.clone(), new_line - 1, character))
    }

    /// Issues a `gd`/`gr`/`K` request for the cursor's current position:
    /// validates the row and the file's on-disk existence, lazily creates
    /// the LSP client against `repo_root` on first use, and records the
    /// request as pending. Sets a footer message either way — `"lsp:
    /// resolving…"` while awaiting a response, or `"no code intelligence
    /// here"` for any case that can't even start a request (invalid row,
    /// no repo root, missing file, or no server available for this
    /// language). A new request always supersedes interest in whatever was
    /// previously pending.
    fn request_code_intel(&mut self, kind: PeekKind) {
        let Some((path, line, character)) = self.code_intel_position() else {
            self.set_status_message("no code intelligence here");
            return;
        };
        let Some(root) = self.repo_root.clone() else {
            self.set_status_message("no code intelligence here");
            return;
        };
        let abs_path = root.join(&path);
        if !abs_path.is_file() {
            self.set_status_message("no code intelligence here");
            return;
        }

        if self.lsp.is_none() {
            self.lsp = Some(Box::new(LspManager::new(root)));
        }
        // A new request always cancels interest in whatever was pending.
        self.pending_lsp = None;
        let Some(lsp) = self.lsp.as_mut() else {
            return;
        };
        let request = match kind {
            PeekKind::Definition => lsp.request_definition(&abs_path, line, character),
            PeekKind::References => lsp.request_references(&abs_path, line, character),
            PeekKind::Hover => lsp.request_hover(&abs_path, line, character),
        };
        match request {
            Some(id) => {
                self.pending_lsp = Some((id, kind));
                self.set_status_message("lsp: resolving\u{2026}");
            }
            None => self.set_status_message("no code intelligence here"),
        }
    }

    /// Drains events from the LSP client (if one exists) and routes them.
    /// Never blocks; a no-op without a live client. Called once per event
    /// loop tick, on both a keypress and a timeout, so responses keep
    /// flowing while the user isn't typing.
    pub fn poll_lsp(&mut self) {
        let Some(lsp) = self.lsp.as_mut() else {
            return;
        };
        let events = lsp.poll();
        for event in events {
            self.handle_lsp_event(event);
        }
    }

    /// Routes one [`LspEvent`]: an id that doesn't match the currently
    /// pending request is ignored (a stale response, or one superseded by
    /// a newer request). A matching event opens the peek overlay
    /// (Definition/References with results, or Hover), or sets a footer
    /// message instead (`"no results"` for an empty location list,
    /// `"lsp: failed"` for [`LspEvent::Failed`]).
    fn handle_lsp_event(&mut self, event: LspEvent) {
        let Some((pending_id, kind)) = self.pending_lsp else {
            return;
        };
        let id = match &event {
            LspEvent::Definition { id, .. } => *id,
            LspEvent::References { id, .. } => *id,
            LspEvent::Hover { id, .. } => *id,
            LspEvent::Failed { id } => *id,
        };
        if id != pending_id {
            return;
        }
        self.pending_lsp = None;

        match event {
            LspEvent::Definition { locations, .. } => {
                self.open_peek_locations(kind, locations);
            }
            LspEvent::References { locations, .. } => {
                self.open_peek_locations(kind, locations);
            }
            LspEvent::Hover { contents, .. } => {
                self.peek = Some(PeekState::hover(contents));
                self.mode = Mode::Peek;
            }
            LspEvent::Failed { .. } => self.set_status_message("lsp: failed"),
        }
    }

    fn open_peek_locations(&mut self, kind: PeekKind, locations: Vec<SourceLocation>) {
        if locations.is_empty() {
            self.set_status_message("no results");
            return;
        }
        self.peek = Some(PeekState::locations(kind, locations));
        self.mode = Mode::Peek;
        self.refresh_peek_preview();
    }

    // -- Peek overlay --------------------------------------------------------

    /// Populates the preview cache for the currently selected location, if
    /// it isn't already cached: reads the file from disk and highlights it
    /// (best-effort — an unreadable file or unsupported language leaves it
    /// uncached, and the overlay shows "(preview unavailable)"). A no-op
    /// for Hover (no location list) or once a path is already cached.
    fn refresh_peek_preview(&mut self) {
        let Some(peek) = self.peek.as_ref() else {
            return;
        };
        if matches!(peek.kind, PeekKind::Hover) {
            return;
        }
        let Some(loc) = peek.locations.get(peek.selected) else {
            return;
        };
        let path = loc.path.clone();
        if peek.preview_cache.contains_key(&path) {
            return;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            return;
        };
        let lines: Vec<String> = content.lines().map(str::to_string).collect();
        let spans = match path.to_str().and_then(Lang::from_path) {
            Some(lang) => self.highlighter.highlight_lines(lang, &content),
            None => Vec::new(),
        };
        if let Some(peek) = self.peek.as_mut() {
            peek.preview_cache
                .insert(path, CachedPreview { lines, spans });
        }
    }

    /// Moves the peek selection down one result (Definition/References), or
    /// scrolls the hover text down one line. A no-op if no overlay is open.
    pub fn peek_move_down(&mut self) {
        let Some(peek) = self.peek.as_mut() else {
            return;
        };
        match peek.kind {
            PeekKind::Hover => {
                let max = peek.hover_line_count().saturating_sub(1);
                peek.hover_scroll = (peek.hover_scroll + 1).min(max);
            }
            PeekKind::Definition | PeekKind::References => {
                if !peek.locations.is_empty() {
                    peek.selected = (peek.selected + 1).min(peek.locations.len() - 1);
                }
                self.refresh_peek_preview();
            }
        }
    }

    /// Moves the peek selection up one result, or scrolls hover text up one
    /// line. A no-op if no overlay is open.
    pub fn peek_move_up(&mut self) {
        let Some(peek) = self.peek.as_mut() else {
            return;
        };
        match peek.kind {
            PeekKind::Hover => peek.hover_scroll = peek.hover_scroll.saturating_sub(1),
            PeekKind::Definition | PeekKind::References => {
                peek.selected = peek.selected.saturating_sub(1);
                self.refresh_peek_preview();
            }
        }
    }

    /// Closes the peek overlay, returning to [`Mode::Normal`].
    pub fn close_peek(&mut self) {
        self.peek = None;
        self.mode = Mode::Normal;
    }

    /// Applies the Peek-mode `Enter` gesture: for Definition/References,
    /// jumps the diff cursor to the closest row for the selected result's
    /// new-side line and closes the overlay if the result's file is one of
    /// the diff's files, or sets a `"not in diff"` footer message
    /// (v1 — full cross-file browsing is out of scope) otherwise. A no-op
    /// for Hover.
    pub fn peek_enter(&mut self) {
        let Some(peek) = &self.peek else {
            return;
        };
        if !matches!(peek.kind, PeekKind::Definition | PeekKind::References) {
            return;
        }
        let Some(loc) = peek.locations.get(peek.selected) else {
            return;
        };
        let target_path = loc.path.clone();
        let target_line = loc.line + 1; // 0-based LSP line -> 1-based new_line

        let file_index = self.view.files.iter().position(|f| {
            self.repo_root
                .as_ref()
                .map(|root| root.join(&f.path) == target_path)
                .unwrap_or(false)
        });

        let Some(file_index) = file_index else {
            self.set_status_message("not in diff");
            return;
        };

        self.view.selected_file = file_index;
        self.rebuild_rows();
        self.view.cursor = closest_row_for_new_line(&self.view.rows, target_line).unwrap_or(0);
        self.view.scroll = 0;
        self.view.sbs_scroll = 0;
        self.view.ensure_visible();
        self.close_peek();
    }
}

/// Which [`Action`]s remain live in [`Mode::Visual`]. Everything else
/// (hunk/file navigation, half-page scroll) is disabled while a selection
/// is in progress.
fn visual_mode_allows(action: Action) -> bool {
    matches!(
        action,
        Action::CursorDown
            | Action::CursorUp
            | Action::CursorLeft
            | Action::CursorRight
            | Action::WordForward
            | Action::WordBackward
            | Action::EnterVisual
            | Action::Compose
            | Action::ToggleList
            | Action::ToggleStage
            | Action::ToggleStagingPanel
            | Action::ToggleHelp
            | Action::ToggleView
    )
}

/// The `Line` target for a diff line row: `Removed` lines anchor to the old
/// side/number, `Added`/`Context` lines to the new side/number. `None` only
/// if the row's own invariant (removed lines always carry `old_line`,
/// non-removed lines always carry `new_line`) is somehow violated.
fn line_target(path: &str, line: &LineRow) -> Option<Target> {
    match line.origin {
        LineOrigin::Removed => line.old_line.map(|n| Target::line(path, n, Side::Old)),
        LineOrigin::Added | LineOrigin::Context => {
            line.new_line.map(|n| Target::line(path, n, Side::New))
        }
    }
}

/// Converts a 0-based char index within `content` to its UTF-16 code-unit
/// offset, matching the LSP position convention (`gd`/`gr`/`K` requests use
/// this to convert the column cursor's char index into a wire position).
/// Characters outside the Basic Multilingual Plane (e.g. most emoji) count
/// as 2 UTF-16 units, per [`char::len_utf16`].
fn utf16_offset(content: &str, char_index: usize) -> u32 {
    content
        .chars()
        .take(char_index)
        .map(char::len_utf16)
        .sum::<usize>() as u32
}

/// The row in `rows` whose `new_line` is closest to `target_line` (ties
/// broken toward the earlier row). `None` if `rows` has no `Line` row with
/// a `new_line` at all.
fn closest_row_for_new_line(rows: &[Row], target_line: u32) -> Option<usize> {
    rows.iter()
        .enumerate()
        .filter_map(|(i, r)| match r {
            Row::Line(l) => l.new_line.map(|n| (i, n)),
            _ => None,
        })
        .min_by_key(|&(_, n)| (i64::from(n) - i64::from(target_line)).abs())
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotate::Classification;
    use crate::git::RawFilePatch;
    use crate::ui::compose::TextBuffer;
    use crate::ui::diff_view_state::ViewMode;
    use crate::ui::rows::SbsRow;

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

    fn file_with_raw(path: &str, raw: &str) -> FileDiff {
        FileDiff::from_patch(&RawFilePatch {
            path: path.to_string(),
            old_path: None,
            raw: raw.to_string(),
            is_binary: false,
        })
        .unwrap()
    }

    #[test]
    fn cursor_down_clamps_at_last_row() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        let last = app.view.rows.len() - 1;
        for _ in 0..20 {
            app.apply(Action::CursorDown);
        }
        assert_eq!(app.view.cursor, last);
    }

    #[test]
    fn cursor_up_clamps_at_zero() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.apply(Action::CursorUp);
        assert_eq!(app.view.cursor, 0);
    }

    #[test]
    fn cursor_motion_on_empty_diff_stays_at_zero() {
        let mut app = App::new(vec![]);
        app.apply(Action::CursorDown);
        assert_eq!(app.view.cursor, 0);
        app.apply(Action::HalfPageDown);
        assert_eq!(app.view.cursor, 0);
    }

    #[test]
    fn half_page_motion_uses_last_known_viewport_height() {
        // 5 hunks -> 1 + 5*3 = 16 rows, plenty of headroom for a
        // half-page-of-10 step in either direction.
        let mut app = App::new(vec![file("a.rs", 5)]);
        app.view.set_viewport_height(10);
        app.apply(Action::HalfPageDown);
        assert_eq!(app.view.cursor, 5);
        app.apply(Action::HalfPageUp);
        assert_eq!(app.view.cursor, 0);
    }

    #[test]
    fn half_page_never_steps_by_zero_on_tiny_viewport() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.view.set_viewport_height(1);
        app.apply(Action::HalfPageDown);
        assert_eq!(app.view.cursor, 1);
    }

    #[test]
    fn ensure_visible_scrolls_down_to_follow_cursor() {
        let mut app = App::new(vec![file("a.rs", 3)]);
        app.view.set_viewport_height(3);
        for _ in 0..6 {
            app.apply(Action::CursorDown);
        }
        assert_eq!(app.view.cursor, 6);
        assert!(app.view.scroll <= app.view.cursor);
        assert!(app.view.cursor < app.view.scroll + 3);
    }

    #[test]
    fn ensure_visible_scrolls_up_to_follow_cursor() {
        let mut app = App::new(vec![file("a.rs", 3)]);
        app.view.set_viewport_height(3);
        for _ in 0..6 {
            app.apply(Action::CursorDown);
        }
        for _ in 0..6 {
            app.apply(Action::CursorUp);
        }
        assert_eq!(app.view.cursor, 0);
        assert_eq!(app.view.scroll, 0);
    }

    #[test]
    fn next_hunk_jumps_within_file() {
        let mut app = App::new(vec![file("a.rs", 2)]);
        app.apply(Action::NextHunk);
        let Row::HunkHeader { hunk_index, .. } = &app.view.rows[app.view.cursor] else {
            panic!("expected hunk header at cursor");
        };
        assert_eq!(*hunk_index, 0);

        app.apply(Action::NextHunk);
        let Row::HunkHeader { hunk_index, .. } = &app.view.rows[app.view.cursor] else {
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
        assert_eq!(app.view.selected_file, 1);
        assert!(matches!(
            app.view.rows[app.view.cursor],
            Row::HunkHeader { .. }
        ));
    }

    #[test]
    fn next_hunk_at_last_file_last_hunk_is_no_op() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.apply(Action::NextHunk);
        let cursor_before = app.view.cursor;
        let file_before = app.view.selected_file;
        app.apply(Action::NextHunk);
        assert_eq!(app.view.cursor, cursor_before);
        assert_eq!(app.view.selected_file, file_before);
    }

    #[test]
    fn prev_hunk_crosses_file_boundary_backwards() {
        let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
        app.apply(Action::NextFile); // move to b.rs, cursor reset to top (FileHeader)
        assert_eq!(app.view.selected_file, 1);
        app.apply(Action::PrevHunk); // no hunk header before cursor in b.rs -> cross back
        assert_eq!(app.view.selected_file, 0);
        assert!(matches!(
            app.view.rows[app.view.cursor],
            Row::HunkHeader { .. }
        ));
    }

    #[test]
    fn prev_hunk_at_first_file_before_first_hunk_is_no_op() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        let cursor_before = app.view.cursor;
        app.apply(Action::PrevHunk);
        assert_eq!(app.view.cursor, cursor_before);
        assert_eq!(app.view.selected_file, 0);
    }

    #[test]
    fn next_file_switches_and_resets_cursor() {
        let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
        app.apply(Action::CursorDown);
        app.apply(Action::NextFile);
        assert_eq!(app.view.selected_file, 1);
        assert_eq!(app.view.cursor, 0);
        assert_eq!(app.view.scroll, 0);
    }

    #[test]
    fn next_file_clamps_at_last_file() {
        let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
        app.apply(Action::NextFile);
        app.apply(Action::NextFile);
        assert_eq!(app.view.selected_file, 1);
    }

    #[test]
    fn prev_file_clamps_at_first_file() {
        let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
        app.apply(Action::PrevFile);
        assert_eq!(app.view.selected_file, 0);
    }

    #[test]
    fn prev_file_switches_and_resets_cursor() {
        let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
        app.apply(Action::NextFile);
        app.apply(Action::CursorDown);
        app.apply(Action::PrevFile);
        assert_eq!(app.view.selected_file, 0);
        assert_eq!(app.view.cursor, 0);
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
        let cursor = app.view.cursor;
        app.apply(Action::Quit);
        app.apply(Action::QuitDiscard);
        assert_eq!(app.view.cursor, cursor);
    }

    // -- Visual mode ------------------------------------------------------

    #[test]
    fn enter_visual_on_line_row_sets_anchor() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.apply(Action::CursorDown); // hunk header
        app.apply(Action::CursorDown); // onto a line row
        let cursor = app.view.cursor;
        assert!(matches!(app.view.rows[cursor], Row::Line(_)));
        app.apply(Action::EnterVisual);
        assert_eq!(app.mode, Mode::Visual { anchor: cursor });
    }

    #[test]
    fn enter_visual_on_header_row_is_a_no_op() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        assert!(matches!(app.view.rows[0], Row::FileHeader { .. }));
        app.apply(Action::EnterVisual);
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn v_again_cancels_visual_mode() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.apply(Action::CursorDown); // hunk header
        app.apply(Action::CursorDown); // line row
        app.apply(Action::EnterVisual);
        assert!(matches!(app.mode, Mode::Visual { .. }));
        app.apply(Action::EnterVisual);
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn visual_mode_disables_hunk_and_file_navigation() {
        let mut app = App::new(vec![file("a.rs", 2)]);
        app.apply(Action::CursorDown); // hunk header
        app.apply(Action::CursorDown); // line row
        app.apply(Action::EnterVisual);
        let cursor_before = app.view.cursor;
        app.apply(Action::NextHunk);
        app.apply(Action::NextFile);
        app.apply(Action::HalfPageDown);
        assert_eq!(app.view.cursor, cursor_before);
        assert!(matches!(app.mode, Mode::Visual { .. }));
    }

    #[test]
    fn visual_mode_j_k_extend_selection() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.apply(Action::CursorDown); // hunk header
        app.apply(Action::CursorDown); // first line row
        let anchor = app.view.cursor;
        app.apply(Action::EnterVisual);
        app.apply(Action::CursorDown);
        assert_eq!(app.mode, Mode::Visual { anchor });
        assert!(app.view.cursor > anchor);
    }

    // -- Target derivation --------------------------------------------------

    #[test]
    fn target_for_cursor_on_removed_line_uses_old_side() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,2 +1,1 @@
-removed
 kept
";
        let mut app = App::new(vec![file_with_raw("f.rs", raw)]);
        app.apply(Action::CursorDown); // hunk header -> row 1
        app.apply(Action::CursorDown); // removed line -> row 2
        let target = app.target_for_cursor().unwrap();
        assert_eq!(target, Target::line("f.rs", 1, Side::Old));
    }

    #[test]
    fn target_for_cursor_on_added_line_uses_new_side() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,1 +1,2 @@
 kept
+added
";
        let mut app = App::new(vec![file_with_raw("f.rs", raw)]);
        app.apply(Action::CursorDown); // hunk header
        app.apply(Action::CursorDown); // context "kept" -> new side too
        app.apply(Action::CursorDown); // added line
        let target = app.target_for_cursor().unwrap();
        assert_eq!(target, Target::line("f.rs", 2, Side::New));
    }

    #[test]
    fn target_for_cursor_on_context_line_uses_new_side() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,1 +1,1 @@
 kept
";
        let mut app = App::new(vec![file_with_raw("f.rs", raw)]);
        app.apply(Action::CursorDown); // hunk header
        app.apply(Action::CursorDown); // context line
        let target = app.target_for_cursor().unwrap();
        assert_eq!(target, Target::line("f.rs", 1, Side::New));
    }

    #[test]
    fn target_for_cursor_on_hunk_header_spans_new_side() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,1 +1,3 @@
 a
+b
+c
";
        let mut app = App::new(vec![file_with_raw("f.rs", raw)]);
        app.apply(Action::CursorDown); // hunk header row
        let target = app.target_for_cursor().unwrap();
        assert_eq!(target, Target::hunk("f.rs", 1, 3).unwrap());
    }

    #[test]
    fn target_for_cursor_on_hunk_header_falls_back_to_old_side_when_new_count_zero() {
        let raw = "\
diff --git a/gone.rs b/gone.rs
deleted file mode 100644
index 111..000
--- a/gone.rs
+++ /dev/null
@@ -1,3 +0,0 @@
-a
-b
-c
";
        let mut app = App::new(vec![file_with_raw("gone.rs", raw)]);
        app.apply(Action::CursorDown); // hunk header row
        let target = app.target_for_cursor().unwrap();
        assert_eq!(target, Target::hunk("gone.rs", 1, 3).unwrap());
    }

    #[test]
    fn target_for_cursor_on_file_header_is_file_target() {
        let app = App::new(vec![file("a.rs", 1)]);
        let target = app.target_for_cursor().unwrap();
        assert_eq!(target, Target::file("a.rs"));
    }

    #[test]
    fn target_for_cursor_on_binary_row_is_file_target() {
        let raw = "\
diff --git a/img.png b/img.png
index 1..2 100644
Binary files a/img.png and b/img.png differ
";
        let mut app = App::new(vec![
            FileDiff::from_patch(&RawFilePatch {
                path: "img.png".to_string(),
                old_path: None,
                raw: raw.to_string(),
                is_binary: true,
            })
            .unwrap(),
        ]);
        app.apply(Action::CursorDown); // Binary row
        let target = app.target_for_cursor().unwrap();
        assert_eq!(target, Target::file("img.png"));
    }

    // -- Side-by-side view --------------------------------------------------

    #[test]
    fn toggle_view_flips_and_round_trips() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        assert_eq!(app.view.layout, ViewMode::Unified);
        app.apply(Action::ToggleView);
        assert_eq!(app.view.layout, ViewMode::SideBySide);
        app.apply(Action::ToggleView);
        assert_eq!(app.view.layout, ViewMode::Unified);
    }

    #[test]
    fn toggle_view_is_preserved_across_file_switches() {
        let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
        app.apply(Action::ToggleView);
        assert_eq!(app.view.layout, ViewMode::SideBySide);
        app.apply(Action::NextFile);
        assert_eq!(app.view.layout, ViewMode::SideBySide);
    }

    #[test]
    fn toggle_view_is_allowed_in_visual_mode() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.apply(Action::CursorDown); // hunk header
        app.apply(Action::CursorDown); // line row
        app.apply(Action::EnterVisual);
        app.apply(Action::ToggleView);
        assert_eq!(app.view.layout, ViewMode::SideBySide);
        assert!(matches!(app.mode, Mode::Visual { .. }));
    }

    #[test]
    fn sbs_rows_are_derived_alongside_rows_on_build() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,2 +1,2 @@
-old1
+new1
 ctx
";
        let app = App::new(vec![file_with_raw("f.rs", raw)]);
        // rows: FileHeader(0) HunkHeader(1) old1(2) new1(3) ctx(4)
        assert_eq!(app.view.rows.len(), 5);
        // sbs_rows: Full(0) Full(1) Paired{2,3} Context(4) -> 4 visual rows.
        assert_eq!(app.view.sbs_rows.len(), 4);
        assert_eq!(app.view.sbs_rows[0], SbsRow::Full(0));
        assert_eq!(app.view.sbs_rows[1], SbsRow::Full(1));
        assert_eq!(app.view.sbs_rows[2], SbsRow::Paired { old: 2, new: 3 });
        assert_eq!(app.view.sbs_rows[3], SbsRow::Context(4));
    }

    /// [`App::ensure_visible`] keeps `sbs_scroll` in visual-row space: a
    /// tiny viewport whose cursor sits on the paired line must not scroll
    /// past the (fewer) visual rows even though the source-row cursor has
    /// advanced past where `scroll` (row-space) would put it.
    #[test]
    fn sbs_scroll_tracks_visual_rows_not_source_rows() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,2 +1,2 @@
-old1
+new1
 ctx
";
        let mut app = App::new(vec![file_with_raw("f.rs", raw)]);
        app.view.set_viewport_height(2);
        // Cursor onto the paired "new1" row (source row 3, visual row 2).
        app.apply(Action::CursorDown); // hunk header, source row 1
        app.apply(Action::CursorDown); // old1, source row 2
        app.apply(Action::CursorDown); // new1, source row 3 (visual row 2)
        assert_eq!(app.view.cursor, 3);
        // Visual row 2 must be visible within a 2-row viewport: sbs_scroll
        // in [1, 2].
        assert!(app.view.sbs_scroll <= 2 && app.view.sbs_scroll + 2 > 2);
    }

    /// The cursor stays a source-row index in both view modes (see
    /// [`ViewMode`]'s docs): every gesture that derives an annotation
    /// target must land on the exact same [`Target`] whether `t` has been
    /// pressed or not, since side-by-side is purely a rendering-time view
    /// over the same `rows`.
    #[test]
    fn target_for_cursor_is_identical_in_both_views() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,2 +1,2 @@
-removed
+added
";
        let mut app = App::new(vec![file_with_raw("f.rs", raw)]);
        app.apply(Action::CursorDown); // hunk header
        app.apply(Action::CursorDown); // removed line
        let unified_target = app.target_for_cursor();

        app.apply(Action::ToggleView);
        let sbs_target = app.target_for_cursor();
        assert_eq!(unified_target, sbs_target);
        assert_eq!(unified_target, Some(Target::line("f.rs", 1, Side::Old)));
    }

    /// Same parity check for the Visual-mode range target.
    #[test]
    fn target_for_visual_is_identical_in_both_views() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.apply(Action::CursorDown); // hunk header
        app.apply(Action::CursorDown); // removed line
        let anchor = app.view.cursor;
        app.apply(Action::EnterVisual);
        app.apply(Action::CursorDown);
        let unified_target = app.target_for_visual(anchor);

        app.apply(Action::ToggleView);
        let sbs_target = app.target_for_visual(anchor);
        assert_eq!(unified_target, sbs_target);
    }

    /// Same parity check for the `space` staging gesture: two apps built
    /// from the same fixture, one toggled to side-by-side before staging,
    /// must resolve to the exact same patch — the gesture is derived from
    /// `rows[cursor]`, which side-by-side never touches.
    #[test]
    fn staging_gesture_target_is_identical_in_both_views() {
        let p = raw_patch("a.rs", 1);
        let (mut unified_app, unified_calls) = app_with_fake(
            vec![p.clone()],
            DiffTarget::WorkingTree,
            vec![p.clone()],
            vec![],
        );
        unified_app.apply(Action::CursorDown); // hunk header
        unified_app.apply(Action::CursorDown); // line row
        unified_app.apply(Action::ToggleStage);
        let StageCall::Apply(unified_patch) = single_call(&unified_calls) else {
            panic!("expected apply_cached");
        };

        let (mut sbs_app, sbs_calls) =
            app_with_fake(vec![p.clone()], DiffTarget::WorkingTree, vec![p], vec![]);
        sbs_app.apply(Action::ToggleView);
        sbs_app.apply(Action::CursorDown); // hunk header
        sbs_app.apply(Action::CursorDown); // line row
        sbs_app.apply(Action::ToggleStage);
        let StageCall::Apply(sbs_patch) = single_call(&sbs_calls) else {
            panic!("expected apply_cached");
        };

        assert_eq!(unified_patch, sbs_patch);
    }

    /// Search matches are source-row indices; a match on a source row that
    /// ends up on the left (old) side of a paired visual row must not also
    /// be reported for the right (new) side's row, and vice versa — i.e.
    /// the match set itself is unaffected by which view is active.
    #[test]
    fn search_matches_are_source_rows_unaffected_by_view() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,2 +1,2 @@
-old needle
+new needle
";
        let mut app = App::new(vec![file_with_raw("f.rs", raw)]);
        app.search.pattern = Some("needle".to_string());
        app.search.recompute(&app.view.rows);
        let unified_matches = app.search.matches.clone();

        app.apply(Action::ToggleView);
        app.search.recompute(&app.view.rows);
        let sbs_matches = app.search.matches.clone();
        assert_eq!(unified_matches, sbs_matches);
        // Both the removed and added rows matched (each contains "needle"),
        // and they resolve to distinct source rows / distinct sides of the
        // same paired visual row.
        assert_eq!(unified_matches.len(), 2);
        let (Row::Line(old_line), Row::Line(new_line)) = (
            &app.view.rows[unified_matches[0]],
            &app.view.rows[unified_matches[1]],
        ) else {
            panic!("expected line rows");
        };
        assert_eq!(old_line.origin, LineOrigin::Removed);
        assert_eq!(new_line.origin, LineOrigin::Added);
    }

    #[test]
    fn target_for_visual_removed_only_uses_old_side() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,3 +0,0 @@
-a
-b
-c
";
        let mut app = App::new(vec![file_with_raw("f.rs", raw)]);
        app.apply(Action::CursorDown); // hunk header
        app.apply(Action::CursorDown); // line a
        let anchor = app.view.cursor;
        app.apply(Action::EnterVisual);
        app.apply(Action::CursorDown); // line b
        app.apply(Action::CursorDown); // line c
        let target = app.target_for_visual(anchor).unwrap();
        assert_eq!(target, Target::range("f.rs", 1, 3, Side::Old).unwrap());
    }

    #[test]
    fn target_for_visual_mixed_uses_new_side_of_non_removed_rows() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,2 +1,3 @@
-old
+new1
+new2
 ctx
";
        let mut app = App::new(vec![file_with_raw("f.rs", raw)]);
        app.apply(Action::CursorDown); // hunk header
        app.apply(Action::CursorDown); // removed "old"
        let anchor = app.view.cursor;
        app.apply(Action::EnterVisual);
        app.apply(Action::CursorDown); // new1
        app.apply(Action::CursorDown); // new2
        app.apply(Action::CursorDown); // ctx
        let target = app.target_for_visual(anchor).unwrap();
        // new1=1, new2=2, ctx=3 -> spans 1..3 on the new side.
        assert_eq!(target, Target::range("f.rs", 1, 3, Side::New).unwrap());
    }

    #[test]
    fn target_for_visual_with_no_line_rows_is_none() {
        let app = App::new(vec![file("a.rs", 1)]);
        // Cursor and anchor both on the FileHeader row (0).
        let target = app.target_for_visual(0);
        assert_eq!(target, None);
    }

    // -- Compose -----------------------------------------------------------

    #[test]
    fn compose_action_in_normal_opens_compose_with_cursor_target() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.apply(Action::Compose);
        assert_eq!(app.mode, Mode::Compose);
        let compose = app.compose.as_ref().unwrap();
        assert_eq!(compose.target, Target::file("a.rs"));
        assert_eq!(compose.editing_id, None);
    }

    #[test]
    fn compose_action_with_no_target_is_a_no_op() {
        let mut app = App::new(vec![]);
        app.apply(Action::Compose);
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.compose.is_none());
    }

    #[test]
    fn compose_action_in_visual_uses_range_target() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,2 +1,2 @@
 a
+b
";
        let mut app = App::new(vec![file_with_raw("f.rs", raw)]);
        app.apply(Action::CursorDown); // hunk header
        app.apply(Action::CursorDown); // line a
        app.apply(Action::EnterVisual);
        app.apply(Action::CursorDown); // line b
        app.apply(Action::Compose);
        assert_eq!(app.mode, Mode::Compose);
        let compose = app.compose.as_ref().unwrap();
        assert_eq!(
            compose.target,
            Target::range("f.rs", 1, 2, Side::New).unwrap()
        );
    }

    #[test]
    fn cancel_compose_discards_draft() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.apply(Action::Compose);
        app.compose.as_mut().unwrap().buffer.insert_char('x');
        app.cancel_compose();
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.compose.is_none());
        assert!(app.annotations.is_empty());
    }

    #[test]
    fn submit_compose_with_body_adds_annotation_and_refreshes_rows() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.apply(Action::Compose);
        for c in "looks good".chars() {
            app.compose.as_mut().unwrap().buffer.insert_char(c);
        }
        app.submit_compose();
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.annotations.len(), 1);
        assert_eq!(app.annotations.iter().next().unwrap().body, "looks good");
        // Row model was rebuilt: the FileHeader row is now flagged annotated.
        assert!(matches!(
            app.view.rows[0],
            Row::FileHeader {
                annotated: true,
                ..
            }
        ));
    }

    #[test]
    fn submit_compose_with_empty_body_cancels_without_error() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.apply(Action::Compose);
        app.compose.as_mut().unwrap().buffer.insert_char(' ');
        app.submit_compose();
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.annotations.is_empty());
    }

    #[test]
    fn submit_compose_while_editing_updates_body_and_classification() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        let id = app
            .annotations
            .add(Target::file("a.rs"), Classification::Nit, "old body")
            .unwrap();
        app.edit_focused_annotation(); // list_cursor defaults to 0
        app.compose.as_mut().unwrap().buffer = TextBuffer::new();
        for c in "new body".chars() {
            app.compose.as_mut().unwrap().buffer.insert_char(c);
        }
        app.compose.as_mut().unwrap().classification = Classification::Praise;
        app.submit_compose();
        let annotation = app.annotations.iter().find(|a| a.id == id).unwrap();
        assert_eq!(annotation.body, "new body");
        assert_eq!(annotation.classification, Classification::Praise);
    }

    // -- Annotation list panel ---------------------------------------------

    #[test]
    fn toggle_list_opens_and_closes() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.apply(Action::ToggleList);
        assert_eq!(app.mode, Mode::List);
        app.apply(Action::ToggleList);
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn list_move_down_and_up_clamp() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.annotations
            .add(Target::file("a.rs"), Classification::Nit, "one")
            .unwrap();
        app.annotations
            .add(Target::file("a.rs"), Classification::Issue, "two")
            .unwrap();
        app.list_move_down();
        assert_eq!(app.list_cursor, 1);
        app.list_move_down();
        assert_eq!(app.list_cursor, 1); // clamped at last
        app.list_move_up();
        assert_eq!(app.list_cursor, 0);
        app.list_move_up();
        assert_eq!(app.list_cursor, 0); // clamped at first
    }

    #[test]
    fn jump_to_focused_annotation_switches_file_and_places_cursor() {
        let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
        app.annotations
            .add(
                Target::line("b.rs", 1, Side::Old),
                Classification::Issue,
                "note",
            )
            .unwrap();
        app.list_cursor = 0;
        app.jump_to_focused_annotation();
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.view.selected_file, 1);
        let Row::Line(line) = &app.view.rows[app.view.cursor] else {
            panic!("expected cursor on a line row");
        };
        assert_eq!(line.old_line, Some(1));
    }

    #[test]
    fn edit_focused_annotation_prefills_compose() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.annotations
            .add(Target::file("a.rs"), Classification::Question, "why?")
            .unwrap();
        app.list_cursor = 0;
        app.edit_focused_annotation();
        assert_eq!(app.mode, Mode::Compose);
        let compose = app.compose.as_ref().unwrap();
        assert_eq!(compose.buffer.text(), "why?");
        assert_eq!(compose.classification, Classification::Question);
        assert_eq!(compose.editing_id, Some(0));
    }

    #[test]
    fn delete_focused_annotation_removes_and_refreshes() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.annotations
            .add(Target::file("a.rs"), Classification::Nit, "note")
            .unwrap();
        app.list_cursor = 0;
        app.delete_focused_annotation();
        assert!(app.annotations.is_empty());
        assert!(matches!(
            app.view.rows[0],
            Row::FileHeader {
                annotated: false,
                ..
            }
        ));
    }

    #[test]
    fn list_actions_on_empty_store_are_no_ops() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.list_move_down();
        assert_eq!(app.list_cursor, 0);
        app.delete_focused_annotation();
        assert!(app.annotations.is_empty());
        app.edit_focused_annotation();
        assert!(app.compose.is_none());
    }

    // -- Staging -------------------------------------------------------------

    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::git::{ChangeKind, FileStatus, GitError, StatusCode};

    /// One recorded call against the fake staging backend.
    #[derive(Debug, Clone, PartialEq)]
    enum StageCall {
        StageFile(String),
        UnstageFile(String),
        Apply(String),
        Unapply(String),
    }

    /// A recording [`StageOps`] fake: staging calls are appended to a
    /// shared log; `diff`/`status` return fixed data (what refresh will
    /// see after an operation); `fail_ops` makes every staging call error.
    #[derive(Default)]
    struct FakeGit {
        calls: Rc<RefCell<Vec<StageCall>>>,
        diff: Vec<RawFilePatch>,
        status: Vec<FileStatus>,
        untracked_content: std::collections::HashMap<String, Vec<u8>>,
        fail_ops: bool,
        show_calls: Rc<RefCell<usize>>,
        show_content: Option<String>,
    }

    impl FakeGit {
        fn op_result(&self) -> Result<(), GitError> {
            if self.fail_ops {
                Err(GitError::Parse("simulated git failure".to_string()))
            } else {
                Ok(())
            }
        }
    }

    impl StageOps for FakeGit {
        fn diff(&self, _target: &DiffTarget) -> Result<Vec<RawFilePatch>, GitError> {
            Ok(self.diff.clone())
        }

        fn status(&self) -> Result<Vec<FileStatus>, GitError> {
            Ok(self.status.clone())
        }

        fn stage_file(&self, path: &str) -> Result<(), GitError> {
            self.calls
                .borrow_mut()
                .push(StageCall::StageFile(path.to_string()));
            self.op_result()
        }

        fn unstage_file(&self, path: &str) -> Result<(), GitError> {
            self.calls
                .borrow_mut()
                .push(StageCall::UnstageFile(path.to_string()));
            self.op_result()
        }

        fn apply_cached(&self, patch: &str) -> Result<(), GitError> {
            self.calls
                .borrow_mut()
                .push(StageCall::Apply(patch.to_string()));
            self.op_result()
        }

        fn unapply_cached(&self, patch: &str) -> Result<(), GitError> {
            self.calls
                .borrow_mut()
                .push(StageCall::Unapply(patch.to_string()));
            self.op_result()
        }

        fn read_worktree_file(&self, path: &str) -> Option<Vec<u8>> {
            self.untracked_content.get(path).cloned()
        }

        fn show_file(&self, _spec: &str) -> Option<String> {
            *self.show_calls.borrow_mut() += 1;
            self.show_content.clone()
        }
    }

    fn raw_patch(path: &str, hunk_count: usize) -> RawFilePatch {
        let mut raw = format!(
            "diff --git a/{path} b/{path}\nindex 1..2 100644\n--- a/{path}\n+++ b/{path}\n"
        );
        for h in 0..hunk_count {
            let start = 1 + h * 10;
            raw.push_str(&format!("@@ -{start},1 +{start},1 @@\n-old{h}\n+new{h}\n"));
        }
        RawFilePatch {
            path: path.to_string(),
            old_path: None,
            raw,
            is_binary: false,
        }
    }

    /// A porcelain status entry with staged (index-side) changes only.
    fn staged_entry(path: &str) -> FileStatus {
        FileStatus {
            kind: ChangeKind::Ordinary,
            staged: StatusCode::Modified,
            unstaged: StatusCode::Unmodified,
            path: path.to_string(),
            orig_path: None,
        }
    }

    /// Builds an `App` over `patches` with a recording fake whose refresh
    /// diff returns `refresh_diff` and refresh status returns `status`.
    /// Returns the app plus the shared call log.
    fn app_with_fake(
        patches: Vec<RawFilePatch>,
        target: DiffTarget,
        refresh_diff: Vec<RawFilePatch>,
        status: Vec<FileStatus>,
    ) -> (App, Rc<RefCell<Vec<StageCall>>>) {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let fake = FakeGit {
            calls: Rc::clone(&calls),
            diff: refresh_diff,
            status,
            ..FakeGit::default()
        };
        let files = patches
            .iter()
            .map(|p| FileDiff::from_patch(p).unwrap())
            .collect();
        let snapshot = ReviewSnapshot {
            files,
            patches: patches.into_iter().map(Some).collect(),
            staged: Vec::new(),
        };
        (App::with_git(snapshot, target, Box::new(fake)), calls)
    }

    /// The single call in the log, panicking if there are zero or many.
    fn single_call(calls: &Rc<RefCell<Vec<StageCall>>>) -> StageCall {
        let calls = calls.borrow();
        assert_eq!(calls.len(), 1, "expected exactly one call, got {calls:?}");
        calls[0].clone()
    }

    #[test]
    fn space_on_hunk_header_stages_that_hunk() {
        let p = raw_patch("a.rs", 2);
        let (mut app, calls) =
            app_with_fake(vec![p.clone()], DiffTarget::WorkingTree, vec![p], vec![]);
        app.apply(Action::NextHunk);
        app.apply(Action::NextHunk); // second hunk's header
        app.apply(Action::ToggleStage);
        let StageCall::Apply(patch) = single_call(&calls) else {
            panic!("expected apply_cached");
        };
        assert!(patch.contains("@@ -11,1 +11,1 @@"));
        assert!(patch.contains("-old1"));
        assert!(!patch.contains("old0"));
        assert_eq!(app.status_message.as_deref(), Some("staged hunk"));
    }

    #[test]
    fn space_on_line_row_stages_enclosing_hunk() {
        let p = raw_patch("a.rs", 1);
        let (mut app, calls) =
            app_with_fake(vec![p.clone()], DiffTarget::WorkingTree, vec![p], vec![]);
        app.apply(Action::CursorDown); // hunk header
        app.apply(Action::CursorDown); // line row
        assert!(matches!(app.view.rows[app.view.cursor], Row::Line(_)));
        app.apply(Action::ToggleStage);
        let StageCall::Apply(patch) = single_call(&calls) else {
            panic!("expected apply_cached");
        };
        assert!(patch.contains("-old0"));
        assert!(patch.contains("+new0"));
    }

    #[test]
    fn space_on_file_header_stages_whole_file() {
        let p = raw_patch("a.rs", 1);
        let (mut app, calls) =
            app_with_fake(vec![p.clone()], DiffTarget::WorkingTree, vec![p], vec![]);
        assert!(matches!(
            app.view.rows[app.view.cursor],
            Row::FileHeader { .. }
        ));
        app.apply(Action::ToggleStage);
        assert_eq!(
            single_call(&calls),
            StageCall::StageFile("a.rs".to_string())
        );
        assert_eq!(app.status_message.as_deref(), Some("staged a.rs"));
    }

    #[test]
    fn space_on_binary_row_stages_whole_file() {
        let raw = "\
diff --git a/img.png b/img.png
index 1..2 100644
Binary files a/img.png and b/img.png differ
";
        let p = RawFilePatch {
            path: "img.png".to_string(),
            old_path: None,
            raw: raw.to_string(),
            is_binary: true,
        };
        let (mut app, calls) =
            app_with_fake(vec![p.clone()], DiffTarget::WorkingTree, vec![p], vec![]);
        app.apply(Action::CursorDown); // Binary row
        assert!(matches!(app.view.rows[app.view.cursor], Row::Binary));
        app.apply(Action::ToggleStage);
        assert_eq!(
            single_call(&calls),
            StageCall::StageFile("img.png".to_string())
        );
    }

    #[test]
    fn space_on_untracked_file_falls_back_to_stage_file_at_any_granularity() {
        // A synthetic untracked file has no raw patch (`patches[i]` is
        // `None`); even a line-row cursor must stage the whole file.
        let calls = Rc::new(RefCell::new(Vec::new()));
        let fake = FakeGit {
            calls: Rc::clone(&calls),
            ..FakeGit::default()
        };
        let snapshot = ReviewSnapshot {
            files: vec![FileDiff::synthetic_added("new.rs".to_string(), "x\ny\n")],
            patches: vec![None],
            staged: Vec::new(),
        };
        let mut app = App::with_git(snapshot, DiffTarget::WorkingTree, Box::new(fake));
        app.apply(Action::CursorDown); // hunk header
        app.apply(Action::CursorDown); // line row
        assert!(matches!(app.view.rows[app.view.cursor], Row::Line(_)));
        app.apply(Action::ToggleStage);
        assert_eq!(
            single_call(&calls),
            StageCall::StageFile("new.rs".to_string())
        );
    }

    #[test]
    fn untracked_visual_selection_falls_back_to_stage_file() {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let fake = FakeGit {
            calls: Rc::clone(&calls),
            ..FakeGit::default()
        };
        let snapshot = ReviewSnapshot {
            files: vec![FileDiff::synthetic_added("new.rs".to_string(), "x\ny\n")],
            patches: vec![None],
            staged: Vec::new(),
        };
        let mut app = App::with_git(snapshot, DiffTarget::WorkingTree, Box::new(fake));
        app.apply(Action::CursorDown);
        app.apply(Action::CursorDown);
        app.apply(Action::EnterVisual);
        app.apply(Action::CursorDown);
        app.apply(Action::ToggleStage);
        assert_eq!(
            single_call(&calls),
            StageCall::StageFile("new.rs".to_string())
        );
    }

    #[test]
    fn staged_target_space_unstages_hunk() {
        let p = raw_patch("a.rs", 1);
        let (mut app, calls) = app_with_fake(vec![p.clone()], DiffTarget::Staged, vec![p], vec![]);
        app.apply(Action::CursorDown); // hunk header
        app.apply(Action::ToggleStage);
        assert!(matches!(single_call(&calls), StageCall::Unapply(_)));
        assert_eq!(app.status_message.as_deref(), Some("unstaged hunk"));
    }

    #[test]
    fn staged_target_file_header_unstages_file() {
        let p = raw_patch("a.rs", 1);
        let (mut app, calls) = app_with_fake(vec![p.clone()], DiffTarget::Staged, vec![p], vec![]);
        app.apply(Action::ToggleStage);
        assert_eq!(
            single_call(&calls),
            StageCall::UnstageFile("a.rs".to_string())
        );
    }

    #[test]
    fn range_target_space_is_noop_with_message() {
        let p = raw_patch("a.rs", 1);
        let (mut app, calls) = app_with_fake(
            vec![p.clone()],
            DiffTarget::Range("main..HEAD".to_string()),
            vec![p],
            vec![],
        );
        let files_before = app.view.files.len();
        app.apply(Action::ToggleStage);
        assert!(calls.borrow().is_empty());
        assert_eq!(app.status_message.as_deref(), Some("read-only diff target"));
        assert_eq!(app.view.files.len(), files_before);
    }

    #[test]
    fn visual_selection_stages_only_selected_lines() {
        let p = raw_patch("a.rs", 1);
        let (mut app, calls) =
            app_with_fake(vec![p.clone()], DiffTarget::WorkingTree, vec![p], vec![]);
        // rows: FileHeader(0) HunkHeader(1) -old0(2) +new0(3)
        app.apply(Action::CursorDown);
        app.apply(Action::CursorDown);
        app.apply(Action::CursorDown); // +new0
        app.apply(Action::EnterVisual); // anchor on +new0 only
        app.apply(Action::ToggleStage);
        let StageCall::Apply(patch) = single_call(&calls) else {
            panic!("expected apply_cached");
        };
        // Selected addition kept; unselected removal downgraded to context.
        assert!(patch.contains("+new0\n"));
        assert!(patch.contains(" old0\n"));
        assert!(!patch.contains("-old0"));
        assert_eq!(app.status_message.as_deref(), Some("staged 1 line"));
        assert_eq!(app.mode, Mode::Normal); // visual exits on success
    }

    #[test]
    fn visual_selection_on_staged_target_unstages_lines() {
        let p = raw_patch("a.rs", 1);
        let (mut app, calls) = app_with_fake(vec![p.clone()], DiffTarget::Staged, vec![p], vec![]);
        app.apply(Action::CursorDown);
        app.apply(Action::CursorDown); // -old0
        app.apply(Action::EnterVisual);
        app.apply(Action::CursorDown); // extend to +new0
        app.apply(Action::ToggleStage);
        let StageCall::Unapply(patch) = single_call(&calls) else {
            panic!("expected unapply_cached");
        };
        assert!(patch.contains("-old0"));
        assert!(patch.contains("+new0"));
        assert_eq!(app.status_message.as_deref(), Some("unstaged 2 lines"));
    }

    #[test]
    fn visual_selection_spanning_multiple_hunks_is_rejected() {
        let p = raw_patch("a.rs", 2);
        let (mut app, calls) =
            app_with_fake(vec![p.clone()], DiffTarget::WorkingTree, vec![p], vec![]);
        // rows: FH(0) HH0(1) -old0(2) +new0(3) HH1(4) -old1(5) +new1(6)
        app.apply(Action::CursorDown);
        app.apply(Action::CursorDown); // -old0
        app.apply(Action::EnterVisual);
        for _ in 0..3 {
            app.apply(Action::CursorDown); // through HH1 into -old1
        }
        app.apply(Action::ToggleStage);
        assert!(calls.borrow().is_empty());
        assert_eq!(
            app.status_message.as_deref(),
            Some("selection spans multiple hunks")
        );
        assert!(matches!(app.mode, Mode::Visual { .. })); // selection kept
    }

    #[test]
    fn visual_selection_with_no_changed_lines_is_rejected() {
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
        let p = RawFilePatch {
            path: "f.rs".to_string(),
            old_path: None,
            raw: raw.to_string(),
            is_binary: false,
        };
        let (mut app, calls) =
            app_with_fake(vec![p.clone()], DiffTarget::WorkingTree, vec![p], vec![]);
        app.apply(Action::CursorDown);
        app.apply(Action::CursorDown); // ctx1
        app.apply(Action::EnterVisual); // select just the context line
        app.apply(Action::ToggleStage);
        assert!(calls.borrow().is_empty());
        assert_eq!(
            app.status_message.as_deref(),
            Some("no changed lines in selection")
        );
    }

    #[test]
    fn refresh_keeps_selected_file_by_path_when_order_changes() {
        let a = raw_patch("a.rs", 1);
        let b = raw_patch("b.rs", 1);
        // After the operation the diff comes back reordered: [b, a].
        let (mut app, _calls) = app_with_fake(
            vec![a.clone(), b.clone()],
            DiffTarget::WorkingTree,
            vec![b, a],
            vec![],
        );
        app.apply(Action::NextFile); // select b.rs (index 1)
        app.apply(Action::ToggleStage); // stage b.rs whole-file, then refresh
        assert_eq!(app.view.files[app.view.selected_file].path, "b.rs");
        assert_eq!(app.view.selected_file, 0); // b.rs moved to index 0
    }

    #[test]
    fn refresh_selects_nearest_file_when_selected_disappears() {
        let a = raw_patch("a.rs", 1);
        let b = raw_patch("b.rs", 1);
        // Staging all of b.rs removes it from the working-tree diff.
        let (mut app, _calls) =
            app_with_fake(vec![a.clone(), b], DiffTarget::WorkingTree, vec![a], vec![]);
        app.apply(Action::NextFile); // select b.rs (index 1)
        app.apply(Action::ToggleStage);
        assert_eq!(app.view.selected_file, 0);
        assert_eq!(app.view.files[app.view.selected_file].path, "a.rs");
        assert!(app.view.cursor <= app.view.rows.len().saturating_sub(1));
    }

    #[test]
    fn refresh_clamps_cursor_when_file_shrinks() {
        let big = raw_patch("a.rs", 3); // 1 + 3*3 = 10 rows
        let small = raw_patch("a.rs", 1); // 4 rows
        let (mut app, _calls) =
            app_with_fake(vec![big], DiffTarget::WorkingTree, vec![small], vec![]);
        for _ in 0..9 {
            app.apply(Action::CursorDown);
        }
        assert_eq!(app.view.cursor, 9);
        app.apply(Action::ToggleStage); // hunk op + refresh to the small diff
        assert!(app.view.cursor < app.view.rows.len());
        assert_eq!(app.view.rows.len(), 4);
    }

    #[test]
    fn refresh_after_empty_diff_resets_cursor_and_selection() {
        let p = raw_patch("a.rs", 1);
        let (mut app, _calls) = app_with_fake(vec![p], DiffTarget::WorkingTree, vec![], vec![]);
        app.apply(Action::CursorDown);
        app.apply(Action::ToggleStage);
        assert!(app.view.files.is_empty());
        assert_eq!(app.view.cursor, 0);
        assert_eq!(app.view.scroll, 0);
        assert_eq!(app.view.selected_file, 0);
    }

    #[test]
    fn refresh_updates_staged_list_and_counts_from_status() {
        let p = raw_patch("a.rs", 1);
        let (mut app, _calls) = app_with_fake(
            vec![p.clone()],
            DiffTarget::WorkingTree,
            vec![p],
            vec![staged_entry("a.rs")],
        );
        assert!(app.staged.is_empty());
        app.apply(Action::ToggleStage); // whole file, then refresh
        assert_eq!(app.staged.len(), 1);
        assert_eq!(app.staged[0].path, "a.rs");
        assert_eq!(app.staged[0].letter, 'M');
    }

    #[test]
    fn stage_error_sets_message_and_leaves_state_unchanged() {
        let p = raw_patch("a.rs", 1);
        let calls = Rc::new(RefCell::new(Vec::new()));
        let fake = FakeGit {
            calls: Rc::clone(&calls),
            // If a refresh ran anyway, files would empty out — the
            // assertion below would catch it.
            diff: vec![],
            fail_ops: true,
            ..FakeGit::default()
        };
        let snapshot = ReviewSnapshot {
            files: vec![FileDiff::from_patch(&p).unwrap()],
            patches: vec![Some(p)],
            staged: Vec::new(),
        };
        let mut app = App::with_git(snapshot, DiffTarget::WorkingTree, Box::new(fake));
        app.apply(Action::CursorDown); // hunk header
        let cursor_before = app.view.cursor;
        app.apply(Action::ToggleStage);
        assert_eq!(app.view.files.len(), 1);
        assert_eq!(app.view.cursor, cursor_before);
        assert!(
            app.status_message
                .as_deref()
                .is_some_and(|m| m.contains("simulated git failure"))
        );
    }

    #[test]
    fn space_without_git_backend_sets_message_only() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.apply(Action::ToggleStage);
        assert_eq!(
            app.status_message.as_deref(),
            Some("staging unavailable (no git backend)")
        );
    }

    #[test]
    fn toggle_stage_in_list_and_compose_modes_is_a_no_op() {
        let p = raw_patch("a.rs", 1);
        let (mut app, calls) =
            app_with_fake(vec![p.clone()], DiffTarget::WorkingTree, vec![p], vec![]);
        app.mode = Mode::List;
        app.apply(Action::ToggleStage);
        app.mode = Mode::Compose;
        app.apply(Action::ToggleStage);
        assert!(calls.borrow().is_empty());
        assert!(app.status_message.is_none());
    }

    #[test]
    fn status_message_set_and_clear() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.set_status_message("staged hunk");
        assert_eq!(app.status_message.as_deref(), Some("staged hunk"));
        app.clear_status_message();
        assert!(app.status_message.is_none());
    }

    // -- Staging panel -------------------------------------------------------

    #[test]
    fn toggle_staging_panel_opens_with_fresh_status_and_closes() {
        let p = raw_patch("a.rs", 1);
        let (mut app, _calls) = app_with_fake(
            vec![p.clone()],
            DiffTarget::WorkingTree,
            vec![p],
            vec![staged_entry("other.rs")],
        );
        app.apply(Action::ToggleStagingPanel);
        assert_eq!(app.mode, Mode::Staging);
        assert_eq!(app.staged.len(), 1); // re-read from status on open
        app.close_staging();
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn staging_panel_navigation_clamps() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.staged = vec![
            StagedFile {
                path: "a.rs".to_string(),
                letter: 'M',
            },
            StagedFile {
                path: "b.rs".to_string(),
                letter: 'A',
            },
        ];
        app.staging_move_down();
        assert_eq!(app.staging_cursor, 1);
        app.staging_move_down();
        assert_eq!(app.staging_cursor, 1); // clamped at last
        app.staging_move_up();
        assert_eq!(app.staging_cursor, 0);
        app.staging_move_up();
        assert_eq!(app.staging_cursor, 0); // clamped at first
    }

    #[test]
    fn staging_panel_unstage_keeps_panel_open_and_clamps_cursor() {
        let p = raw_patch("a.rs", 1);
        // Post-refresh status: only one staged file remains.
        let (mut app, calls) = app_with_fake(
            vec![p.clone()],
            DiffTarget::WorkingTree,
            vec![p],
            vec![staged_entry("a.rs")],
        );
        app.staged = vec![staged_entry_file("a.rs"), staged_entry_file("b.rs")];
        app.mode = Mode::Staging;
        app.staging_cursor = 1; // focus b.rs
        app.unstage_focused_file();
        assert_eq!(
            single_call(&calls),
            StageCall::UnstageFile("b.rs".to_string())
        );
        assert_eq!(app.mode, Mode::Staging); // panel stays open
        assert_eq!(app.staged.len(), 1); // refreshed list
        assert_eq!(app.staging_cursor, 0); // clamped into range
        assert_eq!(app.status_message.as_deref(), Some("unstaged b.rs"));
    }

    fn staged_entry_file(path: &str) -> StagedFile {
        StagedFile {
            path: path.to_string(),
            letter: 'M',
        }
    }

    #[test]
    fn staging_panel_unstage_on_empty_list_is_a_no_op() {
        let p = raw_patch("a.rs", 1);
        let (mut app, calls) =
            app_with_fake(vec![p.clone()], DiffTarget::WorkingTree, vec![p], vec![]);
        app.mode = Mode::Staging;
        app.unstage_focused_file();
        assert!(calls.borrow().is_empty());
        assert_eq!(app.mode, Mode::Staging);
    }

    #[test]
    fn visual_space_allows_staging_but_navigation_stays_disabled() {
        let p = raw_patch("a.rs", 1);
        let (mut app, calls) =
            app_with_fake(vec![p.clone()], DiffTarget::WorkingTree, vec![p], vec![]);
        app.apply(Action::CursorDown);
        app.apply(Action::CursorDown); // line row
        app.apply(Action::EnterVisual);
        app.apply(Action::ToggleStage);
        assert_eq!(calls.borrow().len(), 1);
    }

    // -- Syntax highlight cache -----------------------------------------------

    fn highlight_patch(path: &str) -> RawFilePatch {
        RawFilePatch {
            path: path.to_string(),
            old_path: None,
            raw: format!(
                "diff --git a/{path} b/{path}\nindex 1..2 100644\n--- a/{path}\n+++ b/{path}\n@@ -1,1 +1,1 @@\n-fn old() {{}}\n+fn new() {{}}\n"
            ),
            is_binary: false,
        }
    }

    /// Selecting a file's rows highlights each side at most once, even
    /// across repeated selection (file switches back and forth) — the
    /// `HighlightCache` keyed by `(path, side)` must be hit, not re-fetched
    /// and re-highlighted, on every rebuild.
    #[test]
    fn selecting_the_same_file_repeatedly_highlights_each_side_once() {
        let a = highlight_patch("a.rs");
        let b = highlight_patch("b.rs");
        let show_calls = Rc::new(RefCell::new(0));
        let fake = FakeGit {
            diff: vec![a.clone(), b.clone()],
            show_calls: Rc::clone(&show_calls),
            show_content: Some("fn old() {}\nfn new() {}\n".to_string()),
            ..FakeGit::default()
        };
        let snapshot = ReviewSnapshot {
            files: vec![
                FileDiff::from_patch(&a).unwrap(),
                FileDiff::from_patch(&b).unwrap(),
            ],
            patches: vec![Some(a), Some(b)],
            staged: Vec::new(),
        };
        let mut app = App::with_git(snapshot, DiffTarget::Staged, Box::new(fake));
        // `with_git`'s initial rebuild already highlighted a.rs's two sides
        // (it has both an added and a removed line).
        assert_eq!(*show_calls.borrow(), 2);

        app.apply(Action::NextFile); // -> b.rs: two fresh fetches
        assert_eq!(*show_calls.borrow(), 4);
        app.apply(Action::PrevFile); // -> a.rs: cache hit, no new fetches
        assert_eq!(*show_calls.borrow(), 4);
        app.apply(Action::NextFile); // -> b.rs: cache hit again
        assert_eq!(*show_calls.borrow(), 4);
    }

    // -- Search ----------------------------------------------------------------

    fn search_file() -> FileDiff {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,3 +1,3 @@ fn foo() {
 alpha
-beta
+gamma
 delta
";
        file_with_raw("f.rs", raw)
    }

    #[test]
    fn slash_opens_search_mode_with_empty_buffer() {
        let mut app = App::new(vec![search_file()]);
        app.apply(Action::Search);
        assert_eq!(app.mode, Mode::Search);
        assert_eq!(app.search_input, "");
    }

    #[test]
    fn slash_is_disabled_in_visual_mode() {
        let mut app = App::new(vec![search_file()]);
        app.apply(Action::CursorDown); // hunk header
        app.apply(Action::CursorDown); // line row
        app.apply(Action::EnterVisual);
        app.apply(Action::Search);
        assert!(matches!(app.mode, Mode::Visual { .. }));
    }

    #[test]
    fn confirm_search_jumps_to_first_match_at_or_after_cursor() {
        let mut app = App::new(vec![search_file()]);
        app.apply(Action::Search);
        for c in "gamma".chars() {
            app.search_input.push(c);
        }
        app.confirm_search();
        assert_eq!(app.mode, Mode::Normal);
        let Row::Line(line) = &app.view.rows[app.view.cursor] else {
            panic!("expected cursor on a line row");
        };
        assert_eq!(line.content, "gamma");
        assert_eq!(app.status_message.as_deref(), Some("match 1/1"));
    }

    #[test]
    fn confirm_search_with_no_matches_sets_message_but_keeps_pattern() {
        let mut app = App::new(vec![search_file()]);
        app.apply(Action::Search);
        for c in "zzz".chars() {
            app.search_input.push(c);
        }
        app.confirm_search();
        assert_eq!(app.status_message.as_deref(), Some("no matches"));
        assert_eq!(app.search.pattern.as_deref(), Some("zzz"));
    }

    #[test]
    fn confirm_search_with_empty_buffer_clears_active_pattern() {
        let mut app = App::new(vec![search_file()]);
        app.search.pattern = Some("gamma".to_string());
        app.search.matches = vec![4];
        app.apply(Action::Search); // buffer starts empty
        app.confirm_search();
        assert_eq!(app.search.pattern, None);
        assert!(app.search.matches.is_empty());
    }

    #[test]
    fn esc_with_nonempty_buffer_cancels_without_clearing_active_pattern() {
        let mut app = App::new(vec![search_file()]);
        app.apply(Action::Search);
        for c in "gamma".chars() {
            app.search_input.push(c);
        }
        app.confirm_search(); // active pattern is now "gamma"
        app.apply(Action::Search); // reopen with a fresh buffer
        app.search_input.push('x');
        app.cancel_search();
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.search.pattern.as_deref(), Some("gamma"));
    }

    #[test]
    fn esc_with_empty_buffer_clears_active_pattern() {
        let mut app = App::new(vec![search_file()]);
        app.search.pattern = Some("gamma".to_string());
        app.search.matches = vec![4];
        app.apply(Action::Search);
        app.cancel_search();
        assert_eq!(app.search.pattern, None);
        assert!(app.search.matches.is_empty());
    }

    #[test]
    fn search_next_and_prev_wrap_around_both_directions() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,3 +1,3 @@
 foo
 foo
 foo
";
        let mut app = App::new(vec![file_with_raw("f.rs", raw)]);
        app.apply(Action::Search);
        for c in "foo".chars() {
            app.search_input.push(c);
        }
        app.confirm_search();
        let first = app.view.cursor;

        app.apply(Action::SearchNext);
        let second = app.view.cursor;
        assert_ne!(first, second);

        app.apply(Action::SearchNext);
        let third = app.view.cursor;
        assert_ne!(second, third);

        app.apply(Action::SearchNext); // wraps forward back to the first match
        assert_eq!(app.view.cursor, first);

        app.apply(Action::SearchPrev); // wraps backward to the last match
        assert_eq!(app.view.cursor, third);
    }

    #[test]
    fn search_next_without_active_pattern_sets_message() {
        let mut app = App::new(vec![search_file()]);
        app.apply(Action::SearchNext);
        assert_eq!(app.status_message.as_deref(), Some("no search pattern"));
    }

    #[test]
    fn search_next_with_zero_matches_sets_message() {
        let mut app = App::new(vec![search_file()]);
        app.apply(Action::Search);
        for c in "zzz".chars() {
            app.search_input.push(c);
        }
        app.confirm_search();
        app.apply(Action::SearchNext);
        assert_eq!(app.status_message.as_deref(), Some("no matches"));
    }

    #[test]
    fn search_matches_are_smartcase() {
        let mut app = App::new(vec![search_file()]);
        app.apply(Action::Search);
        for c in "GAMMA".chars() {
            app.search_input.push(c);
        }
        app.confirm_search();
        // "gamma" (lowercase in the file) does not match the uppercase,
        // case-sensitive pattern "GAMMA".
        assert_eq!(app.status_message.as_deref(), Some("no matches"));
    }

    #[test]
    fn hunk_header_section_text_is_searchable() {
        let mut app = App::new(vec![search_file()]);
        app.apply(Action::Search);
        for c in "foo".chars() {
            app.search_input.push(c);
        }
        app.confirm_search();
        assert!(matches!(
            app.view.rows[app.view.cursor],
            Row::HunkHeader { .. }
        ));
    }

    #[test]
    fn search_pattern_survives_row_rebuild() {
        let mut app = App::new(vec![search_file()]);
        app.apply(Action::Search);
        for c in "gamma".chars() {
            app.search_input.push(c);
        }
        app.confirm_search();
        assert_eq!(app.search.matches.len(), 1);

        // Adding an annotation triggers `refresh_rows` -> `rebuild_rows`,
        // which must recompute matches against the rebuilt rows rather
        // than dropping the active pattern.
        app.apply(Action::Compose);
        for c in "note".chars() {
            app.compose.as_mut().unwrap().buffer.insert_char(c);
        }
        app.submit_compose();

        assert_eq!(app.search.pattern.as_deref(), Some("gamma"));
        assert_eq!(app.search.matches.len(), 1);
    }

    // -- Column cursor ---------------------------------------------------------

    #[test]
    fn h_and_l_move_column_within_bounds() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,1 +1,1 @@
 abcde
";
        let mut app = App::new(vec![file_with_raw("f.rs", raw)]);
        app.apply(Action::CursorDown); // hunk header
        app.apply(Action::CursorDown); // line "abcde"
        assert_eq!(app.view.effective_column(), Some(0));
        app.apply(Action::CursorRight);
        app.apply(Action::CursorRight);
        assert_eq!(app.view.effective_column(), Some(2));
        for _ in 0..10 {
            app.apply(Action::CursorRight);
        }
        assert_eq!(app.view.effective_column(), Some(4)); // clamped at last char
        for _ in 0..10 {
            app.apply(Action::CursorLeft);
        }
        assert_eq!(app.view.effective_column(), Some(0));
    }

    #[test]
    fn column_is_hidden_on_header_rows() {
        let app = App::new(vec![file("a.rs", 1)]);
        assert!(matches!(app.view.rows[0], Row::FileHeader { .. }));
        assert_eq!(app.view.effective_column(), None);
    }

    #[test]
    fn w_and_b_jump_between_words() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,1 +1,1 @@
 foo bar_baz  qux
";
        let mut app = App::new(vec![file_with_raw("f.rs", raw)]);
        app.apply(Action::CursorDown);
        app.apply(Action::CursorDown);
        assert_eq!(app.view.effective_column(), Some(0)); // 'f' of foo
        app.apply(Action::WordForward);
        assert_eq!(app.view.effective_column(), Some(4)); // 'b' of bar_baz (word = alnum/_)
        app.apply(Action::WordForward);
        assert_eq!(app.view.effective_column(), Some(13)); // 'q' of qux
        app.apply(Action::WordBackward);
        assert_eq!(app.view.effective_column(), Some(4));
        app.apply(Action::WordBackward);
        assert_eq!(app.view.effective_column(), Some(0));
    }

    #[test]
    fn column_clamps_when_switching_to_a_shorter_row() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,2 +1,2 @@
 a really long line here
 x
";
        let long_line_last_col = "a really long line here".chars().count() - 1;
        let mut app = App::new(vec![file_with_raw("f.rs", raw)]);
        app.apply(Action::CursorDown); // hunk header
        app.apply(Action::CursorDown); // long line
        for _ in 0..40 {
            app.apply(Action::CursorRight);
        }
        assert_eq!(app.view.effective_column(), Some(long_line_last_col));
        app.apply(Action::CursorDown); // short line "x"
        assert_eq!(app.view.effective_column(), Some(0));
    }

    #[test]
    fn column_motion_is_a_noop_off_a_line_row() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.apply(Action::CursorRight);
        app.apply(Action::WordForward);
        assert_eq!(app.view.effective_column(), None);
    }

    // -- UTF-16 offset conversion (for LSP position derivation) ----------------

    #[test]
    fn utf16_offset_ascii_matches_char_index() {
        assert_eq!(utf16_offset("hello", 3), 3);
    }

    #[test]
    fn utf16_offset_multibyte_bmp_char_counts_as_one_unit() {
        // 'é' is 2 bytes in UTF-8 but a single UTF-16 code unit.
        assert_eq!(utf16_offset("café", 4), 4);
    }

    #[test]
    fn utf16_offset_surrogate_pair_counts_as_two_units() {
        // An emoji outside the BMP is one `char` but 2 UTF-16 code units.
        let content = "a\u{1F600}b";
        assert_eq!(utf16_offset(content, 0), 0); // before 'a'
        assert_eq!(utf16_offset(content, 1), 1); // before the emoji
        assert_eq!(utf16_offset(content, 2), 3); // after the emoji (1 + 2)
    }

    // -- LSP: gd/gr/K request routing and event handling ------------------------

    #[derive(Debug, Clone, PartialEq)]
    enum LspCall {
        Definition(PathBuf, u32, u32),
        References(PathBuf, u32, u32),
        Hover(PathBuf, u32, u32),
    }

    struct FakeLsp {
        calls: Rc<RefCell<Vec<LspCall>>>,
        next_id: u64,
        deny: bool,
        poll_queue: Rc<RefCell<std::collections::VecDeque<Vec<LspEvent>>>>,
        shutdown_called: Rc<RefCell<bool>>,
    }

    impl FakeLsp {
        fn record(&mut self, call: LspCall) -> Option<RequestId> {
            if self.deny {
                return None;
            }
            self.next_id += 1;
            self.calls.borrow_mut().push(call);
            Some(RequestId(self.next_id))
        }
    }

    impl LspClient for FakeLsp {
        fn request_definition(
            &mut self,
            path: &std::path::Path,
            line: u32,
            character: u32,
        ) -> Option<RequestId> {
            self.record(LspCall::Definition(path.to_path_buf(), line, character))
        }

        fn request_references(
            &mut self,
            path: &std::path::Path,
            line: u32,
            character: u32,
        ) -> Option<RequestId> {
            self.record(LspCall::References(path.to_path_buf(), line, character))
        }

        fn request_hover(
            &mut self,
            path: &std::path::Path,
            line: u32,
            character: u32,
        ) -> Option<RequestId> {
            self.record(LspCall::Hover(path.to_path_buf(), line, character))
        }

        fn poll(&mut self) -> Vec<LspEvent> {
            self.poll_queue.borrow_mut().pop_front().unwrap_or_default()
        }

        fn shutdown(self: Box<Self>) {
            *self.shutdown_called.borrow_mut() = true;
        }
    }

    /// A diff over `path` with rows: FileHeader(0) HunkHeader(1)
    /// context "fn main() {" new_line=1 (2) removed "    old();" (3) added
    /// "    new();" new_line=2 (4).
    fn lsp_fixture_raw() -> &'static str {
        "\
diff --git a/src/main.rs b/src/main.rs
index 1..2 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,2 +1,2 @@
 fn main() {
-    old();
+    new();
"
    }

    /// An `App` over `src/main.rs`, whose path also exists as a real file
    /// under a fresh tempdir (`gd`/`gr`/`K` check the file exists on disk),
    /// wired to a `FakeLsp` via `inject_lsp_client`. Returns the app, the
    /// tempdir (kept alive so the file keeps existing), and handles to
    /// inspect issued calls / feed scripted `poll()` responses.
    #[allow(clippy::type_complexity)]
    fn lsp_test_app() -> (
        App,
        tempfile::TempDir,
        Rc<RefCell<Vec<LspCall>>>,
        Rc<RefCell<std::collections::VecDeque<Vec<LspEvent>>>>,
    ) {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join("src")).expect("mkdir");
        std::fs::write(
            tmp.path().join("src/main.rs"),
            "fn main() {\n    new();\n}\n",
        )
        .expect("write fixture");

        let mut app = App::new(vec![file_with_raw("src/main.rs", lsp_fixture_raw())]);
        let calls = Rc::new(RefCell::new(Vec::new()));
        let poll_queue = Rc::new(RefCell::new(std::collections::VecDeque::new()));
        let fake = FakeLsp {
            calls: Rc::clone(&calls),
            next_id: 0,
            deny: false,
            poll_queue: Rc::clone(&poll_queue),
            shutdown_called: Rc::new(RefCell::new(false)),
        };
        app.inject_lsp_client(Box::new(fake), tmp.path().to_path_buf());
        (app, tmp, calls, poll_queue)
    }

    /// Moves the cursor onto the fixture's added line (`    new();`,
    /// new_line 2) — the only row `code_intel_position` accepts.
    fn move_to_added_line(app: &mut App) {
        for _ in 0..4 {
            app.apply(Action::CursorDown);
        }
    }

    #[test]
    fn gd_on_removed_line_sets_no_code_intelligence_message() {
        let (mut app, _tmp, calls, _poll) = lsp_test_app();
        app.apply(Action::CursorDown); // hunk header
        app.apply(Action::CursorDown); // context line
        app.apply(Action::CursorDown); // removed line
        assert!(matches!(
            app.view.rows[app.view.cursor],
            Row::Line(LineRow {
                origin: LineOrigin::Removed,
                ..
            })
        ));
        app.apply(Action::GotoDefinition);
        assert!(calls.borrow().is_empty());
        assert_eq!(
            app.status_message.as_deref(),
            Some("no code intelligence here")
        );
    }

    #[test]
    fn gd_on_header_row_sets_no_code_intelligence_message() {
        let (mut app, _tmp, calls, _poll) = lsp_test_app();
        assert!(matches!(
            app.view.rows[app.view.cursor],
            Row::FileHeader { .. }
        ));
        app.apply(Action::GotoDefinition);
        assert!(calls.borrow().is_empty());
        assert_eq!(
            app.status_message.as_deref(),
            Some("no code intelligence here")
        );
    }

    #[test]
    fn gd_on_missing_file_sets_no_code_intelligence_message() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        // Deliberately no file written under `tmp` at "src/main.rs".
        let mut app = App::new(vec![file_with_raw("src/main.rs", lsp_fixture_raw())]);
        let calls = Rc::new(RefCell::new(Vec::new()));
        let fake = FakeLsp {
            calls: Rc::clone(&calls),
            next_id: 0,
            deny: false,
            poll_queue: Rc::new(RefCell::new(std::collections::VecDeque::new())),
            shutdown_called: Rc::new(RefCell::new(false)),
        };
        app.inject_lsp_client(Box::new(fake), tmp.path().to_path_buf());
        move_to_added_line(&mut app);
        app.apply(Action::GotoDefinition);
        assert!(calls.borrow().is_empty());
        assert_eq!(
            app.status_message.as_deref(),
            Some("no code intelligence here")
        );
    }

    #[test]
    fn gd_without_repo_root_sets_no_code_intelligence_message() {
        let mut app = App::new(vec![file_with_raw("src/main.rs", lsp_fixture_raw())]);
        move_to_added_line(&mut app);
        app.apply(Action::GotoDefinition);
        assert_eq!(
            app.status_message.as_deref(),
            Some("no code intelligence here")
        );
    }

    #[test]
    fn gd_on_valid_row_dispatches_request_and_sets_resolving_message() {
        let (mut app, tmp, calls, _poll) = lsp_test_app();
        move_to_added_line(&mut app);
        app.apply(Action::GotoDefinition);
        assert_eq!(
            calls.borrow()[0],
            LspCall::Definition(tmp.path().join("src/main.rs"), 1, 0)
        );
        assert_eq!(
            app.status_message.as_deref(),
            Some("lsp: resolving\u{2026}")
        );
    }

    /// `gd`'s LSP position is derived from `rows[cursor]`/the column
    /// cursor, exactly like every other target-derivation gesture — toggling
    /// side-by-side must not change the requested position.
    #[test]
    fn gd_position_is_identical_in_both_views() {
        let (mut app, tmp, calls, _poll) = lsp_test_app();
        move_to_added_line(&mut app);
        app.apply(Action::ToggleView);
        app.apply(Action::GotoDefinition);
        assert_eq!(
            calls.borrow()[0],
            LspCall::Definition(tmp.path().join("src/main.rs"), 1, 0)
        );
    }

    #[test]
    fn gr_and_k_dispatch_their_own_request_kinds() {
        let (mut app, tmp, calls, _poll) = lsp_test_app();
        move_to_added_line(&mut app);
        app.apply(Action::GotoReferences);
        assert_eq!(
            calls.borrow()[0],
            LspCall::References(tmp.path().join("src/main.rs"), 1, 0)
        );
        app.apply(Action::Hover);
        assert_eq!(
            calls.borrow()[1],
            LspCall::Hover(tmp.path().join("src/main.rs"), 1, 0)
        );
    }

    #[test]
    fn gd_uses_the_column_cursor_for_the_character_offset() {
        let (mut app, tmp, calls, _poll) = lsp_test_app();
        move_to_added_line(&mut app); // "    new();" -- col 4 is 'n'
        for _ in 0..4 {
            app.apply(Action::CursorRight);
        }
        app.apply(Action::GotoDefinition);
        assert_eq!(
            calls.borrow()[0],
            LspCall::Definition(tmp.path().join("src/main.rs"), 1, 4)
        );
    }

    #[test]
    fn second_request_supersedes_interest_in_the_first_pending_id() {
        let (mut app, _tmp, _calls, poll_queue) = lsp_test_app();
        move_to_added_line(&mut app);

        app.apply(Action::GotoDefinition);
        let first_id = app.pending_lsp.expect("pending after gd").0;
        app.apply(Action::GotoReferences);
        let second_id = app.pending_lsp.expect("pending after gr").0;
        assert_ne!(first_id, second_id);

        // A response for the superseded first id must be ignored.
        poll_queue
            .borrow_mut()
            .push_back(vec![LspEvent::Definition {
                id: first_id,
                locations: vec![SourceLocation {
                    path: PathBuf::from("/tmp/unused.rs"),
                    line: 0,
                    character: 0,
                }],
            }]);
        app.poll_lsp();
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.peek.is_none());

        // The second (References) request's own response still opens the
        // overlay.
        poll_queue
            .borrow_mut()
            .push_back(vec![LspEvent::References {
                id: second_id,
                locations: vec![SourceLocation {
                    path: PathBuf::from("/tmp/unused.rs"),
                    line: 0,
                    character: 0,
                }],
            }]);
        app.poll_lsp();
        assert_eq!(app.mode, Mode::Peek);
    }

    #[test]
    fn unrelated_event_id_is_ignored() {
        let (mut app, _tmp, _calls, poll_queue) = lsp_test_app();
        move_to_added_line(&mut app);
        app.apply(Action::GotoDefinition);
        let real_id = app.pending_lsp.expect("pending after gd").0;

        poll_queue
            .borrow_mut()
            .push_back(vec![LspEvent::Definition {
                id: RequestId(real_id.0 + 999),
                locations: vec![SourceLocation {
                    path: PathBuf::from("/tmp/unused.rs"),
                    line: 0,
                    character: 0,
                }],
            }]);
        app.poll_lsp();
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.pending_lsp.map(|(id, _)| id), Some(real_id));
    }

    #[test]
    fn empty_definition_result_sets_no_results_message() {
        let (mut app, _tmp, _calls, poll_queue) = lsp_test_app();
        move_to_added_line(&mut app);
        app.apply(Action::GotoDefinition);
        let id = app.pending_lsp.expect("pending after gd").0;

        poll_queue
            .borrow_mut()
            .push_back(vec![LspEvent::Definition {
                id,
                locations: vec![],
            }]);
        app.poll_lsp();
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.status_message.as_deref(), Some("no results"));
    }

    #[test]
    fn failed_event_sets_footer_message_and_does_not_open_overlay() {
        let (mut app, _tmp, _calls, poll_queue) = lsp_test_app();
        move_to_added_line(&mut app);
        app.apply(Action::Hover);
        let id = app.pending_lsp.expect("pending after K").0;

        poll_queue
            .borrow_mut()
            .push_back(vec![LspEvent::Failed { id }]);
        app.poll_lsp();
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.status_message.as_deref(), Some("lsp: failed"));
    }

    #[test]
    fn hover_event_opens_peek_overlay_with_contents() {
        let (mut app, _tmp, _calls, poll_queue) = lsp_test_app();
        move_to_added_line(&mut app);
        app.apply(Action::Hover);
        let id = app.pending_lsp.expect("pending after K").0;

        poll_queue.borrow_mut().push_back(vec![LspEvent::Hover {
            id,
            contents: "some docs".to_string(),
        }]);
        app.poll_lsp();
        assert_eq!(app.mode, Mode::Peek);
        assert_eq!(app.peek.as_ref().unwrap().hover_text, "some docs");
    }

    #[test]
    fn take_lsp_client_returns_the_injected_client_once() {
        let (mut app, _tmp, _calls, _poll) = lsp_test_app();
        assert!(app.take_lsp_client().is_some());
        assert!(app.take_lsp_client().is_none());
    }

    // -- Peek overlay -------------------------------------------------------

    fn source_loc(path: &std::path::Path, line: u32) -> SourceLocation {
        SourceLocation {
            path: path.to_path_buf(),
            line,
            character: 0,
        }
    }

    #[test]
    fn peek_move_down_and_up_clamp_selection() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.peek = Some(PeekState::locations(
            PeekKind::References,
            vec![
                source_loc(std::path::Path::new("/tmp/a.rs"), 0),
                source_loc(std::path::Path::new("/tmp/b.rs"), 1),
            ],
        ));
        app.mode = Mode::Peek;

        app.peek_move_down();
        assert_eq!(app.peek.as_ref().unwrap().selected, 1);
        app.peek_move_down(); // clamped at last
        assert_eq!(app.peek.as_ref().unwrap().selected, 1);
        app.peek_move_up();
        assert_eq!(app.peek.as_ref().unwrap().selected, 0);
        app.peek_move_up(); // clamped at first
        assert_eq!(app.peek.as_ref().unwrap().selected, 0);
    }

    #[test]
    fn hover_scroll_moves_and_clamps() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.peek = Some(PeekState::hover("one\ntwo\nthree".to_string()));
        app.mode = Mode::Peek;

        app.peek_move_down();
        assert_eq!(app.peek.as_ref().unwrap().hover_scroll, 1);
        for _ in 0..5 {
            app.peek_move_down();
        }
        assert_eq!(app.peek.as_ref().unwrap().hover_scroll, 2); // clamped
        app.peek_move_up();
        assert_eq!(app.peek.as_ref().unwrap().hover_scroll, 1);
        app.peek_move_up();
        app.peek_move_up();
        assert_eq!(app.peek.as_ref().unwrap().hover_scroll, 0); // clamped at 0
    }

    #[test]
    fn close_peek_returns_to_normal() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.peek = Some(PeekState::hover("x".to_string()));
        app.mode = Mode::Peek;
        app.close_peek();
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.peek.is_none());
    }

    #[test]
    fn peek_enter_jumps_into_diff_when_path_matches_a_diff_file() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let mut app = App::new(vec![file_with_raw("src/main.rs", lsp_fixture_raw())]);
        app.set_repo_root(tmp.path().to_path_buf());
        app.peek = Some(PeekState::locations(
            PeekKind::Definition,
            vec![source_loc(&tmp.path().join("src/main.rs"), 1)], // 0-based -> new_line 2
        ));
        app.mode = Mode::Peek;

        app.peek_enter();

        assert_eq!(app.mode, Mode::Normal);
        assert!(app.peek.is_none());
        let Row::Line(line) = &app.view.rows[app.view.cursor] else {
            panic!("expected cursor on a line row");
        };
        assert_eq!(line.new_line, Some(2));
    }

    #[test]
    fn peek_enter_on_unrelated_path_shows_not_in_diff_and_stays_open() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let mut app = App::new(vec![file_with_raw("src/main.rs", lsp_fixture_raw())]);
        app.set_repo_root(tmp.path().to_path_buf());
        app.peek = Some(PeekState::locations(
            PeekKind::Definition,
            vec![source_loc(&tmp.path().join("other.rs"), 0)],
        ));
        app.mode = Mode::Peek;

        app.peek_enter();

        assert_eq!(app.mode, Mode::Peek);
        assert_eq!(app.status_message.as_deref(), Some("not in diff"));
    }

    #[test]
    fn peek_enter_is_a_noop_for_hover() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.peek = Some(PeekState::hover("docs".to_string()));
        app.mode = Mode::Peek;
        app.peek_enter();
        assert_eq!(app.mode, Mode::Peek);
        assert!(app.peek.is_some());
    }
}
