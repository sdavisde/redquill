//! Modal key handling. Normal and Visual mode dispatch every keystroke
//! through the [`super::Keymap`] table; the other modes (Compose, List,
//! Staging panel, Search, Peek) are modal — while one is active, every
//! keystroke is handled directly here instead of going through the table,
//! since most of what they read (printable characters, `j`/`k` as list
//! navigation rather than a bound action) isn't expressible as one fixed
//! [`super::Action`] per key.
//!
//! Each handler drives [`super::App`] purely through its public methods and
//! modal state; no App internals are reached into here. Every modal handler
//! bypasses the [`super::Keymap`] table entirely, resolving instead against
//! its own table in [`super::modal_keys`] — drift-tested against the `?`
//! help overlay's hints in both directions.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::App;
use super::modal_keys::{
    self, AcceptedPanelAction, CommitMessageAction, ComposeAction, ConfirmRemoteOpAction,
    EndReviewAction, FilterEditAction, FinderAction, LauncherAction, ListAction, PeekAction,
    ProjectSearchInputAction, ProjectSearchResultsAction, SearchAction, StagingAction,
    SwitcherAction, ThreadViewAction,
};
use super::motion;

/// What [`intercept_motion_count`] did with one key.
enum MotionIntercept {
    /// The key was fully consumed (a digit extended the count, or `Esc`
    /// cancelled one in progress); the caller must not dispatch it further.
    Handled,
    /// Not a count-interception key: resolve it against the mode's own
    /// table as usual, repeating a motion action this many times (`None` —
    /// apply once).
    Resolve(Option<usize>),
}

/// Shared digit/Esc count-prefix interception for every modal-list handler
/// (List, Staging, Accepted panel, Switcher, Peek) — the modal-table
/// counterpart of `handle_panel_key`'s identical bookkeeping, both built on
/// `App::motion_count` and `motion::accumulate_digit` (see that field's doc
/// for why this is a separate field from `dispatch_key`'s Normal/Visual
/// count). `Esc` cancelling an in-progress count takes priority over the
/// mode's own `Esc` binding (usually "close"), mirroring Normal/Visual's
/// count-cancel-before-anything-else semantics.
fn intercept_motion_count(app: &mut App, key: KeyEvent) -> MotionIntercept {
    if key.code == KeyCode::Esc && app.motion_count.take().is_some() {
        return MotionIntercept::Handled;
    }
    if key.modifiers == KeyModifiers::NONE
        && let KeyCode::Char(c) = key.code
        && let motion::DigitOutcome::Consumed(n) = motion::accumulate_digit(app.motion_count, c)
    {
        app.motion_count = Some(n);
        return MotionIntercept::Handled;
    }
    MotionIntercept::Resolve(app.motion_count.take())
}

/// Applies `f` [`motion::clamp_count`]-many times for `count` — the shared
/// repeat wrapper every modal-list handler's step/half-page/full-page arms
/// use, so a count prefix (`3j`, `2Ctrl-d`) composes the same way it does
/// in Normal/Visual and panel scope.
fn apply_motion_n_times(count: Option<usize>, mut f: impl FnMut()) {
    for _ in 0..motion::clamp_count(count) {
        f();
    }
}

/// One filter-adopting context's hooks into its own `App` methods — the
/// generic surface [`intercept_filter`] drives so the interception logic
/// (spec 12 FR-7) is written exactly once instead of once per context
/// (List, Staging/Accepted panel, Switcher). Non-capturing closures, so
/// these coerce to plain `fn` pointers and a hook table is a `const`.
struct FilterHooks {
    is_active: fn(&App) -> bool,
    is_editing: fn(&App) -> bool,
    push_char: fn(&mut App, char),
    backspace: fn(&mut App),
    lock: fn(&mut App),
    resume_editing: fn(&mut App),
    clear: fn(&mut App),
}

/// Shared `/` filter-mode key interception (spec 12 FR-7/FR-10): while a
/// filter is active and still being edited, bare printable chars extend the
/// query and Enter/Esc/Backspace resolve against `app.modal_keys.filter_edit`
/// (never the caller's own table — every key belongs to the query while
/// typing); while locked, `/` resumes editing and `Esc` clears the filter
/// outright, ahead of the caller's own `Esc` binding (usually "close"),
/// mirroring `intercept_motion_count`'s identical count-cancel-first
/// priority. Returns `true` if the key was fully handled here — the caller
/// must not dispatch it further.
fn intercept_filter(app: &mut App, key: KeyEvent, hooks: &FilterHooks) -> bool {
    if !(hooks.is_active)(app) {
        return false;
    }
    if (hooks.is_editing)(app) {
        if let Some(action) = modal_keys::resolve(&app.modal_keys.filter_edit, key) {
            match action {
                FilterEditAction::Lock => (hooks.lock)(app),
                FilterEditAction::Clear => (hooks.clear)(app),
                FilterEditAction::DeleteChar => (hooks.backspace)(app),
            }
            return true;
        }
        // Bare, unmodified `Char` extends the query — never remappable, per
        // the free-text-mode contract (mirrors `insert_if_plain_char`).
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if let (KeyCode::Char(c), false, false) = (key.code, ctrl, alt) {
            (hooks.push_char)(app, c);
        }
        return true;
    }
    match key.code {
        KeyCode::Char('/') => {
            (hooks.resume_editing)(app);
            true
        }
        KeyCode::Esc => {
            (hooks.clear)(app);
            true
        }
        _ => false,
    }
}

