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
use crate::config::{Config, ConfigWarning};
use crate::diff::FileDiff;
use crate::git::{
    BranchStatus, CommitLogEntry, CommitSummary, DiffTarget, RawFilePatch, RemoteOp, StagingMode,
    StashEntry, commit_command_line, remote_command,
};
use crate::highlight::Highlighter;
use crate::lsp::RequestId;
use crate::review::ReviewStatus;

use super::background::{BackgroundTasks, CommandOutcome, TaskId, run_command};
use super::command_log::{CommandLog, CommandLogEntry};
use super::commit_message::CommitMessageState;
use super::compose::ComposeState;
use super::diff_view_state::DiffViewState;
use super::editor::EditorLaunch;
use super::file_finder::{FinderState, InFlightFinderLoad};
use super::history::InFlightHistory;
use super::keymap::Action;
use super::lsp_ops::LspClient;
use super::peek::{PeekKind, PeekState};
use super::project_search::ProjectSearchState;
use super::refresh::InFlightRefresh;
use super::review_branch::ReviewBranchState;
use super::rows::Row;
use super::search::SearchState;
use super::stage_ops::{ReviewSnapshot, StageOps, StagedFile, StagedState};
use super::switcher::SwitcherState;
use super::syntax::HighlightCache;
use super::targeting;
use super::theme::Theme;
use crate::search::FileCandidate;

/// The git panel's two tabs: Changes is the existing CHANGES/UNTRACKED/
/// STASHES panel content; History lists the branch's commit log for opening
/// a historical commit into the main diff view. Carried inside
/// [`Mode::Panel`] (mode-scoped state), not a parallel `App` field, except
/// for [`App::last_panel_tab`] — the deliberate exception documented there
/// for state that must survive the panel losing focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PanelTab {
    /// The CHANGES/UNTRACKED/STASHES sections (the pre-existing panel).
    #[default]
    Changes,
    /// The commit-log list.
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
    /// The review-branch modal (`R`, panel scope) is open: lists local
    /// branches (excluding the one currently checked out) so the user can
    /// start a review session in place, styled and behaved like
    /// [`Mode::Switcher`]'s Branches tab (see
    /// [`super::review_branch::ReviewBranchState`]). Its own mode rather
    /// than a third switcher tab, since confirming here resolves a base ref
    /// and ensures a managed worktree exists instead of switching onto an
    /// already-checked-out ref.
    ReviewBranch,
    /// The commit-message modal (`c`, panel scope) is open.
    CommitMessage,
    /// The fuzzy file finder overlay (`gp`) is open. The read-only file view
    /// it opens into is *not* a separate mode — it's [`Mode::Normal`] over a
    /// [`crate::git::DiffTarget::File`] target (see [`super::file_view`]).
    Finder,
    /// The full-screen Project Search view (`g/`) is open. Unlike the commit
    /// view / file view, opening it never touches `view`/`target` (it has
    /// its own dedicated state — see
    /// [`super::project_search::ProjectSearchState`]), so `Esc` back to the
    /// diff needs no suspend/restore beyond flipping the mode back to
    /// whatever it was captured as on open. The read-only file view a hit's
    /// `Enter` opens into *is* a nested suspension (same mechanism as
    /// [`Mode::Finder`]), landing back here — not `Mode::Normal` — so the
    /// query/toggles/results/selection survive the round trip (see
    /// [`App::file_view_return_mode`]).
    ProjectSearch,
    /// The end-review modal (`q` in a review session) is open: pause /
    /// finish / cancel. `origin` is where `q` was pressed from — `Cancel`
    /// restores it exactly. This is the state-design exception documented
    /// on [`EndReviewOrigin`]: it would ordinarily be a struct field ("must
    /// survive mode exit"), but since it only matters for *this* mode's
    /// lifetime, carrying it as the variant's own payload keeps it from
    /// going stale as a field while every other mode is active. `cursor` is
    /// the `j`/`k`-highlighted option (0 = Pause, 1 = Finish, 2 = Cancel —
    /// the modal's display order), reset to `0` on open; the pre-existing
    /// `p`/`f`/`c` mnemonics dispatch immediately regardless of `cursor`.
    EndReview {
        origin: EndReviewOrigin,
        cursor: usize,
    },
    /// The pull/push confirm modal (`p`/`P` in a review session) is open:
    /// confirming this specific remote-writing op against the branch under
    /// review is the confirm-first guard `p`/`P` gain during a review (`f`
    /// fetch stays unprompted — see
    /// [`super::modes::handle_panel_key`]). Only ever opened from the
    /// focused git panel (`p`/`P` are panel-scope bindings — see
    /// [`RemoteOp`]'s import), so `cursor`/`tab` are exactly what `Esc` or a
    /// confirmed op restores [`Mode::Panel`] to; `op` is the operation a
    /// confirm actually runs — resolved once at open time (mirroring
    /// [`App::remote_push_op`]'s own resolution point), not re-derived at
    /// confirm time, so the modal's own question text and the op it runs
    /// can never disagree.
    ConfirmRemoteOp {
        op: RemoteOp,
        cursor: usize,
        tab: PanelTab,
    },
}

