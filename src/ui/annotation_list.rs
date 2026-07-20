//! The annotation-list panel's handlers: opening/closing the panel, moving
//! its focus, acting on the focused annotation (jump / edit / delete), and
//! the panel's `/` filter (spec 12 FR-7/FR-8): while a filter is active,
//! `list_cursor` is a position within the filtered view, not a raw
//! annotation index — [`App::list_real_index`] is the one translation point
//! every verb/motion routes through.
//!
//! Kept out of `app.rs` so the coordinator stays thin; these methods drive
//! the annotation store and the diff view purely through `App`'s own state.
//!
//! [`App::delete_focused_annotation`] additionally saves-on-change via
//! `App::persist_review_state`, a no-op outside a review session — see
//! `super::review_ops`'s module doc.

use super::App;
use super::Mode;
use super::list_filter::ListFilter;
use super::list_panel;
use super::list_panel::ListEntry;

impl App {
    /// Toggles the annotation list panel: opens it from Normal/Visual, closes
    /// it from List. A no-op while another modal owns the keyboard. The
    /// filter is transient per-open (spec 12 Non-Goal 5): closing always
    /// drops it, so reopening never shows a stale query.
    pub(super) fn toggle_list(&mut self) {
        match self.mode {
            Mode::List => {
                self.mode = Mode::Normal;
                self.list_filter = None;
            }
            Mode::Compose
            | Mode::Staging
            | Mode::Panel { .. }
            | Mode::Search
            | Mode::Peek
            | Mode::Switcher
            | Mode::ReviewLauncher { .. }
            | Mode::CommitMessage
            | Mode::Finder
            | Mode::ProjectSearch
            | Mode::EndReview { .. }
            | Mode::ConfirmRemoteOp { .. }
            | Mode::ThreadView
            | Mode::SubmitForge => {}
            Mode::Normal | Mode::Visual { .. } => {
                let total = self.list_total_len();
                if total > 0 {
                    self.list_cursor = self.list_cursor.min(total - 1);
                }
                self.mode = Mode::List;
                self.motion_count = None;
            }
        }
    }

    /// Closes the annotation list panel, returning to [`Mode::Normal`] and
    /// dropping any active filter (transient per-open).
    pub fn close_list(&mut self) {
        self.mode = Mode::Normal;
        self.list_filter = None;
    }

    /// The list panel's unified entry order: every annotation (by store id),
    /// then every drafted reply (by reply id), each in its store's insertion
    /// order. The one index space the filter labels, the rendered rows, and
    /// every verb agree on — with no drafted replies it's exactly the
    /// annotation list, so the panel behaves identically to before replies
    /// existed.
    pub(super) fn list_entries(&self) -> Vec<ListEntry> {
        let mut entries: Vec<ListEntry> = self
            .annotations
            .iter()
            .map(|a| ListEntry::Annotation(a.id))
            .collect();
        entries.extend(self.replies.iter().map(|r| ListEntry::Reply(r.id)));
        entries
    }

    /// The total number of panel entries (annotations + replies) — the raw
    /// (unfiltered) row count.
    fn list_total_len(&self) -> usize {
        self.annotations.len() + self.replies.len()
    }

    /// Builds the annotation list's `/`-filterable labels, in the same
    /// insertion order `list_entries()`/`list_cursor` already index over, so
    /// a filtered position always maps back to the right entry (annotation or
    /// reply).
    fn list_filter_labels(&self) -> Vec<String> {
        self.list_entries()
            .iter()
            .map(|entry| self.entry_filter_label(entry))
            .collect()
    }

    /// The `/`-filterable label for one entry — an annotation's own label, or
    /// a reply's thread-anchored label (resolving the thread from the overlay
    /// for its location column).
    fn entry_filter_label(&self, entry: &ListEntry) -> String {
        match *entry {
            ListEntry::Annotation(id) => self
                .annotations
                .iter()
                .find(|a| a.id == id)
                .map(list_panel::filter_label)
                .unwrap_or_default(),
            ListEntry::Reply(id) => self
                .replies
                .get(id)
                .map(|reply| {
                    list_panel::reply_filter_label(reply, self.thread_overlay.find(reply.thread_id))
                })
                .unwrap_or_default(),
        }
    }

    /// The list panel's effective row count: the active filter's filtered
    /// view when one is set, the full entry count otherwise. Every motion
    /// clamps against this instead of `annotations.len()` directly, so
    /// paging/jumping moves through what the user sees (spec 12's
    /// filtered-view design constraint).
    fn list_effective_len(&self) -> usize {
        self.list_filter
            .as_ref()
            .map_or(self.list_total_len(), ListFilter::len)
    }

