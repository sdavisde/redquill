//! [`App`]: the TUI's state and the pure state transitions every [`Action`]
//! performs. No rendering or terminal I/O lives here — these are plain
//! methods, unit-tested without a terminal.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::annotate::{AnnotationStore, Source, Target};
// `Side` is only referenced by test-only helpers here (and by the `super::*`
// re-export the `tests` module relies on), so gate it to avoid an unused
// import in the production build.
#[cfg(test)]
use crate::annotate::Side;
use crate::diff::FileDiff;
use crate::git::{
    BranchStatus, CommitLogEntry, CommitSummary, DiffTarget, RawFilePatch, RemoteOp, StagingMode,
    StashEntry, commit_command_line, remote_command,
};
use crate::highlight::Highlighter;
use crate::lsp::RequestId;

use super::background::{BackgroundTasks, CommandOutcome, TaskId, run_command};
use super::command_log::{CommandLog, CommandLogEntry};
use super::commit_message::CommitMessageState;
use super::compose::ComposeState;
use super::diff_view_state::DiffViewState;
use super::history::InFlightHistory;
use super::keymap::Action;
use super::lsp_ops::LspClient;
use super::peek::{PeekKind, PeekState};
use super::refresh::InFlightRefresh;
use super::rows::Row;
use super::search::SearchState;
use super::stage_ops::{ReviewSnapshot, StageOps, StagedFile, StagedState};
use super::switcher::SwitcherState;
use super::syntax::HighlightCache;
use super::targeting;
use super::theme::Theme;

