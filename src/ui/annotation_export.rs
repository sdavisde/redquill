//! Handing the reviewer their annotations without quitting: the clipboard
//! half of the `submit-forge-review` gesture (`U`).
//!
//! A forge PR review submits to the forge; every other target — the working
//! tree, `--staged`, a commit or range, a local branch review with no PR
//! behind it — has nowhere to submit, so the same key copies the annotations
//! to the clipboard in the public markdown format (see
//! [`crate::annotate::render_markdown`], the same bytes `main` presents on
//! quit). Nothing is consumed or cleared by a copy: the reviewer can keep
//! annotating and copy again, and the same annotations still reach `main`'s
//! on-quit presentation. See [`super::forge_submit::App::open_submit_forge`]
//! for the branch that picks between the two destinations.
//!
//! The copy is synchronous. It is a user-initiated one-shot on a cached
//! clipboard handle (see [`crate::clipboard`]) rather than a per-tick or
//! per-keystroke cost, so it doesn't belong on a background thread the way
//! git subprocesses and state saves do.

use crate::annotate::render_markdown;

use super::app::App;

/// The footer line for a copy attempt: `count` annotations were offered and
/// `error` is the clipboard's own message if the write failed.
///
/// Pure and separate from the gesture so the one thing that actually matters
/// here is testable without a system clipboard: a failed copy must never read
/// as a successful one. A reviewer who is told their review is on the
/// clipboard, and then pastes an empty buffer into an agent prompt, has lost
/// the review — so "clipboard unavailable" has to be as visible as the
/// success line.
pub(super) fn copy_status_message(count: usize, error: Option<&str>) -> String {
    match (count, error) {
        (0, _) => "no annotations to copy".to_string(),
        (_, Some(e)) => format!("clipboard unavailable ({e}) \u{2014} nothing copied"),
        (1, None) => "copied 1 annotation to the clipboard".to_string(),
        (n, None) => format!("copied {n} annotations to the clipboard"),
    }
}

impl App {
    /// Copies every annotation in this session to the clipboard as markdown
    /// and reports the outcome in the footer. A no-op (beyond the footer
    /// line) with nothing to copy.
    ///
    /// There is no stdout fallback here, unlike `main`'s on-quit
    /// presentation: stdout carries the annotation format for other programs
    /// to parse, and writing to it mid-render would corrupt the screen. A
    /// clipboard failure is reported and nothing else happens — the reviewer
    /// still has the on-quit path, and the annotations are untouched.
    pub(super) fn copy_annotations_to_clipboard(&mut self) {
        self.copy_annotations_with(|markdown| {
            crate::clipboard::copy(markdown).map_err(|e| e.to_string())
        });
    }