    /// Translates `list_cursor` (a filtered position while a filter is
    /// active, a raw index otherwise) into a real entry index. The one point
    /// every verb (jump/edit/delete) routes through.
    fn list_real_index(&self) -> Option<usize> {
        match &self.list_filter {
            Some(f) => f.real_index(self.list_cursor),
            None => (self.list_cursor < self.list_total_len()).then_some(self.list_cursor),
        }
    }

    /// The entry the list cursor is focused on, resolved through the active
    /// filter — the one dispatch point the jump/edit/delete verbs branch on.
    fn focused_list_entry(&self) -> Option<ListEntry> {
        let index = self.list_real_index()?;
        self.list_entries().into_iter().nth(index)
    }

    /// Enters filter mode (`/`): a no-op if it's already active (`/` while
    /// locked resumes editing instead — see [`App::list_resume_filter_editing`]).
    pub(super) fn list_enter_filter(&mut self) {
        if self.list_filter.is_none() {
            let labels = self.list_filter_labels();
            self.list_filter = Some(ListFilter::open(&labels));
        }
    }

    /// Resumes editing a locked filter (`/` while locked).
    pub(super) fn list_resume_filter_editing(&mut self) {
        if let Some(f) = self.list_filter.as_mut() {
            f.resume_editing();
        }
    }

    /// Locks the active filter (`Enter` while editing), handing key
    /// handling back to the list's own verbs.
    pub(super) fn list_lock_filter(&mut self) {
        if let Some(f) = self.list_filter.as_mut() {
            f.lock();
        }
    }

    /// Clears the active filter entirely (`Esc`).
    pub(super) fn list_clear_filter(&mut self) {
        self.list_filter = None;
        self.list_cursor = self
            .list_cursor
            .min(self.list_total_len().saturating_sub(1));
    }

    /// Appends `c` to the active filter's query and re-clamps the cursor
    /// into the freshly reranked view. A no-op if no filter is active.
    pub(super) fn list_filter_push_char(&mut self, c: char) {
        let labels = self.list_filter_labels();
        if let Some(f) = self.list_filter.as_mut() {
            f.push_char(c, &labels);
        }
        self.list_clamp_cursor_to_filter();
    }

    /// Deletes the last character of the active filter's query. A no-op if
    /// no filter is active.
    pub(super) fn list_filter_backspace(&mut self) {
        let labels = self.list_filter_labels();
        if let Some(f) = self.list_filter.as_mut() {
            f.backspace(&labels);
        }
        self.list_clamp_cursor_to_filter();
    }

    fn list_clamp_cursor_to_filter(&mut self) {
        if let Some(f) = self.list_filter.as_ref() {
            self.list_cursor = self.list_cursor.min(f.len().saturating_sub(1));
        }
    }

    /// Moves the list panel's focus down one row, clamped at the last.
    pub fn list_move_down(&mut self) {
        let len = self.list_effective_len();
        if len > 0 {
            self.list_cursor = (self.list_cursor + 1).min(len - 1);
        }
    }

    /// Moves the list panel's focus up one row, clamped at the first.
    pub fn list_move_up(&mut self) {
        self.list_cursor = self.list_cursor.saturating_sub(1);
    }

    /// The list panel's page-size proxy for half/full-page motions: the
    /// panel doesn't track its own render height, so this approximates it
    /// with the diff pane's own tracked viewport height (see
    /// `git_panel::App::panel_viewport_proxy` for the identical rationale).
    fn list_viewport_proxy(&self) -> usize {
        self.view.viewport_height()
    }

    /// Moves the list panel's focus down half a viewport (`Ctrl-d`; shared
    /// motion set, see `super::motion`).
    pub fn list_half_page_down(&mut self) {
        let step = super::motion::half_page(self.list_viewport_proxy());
        self.list_cursor =
            super::motion::step(self.list_cursor, self.list_effective_len(), step, true);
    }

    /// Moves the list panel's focus up half a viewport (`Ctrl-u`).
    pub fn list_half_page_up(&mut self) {
        let step = super::motion::half_page(self.list_viewport_proxy());
        self.list_cursor =
            super::motion::step(self.list_cursor, self.list_effective_len(), step, false);
    }