/// The git panel's two tabs (spec 05 Unit 3): Changes is the existing
/// CHANGES/UNTRACKED/STASHES panel content; History lists the branch's commit
/// log for opening a historical commit into the main diff view. Carried
/// inside [`Mode::Panel`] (mode-scoped state), not a parallel `App` field,
/// except for [`App::last_panel_tab`] — the deliberate exception documented
/// there for state that must survive the panel losing focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PanelTab {
    /// The CHANGES/UNTRACKED/STASHES sections (the pre-existing panel).
    #[default]
    Changes,
    /// The commit-log list (spec 05 Unit 3).
    History,
}

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
    /// The git panel (sidebar) holds focus: `cursor` navigates the active
    /// tab's rows (an index into that tab's flattened navigable-row list),
    /// bypassing the diff-scope keymap; `tab` selects Changes vs. History
    /// (see [`PanelTab`]). Reset to `cursor: 0` on entry; only exists while
    /// the panel is focused, so it can never carry a stale index while
    /// inactive.
    Panel { cursor: usize, tab: PanelTab },
    /// The search input is open in the footer, composing a pattern.
    Search,
    /// The LSP peek overlay (`gd`/`gr`/`K` results) is open.
    Peek,
    /// The branch/worktree switcher modal (`b`, panel scope) is open.
    Switcher,
    /// The commit-message modal (`c`, panel scope, spec 04) is open.
    CommitMessage,
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
    /// The help overlay's keybind filter (`/`), lazygit-style. `None`: no
    /// filter, the overlay's scroll keys dispatch normally. `Some((query,
    /// editing))`: a filter is active; `editing` is `true` while capturing
    /// free-text keystrokes (just after `/`, or still typing) — during which
    /// scroll keys are inert, like `Mode::Search` — and `false` once `Enter`
    /// locks the query in and hands control back to the scroll keys
    /// (mirroring lazygit: `Enter` commits the filter, a subsequent `Esc`
    /// clears it, and only a second `Esc` closes the overlay). Reset to
    /// `None` wherever help closes or reopens.
    pub(super) help_search: Option<(String, bool)>,
    /// Annotations accumulated this session.
    pub annotations: AnnotationStore,
    /// The current interaction mode.
    pub mode: Mode,
    /// The Compose modal's state, when `mode == Mode::Compose`.
    pub compose: Option<ComposeState>,
    /// The commit-message modal's state, when `mode == Mode::CommitMessage`
    /// (spec 04; see [`super::commit_message`]).
    pub commit_message: Option<CommitMessageState>,
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
    /// A transient one-line message for the status footer (errors, no-op
    /// explanations, success echoes). Cleared on the next keypress.
    pub status_message: Option<String>,
    /// The git backend staging and refresh run through. `None` in
    /// git-less contexts (e.g. pure-navigation unit tests), where staging
    /// degrades to a footer message.
    pub(super) stage_ops: Option<Box<dyn StageOps>>,
    /// The color palette every renderer routes through.
    pub theme: Theme,
    /// The tree-sitter highlighting engine. Owned here so its per-language
    /// config cache persists across selections. `pub(super)` for the
    /// code-intelligence module's peek-preview highlighting.
    pub(super) highlighter: Highlighter,
    /// Highlighted line spans, cached per `(path, side)` and cleared on
    /// every [`App::refresh`] (see [`syntax::HighlightCache`]).
    pub(super) highlight_cache: HighlightCache,
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
    /// The branch/worktree switcher modal's state, `Some` only while
    /// [`Mode::Switcher`] is active (see [`App::open_switcher`] /
    /// [`App::close_switcher`]).
    pub switcher: Option<SwitcherState>,
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
    /// The background-task poller every mutating background git operation
    /// (see [`GitOpKind`]) runs through. Spawning returns immediately;
    /// [`App::poll_git_ops`] drains completed outcomes once per event-loop
    /// tick.
    pub(super) background: BackgroundTasks<CommandOutcome>,
    /// The in-memory, bounded log of every git command redquill ran, rendered
    /// in the toggleable command-log pane.
    pub(super) command_log: CommandLog,
    /// The single mutating background git operation currently in flight, if
    /// any. Enforces the "at most one mutating background git op at a time"
    /// invariant: while this is `Some`, further requests are rejected with a
    /// message rather than queued, and a branch-switch attempt is likewise
    /// blocked (see the switcher's confirm handler in [`super::switcher`]).
    pub(super) git_op: Option<InFlightGitOp>,
    /// Whether the command-log pane is open in the bottom-panel slot. Toggled
    /// with `@` from both the diff view and the focused panel.
    pub(super) command_log_open: bool,
    /// The background-task poller the async working-tree refresh runs through.
    /// Separate from `background` so a mutating git op's and refresh's results
    /// never mix in one drain. Yields `None` when the background read hit a
    /// git error.
    pub(super) refresh_tasks: BackgroundTasks<Option<ReviewSnapshot>>,
    /// The single async refresh currently in flight, if any (single-flight,
    /// like `git_op`). Carries the generation it was spawned at so a
    /// snapshot that predates a foreground refresh is discarded, not applied.
    pub(super) refresh_in_flight: Option<InFlightRefresh>,
    /// Bumped by every synchronous refresh — and therefore by every staging or
    /// remote mutation, which all refresh afterward. An async snapshot is
    /// applied only if this still matches the value captured when it spawned:
    /// the staleness guard that stops a background read from clobbering a
    /// concurrent stage.
    pub(super) refresh_generation: u64,
    /// Commit-log rows loaded so far for the git panel's History tab (spec
    /// 05 Unit 3), newest first, accumulated page by page and never
    /// discarded — re-entering the tab never re-fetches what's already
    /// loaded. Empty until the first background page lands (or forever, in a
    /// git-less context or a repository with no commits).
    pub(super) history: Vec<CommitLogEntry>,
    /// Whether a `git log` page past the last one returned fewer than a full
    /// page — no more history to fetch. Sticky for the session (history never
    /// shrinks).
    pub(super) history_exhausted: bool,
    /// The single background commit-log fetch in flight, if any
    /// (single-flight, mirroring [`InFlightRefresh`]).
    pub(super) history_in_flight: Option<InFlightHistory>,
    /// Bumped whenever previously-loaded history is invalidated, so a
    /// straggling fetch spawned before the bump is dropped on arrival rather
    /// than applied (mirrors `refresh_generation`). Nothing in this task
    /// invalidates history yet, so it starts at `0` and never advances in
    /// production; the guard exists so a future invalidation point (e.g. a
    /// branch switch) has somewhere to hook in without redesigning the
    /// staleness contract, and is directly exercised by tests.
    pub(super) history_generation: u64,
    /// The background-task poller commit-log page fetches run through,
    /// separate from `background`/`refresh_tasks` so their results are
    /// drained independently (see [`App::poll_history`]).
    pub(super) history_tasks: BackgroundTasks<Option<Vec<CommitLogEntry>>>,
    /// Which git-panel tab focusing the panel lands on (see [`PanelTab`]).
    /// Lives here rather than only in [`Mode::Panel`] because it must survive
    /// the panel losing focus — "reopen the panel where you left off" is the
    /// documented exception in `docs/rust-best-practices.md`'s state-design
    /// guidance for state that must outlive mode exit.
    pub(super) last_panel_tab: PanelTab,
    /// The commit currently displayed by a commit view opened from the
    /// History tab (its metadata, looked up from `history` rather than
    /// re-fetched), for [`super::diff_view`]'s header block. `None` for every
    /// other target.
    pub(super) active_commit: Option<CommitLogEntry>,
    /// The suspended prior view, set when a commit view is opened and
    /// restored on return (`Esc`). A struct field — not part of `Mode` —
    /// because it must survive `Mode::Normal` for the life of the commit
    /// view (the same "must survive mode exit" exception `last_panel_tab`
    /// documents). `Some` only while a commit view is open.
    pub(super) suspended_view: Option<SuspendedView>,
}