/// [`FilterHooks`] for the annotation list panel's filter (`App::list_filter`).
const LIST_FILTER_HOOKS: FilterHooks = FilterHooks {
    is_active: |app| app.list_filter.is_some(),
    is_editing: |app| app.list_filter.as_ref().is_some_and(|f| f.is_editing()),
    push_char: |app, c| app.list_filter_push_char(c),
    backspace: |app| app.list_filter_backspace(),
    lock: |app| app.list_lock_filter(),
    resume_editing: |app| app.list_resume_filter_editing(),
    clear: |app| app.list_clear_filter(),
};

/// [`FilterHooks`] for the staging/accepted panel's filter
/// (`App::staging_filter`) — shared by both panels, like every other
/// staging method (see `App::staged`'s doc).
const STAGING_FILTER_HOOKS: FilterHooks = FilterHooks {
    is_active: |app| app.staging_filter.is_some(),
    is_editing: |app| app.staging_filter.as_ref().is_some_and(|f| f.is_editing()),
    push_char: |app, c| app.staging_filter_push_char(c),
    backspace: |app| app.staging_filter_backspace(),
    lock: |app| app.staging_lock_filter(),
    resume_editing: |app| app.staging_resume_filter_editing(),
    clear: |app| app.staging_clear_filter(),
};

/// [`FilterHooks`] for the switcher's active-tab filter
/// (`SwitcherState::filter`).
const SWITCHER_FILTER_HOOKS: FilterHooks = FilterHooks {
    is_active: |app| app.switcher.as_ref().is_some_and(|s| s.filter.is_some()),
    is_editing: |app| {
        app.switcher
            .as_ref()
            .and_then(|s| s.filter.as_ref())
            .is_some_and(|f| f.is_editing())
    },
    push_char: |app, c| app.switcher_filter_push_char(c),
    backspace: |app| app.switcher_filter_backspace(),
    lock: |app| app.switcher_lock_filter(),
    resume_editing: |app| app.switcher_resume_filter_editing(),
    clear: |app| app.switcher_clear_filter(),
};

/// [`FilterHooks`] for the Review launcher's active-tab filter
/// (`App::launcher_filter`), spec 12 FR-12.
const LAUNCHER_FILTER_HOOKS: FilterHooks = FilterHooks {
    is_active: |app| app.launcher_filter.is_some(),
    is_editing: |app| app.launcher_filter.as_ref().is_some_and(|f| f.is_editing()),
    push_char: |app, c| app.review_launcher_filter_push_char(c),
    backspace: |app| app.review_launcher_filter_backspace(),
    lock: |app| app.review_launcher_lock_filter(),
    resume_editing: |app| app.review_launcher_resume_filter_editing(),
    clear: |app| app.review_launcher_clear_filter(),
};

/// Handles one key event while [`super::Mode::Compose`] is active. Resolves
/// against `app.modal_keys.compose` first; an unresolved, unmodified `Char`
/// inserts as literal text (never remappable, per the free-text-mode
/// contract). See [`modal_keys::COMPOSE_HINTS`].
pub(super) fn handle_compose_key(app: &mut App, key: KeyEvent) {
    if let Some(action) = modal_keys::resolve(&app.modal_keys.compose, key) {
        match action {
            ComposeAction::Cancel => app.cancel_compose(),
            ComposeAction::Submit => app.submit_compose(),
            ComposeAction::CycleClassification => {
                if let Some(compose) = app.compose.as_mut() {
                    compose.classification = compose.classification.cycle();
                }
            }
            ComposeAction::Edit(edit) => {
                if let Some(compose) = app.compose.as_mut() {
                    edit.apply(&mut compose.buffer);
                }
            }
        }
        return;
    }
    insert_if_plain_char(app.compose.as_mut().map(|c| &mut c.buffer), key);
}

/// Handles one key event while [`super::Mode::CommitMessage`] is active.
/// Resolves against `app.modal_keys.commit_message` first (same contract as
/// [`handle_compose_key`], minus classification cycling); an unresolved,
/// unmodified `Char` inserts as literal text, so `q` types a `q` rather than
/// quitting (an open overlay never quits the app).
pub(super) fn handle_commit_message_key(app: &mut App, key: KeyEvent) {
    if let Some(action) = modal_keys::resolve(&app.modal_keys.commit_message, key) {
        match action {
            CommitMessageAction::Cancel => app.close_commit_message(),
            CommitMessageAction::Submit => app.submit_commit_message(),
            CommitMessageAction::Edit(edit) => {
                if let Some(state) = app.commit_message.as_mut() {
                    edit.apply(&mut state.buffer);
                }
            }
        }
        return;
    }
    insert_if_plain_char(app.commit_message.as_mut().map(|c| &mut c.buffer), key);
}

/// Inserts `key`'s character into `buffer` when it's a bare, unmodified
/// `Char` — the one thing every free-text mode's resolve-first dispatch
/// falls back to, and the one thing config can never remap. A no-op for
/// anything else (a `Char` chorded with Ctrl/Alt that the mode's table
/// doesn't document, or a non-`Char` key already exhausted by `resolve`).
fn insert_if_plain_char(buffer: Option<&mut super::compose::TextBuffer>, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    if let (KeyCode::Char(c), false, false) = (key.code, ctrl, alt)
        && let Some(buffer) = buffer
    {
        buffer.insert_char(c);
    }
}