    /// Moves the list panel's focus down a full viewport (`Ctrl-f`).
    pub fn list_full_page_down(&mut self) {
        let step = super::motion::full_page(self.list_viewport_proxy());
        self.list_cursor =
            super::motion::step(self.list_cursor, self.list_effective_len(), step, true);
    }

    /// Moves the list panel's focus up a full viewport (`Ctrl-b`).
    pub fn list_full_page_up(&mut self) {
        let step = super::motion::full_page(self.list_viewport_proxy());
        self.list_cursor =
            super::motion::step(self.list_cursor, self.list_effective_len(), step, false);
    }

    /// Jumps the list panel's focus to the first row (`g`/`Home`).
    pub fn list_jump_to_top(&mut self) {
        self.list_cursor = super::motion::jump_top();
    }

    /// Jumps the list panel's focus to the last row (`G`/`End`).
    pub fn list_jump_to_bottom(&mut self) {
        self.list_cursor = super::motion::jump_bottom(self.list_effective_len());
    }

    /// Switches to the focused entry's location and closes the list panel:
    /// an annotation jumps to its file/anchor row; a reply jumps to its
    /// thread's anchor row. A no-op that just closes the panel if nothing is
    /// focused or the location can no longer be found.
    pub fn jump_to_focused_annotation(&mut self) {
        match self.focused_list_entry() {
            Some(ListEntry::Annotation(id)) => {
                self.list_filter = None;
                self.jump_to_annotation(id);
            }
            Some(ListEntry::Reply(id)) => {
                self.list_filter = None;
                self.jump_to_reply(id);
            }
            None => {
                self.mode = Mode::Normal;
                self.list_filter = None;
            }
        }
    }

    /// Moves the diff cursor to the anchor row of a drafted reply's thread
    /// and closes the panel. Falls back to just closing when the thread is no
    /// longer in the overlay (e.g. it wasn't re-fetched this session).
    fn jump_to_reply(&mut self, reply_id: usize) {
        let row = self
            .replies
            .get(reply_id)
            .and_then(|reply| self.thread_overlay.find(reply.thread_id))
            .and_then(|thread| self.thread_anchor_row(thread));
        if let Some(row) = row {
            self.view.cursor = row;
            self.view.scroll = 0;
            self.view.ensure_visible();
        }
        self.mode = Mode::Normal;
    }

    fn jump_to_annotation(&mut self, id: usize) {
        let Some(annotation) = self.annotations.iter().find(|a| a.id == id) else {
            self.mode = Mode::Normal;
            return;
        };
        let target = annotation.target.clone();
        let path = target.path().to_string();
        if let Some(index) = self.view.files.iter().position(|f| f.path == path) {
            // Expand the target section so line/hunk anchors are reachable,
            // then resolve the anchor against the whole buffer (a File target
            // lands on the section header; line/hunk/range targets resolve
            // within this file's row span). Fall back to the section header
            // if the specific anchor row can no longer be found.
            self.view.set_collapsed(&path, false);
            self.rebuild_rows();
            self.view.cursor = self
                .view
                .anchor_row_in_buffer(&target)
                .unwrap_or_else(|| self.view.header_row_of_file[index]);
            self.view.scroll = 0;
            self.view.ensure_visible();
            self.mode = Mode::Normal;
            return;
        }
        // Not in the currently-loaded buffer: a worktree-file-content target
        // re-opens via `open_file_view` at its anchor line; every other
        // target shape whose path isn't loaded has no reliable file-view
        // equivalent (a historical target's line numbers over the live
        // worktree file would be misleading), so it degrades to the
        // existing no-op-but-close-the-list behavior.
        if let Some(line) = target.worktree_anchor_line() {
            self.open_file_view(path, Some(line));
            return;
        }
        self.mode = Mode::Normal;
    }

    /// Opens Compose pre-filled with the focused entry for editing (the
    /// filtered selection, while a filter is active): an annotation edits its
    /// body/classification; a reply edits its body in reply mode.
    pub fn edit_focused_annotation(&mut self) {
        match self.focused_list_entry() {
            Some(ListEntry::Annotation(id)) => self.open_compose_for(id),
            Some(ListEntry::Reply(id)) => self.open_reply_compose_for(id),
            None => {}
        }
    }

    /// Opens Compose in reply mode over an existing draft reply. A no-op if
    /// the reply id is unknown.
    fn open_reply_compose_for(&mut self, reply_id: usize) {
        let Some(reply) = self.replies.get(reply_id) else {
            return;
        };
        self.compose = Some(super::compose::ComposeState::editing_reply(
            reply.id,
            reply.thread_id,
            &reply.body,
        ));
        self.mode = Mode::Compose;
    }

