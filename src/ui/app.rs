//! [`App`]: the TUI's state and the pure state transitions every [`Action`]
//! performs. No rendering or terminal I/O lives here — these are plain
//! methods, unit-tested without a terminal.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::annotate::{AnnotationStore, Side, Target};
use crate::diff::{FileDiff, LineOrigin};
use crate::git::{
    BranchStatus, CommitSummary, DiffTarget, RawFilePatch, RemoteOp, StashEntry, remote_command,
};
use crate::highlight::Highlighter;
use crate::lsp::RequestId;

use super::background::{BackgroundTasks, CommandOutcome, TaskId, run_command};
use super::command_log::{CommandLog, CommandLogEntry};
use super::compose::ComposeState;
use super::diff_view_state::DiffViewState;
use super::keymap::Action;
use super::lsp_ops::LspClient;
use super::peek::{PeekKind, PeekState};
use super::rows::{LineRow, Row, StagedMarker, SyntaxSpans, build_multibuffer, hunk_span};
use super::search::SearchState;
use super::stage_ops::{
    ReviewError, ReviewSnapshot, StageOps, StagedFile, StagedState, build_review,
    staged_from_status, staged_states_from_status,
};
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
    /// The git panel (sidebar) holds focus: its cursor navigates the
    /// CHANGES/UNTRACKED/STASHES sections, bypassing the diff-scope keymap.
    Panel,
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
    /// The help overlay's vertical scroll offset (top visible content line).
    /// The key handler advances it freely; [`super::help::render`] clamps it
    /// to the content/viewport and writes the clamped value back, so state
    /// and view never disagree. Reset to 0 whenever the overlay toggles.
    pub(super) help_scroll: Cell<u16>,
    /// The help overlay's scrollable-region height, set by
    /// [`super::help::render`] each frame so the key handler can page by a
    /// real viewport (PageUp/PageDown) rather than a guessed constant.
    pub(super) help_viewport: Cell<u16>,
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
    /// Current branch / upstream / ahead-behind state, read at startup and
    /// on every [`App::refresh`]. `None` in git-less contexts, or until the
    /// first successful read.
    pub branch: Option<BranchStatus>,
    /// The stash list (newest first) as of the latest refresh; empty in
    /// git-less contexts or when nothing is stashed.
    pub stashes: Vec<StashEntry>,
    /// A one-line summary of the tip commit (`HEAD`), read at startup and on
    /// every [`App::refresh`], shown in the git panel's bottom section.
    /// `None` in git-less contexts, or in a repository with no commits yet.
    pub last_commit: Option<CommitSummary>,
    /// Repo-relative paths of untracked files among `view.files`, used by
    /// the git panel to split its CHANGES and UNTRACKED sections. Derived on
    /// refresh from which entries have no real patch; empty without git.
    pub untracked_paths: Vec<String>,
    /// Per-path [`StagedState`] driving the `●`/`±` section-header and git
    /// panel markers, refreshed alongside `staged`. Missing entries are
    /// [`StagedState::Unstaged`].
    pub staged_states: HashMap<String, StagedState>,
    /// The focused row index into `staged` in the staging panel.
    pub staging_cursor: usize,
    /// The git panel's cursor: an index into the flattened list of navigable
    /// panel rows (CHANGES + UNTRACKED files, then STASHES; section headers
    /// are skipped). Only meaningful while `mode == Mode::Panel`; clamped on
    /// render so a stale index never points past the list.
    pub panel_cursor: usize,
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
    /// The background-task poller remote operations run through. Spawning
    /// returns immediately; [`App::poll_remote`] drains completed outcomes
    /// once per event-loop tick.
    pub(super) background: BackgroundTasks<CommandOutcome>,
    /// The in-memory, bounded log of every git command redquill ran, rendered
    /// in the toggleable command-log pane.
    pub(super) command_log: CommandLog,
    /// The single remote operation currently in flight, if any. Enforces the
    /// "at most one remote op at a time" guard: while this is `Some`, further
    /// remote requests are rejected with a message rather than queued.
    pub(super) remote_op: Option<InFlightRemote>,
    /// Whether the command-log pane is open in the bottom-panel slot. Toggled
    /// with `@` from both the diff view and the focused panel.
    pub(super) command_log_open: bool,
}

