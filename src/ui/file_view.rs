//! The read-only whole-file view (spec 06 Unit 1): opens any worktree file —
//! not just files with a diff — as a synthesized all-context body via the
//! [`DiffTarget::File`] capability variant, reusing the diff rendering
//! surface, the existing scroll/jump motions, and the suspend/restore
//! mechanism [`super::git_panel`]'s commit view pioneered.
//!
//! Deliberately *not* a new [`Mode`] variant: the file view is
//! [`Mode::Normal`] over a `File` target, so every Normal-mode navigation
//! gesture works unchanged, and the existing capability gating
//! (`target.staging_mode()` / `target.supports_code_intel()`, both already
//! wired into the footer/help overlay per the spec 05 pattern) hides
//! staging/commit/LSP keys automatically — no new gating logic needed here.
//!
//! Suspension uses its own field ([`App::suspended_file_view`]), not
//! [`super::app::SuspendedView`]'s sibling `suspended_view` field (commit
//! views): the two are independent so a file opened from within a commit
//! view suspends the commit view (not the true original state), and `Esc`
//! unwinds one layer at a time, mirroring how nested commit opens already
//! collapse into a single suspension rather than stacking.

use crate::diff::FileDiff;
use crate::git::DiffTarget;

use super::app::{App, Mode, SuspendedView};
use super::code_intel::closest_row_for_new_line;
use super::diff_view_state::DiffViewState;

impl App {
    /// Opens `path` (repo-relative) as a read-only whole-file view, cursor
    /// landing on `line` (1-based) if given, else the top of the file.
    /// `Esc` from the view restores `Mode::Normal` — see
    /// [`App::open_file_view_with_return_mode`] for opening with a different
    /// restore mode (Project Search's confirm gesture, spec 06 Unit 2).
    ///
    /// Suspends the prior view the first time a file view is opened while
    /// none is already showing; a nested open (a file view opened from
    /// within another file view — not reachable via the finder alone today,
    /// but kept consistent with [`super::git_panel::App::open_commit_view`]'s
    /// nested-commit behavior) replaces the displayed file but leaves the
    /// original suspension untouched, so `Esc` always returns to the true
    /// starting point.
    ///
    /// Degrades to a footer message, leaving the current view untouched, on:
    /// no git backend attached, an unreadable path, or non-UTF-8 (binary)
    /// content — the file view has no rendering for binary content today.
    pub(super) fn open_file_view(&mut self, path: String, line: Option<u32>) {
        self.open_file_view_with_return_mode(path, line, Mode::Normal);
    }

    /// [`App::open_file_view`], but `Esc` restores `return_mode` instead of
    /// always `Mode::Normal` — used by Project Search's confirm gesture
    /// (spec 06 Unit 2) so opening a hit while in `Mode::ProjectSearch`
    /// lands back there (query/toggles/results/selection intact) rather than
    /// falling through to the diff. `return_mode` is captured only on the
    /// first-level open (mirrors `suspended_file_view`'s own nested-open
    /// rule below): a second file opened without returning must not
    /// overwrite the true restore target.
    pub(super) fn open_file_view_with_return_mode(
        &mut self,
        path: String,
        line: Option<u32>,
        return_mode: Mode,
    ) {
        let Some(ops) = self.stage_ops.as_deref() else {
            self.set_status_message("file view unavailable (no git backend)");
            return;
        };
        let Some(bytes) = ops.read_worktree_file(&path) else {
            self.set_status_message(format!("could not read {path}"));
            return;
        };
        let content = match String::from_utf8(bytes) {
            Ok(content) => content,
            Err(_) => {
                self.set_status_message(format!("{path}: not valid UTF-8 (binary?)"));
                return;
            }
        };
        let file = FileDiff::synthetic_context(path.clone(), &content);
        let target = DiffTarget::File(path);

        if self.suspended_file_view.is_none() {
            self.file_view_return_mode = return_mode;
            let new_view = DiffViewState::new(vec![file]);
            let old_view = std::mem::replace(&mut self.view, new_view);
            self.suspended_file_view = Some(SuspendedView {
                target: std::mem::replace(&mut self.target, target),
                view: old_view,
                patches: std::mem::replace(&mut self.patches, vec![None]),
                staged: std::mem::take(&mut self.staged),
                staged_states: std::mem::take(&mut self.staged_states),
            });
        } else {
            self.target = target;
            self.view = DiffViewState::new(vec![file]);
            self.patches = vec![None];
        }

        // The just-suspended (or just-replaced) content shares the highlight
        // cache by path; clearing it prevents cross-contamination between
        // the suspended view's cached spans and the file view's (mirrors
        // `open_commit_view`).
        self.highlight_cache.clear();
        self.rebuild_rows();

        if let Some(line) = line {
            let local = closest_row_for_new_line(&self.view.rows, line).unwrap_or(0);
            self.view.cursor = local;
            self.view.scroll = 0;
            self.view.ensure_visible();
        }
        self.mode = Mode::Normal;
    }

