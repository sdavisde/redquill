//! State for the commit-message modal ([`super::app::Mode::CommitMessage`],
//! spec 04, `docs/specs/04-spec-commit-staged.md`): the multi-line message
//! being typed (reusing Compose's [`TextBuffer`] — it is a plain text buffer,
//! not annotation-specific) and the git panel's cursor row to restore when
//! the modal closes. Also carries the `App` handlers that open, close, and
//! submit the modal, split out of `app.rs` alongside this state so all
//! commit-modal logic lives in one module (mirrors [`super::switcher`]'s
//! state-plus-handlers split).

use super::app::{App, Mode};
use super::compose::TextBuffer;

/// The commit-message modal's state: the message buffer and the git panel's
/// cursor row captured when the modal opened, restored by
/// [`App::close_commit_message`] so `Esc` (and a successful submit) lands
/// the user back on the same panel row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitMessageState {
    /// The message being typed. The first line is the commit subject;
    /// `Ctrl-j` inserts body lines below it.
    pub buffer: TextBuffer,
    /// The git panel's cursor row captured when the modal opened.
    pub panel_cursor: usize,
}

impl CommitMessageState {
    /// Fresh state with an empty buffer, remembering `panel_cursor` for
    /// restore-on-close.
    pub fn new(panel_cursor: usize) -> CommitMessageState {
        CommitMessageState {
            buffer: TextBuffer::new(),
            panel_cursor,
        }
    }
}

/// Whether `message` is empty or whitespace-only — the reject condition for
/// `Enter` in the modal (spec 04 Unit 1). A pure predicate so the rule is
/// unit-testable without an `App`.
pub(super) fn message_is_blank(message: &str) -> bool {
    message.trim().is_empty()
}

impl App {
    /// Opens the commit-message modal (`c`, panel scope, spec 04 Unit 1) —
    /// but only when at least one change is staged; with nothing staged it
    /// degrades to a footer message ("nothing staged to commit") and the
    /// panel keeps focus. Captures the panel's current cursor row so
    /// [`App::close_commit_message`] can restore it; `self.mode` and
    /// `self.commit_message` are only touched together, on the success path,
    /// so the guard never leaves a half-open modal.
    pub(super) fn open_commit_message(&mut self) {
        if self.staged.is_empty() {
            self.set_status_message("nothing staged to commit");
            return;
        }
        let panel_cursor = self.panel_cursor();
        self.commit_message = Some(CommitMessageState::new(panel_cursor));
        self.mode = Mode::CommitMessage;
    }

    /// Closes the commit-message modal, returning to [`Mode::Panel`] at the
    /// cursor row it had before the modal opened — re-clamped against the
    /// panel's current row count in case it shrank while the modal was open
    /// (mirrors [`App::close_switcher`]). The draft message is discarded.
    pub fn close_commit_message(&mut self) {
        let cursor = self
            .commit_message
            .take()
            .map(|s| s.panel_cursor)
            .unwrap_or(0);
        let len = self.panel_row_count();
        self.mode = Mode::Panel {
            cursor: cursor.min(len.saturating_sub(1)),
            tab: self.last_panel_tab,
        };
    }