    /// [`App::copy_annotations_to_clipboard`]'s body, with the clipboard
    /// write injected — the seam this repo puts in front of every external
    /// service, kept as a plain closure rather than a trait because there is
    /// exactly one operation and no state behind it.
    ///
    /// It also means the tests never touch the developer's real clipboard:
    /// `cargo test` runs on a machine whose clipboard belongs to a person,
    /// not to the suite, so a test that exercised the live path would be a
    /// host side effect of the kind this repo's tempdir rules exist to
    /// prevent. `copy` is not called at all when there is nothing to copy.
    fn copy_annotations_with(&mut self, copy: impl FnOnce(&str) -> Result<(), String>) {
        let count = self.annotations.len();
        if count == 0 {
            self.set_status_message(copy_status_message(0, None));
            return;
        }
        let markdown = render_markdown(&self.annotations);
        let error = copy(&markdown).err();
        self.set_status_message(copy_status_message(count, error.as_deref()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotate::{Classification, Target};
    use crate::diff::FileDiff;
    use crate::git::RawFilePatch;
    use std::cell::RefCell;

    /// "… to the clipboard" is the phrase that constitutes the promise; a
    /// message without it is not claiming the annotations got there.
    fn claims_success(message: &str) -> bool {
        message.contains("to the clipboard")
    }

    /// The guardrail: whichever way a copy goes, the footer must not tell the
    /// reviewer their annotations are on the clipboard unless they are.
    #[test]
    fn a_failed_or_empty_copy_never_reads_as_a_successful_one() {
        let ok = copy_status_message(3, None);
        assert!(claims_success(&ok), "got {ok:?}");
        assert!(ok.contains('3'), "the count must be reported: {ok:?}");

        let failed = copy_status_message(3, Some("no display server"));
        assert!(
            !claims_success(&failed),
            "a failed copy must not read as a success: {failed:?}"
        );
        assert!(
            failed.contains("no display server"),
            "the clipboard's own reason must surface: {failed:?}"
        );

        let empty = copy_status_message(0, None);
        assert!(
            !claims_success(&empty),
            "an empty set must not claim a copy: {empty:?}"
        );
    }

    fn app_with_annotations(bodies: &[&str]) -> App {
        let raw = "\
diff --git a/src/a.rs b/src/a.rs
index 111..222 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -1,2 +1,2 @@
 fn main() {
-    old();
+    new();
";
        let file = FileDiff::from_patch(&RawFilePatch {
            path: "src/a.rs".to_string(),
            old_path: None,
            raw: raw.to_string(),
            is_binary: false,
        })
        .unwrap();
        let mut app = App::new(vec![file]);
        for body in bodies {
            app.annotations
                .add(Target::file("src/a.rs"), Classification::Question, *body)
                .unwrap();
        }
        app
    }

    /// What reaches the clipboard is the public markdown format over the
    /// whole annotation set — the same bytes `main` presents on quit, since
    /// the point of the gesture is to hand an agent a review without quitting
    /// first. The defect this catches is the copy shipping a partial set (a
    /// stray `unpublished()` filter, say) or some other rendering.
    #[test]
    fn the_copy_hands_over_the_public_markdown_for_every_annotation() {
        let mut app = app_with_annotations(&["first note", "second note"]);
        let seen = RefCell::new(None);

        app.copy_annotations_with(|markdown| {
            *seen.borrow_mut() = Some(markdown.to_string());
            Ok(())
        });

        let copied = seen.into_inner().expect("the clipboard write must run");
        assert_eq!(copied, crate::annotate::render_markdown(&app.annotations));
        assert!(copied.contains("first note"), "got {copied:?}");
        assert!(copied.contains("second note"), "got {copied:?}");
        assert!(
            app.status_message.as_deref().is_some_and(claims_success),
            "got {:?}",
            app.status_message
        );
        assert_eq!(
            app.annotations.len(),
            2,
            "a copy must not consume the annotations — the reviewer keeps working"
        );
    }

    /// Nothing to copy means the clipboard is never touched at all, rather
    /// than an empty string replacing whatever the reviewer had on it.
    #[test]
    fn an_empty_annotation_set_never_reaches_the_clipboard() {
        let mut app = app_with_annotations(&[]);
        let called = RefCell::new(false);

        app.copy_annotations_with(|_| {
            *called.borrow_mut() = true;
            Ok(())
        });

        assert!(!called.into_inner(), "the clipboard must be left alone");
        assert!(
            app.status_message
                .as_deref()
                .is_some_and(|m| !claims_success(m)),
            "got {:?}",
            app.status_message
        );
    }

    /// A clipboard that refuses is reported, not swallowed — the reviewer
    /// must not paste an empty buffer into an agent prompt believing the
    /// review went with it.
    #[test]
    fn a_clipboard_failure_surfaces_instead_of_reading_as_success() {
        let mut app = app_with_annotations(&["a note"]);

        app.copy_annotations_with(|_| Err("no display server".to_string()));

        let message = app.status_message.as_deref().unwrap_or_default();
        assert!(!claims_success(message), "got {message:?}");
        assert!(message.contains("no display server"), "got {message:?}");
    }
}
