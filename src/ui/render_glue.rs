//! Row and highlight assembly: [`super::App`]'s side of the seam between the
//! diff model and the rendered multibuffer. `rebuild_rows` populates the
//! syntax-highlight cache for what is on screen and concatenates every file's
//! rows into the one buffer [`super::DiffViewState`] renders;
//! `ensure_visible_highlights` extends that to whatever scrolls in afterwards;
//! `refresh_rows` is the lighter post-annotation-mutation rebuild. Kept out of
//! `app.rs` so the coordinator stays thin; these are the shared mutation
//! points many gestures funnel through, so their signatures are unchanged.

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
    /// cache for the in-use side(s) of every *expanded, on-screen* file, then
    /// concatenates every file's rows into one buffer via
    /// [`build_multibuffer`], carrying per-file collapse state and staged
    /// markers. Also recomputes the active search's matches and re-derives
    /// `selected_file` from the cursor. This is `App`'s side of the seam:
    /// highlighting and the git backend live here, and the built buffer is fed
    /// into [`super::DiffViewState`].
    ///
    /// Highlighting is scoped to the viewport, not the review: a collapsed
    /// file shows only a header and is never highlighted until expanded, and
    /// an off-screen file waits until it scrolls in (see
    /// [`App::ensure_visible_highlights`]). Each blob is highlighted at most
    /// once however many views reach it — the cache is keyed by content
    /// source, not by file — and only re-highlighted when its bytes could
    /// have changed.
    pub(super) fn rebuild_rows(&mut self) {
        // Resolved once and reused twice below: the populate pass walks the
        // on-screen slice of this, the span lookup walks all of it.
        let sources: Vec<[Option<ContentSource>; 2]> = (0..self.view.files.len())
            .map(|i| self.highlight_sources(i))
            .collect();

        // Highlight only what this frame can actually show. Everything else
        // is highlighted when it scrolls into view (see
        // `ensure_visible_highlights`) — at a couple of milliseconds a file,
        // that is cheaper than parsing a whole review up front, and it is
        // what keeps opening a 100-file commit instant.
        for index in self.visible_files() {
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
        // A file with no cached spans yet (off screen, or not reached by the
        // populate pass above) renders unhighlighted — the same degradation a
        // language with no grammar already gets — and picks up its spans on
        // the rebuild that follows it scrolling into view.
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
    /// One definition, so the populate pass and the per-frame readiness check
    /// can't disagree about what a given file needs.
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

    /// The files whose rows intersect the viewport, plus the cursor's file.
    ///
    /// Derived from the row and layout state of the *last* build — which is
    /// what is on screen right now. A build that shifts rows can leave this
    /// momentarily stale, so the cursor's file is included unconditionally
    /// (it is authoritative even when the layout isn't) and
    /// [`App::ensure_visible_highlights`] re-checks before the next draw.
    fn visible_files(&self) -> Vec<usize> {
        let mut files = vec![self.view.selected_file];
        if !self.view.file_of_row.is_empty() {
            let last_row = self.view.file_of_row.len() - 1;
            let top = self.view.logical_of_visual(self.view.scroll).min(last_row);
            let bottom = self
                .view
                .logical_of_visual(self.view.scroll + self.view.viewport_height())
                .min(last_row);
            files.extend(self.view.file_of_row[top..=bottom].iter().copied());
        }
        files.sort_unstable();
        files.dedup();
        files
    }

    /// Highlights any file that is on screen but not yet cached, rebuilding
    /// the rows so the fresh spans reach them.
    ///
    /// Scrolling changes which files are visible without going through
    /// [`App::rebuild_rows`], so this is what makes highlighting follow the
    /// viewport. Called once per frame before the draw; on the overwhelming
    /// majority of frames everything visible is already cached and this costs
    /// one set walk and no rebuild.
    pub(super) fn ensure_visible_highlights(&mut self) {
        let ready = self.visible_files().into_iter().all(|index| {
            self.highlight_sources(index)
                .iter()
                .flatten()
                .all(|source| self.highlight_cache.is_cached(source))
        });
        if !ready {
            self.rebuild_rows();
        }
    }
}
