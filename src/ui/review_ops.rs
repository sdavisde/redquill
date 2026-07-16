//! Accept/defer gestures for review sessions (spec 08 Unit 3): `Space`
//! toggles `Accepted`, `S` accepts unconditionally, `d` toggles `Deferred` —
//! thin `App`-side wiring around the pure transitions in `crate::review`,
//! mirroring `staging.rs`'s stage-auto-collapse gesture for the review
//! target (accept auto-collapses the section, un-accept expands it; defer
//! collapses too).
//!
//! `super::dispatch_key` is what routes the resolved `Action::ToggleStage`/
//! `Action::StageFile` here (as `Action::ToggleAccept`/`Action::AcceptFile`)
//! instead of `staging.rs`'s `toggle_stage`/`stage_file`, only while
//! [`App::in_review_session`] is true (see its doc). Every handler here is
//! additionally self-guarded on the same predicate, so calling one directly
//! — from a test, or a future caller that forgets the dispatch-time
//! translation — can never produce review state outside a review session.

use crate::review::{ReviewStatus, accept, toggle_accept, toggle_defer};

use super::App;

impl App {
    /// The review status of `path` (missing entry = `Unreviewed`) — the
    /// single place this question is answered, so the sidebar marker, the
    /// section-header marker, and the banner's progress count can't drift
    /// apart (mirrors `App::stage_ops`'s "one predicate" convention).
    pub(super) fn review_status(&self, path: &str) -> ReviewStatus {
        self.review_states.get(path).copied().unwrap_or_default()
    }

    /// Records `status` for `path`, dropping the map entry entirely on
    /// `Unreviewed` so the map only ever holds non-default entries —
    /// exactly `staged_states`' own "missing = default" convention.
    fn set_review_status(&mut self, path: &str, status: ReviewStatus) {
        if status == ReviewStatus::Unreviewed {
            self.review_states.remove(path);
        } else {
            self.review_states.insert(path.to_string(), status);
        }
    }

    /// The path of the file under the cursor, or `None` on an empty diff.
    fn cursor_file_path(&self) -> Option<String> {
        self.view
            .files
            .get(self.view.file_of_cursor())
            .map(|f| f.path.clone())
    }

    /// Applies a resolved review-status `transition` to the cursor file:
    /// records the new status, collapses the section when the new status
    /// equals `collapse_when` and expands it otherwise (mirroring
    /// `App::stage_file`'s stage-auto-collapse), then rebuilds rows and
    /// keeps the cursor on the file's header row (mirroring
    /// `App::toggle_collapse`'s post-rebuild cursor fix). A no-op on an
    /// empty diff.
    fn apply_review_transition(
        &mut self,
        transition: impl Fn(ReviewStatus) -> ReviewStatus,
        collapse_when: ReviewStatus,
    ) {
        let Some(path) = self.cursor_file_path() else {
            return;
        };
        let next = transition(self.review_status(&path));
        self.set_review_status(&path, next);
        self.view.set_collapsed(&path, next == collapse_when);
        self.rebuild_rows();
        if let Some(index) = self.view.files.iter().position(|f| f.path == path) {
            self.view.cursor = self.view.header_row_of_file[index];
            self.view.ensure_visible();
        }
    }

    /// `Space` in a review session: toggles the cursor file between
    /// `Accepted` and `Unreviewed` (see [`crate::review::toggle_accept`]),
    /// collapsing the section on accept and expanding it on un-accept. Only
    /// ever reached while [`App::in_review_session`] holds — the guard here
    /// is defense in depth (see the module doc).
    pub(super) fn toggle_accept_file(&mut self) {
        if !self.in_review_session() {
            return;
        }
        self.apply_review_transition(toggle_accept, ReviewStatus::Accepted);
    }

    /// `S` in a review session: accepts the cursor file unconditionally from
    /// anywhere inside it (see [`crate::review::accept`]), mirroring
    /// `App::stage_file`'s "works from anywhere" gesture — always collapses.
    /// Same reachability guard as [`App::toggle_accept_file`].
    pub(super) fn accept_file(&mut self) {
        if !self.in_review_session() {
            return;
        }
        self.apply_review_transition(accept, ReviewStatus::Accepted);
    }

    /// `d` in a review session: toggles the cursor file between `Deferred`
    /// and `Unreviewed` (see [`crate::review::toggle_defer`]), collapsing
    /// on defer and expanding on un-defer. Unlike accept, `d`'s keymap
    /// binding is unconditional (the key was previously free — see
    /// `keymap.rs`), so this guard is what keeps a non-review session's `d`
    /// a total no-op, byte-for-byte the same as when the key was unbound.
    pub(super) fn toggle_defer_file(&mut self) {
        if !self.in_review_session() {
            return;
        }
        self.apply_review_transition(toggle_defer, ReviewStatus::Deferred);
    }
}

#[cfg(test)]
mod tests {
    use crate::diff::FileDiff;
    use crate::git::{DiffTarget, RawFilePatch};

    use super::super::keymap::Action;
    use super::*;

    fn file(path: &str) -> FileDiff {
        let raw = format!(
            "diff --git a/{path} b/{path}\nindex 1..2 100644\n--- a/{path}\n+++ b/{path}\n@@ -1,1 +1,1 @@\n-old\n+new\n"
        );
        FileDiff::from_patch(&RawFilePatch {
            path: path.to_string(),
            old_path: None,
            raw,
            is_binary: false,
        })
        .unwrap()
    }

    fn review_app(paths: &[&str]) -> App {
        let mut app = App::new(paths.iter().map(|p| file(p)).collect());
        app.target = DiffTarget::Review {
            base: "main".to_string(),
            branch: "feature".to_string(),
        };
        app
    }