/// Handles one key event while [`super::Mode::List`] is active: `j`/`k` move
/// focus, `Enter` jumps to the annotation and closes the panel, `e` edits
/// it, `d` deletes it, `a`/`Esc` close the panel.
pub(super) fn handle_list_key(app: &mut App, key: KeyEvent) {
    if intercept_filter(app, key, &LIST_FILTER_HOOKS) {
        return;
    }
    let count = match intercept_motion_count(app, key) {
        MotionIntercept::Handled => return,
        MotionIntercept::Resolve(count) => count,
    };
    let Some(action) = modal_keys::resolve(&app.modal_keys.list, key) else {
        return;
    };
    match action {
        ListAction::MoveDown => apply_motion_n_times(count, || app.list_move_down()),
        ListAction::MoveUp => apply_motion_n_times(count, || app.list_move_up()),
        ListAction::HalfPageDown => apply_motion_n_times(count, || app.list_half_page_down()),
        ListAction::HalfPageUp => apply_motion_n_times(count, || app.list_half_page_up()),
        ListAction::FullPageDown => apply_motion_n_times(count, || app.list_full_page_down()),
        ListAction::FullPageUp => apply_motion_n_times(count, || app.list_full_page_up()),
        ListAction::JumpToTop => app.list_jump_to_top(),
        ListAction::JumpToBottom => app.list_jump_to_bottom(),
        ListAction::Jump => app.jump_to_focused_annotation(),
        ListAction::Edit => app.edit_focused_annotation(),
        ListAction::Delete => app.delete_focused_annotation(),
        ListAction::EnterFilter => app.list_enter_filter(),
        ListAction::Close => app.close_list(),
    }
}

/// Handles one key event while [`super::Mode::Staging`] is active: `j`/`k`
/// move focus, `Space`/`Enter` unstage the focused file (the panel stays
/// open), `s`/`Esc` close the panel.
pub(super) fn handle_staging_key(app: &mut App, key: KeyEvent) {
    if intercept_filter(app, key, &STAGING_FILTER_HOOKS) {
        return;
    }
    let count = match intercept_motion_count(app, key) {
        MotionIntercept::Handled => return,
        MotionIntercept::Resolve(count) => count,
    };
    // Review sessions repurpose `Mode::Staging` as the accepted-files panel:
    // resolve against its own table instead of the local staging panel's,
    // so the two never cross-dispatch (`unstage_focused_file`
    // would be untruthful during a review — there is nothing staged to
    // unstage — and `un_accept_focused_file` would be meaningless locally).
    if app.in_review_session() {
        let Some(action) = modal_keys::resolve(&app.modal_keys.accepted_panel, key) else {
            return;
        };
        match action {
            AcceptedPanelAction::MoveDown => {
                apply_motion_n_times(count, || app.staging_move_down())
            }
            AcceptedPanelAction::MoveUp => apply_motion_n_times(count, || app.staging_move_up()),
            AcceptedPanelAction::HalfPageDown => {
                apply_motion_n_times(count, || app.staging_half_page_down())
            }
            AcceptedPanelAction::HalfPageUp => {
                apply_motion_n_times(count, || app.staging_half_page_up())
            }
            AcceptedPanelAction::FullPageDown => {
                apply_motion_n_times(count, || app.staging_full_page_down())
            }
            AcceptedPanelAction::FullPageUp => {
                apply_motion_n_times(count, || app.staging_full_page_up())
            }
            AcceptedPanelAction::JumpToTop => app.staging_jump_to_top(),
            AcceptedPanelAction::JumpToBottom => app.staging_jump_to_bottom(),
            AcceptedPanelAction::UnAccept => app.un_accept_focused_file(),
            AcceptedPanelAction::EnterFilter => app.staging_enter_filter(),
            AcceptedPanelAction::Close => app.close_staging(),
        }
        return;
    }
    let Some(action) = modal_keys::resolve(&app.modal_keys.staging, key) else {
        return;
    };
    match action {
        StagingAction::MoveDown => apply_motion_n_times(count, || app.staging_move_down()),
        StagingAction::MoveUp => apply_motion_n_times(count, || app.staging_move_up()),
        StagingAction::HalfPageDown => apply_motion_n_times(count, || app.staging_half_page_down()),
        StagingAction::HalfPageUp => apply_motion_n_times(count, || app.staging_half_page_up()),
        StagingAction::FullPageDown => apply_motion_n_times(count, || app.staging_full_page_down()),
        StagingAction::FullPageUp => apply_motion_n_times(count, || app.staging_full_page_up()),
        StagingAction::JumpToTop => app.staging_jump_to_top(),
        StagingAction::JumpToBottom => app.staging_jump_to_bottom(),
        StagingAction::Unstage => app.unstage_focused_file(),
        StagingAction::EnterFilter => app.staging_enter_filter(),
        StagingAction::Close => app.close_staging(),
    }
}