/// Where `q` was pressed from, carried by [`Mode::EndReview`] so its Cancel
/// gesture can restore the exact prior mode. A dedicated small enum rather
/// than `Box<Mode>` recursion: [`Mode`] derives `Copy` (every call site that
/// matches `app.mode` by value depends on that), and a `Box` field would
/// remove it crate-wide. `q` is only ever intercepted from these three
/// contexts (see [`super::quit_action`]/[`super::modes::handle_panel_key`]),
/// so this closed enum covers every case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReviewOrigin {
    Normal,
    Visual { anchor: usize },
    Panel { cursor: usize, tab: PanelTab },
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
    /// (see [`super::commit_message`]).
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
    /// Files with staged changes, per the latest `git status` refresh — the
    /// local staging panel's list. During a review session this field is
    /// dual-purposed for the accepted-files panel instead (see
    /// `super::review_ops`); the two never overlap since a review session's
    /// `git status` is always clean.
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
    /// Per-path [`ReviewStatus`] driving the accept/defer markers and the
    /// review banner's progress count (see [`super::review_ops`]),
    /// mirroring how `staged_states` drives the
    /// `●`/`±` markers. Missing entries are [`ReviewStatus::Unreviewed`].
    /// Only ever grows outside its default empty state during a review
    /// session — `Space`/`S`/`d` only ever produce a review-status change
    /// while [`App::in_review_session`] holds (see `super::review_ops`'s
    /// self-guards), so a plain working-tree/staged/range session leaves
    /// this permanently empty.
    pub review_states: HashMap<String, ReviewStatus>,
    /// The focused row index into `staged` in the staging panel.
    pub staging_cursor: usize,
    /// A transient one-line message for the status footer (errors, no-op
    /// explanations, success echoes). Cleared on the next keypress.
    pub status_message: Option<String>,
    /// The config loaded once at startup via [`App::set_config`]. Defaults
    /// to [`Config::default()`] — today's shipped behavior — for every
    /// `App` built without that call (every pre-existing unit test).
    pub config: Config,
    /// Problems encountered loading `config`, shown in the dismissible
    /// status-line notice (see [`App::config_warning_notice`]) and never
    /// printed to stdout (stdout is reserved for the annotation markdown).
    pub config_warnings: Vec<ConfigWarning>,
    /// Whether the user has dismissed the config-warning notice this
    /// session (`!`, [`Action::DismissConfigWarning`]). Config loads exactly
    /// once at startup, so this never needs to reset mid-session.
    pub config_warning_dismissed: bool,
    /// Every modal mode's effective key table — [`super::modal_keys`]'s
    /// compiled-in defaults with `[keys.<mode>]` config overrides already
    /// applied, built exactly once via [`App::set_modal_keys`] alongside the
    /// main keymap in [`super::run`].
    /// Defaults to [`super::modal_keys::ModalKeymaps::default`] (the
    /// unmodified compiled-in tables) for every `App` built without that
    /// call, matching `config`'s own default-to-shipped-behavior contract.
    pub(super) modal_keys: super::modal_keys::ModalKeymaps,
    /// The git backend staging and refresh run through. `None` in
    /// git-less contexts (e.g. pure-navigation unit tests), where staging
    /// degrades to a footer message.
    pub(super) stage_ops: Option<Box<dyn StageOps>>,
    /// The color palette every renderer routes through.
    pub theme: Theme,
    /// The editor `g<Space>` suspends the TUI to open: either a
    /// `[editor]`-config template (`EditorLaunch::Template`, e.g. from
    /// `preset = "zed"`) or a plain command (`EditorLaunch::Command`, e.g.
    /// `"nvim"` or `"code --wait"`). Resolved once in `main` via
    /// `resolve_editor`'s five-tier precedence (`--editor` flag > `[editor]`
    /// config > `$VISUAL` > `$EDITOR` > `"nvim"`) and set via
    /// [`App::set_editor`]; defaults to [`EditorLaunch::default`] (today's
    /// `"nvim"` fallback) so pure-navigation unit tests that build an `App`
    /// directly (bypassing `main`'s resolution) still have a usable default.
    pub editor: EditorLaunch,
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
    /// The review-branch modal's state, `Some` only while
    /// [`Mode::ReviewBranch`] is active (see
    /// [`super::review_branch::App::open_review_branch_modal`] /
    /// [`super::review_branch::App::close_review_branch_modal`]). Named
    /// distinctly from [`App::review_branch`] (the *existing* method naming
    /// the branch under review) so the field and the predicate can never be
    /// confused at a call site.
    pub review_branch_modal: Option<ReviewBranchState>,
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
    /// Commit-log rows loaded so far for the git panel's History tab,
    /// newest first, accumulated page by page and never discarded —
    /// re-entering the tab never re-fetches what's already loaded. Empty
    /// until the first background page lands (or forever, in a git-less
    /// context or a repository with no commits).
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
    /// than applied (mirrors `refresh_generation`). Stays at `0` in
    /// production today; exists so a future invalidation point has
    /// somewhere to hook in, and is directly exercised by tests.
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
    /// The fuzzy file finder overlay's state, `Some` only while
    /// [`Mode::Finder`] is active (see [`App::open_finder`] /
    /// [`App::close_finder`]).
    pub(super) finder: Option<FinderState>,
    /// The background-task poller the finder's candidate-list load runs
    /// through, separate from the other pollers so its results are drained
    /// independently (see [`App::poll_finder`]).
    pub(super) finder_tasks: BackgroundTasks<Option<Vec<FileCandidate>>>,
    /// The single background candidate-list load currently in flight, if
    /// any (single-flight, mirroring [`InFlightHistory`]).
    pub(super) finder_in_flight: Option<InFlightFinderLoad>,
    /// Bumped every time the finder opens, so a straggling load spawned by a
    /// previous open (closed and reopened quickly) is dropped on arrival
    /// rather than applied to the new session (mirrors `refresh_generation`).
    pub(super) finder_generation: u64,
    /// The suspended prior view, set when the read-only file view is opened
    /// and restored on return (`Esc`). Independent of `suspended_view`
    /// (commit views): the two nest one layer at a time rather than
    /// sharing a slot — see `ui::file_view`'s module doc.
    pub(super) suspended_file_view: Option<SuspendedView>,
    /// The mode `Esc` restores when the read-only file view closes (see
    /// [`App::return_from_file_view`]): `Mode::Normal` for every opener
    /// except Project Search's confirm gesture, which opens a hit while
    /// already in `Mode::ProjectSearch` and wants
    /// its query/toggles/results/selection to survive the round trip.
    /// Captured only on the first-level open, mirroring
    /// `suspended_file_view`'s own nested-open rule (a second file opened
    /// without returning must not overwrite the true restore target);
    /// meaningless while `suspended_file_view` is `None`.
    pub(super) file_view_return_mode: Mode,
    /// The Project Search full-screen view's state, `Some`
    /// from [`App::open_project_search`] until [`App::close_project_search`]
    /// — kept alive (and untouched) while a hit's file view is showing on
    /// top, so `Esc` from that file view resumes with everything intact.
    pub(super) project_search: Option<ProjectSearchState>,
    /// A git backend rooted *outside* the managed review worktree, used only
    /// for `git worktree remove`/`prune` at finish time. `stage_ops` is
    /// rooted *inside* the worktree for a review session (so
    /// diff/LSP/staging-panel reads are truthful against it) — but git may
    /// refuse to remove a worktree the calling process/runner sits in, so
    /// finish must run through a separate handle rooted at the original
    /// repository instead. `None` outside a review session, or in
    /// git-less/test contexts that never call finish.
    pub(super) review_origin_ops: Option<Box<dyn StageOps>>,
    /// The path `<git-common-dir>/redquill/review-state.json` resolves to
    /// for this session, set once at startup by
    /// [`App::set_review_state_path`]. `None` outside a review session (or
    /// in git-less/test contexts), in which case every persistence gesture
    /// degrades to a no-op — see [`App::persist_review_state`].
    pub(super) review_state_path: Option<PathBuf>,
    /// The blob SHA each currently `Accepted`/`ChangedSinceAccepted` path in
    /// `review_states` was accepted at, mirrored 1:1 alongside
    /// `review_states` so [`App::persist_review_state`] can write it back
    /// out and reconciliation on the *next* session can compare against it.
    /// Missing entries mean "no blob to record" (an accepted deletion), not
    /// "unknown".
    pub(super) review_blob_shas: HashMap<String, Option<String>>,
    /// The background-task poller [`App::persist_review_state`] spawns each
    /// save on, so the write never blocks the render loop. Drained once per
    /// tick by [`App::poll_review_save`]; a failed save surfaces as a status
    /// message rather than blocking or losing the in-memory state.
    pub(super) review_save_tasks: BackgroundTasks<Result<(), String>>,
    /// The count of review-state saves spawned but not yet drained by
    /// [`App::poll_review_save`], so tests (and a future quit-safety wait)
    /// can check whether every in-flight save has landed.
    pub(super) review_saves_pending: u32,
    /// Single-flight guard: bursts of review-state changes coalesce into one
    /// in-flight background write plus at most one follow-up.
    pub(super) review_save_in_flight: bool,
    /// Set when a save is requested while [`App::review_save_in_flight`] is
    /// already true; cleared by [`App::poll_review_save`], which spawns
    /// exactly one more save (capturing current state) whenever it drains a
    /// result and finds this set.
    pub(super) review_save_dirty: bool,
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
/// [`InFlightGitOp`]): one of the sanctioned remote ops, or a commit. A
/// closed enum so a new operation can't be added without updating
/// [`GitOpKind::label`].
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
            review_states: HashMap::new(),
            staging_cursor: 0,
            status_message: None,
            config: Config::default(),
            config_warnings: Vec::new(),
            config_warning_dismissed: false,
            modal_keys: super::modal_keys::ModalKeymaps::default(),
            stage_ops: None,
            theme: Theme::default(),
            editor: EditorLaunch::default(),
            highlighter: Highlighter::new(),
            highlight_cache: HighlightCache::default(),
            search: SearchState::default(),
            search_input: String::new(),
            repo_root: None,
            peek: None,
            switcher: None,
            review_branch_modal: None,
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
            finder: None,
            finder_tasks: BackgroundTasks::new(),
            finder_in_flight: None,
            finder_generation: 0,
            suspended_file_view: None,
            file_view_return_mode: Mode::Normal,
            project_search: None,
            review_origin_ops: None,
            review_state_path: None,
            review_blob_shas: HashMap::new(),
            review_save_tasks: BackgroundTasks::new(),
            review_saves_pending: 0,
            review_save_in_flight: false,
            review_save_dirty: false,
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

    /// Sets the editor `g<Space>` opens (resolved by `main` per
    /// `resolve_editor`'s five-tier precedence: the `--editor` flag,
    /// `[editor]` config, then `$VISUAL`, then `$EDITOR`, then `"nvim"`).
    pub fn set_editor(&mut self, editor: EditorLaunch) {
        self.editor = editor;
    }

    /// Whether the active diff target is a branch review session: gates the
    /// banner, `q`'s end-review-modal behavior, and the accept/defer keys.
    /// One named predicate so "is this a review?" can't be answered
    /// inconsistently across call sites.
    pub(super) fn in_review_session(&self) -> bool {
        matches!(self.target, DiffTarget::Review { .. })
    }

    /// The branch under review, when [`App::in_review_session`] is true.
    pub(super) fn review_branch(&self) -> Option<&str> {
        match &self.target {
            DiffTarget::Review { branch, .. } => Some(branch.as_str()),
            _ => None,
        }
    }

    /// The review banner's `(accepted, total)` progress count: `accepted`
    /// counts files whose [`App::review_status`] is
    /// [`ReviewStatus::Accepted`]; `total` is the file count. `review_states`
    /// is only ever non-empty during a review session (see its doc), so this
    /// is naturally `(0, len)` everywhere else.
    pub(super) fn review_progress(&self) -> (usize, usize) {
        let accepted = self
            .view
            .files
            .iter()
            .filter(|f| self.review_status(&f.path) == ReviewStatus::Accepted)
            .count();
        (accepted, self.view.files.len())
    }

    /// Attaches the origin-rooted backend [`App::finish_review`] runs
    /// `worktree_remove`/`worktree_prune` through (see
    /// [`App::review_origin_ops`]'s doc for why it must be a separate handle
    /// from `stage_ops`). Only meaningful for a review session; callers
    /// outside one simply never call `finish_review`.
    pub fn set_review_origin_ops(&mut self, ops: Box<dyn StageOps>) {
        self.review_origin_ops = Some(ops);
    }

    /// Sets the path this session persists review progress to
    /// (`<git-common-dir>/redquill/review-state.json`), resolved once by
    /// `main`'s review-session bootstrap before the first render. Every
    /// persistence gesture ([`App::persist_review_state`]) is a no-op
    /// without this — outside a review session, or in a git-less/test
    /// context, nothing is ever written.
    pub fn set_review_state_path(&mut self, path: PathBuf) {
        self.review_state_path = Some(path);
    }

    /// Seeds `review_states`/`review_blob_shas` from a freshly loaded and
    /// reconciled persisted review, applying the matching initial collapse
    /// state the same way [`App::with_git`] seeds it for staged files:
    /// `Accepted`/`Deferred` start collapsed; `ChangedSinceAccepted` starts
    /// **expanded** to draw the reviewer's eye back to what changed. Called
    /// once at session start, before the first render.
    pub fn set_review_states(
        &mut self,
        states: HashMap<String, ReviewStatus>,
        blob_shas: HashMap<String, Option<String>>,
    ) {
        for (path, status) in &states {
            let collapse = matches!(status, ReviewStatus::Accepted | ReviewStatus::Deferred);
            self.view.set_collapsed(path, collapse);
        }
        self.review_states = states;
        self.review_blob_shas = blob_shas;
        self.rebuild_rows();
    }

    /// Drains completed background review-state saves, once per event-loop
    /// tick alongside [`App::poll_git_ops`]. A failed save surfaces as a
    /// status message rather than rolling back in-memory state, so the next
    /// status change's save simply retries with current data. Clears
    /// [`App::review_save_in_flight`] on every drain and, if
    /// [`App::review_save_dirty`] was set while that save was running,
    /// immediately spawns exactly one follow-up via
    /// [`App::persist_review_state`].
    pub(super) fn poll_review_save(&mut self) {
        for (_, result) in self.review_save_tasks.poll() {
            self.review_saves_pending = self.review_saves_pending.saturating_sub(1);
            self.review_save_in_flight = false;
            match result {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    self.set_status_message(format!("review state save failed: {e}"));
                }
                Err(panic) => {
                    self.set_status_message(format!("review state save failed: {}", panic.message));
                }
            }
        }
        if !self.review_save_in_flight && self.review_save_dirty {
            self.review_save_dirty = false;
            self.persist_review_state();
        }
    }

    /// Sets the loaded config and any warnings collected while loading it
    /// (`main`'s one-shot call, before the first render — see
    /// `crate::config::load`; there is no reload path). `config.search`
    /// seeds the *next* Project Search session's startup toggles (see
    /// [`super::project_search::ProjectSearchState::seeded`]); an
    /// already-open session is untouched, per the FR.
    pub fn set_config(&mut self, config: Config, warnings: Vec<ConfigWarning>) {
        self.config = config;
        self.config_warnings = warnings;
        self.config_warning_dismissed = false;
    }

    /// Dismisses the config-warning notice (`!`,
    /// [`Action::DismissConfigWarning`]). A no-op once already dismissed or
    /// when there was nothing to show.
    pub fn dismiss_config_warning(&mut self) {
        self.config_warning_dismissed = true;
    }

    /// Whether the config-warning notice should currently render: at least
    /// one warning was collected and the user hasn't dismissed it this
    /// session.
    pub fn config_warning_visible(&self) -> bool {
        !self.config_warning_dismissed && !self.config_warnings.is_empty()
    }

    /// The notice text for the status line: the first warning's message,
    /// plus `"(and N more)"` when there is more than one. `None` when
    /// [`App::config_warning_visible`] is `false`.
    pub fn config_warning_notice(&self) -> Option<String> {
        if !self.config_warning_visible() {
            return None;
        }
        let first = self.config_warnings.first()?;
        let rest = self.config_warnings.len() - 1;
        Some(if rest > 0 {
            format!("config: {first} (and {rest} more)")
        } else {
            format!("config: {first}")
        })
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
    /// current diff. This is the narrow select-by-path seam the git panel
    /// drives file selection through; the sidebar highlight follows the
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
    /// `Quit`, `QuitDiscard`, and `OpenEditor` are no-ops here — the event
    /// loop intercepts them before they reach `apply` (ending the session, or
    /// suspending the TUI to spawn the configured editor, respectively). In
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
            Action::OpenReviewBranch => self.open_review_branch_modal(),
            Action::OpenFileFinder => self.open_finder(),
            Action::OpenProjectSearch => self.open_project_search(),
            Action::ToggleCommandLog => self.toggle_command_log(),
            Action::Refresh => self.manual_refresh(),
            Action::DismissConfigWarning => self.dismiss_config_warning(),
            Action::ToggleAccept => self.toggle_accept_file(),
            Action::AcceptFile => self.accept_file(),
            Action::ToggleDefer => self.toggle_defer_file(),
            // `Quit`/`QuitDiscard` end the session; `OpenEditor` suspends the
            // TUI to spawn the configured editor. Both are intercepted by
            // `super::dispatch_key` before reaching here (see `Action::Quit`'s
            // doc comment), so this is a no-op the same way theirs is.
            Action::Quit | Action::QuitDiscard | Action::OpenEditor => {}
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
    ///
    /// When the active [`DiffTarget`] is [`DiffTarget::File`] (the read-only
    /// file view), the derived target is routed through
    /// [`targeting::as_worktree_target`] so it lands on the `(=)`
    /// "current worktree file content" forms rather than a diff-shaped
    /// `Line`/`Range`/`Hunk` target.
    pub fn target_for_cursor(&self) -> Option<Target> {
        let file = self.view.files.get(self.view.file_of_cursor())?;
        let target = targeting::target_for_cursor(file, &self.view.rows, self.view.cursor)?;
        Some(self.maybe_as_worktree_target(target))
    }

    /// The annotation target for a [`Mode::Visual`] selection between
    /// `anchor` and the cursor. Gathers the selected file and cursor and
    /// delegates to [`targeting::target_for_visual`]; see
    /// [`App::target_for_cursor`]'s doc for the file-view `(=)` conversion
    /// this also applies.
    pub fn target_for_visual(&self, anchor: usize) -> Option<Target> {
        let file = self.view.files.get(self.view.file_of_cursor())?;
        let target = targeting::target_for_visual(file, &self.view.rows, self.view.cursor, anchor)?;
        Some(self.maybe_as_worktree_target(target))
    }

    /// Routes `target` through [`targeting::as_worktree_target`] iff the
    /// active target is [`DiffTarget::File`] (the read-only file view);
    /// returns it unchanged for every diff-backed target.
    fn maybe_as_worktree_target(&self, target: Target) -> Target {
        if matches!(self.target, DiffTarget::File(_)) {
            targeting::as_worktree_target(target)
        } else {
            target
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
        // Save-on-change: a no-op outside a review session (no
        // `review_state_path` set) — see `review_ops`'s module doc for why
        // this is safe to call unconditionally.
        self.persist_review_state();
    }

    /// Derives the [`Source`] to record for an annotation composed against
    /// the current view: the active [`DiffTarget`]'s kind, using
    /// `active_commit`'s already-`core.abbrev`-aware short SHA for a commit
    /// target rather than having `annotate/` (or this method) recompute an
    /// abbreviation of its own. Falls back to the full rev string if a
    /// commit is somehow open with no matching `active_commit` entry —
    /// defensive fallback; never expected in practice.
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
            // The read-only file view always synthesizes its body from the
            // live worktree, never a historical revision, so its
            // annotations are authored against the working-tree source too
            // — `Target::WorktreeLine`/`Target::WorktreeRange` (the `(=)`
            // marker) is what distinguishes a file-view annotation from an
            // ordinary working-tree diff one, not `Source`.
            DiffTarget::File(_) => Source::WorkingTree,
            // No dedicated `Source` variant: a review's three-dot range is
            // exactly the shape `Source::Range` already models, so this
            // produces the same `Reviewing: base...branch` metadata line
            // the `Range` source would for that literal range string.
            DiffTarget::Review { base, branch } => Source::Range(format!("{base}...{branch}")),
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

    /// Requests a commit of the currently staged changes: `git commit -m
    /// <message>`, spawned on the same background pool and
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