/// The prior view state suspended while a commit view (opened from the git
/// panel's History tab) is displayed, restored verbatim by
/// [`super::git_panel::App::return_from_commit_view`]. See
/// [`App::suspended_view`] for why this lives in a struct field.
pub(super) struct SuspendedView {
    /// The diff target being reviewed before the commit view opened.
    pub(super) target: DiffTarget,
    /// The full per-view state (files, rows, cursor, scroll, collapse map)
    /// for `target`.
    pub(super) view: DiffViewState,
    /// `target`'s raw patches, index-aligned with `view.files`.
    pub(super) patches: Vec<Option<RawFilePatch>>,
    /// `target`'s staged-file list.
    pub(super) staged: Vec<StagedFile>,
    /// `target`'s per-path staged-state map.
    pub(super) staged_states: HashMap<String, StagedState>,
}

/// Which mutating background git operation is in flight (see
/// [`InFlightGitOp`]): one of the sanctioned remote ops, or a commit
/// (spec 04, `docs/specs/04-spec-commit-staged.md`). A closed enum so a new
/// operation can't be added without updating [`GitOpKind::label`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GitOpKind {
    /// A fetch/pull/push/publish.
    Remote(RemoteOp),
    /// `git commit -m <message>`.
    Commit,
}

impl GitOpKind {
    /// A short label for the running indicator and completion footer
    /// (`"fetch"`, `"pull"`, `"push"`, `"publish"`, `"commit"`).
    pub(super) fn label(self) -> &'static str {
        match self {
            GitOpKind::Remote(op) => op.label(),
            GitOpKind::Commit => "commit",
        }
    }
}