/// Handles one key event while [`super::Mode::Search`] is active: printable
/// chars insert into the pattern buffer (never remappable), `Enter` confirms
/// (jumping to the first match at-or-after the cursor), `Esc` cancels
/// (clearing the active pattern only if the buffer was left empty), Backspace
/// deletes. The three control keys resolve against `app.modal_keys.search`
/// first.
pub(super) fn handle_search_key(app: &mut App, key: KeyEvent) {
    let Some(action) = modal_keys::resolve(&app.modal_keys.search, key) else {
        if let KeyCode::Char(c) = key.code {
            app.search_input.push(c);
        }
        return;
    };
    match action {
        SearchAction::Cancel => app.cancel_search(),
        SearchAction::Confirm => app.confirm_search(),
        SearchAction::DeleteChar => {
            app.search_input.pop();
        }
    }
}

/// Handles one key event while [`super::Mode::Panel`] is active (the git
/// panel holds focus): keys resolve through the [`super::Keymap`] table in
/// panel scope (`` ` ``/`Esc` toggle/close focus back, `j`/`k` move the panel
/// cursor, `Enter` opens the cursor's file). Unlike the other modal handlers
/// this one stays keymap-driven — panel navigation is a first-class, scoped
/// part of the keymap, not an ad-hoc match — so anything not bound in panel
/// scope is ignored. The per-file keys (`Space`/`S`/`d`) route through
/// [`panel_file_action`], which gates on the highlighted row and translates
/// stage gestures to accept gestures during a review session.
///
/// The focused git panel is a first-class view rather than an overlay, so the
/// quit family (`q`/`Q`/Ctrl-C) quits from it just as from the diff view —
/// hence the [`super::Flow`] return, letting the event loop end the session.
///
/// `p`/`P` (pull/push) additionally open the confirm modal instead of
/// running immediately whenever [`super::app::App::in_review_session`] holds
/// — `f` (fetch) is untouched, since reviewers are expected to fetch freely.
///
/// `s` closes the panel before opening the staging panel (`ToggleStagingPanel`
/// otherwise no-ops while `Mode::Panel` is active — see `staging.rs`'s
/// doc); `/` needs no such shim, since `enter_search` overwrites `self.mode`
/// unconditionally and so already behaves as if the panel were closed first.
/// `Esc`/`` ` `` (`FocusGitPanel`) and every other action run through the
/// unchanged generic `app.apply(action)` path.
pub(super) fn handle_panel_key(
    app: &mut App,
    key: KeyEvent,
    keymap: &super::Keymap,
) -> super::Flow {
    use super::keymap::Scope;
    use super::{Action, Flow, QuitOutcome};

    // Same digit/Esc count-prefix bookkeeping every modal-list handler
    // shares (see `intercept_motion_count`'s doc) — Panel scope has no
    // two-key pending prefix to interact with, so it uses the same
    // App::motion_count field rather than dispatch_key's Normal/Visual one.
    let count = match intercept_motion_count(app, key) {
        MotionIntercept::Handled => return Flow::Continue,
        MotionIntercept::Resolve(count) => count,
    };
    match keymap.lookup_in(Scope::Panel, key) {
        Some(Action::Quit) => super::quit_action(app),
        Some(Action::QuitDiscard) => Flow::Quit(QuitOutcome::Discard),
        Some(Action::RemotePull) if app.in_review_session() => {
            app.open_confirm_remote_op_modal(crate::git::RemoteOp::Pull);
            Flow::Continue
        }
        Some(Action::RemotePush) if app.in_review_session() => {
            app.open_confirm_remote_op_modal(app.remote_push_op());
            Flow::Continue
        }
        Some(
            action @ (Action::ToggleStage
            | Action::StageFile
            | Action::ToggleAccept
            | Action::AcceptFile
            | Action::ToggleDefer),
        ) => {
            panel_file_action(app, action);
            Flow::Continue
        }
        Some(Action::ToggleStagingPanel) => {
            app.toggle_git_panel();
            app.apply(Action::ToggleStagingPanel);
            Flow::Continue
        }
        Some(action) => {
            // A count prefix repeats the panel motions exactly like
            // Normal/Visual's (`repeat_count` — shared via
            // `motion::clamp_count`); every other panel action applies
            // exactly once regardless of a stray accumulated count.
            for _ in 0..super::repeat_count(action, count) {
                app.apply(action);
            }
            Flow::Continue
        }
        None => Flow::Continue,
    }
}

/// Routes the panel's per-file keys (stage / accept / defer) to the
/// highlighted row. Only Changes-tab file rows act: the History tab has no
/// file rows, so the keys are inert there, and a directory row hints
/// instead of recursively applying. `panel_follow` keeps the diff cursor on
/// the highlighted file, so the existing cursor-file operations already
/// target the right file; it is re-run here defensively before acting.
/// Review sessions translate the stage gestures to their accept
/// counterparts, mirroring the diff view's dispatch translation, and the
/// stage path forces the whole-file gesture — the diff cursor's row kind is
/// follow-sync bookkeeping, not a user gesture.
fn panel_file_action(app: &mut App, action: super::Action) {
    use super::Action;
    use super::app::PanelTab;
    use super::git_panel::{PanelRow, navigable_rows};
    if app.panel_tab() == PanelTab::History {
        return;
    }
    match navigable_rows(app).get(app.panel_cursor()) {
        Some(PanelRow::File(_)) => {}
        Some(PanelRow::Dir(_)) => {
            app.set_status_message("directory rows can't take file actions");
            return;
        }
        None => return,
    }
    app.panel_follow();
    let action = if app.in_review_session() {
        match action {
            Action::ToggleStage => Action::ToggleAccept,
            Action::StageFile => Action::AcceptFile,
            other => other,
        }
    } else {
        action
    };
    match action {
        Action::ToggleStage => super::staging::toggle_stage_whole_file(app),
        // `StageFile` is already a whole-file gesture keyed off the cursor
        // file; the accept/defer handlers self-guard on the session kind.
        other => app.apply(other),
    }
}

