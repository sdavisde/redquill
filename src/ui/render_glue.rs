//! Row and highlight assembly: [`super::App`]'s side of the seam between the
//! diff model and the rendered multibuffer. `rebuild_rows` populates the
//! syntax-highlight cache and concatenates every file's rows into the one
//! buffer [`super::DiffViewState`] renders; `refresh_rows` is the lighter
//! post-annotation-mutation rebuild. Kept out of `app.rs` so the coordinator
//! stays thin; these are the shared mutation points many gestures funnel
//! through, so their signatures are unchanged.

use crate::annotate::Side;
use crate::review::ReviewStatus;

use super::App;
use super::rows::{ReviewMarker, StagedMarker, SyntaxSpans, build_multibuffer};
use super::stage_ops::StagedState;
use super::syntax::{self, ContentSource};

impl App {
    /// Rebuilds `rows` for the currently selected file against the current
    /// `annotations`, then re-clamps the cursor. Called after any mutation
    /// to the annotation store so inline display/gutter markers stay in
    /// sync.
    pub(super) fn refresh_rows(&mut self) {
        if self.view.files.get(self.view.selected_file).is_some() {
            self.rebuild_rows();
            self.view.cursor = self
                .view
                .nearest_addressable(self.view.cursor.min(self.view.max_cursor()), true);
            self.view.ensure_visible();
        }
    }

    /// Rebuilds the whole multi-file row buffer: populates the syntax-highlight
    /// cache for the in-use side(s) of every *expanded* file, then concatenates
    /// every file's rows into one buffer via [`build_multibuffer`], carrying
    /// per-file collapse state and staged markers. Also recomputes the active
    /// search's matches and re-derives `selected_file` from the cursor. This is
    /// `App`'s side of the seam: highlighting and the git backend live here,
    /// and the built buffer is fed into [`super::DiffViewState`].
    ///
    /// Each blob is highlighted at most once however many views reach it — the
    /// cache is keyed by content source, not by file — and is re-highlighted
    /// only when its bytes could have changed. A collapsed file shows only a
    /// header and is never highlighted until expanded.
    pub(super) fn rebuild_rows(&mut self) {
        // Resolved once and reused twice below, by the populate pass and by
        // the span lookup.
        let sources: Vec<[Option<ContentSource>; 2]> = (0..self.view.files.len())
            .map(|i| self.highlight_sources(i))
            .collect();

        for index in 0..self.view.files.len() {
            let (Some(pair), Some(file)) = (sources.get(index), self.view.files.get(index)) else {
                continue;
            };
            let path = file.path.clone();
            for source in pair.iter().flatten() {
                syntax::populate_cache(
                    &mut self.highlight_cache,
                    &mut self.highlighter,
                    self.stage_ops.as_deref(),
                    source,
                    &path,
                );
            }
        }

        let collapsed: Vec<bool> = self
            .view
            .files
            .iter()
            .map(|f| self.view.is_collapsed(&f.path))
            .collect();
        let markers: Vec<StagedMarker> = self
            .view
            .files
            .iter()
            .map(
                |f| match self.staged_states.get(&f.path).copied().unwrap_or_default() {
                    StagedState::Full => StagedMarker::Staged,
                    StagedState::Partial => StagedMarker::Partial,
                    StagedState::Unstaged => StagedMarker::None,
                },
            )
            .collect();
        // `review_states` is only ever non-empty during a review session
        // (see its doc on `App`), so this is `ReviewMarker::None` for every
        // file outside one — no `in_review_session()` check needed here.
        let review_markers: Vec<ReviewMarker> = self
            .view
            .files
            .iter()
            .map(|f| match self.review_status(&f.path) {
                ReviewStatus::Unreviewed => ReviewMarker::None,
                ReviewStatus::Accepted => ReviewMarker::Accepted,
                ReviewStatus::Deferred => ReviewMarker::Deferred,
                ReviewStatus::ChangedSinceAccepted => ReviewMarker::Changed,
            })
            .collect();
        // A file with no cached spans (a side with no content to source, or a
        // language with no grammar) renders unhighlighted — the same silent
        // degradation as before.
        let syntax: Vec<SyntaxSpans> = sources
            .iter()
            .map(|[new, old]| SyntaxSpans {
                new: new
                    .as_ref()
                    .map_or(&[][..], |s| self.highlight_cache.get(s)),
                old: old
                    .as_ref()
                    .map_or(&[][..], |s| self.highlight_cache.get(s)),
            })
            .collect();

        // Published annotations whose forge copy is already shown in the
        // thread overlay are dropped from the row build (the forge copy is
        // authoritative on screen); the real store is untouched, so the list
        // panel and stdout serialization still see every annotation. The
        // common case (no suppression) reuses the store directly with no
        // clone.
        let suppressed = self.suppressed_published_annotation_ids();
        let filtered;
        let annotations_for_rows = if suppressed.is_empty() {
            &self.annotations
        } else {
            filtered = self.annotations.without_ids(&suppressed);
            &filtered
        };
        let mb = build_multibuffer(
            &self.view.files,
            &collapsed,
            &markers,
            &review_markers,
            annotations_for_rows,
            &syntax,
        );
        self.view.rows = mb.rows;
        self.view.file_of_row = mb.file_of_row;
        self.view.header_row_of_file = mb.header_row_of_file;
        self.view.gutter_width = mb.gutter_width;
        // Overlay the imported-thread gutter markers after the build, so the
        // row builder stays overlay-free. A no-op when no threads are loaded.
        self.decorate_thread_markers();
        // Splice each thread's inline conversation (and any drafted replies)
        // in after its anchor row. Also a no-op with no threads loaded, so a
        // non-PR diff pays nothing.
        self.splice_inline_threads();
        self.view.rebuild_layout();
        self.view.selected_file = self.view.file_of_cursor();
        self.search.recompute(&self.view.rows);
    }

    /// The content sources for one file's two sides, `None` per side where
    /// that side needs no highlighting at all: a collapsed file (only its
    /// header renders), a side no row uses (the old side of a pure addition),
    /// or a synthetic file's absent old side.
    ///
    /// One definition, so the populate pass and the span lookup can't
    /// disagree about what a given file needs.
    fn highlight_sources(&self, index: usize) -> [Option<ContentSource>; 2] {
        let Some(file) = self.view.files.get(index) else {
            return [None, None];
        };
        if self.view.is_collapsed(&file.path) {
            return [None, None];
        }
        let synthetic = self.patches.get(index).is_none_or(|p| p.is_none());
        [Side::New, Side::Old].map(|side| {
            if !syntax::side_in_use(file, side) {
                return None;
            }
            syntax::source_for(
                &self.target,
                side,
                &file.path,
                file.old_path.as_deref(),
                synthetic,
            )
        })
    }
}