    // -- ToggleAccept (Space) -------------------------------------------------

    #[test]
    fn toggle_accept_sets_accepted_and_collapses() {
        let mut app = review_app(&["a.rs"]);
        app.apply(Action::ToggleAccept);
        assert_eq!(app.review_status("a.rs"), ReviewStatus::Accepted);
        assert!(app.view.is_collapsed("a.rs"));
    }

    #[test]
    fn toggle_accept_again_un_accepts_and_expands() {
        let mut app = review_app(&["a.rs"]);
        app.apply(Action::ToggleAccept);
        app.apply(Action::ToggleAccept);
        assert_eq!(app.review_status("a.rs"), ReviewStatus::Unreviewed);
        assert!(!app.view.is_collapsed("a.rs"));
    }

    #[test]
    fn toggle_accept_outside_a_review_session_is_a_no_op() {
        let mut app = App::new(vec![file("a.rs")]);
        assert_eq!(app.target, DiffTarget::WorkingTree);
        app.apply(Action::ToggleAccept);
        assert_eq!(app.review_status("a.rs"), ReviewStatus::Unreviewed);
        assert!(!app.view.is_collapsed("a.rs"));
    }

    // -- AcceptFile (S) ---------------------------------------------------------

    #[test]
    fn accept_file_sets_accepted_from_anywhere_in_the_file() {
        let mut app = review_app(&["a.rs"]);
        // Move off the header row, into the file's body, then accept — the
        // gesture must resolve to the *file* under the cursor, not just its
        // header (mirrors `StageFile`).
        app.view.cursor = app.view.rows.len().saturating_sub(1);
        app.apply(Action::AcceptFile);
        assert_eq!(app.review_status("a.rs"), ReviewStatus::Accepted);
        assert!(app.view.is_collapsed("a.rs"));
    }

    #[test]
    fn accept_file_is_unconditional_even_when_already_deferred() {
        let mut app = review_app(&["a.rs"]);
        app.apply(Action::ToggleDefer);
        assert_eq!(app.review_status("a.rs"), ReviewStatus::Deferred);
        app.apply(Action::AcceptFile);
        assert_eq!(app.review_status("a.rs"), ReviewStatus::Accepted);
    }

    #[test]
    fn accept_file_outside_a_review_session_is_a_no_op() {
        let mut app = App::new(vec![file("a.rs")]);
        app.apply(Action::AcceptFile);
        assert_eq!(app.review_status("a.rs"), ReviewStatus::Unreviewed);
    }

    // -- ToggleDefer (d) ----------------------------------------------------

    #[test]
    fn toggle_defer_sets_deferred_and_collapses() {
        let mut app = review_app(&["a.rs"]);
        app.apply(Action::ToggleDefer);
        assert_eq!(app.review_status("a.rs"), ReviewStatus::Deferred);
        assert!(app.view.is_collapsed("a.rs"));
    }

    #[test]
    fn toggle_defer_again_un_defers_and_expands() {
        let mut app = review_app(&["a.rs"]);
        app.apply(Action::ToggleDefer);
        app.apply(Action::ToggleDefer);
        assert_eq!(app.review_status("a.rs"), ReviewStatus::Unreviewed);
        assert!(!app.view.is_collapsed("a.rs"));
    }

    #[test]
    fn toggle_defer_outside_a_review_session_is_a_total_no_op() {
        // `d` is bound unconditionally in the keymap (it was previously
        // free) — outside a review session this must behave exactly as an
        // unbound key always did: no state change, no status message.
        let mut app = App::new(vec![file("a.rs")]);
        app.apply(Action::ToggleDefer);
        assert_eq!(app.review_status("a.rs"), ReviewStatus::Unreviewed);
        assert!(!app.view.is_collapsed("a.rs"));
        assert!(app.status_message.is_none());
    }

    // -- Accept/defer are mutually exclusive on one file ----------------------

    #[test]
    fn accepting_a_deferred_file_replaces_its_status() {
        let mut app = review_app(&["a.rs"]);
        app.apply(Action::ToggleDefer);
        assert_eq!(app.review_status("a.rs"), ReviewStatus::Deferred);
        app.apply(Action::ToggleAccept);
        assert_eq!(app.review_status("a.rs"), ReviewStatus::Accepted);
        assert!(app.view.is_collapsed("a.rs"));
    }

    #[test]
    fn deferring_an_accepted_file_replaces_its_status() {
        let mut app = review_app(&["a.rs"]);
        app.apply(Action::ToggleAccept);
        assert_eq!(app.review_status("a.rs"), ReviewStatus::Accepted);
        app.apply(Action::ToggleDefer);
        assert_eq!(app.review_status("a.rs"), ReviewStatus::Deferred);
        assert!(app.view.is_collapsed("a.rs"));
    }

    // -- review_progress wiring (banner count) ---------------------------------

    #[test]
    fn review_progress_counts_only_accepted_files() {
        let mut app = review_app(&["a.rs", "b.rs", "c.rs"]);
        app.apply(Action::ToggleAccept); // a.rs, cursor starts on its header
        assert_eq!(app.review_progress(), (1, 3));
        app.select_file_by_path("b.rs");
        app.apply(Action::ToggleDefer); // deferred, not accepted
        assert_eq!(app.review_progress(), (1, 3));
    }

    #[test]
    fn review_progress_is_zero_outside_a_review_session() {
        let app = App::new(vec![file("a.rs"), file("b.rs")]);
        assert_eq!(app.review_progress(), (0, 2));
    }
}