/// Handles one key event while [`super::Mode::ConfirmRemoteOp`] is active:
/// resolves against `app.modal_keys.confirm_remote_op`, dispatching
/// confirm/cancel through [`App`]'s state-transition methods
/// (`src/ui/confirm_remote_op.rs`).
pub(super) fn handle_confirm_remote_op_key(app: &mut App, key: KeyEvent) {
    let Some(action) = modal_keys::resolve(&app.modal_keys.confirm_remote_op, key) else {
        return;
    };
    match action {
        ConfirmRemoteOpAction::Confirm => app.confirm_remote_op(),
        ConfirmRemoteOpAction::Cancel => app.cancel_confirm_remote_op(),
    }
}

/// Handles one key event while [`super::Mode::Peek`] is active: `j`/`k` move
/// through results (or scroll hover text), `Enter` jumps the diff cursor to
/// a Definition/References result that's one of the diff's files (closing
/// the overlay) or sets `not in diff` otherwise (a no-op for Hover), `Esc`
/// closes back to Normal (`q` is inert — an open overlay never quits the
/// app).
pub(super) fn handle_peek_key(app: &mut App, key: KeyEvent) {
    use super::code_intel;
    let count = match intercept_motion_count(app, key) {
        MotionIntercept::Handled => return,
        MotionIntercept::Resolve(count) => count,
    };
    let Some(action) = modal_keys::resolve(&app.modal_keys.peek, key) else {
        return;
    };
    match action {
        PeekAction::MoveDown => apply_motion_n_times(count, || code_intel::peek_move_down(app)),
        PeekAction::MoveUp => apply_motion_n_times(count, || code_intel::peek_move_up(app)),
        PeekAction::HalfPageDown => {
            apply_motion_n_times(count, || code_intel::peek_half_page_down(app))
        }
        PeekAction::HalfPageUp => {
            apply_motion_n_times(count, || code_intel::peek_half_page_up(app))
        }
        PeekAction::FullPageDown => {
            apply_motion_n_times(count, || code_intel::peek_full_page_down(app))
        }
        PeekAction::FullPageUp => {
            apply_motion_n_times(count, || code_intel::peek_full_page_up(app))
        }
        PeekAction::JumpToTop => code_intel::peek_jump_to_top(app),
        PeekAction::JumpToBottom => code_intel::peek_jump_to_bottom(app),
        PeekAction::Enter => code_intel::peek_enter(app),
        PeekAction::Close => code_intel::close_peek(app),
    }
}

/// Handles one key event while [`super::Mode::ThreadView`] is active (the
/// imported comment-thread overlay): a read-only viewer, so only scroll and
/// close, resolved against `app.modal_keys.thread_view`. See
/// [`modal_keys::THREAD_VIEW_KEYS`].
pub(super) fn handle_thread_view_key(app: &mut App, key: KeyEvent) {
    let count = match intercept_motion_count(app, key) {
        MotionIntercept::Handled => return,
        MotionIntercept::Resolve(count) => count,
    };
    let Some(action) = modal_keys::resolve(&app.modal_keys.thread_view, key) else {
        return;
    };
    match action {
        ThreadViewAction::ScrollDown => {
            apply_motion_n_times(count, || app.thread_view_scroll_down())
        }
        ThreadViewAction::ScrollUp => apply_motion_n_times(count, || app.thread_view_scroll_up()),
        ThreadViewAction::Close => app.close_thread_view(),
    }
}

/// Handles one key event while [`super::Mode::Finder`] is active (the fuzzy
/// file finder overlay): printable chars extend the query (re-ranking on
/// every keystroke, never remappable), and the control keys — Backspace,
/// `Up`/`Down` move the selection, `Enter` opens the selected file, `Esc`
/// closes losslessly — resolve against `app.modal_keys.finder` first. See
/// [`modal_keys::FINDER_HINTS`] (control keys only; free-text chars are the
/// exemption every other free-text mode's hint table carries).
pub(super) fn handle_finder_key(app: &mut App, key: KeyEvent) {
    let Some(action) = modal_keys::resolve(&app.modal_keys.finder, key) else {
        if let KeyCode::Char(c) = key.code {
            app.finder_input_char(c);
        }
        return;
    };
    match action {
        FinderAction::MoveUp => app.finder_move_up(),
        FinderAction::MoveDown => app.finder_move_down(),
        FinderAction::Open => app.finder_confirm(),
        FinderAction::Close => app.close_finder(),
        FinderAction::DeleteChar => app.finder_backspace(),
    }
}

