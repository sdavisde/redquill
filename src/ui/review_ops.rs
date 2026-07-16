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

use crate::review::{ReviewStatus, toggle_accept, toggle_defer};

use super::App;
use super::stage_ops::StagedFile;

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

    /// `S` in a review session: toggles the cursor file between `Accepted`
    /// and `Unreviewed` from anywhere inside it (see
    /// [`crate::review::toggle_accept`]) — the full `StageFile` toggle
    /// analogue (spec 08 Unit 5, amending Unit 3's originally
    /// one-directional `S`): an already-`Accepted` file un-accepts and
    /// re-expands, exactly like [`App::toggle_accept_file`]/`Space`. The two
    /// handlers apply the identical transition and differ only in which key
    /// resolves to them — review has no hunk/line-level granularity for
    /// `Space` to differentiate on (a deliberate omission, spec 08 Unit 5),
    /// so unlike `ToggleStage`/`StageFile`'s granularity split, `Space`/`S`
    /// are now behavioral synonyms by design. Same reachability guard as
    /// [`App::toggle_accept_file`].
    pub(super) fn accept_file(&mut self) {
        if !self.in_review_session() {
            return;
        }
        self.apply_review_transition(toggle_accept, ReviewStatus::Accepted);
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

    // -- Accepted-files panel (spec 08 Unit 5) -------------------------------

    /// Rebuilds `App::staged` (and, transitively, what the staging panel
    /// renders and indexes via `staging_cursor`) from `review_states` for
    /// the **accepted-files panel**: every file in `view.files` (diff
    /// order, so entries appear in the same stable order the sidebar uses)
    /// whose review status is `Accepted`, with `letter` taken from its
    /// `FileChangeKind` — the same letter the CHANGES sidebar shows.
    /// `App::staged`'s doc explains why sharing that storage with the local
    /// staging panel is safe (the two are mutually exclusive by session).
    pub(super) fn refresh_accepted_list(&mut self) {
        self.staged = self
            .view
            .files
            .iter()
            .filter(|f| self.review_status(&f.path) == ReviewStatus::Accepted)
            .map(|f| StagedFile {
                path: f.path.clone(),
                letter: f.kind.letter(),
            })
            .collect();
    }

    /// Un-accepts the accepted-files panel's focused entry (`Space`/`Enter`,
    /// spec 08 Unit 5): sets its status back to `Unreviewed`, re-expands its
    /// diff section, rebuilds rows, then refreshes the panel list (which
    /// shrinks by one) and re-clamps the cursor — the review analogue of
    /// `App::unstage_focused_file`. A no-op on an empty list. The banner's
    /// `(accepted, total)` count (`App::review_progress`) reflects this
    /// immediately, since it always recomputes live from `review_states`.
    pub(super) fn un_accept_focused_file(&mut self) {
        let Some(entry) = self.staged.get(self.staging_cursor) else {
            return;
        };
        let path = entry.path.clone();
        self.set_review_status(&path, ReviewStatus::Unreviewed);
        self.view.set_collapsed(&path, false);
        self.rebuild_rows();
        self.refresh_accepted_list();
        self.staging_cursor = self.staging_cursor.min(self.staged.len().saturating_sub(1));
        self.set_status_message(format!("un-accepted {path}"));
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
    fn accept_file_from_deferred_accepts_and_collapses() {
        let mut app = review_app(&["a.rs"]);
        app.apply(Action::ToggleDefer);
        assert_eq!(app.review_status("a.rs"), ReviewStatus::Deferred);
        app.apply(Action::AcceptFile);
        assert_eq!(app.review_status("a.rs"), ReviewStatus::Accepted);
        assert!(app.view.is_collapsed("a.rs"));
    }

    /// Spec 08 Unit 5 parity fix: `S` on an already-`Accepted` file
    /// un-accepts it back to `Unreviewed` and re-expands its section — the
    /// full `StageFile` toggle analogue, not the one-directional accept
    /// Unit 3 originally shipped.
    #[test]
    fn accept_file_toggles_an_already_accepted_file_back_to_unreviewed_and_expands() {
        let mut app = review_app(&["a.rs"]);
        app.apply(Action::AcceptFile);
        assert_eq!(app.review_status("a.rs"), ReviewStatus::Accepted);
        assert!(app.view.is_collapsed("a.rs"));

        app.apply(Action::AcceptFile);
        assert_eq!(app.review_status("a.rs"), ReviewStatus::Unreviewed);
        assert!(!app.view.is_collapsed("a.rs"));
    }

    /// `S` from anywhere in an already-collapsed accepted file still
    /// resolves to *that* file's toggle (not a no-op just because the
    /// cursor isn't on the header row).
    #[test]
    fn accept_file_toggle_works_from_anywhere_in_the_file() {
        let mut app = review_app(&["a.rs"]);
        app.apply(Action::AcceptFile); // accept + collapse
        assert!(app.view.is_collapsed("a.rs"));
        // Un-accept expands the section again, so the cursor can move into
        // the body before toggling a second time.
        app.view.cursor = app.view.rows.len().saturating_sub(1);
        app.apply(Action::AcceptFile);
        assert_eq!(app.review_status("a.rs"), ReviewStatus::Unreviewed);
    }

    #[test]
    fn accept_file_outside_a_review_session_is_a_no_op() {
        let mut app = App::new(vec![file("a.rs")]);
        app.apply(Action::AcceptFile);
        assert_eq!(app.review_status("a.rs"), ReviewStatus::Unreviewed);
    }

    /// `Space`'s own toggle is untouched by the `S` parity fix — same
    /// transition, same reachability guard, regression-pinned independently
    /// of `S`'s tests above (spec 08 Unit 5 amends only `S`'s direction).
    #[test]
    fn toggle_accept_space_behavior_is_unchanged_by_the_s_parity_fix() {
        let mut app = review_app(&["a.rs"]);
        app.apply(Action::ToggleAccept);
        assert_eq!(app.review_status("a.rs"), ReviewStatus::Accepted);
        assert!(app.view.is_collapsed("a.rs"));
        app.apply(Action::ToggleAccept);
        assert_eq!(app.review_status("a.rs"), ReviewStatus::Unreviewed);
        assert!(!app.view.is_collapsed("a.rs"));
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
