//! Modal key handling. Normal and Visual mode dispatch every keystroke
//! through the [`super::Keymap`] table; the other modes (Compose, List,
//! Staging panel, Search, Peek) are modal — while one is active, every
//! keystroke is handled directly here instead of going through the table,
//! since most of what they read (printable characters, `j`/`k` as list
//! navigation rather than a bound action) isn't expressible as one fixed
//! [`super::Action`] per key.
//!
//! Each handler drives [`super::App`] purely through its public methods and
//! modal state; no App internals are reached into here.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::App;
use super::modal_keys::{
    self, CommitMessageAction, ComposeAction, EndReviewAction, FinderAction, ListAction,
    PeekAction, ProjectSearchInputAction, ProjectSearchResultsAction, SearchAction, StagingAction,
    SwitcherAction,
};

/// Handles one key event while [`super::Mode::Compose`] is active. Resolves
/// against `app.modal_keys.compose` (spec 07 Unit 4 task 5.3/5.4: the
/// effective, config-overridable table — see [`modal_keys::ComposeAction`])
/// first; an unresolved, unmodified `Char` inserts as literal text (never
/// remappable, per the free-text-mode contract). Bypasses the
/// [`super::Keymap`] table entirely; documented in
/// [`modal_keys::COMPOSE_HINTS`], drift-tested both directions.
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

/// Handles one key event while [`super::Mode::CommitMessage`] is active
/// (spec 04). Resolves against `app.modal_keys.commit_message` first (same
/// contract as [`handle_compose_key`], minus classification cycling — see
/// [`modal_keys::CommitMessageAction`]); an unresolved, unmodified `Char`
/// inserts as literal text, so `q` types a `q` rather than quitting (an open
/// overlay never quits the app). Documented in
/// [`modal_keys::COMMIT_MESSAGE_HINTS`], drift-tested in both directions.
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
/// falls back to, and the one thing config can never remap (spec 07 Unit 4
/// FR: "character insertion is not an action and cannot be bound"). A no-op
/// for anything else (a `Char` chorded with Ctrl/Alt that the mode's table
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
/// it, `d` deletes it, `a`/`Esc` close the panel. Dispatch is driven by
/// `app.modal_keys.list` (spec 07 Unit 4 task 5.3/5.4: `modal_keys::LIST_KEYS`
/// plus its `[keys.list]` config overrides) — the same table the help
/// overlay renders — so the keys can't drift from their documentation.
/// Bypasses the [`super::Keymap`] table entirely.
pub(super) fn handle_list_key(app: &mut App, key: KeyEvent) {
    let Some(action) = modal_keys::resolve(&app.modal_keys.list, key) else {
        return;
    };
    match action {
        ListAction::MoveDown => app.list_move_down(),
        ListAction::MoveUp => app.list_move_up(),
        ListAction::Jump => app.jump_to_focused_annotation(),
        ListAction::Edit => app.edit_focused_annotation(),
        ListAction::Delete => app.delete_focused_annotation(),
        ListAction::Close => app.close_list(),
    }
}

/// Handles one key event while [`super::Mode::Staging`] is active: `j`/`k`
/// move focus, `Space`/`Enter` unstage the focused file (the panel stays
/// open), `s`/`Esc` close the panel. Dispatch is driven by
/// `app.modal_keys.staging` — the same table the help overlay renders.
/// Bypasses the [`super::Keymap`] table entirely.
pub(super) fn handle_staging_key(app: &mut App, key: KeyEvent) {
    let Some(action) = modal_keys::resolve(&app.modal_keys.staging, key) else {
        return;
    };
    match action {
        StagingAction::MoveDown => app.staging_move_down(),
        StagingAction::MoveUp => app.staging_move_up(),
        StagingAction::Unstage => app.unstage_focused_file(),
        StagingAction::Close => app.close_staging(),
    }
}