/// Handles one key event while [`super::Mode::ProjectSearch`] is active,
/// dispatching against whichever table matches the current
/// [`super::project_search::SearchFocus`] (Input vs. Results — see that
/// type's doc for the full per-focus key contract). `Tab` and the three
/// `Alt`-chord toggles act the same from either focus, routing to the same
/// `App` methods.
pub(super) fn handle_project_search_key(app: &mut App, key: KeyEvent) {
    use super::project_search::SearchFocus;
    let results_focused = app
        .project_search
        .as_ref()
        .is_some_and(|state| state.focus == SearchFocus::Results);

    if results_focused {
        let Some(action) = modal_keys::resolve(&app.modal_keys.project_search_results, key) else {
            // Results focus never types free text — an unresolved key is
            // simply inert here.
            return;
        };
        match action {
            ProjectSearchResultsAction::EditQuery => app.project_search_focus_input(),
            ProjectSearchResultsAction::Close => app.project_search_esc(),
            ProjectSearchResultsAction::MoveUp => app.project_search_move_up(),
            ProjectSearchResultsAction::MoveDown => app.project_search_move_down(),
            ProjectSearchResultsAction::Open => app.project_search_confirm(),
            ProjectSearchResultsAction::ToggleFocus => app.project_search_toggle_focus(),
            ProjectSearchResultsAction::ToggleCase => app.project_search_toggle_case(),
            ProjectSearchResultsAction::ToggleWholeWord => app.project_search_toggle_whole_word(),
            ProjectSearchResultsAction::ToggleLiteral => app.project_search_toggle_literal(),
        }
        return;
    }

    if let Some(action) = modal_keys::resolve(&app.modal_keys.project_search_input, key) {
        match action {
            ProjectSearchInputAction::MoveUp => app.project_search_move_up(),
            ProjectSearchInputAction::MoveDown => app.project_search_move_down(),
            ProjectSearchInputAction::Open => app.project_search_confirm(),
            ProjectSearchInputAction::FocusResults => app.project_search_esc(),
            ProjectSearchInputAction::ToggleFocus => app.project_search_toggle_focus(),
            ProjectSearchInputAction::DeleteChar => app.project_search_backspace(),
            ProjectSearchInputAction::ToggleCase => app.project_search_toggle_case(),
            ProjectSearchInputAction::ToggleWholeWord => app.project_search_toggle_whole_word(),
            ProjectSearchInputAction::ToggleLiteral => app.project_search_toggle_literal(),
        }
        return;
    }

    // Free-text fallback (Input focus only, matching the original
    // `!results_focused` guard): a bare, unmodified `Char` extends the
    // query — never remappable, per the free-text-mode contract.
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    if let KeyCode::Char(c) = key.code
        && !alt
    {
        app.project_search_input_char(c);
    }
}

/// Handles one key event while [`super::Mode::Switcher`] is active (the
/// branch/worktree switcher modal is open): `Tab`/`BackTab`/`h`/`l`/arrow
/// keys switch between the Branches and Worktrees tabs, `j`/`k` move the
/// active tab's cursor, `Enter` acts on the selected row (see
/// [`super::App::switcher_confirm`]), and `Esc` closes the modal back to
/// the git panel at its pre-open cursor row. `q` isn't in the table and is
/// therefore inert here: an open overlay never quits the app.
pub(super) fn handle_switcher_key(app: &mut App, key: KeyEvent) {
    if intercept_filter(app, key, &SWITCHER_FILTER_HOOKS) {
        return;
    }
    let count = match intercept_motion_count(app, key) {
        MotionIntercept::Handled => return,
        MotionIntercept::Resolve(count) => count,
    };
    let Some(action) = modal_keys::resolve(&app.modal_keys.switcher, key) else {
        return;
    };
    match action {
        SwitcherAction::ToggleTab => app.switcher_toggle_tab(),
        SwitcherAction::MoveDown => apply_motion_n_times(count, || app.switcher_move_down()),
        SwitcherAction::MoveUp => apply_motion_n_times(count, || app.switcher_move_up()),
        SwitcherAction::HalfPageDown => {
            apply_motion_n_times(count, || app.switcher_half_page_down())
        }
        SwitcherAction::HalfPageUp => apply_motion_n_times(count, || app.switcher_half_page_up()),
        SwitcherAction::FullPageDown => {
            apply_motion_n_times(count, || app.switcher_full_page_down())
        }
        SwitcherAction::FullPageUp => apply_motion_n_times(count, || app.switcher_full_page_up()),
        SwitcherAction::JumpToTop => app.switcher_jump_to_top(),
        SwitcherAction::JumpToBottom => app.switcher_jump_to_bottom(),
        SwitcherAction::Confirm => app.switcher_confirm(),
        SwitcherAction::EnterFilter => app.switcher_enter_filter(),
        SwitcherAction::Close => app.close_switcher(),
    }
}

