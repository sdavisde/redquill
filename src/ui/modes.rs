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
use super::modal_keys::{self, ListAction, PeekAction, StagingAction};

/// Handles one key event while [`super::Mode::Compose`] is active: printable
/// chars insert, Backspace deletes, arrow keys move within the text, `Ctrl-j`
/// inserts a newline, `Ctrl-t` cycles the classification, `Enter` submits,
/// `Esc` cancels. Bypasses the [`super::Keymap`] table entirely.
pub(super) fn handle_compose_key(app: &mut App, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => app.cancel_compose(),
        KeyCode::Enter => app.submit_compose(),
        KeyCode::Char('j') if ctrl => {
            if let Some(compose) = app.compose.as_mut() {
                compose.buffer.newline();
            }
        }
        KeyCode::Char('t') if ctrl => {
            if let Some(compose) = app.compose.as_mut() {
                compose.classification = compose.classification.cycle();
            }
        }
        KeyCode::Backspace => {
            if let Some(compose) = app.compose.as_mut() {
                compose.buffer.backspace();
            }
        }
        KeyCode::Left => {
            if let Some(compose) = app.compose.as_mut() {
                compose.buffer.move_left();
            }
        }
        KeyCode::Right => {
            if let Some(compose) = app.compose.as_mut() {
                compose.buffer.move_right();
            }
        }
        KeyCode::Up => {
            if let Some(compose) = app.compose.as_mut() {
                compose.buffer.move_up();
            }
        }
        KeyCode::Down => {
            if let Some(compose) = app.compose.as_mut() {
                compose.buffer.move_down();
            }
        }
        KeyCode::Char(c) if !ctrl => {
            if let Some(compose) = app.compose.as_mut() {
                compose.buffer.insert_char(c);
            }
        }
        _ => {}
    }
}

/// Handles one key event while [`super::Mode::List`] is active: `j`/`k` move
/// focus, `Enter` jumps to the annotation and closes the panel, `e` edits
/// it, `d` deletes it, `a`/`Esc` close the panel. Dispatch is driven by
/// [`modal_keys::LIST_KEYS`] — the same table the help overlay renders — so
/// the keys can't drift from their documentation. Bypasses the
/// [`super::Keymap`] table entirely.
pub(super) fn handle_list_key(app: &mut App, key: KeyEvent) {
    let Some(action) = modal_keys::resolve(modal_keys::LIST_KEYS, key) else {
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
/// [`modal_keys::STAGING_KEYS`] — the same table the help overlay renders.
/// Bypasses the [`super::Keymap`] table entirely.
pub(super) fn handle_staging_key(app: &mut App, key: KeyEvent) {
    let Some(action) = modal_keys::resolve(modal_keys::STAGING_KEYS, key) else {
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
/// chars insert into the pattern buffer, Backspace deletes, `Enter` confirms
/// (jumping to the first match at-or-after the cursor), `Esc` cancels
/// (clearing the active pattern only if the buffer was left empty). Bypasses
/// the [`super::Keymap`] table entirely.
pub(super) fn handle_search_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => app.cancel_search(),
        KeyCode::Enter => app.confirm_search(),
        KeyCode::Backspace => {
            app.search_input.pop();
        }
        KeyCode::Char(c) => app.search_input.push(c),
        _ => {}
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
        Some(Action::Quit) => Flow::Quit(QuitOutcome::Emit),
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
    let Some(action) = modal_keys::resolve(modal_keys::PEEK_KEYS, key) else {
        return;
    };
    match action {
        PeekAction::MoveDown => code_intel::peek_move_down(app),
        PeekAction::MoveUp => code_intel::peek_move_up(app),
        PeekAction::Enter => code_intel::peek_enter(app),
        PeekAction::Close => code_intel::close_peek(app),
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
}