    /// Deletes the focused entry — annotation or reply (the filtered
    /// selection, while a filter is active). No confirmation — deletion is
    /// cheap to redo.
    pub fn delete_focused_annotation(&mut self) {
        match self.focused_list_entry() {
            Some(ListEntry::Annotation(id)) => self.delete_annotation_by_id(id),
            Some(ListEntry::Reply(id)) => self.delete_reply_by_id(id),
            None => {}
        }
    }

    /// Removes the annotation with `id` from the store, re-clamps the list
    /// cursor, rebuilds the diff rows, and saves-on-change. Shared by the
    /// list panel's delete and the diff view's in-place delete so both paths
    /// behave identically (no confirmation). A no-op if `id` is unknown.
    ///
    /// An active filter is reranked against the shrunken list (rather than
    /// dropped) so a delete keeps the reviewer in their narrowed view.
    pub(super) fn delete_annotation_by_id(&mut self, id: usize) {
        let _ = self.annotations.remove(id);
        self.after_list_delete();
    }

    /// Removes the drafted reply with `reply_id`, then runs the same
    /// filter-rerank / cursor-clamp / rebuild / save-on-change tail the
    /// annotation delete uses, so deleting a reply from the panel behaves
    /// identically. A no-op if `reply_id` is unknown.
    pub(super) fn delete_reply_by_id(&mut self, reply_id: usize) {
        if !self.replies.remove(reply_id) {
            return;
        }
        self.after_list_delete();
    }

    /// Shared tail for a panel delete (annotation or reply): reranks an
    /// active filter against the shrunken entry list (rather than dropping
    /// it, so the reviewer stays in their narrowed view), re-clamps the list
    /// cursor, rebuilds the diff rows, and saves-on-change.
    fn after_list_delete(&mut self) {
        if self.list_filter.is_some() {
            let labels = self.list_filter_labels();
            if let Some(f) = self.list_filter.as_mut() {
                f.refresh(&labels);
            }
        }
        let len = self.list_effective_len();
        self.list_cursor = if len == 0 {
            0
        } else {
            self.list_cursor.min(len - 1)
        };
        self.refresh_rows();
        // Save-on-change — see `review_ops`'s module doc for why this is
        // safe to call unconditionally outside a review session.
        self.persist_review_state();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotate::{Classification, Target};
    use crate::diff::FileDiff;
    use crate::git::RawFilePatch;
    use crate::ui::modes::handle_list_key;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    /// Three annotations with distinctly fuzzy-matchable bodies, in `Mode::List`.
    fn app_with_three_annotations() -> (App, usize) {
        let mut app = App::new(vec![sample_file()]);
        app.annotations
            .add(
                Target::file("src/main.rs"),
                Classification::Question,
                "alpha note",
            )
            .unwrap();
        let bravo_id = app
            .annotations
            .add(
                Target::file("src/main.rs"),
                Classification::Question,
                "bravo note",
            )
            .unwrap();
        app.annotations
            .add(
                Target::file("src/main.rs"),
                Classification::Question,
                "charlie note",
            )
            .unwrap();
        app.mode = Mode::List;
        (app, bravo_id)
    }

    // -- Filter + motion + verb composition (spec 12 FR-8) -----------------

    #[test]
    fn filter_narrows_and_edit_acts_on_the_filtered_selection() {
        let (mut app, bravo_id) = app_with_three_annotations();
        handle_list_key(&mut app, key('/'));
        for c in "bravo".chars() {
            handle_list_key(&mut app, key(c));
        }
        assert_eq!(
            app.list_filter.as_ref().unwrap().len(),
            1,
            "only the bravo annotation matches"
        );
        handle_list_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            !app.list_filter.as_ref().unwrap().is_editing(),
            "Enter must lock the filter"
        );
        handle_list_key(&mut app, key('e'));
        assert_eq!(app.mode, Mode::Compose);
        assert_eq!(
            app.compose.as_ref().unwrap().editing_id,
            Some(bravo_id),
            "e must edit the filtered (bravo) annotation, not list position 0"
        );
    }