/// Handles one key event while [`super::Mode::Search`] is active: printable
/// chars insert into the pattern buffer (never remappable), `Enter` confirms
/// (jumping to the first match at-or-after the cursor), `Esc` cancels
/// (clearing the active pattern only if the buffer was left empty), Backspace
/// deletes. The three control keys resolve against `app.modal_keys.search`
/// first (spec 07 Unit 4 task 5.3). Bypasses the [`super::Keymap`] table
/// entirely.
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
/// panel scope (`` ` `` toggles focus back, `j`/`k` move the panel cursor,
/// `Enter` opens the cursor's file). Unlike the other modal handlers this one
/// stays keymap-driven — panel navigation is a first-class, scoped part of
/// the keymap, not an ad-hoc match — so anything not bound in panel scope is
/// ignored (the review-loop keys never fire while the panel is focused).
///
/// The focused git panel is a first-class view rather than an overlay, so the
/// quit family (`q`/`Q`/Ctrl-C) quits from it just as from the diff view —
/// hence the [`super::Flow`] return, letting the event loop end the session.
pub(super) fn handle_panel_key(
    app: &mut App,
    key: KeyEvent,
    keymap: &super::Keymap,
) -> super::Flow {
    use super::keymap::Scope;
    use super::{Action, Flow, QuitOutcome};
    match keymap.lookup_in(Scope::Panel, key) {
        Some(Action::Quit) => super::quit_action(app),
        Some(Action::QuitDiscard) => Flow::Quit(QuitOutcome::Discard),
        Some(action) => {
            app.apply(action);
            Flow::Continue
        }
        None => Flow::Continue,
    }
}

/// Handles one key event while [`super::Mode::Peek`] is active: `j`/`k` move
/// through results (or scroll hover text), `Enter` jumps the diff cursor to
/// a Definition/References result that's one of the diff's files (closing
/// the overlay) or sets `not in diff` otherwise (a no-op for Hover), `Esc`
/// closes back to Normal (`q` is inert — an open overlay never quits the
/// app). Dispatch is driven by [`modal_keys::PEEK_KEYS`] — the same table the
/// help overlay renders. Bypasses the [`super::Keymap`] table entirely.
pub(super) fn handle_peek_key(app: &mut App, key: KeyEvent) {
    use super::code_intel;
    let Some(action) = modal_keys::resolve(&app.modal_keys.peek, key) else {
        return;
    };
    match action {
        PeekAction::MoveDown => code_intel::peek_move_down(app),
        PeekAction::MoveUp => code_intel::peek_move_up(app),
        PeekAction::Enter => code_intel::peek_enter(app),
        PeekAction::Close => code_intel::close_peek(app),
    }
}

/// Handles one key event while [`super::Mode::Finder`] is active (the fuzzy
/// file finder overlay, spec 06 Unit 1): printable chars extend the query
/// (re-ranking on every keystroke, never remappable), and the control keys —
/// Backspace, `Up`/`Down` move the selection, `Enter` opens the selected
/// file, `Esc` closes losslessly — resolve against `app.modal_keys.finder`
/// first (spec 07 Unit 4 task 5.3). Bypasses the [`super::Keymap`] table
/// entirely, like Compose/Search — free text and navigation together aren't
/// expressible as one fixed [`super::Action`] per key. Documented in
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

