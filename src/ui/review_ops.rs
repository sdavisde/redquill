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
//!
//! Also owns persistence (spec 08 Unit 4): every status-changing gesture
//! below ends with [`App::persist_review_state`], which spawns the actual
//! disk write on a background thread (never the render loop, mirroring
//! `App::request_remote_op`'s pattern) via
//! [`crate::review::store::save_review`]. Blob-SHA capture
//! ([`App::maybe_capture_blob_sha`]) is the one exception kept synchronous:
//! a single local `git rev-parse` is exactly the class of quick git call
//! `App::stage_file`'s own synchronous `ops.stage_file` already makes
//! directly from a key handler, so this follows that shipped precedent
//! rather than introducing a second, inconsistent threading story for one
//! more fast local read.

use std::collections::BTreeMap;

use crate::git::DiffTarget;
use crate::review::store::{PersistedFile, PersistedReview, PersistedStatus};
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
    /// exactly `staged_states`' own "missing = default" convention. Also
    /// drops any captured `review_blob_shas` entry whenever the new status
    /// isn't `Accepted`/`ChangedSinceAccepted` (those are the only two
    /// statuses a blob SHA is ever meaningful for) — [`App::persist_review_state`]
    /// treats a missing entry as "no blob to record", so this keeps a
    /// stale SHA from lingering after an un-accept or a defer.
    fn set_review_status(&mut self, path: &str, status: ReviewStatus) {
        if status == ReviewStatus::Unreviewed {
            self.review_states.remove(path);
        } else {
            self.review_states.insert(path.to_string(), status);
        }
        if !matches!(
            status,
            ReviewStatus::Accepted | ReviewStatus::ChangedSinceAccepted
        ) {
            self.review_blob_shas.remove(path);
        }
    }

    /// Captures `path`'s current blob SHA on the branch under review
    /// (`git rev-parse <branch>:<path>`, spec 08 Unit 4) whenever `next` is
    /// `Accepted` — the moment-of-acceptance capture the spec calls for,
    /// including the re-accept case (`ChangedSinceAccepted -> Accepted`,
    /// which fetches the *fresh* SHA, superseding the stale one this
    /// overwrites). A no-op for every other status (nothing to capture) and
    /// degrades silently (records no SHA rather than erroring) without a
    /// git backend or outside a review session — the caller is always
    /// already inside `apply_review_transition`, which only ever runs while
    /// `in_review_session()` holds.
    fn maybe_capture_blob_sha(&mut self, path: &str, next: ReviewStatus) {
        if next != ReviewStatus::Accepted {
            return;
        }
        let DiffTarget::Review { branch, .. } = &self.target else {
            return;
        };
        let branch = branch.clone();
        let sha = self
            .stage_ops()
            .and_then(|ops| ops.blob_sha(&branch, path).ok())
            .flatten();
        self.review_blob_shas.insert(path.to_string(), sha);
    }

    /// Builds this review's current [`PersistedReview`] from
    /// `review_states`/`review_blob_shas` and spawns the atomic disk write
    /// on a background thread (spec 08 Unit 4's "off the render loop"
    /// requirement — mirrors [`App::request_remote_op`]'s pattern). A no-op
    /// without a resolved [`App::review_state_path`] (outside a review
    /// session, or a git-less/test context) or without a live
    /// [`crate::git::DiffTarget::Review`] target/repo root.
    ///
    /// `ChangedSinceAccepted` files persist as `PersistedStatus::Accepted`
    /// with their **preserved** (stale) blob SHA, not a fresh one — this is
    /// deliberate: the status itself is never set by a live gesture (only
    /// [`App::set_review_states`]'s load-time reconciliation ever produces
    /// it), so by construction the only way this function ever sees one is
    /// "still hasn't been re-accepted since it went stale", and persisting
    /// its original SHA unchanged is exactly what keeps the *next* session's
    /// reconciliation re-deriving `ChangedSinceAccepted` again rather than
    /// silently "fixing" the staleness by accident.
    fn persist_review_state(&mut self) {
        let Some(state_path) = self.review_state_path.clone() else {
            return;
        };
        let DiffTarget::Review { base, branch } = self.target.clone() else {
            return;
        };
        let Some(worktree_path) = self.repo_root.clone() else {
            return;
        };

        let mut files: BTreeMap<String, PersistedFile> = BTreeMap::new();
        for (path, status) in &self.review_states {
            let persisted_status = match status {
                ReviewStatus::Accepted | ReviewStatus::ChangedSinceAccepted => {
                    PersistedStatus::Accepted
                }
                ReviewStatus::Deferred => PersistedStatus::Deferred,
                ReviewStatus::Unreviewed => continue,
            };
            files.insert(
                path.clone(),
                PersistedFile {
                    status: persisted_status,
                    blob_sha: self.review_blob_shas.get(path).cloned().flatten(),
                },
            );
        }
        let review = PersistedReview {
            base,
            worktree_path,
            files,
        };
        self.review_save_tasks.spawn(move || {
            crate::review::store::save_review(&state_path, &branch, review)
                .map_err(|e| e.to_string())
        });
        self.review_saves_pending += 1;
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
        self.maybe_capture_blob_sha(&path, next);
        self.view.set_collapsed(&path, next == collapse_when);
        self.rebuild_rows();
        if let Some(index) = self.view.files.iter().position(|f| f.path == path) {
            self.view.cursor = self.view.header_row_of_file[index];
            self.view.ensure_visible();
        }
        self.persist_review_state();
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
        self.persist_review_state();
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

    // -- Persistence wiring (spec 08 Unit 4) -----------------------------------

    use crate::git::GitError;
    use crate::review::store;
    use std::collections::HashMap as StdHashMap;
    use std::time::{Duration, Instant};

    /// A `StageOps` fake that answers `blob_sha` from a canned table and
    /// stubs every other method with an inert default — enough to exercise
    /// `App::persist_review_state`'s blob-SHA capture without a real git
    /// backend.
    struct BlobShaFake {
        shas: StdHashMap<(String, String), Option<String>>,
    }

    impl super::super::stage_ops::StageOps for BlobShaFake {
        fn diff(&self, _target: &DiffTarget) -> Result<Vec<crate::git::RawFilePatch>, GitError> {
            Ok(Vec::new())
        }
        fn status(&self) -> Result<Vec<crate::git::FileStatus>, GitError> {
            Ok(Vec::new())
        }
        fn stage_file(&self, _path: &str) -> Result<(), GitError> {
            Ok(())
        }
        fn unstage_file(&self, _path: &str) -> Result<(), GitError> {
            Ok(())
        }
        fn apply_cached(&self, _patch: &str) -> Result<(), GitError> {
            Ok(())
        }
        fn unapply_cached(&self, _patch: &str) -> Result<(), GitError> {
            Ok(())
        }
        fn read_worktree_file(&self, _path: &str) -> Option<Vec<u8>> {
            None
        }
        fn show_file(&self, _spec: &str) -> Option<String> {
            None
        }
        fn blob_sha(&self, branch: &str, path: &str) -> Result<Option<String>, GitError> {
            Ok(self
                .shas
                .get(&(branch.to_string(), path.to_string()))
                .cloned()
                .flatten())
        }
    }

    /// A review-session `App` with a real background-thread-capable fake
    /// backend and a tempdir-backed state path, ready to exercise
    /// `persist_review_state`'s real (backgrounded) write.
    fn persisting_review_app(shas: &[(&str, &str, &str)]) -> (App, tempfile::TempDir) {
        let mut table = StdHashMap::new();
        for (branch, path, sha) in shas {
            table.insert(
                (branch.to_string(), path.to_string()),
                Some(sha.to_string()),
            );
        }
        let mut app = review_app(&["a.rs", "b.rs"]);
        app.stage_ops = Some(Box::new(BlobShaFake { shas: table }));
        app.repo_root = Some(std::path::PathBuf::from("/tmp/redquill-worktrees/feature"));
        let tmp = tempfile::TempDir::new().unwrap();
        let state_path = tmp.path().join("review-state.json");
        app.set_review_state_path(state_path);
        (app, tmp)
    }

    /// Polls until every in-flight review-state save has drained (or a 5s
    /// deadline passes) — the same drain-loop shape
    /// `commit_integration_tests.rs`'s `wait_for_commit` uses for `git_op`,
    /// checking `review_saves_pending` instead.
    fn wait_for_review_save(app: &mut App) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.review_saves_pending > 0 && Instant::now() < deadline {
            app.poll_review_save();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            app.review_saves_pending, 0,
            "review save did not complete in time"
        );
    }

    #[test]
    fn accepting_a_file_persists_its_blob_sha_to_disk() {
        let (mut app, _tmp) = persisting_review_app(&[("feature", "a.rs", "sha-a-1")]);
        let path = app.review_state_path.clone().unwrap();

        app.apply(Action::ToggleAccept); // a.rs
        wait_for_review_save(&mut app);

        let state = store::load(&path);
        let review = state.reviews.get("feature").expect("branch entry saved");
        assert_eq!(review.base, "main");
        let entry = review.files.get("a.rs").expect("a.rs entry saved");
        assert_eq!(entry.status, store::PersistedStatus::Accepted);
        assert_eq!(entry.blob_sha.as_deref(), Some("sha-a-1"));
    }

    #[test]
    fn un_accepting_removes_the_files_entry_from_the_saved_state() {
        let (mut app, _tmp) = persisting_review_app(&[("feature", "a.rs", "sha-a-1")]);
        let path = app.review_state_path.clone().unwrap();

        app.apply(Action::ToggleAccept);
        wait_for_review_save(&mut app);
        app.apply(Action::ToggleAccept); // un-accept
        wait_for_review_save(&mut app);

        let state = store::load(&path);
        let review = state.reviews.get("feature").expect("branch entry saved");
        assert!(
            !review.files.contains_key("a.rs"),
            "an un-accepted file must not linger in the saved state"
        );
    }

    #[test]
    fn deferring_persists_with_no_blob_sha() {
        let (mut app, _tmp) = persisting_review_app(&[]);
        let path = app.review_state_path.clone().unwrap();

        app.apply(Action::ToggleDefer); // a.rs
        wait_for_review_save(&mut app);

        let state = store::load(&path);
        let entry = state.reviews["feature"].files.get("a.rs").unwrap();
        assert_eq!(entry.status, store::PersistedStatus::Deferred);
        assert_eq!(entry.blob_sha, None);
    }

    #[test]
    fn accept_file_s_also_persists() {
        let (mut app, _tmp) = persisting_review_app(&[("feature", "a.rs", "sha-s")]);
        let path = app.review_state_path.clone().unwrap();

        app.apply(Action::AcceptFile);
        wait_for_review_save(&mut app);

        let state = store::load(&path);
        let entry = state.reviews["feature"].files.get("a.rs").unwrap();
        assert_eq!(entry.blob_sha.as_deref(), Some("sha-s"));
    }

    #[test]
    fn re_accepting_a_changed_since_accepted_file_persists_the_fresh_sha() {
        let (mut app, _tmp) = persisting_review_app(&[("feature", "a.rs", "sha-fresh")]);
        let path = app.review_state_path.clone().unwrap();
        // Seed a.rs as already ChangedSinceAccepted with a stale SHA, as
        // `App::set_review_states` would after a reconciled load.
        app.review_states
            .insert("a.rs".to_string(), ReviewStatus::ChangedSinceAccepted);
        app.review_blob_shas
            .insert("a.rs".to_string(), Some("sha-stale".to_string()));

        app.apply(Action::ToggleAccept); // re-accept a.rs (cursor starts there)
        wait_for_review_save(&mut app);

        assert_eq!(app.review_status("a.rs"), ReviewStatus::Accepted);
        let state = store::load(&path);
        let entry = state.reviews["feature"].files.get("a.rs").unwrap();
        assert_eq!(
            entry.blob_sha.as_deref(),
            Some("sha-fresh"),
            "re-accepting must capture the fresh SHA, not the stale one"
        );
    }

    #[test]
    fn accepted_panel_un_accept_persists_too() {
        // A distinct call site from `apply_review_transition` (the panel's
        // `Space`/`Enter` un-accept, spec 08 Unit 5) — must persist on its
        // own, not just Space/S's shared path.
        let (mut app, _tmp) = persisting_review_app(&[("feature", "a.rs", "sha-a-1")]);
        let path = app.review_state_path.clone().unwrap();

        app.apply(Action::ToggleAccept); // accept a.rs
        wait_for_review_save(&mut app);
        app.refresh_accepted_list();
        app.staging_cursor = 0;
        app.un_accept_focused_file();
        wait_for_review_save(&mut app);

        let state = store::load(&path);
        let review = state.reviews.get("feature").expect("branch entry saved");
        assert!(
            !review.files.contains_key("a.rs"),
            "the panel's un-accept must persist too, not just Space/S's"
        );
    }

    #[test]
    fn persist_is_a_no_op_without_a_state_path() {
        // No `set_review_state_path` call: outside a review session with
        // persistence wired up (e.g. a plain in-memory test `App`), nothing
        // is spawned and nothing panics.
        let mut app = review_app(&["a.rs"]);
        app.apply(Action::ToggleAccept);
        assert!(app.review_save_tasks.poll().is_empty());
    }
}