    #[test]
    fn filter_narrows_motion_moves_within_it_and_delete_removes_the_right_one() {
        let (mut app, bravo_id) = app_with_three_annotations();
        // Narrow to "note" (matches all three), then step down once so the
        // filtered cursor sits on a row other than position 0 — proving
        // motion moves through the filtered view, not the raw list.
        handle_list_key(&mut app, key('/'));
        for c in "note".chars() {
            handle_list_key(&mut app, key(c));
        }
        handle_list_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.list_filter.as_ref().unwrap().len(), 3);
        handle_list_key(&mut app, key('j'));
        let real_index = app.list_real_index().unwrap();
        assert_eq!(
            app.annotations.iter().nth(real_index).unwrap().id,
            bravo_id,
            "insertion order puts bravo second"
        );
        handle_list_key(&mut app, key('d'));
        assert_eq!(app.annotations.len(), 2);
        assert!(
            app.annotations.iter().all(|a| a.id != bravo_id),
            "delete must remove the filtered (bravo) selection"
        );
    }

    #[test]
    fn a_query_with_no_matches_shows_the_empty_state_and_esc_clears_it() {
        let (mut app, _) = app_with_three_annotations();
        handle_list_key(&mut app, key('/'));
        for c in "zzz".chars() {
            handle_list_key(&mut app, key(c));
        }
        assert!(app.list_filter.as_ref().unwrap().is_empty());
        handle_list_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(
            app.list_filter.is_none(),
            "Esc must clear the filter entirely"
        );
        assert_eq!(
            app.mode,
            Mode::List,
            "Esc while filtering must not close the panel"
        );
    }

    #[test]
    fn closing_the_list_panel_drops_the_filter() {
        let (mut app, _) = app_with_three_annotations();
        handle_list_key(&mut app, key('/'));
        for c in "bravo".chars() {
            handle_list_key(&mut app, key(c));
        }
        handle_list_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.list_filter.is_some());
        app.close_list();
        assert!(
            app.list_filter.is_none(),
            "the filter is transient per-open (spec 12 Non-Goal 5)"
        );
    }

    // -- Journey B: 30 annotations, find-and-edit by filter (spec 12) -------

    /// Journey driver for spec 12's Success Metric 2: a session with 30
    /// annotations, `/`, three characters narrow the list, `Enter` locks,
    /// `e` edits the intended one — driven key by key through the real
    /// dispatch path. Every logged step is asserted, so this is a
    /// regression test as well as the transcript generator
    /// (`RQ_JOURNEY_DUMP=1 cargo test --lib find_annotation_journey_transcript
    /// -- --nocapture` captures the persisted `12-proofs/` transcript).
    #[test]
    fn find_annotation_journey_transcript() {
        let mut log = String::new();
        let mut step = |title: &str, body: &str| {
            log.push_str(&format!("\n=== {title} ===\n{body}\n"));
        };

        let mut app = App::new(vec![sample_file()]);
        // 29 routine annotations plus the one distinctive one below: 30
        // total, matching spec 12's Success Metric 2 exactly.
        for i in 0..29 {
            app.annotations
                .add(
                    Target::file("src/main.rs"),
                    Classification::Question,
                    format!("routine note {i}"),
                )
                .unwrap();
        }
        // The distinctive annotation: a three-character fragment of it is
        // the only thing that narrows the 30-annotation list down to a
        // single row.
        let zephyr_id = app
            .annotations
            .add(
                Target::file("src/main.rs"),
                Classification::Question,
                "zephyr: check the retry backoff here",
            )
            .unwrap();
        app.mode = Mode::List;
        step(
            "journey B: annotation list open over 30 annotations",
            &format!("annotations: {}", app.annotations.len()),
        );

        handle_list_key(&mut app, key('/'));
        assert!(app.list_filter.as_ref().unwrap().is_editing());
        step(
            "press /: filter mode entered, whole list still visible",
            &format!("filtered rows: {}", app.list_filter.as_ref().unwrap().len()),
        );

        for c in "zep".chars() {
            handle_list_key(&mut app, key(c));
        }
        assert_eq!(
            app.list_filter.as_ref().unwrap().len(),
            1,
            "three characters must narrow to exactly the zephyr annotation"
        );
        step(
            "type z, e, p: list narrows to the one matching annotation",
            &format!(
                "query: \"{}\"  filtered rows: {}",
                app.list_filter.as_ref().unwrap().query(),
                app.list_filter.as_ref().unwrap().len()
            ),
        );

        handle_list_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!app.list_filter.as_ref().unwrap().is_editing());
        step(
            "press Enter: filter locks, list verbs resume",
            "filter locked",
        );

        handle_list_key(&mut app, key('e'));
        assert_eq!(app.mode, Mode::Compose);
        assert_eq!(
            app.compose.as_ref().unwrap().editing_id,
            Some(zephyr_id),
            "e must edit the filtered (zephyr) annotation"
        );
        step(
            "press e: Compose opens on the filtered (zephyr) annotation",
            &format!("editing annotation id: {zephyr_id}"),
        );

        if std::env::var("RQ_JOURNEY_DUMP").is_ok() {
            eprintln!("{log}");
        }
    }

    // -- Draft replies in the annotation-list panel (FR-14) -----------------

    #[test]
    fn a_drafted_reply_appears_as_a_reply_entry_with_its_marker() {
        let mut app = App::new(vec![sample_file()]);
        app.annotations
            .add(Target::file("src/main.rs"), Classification::Issue, "fix")
            .unwrap();
        app.replies.add(42, "answering the thread");

        let entries = app.list_entries();
        assert_eq!(entries.len(), 2, "one annotation, then one reply");
        assert!(matches!(entries[0], ListEntry::Annotation(_)));
        assert!(matches!(entries[1], ListEntry::Reply(_)));

        // The reply's filter label carries the `↳` marker and the word
        // "reply", so a `/reply` filter surfaces exactly the drafted replies.
        let labels = app.list_filter_labels();
        assert!(
            labels[1].contains('\u{21b3}') && labels[1].contains("reply"),
            "reply label must carry its marker: {:?}",
            labels[1]
        );
    }

    #[test]
    fn editing_a_focused_reply_opens_compose_in_reply_mode() {
        let mut app = App::new(vec![sample_file()]);
        app.replies.add(77, "original reply");
        app.mode = Mode::List;
        app.list_cursor = 0; // the only entry is the reply

        app.edit_focused_annotation();

        assert_eq!(app.mode, Mode::Compose);
        let compose = app.compose.as_ref().unwrap();
        assert_eq!(compose.thread_id, Some(77));
        assert_eq!(compose.buffer.text(), "original reply");
    }

    #[test]
    fn deleting_a_focused_reply_removes_it_from_the_store() {
        let mut app = App::new(vec![sample_file()]);
        app.annotations
            .add(Target::file("src/main.rs"), Classification::Issue, "fix")
            .unwrap();
        app.replies.add(88, "to be deleted");
        app.mode = Mode::List;
        app.list_cursor = 1; // the reply follows the one annotation

        app.delete_focused_annotation();

        assert!(app.replies.is_empty(), "the reply must be gone");
        assert_eq!(app.annotations.len(), 1, "the annotation is untouched");
    }

    // -- Published indicator in the list panel (spec 13 T4.2) ---------------

    #[test]
    fn a_published_annotations_filter_label_carries_the_published_tag() {
        let mut app = App::new(vec![sample_file()]);
        let id = app
            .annotations
            .add(Target::file("src/main.rs"), Classification::Issue, "fix")
            .unwrap();
        app.annotations.set_published(id, true).unwrap();

        let labels = app.list_filter_labels();
        assert!(
            labels[0].starts_with("[published] "),
            "a published annotation's label must lead with the published tag: {:?}",
            labels[0]
        );
    }

    #[test]
    fn an_unpublished_annotations_filter_label_carries_no_published_tag() {
        let mut app = App::new(vec![sample_file()]);
        app.annotations
            .add(Target::file("src/main.rs"), Classification::Issue, "fix")
            .unwrap();

        let labels = app.list_filter_labels();
        assert!(!labels[0].contains("[published]"));
    }

    #[test]
    fn a_published_replys_filter_label_carries_the_published_tag() {
        let mut app = App::new(vec![sample_file()]);
        let id = app.replies.add(42, "answering the thread").unwrap();
        app.replies.set_published(id, true);

        let labels = app.list_filter_labels();
        assert!(
            labels[0].starts_with("[published] "),
            "a published reply's label must lead with the published tag: {:?}",
            labels[0]
        );
        assert!(
            labels[0].contains('\u{21b3}') && labels[0].contains("reply"),
            "the published tag must not replace the existing reply marker: {:?}",
            labels[0]
        );
    }

    #[test]
    fn an_unpublished_replys_filter_label_carries_no_published_tag() {
        let mut app = App::new(vec![sample_file()]);
        app.replies.add(42, "answering the thread");

        let labels = app.list_filter_labels();
        assert!(!labels[0].contains("[published]"));
    }
}
