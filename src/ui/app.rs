//! [`App`]: the TUI's state and the pure state transitions every [`Action`]
//! performs. No rendering or terminal I/O lives here — these are plain
//! methods, unit-tested without a terminal.

use std::path::PathBuf;

use crate::annotate::{AnnotationStore, Side, Target};
use crate::diff::{FileDiff, LineOrigin};
use crate::git::{DiffTarget, RawFilePatch};
use crate::highlight::Highlighter;
use crate::lsp::RequestId;

use super::compose::ComposeState;
use super::diff_view_state::DiffViewState;
use super::keymap::Action;
use super::lsp_ops::LspClient;
use super::peek::{PeekKind, PeekState};
use super::rows::{LineRow, Row, SyntaxSpans, anchor_row_index, build_rows, hunk_span};
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
    /// config cache persists across selections. `pub(super)` for the
    /// code-intelligence module's peek-preview highlighting.
    pub(super) highlighter: Highlighter,
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
    /// against `repo_root`. `None` until then. `pub(super)` for the
    /// code-intelligence module.
    pub(super) lsp: Option<Box<dyn LspClient>>,
    /// The request id + kind `gd`/`gr`/`K` is currently awaiting a
    /// response for. A new request overwrites this (cancelling interest in
    /// whatever was pending before); an [`crate::lsp::LspEvent`] whose id
    /// doesn't match is ignored. `pub(super)` for the code-intelligence
    /// module.
    pub(super) pending_lsp: Option<(RequestId, PeekKind)>,
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
    pub(super) fn inject_lsp_client(&mut self, client: Box<dyn LspClient>, root: PathBuf) {
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
            Action::EnterVisual => self.toggle_visual(),
            Action::Compose => self.open_compose(),
            Action::ToggleList => self.toggle_list(),
            Action::ToggleStage => super::staging::toggle_stage(self),
            Action::ToggleStagingPanel => self.toggle_staging_panel(),
            Action::Search => self.enter_search(),
            Action::SearchNext => self.search_advance(true),
            Action::SearchPrev => self.search_advance(false),
            Action::GotoDefinition => super::code_intel::request(self, PeekKind::Definition),
            Action::GotoReferences => super::code_intel::request(self, PeekKind::References),
            Action::Hover => super::code_intel::request(self, PeekKind::Hover),
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
    pub(super) fn rebuild_rows(&mut self) {
        let Some(file) = self.view.files.get(self.view.selected_file) else {
            self.view.rows = Vec::new();
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
        self.view.rows = rows;
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

    /// The staging backend, if one is attached, borrowed as a trait object
    /// for the UI-side staging module. `None` in git-less contexts.
    pub(super) fn stage_ops(&self) -> Option<&dyn StageOps> {
        self.stage_ops.as_deref()
    }

    /// Re-runs the diff and status for the current target, rebuilds
    /// files/patches/rows and the staged list, then restores position: the
    /// previously selected file is kept by path if it still exists (else
    /// the index is clamped to the nearest remaining file), and cursor,
    /// scroll, and the staging-panel cursor are clamped into range. On any
    /// git/parse error the state is left unchanged and a footer message is
    /// set. A no-op without a git backend.
    pub(super) fn refresh(&mut self) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotate::Classification;
    use crate::git::RawFilePatch;
    use crate::ui::compose::TextBuffer;

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
}