/// A remote operation that has been spawned and is awaiting completion. Its
/// [`TaskId`] correlates the background result back to the operation so a
/// stale or foreign task never clears the guard.
#[derive(Debug, Clone, Copy)]
pub(super) struct InFlightRemote {
    /// The background task delivering this operation's outcome.
    pub(super) id: TaskId,
    /// Which remote operation is running (drives the label and command line).
    pub(super) op: RemoteOp,
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
            help_scroll: Cell::new(0),
            help_viewport: Cell::new(0),
            annotations,
            mode: Mode::Normal,
            compose: None,
            list_cursor: 0,
            patches,
            target: DiffTarget::WorkingTree,
            staged: Vec::new(),
            branch: None,
            stashes: Vec::new(),
            last_commit: None,
            untracked_paths: Vec::new(),
            staged_states: HashMap::new(),
            staging_cursor: 0,
            panel_cursor: 0,
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
            background: BackgroundTasks::new(),
            command_log: CommandLog::new(),
            remote_op: None,
            command_log_open: false,
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
        app.staged_states = snapshot.staged_states;
        app.target = target;
        app.stage_ops = Some(ops);
        app.recompute_untracked();
        app.refresh_repo_state();
        app.highlight_cache.clear();
        // Initial collapse state: only fully-staged files start collapsed
        // (there's nothing left to review in them); partially-staged files
        // keep their unstaged work visible, and everything else is expanded.
        let full_staged: Vec<String> = app
            .staged_states
            .iter()
            .filter(|(_, state)| **state == StagedState::Full)
            .map(|(path, _)| path.clone())
            .collect();
        for path in full_staged {
            app.view.set_collapsed(&path, true);
        }
        app.rebuild_rows();
        app
    }

    /// Best-effort re-read of branch/upstream/ahead-behind state and the
    /// stash list through the git backend. Each read updates its field only
    /// on success, so a transient failure keeps the last-known values; a
    /// no-op without a git backend.
    fn refresh_repo_state(&mut self) {
        let Some(ops) = self.stage_ops.as_deref() else {
            return;
        };
        if let Ok(branch) = ops.branch_status() {
            self.branch = Some(branch);
        }
        if let Ok(stashes) = ops.stash_list() {
            self.stashes = stashes;
        }
        if let Ok(commit) = ops.last_commit() {
            self.last_commit = commit;
        }
    }

    /// Recomputes `untracked_paths` from the current files/patches: an entry
    /// with no real patch is a synthetic untracked file (see
    /// [`build_review`]). Only meaningful with a git backend attached.
    fn recompute_untracked(&mut self) {
        self.untracked_paths = self
            .view
            .files
            .iter()
            .zip(&self.patches)
            .filter(|(_, patch)| patch.is_none())
            .map(|(file, _)| file.path.clone())
            .collect();
    }

    /// Sets the workspace root `gd`/`gr`/`K` spawn LSP servers against
    /// (the GitRunner's repo root). Without this, code-intelligence
    /// requests degrade to a footer message.
    pub fn set_repo_root(&mut self, root: PathBuf) {
        self.repo_root = Some(root);
    }

    /// Selects the file whose path is `path`: expands its section if
    /// collapsed, moves the cursor to its section-header row, and scrolls it
    /// into view. Returns `false` (changing nothing) for a path not in the
    /// current diff. This is the narrow select-by-path seam spec 02's git
    /// panel drives file selection through; the sidebar highlight follows the
    /// cursor's owning file, so moving the cursor here is what "selects" the
    /// file everywhere.
    pub fn select_file_by_path(&mut self, path: &str) -> bool {
        let Some(index) = self.view.files.iter().position(|f| f.path == path) else {
            return false;
        };
        if self.view.is_collapsed(path) {
            self.view.set_collapsed(path, false);
            self.rebuild_rows();
        }
        self.view.cursor = self.view.header_row_of_file[index];
        self.view.scroll = 0;
        self.view.ensure_visible();
        true
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

    /// The number of `(path, side)` entries in the highlight cache (test hook).
    #[cfg(test)]
    pub(super) fn highlight_cache_len(&self) -> usize {
        self.highlight_cache.len()
    }

    /// Whether the highlight cache holds an entry for `(path, side)` (test
    /// hook — distinguishes "cached, no spans" from "not cached").
    #[cfg(test)]
    pub(super) fn highlight_cache_contains(&self, path: &str, side: Side) -> bool {
        self.highlight_cache.contains(path, side)
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
            Action::NextHunk => self.view.next_hunk(),
            Action::PrevHunk => self.view.prev_hunk(),
            Action::NextFile => self.view.next_section(),
            Action::PrevFile => self.view.prev_section(),
            Action::ToggleCollapse => self.toggle_collapse(),
            Action::ToggleHelp => {
                self.help_open = !self.help_open;
                self.help_scroll.set(0);
            }
            Action::EnterVisual => self.toggle_visual(),
            Action::Compose => self.open_compose(),
            Action::ToggleList => self.toggle_list(),
            Action::ToggleStage => super::staging::toggle_stage(self),
            Action::StageFile => self.stage_file(),
            Action::ToggleStagingPanel => self.toggle_staging_panel(),
            Action::Search => self.enter_search(),
            Action::SearchNext => self.search_advance(true),
            Action::SearchPrev => self.search_advance(false),
            Action::GotoDefinition => super::code_intel::request(self, PeekKind::Definition),
            Action::GotoReferences => super::code_intel::request(self, PeekKind::References),
            Action::Hover => super::code_intel::request(self, PeekKind::Hover),
            Action::FocusGitPanel => self.toggle_git_panel(),
            Action::PanelCursorDown => self.panel_move_down(),
            Action::PanelCursorUp => self.panel_move_up(),
            Action::PanelSelect => self.panel_select(),
            Action::RemoteFetch => self.request_remote_op(RemoteOp::Fetch),
            Action::RemotePull => self.request_remote_op(RemoteOp::Pull),
            Action::RemotePush => self.request_remote_op(RemoteOp::Push),
            Action::ToggleCommandLog => self.toggle_command_log(),
            Action::Refresh => self.manual_refresh(),
            Action::Quit | Action::QuitDiscard => {}
        }
    }

    /// Toggles the collapse state of the file section under the cursor, then
    /// rebuilds the buffer and re-clamps the cursor into the (now shorter or
    /// longer) buffer, keeping it on the toggled file's header. A no-op on an
    /// empty diff.
    fn toggle_collapse(&mut self) {
        let Some(path) = self.view.toggle_collapse_at_cursor() else {
            return;
        };
        self.rebuild_rows();
        // Keep the cursor on the toggled file's header so a collapse doesn't
        // strand it inside a section that no longer has body rows.
        if let Some(index) = self.view.files.iter().position(|f| f.path == path) {
            self.view.cursor = self.view.header_row_of_file[index];
            self.view.ensure_visible();
        }
    }

    /// Stages or unstages the whole file under the cursor (the `S` gesture),
    /// then auto-collapses (on stage) or auto-expands (on unstage) its
    /// section. Direction is decided by the file's [`StagedState`]: a
    /// fully-staged file unstages and re-expands; an unstaged or partially
    /// staged file stages and collapses. Reuses the existing [`StageOps`]
    /// gestures (`stage_file`/`unstage_file`) — no new git-layer code. A
    /// read-only range target and a missing git backend both degrade to a
    /// footer message; a git failure leaves state unchanged.
    fn stage_file(&mut self) {
        if matches!(self.target, DiffTarget::Range(_)) {
            self.set_status_message("read-only diff target");
            return;
        }
        let Some(file) = self.view.files.get(self.view.file_of_cursor()) else {
            return;
        };
        let path = file.path.clone();
        let staging =
            self.staged_states.get(&path).copied().unwrap_or_default() != StagedState::Full;

        let result = {
            let Some(ops) = self.stage_ops.as_deref() else {
                self.set_status_message("staging unavailable (no git backend)");
                return;
            };
            if staging {
                ops.stage_file(&path)
            } else {
                ops.unstage_file(&path)
            }
        };
        match result {
            Ok(()) => {
                // Collapse on stage / expand on unstage. `refresh` preserves
                // the collapse map by path and re-applies the auto-expand
                // rule, so a file that becomes fully staged stays collapsed
                // and an unstaged one stays open.
                self.view.set_collapsed(&path, staging);
                let verb = if staging { "staged" } else { "unstaged" };
                self.set_status_message(format!("{verb} {path}"));
                self.refresh();
            }
            Err(e) => self.set_status_message(e.to_string()),
        }
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

    /// Rebuilds the whole multi-file row buffer: lazily populates the
    /// syntax-highlight cache for the in-use side(s) of every *expanded*
    /// file (collapsed files show only a header, so they are never
    /// highlighted until expanded — a cache miss is highlighted at most once
    /// per `(path, side)` between refreshes), then concatenates every file's
    /// rows into one buffer via [`build_multibuffer`], carrying per-file
    /// collapse state and staged markers. Also recomputes the active
    /// search's matches and re-derives `selected_file` from the cursor. This
    /// is `App`'s side of the seam: highlighting and the git backend live
    /// here, and the built buffer is fed into [`DiffViewState`].
    pub(super) fn rebuild_rows(&mut self) {
        // Per-file metadata, collected first (cloning paths) so the cache /
        // highlighter can be mutably borrowed without also holding `files`.
        struct Meta {
            path: String,
            old_path: Option<String>,
            collapsed: bool,
            needs_new: bool,
            needs_old: bool,
            synthetic: bool,
        }
        let metas: Vec<Meta> = self
            .view
            .files
            .iter()
            .enumerate()
            .map(|(i, file)| {
                let collapsed = self.view.is_collapsed(&file.path);
                Meta {
                    path: file.path.clone(),
                    old_path: file.old_path.clone(),
                    collapsed,
                    needs_new: !collapsed && syntax::side_in_use(file, Side::New),
                    needs_old: !collapsed && syntax::side_in_use(file, Side::Old),
                    synthetic: self.patches.get(i).is_none_or(|p| p.is_none()),
                }
            })
            .collect();

        for meta in &metas {
            if meta.needs_new {
                syntax::populate_cache(
                    &mut self.highlight_cache,
                    &mut self.highlighter,
                    self.stage_ops.as_deref(),
                    &self.target,
                    &meta.path,
                    meta.old_path.as_deref(),
                    Side::New,
                    meta.synthetic,
                );
            }
            if meta.needs_old {
                syntax::populate_cache(
                    &mut self.highlight_cache,
                    &mut self.highlighter,
                    self.stage_ops.as_deref(),
                    &self.target,
                    &meta.path,
                    meta.old_path.as_deref(),
                    Side::Old,
                    meta.synthetic,
                );
            }
        }

        let collapsed: Vec<bool> = metas.iter().map(|m| m.collapsed).collect();
        let markers: Vec<StagedMarker> = self
            .view
            .files
            .iter()
            .map(
                |f| match self.staged_states.get(&f.path).copied().unwrap_or_default() {
                    StagedState::Full => StagedMarker::Staged,
                    StagedState::Partial => StagedMarker::Partial,
                    StagedState::Unstaged => StagedMarker::None,
                },
            )
            .collect();
        let syntax: Vec<SyntaxSpans> = self
            .view
            .files
            .iter()
            .map(|f| SyntaxSpans {
                new: self.highlight_cache.get(&f.path, Side::New),
                old: self.highlight_cache.get(&f.path, Side::Old),
            })
            .collect();

        let mb = build_multibuffer(
            &self.view.files,
            &collapsed,
            &markers,
            &self.annotations,
            &syntax,
        );
        self.view.rows = mb.rows;
        self.view.file_of_row = mb.file_of_row;
        self.view.header_row_of_file = mb.header_row_of_file;
        self.view.selected_file = self.view.file_of_cursor();
        self.search.recompute(&self.view.rows);
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
        let file = self.view.files.get(self.view.file_of_cursor())?;
        match self.view.rows.get(self.view.cursor)? {
            Row::Line(line) => line_target(&file.path, line),
            Row::HunkHeader { hunk_index, .. } => self.hunk_target(*hunk_index),
            Row::FileHeader { .. } | Row::Binary => Some(Target::file(&file.path)),
            Row::Annotation { .. } => None,
        }
    }

    fn hunk_target(&self, hunk_index: usize) -> Option<Target> {
        let file = self.view.files.get(self.view.file_of_cursor())?;
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
        let file = self.view.files.get(self.view.file_of_cursor())?;
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
            Mode::Compose | Mode::Staging | Mode::Panel | Mode::Search | Mode::Peek => {}
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
            // Expand the target section so line/hunk anchors are reachable,
            // then resolve the anchor against the whole buffer (a File target
            // lands on the section header; line/hunk/range targets resolve
            // within this file's row span). Fall back to the section header
            // if the specific anchor row can no longer be found.
            self.view.set_collapsed(&path, false);
            self.rebuild_rows();
            self.view.cursor = self
                .view
                .anchor_row_in_buffer(&target)
                .unwrap_or_else(|| self.view.header_row_of_file[index]);
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
        if let Err(e) = self.rebuild_from_git() {
            self.set_status_message(format!("refresh failed: {e}"));
        }
    }

    /// Re-reads the diff/status for the current target and applies the fresh
    /// snapshot (see [`App::apply_snapshot`]). Surfaces a git/parse failure to
    /// the caller rather than swallowing it; a no-op (and `Ok`) without a git
    /// backend.
    fn rebuild_from_git(&mut self) -> Result<(), ReviewError> {
        let snapshot = {
            let Some(ops) = self.stage_ops.as_deref() else {
                return Ok(());
            };
            build_review(ops, &self.target)?
        };
        self.apply_snapshot(snapshot);
        Ok(())
    }

    /// The `R` action: an unconditional refresh with a footer acknowledgement,
    /// so a manual reload always confirms it ran even when nothing changed.
    fn manual_refresh(&mut self) {
        match self.rebuild_from_git() {
            Ok(()) => self.set_status_message("refreshed"),
            Err(e) => self.set_status_message(format!("refresh failed: {e}")),
        }
    }

    /// Re-reads the working tree only when it has actually changed since the
    /// last refresh, applying the fresh snapshot and returning whether it did.
    /// This is the lazygit-style polling path (see [`super::event_loop`]): the
    /// diff/status git reads run every tick, but the expensive row rebuild and
    /// cursor restoration are skipped whenever the review is byte-identical to
    /// what's already displayed, so idle polling never disturbs scrolling.
    /// Silent on a transient git error — it keeps the current view and retries
    /// next tick, degrading quietly like the LSP layer.
    pub(super) fn auto_refresh(&mut self) -> bool {
        let snapshot = {
            let Some(ops) = self.stage_ops.as_deref() else {
                return false;
            };
            match build_review(ops, &self.target) {
                Ok(snapshot) => snapshot,
                Err(_) => return false,
            }
        };
        if snapshot.files == self.view.files
            && snapshot.staged == self.staged
            && snapshot.staged_states == self.staged_states
        {
            return false;
        }
        self.apply_snapshot(snapshot);
        true
    }

    /// Runs an [`App::auto_refresh`] unless a background reload would be
    /// unwelcome: a remote op is mid-flight (its completion refreshes anyway,
    /// and the intermediate tree is transient — mirrors lazygit pausing
    /// background refreshes during its own git ops); the target is a fixed
    /// range (nothing to pick up); or the user has in-progress input or a
    /// Visual selection anchored to positions a rebuild would move.
    pub(super) fn maybe_auto_refresh(&mut self) {
        if self.remote_op.is_some() || matches!(self.target, DiffTarget::Range(_)) {
            return;
        }
        if matches!(
            self.mode,
            Mode::Compose | Mode::Search | Mode::Visual { .. }
        ) {
            return;
        }
        self.auto_refresh();
    }

    /// Applies a freshly built [`ReviewSnapshot`]: swaps in the new
    /// files/patches/staged state, maintains the collapse map, invalidates
    /// only the highlight-cache entries whose file content changed, rebuilds
    /// rows, and restores the cursor/scroll/staging-panel position.
    fn apply_snapshot(&mut self, snapshot: ReviewSnapshot) {
        // Remember the cursor's file by path and its offset within that
        // file's section, so the same spot is restored even if files
        // reorder or the section shrinks.
        let cursor_file = self.view.file_of_cursor();
        let previous_path = self.view.files.get(cursor_file).map(|f| f.path.clone());
        let local_offset = self.view.cursor.saturating_sub(
            self.view
                .header_row_of_file
                .get(cursor_file)
                .copied()
                .unwrap_or(0),
        );

        // Take the previous files out (rather than clone) so their content
        // can be compared per path against the incoming snapshot for targeted
        // highlight-cache invalidation below.
        let old_by_path: HashMap<String, FileDiff> = std::mem::take(&mut self.view.files)
            .into_iter()
            .map(|f| (f.path.clone(), f))
            .collect();

        self.view.files = snapshot.files;
        self.patches = snapshot.patches;
        self.staged = snapshot.staged;
        self.staged_states = snapshot.staged_states;
        self.recompute_untracked();
        self.refresh_repo_state();

        // Collapse-map maintenance (spec Unit 2, "nothing hides"):
        // - drop entries for files that left the review, then
        // - auto-expand any collapsed file that is now *partially* staged
        //   (staged, then edited again — its fresh unstaged work must not
        //   stay hidden behind a collapsed header, and it renders `±`).
        // Fully-staged collapsed files stay collapsed (nothing to review),
        // and every other file keeps whatever collapse state it had.
        let present: HashSet<String> = self.view.files.iter().map(|f| f.path.clone()).collect();
        self.view.retain_collapsed(|path| present.contains(path));
        let reexpand: Vec<String> = self
            .view
            .files
            .iter()
            .map(|f| f.path.clone())
            .filter(|path| self.view.is_collapsed(path))
            .filter(|path| {
                self.staged_states.get(path).copied().unwrap_or_default() == StagedState::Partial
            })
            .collect();
        for path in reexpand {
            self.view.set_collapsed(&path, false);
        }

        // Per-file highlight-cache invalidation (spec 03, task 5.1): keep the
        // cached spans for files whose diff content is byte-identical across
        // the refresh, invalidate only files whose `FileDiff` changed (or are
        // newly present), and drop entries for files that left the review so
        // the cache can't grow without bound. `FileDiff` equality is a sound
        // and complete proxy for "the highlighted content could have changed":
        // the diff is a pure function of both sides' whole-file source, so an
        // unchanged `FileDiff` means unchanged content and still-valid spans
        // (renames included — `old_path` is part of the compared value). The
        // cache is keyed by each file's current path, matching `rebuild_rows`.
        for file in &self.view.files {
            if old_by_path.get(&file.path) != Some(file) {
                self.highlight_cache.invalidate_path(&file.path);
            }
        }
        self.highlight_cache
            .retain_paths(|path| present.contains(path));
        self.rebuild_rows();
        if self.view.rows.is_empty() {
            self.view.cursor = 0;
            self.view.scroll = 0;
        } else {
            let restored = previous_path
                .as_deref()
                .and_then(|path| self.view.files.iter().position(|f| f.path == path));
            let target = match restored {
                Some(j) => {
                    let (start, end) = self.view.section_span(j);
                    (start + local_offset).min(end.saturating_sub(1))
                }
                None => self.view.cursor.min(self.view.max_cursor()),
            };
            self.view.cursor = self
                .view
                .nearest_addressable(target.min(self.view.max_cursor()), false);
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
            Mode::Compose | Mode::List | Mode::Panel | Mode::Search | Mode::Peek => {}
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

    // -- Git panel focus & navigation -------------------------------------

    /// Whether the git panel currently holds focus (drives border emphasis
    /// and which pane's cursor renders).
    pub fn git_panel_focused(&self) -> bool {
        matches!(self.mode, Mode::Panel)
    }

    /// Toggles git-panel focus: from Normal/Visual it focuses the panel
    /// (resetting its cursor to the top); from the focused panel it returns
    /// to Normal. A no-op while another modal (Compose/List/Staging/Search/
    /// Peek) owns the keyboard, mirroring the other panel toggles.
    fn toggle_git_panel(&mut self) {
        match self.mode {
            Mode::Panel => self.mode = Mode::Normal,
            Mode::Compose | Mode::List | Mode::Staging | Mode::Search | Mode::Peek => {}
            Mode::Normal | Mode::Visual { .. } => {
                self.panel_cursor = 0;
                self.mode = Mode::Panel;
            }
        }
    }

    /// Moves the panel cursor down one navigable row, clamped at the last.
    pub fn panel_move_down(&mut self) {
        let len = super::git_panel::navigable_rows(self).len();
        self.panel_cursor = super::git_panel::moved_cursor(self.panel_cursor, len, true);
    }

    /// Moves the panel cursor up one navigable row, clamped at the first.
    pub fn panel_move_up(&mut self) {
        let len = super::git_panel::navigable_rows(self).len();
        self.panel_cursor = super::git_panel::moved_cursor(self.panel_cursor, len, false);
    }

    /// Acts on the panel cursor's current row: a file row scrolls the
    /// multibuffer to that file's section (via [`App::select_file_by_path`])
    /// and returns focus to the diff; a stash row (or an out-of-range cursor)
    /// is a no-op, leaving the panel focused.
    pub fn panel_select(&mut self) {
        let rows = super::git_panel::navigable_rows(self);
        if let Some(super::git_panel::PanelRow::File(i)) = rows.get(self.panel_cursor) {
            let path = self.view.files[*i].path.clone();
            self.select_file_by_path(&path);
            self.mode = Mode::Normal;
        }
    }

    // -- Remote operations & command log ----------------------------------

    /// Toggles the command-log pane in the bottom-panel slot.
    fn toggle_command_log(&mut self) {
        self.command_log_open = !self.command_log_open;
    }

    /// The label of the remote operation currently in flight, if any (drives
    /// the running indicator). `None` when nothing is running.
    pub fn remote_running_label(&self) -> Option<&'static str> {
        self.remote_op.as_ref().map(|o| o.op.label())
    }

    /// Requests a remote operation (`fetch`/`pull`/`push`), spawning it on a
    /// background thread so the render loop never blocks. Enforces the
    /// single-in-flight guard: if a remote op is already running the request
    /// is rejected with a status message and nothing is spawned. Without a
    /// known repository root (git-less contexts) the request degrades to a
    /// message, like every other git-backed gesture.
    ///
    /// The child command is a fixed argv with `GIT_TERMINAL_PROMPT=0` (see
    /// [`crate::git::remote_command`]); no shell, no `--force`, no credential
    /// handling.
    pub(super) fn request_remote_op(&mut self, op: RemoteOp) {
        if let Some(running) = self.remote_op.as_ref() {
            self.set_status_message(format!(
                "{} already running — wait for it to finish",
                running.op.label()
            ));
            return;
        }
        let Some(root) = self.repo_root.clone() else {
            self.set_status_message("remote operations unavailable (no repository)");
            return;
        };
        let mut command = remote_command(op, &root);
        let id = self.background.spawn(move || run_command(&mut command));
        self.remote_op = Some(InFlightRemote { id, op });
        self.set_status_message(format!("{}\u{2026}", op.label()));
    }

    /// Drains completed background remote operations (once per event-loop
    /// tick, alongside [`super::code_intel::poll`]). For the in-flight op's
    /// result it appends a [`CommandLogEntry`], clears the guard, re-runs the
    /// full refresh (diff/status plus branch/stash reads), and sets a
    /// success/failure footer summary. Foreign or stale task ids are ignored.
    pub(super) fn poll_remote(&mut self) {
        let done = self.background.poll();
        for (id, result) in done {
            let Some(in_flight) = self.remote_op else {
                continue;
            };
            if in_flight.id != id {
                continue;
            }
            let op = in_flight.op;
            self.remote_op = None;

            let entry = match result {
                Ok(outcome) => CommandLogEntry {
                    command_line: op.command_line(),
                    success: outcome.success,
                    code: outcome.code,
                    stdout: outcome.stdout,
                    stderr: outcome.stderr,
                },
                Err(panic) => CommandLogEntry {
                    command_line: op.command_line(),
                    success: false,
                    code: None,
                    stdout: String::new(),
                    stderr: panic.message,
                },
            };
            let success = entry.success;
            self.command_log.push(entry);

            // Re-read the working tree so the changes list, branch header, and
            // ahead/behind reflect the remote op; staged markers and
            // annotations survive exactly as they do after any refresh.
            self.refresh();

            if success {
                self.set_status_message(format!("{} succeeded", op.label()));
            } else {
                self.set_status_message(format!(
                    "{} failed \u{2014} see command log (@)",
                    op.label()
                ));
            }
        }
    }

    /// Best-effort re-read of the staged-file list (and per-path staged
    /// states) from `git status`, keeping the previous values on any failure.
    fn refresh_staged_list(&mut self) {
        let (staged, states) = {
            let Some(ops) = self.stage_ops.as_deref() else {
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
            | Action::ToggleCommandLog
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
    fn next_file_jumps_to_next_section_header() {
        let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
        app.apply(Action::CursorDown);
        app.apply(Action::NextFile);
        assert_eq!(app.view.selected_file, 1);
        // Cursor lands on b.rs's section header, not row 0.
        assert_eq!(app.view.cursor, app.view.header_row_of_file[1]);
        assert!(matches!(
            app.view.rows[app.view.cursor],
            Row::FileHeader { file_index: 1, .. }
        ));
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
    fn prev_file_jumps_back_across_sections() {
        let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
        app.apply(Action::NextFile); // -> b.rs header
        assert_eq!(app.view.selected_file, 1);
        // From b.rs's header, prev-section jumps to a.rs's header.
        app.apply(Action::PrevFile);
        assert_eq!(app.view.selected_file, 0);
        assert_eq!(app.view.cursor, app.view.header_row_of_file[0]);
    }

    #[test]
    fn toggle_collapse_collapses_and_expands_file_under_cursor() {
        let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
        let expanded_len = app.view.rows.len();
        // Cursor starts on a.rs's header.
        app.apply(Action::ToggleCollapse);
        assert!(app.view.is_collapsed("a.rs"));
        assert!(app.view.rows.len() < expanded_len);
        // a.rs now contributes exactly its (collapsed) header row.
        assert!(matches!(
            app.view.rows[app.view.header_row_of_file[0]],
            Row::FileHeader {
                collapsed: true,
                file_index: 0,
                ..
            }
        ));
        // Cursor stays on a.rs's header, still addressable.
        assert_eq!(app.view.cursor, app.view.header_row_of_file[0]);
        assert!(app.view.rows[app.view.cursor].is_addressable());

        app.apply(Action::ToggleCollapse);
        assert!(!app.view.is_collapsed("a.rs"));
        assert_eq!(app.view.rows.len(), expanded_len);
    }

    #[test]
    fn toggle_collapse_targets_the_cursor_file_not_the_first() {
        let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
        app.apply(Action::NextFile); // cursor onto b.rs's header
        app.apply(Action::ToggleCollapse);
        assert!(app.view.is_collapsed("b.rs"));
        assert!(!app.view.is_collapsed("a.rs"));
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
    fn jump_to_annotation_expands_a_collapsed_target_section() {
        // Jumping to an annotation whose file is collapsed must re-expand
        // that section so the line/hunk anchor is reachable, then land on it.
        let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
        app.annotations
            .add(
                Target::line("b.rs", 1, Side::New),
                Classification::Issue,
                "note",
            )
            .unwrap();
        app.view.set_collapsed("b.rs", true);
        app.rebuild_rows();
        assert!(app.view.is_collapsed("b.rs"));

        app.list_cursor = 0;
        app.jump_to_focused_annotation();

        assert!(!app.view.is_collapsed("b.rs")); // re-expanded
        assert_eq!(app.view.selected_file, 1);
        let Row::Line(line) = &app.view.rows[app.view.cursor] else {
            panic!("expected cursor on a line row");
        };
        assert_eq!(line.new_line, Some(1));
    }

    // -- Markdown-on-quit output is unchanged by the multibuffer -------------

    #[test]
    fn multi_section_annotations_emit_unchanged_markdown() {
        // The stdout markdown is a public API keyed purely off the annotation
        // store's insertion order — the multibuffer never touches it. Compose
        // annotations across several sections and assert byte-for-byte output.
        use crate::annotate::render_markdown;
        let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1), file("c.rs", 1)]);
        app.annotations
            .add(Target::file("a.rs"), Classification::Praise, "clean module")
            .unwrap();
        app.annotations
            .add(
                Target::line("b.rs", 1, Side::New),
                Classification::Question,
                "why new0?",
            )
            .unwrap();
        app.annotations
            .add(
                Target::hunk("c.rs", 1, 1).unwrap(),
                Classification::Nit,
                "tidy this hunk",
            )
            .unwrap();
        app.rebuild_rows();

        let expected = "\
## a.rs\n\n[praise] clean module\n\n\
## b.rs:1 (+)\n\n[question] why new0?\n\n\
## c.rs:1-1 (+)\n\n[nit] tidy this hunk\n";
        assert_eq!(render_markdown(&app.annotations), expected);
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
        // Interior-mutable overrides: when set, `diff`/`status` read through
        // these instead, so a test can mutate the refresh result mid-flow
        // (e.g. to simulate an external edit landing between operations).
        diff_override: Option<Rc<RefCell<Vec<RawFilePatch>>>>,
        status_override: Option<Rc<RefCell<Vec<FileStatus>>>>,
        untracked_content: std::collections::HashMap<String, Vec<u8>>,
        fail_ops: bool,
        show_calls: Rc<RefCell<usize>>,
        show_content: Option<String>,
        branch: Option<BranchStatus>,
        stashes: Vec<StashEntry>,
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
            match &self.diff_override {
                Some(h) => Ok(h.borrow().clone()),
                None => Ok(self.diff.clone()),
            }
        }

        fn status(&self) -> Result<Vec<FileStatus>, GitError> {
            match &self.status_override {
                Some(h) => Ok(h.borrow().clone()),
                None => Ok(self.status.clone()),
            }
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

        fn branch_status(&self) -> Result<BranchStatus, GitError> {
            self.branch
                .clone()
                .ok_or_else(|| GitError::Parse("no branch".into()))
        }

        fn stash_list(&self) -> Result<Vec<StashEntry>, GitError> {
            Ok(self.stashes.clone())
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

    /// A porcelain status entry with staged (index-side) changes only
    /// (`M.` → fully staged).
    fn staged_entry(path: &str) -> FileStatus {
        FileStatus {
            kind: ChangeKind::Ordinary,
            staged: StatusCode::Modified,
            unstaged: StatusCode::Unmodified,
            path: path.to_string(),
            orig_path: None,
        }
    }

    /// A porcelain status entry with both staged and unstaged changes
    /// (`MM` → partially staged).
    fn partial_entry(path: &str) -> FileStatus {
        FileStatus {
            kind: ChangeKind::Ordinary,
            staged: StatusCode::Modified,
            unstaged: StatusCode::Modified,
            path: path.to_string(),
            orig_path: None,
        }
    }

    /// A porcelain status entry with working-tree-only changes (`.M` →
    /// unstaged).
    fn unstaged_entry(path: &str) -> FileStatus {
        FileStatus {
            kind: ChangeKind::Ordinary,
            staged: StatusCode::Unmodified,
            unstaged: StatusCode::Modified,
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
            staged_states: HashMap::new(),
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
            staged_states: HashMap::new(),
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
            staged_states: HashMap::new(),
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
    fn refresh_repopulates_branch_and_stash_and_preserves_staged_and_annotations() {
        let p = raw_patch("a.rs", 1);
        let calls = Rc::new(RefCell::new(Vec::new()));
        let fake = FakeGit {
            calls: Rc::clone(&calls),
            diff: vec![p.clone()],
            status: vec![staged_entry("a.rs")],
            branch: Some(BranchStatus {
                name: "main".into(),
                detached: false,
                upstream: Some("origin/main".into()),
                ahead_behind: Some((2, 1)),
            }),
            stashes: vec![StashEntry {
                stash_ref: "stash@{0}".into(),
                branch: Some("main".into()),
                message: "wip: parser".into(),
            }],
            ..FakeGit::default()
        };
        let snapshot = ReviewSnapshot {
            files: vec![FileDiff::from_patch(&p).unwrap()],
            patches: vec![Some(p)],
            staged: Vec::new(),
            staged_states: HashMap::new(),
        };
        let mut app = App::with_git(snapshot, DiffTarget::WorkingTree, Box::new(fake));
        // Startup read populated branch/stash state.
        assert_eq!(app.branch.as_ref().unwrap().name, "main");
        assert_eq!(app.stashes.len(), 1);
        // An annotation made this session must survive the refresh.
        app.annotations
            .add(Target::file("a.rs"), Classification::Nit, "look here")
            .unwrap();

        app.refresh();

        assert_eq!(app.branch.as_ref().unwrap().ahead_behind, Some((2, 1)));
        assert_eq!(app.stashes[0].message, "wip: parser");
        // Staged markers survive: the refresh status reports a.rs staged.
        assert_eq!(app.staged.len(), 1);
        assert_eq!(app.staged[0].path, "a.rs");
        // Annotations survive the refresh exactly as today.
        assert_eq!(app.annotations.len(), 1);
    }

    // -- Remote operations & command log (task 4.0) ------------------------

    /// While a remote op is in flight, a second request is rejected with a
    /// status message and spawns nothing — the guard is a message, not a queue.
    #[test]
    fn second_remote_request_while_one_in_flight_is_rejected_and_does_not_spawn() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        // A repo root is present (so a request *could* spawn) and a fetch is
        // already recorded as in flight.
        app.repo_root = Some(std::path::PathBuf::from("/tmp"));
        app.remote_op = Some(InFlightRemote {
            id: TaskId(7),
            op: RemoteOp::Fetch,
        });

        app.request_remote_op(RemoteOp::Pull);

        // The in-flight op is untouched (still the fetch), the request was
        // rejected with a message, and nothing new was spawned.
        assert_eq!(app.remote_op.map(|o| o.op), Some(RemoteOp::Fetch));
        assert!(
            app.status_message
                .as_deref()
                .is_some_and(|m| m.contains("already running")),
            "got {:?}",
            app.status_message
        );
        assert!(
            app.background.poll().is_empty(),
            "the rejected request must not spawn a background task"
        );
    }

    /// Without a known repository root, a remote request degrades to a footer
    /// message rather than panicking or spawning.
    #[test]
    fn remote_request_without_a_repo_root_is_a_message_only() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        assert!(app.repo_root.is_none());
        app.request_remote_op(RemoteOp::Fetch);
        assert!(app.remote_op.is_none());
        assert_eq!(
            app.status_message.as_deref(),
            Some("remote operations unavailable (no repository)")
        );
    }

    /// The full spawn -> poll -> log pipeline, driven through the *real*
    /// [`BackgroundTasks`] path with a benign successful command standing in
    /// for git: on completion the command log gains an entry, the refresh
    /// re-reads branch/stash state, and staged markers plus annotations
    /// survive exactly as after any refresh.
    #[cfg(unix)]
    #[test]
    fn completed_remote_op_logs_and_refreshes_preserving_staged_and_annotations() {
        use std::process::Command;
        use std::time::{Duration, Instant};

        let p = raw_patch("a.rs", 1);
        let calls = Rc::new(RefCell::new(Vec::new()));
        let fake = FakeGit {
            calls: Rc::clone(&calls),
            diff: vec![p.clone()],
            status: vec![staged_entry("a.rs")],
            branch: Some(BranchStatus {
                name: "main".into(),
                detached: false,
                upstream: Some("origin/main".into()),
                ahead_behind: Some((0, 0)),
            }),
            stashes: vec![StashEntry {
                stash_ref: "stash@{0}".into(),
                branch: Some("main".into()),
                message: "wip: parser".into(),
            }],
            ..FakeGit::default()
        };
        let snapshot = ReviewSnapshot {
            files: vec![FileDiff::from_patch(&p).unwrap()],
            patches: vec![Some(p)],
            staged: Vec::new(),
            staged_states: HashMap::new(),
        };
        let mut app = App::with_git(snapshot, DiffTarget::WorkingTree, Box::new(fake));
        app.annotations
            .add(Target::file("a.rs"), Classification::Nit, "look here")
            .unwrap();

        // Spawn a benign successful command through the real background poller
        // and mark it as the in-flight fetch (this is exactly what
        // `request_remote_op` does, minus running git itself).
        let id = app
            .background
            .spawn(|| run_command(&mut Command::new("true")));
        app.remote_op = Some(InFlightRemote {
            id,
            op: RemoteOp::Fetch,
        });

        // Drain until the op is logged (mirrors the event-loop tick).
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.command_log.is_empty() && Instant::now() < deadline {
            app.poll_remote();
            std::thread::sleep(Duration::from_millis(5));
        }

        // The command log gained exactly one entry for the fetch.
        assert_eq!(app.command_log.len(), 1);
        let entry = app.command_log.entries().next().unwrap();
        assert_eq!(entry.command_line, "git fetch");
        assert!(entry.success);
        // The guard is cleared, so a fresh op could start.
        assert!(app.remote_op.is_none());
        // Refresh ran: branch/stash re-read; staged markers + annotations survive.
        assert_eq!(app.branch.as_ref().unwrap().name, "main");
        assert_eq!(app.stashes.len(), 1);
        assert_eq!(app.staged.len(), 1);
        assert_eq!(app.staged[0].path, "a.rs");
        assert_eq!(app.annotations.len(), 1);
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
            staged_states: HashMap::new(),
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

    // -- Stage-and-collapse review flow (S) ----------------------------------

    /// Builds an `App` whose fake reads its refresh diff/status through
    /// mutable handles, so a test can change what the next `refresh` sees.
    /// The snapshot is derived from `initial_status` (staged list + states)
    /// over `initial_files`.
    #[allow(clippy::type_complexity)]
    fn app_with_mutable_fake(
        initial_files: Vec<RawFilePatch>,
        initial_status: Vec<FileStatus>,
        refresh_diff: Vec<RawFilePatch>,
        refresh_status: Vec<FileStatus>,
    ) -> (
        App,
        Rc<RefCell<Vec<StageCall>>>,
        Rc<RefCell<Vec<RawFilePatch>>>,
        Rc<RefCell<Vec<FileStatus>>>,
    ) {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let diff_h = Rc::new(RefCell::new(refresh_diff));
        let status_h = Rc::new(RefCell::new(refresh_status));
        let fake = FakeGit {
            calls: Rc::clone(&calls),
            diff_override: Some(Rc::clone(&diff_h)),
            status_override: Some(Rc::clone(&status_h)),
            ..FakeGit::default()
        };
        let files = initial_files
            .iter()
            .map(|p| FileDiff::from_patch(p).unwrap())
            .collect();
        let snapshot = ReviewSnapshot {
            files,
            patches: initial_files.into_iter().map(Some).collect(),
            staged: staged_from_status(&initial_status),
            staged_states: staged_states_from_status(&initial_status),
        };
        let app = App::with_git(snapshot, DiffTarget::WorkingTree, Box::new(fake));
        (app, calls, diff_h, status_h)
    }

    #[test]
    fn stage_file_stages_the_file_and_collapses_its_section() {
        let p = raw_patch("a.rs", 1);
        // After staging, a.rs is fully staged and gone from the working diff;
        // decision (A) keeps it as a header-only section from status.
        let (mut app, calls) = app_with_fake(
            vec![p],
            DiffTarget::WorkingTree,
            vec![], // nothing unstaged left
            vec![staged_entry("a.rs")],
        );
        assert!(!app.view.is_collapsed("a.rs"));
        app.apply(Action::StageFile);
        assert_eq!(
            single_call(&calls),
            StageCall::StageFile("a.rs".to_string())
        );
        assert!(app.view.is_collapsed("a.rs"));
        // The section persists (never hides), now marked fully staged.
        assert_eq!(app.view.files.len(), 1);
        assert_eq!(app.view.files[0].path, "a.rs");
        assert_eq!(
            app.staged_states.get("a.rs").copied(),
            Some(StagedState::Full)
        );
        assert_eq!(app.status_message.as_deref(), Some("staged a.rs"));
    }

    #[test]
    fn stage_file_on_fully_staged_file_unstages_and_expands() {
        let p = raw_patch("a.rs", 1);
        // Start fully staged (collapsed at launch); after unstaging, a.rs is
        // back in the working diff with nothing staged.
        let (mut app, calls, _diff, _status) = app_with_mutable_fake(
            vec![p.clone()],
            vec![staged_entry("a.rs")],   // initial: M. -> Full
            vec![p],                      // refresh diff after unstage
            vec![unstaged_entry("a.rs")], // refresh status: unstaged only
        );
        assert!(app.view.is_collapsed("a.rs")); // fully staged starts collapsed
        // Cursor sits on the collapsed header.
        app.apply(Action::StageFile);
        assert_eq!(
            single_call(&calls),
            StageCall::UnstageFile("a.rs".to_string())
        );
        assert!(!app.view.is_collapsed("a.rs")); // auto-expanded
        assert_eq!(app.staged_states.get("a.rs").copied(), None); // no longer staged
        assert_eq!(app.status_message.as_deref(), Some("unstaged a.rs"));
    }

    #[test]
    fn stage_file_records_only_stageops_methods() {
        // Both directions must go through `stage_file`/`unstage_file` — no
        // new git-layer gestures.
        let p = raw_patch("a.rs", 1);
        let (mut app, calls) = app_with_fake(
            vec![p],
            DiffTarget::WorkingTree,
            vec![],
            vec![staged_entry("a.rs")],
        );
        app.apply(Action::StageFile);
        for call in calls.borrow().iter() {
            assert!(
                matches!(call, StageCall::StageFile(_) | StageCall::UnstageFile(_)),
                "unexpected call {call:?}"
            );
        }
    }

    #[test]
    fn stage_file_on_read_only_range_is_a_noop_with_message() {
        let p = raw_patch("a.rs", 1);
        let (mut app, calls) = app_with_fake(
            vec![p.clone()],
            DiffTarget::Range("main..HEAD".to_string()),
            vec![p],
            vec![],
        );
        app.apply(Action::StageFile);
        assert!(calls.borrow().is_empty());
        assert_eq!(app.status_message.as_deref(), Some("read-only diff target"));
    }

    #[test]
    fn stage_file_error_sets_message_and_leaves_state_unchanged() {
        let p = raw_patch("a.rs", 1);
        let calls = Rc::new(RefCell::new(Vec::new()));
        let fake = FakeGit {
            calls: Rc::clone(&calls),
            diff: vec![],
            fail_ops: true,
            ..FakeGit::default()
        };
        let snapshot = ReviewSnapshot {
            files: vec![FileDiff::from_patch(&p).unwrap()],
            patches: vec![Some(p)],
            staged: Vec::new(),
            staged_states: HashMap::new(),
        };
        let mut app = App::with_git(snapshot, DiffTarget::WorkingTree, Box::new(fake));
        app.apply(Action::StageFile);
        // The stage was attempted but failed; nothing collapsed, file kept.
        assert_eq!(
            single_call(&calls),
            StageCall::StageFile("a.rs".to_string())
        );
        assert!(!app.view.is_collapsed("a.rs"));
        assert_eq!(app.view.files.len(), 1);
        assert!(
            app.status_message
                .as_deref()
                .is_some_and(|m| m.contains("simulated git failure"))
        );
    }

    #[test]
    fn hunk_stage_marks_file_partial_and_keeps_it_expanded() {
        // Staging one of two hunks leaves the file partially staged: its
        // header marker becomes `±` and the section stays expanded (its
        // unstaged work is still visible).
        let p = raw_patch("a.rs", 2);
        let (mut app, _calls) = app_with_fake(
            vec![p.clone()],
            DiffTarget::WorkingTree,
            vec![p],                     // still has unstaged content
            vec![partial_entry("a.rs")], // MM -> Partial
        );
        app.apply(Action::NextHunk); // onto a hunk header
        app.apply(Action::ToggleStage); // stage that hunk, then refresh
        assert_eq!(
            app.staged_states.get("a.rs").copied(),
            Some(StagedState::Partial)
        );
        let header = app.view.header_row_of_file[0];
        let Row::FileHeader { staged_marker, .. } = &app.view.rows[header] else {
            panic!("expected file header");
        };
        assert_eq!(*staged_marker, StagedMarker::Partial);
        assert!(!app.view.is_collapsed("a.rs"));
    }

    // -- Refresh collapse-map rules (task 3.3) -------------------------------

    #[test]
    fn refresh_auto_expands_a_partially_staged_collapsed_file() {
        // The "nothing hides" guarantee: a fully-staged (collapsed) file that
        // gets edited again comes back partially staged and must re-expand.
        let p = raw_patch("a.rs", 1);
        let (mut app, _calls, _diff, _status) = app_with_mutable_fake(
            vec![p.clone()],
            vec![staged_entry("a.rs")],  // initial: Full -> collapsed
            vec![p],                     // external edit: unstaged diff present
            vec![partial_entry("a.rs")], // now MM -> Partial
        );
        assert!(app.view.is_collapsed("a.rs"));
        app.refresh(); // picks up the external edit
        assert!(!app.view.is_collapsed("a.rs")); // re-expanded
        assert_eq!(
            app.staged_states.get("a.rs").copied(),
            Some(StagedState::Partial)
        );
        let header = app.view.header_row_of_file[0];
        let Row::FileHeader { staged_marker, .. } = &app.view.rows[header] else {
            panic!("expected file header");
        };
        assert_eq!(*staged_marker, StagedMarker::Partial); // ±
    }

    #[test]
    fn refresh_keeps_a_still_fully_staged_collapsed_file_collapsed() {
        let p = raw_patch("a.rs", 1);
        let (mut app, _calls, _diff, _status) = app_with_mutable_fake(
            vec![p],
            vec![staged_entry("a.rs")], // Full -> collapsed
            vec![],                     // still nothing unstaged
            vec![staged_entry("a.rs")], // still Full
        );
        assert!(app.view.is_collapsed("a.rs"));
        app.refresh();
        assert!(app.view.is_collapsed("a.rs")); // stays collapsed
    }

    #[test]
    fn refresh_preserves_a_manually_collapsed_unstaged_file() {
        // A file the reviewer collapsed with `za` that has only unstaged
        // changes keeps its collapse state (it isn't partially staged).
        let a = raw_patch("a.rs", 1);
        let b = raw_patch("b.rs", 1);
        let (mut app, _calls, _diff, _status) = app_with_mutable_fake(
            vec![a.clone(), b.clone()],
            vec![],     // nothing staged
            vec![a, b], // refresh returns the same two files
            vec![],
        );
        app.view.set_collapsed("a.rs", true);
        app.rebuild_rows();
        app.refresh();
        assert!(app.view.is_collapsed("a.rs")); // survives refresh
        assert!(!app.view.is_collapsed("b.rs"));
    }

    #[test]
    fn refresh_drops_collapse_entries_for_departed_files() {
        // b.rs leaves the review on refresh; its collapse-map entry must be
        // dropped rather than lingering as stale state.
        let a = raw_patch("a.rs", 1);
        let b = raw_patch("b.rs", 1);
        let (mut app, _calls, _diff, _status) = app_with_mutable_fake(
            vec![a.clone(), b.clone()],
            vec![],
            vec![a], // b.rs gone from the refresh diff
            vec![],
        );
        app.view.set_collapsed("b.rs", true);
        app.rebuild_rows();
        app.refresh();
        assert!(!app.view.collapse_contains("b.rs")); // entry cleaned up
    }

    // -- Auto-refresh (working-tree polling) --------------------------------

    #[test]
    fn auto_refresh_applies_an_external_edit_and_reports_the_change() {
        // The working tree gains a second hunk between polls (an agent edited
        // the file): auto_refresh picks it up and reports that it applied.
        let (mut app, _calls, diff_h, _status_h) = app_with_mutable_fake(
            vec![raw_patch("a.rs", 1)],
            vec![],
            vec![raw_patch("a.rs", 1)], // override starts identical to the view
            vec![],
        );
        assert_eq!(app.view.files[0].hunks.len(), 1);

        // Simulate the external edit landing between ticks.
        *diff_h.borrow_mut() = vec![raw_patch("a.rs", 2)];
        assert!(app.auto_refresh(), "changed tree should apply");
        assert_eq!(app.view.files[0].hunks.len(), 2);
    }

    #[test]
    fn auto_refresh_is_a_noop_when_the_tree_is_unchanged() {
        // Nothing changed since the last refresh: auto_refresh must not rebuild
        // (it returns false) so idle polling never disturbs the view.
        let (mut app, _calls, _diff_h, _status_h) = app_with_mutable_fake(
            vec![raw_patch("a.rs", 1)],
            vec![],
            vec![raw_patch("a.rs", 1)], // identical to what's displayed
            vec![],
        );
        assert!(!app.auto_refresh(), "unchanged tree should be a no-op");
        assert_eq!(app.status_message, None);
    }

    #[test]
    fn maybe_auto_refresh_skips_while_a_remote_op_is_in_flight() {
        // A remote op's own completion refreshes; the intermediate tree it
        // produces must not be picked up mid-flight.
        let (mut app, _calls, diff_h, _status_h) = app_with_mutable_fake(
            vec![raw_patch("a.rs", 1)],
            vec![],
            vec![raw_patch("a.rs", 1)],
            vec![],
        );
        *diff_h.borrow_mut() = vec![raw_patch("a.rs", 2)];
        app.remote_op = Some(InFlightRemote {
            id: TaskId(0),
            op: RemoteOp::Fetch,
        });
        app.maybe_auto_refresh();
        assert_eq!(app.view.files[0].hunks.len(), 1, "skipped during remote op");
        // With the op cleared, the same pending edit is picked up.
        app.remote_op = None;
        app.maybe_auto_refresh();
        assert_eq!(app.view.files[0].hunks.len(), 2);
    }

    #[test]
    fn maybe_auto_refresh_skips_on_a_read_only_range_target() {
        // A fixed range never changes under the reviewer, so polling is a
        // no-op — even though the (contrived) fake would return a new diff.
        let (mut app, _calls) = app_with_fake(
            vec![raw_patch("a.rs", 1)],
            DiffTarget::Range("HEAD~1..HEAD".to_string()),
            vec![raw_patch("a.rs", 2)],
            vec![],
        );
        app.maybe_auto_refresh();
        assert_eq!(app.view.files[0].hunks.len(), 1, "range target not polled");
        // The guard is what skips it: a direct auto_refresh still applies.
        assert!(app.auto_refresh());
        assert_eq!(app.view.files[0].hunks.len(), 2);
    }

    #[test]
    fn maybe_auto_refresh_skips_while_composing() {
        // Mid-input the diff must not shift under the user; once back in
        // Normal the pending edit is picked up.
        let (mut app, _calls, diff_h, _status_h) = app_with_mutable_fake(
            vec![raw_patch("a.rs", 1)],
            vec![],
            vec![raw_patch("a.rs", 1)],
            vec![],
        );
        *diff_h.borrow_mut() = vec![raw_patch("a.rs", 2)];
        app.mode = Mode::Compose;
        app.maybe_auto_refresh();
        assert_eq!(app.view.files[0].hunks.len(), 1, "skipped while composing");
        app.mode = Mode::Normal;
        app.maybe_auto_refresh();
        assert_eq!(app.view.files[0].hunks.len(), 2);
    }

    #[test]
    fn manual_refresh_applies_the_edit_and_acknowledges_in_the_footer() {
        // `R` always confirms it ran (even mid-input, where auto-refresh is
        // suppressed) and picks up the external edit.
        let (mut app, _calls, diff_h, _status_h) = app_with_mutable_fake(
            vec![raw_patch("a.rs", 1)],
            vec![],
            vec![raw_patch("a.rs", 1)],
            vec![],
        );
        *diff_h.borrow_mut() = vec![raw_patch("a.rs", 2)];
        app.apply(Action::Refresh);
        assert_eq!(app.view.files[0].hunks.len(), 2);
        assert_eq!(app.status_message.as_deref(), Some("refreshed"));
    }

    #[test]
    fn nothing_hides_smoke_stage_two_then_edit_one_reexpands() {
        // Drives the Unit-2 smoke via the FakeGit harness: three files,
        // stage two (watch them collapse), then an external edit lands on a
        // staged file; the next refresh re-expands it with `±`.
        let a = raw_patch("a.rs", 1);
        let b = raw_patch("b.rs", 1);
        let c = raw_patch("c.rs", 1);
        let (mut app, calls, diff_h, status_h) = app_with_mutable_fake(
            vec![a.clone(), b.clone(), c.clone()],
            vec![], // nothing staged initially -> all expanded
            // The fake starts reflecting the state after a.rs is staged.
            vec![b.clone(), c.clone()],
            vec![staged_entry("a.rs")],
        );
        // All three expanded to start.
        assert!(!app.view.is_collapsed("a.rs"));
        assert!(!app.view.is_collapsed("b.rs"));
        assert!(!app.view.is_collapsed("c.rs"));

        // Stage a.rs (cursor on its header): it collapses.
        app.apply(Action::StageFile);
        assert!(app.view.is_collapsed("a.rs"));

        // Now stage b.rs. Update the fake to the post-(a,b)-stage state,
        // then move the cursor onto b.rs and stage it.
        *diff_h.borrow_mut() = vec![c.clone()];
        *status_h.borrow_mut() = vec![staged_entry("a.rs"), staged_entry("b.rs")];
        app.view.cursor = app.view.header_row_of_file[app
            .view
            .files
            .iter()
            .position(|f| f.path == "b.rs")
            .unwrap()];
        app.apply(Action::StageFile);
        assert!(app.view.is_collapsed("b.rs"));
        assert!(app.view.is_collapsed("a.rs"));
        // Two S gestures, both whole-file stages.
        assert_eq!(
            *calls.borrow(),
            vec![
                StageCall::StageFile("a.rs".to_string()),
                StageCall::StageFile("b.rs".to_string()),
            ]
        );

        // External edit lands on the staged a.rs: it is now partially staged
        // and reappears in the working diff. A refresh must re-expand it.
        *diff_h.borrow_mut() = vec![a.clone(), c.clone()];
        *status_h.borrow_mut() = vec![partial_entry("a.rs"), staged_entry("b.rs")];
        app.refresh();

        assert!(!app.view.is_collapsed("a.rs")); // re-expanded — nothing hides
        assert!(app.view.is_collapsed("b.rs")); // still fully staged, collapsed
        assert_eq!(
            app.staged_states.get("a.rs").copied(),
            Some(StagedState::Partial)
        );
        let header = app.view.header_row_of_file[app
            .view
            .files
            .iter()
            .position(|f| f.path == "a.rs")
            .unwrap()];
        let Row::FileHeader { staged_marker, .. } = &app.view.rows[header] else {
            panic!("expected file header");
        };
        assert_eq!(*staged_marker, StagedMarker::Partial); // ±
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

    /// The multibuffer highlights every *expanded* file's in-use sides once
    /// at build time, and the `(path, side)`-keyed cache means later motions
    /// (section jumps back and forth) re-fetch nothing.
    #[test]
    fn multibuffer_highlights_every_expanded_file_once() {
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
            staged_states: HashMap::new(),
        };
        let mut app = App::with_git(snapshot, DiffTarget::Staged, Box::new(fake));
        // Both files expanded -> a.rs (2 sides) + b.rs (2 sides) = 4 fetches.
        assert_eq!(*show_calls.borrow(), 4);

        app.apply(Action::NextFile); // -> b.rs header: cache hit
        app.apply(Action::PrevFile); // -> a.rs header: cache hit
        app.apply(Action::NextFile); // -> b.rs header: cache hit
        assert_eq!(*show_calls.borrow(), 4);
    }

    /// A file that starts collapsed (fully staged at launch) is not
    /// highlighted until it is expanded — the lazy-per-file population rule.
    #[test]
    fn collapsed_file_is_not_highlighted_until_expanded() {
        let a = highlight_patch("a.rs");
        let c = highlight_patch("c.rs");
        let show_calls = Rc::new(RefCell::new(0));
        let fake = FakeGit {
            diff: vec![a.clone(), c.clone()],
            show_calls: Rc::clone(&show_calls),
            show_content: Some("fn old() {}\nfn new() {}\n".to_string()),
            ..FakeGit::default()
        };
        let snapshot = ReviewSnapshot {
            files: vec![
                FileDiff::from_patch(&a).unwrap(),
                FileDiff::from_patch(&c).unwrap(),
            ],
            patches: vec![Some(a), Some(c)],
            // c.rs starts fully staged -> starts collapsed.
            staged: vec![StagedFile {
                path: "c.rs".to_string(),
                letter: 'M',
            }],
            staged_states: HashMap::from([("c.rs".to_string(), StagedState::Full)]),
        };
        let mut app = App::with_git(snapshot, DiffTarget::Staged, Box::new(fake));
        // Only a.rs (expanded) is highlighted; collapsed c.rs is skipped.
        assert_eq!(*show_calls.borrow(), 2);

        // Expanding c.rs highlights its two sides on the rebuild.
        app.view.set_collapsed("c.rs", false);
        app.rebuild_rows();
        assert_eq!(*show_calls.borrow(), 4);
    }

    /// Builds an `App` (Staged target) whose fake reads its refresh diff/status
    /// through mutable handles and counts `show_file` fetches, so a refresh
    /// test can change what the next `refresh` sees and observe whether the
    /// highlight cache was reused or re-fetched. Returns the app, the diff
    /// handle, and the show-call counter.
    #[allow(clippy::type_complexity)]
    fn app_with_counting_fake(
        initial: Vec<RawFilePatch>,
    ) -> (App, Rc<RefCell<Vec<RawFilePatch>>>, Rc<RefCell<usize>>) {
        let show_calls = Rc::new(RefCell::new(0));
        let diff_h = Rc::new(RefCell::new(initial.clone()));
        let status_h = Rc::new(RefCell::new(Vec::new()));
        let fake = FakeGit {
            diff_override: Some(Rc::clone(&diff_h)),
            status_override: Some(Rc::clone(&status_h)),
            show_calls: Rc::clone(&show_calls),
            show_content: Some("fn old() {}\nfn new() {}\n".to_string()),
            ..FakeGit::default()
        };
        let snapshot = ReviewSnapshot {
            files: initial
                .iter()
                .map(|p| FileDiff::from_patch(p).unwrap())
                .collect(),
            patches: initial.into_iter().map(Some).collect(),
            staged: Vec::new(),
            staged_states: HashMap::new(),
        };
        let app = App::with_git(snapshot, DiffTarget::Staged, Box::new(fake));
        (app, diff_h, show_calls)
    }

    #[test]
    fn refresh_preserves_highlight_cache_for_unchanged_files() {
        let a = highlight_patch("a.rs");
        let (mut app, _diff, show_calls) = app_with_counting_fake(vec![a]);
        // a.rs expanded, both sides -> 2 fetches at build.
        assert_eq!(*show_calls.borrow(), 2);
        assert!(app.highlight_cache_contains("a.rs", Side::New));

        // The refresh sees byte-identical diff content -> the cache survives
        // and nothing is re-fetched.
        app.refresh();
        assert_eq!(*show_calls.borrow(), 2);
        assert!(app.highlight_cache_contains("a.rs", Side::New));
        assert!(app.highlight_cache_contains("a.rs", Side::Old));
    }

    #[test]
    fn refresh_invalidates_highlight_cache_for_changed_files() {
        let a = highlight_patch("a.rs");
        let (mut app, diff, show_calls) = app_with_counting_fake(vec![a]);
        assert_eq!(*show_calls.borrow(), 2);

        // The file's diff content changes underneath us (an external edit):
        // its cache entry must be invalidated and both sides re-fetched.
        *diff.borrow_mut() = vec![raw_patch("a.rs", 1)];
        app.refresh();
        assert_eq!(*show_calls.borrow(), 4);
    }

    #[test]
    fn refresh_drops_highlight_cache_entries_for_removed_files() {
        let a = highlight_patch("a.rs");
        let b = highlight_patch("b.rs");
        let (mut app, diff, show_calls) = app_with_counting_fake(vec![a, b]);
        // Two expanded files, two sides each -> 4 fetches.
        assert_eq!(*show_calls.borrow(), 4);
        assert!(app.highlight_cache_contains("b.rs", Side::New));

        // b.rs leaves the review; its cache entries must be dropped (no
        // unbounded growth), while a.rs (unchanged) keeps its cached spans.
        *diff.borrow_mut() = vec![highlight_patch("a.rs")];
        app.refresh();
        assert!(app.highlight_cache_contains("a.rs", Side::New));
        assert!(!app.highlight_cache_contains("b.rs", Side::New));
        assert!(!app.highlight_cache_contains("b.rs", Side::Old));
        // a.rs was a cache hit -> no further fetches.
        assert_eq!(*show_calls.borrow(), 4);
        assert_eq!(app.highlight_cache_len(), 2);
    }

    /// A file whose single hunk carries `pairs` removed/added line pairs
    /// (`2 * pairs` changed lines) of realistic Rust, so the word-diff pairing
    /// (the dominant per-rebuild cost) runs on non-trivial content.
    fn perf_file(i: usize, pairs: usize) -> FileDiff {
        let path = format!("src/module_{i}.rs");
        let mut raw = format!(
            "diff --git a/{path} b/{path}\nindex 1..2 100644\n--- a/{path}\n+++ b/{path}\n@@ -1,{pairs} +1,{pairs} @@\n"
        );
        for k in 0..pairs {
            raw.push_str(&format!(
                "-    let value_{k} = compute_old({k}, factor);\n+    let value_{k} = compute_new({k}, factor);\n"
            ));
        }
        FileDiff::from_patch(&RawFilePatch {
            path,
            old_path: None,
            raw,
            is_binary: false,
        })
        .unwrap()
    }

    /// A full `rebuild_rows` over a ~5k-changed-line, 15-file multibuffer —
    /// the exact work every stage/collapse gesture triggers — must be
    /// imperceptible. Measured well under 10ms (recorded in the perf proof),
    /// so incremental rebuild is unnecessary. The assertion uses a generous
    /// CI-safe bound to stay non-flaky; run with `--nocapture` for the number.
    #[test]
    fn rebuild_rows_on_a_5k_line_multibuffer_is_fast() {
        let files: Vec<FileDiff> = (0..15).map(|i| perf_file(i, 168)).collect();
        let total_lines: usize = files
            .iter()
            .flat_map(|f| f.hunks.iter())
            .map(|h| h.lines.len())
            .sum();
        assert!(
            total_lines >= 5000,
            "fixture should be ~5k changed lines, got {total_lines}"
        );

        let mut app = App::new(files);
        let rows = app.view.rows.len();
        // Warm up once, then average a handful of full rebuilds.
        app.rebuild_rows();
        let iters = 20;
        let start = std::time::Instant::now();
        for _ in 0..iters {
            app.rebuild_rows();
        }
        let per = start.elapsed() / iters;
        println!(
            "rebuild_rows: {per:?} avg over {iters} rebuilds, {rows} rows ({total_lines} changed lines)"
        );
        assert!(
            per < std::time::Duration::from_millis(250),
            "rebuild_rows took {per:?} for {total_lines} lines / {rows} rows"
        );
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

    // -- Search across the multibuffer (task 4.2) ---------------------------

    /// A one-hunk file whose added line contains the pattern `needle`.
    fn needle_file(path: &str) -> FileDiff {
        let raw = format!(
            "diff --git a/{path} b/{path}\nindex 1..2 100644\n--- a/{path}\n+++ b/{path}\n@@ -1,1 +1,1 @@\n-old\n+needle here\n"
        );
        file_with_raw(path, &raw)
    }

    fn confirm_needle_search(app: &mut App) {
        app.apply(Action::Search);
        for c in "needle".chars() {
            app.search_input.push(c);
        }
        app.confirm_search();
    }

    #[test]
    fn search_matches_span_file_boundaries() {
        let mut app = App::new(vec![needle_file("a.rs"), needle_file("b.rs")]);
        confirm_needle_search(&mut app);
        // One match per file's section — the search spans the whole buffer.
        assert_eq!(app.search.matches.len(), 2);
        let f0 = app.view.file_of_row[app.search.matches[0]];
        let f1 = app.view.file_of_row[app.search.matches[1]];
        assert_ne!(f0, f1);
    }

    #[test]
    fn collapsed_section_contributes_no_search_matches() {
        let mut app = App::new(vec![needle_file("a.rs"), needle_file("b.rs")]);
        // a.rs collapsed contributes only its header — its needle row is
        // absent from the buffer, so it cannot match.
        app.view.set_collapsed("a.rs", true);
        app.rebuild_rows();
        confirm_needle_search(&mut app);
        assert_eq!(app.search.matches.len(), 1);
        assert_eq!(app.view.file_of_row[app.search.matches[0]], 1);
    }

    #[test]
    fn search_next_wraps_across_the_whole_buffer() {
        let mut app = App::new(vec![needle_file("a.rs"), needle_file("b.rs")]);
        confirm_needle_search(&mut app);
        let first = app.view.cursor;
        assert_eq!(app.view.file_of_cursor(), 0); // first match in a.rs

        app.apply(Action::SearchNext); // -> b.rs's match
        let second = app.view.cursor;
        assert_ne!(first, second);
        assert_eq!(app.view.file_of_cursor(), 1);

        app.apply(Action::SearchNext); // wraps back to a.rs's match
        assert_eq!(app.view.cursor, first);

        app.apply(Action::SearchPrev); // wraps backward to b.rs's match
        assert_eq!(app.view.cursor, second);
    }

    #[test]
    fn toggling_collapse_recomputes_search_matches_without_stale_indices() {
        let mut app = App::new(vec![needle_file("a.rs"), needle_file("b.rs")]);
        confirm_needle_search(&mut app);
        assert_eq!(app.search.matches.len(), 2);

        // Collapse b.rs via the `za` action; its rows leave the buffer, so
        // the match set must recompute (no stale row indices survive).
        app.apply(Action::NextFile); // cursor onto b.rs's header
        assert_eq!(app.view.file_of_cursor(), 1);
        app.apply(Action::ToggleCollapse);
        assert!(app.view.is_collapsed("b.rs"));

        assert_eq!(app.search.matches.len(), 1);
        for &m in &app.search.matches {
            assert!(m < app.view.rows.len(), "stale match index {m}");
            let Row::Line(l) = &app.view.rows[m] else {
                panic!("match row is not a line row");
            };
            assert!(l.content.contains("needle"));
        }
    }

    // -- Select-by-path seam (task 4.4) -------------------------------------

    #[test]
    fn select_file_by_path_moves_cursor_to_section_header() {
        let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
        assert!(app.select_file_by_path("b.rs"));
        assert_eq!(app.view.cursor, app.view.header_row_of_file[1]);
        assert_eq!(app.view.selected_file, 1);
        assert!(matches!(
            app.view.rows[app.view.cursor],
            Row::FileHeader { file_index: 1, .. }
        ));
    }

    #[test]
    fn select_file_by_path_expands_a_collapsed_target() {
        let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
        app.view.set_collapsed("b.rs", true);
        app.rebuild_rows();
        assert!(app.view.is_collapsed("b.rs"));

        assert!(app.select_file_by_path("b.rs"));
        assert!(!app.view.is_collapsed("b.rs")); // expanded on select
        assert_eq!(app.view.cursor, app.view.header_row_of_file[1]);
        assert_eq!(app.view.selected_file, 1);
    }

    #[test]
    fn select_file_by_path_unknown_path_is_a_noop_returning_false() {
        let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1)]);
        app.apply(Action::CursorDown);
        let cursor_before = app.view.cursor;
        assert!(!app.select_file_by_path("missing.rs"));
        assert_eq!(app.view.cursor, cursor_before);
        assert_eq!(app.view.selected_file, 0);
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

    // -- Git panel focus & navigation --------------------------------------

    /// Builds a panel fixture: two tracked files, one untracked, two stashes.
    fn panel_app() -> App {
        let mut app = App::new(vec![file("a.rs", 1), file("b.rs", 1), file("notes.md", 1)]);
        app.untracked_paths = vec!["notes.md".to_string()];
        app.stashes = vec![
            StashEntry {
                stash_ref: "stash@{0}".to_string(),
                branch: Some("main".to_string()),
                message: "wip".to_string(),
            },
            StashEntry {
                stash_ref: "stash@{1}".to_string(),
                branch: Some("main".to_string()),
                message: "spike".to_string(),
            },
        ];
        app
    }

    #[test]
    fn focus_git_panel_toggles_mode_both_ways() {
        let mut app = panel_app();
        assert_eq!(app.mode, Mode::Normal);
        assert!(!app.git_panel_focused());
        app.apply(Action::FocusGitPanel);
        assert_eq!(app.mode, Mode::Panel);
        assert!(app.git_panel_focused());
        app.apply(Action::FocusGitPanel);
        assert_eq!(app.mode, Mode::Normal);
        assert!(!app.git_panel_focused());
    }

    #[test]
    fn focusing_the_panel_resets_its_cursor_to_top() {
        let mut app = panel_app();
        app.panel_cursor = 3;
        app.apply(Action::FocusGitPanel);
        assert_eq!(app.panel_cursor, 0);
    }

    #[test]
    fn panel_cursor_moves_and_clamps_across_sections() {
        let mut app = panel_app();
        app.apply(Action::FocusGitPanel);
        // 5 navigable rows: a.rs, b.rs, notes.md, stash0, stash1.
        assert_eq!(app.panel_cursor, 0);
        app.apply(Action::PanelCursorUp); // clamps at top
        assert_eq!(app.panel_cursor, 0);
        for _ in 0..10 {
            app.apply(Action::PanelCursorDown);
        }
        assert_eq!(app.panel_cursor, 4); // clamps at bottom (last stash)
        app.apply(Action::PanelCursorUp);
        assert_eq!(app.panel_cursor, 3);
    }

    #[test]
    fn panel_enter_on_file_selects_it_and_returns_focus_to_diff() {
        let mut app = panel_app();
        app.apply(Action::FocusGitPanel);
        app.apply(Action::PanelCursorDown); // -> b.rs (index 1)
        app.apply(Action::PanelSelect);
        assert_eq!(app.view.selected_file, 1);
        assert_eq!(app.mode, Mode::Normal); // focus returned to the diff
        // Selecting scrolls the multibuffer to that file's section header.
        assert_eq!(app.view.cursor, app.view.header_row_of_file[1]);
    }

    #[test]
    fn panel_enter_on_untracked_file_selects_it() {
        let mut app = panel_app();
        app.apply(Action::FocusGitPanel);
        app.panel_cursor = 2; // notes.md, the lone UNTRACKED row
        app.apply(Action::PanelSelect);
        assert_eq!(app.view.selected_file, 2);
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn panel_enter_on_stash_is_a_no_op_and_stays_focused() {
        let mut app = panel_app();
        app.apply(Action::FocusGitPanel);
        app.panel_cursor = 3; // first stash row
        let selected_before = app.view.selected_file;
        app.apply(Action::PanelSelect);
        assert_eq!(app.view.selected_file, selected_before); // unchanged
        assert_eq!(app.mode, Mode::Panel); // still focused on the panel
    }

    #[test]
    fn focus_toggle_is_a_no_op_while_a_modal_owns_the_keyboard() {
        let mut app = panel_app();
        app.mode = Mode::Search;
        app.apply(Action::FocusGitPanel);
        assert_eq!(app.mode, Mode::Search); // unchanged: Search still owns keys
    }
}