/// Handles one key event while [`super::Mode::ReviewLauncher`] is active
/// (the Review launcher modal, `R`, `Scope::Global`): `Tab`/`Shift-Tab`/
/// `h`/`l`/arrows switch between the Branches and Commits tabs, the shared
/// motion set (`j`/`k`, half/full-page paging, jump-to-extremes, count
/// prefixes — spec 12 FR-12) moves the cursor, `/` filters the active tab
/// (spec 12 FR-12), `Enter` confirms the highlighted row — starts a branch
/// review on the Branches tab, opens a read-only commit view on the Commits
/// tab — `Esc` closes the modal back to the mode `R` was pressed from, and
/// `a` toggles the Commits tab between its ahead-of-base list and the full
/// recent-HEAD log.
pub(super) fn handle_review_launcher_key(app: &mut App, key: KeyEvent) {
    if intercept_filter(app, key, &LAUNCHER_FILTER_HOOKS) {
        return;
    }
    let count = match intercept_motion_count(app, key) {
        MotionIntercept::Handled => return,
        MotionIntercept::Resolve(count) => count,
    };
    let Some(action) = modal_keys::resolve(&app.modal_keys.review_launcher, key) else {
        return;
    };
    match action {
        LauncherAction::ToggleTab => app.review_launcher_switch_tab(),
        LauncherAction::MoveDown => apply_motion_n_times(count, || app.review_launcher_move_down()),
        LauncherAction::MoveUp => apply_motion_n_times(count, || app.review_launcher_move_up()),
        LauncherAction::HalfPageDown => {
            apply_motion_n_times(count, || app.review_launcher_half_page_down())
        }
        LauncherAction::HalfPageUp => {
            apply_motion_n_times(count, || app.review_launcher_half_page_up())
        }
        LauncherAction::FullPageDown => {
            apply_motion_n_times(count, || app.review_launcher_full_page_down())
        }
        LauncherAction::FullPageUp => {
            apply_motion_n_times(count, || app.review_launcher_full_page_up())
        }
        LauncherAction::JumpToTop => app.review_launcher_jump_to_top(),
        LauncherAction::JumpToBottom => app.review_launcher_jump_to_bottom(),
        LauncherAction::Confirm => app.review_launcher_confirm(),
        LauncherAction::EnterFilter => app.review_launcher_enter_filter(),
        LauncherAction::Close => app.close_review_launcher(),
        LauncherAction::ToggleAllCommits => app.review_launcher_toggle_all_commits(),
    }
}

/// Handles one key event while [`super::Mode::EndReview`] is active (`q` in
/// a review session opens this modal instead of quitting — see
/// [`super::quit_action`]): `p` pauses (quits, emitting annotations through
/// the ordinary on-quit path — the worktree and review state are
/// untouched), `f` finishes (removes the worktree via
/// [`super::App::finish_review`], quitting on success or surfacing the
/// failure and staying open), `c`/`Esc` cancel back to the mode `q` was
/// pressed from. `j`/`k`/arrows move a highlighted selection across the
/// three options and `Enter` confirms whichever one is highlighted (acting
/// exactly like its mnemonic; see [`EndReviewAction::from_cursor`]). Unlike
/// most modal handlers this one returns [`super::Flow`] (like
/// [`handle_panel_key`]): pause/a successful finish end the session, so the
/// event loop must see the quit rather than this function looping it
/// internally.
pub(super) fn handle_end_review_key(app: &mut App, key: KeyEvent) -> super::Flow {
    use super::Flow;
    let Some(action) = modal_keys::resolve(&app.modal_keys.end_review, key) else {
        return Flow::Continue;
    };
    match action {
        EndReviewAction::Pause | EndReviewAction::Finish | EndReviewAction::Cancel => {
            end_review_choice(app, action)
        }
        EndReviewAction::MoveDown => {
            app.end_review_move_down();
            Flow::Continue
        }
        EndReviewAction::MoveUp => {
            app.end_review_move_up();
            Flow::Continue
        }
        EndReviewAction::Confirm => {
            let cursor = app.end_review_cursor().unwrap_or(0);
            end_review_choice(app, EndReviewAction::from_cursor(cursor))
        }
    }
}