/// A mutating background git operation that has been spawned and is
/// awaiting completion (see [`GitOpKind`]). Its [`TaskId`] correlates the
/// background result back to the operation so a stale or foreign task never
/// clears the guard. At most one may be in flight at a time: a single
/// "mutating background git op" invariant, rather than per-operation guards
/// that must each cross-check the others.
///
/// `command_line` is captured at spawn time (rather than recomputed from
/// `kind` on completion) so the completion handler doesn't need an
/// operation's parameters just to log its command line verbatim.
#[derive(Debug, Clone)]
pub(super) struct InFlightGitOp {
    /// The background task delivering this operation's outcome.
    pub(super) id: TaskId,
    /// Which operation is running (drives the running-indicator/completion
    /// label).
    pub(super) kind: GitOpKind,
    /// The full command line, for the command-log entry this op produces on
    /// completion.
    pub(super) command_line: String,
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
            help_search: None,
            annotations,
            mode: Mode::Normal,
            compose: None,
            commit_message: None,
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
            status_message: None,
            stage_ops: None,
            theme: Theme::default(),
            highlighter: Highlighter::new(),
            highlight_cache: HighlightCache::default(),
            search: SearchState::default(),
            search_input: String::new(),
            repo_root: None,
            peek: None,
            switcher: None,
            lsp: None,
            pending_lsp: None,
            background: BackgroundTasks::new(),
            command_log: CommandLog::new(),
            git_op: None,
            command_log_open: false,
            refresh_tasks: BackgroundTasks::new(),
            refresh_in_flight: None,
            refresh_generation: 0,
            history: Vec::new(),
            history_exhausted: false,
            history_in_flight: None,
            history_generation: 0,
            history_tasks: BackgroundTasks::new(),
            last_panel_tab: PanelTab::default(),
            active_commit: None,
            suspended_view: None,
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
    pub(super) fn refresh_repo_state(&mut self) {
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
    pub(super) fn recompute_untracked(&mut self) {
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

    /// Whether a keyboard-capturing overlay is currently up: the help overlay
    /// (`help_open`), the Compose modal, the LSP peek overlay, the
    /// branch/worktree switcher modal, or the commit-message modal. While
    /// one is, it shadows the diff keymap and `q` is inert — an open overlay
    /// never quits the app. A single predicate so this "is an overlay up?"
    /// check, otherwise spread across `mode` and `help_open`, can't drift
    /// between call sites. The command-log pane is deliberately excluded: it
    /// is a bottom pane, not a full-screen overlay, and never captures `q`.
    pub(super) fn overlay_active(&self) -> bool {
        self.help_open
            || matches!(
                self.mode,
                Mode::Compose | Mode::Peek | Mode::Switcher | Mode::CommitMessage
            )
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
            Action::JumpToTop => self.view.jump_to_top(),
            Action::JumpToBottom => self.view.jump_to_bottom(),
            Action::CursorLeft => self.view.move_column_left(),
            Action::CursorRight => self.view.move_column_right(),
            Action::CursorLineStart => self.view.move_column_to_line_start(),
            Action::CursorLineEnd => self.view.move_column_to_line_end(),
            Action::WordForward => self.view.move_word_forward(),
            Action::WordBackward => self.view.move_word_backward(),
            Action::FullPageDown => self.view.full_page_down(),
            Action::FullPageUp => self.view.full_page_up(),
            Action::NextHunk => self.view.next_hunk(),
            Action::PrevHunk => self.view.prev_hunk(),
            Action::NextFile => self.view.next_section(),
            Action::PrevFile => self.view.prev_section(),
            Action::ToggleCollapse => self.toggle_collapse(),
            Action::RecenterCursor => self.view.recenter_cursor(),
            Action::ScrollCursorTop => self.view.scroll_cursor_top(),
            Action::ScrollCursorBottom => self.view.scroll_cursor_bottom(),
            Action::ToggleHelp => {
                self.help_open = !self.help_open;
                self.help_scroll.set(0);
                self.help_search = None;
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
            Action::SearchWordForward => self.search_word_under_cursor(true),
            Action::SearchWordBackward => self.search_word_under_cursor(false),
            Action::GotoDefinition => super::code_intel::request(self, PeekKind::Definition),
            Action::GotoReferences => super::code_intel::request(self, PeekKind::References),
            Action::Hover => super::code_intel::request(self, PeekKind::Hover),
            Action::FocusGitPanel => self.toggle_git_panel(),
            Action::PanelCursorDown => self.panel_move_down(),
            Action::PanelCursorUp => self.panel_move_up(),
            Action::PanelSelect => self.panel_select(),
            Action::TogglePanelTab => self.toggle_panel_tab(),
            Action::RemoteFetch => self.request_remote_op(RemoteOp::Fetch),
            Action::RemotePull => self.request_remote_op(RemoteOp::Pull),
            Action::RemotePush => self.request_remote_op(self.remote_push_op()),
            Action::CommitStaged => self.open_commit_message(),
            Action::OpenSwitcher => self.open_switcher(),
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
        if self.target.staging_mode() == StagingMode::ReadOnly {
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
        targeting::target_for_cursor(file, &self.view.rows, self.view.cursor)
    }

    /// The annotation target for a [`Mode::Visual`] selection between
    /// `anchor` and the cursor. Gathers the selected file and cursor and
    /// delegates to [`targeting::target_for_visual`].
    pub fn target_for_visual(&self, anchor: usize) -> Option<Target> {
        let file = self.view.files.get(self.view.file_of_cursor())?;
        targeting::target_for_visual(file, &self.view.rows, self.view.cursor, anchor)
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
    pub(super) fn open_compose_for(&mut self, id: usize) {
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
                let source = self.annotation_source();
                let _ = self.annotations.add_with_source(
                    compose.target,
                    compose.classification,
                    &body,
                    source,
                );
            }
        }
        self.mode = Mode::Normal;
        self.refresh_rows();
    }

    /// Derives the [`Source`] to record for an annotation composed against
    /// the current view: the active [`DiffTarget`]'s kind, using
    /// `active_commit`'s already-`core.abbrev`-aware short SHA for a commit
    /// target rather than having `annotate/` (or this method) recompute an
    /// abbreviation of its own. Falls back to the full rev string if a
    /// commit is somehow open with no matching `active_commit` entry (should
    /// not happen in practice — `open_commit_view` always sets both
    /// together — but a defensive fallback costs one line and avoids a
    /// panic path per the repository's error-handling rules).
    fn annotation_source(&self) -> Source {
        match &self.target {
            DiffTarget::WorkingTree => Source::WorkingTree,
            DiffTarget::Staged => Source::Staged,
            DiffTarget::Range(spec) => Source::Range(spec.clone()),
            DiffTarget::Commit(sha) => {
                let short_sha = self
                    .active_commit
                    .as_ref()
                    .filter(|commit| &commit.sha == sha)
                    .map(|commit| commit.short_sha.clone())
                    .unwrap_or_else(|| sha.clone());
                Source::Commit(short_sha)
            }
        }
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

    // -- Remote operations & command log ----------------------------------

    /// Toggles the command-log pane in the bottom-panel slot.
    fn toggle_command_log(&mut self) {
        self.command_log_open = !self.command_log_open;
    }

    /// The label of the mutating background git op currently in flight (a
    /// remote op or a commit), if any — drives the running indicator.
    /// `None` when nothing is running.
    pub fn running_op_label(&self) -> Option<&'static str> {
        self.git_op.as_ref().map(|o| o.kind.label())
    }

    /// The concrete operation the push keybind ([`Action::RemotePush`]) runs
    /// right now: [`RemoteOp::Publish`] when the current branch has no
    /// upstream configured (the first push must create the remote branch and
    /// set it as upstream), plain [`RemoteOp::Push`] otherwise. Detached HEAD
    /// and unknown branch state fall back to plain push, whose own git error
    /// is the clearest signal in those states. The single place this question
    /// is answered — the footer hint and the panel's keybind line relabel
    /// from it via [`App::push_publishes`], so the label can never disagree
    /// with what the key does.
    pub(super) fn remote_push_op(&self) -> RemoteOp {
        match &self.branch {
            Some(b) if !b.detached && b.upstream.is_none() => RemoteOp::Publish,
            _ => RemoteOp::Push,
        }
    }

    /// Whether the push keybind currently publishes (see
    /// [`App::remote_push_op`]) — the predicate the footer strip and the git
    /// panel's keybind line use to pick the `publish` label.
    pub(super) fn push_publishes(&self) -> bool {
        self.remote_push_op() == RemoteOp::Publish
    }

    /// Requests a remote operation (`fetch`/`pull`/`push`/`publish`),
    /// spawning it on a
    /// background thread so the render loop never blocks. Enforces the
    /// single-in-flight guard covering every mutating background git op (see
    /// [`GitOpKind`]): if one is already running the request is rejected
    /// with a status message and nothing is spawned. Without a known
    /// repository root (git-less contexts) the request degrades to a
    /// message, like every other git-backed gesture.
    ///
    /// The child command is a fixed argv with `GIT_TERMINAL_PROMPT=0` (see
    /// [`crate::git::remote_command`]); no shell, no `--force`, no credential
    /// handling.
    pub(super) fn request_remote_op(&mut self, op: RemoteOp) {
        if let Some(label) = self.running_op_label() {
            self.set_status_message(format!("{label} already running — wait for it to finish"));
            return;
        }
        let Some(root) = self.repo_root.clone() else {
            self.set_status_message("remote operations unavailable (no repository)");
            return;
        };
        let mut command = remote_command(op, &root);
        let id = self.background.spawn(move || run_command(&mut command));
        self.git_op = Some(InFlightGitOp {
            id,
            kind: GitOpKind::Remote(op),
            command_line: op.command_line(),
        });
        self.set_status_message(format!("{}\u{2026}", op.label()));
    }

    /// Requests a commit of the currently staged changes (spec 04 Unit 2):
    /// `git commit -m <message>`, spawned on the same background pool and
    /// single-flight guard [`App::request_remote_op`] uses (see
    /// [`GitOpKind`]) — rejected with a footer message while a remote op or
    /// another commit is already in flight. Without a git backend the
    /// request degrades to a message, like every other git-backed gesture.
    /// Returns whether the commit was actually spawned, so the modal's
    /// submit handler can keep the typed message on a rejection. Callers
    /// validate the message is non-blank first (see
    /// [`App::submit_commit_message`]).
    ///
    /// The child command is a fixed argv (`["commit", "-m", message]`) with
    /// `GIT_TERMINAL_PROMPT=0`, built behind the [`StageOps`] seam (see
    /// [`crate::git::commit_command`]): the message is passed verbatim
    /// (newlines preserved) as a single argv element — no shell — and no
    /// flag beyond `-m` is ever possible, so hooks run normally (never
    /// `--no-verify`) and the user's git config (signing, sign-off) applies.
    pub(super) fn request_commit(&mut self, message: &str) -> bool {
        if let Some(label) = self.running_op_label() {
            self.set_status_message(format!("{label} already running — wait for it to finish"));
            return false;
        }
        let Some(mut command) = self.stage_ops().and_then(|ops| ops.commit_command(message)) else {
            self.set_status_message("commit unavailable (no git backend)");
            return false;
        };
        let id = self.background.spawn(move || run_command(&mut command));
        self.git_op = Some(InFlightGitOp {
            id,
            kind: GitOpKind::Commit,
            command_line: commit_command_line(message),
        });
        self.set_status_message("commit\u{2026}");
        true
    }

    /// Drains completed mutating background git ops — remote ops and commits
    /// alike (once per event-loop tick, alongside
    /// [`super::code_intel::poll`]). For the in-flight op's result it appends
    /// a [`CommandLogEntry`], clears the guard, re-runs the full refresh
    /// (diff/status plus branch/stash reads — which for a successful commit
    /// also moves the committed files out of CHANGES and updates the
    /// last-commit line and ahead/behind counts), and sets a success/failure
    /// footer summary. Foreign or stale task ids are ignored.
    pub(super) fn poll_git_ops(&mut self) {
        let done = self.background.poll();
        for (id, result) in done {
            let Some(in_flight) = self.git_op.as_ref() else {
                continue;
            };
            if in_flight.id != id {
                continue;
            }
            let kind = in_flight.kind;
            let command_line = in_flight.command_line.clone();
            self.git_op = None;

            let entry = match result {
                Ok(outcome) => CommandLogEntry {
                    command_line,
                    success: outcome.success,
                    code: outcome.code,
                    stdout: outcome.stdout,
                    stderr: outcome.stderr,
                },
                Err(panic) => CommandLogEntry {
                    command_line,
                    success: false,
                    code: None,
                    stdout: String::new(),
                    stderr: panic.message,
                },
            };
            let success = entry.success;
            self.command_log.push(entry);

            // Re-read the working tree so the changes list, branch header, and
            // ahead/behind reflect the op; staged markers and annotations
            // survive exactly as they do after any refresh.
            self.refresh();

            if success {
                self.set_status_message(format!("{} succeeded", kind.label()));
            } else {
                self.set_status_message(format!(
                    "{} failed \u{2014} see command log (@)",
                    kind.label()
                ));
            }
        }
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

    /// Applies the `*`/`#` gesture: seeds a new search pattern from the word
    /// under the column cursor, then jumps to the next (`*`) or previous
    /// (`#`) occurrence via [`App::search_advance`] — same footer messages,
    /// same wraparound. `"no word under cursor"` if the cursor isn't
    /// directly on a word char (see [`DiffViewState::word_at_cursor`]).
    fn search_word_under_cursor(&mut self, forward: bool) {
        let Some(word) = self.view.word_at_cursor() else {
            self.set_status_message("no word under cursor");
            return;
        };
        self.search.pattern = Some(word);
        self.search.recompute(&self.view.rows);
        self.search_advance(forward);
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
            | Action::CursorLineStart
            | Action::CursorLineEnd
            | Action::WordForward
            | Action::WordBackward
            | Action::RecenterCursor
            | Action::ScrollCursorTop
            | Action::ScrollCursorBottom
            | Action::EnterVisual
            | Action::Compose
            | Action::ToggleList
            | Action::ToggleStage
            | Action::ToggleStagingPanel
            | Action::ToggleHelp
            | Action::ToggleCommandLog
    )
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "perf_tests.rs"]
mod perf_tests;