    /// The `Enter` gesture inside the commit-message modal (spec 04 Unit 2):
    /// an empty or whitespace-only message is rejected with a footer message
    /// and the modal stays open; otherwise the commit is requested on the
    /// background poller (see [`App::request_commit`]) and — only if it was
    /// actually spawned — the modal closes back to the panel. A rejected
    /// request (another mutating git op in flight, or no git backend) leaves
    /// the modal open so the typed message isn't lost; the footer explains
    /// why.
    pub fn submit_commit_message(&mut self) {
        let Some(state) = self.commit_message.as_ref() else {
            // Defensive: no state while in the mode — just close.
            self.close_commit_message();
            return;
        };
        let message = state.buffer.text();
        if message_is_blank(&message) {
            self.set_status_message("commit message is empty");
            return;
        }
        if self.request_commit(&message) {
            self.close_commit_message();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::stage_ops::StagedFile;
    use super::*;
    use crate::diff::FileDiff;
    use crate::git::RawFilePatch;

    fn sample_file(path: &str) -> FileDiff {
        let raw = format!(
            "diff --git a/{path} b/{path}\n\
             index 111..222 100644\n\
             --- a/{path}\n\
             +++ b/{path}\n\
             @@ -1,1 +1,1 @@\n\
             -old\n\
             +new\n"
        );
        FileDiff::from_patch(&RawFilePatch {
            path: path.to_string(),
            old_path: None,
            raw,
            is_binary: false,
        })
        .unwrap()
    }

    /// An `App` focused on the git panel with one staged file, ready for the
    /// `c` gesture.
    fn panel_app_with_staged() -> App {
        let mut app = App::new(vec![sample_file("a.rs"), sample_file("b.rs")]);
        app.staged = vec![StagedFile {
            path: "a.rs".to_string(),
            letter: 'M',
        }];
        app.mode = Mode::Panel {
            cursor: 1,
            tab: crate::ui::app::PanelTab::Changes,
        };
        app
    }

    // -- message_is_blank ----------------------------------------------------

    #[test]
    fn blank_messages_are_empty_or_whitespace_only() {
        assert!(message_is_blank(""));
        assert!(message_is_blank("   "));
        assert!(message_is_blank("\n\n"));
        assert!(message_is_blank(" \t \n "));
        assert!(!message_is_blank("fix: parser"));
        assert!(!message_is_blank("\n\nx")); // any non-whitespace counts
    }

    // -- open_commit_message ---------------------------------------------------

    #[test]
    fn open_requires_something_staged() {
        let mut app = panel_app_with_staged();
        app.staged.clear();
        app.open_commit_message();
        assert!(app.commit_message.is_none());
        assert_eq!(
            app.mode,
            Mode::Panel {
                cursor: 1,
                tab: crate::ui::app::PanelTab::Changes
            }
        );
        assert_eq!(
            app.status_message.as_deref(),
            Some("nothing staged to commit")
        );
    }

    #[test]
    fn open_captures_the_panel_cursor_and_starts_empty() {
        let mut app = panel_app_with_staged();
        app.open_commit_message();
        assert_eq!(app.mode, Mode::CommitMessage);
        let state = app.commit_message.as_ref().unwrap();
        assert_eq!(state.panel_cursor, 1);
        assert_eq!(state.buffer.text(), "");
    }

    // -- close_commit_message --------------------------------------------------

    #[test]
    fn close_restores_the_panel_at_its_prior_cursor_row() {
        let mut app = panel_app_with_staged();
        app.open_commit_message();
        app.close_commit_message();
        assert_eq!(
            app.mode,
            Mode::Panel {
                cursor: 1,
                tab: crate::ui::app::PanelTab::Changes
            }
        );
        assert!(app.commit_message.is_none());
    }

    #[test]
    fn close_without_ever_opening_returns_to_panel_at_zero() {
        let mut app = panel_app_with_staged();
        app.mode = Mode::CommitMessage;
        app.close_commit_message();
        assert_eq!(
            app.mode,
            Mode::Panel {
                cursor: 0,
                tab: crate::ui::app::PanelTab::Changes
            }
        );
    }

    // -- submit_commit_message -------------------------------------------------

    #[test]
    fn submit_rejects_a_blank_message_and_keeps_the_modal_open() {
        let mut app = panel_app_with_staged();
        app.open_commit_message();
        app.commit_message.as_mut().unwrap().buffer = TextBuffer::from_str("  \n ");
        app.submit_commit_message();
        assert_eq!(app.mode, Mode::CommitMessage, "modal must stay open");
        assert!(app.commit_message.is_some(), "draft must survive");
        assert_eq!(
            app.status_message.as_deref(),
            Some("commit message is empty")
        );
    }

    #[test]
    fn submit_without_a_git_backend_keeps_the_modal_and_draft() {
        // No backend attached: request_commit rejects, so the typed message
        // must not be lost.
        let mut app = panel_app_with_staged();
        app.open_commit_message();
        app.commit_message.as_mut().unwrap().buffer = TextBuffer::from_str("fix: parser");
        app.submit_commit_message();
        assert_eq!(app.mode, Mode::CommitMessage, "modal must stay open");
        assert_eq!(
            app.commit_message.as_ref().unwrap().buffer.text(),
            "fix: parser",
            "draft must survive the rejection"
        );
        assert_eq!(
            app.status_message.as_deref(),
            Some("commit unavailable (no git backend)")
        );
    }
}