/// Runs one of the end-review modal's three exits — shared by the direct
/// mnemonic keys (`p`/`f`/`c`/`Esc`) and by `Enter`'s confirm-the-highlighted-
/// option path (via [`EndReviewAction::from_cursor`]), so the two paths can
/// never drift apart on what pressing "Finish" actually does. `choice` is
/// always `Pause`/`Finish`/`Cancel` in practice; `MoveDown`/`MoveUp`/
/// `Confirm` fall back to a no-op continue rather than panicking (`from_cursor`
/// never produces them, and [`handle_end_review_key`]'s own match never
/// passes them here — this is a defensive fallback, not a reachable path).
fn end_review_choice(app: &mut App, choice: EndReviewAction) -> super::Flow {
    use super::{Flow, QuitOutcome};
    match choice {
        // Pause discards rather than emits, so a consumer sees each
        // annotation exactly once — on finish. The worktree, review state,
        // and every annotation made this session are still kept; only the
        // stdout side effect changes (see `QuitOutcome`'s doc).
        EndReviewAction::Pause => Flow::Quit(QuitOutcome::Discard),
        EndReviewAction::Finish => match app.finish_review() {
            Some(outcome) => Flow::Quit(outcome),
            None => Flow::Continue,
        },
        EndReviewAction::Cancel => {
            app.cancel_end_review();
            Flow::Continue
        }
        EndReviewAction::MoveDown | EndReviewAction::MoveUp | EndReviewAction::Confirm => {
            Flow::Continue
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::FileDiff;
    use crate::git::RawFilePatch;
    use crate::ui::Mode;

    fn sample_file() -> FileDiff {
        let raw = "\
diff --git a/src/main.rs b/src/main.rs
index 111..222 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,2 +1,2 @@
 fn main() {
-    old();
+    new();
";
        FileDiff::from_patch(&RawFilePatch {
            path: "src/main.rs".to_string(),
            old_path: None,
            raw: raw.to_string(),
            is_binary: false,
        })
        .unwrap()
    }

    #[test]
    fn search_input_editing_via_handle_search_key() {
        let mut app = App::new(vec![sample_file()]);
        app.mode = Mode::Search;
        handle_search_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE),
        );
        handle_search_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
        );
        handle_search_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
        );
        assert_eq!(app.search_input, "old");
        handle_search_key(
            &mut app,
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        );
        assert_eq!(app.search_input, "ol");
        handle_search_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.search.pattern.as_deref(), Some("ol"));
    }

    #[test]
    fn search_esc_cancels_mode() {
        let mut app = App::new(vec![sample_file()]);
        app.mode = Mode::Search;
        app.search_input.push('x');
        handle_search_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Normal);
    }

    // -- `R` stays a literal character in every free-text mode ---------------
    //
    // `R` moved from a diff-scope Refresh binding to a `Scope::Global`
    // Review-launcher binding, but every free-text mode bypasses the keymap
    // table entirely (see each handler's own doc above), so none of them are
    // affected: an unresolved, unmodified `Char` always inserts, regardless
    // of what the outer keymap does with that same key.

    #[test]
    fn r_types_a_literal_character_in_compose_mode() {
        let mut app = App::new(vec![sample_file()]);
        app.apply(crate::ui::Action::Compose);
        assert_eq!(app.mode, Mode::Compose, "cursor must land on a real target");
        handle_compose_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE),
        );
        assert_eq!(
            app.compose.as_ref().map(|c| c.buffer.text()).as_deref(),
            Some("R")
        );
    }

    #[test]
    fn r_types_a_literal_character_in_commit_message_mode() {
        let mut app = App::new(vec![sample_file()]);
        app.commit_message = Some(crate::ui::commit_message::CommitMessageState::new(0));
        app.mode = Mode::CommitMessage;
        handle_commit_message_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE),
        );
        assert_eq!(
            app.commit_message
                .as_ref()
                .map(|c| c.buffer.text())
                .as_deref(),
            Some("R")
        );
    }

    #[test]
    fn r_types_a_literal_character_in_search_mode() {
        let mut app = App::new(vec![sample_file()]);
        app.mode = Mode::Search;
        handle_search_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE),
        );
        assert_eq!(app.search_input, "R");
    }

    #[test]
    fn r_types_a_literal_character_in_finder_mode() {
        let mut app = App::new(vec![sample_file()]);
        app.open_finder();
        handle_finder_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE),
        );
        assert_eq!(app.finder.as_ref().map(|f| f.query.as_str()), Some("R"));
    }

    /// An open overlay never quits the app, so `q` is inert while the Peek
    /// overlay is up; Esc still closes it back to Normal.
    #[test]
    fn peek_q_is_inert_and_esc_closes() {
        let mut app = App::new(vec![sample_file()]);
        app.mode = Mode::Peek;
        handle_peek_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        );
        assert_eq!(app.mode, Mode::Peek, "q must not close the Peek overlay");
        handle_peek_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Normal, "Esc still closes the Peek overlay");
    }

    /// An open overlay never quits the app, so `q` is inert while the
    /// switcher modal is up (it isn't in `SWITCHER_KEYS`, so it resolves to
    /// nothing here); Esc still closes it back to the git panel.
    #[test]
    fn switcher_q_is_inert_and_esc_closes() {
        let mut app = App::new(vec![sample_file()]);
        app.switcher = Some(crate::ui::switcher::SwitcherState::new(
            vec![],
            vec![],
            None,
            0,
        ));
        app.mode = Mode::Switcher;
        handle_switcher_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        );
        assert_eq!(
            app.mode,
            Mode::Switcher,
            "q must not close the switcher modal"
        );
        handle_switcher_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(
            matches!(app.mode, Mode::Panel { .. }),
            "Esc still closes the switcher modal, back to the panel"
        );
    }

    /// An open overlay never quits the app: in the commit-message modal `q`
    /// is free-text input — it types a `q` into the draft rather than
    /// quitting — and Esc closes back to the git panel at its prior cursor
    /// row, discarding the draft.
    #[test]
    fn commit_message_q_types_and_esc_closes_back_to_panel() {
        use crate::ui::commit_message::CommitMessageState;
        let mut app = App::new(vec![sample_file()]);
        app.commit_message = Some(CommitMessageState::new(0));
        app.mode = Mode::CommitMessage;
        handle_commit_message_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        );
        assert_eq!(
            app.mode,
            Mode::CommitMessage,
            "q must not close the commit-message modal"
        );
        assert_eq!(
            app.commit_message.as_ref().unwrap().buffer.text(),
            "q",
            "q is ordinary text input in the modal"
        );
        handle_commit_message_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(
            app.mode,
            Mode::Panel {
                cursor: 0,
                tab: crate::ui::app::PanelTab::Changes
            },
            "Esc closes the modal back to the panel"
        );
        assert!(app.commit_message.is_none(), "the draft is discarded");
    }
}