    /// Whether a read-only file view is currently suspending a prior view
    /// (see [`App::suspended_file_view`]) — mirrors
    /// [`super::git_panel::App::viewing_commit`]'s role for commit views.
    pub(super) fn viewing_file(&self) -> bool {
        self.suspended_file_view.is_some()
    }

    /// Restores the view suspended by [`App::open_file_view`] (`Esc` from the
    /// file view): the prior target, diff-view state (files, rows, cursor,
    /// scroll, collapse map), patches, and staged state all come back
    /// verbatim, and focus returns to whatever mode was captured on open —
    /// `Mode::Normal` for every opener except Project Search's confirm
    /// gesture, which lands back in `Mode::ProjectSearch` (see
    /// [`App::open_file_view_with_return_mode`]). A no-op if no file view is
    /// open.
    pub(super) fn return_from_file_view(&mut self) {
        let Some(suspended) = self.suspended_file_view.take() else {
            return;
        };
        self.target = suspended.target;
        self.view = suspended.view;
        self.patches = suspended.patches;
        self.staged = suspended.staged;
        self.staged_states = suspended.staged_states;
        self.highlight_cache.clear();
        self.rebuild_rows();
        self.mode = self.file_view_return_mode;
        self.file_view_return_mode = Mode::Normal;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::FileDiff as DomainFileDiff;
    use crate::git::{GitError, RawFilePatch, StagingMode};
    use crate::ui::keymap::Action;
    use crate::ui::stage_ops::StageOps;
    use crate::ui::{Row, footer, help};
    use std::collections::HashMap;

    fn sample_file(path: &str) -> DomainFileDiff {
        let raw = format!(
            "diff --git a/{path} b/{path}\nindex 1..2 100644\n--- a/{path}\n+++ b/{path}\n@@ -1,2 +1,2 @@\n fn main() {{\n-    old();\n+    new();\n"
        );
        DomainFileDiff::from_patch(&RawFilePatch {
            path: path.to_string(),
            old_path: None,
            raw,
            is_binary: false,
        })
        .unwrap()
    }

    /// A fake `StageOps` serving fixed worktree file contents by path, so
    /// the file-view tests don't need a real git repo.
    struct FakeOps {
        files: HashMap<String, Vec<u8>>,
    }

    impl StageOps for FakeOps {
        fn diff(&self, _target: &DiffTarget) -> Result<Vec<RawFilePatch>, GitError> {
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
        fn read_worktree_file(&self, path: &str) -> Option<Vec<u8>> {
            self.files.get(path).cloned()
        }
        fn show_file(&self, _spec: &str) -> Option<String> {
            None
        }
    }

    fn app_with_file_content(path: &str, content: &str) -> App {
        let mut app = App::new(vec![sample_file("src/main.rs")]);
        let mut files = HashMap::new();
        files.insert(path.to_string(), content.as_bytes().to_vec());
        app.stage_ops = Some(Box::new(FakeOps { files }));
        app.mode = Mode::Normal;
        app
    }

    // -- open_file_view: content and capability gating -----------------

    #[test]
    fn open_file_view_synthesizes_all_context_content() {
        let mut app = app_with_file_content("docs/notes.md", "one\ntwo\nthree\n");
        app.open_file_view("docs/notes.md".to_string(), None);

        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.target, DiffTarget::File("docs/notes.md".to_string()));
        assert_eq!(app.view.files.len(), 1);
        assert_eq!(app.view.files[0].path, "docs/notes.md");
        let lines: Vec<&str> = app
            .view
            .rows
            .iter()
            .filter_map(|r| match r {
                Row::Line(l) => Some(l.content.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(lines, vec!["one", "two", "three"]);
    }

    #[test]
    fn open_file_view_is_read_only_and_code_intel_free() {
        let mut app = app_with_file_content("a.rs", "fn main() {}\n");
        app.open_file_view("a.rs".to_string(), None);
        assert_eq!(app.target.staging_mode(), StagingMode::ReadOnly);
        assert!(!app.target.supports_code_intel());
    }

    #[test]
    fn open_file_view_without_backend_sets_footer_message_and_leaves_view_unchanged() {
        let mut app = App::new(vec![sample_file("src/main.rs")]);
        let prior_target = app.target.clone();
        app.open_file_view("anything.rs".to_string(), None);
        assert_eq!(app.target, prior_target, "no backend: target unchanged");
        assert!(app.status_message.is_some());
        assert!(!app.viewing_file());
    }

    #[test]
    fn open_file_view_on_unreadable_path_degrades_to_a_footer_message() {
        let mut app = app_with_file_content("a.rs", "content\n");
        app.open_file_view("missing.rs".to_string(), None);
        assert!(app.status_message.is_some());
        assert!(!app.viewing_file(), "a failed open must not suspend");
    }

    #[test]
    fn open_file_view_on_non_utf8_content_degrades_to_a_footer_message() {
        let mut app = App::new(vec![sample_file("src/main.rs")]);
        let mut files = HashMap::new();
        files.insert("binary.bin".to_string(), vec![0xff, 0xfe, 0x00, 0xff]);
        app.stage_ops = Some(Box::new(FakeOps { files }));
        app.open_file_view("binary.bin".to_string(), None);
        assert!(app.status_message.is_some());
        assert!(!app.viewing_file());
    }

    // -- open-at-line -----------------------------------------------------

    #[test]
    fn open_file_view_at_line_positions_the_cursor_on_that_line() {
        let mut app = app_with_file_content("a.rs", "one\ntwo\nthree\nfour\n");
        app.open_file_view("a.rs".to_string(), Some(3));
        let Row::Line(line) = &app.view.rows[app.view.cursor] else {
            panic!("cursor must land on a Line row");
        };
        assert_eq!(line.new_line, Some(3));
        assert_eq!(line.content, "three");
    }

    // -- suspend / restore round trip --------------------------------------

    #[test]
    fn open_then_return_restores_prior_target_cursor_and_mode() {
        let mut app = app_with_file_content("other.rs", "x\ny\nz\n");
        app.view.cursor = 2;
        app.view.scroll = 1;
        let prior_target = app.target.clone();
        let prior_cursor = app.view.cursor;
        let prior_scroll = app.view.scroll;

        app.open_file_view("other.rs".to_string(), None);
        assert!(app.viewing_file());
        assert_ne!(app.target, prior_target);

        // Navigate around inside the file view — must not corrupt the
        // suspended state.
        app.view.cursor = app.view.cursor.min(app.view.max_cursor());

        app.return_from_file_view();

        assert_eq!(app.mode, Mode::Normal);
        assert!(!app.viewing_file());
        assert_eq!(app.target, prior_target, "prior target restored");
        assert_eq!(app.view.cursor, prior_cursor, "prior cursor restored");
        assert_eq!(app.view.scroll, prior_scroll, "prior scroll restored");
    }

    #[test]
    fn return_from_file_view_without_one_open_is_a_no_op() {
        let mut app = App::new(vec![sample_file("src/main.rs")]);
        let prior_target = app.target.clone();
        app.return_from_file_view();
        assert_eq!(app.target, prior_target);
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn opening_a_second_file_without_returning_still_restores_the_original_state() {
        let mut app = app_with_file_content("a.rs", "aaa\n");
        let mut files2 = HashMap::new();
        files2.insert("a.rs".to_string(), b"aaa\n".to_vec());
        files2.insert("b.rs".to_string(), b"bbb\n".to_vec());
        app.stage_ops = Some(Box::new(FakeOps { files: files2 }));
        let prior_target = app.target.clone();

        app.open_file_view("a.rs".to_string(), None);
        let first_opened = app.target.clone();
        app.open_file_view("b.rs".to_string(), None);
        let second_opened = app.target.clone();
        assert_ne!(first_opened, second_opened);

        app.return_from_file_view();
        assert_eq!(
            app.target, prior_target,
            "Esc must restore the true original state, not the first file"
        );
    }

    // -- footer/help omission assertion (capability gating) ----------------

    #[test]
    fn file_view_target_hides_staging_and_code_intel_from_footer_and_help() {
        let mut app = app_with_file_content("a.rs", "fn main() {}\n");
        app.open_file_view("a.rs".to_string(), None);

        let staging_allowed = app.target.staging_mode() != StagingMode::ReadOnly;
        let code_intel_allowed = app.target.supports_code_intel();
        assert!(!staging_allowed);
        assert!(!code_intel_allowed);

        let keymap = crate::ui::Keymap::default_map();
        let entries = footer::build_hints(
            app.mode,
            footer::FooterFlags {
                staging_allowed,
                code_intel_allowed,
                push_publishes: false,
                viewing_commit: false,
                help_open: false,
                project_search_focus: app.project_search_focus(),
                review_session: app.in_review_session(),
            },
            None,
            &keymap,
            &app.modal_keys,
        );
        assert!(
            !entries.iter().any(|e| e.label.contains("stage")),
            "no staging hint may appear in the file-view footer: {entries:?}"
        );

        for action in [Action::ToggleStage, Action::StageFile] {
            assert!(help::binding_hidden(
                action,
                staging_allowed,
                code_intel_allowed
            ));
        }
        for action in [
            Action::GotoDefinition,
            Action::GotoReferences,
            Action::Hover,
        ] {
            assert!(help::binding_hidden(
                action,
                staging_allowed,
                code_intel_allowed
            ));
        }
    }
}