/// Handles one key event while [`super::Mode::ProjectSearch`] is active (the
/// full-screen Project Search view, spec 06 Unit 2, plus the round-1 UX
/// fix's two-focus model — see [`super::project_search::SearchFocus`]):
///
/// - **Input focus**: printable chars extend the query (debounced re-scan,
///   never remappable), and the control keys resolve against
///   `app.modal_keys.project_search_input` (spec 07 Unit 4 task 5.3) —
///   Backspace shortens it, `Up`/`Down` move the result selection, `Enter`
///   opens the selected hit, `Esc` moves to Results focus (view stays open).
/// - **Results focus**: control keys resolve against
///   `app.modal_keys.project_search_results` instead — `j`/`k`/`Up`/`Down`
///   move the result selection (letters no longer type into the query —
///   there's nothing to type into while browsing), `Enter` opens the
///   selected hit, `/` returns to Input focus (query preserved), `Esc` is
///   the final "leave the feature" gesture — closes back to the exact prior
///   diff position.
/// - **Both focuses**: `Tab` toggles focus either direction; the three
///   `Alt`-chord toggles (`Alt-c` case, `Alt-w` whole-word, `Alt-r`
///   regex/literal) cycle their state regardless of which half has focus —
///   `Esc`/`Tab`/`Open`/the three toggles route to the *same*
///   `App` methods from either table (e.g. `project_search_esc` itself
///   branches on focus internally to pick "move to Results" vs. "close the
///   view"), so the two tables' shared action names stay behaviorally
///   identical even though they're separate enums per focus.
///
/// Bypasses the [`super::Keymap`] table entirely, like [`handle_finder_key`]
/// — free text and navigation together aren't expressible as one fixed
/// [`super::Action`] per key. Documented in
/// [`modal_keys::PROJECT_SEARCH_INPUT_HINTS`]/
/// [`modal_keys::PROJECT_SEARCH_RESULTS_HINTS`] (control keys and the
/// Alt-chords only; free-text chars — including bare `c`/`w`/`r`/`j`/`k`/`/`
/// with no Alt while Input-focused — are the exemption every other
/// free-text mode's hint table carries).
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
/// active tab's cursor, `Enter` acts on the selected row (a stub until
/// Task 4 wires up `git switch`/re-root — see
/// [`super::App::switcher_confirm`]), and `Esc` closes the modal back to
/// the git panel at its pre-open cursor row. Dispatch is driven by
/// [`modal_keys::SWITCHER_KEYS`] — the same table the help overlay renders
/// — so, like [`handle_peek_key`], `q` isn't in the table and is therefore
/// inert here: an open overlay never quits the app. Bypasses the
/// [`super::Keymap`] table entirely.
pub(super) fn handle_switcher_key(app: &mut App, key: KeyEvent) {
    let Some(action) = modal_keys::resolve(&app.modal_keys.switcher, key) else {
        return;
    };
    match action {
        SwitcherAction::ToggleTab => app.switcher_toggle_tab(),
        SwitcherAction::MoveDown => app.switcher_move_down(),
        SwitcherAction::MoveUp => app.switcher_move_up(),
        SwitcherAction::Confirm => app.switcher_confirm(),
        SwitcherAction::Close => app.close_switcher(),
    }
}

/// Handles one key event while [`super::Mode::EndReview`] is active (spec 08
/// Unit 2: `q` in a review session opens this modal instead of quitting —
/// see [`super::quit_action`]): `p` pauses (quits, emitting annotations
/// through the ordinary on-quit path — the worktree and review state are
/// untouched), `f` finishes (removes the worktree via
/// [`super::App::finish_review`], quitting on success or surfacing the
/// failure and staying open), `c`/`Esc` cancel back to the mode `q` was
/// pressed from. Dispatch is driven by [`modal_keys::END_REVIEW_KEYS`] — the
/// same table the help overlay renders. Unlike most modal handlers this one
/// returns [`super::Flow`] (like [`handle_panel_key`]): pause/a successful
/// finish end the session, so the event loop must see the quit rather than
/// this function looping it internally.
pub(super) fn handle_end_review_key(app: &mut App, key: KeyEvent) -> super::Flow {
    use super::{Flow, QuitOutcome};
    let Some(action) = modal_keys::resolve(&app.modal_keys.end_review, key) else {
        return Flow::Continue;
    };
    match action {
        EndReviewAction::Pause => Flow::Quit(QuitOutcome::Emit),
        EndReviewAction::Finish => match app.finish_review() {
            Some(outcome) => Flow::Quit(outcome),
            None => Flow::Continue,
        },
        EndReviewAction::Cancel => {
            app.cancel_end_review();
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
