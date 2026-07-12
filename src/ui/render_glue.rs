//! Row and highlight assembly: [`super::App`]'s side of the seam between the
//! diff model and the rendered multibuffer. `rebuild_rows` lazily populates
//! the syntax-highlight cache and concatenates every file's rows into the one
//! buffer [`super::DiffViewState`] renders; `refresh_rows` is the lighter
//! post-annotation-mutation rebuild. Kept out of `app.rs` so the coordinator
//! stays thin; these are the shared mutation points many gestures funnel
//! through, so their signatures are unchanged.

use crate::annotate::Side;

use super::App;
use super::rows::{StagedMarker, SyntaxSpans, build_multibuffer};
use super::stage_ops::StagedState;
use super::syntax;

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

    /// Rebuilds the whole multi-file row buffer: lazily populates the
    /// syntax-highlight cache for the in-use side(s) of every *expanded*
    /// file (collapsed files show only a header, so they are never
    /// highlighted until expanded — a cache miss is highlighted at most once
    /// per `(path, side)` between refreshes), then concatenates every file's
    /// rows into one buffer via [`build_multibuffer`], carrying per-file
    /// collapse state and staged markers. Also recomputes the active
    /// search's matches and re-derives `selected_file` from the cursor. This
    /// is `App`'s side of the seam: highlighting and the git backend live
    /// here, and the built buffer is fed into [`super::DiffViewState`].
    pub(super) fn rebuild_rows(&mut self) {
        // Per-file metadata, collected first (cloning paths) so the cache /
        // highlighter can be mutably borrowed without also holding `files`.
        struct Meta {
            path: String,
            old_path: Option<String>,
            collapsed: bool,
            needs_new: bool,
            needs_old: bool,
            synthetic: bool,
        }
        let metas: Vec<Meta> = self
            .view
            .files
            .iter()
            .enumerate()
            .map(|(i, file)| {
                let collapsed = self.view.is_collapsed(&file.path);
                Meta {
                    path: file.path.clone(),
                    old_path: file.old_path.clone(),
                    collapsed,
                    needs_new: !collapsed && syntax::side_in_use(file, Side::New),
                    needs_old: !collapsed && syntax::side_in_use(file, Side::Old),
                    synthetic: self.patches.get(i).is_none_or(|p| p.is_none()),
                }
            })
            .collect();

        for meta in &metas {
            if meta.needs_new {
                syntax::populate_cache(
                    &mut self.highlight_cache,
                    &mut self.highlighter,
                    self.stage_ops.as_deref(),
                    &self.target,
                    &meta.path,
                    meta.old_path.as_deref(),
                    Side::New,
                    meta.synthetic,
                );
            }
            if meta.needs_old {
                syntax::populate_cache(
                    &mut self.highlight_cache,
                    &mut self.highlighter,
                    self.stage_ops.as_deref(),
                    &self.target,
                    &meta.path,
                    meta.old_path.as_deref(),
                    Side::Old,
                    meta.synthetic,
                );
            }
        }

        let collapsed: Vec<bool> = metas.iter().map(|m| m.collapsed).collect();
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
        let syntax: Vec<SyntaxSpans> = self
            .view
            .files
            .iter()
            .map(|f| SyntaxSpans {
                new: self.highlight_cache.get(&f.path, Side::New),
                old: self.highlight_cache.get(&f.path, Side::Old),
            })
            .collect();

        let mb = build_multibuffer(
            &self.view.files,
            &collapsed,
            &markers,
            &self.annotations,
            &syntax,
        );
        self.view.rows = mb.rows;
        self.view.file_of_row = mb.file_of_row;
        self.view.header_row_of_file = mb.header_row_of_file;
        self.view.selected_file = self.view.file_of_cursor();
        self.search.recompute(&self.view.rows);
    }
}
