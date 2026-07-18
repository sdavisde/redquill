//! Pure resolution of "which annotation is under the diff cursor?" — the
//! selection rule behind the diff view's in-place edit (`e`) and delete (`x`)
//! gestures.
//!
//! Kept free of [`super::App`] and every row-model/TUI type so the rule is
//! unit-testable in isolation: the caller reduces the cursor's row to a
//! [`CursorAnchor`] (via [`CursorAnchor::from_target`] over the same
//! cursor-derived [`Target`] annotation authoring already uses) and hands this
//! module that plus the annotations targeting the cursor's file.
//!
//! **Selection rule:** among the annotations whose target *covers* the cursor
//! (line/range/hunk span containing the cursor line, on a matching side; a
//! file-level target only on the file-header row), the one whose target starts
//! nearest at-or-above the cursor line wins; ties are broken by creation order,
//! oldest (lowest store id) first. A covering target's start is always at or
//! above the cursor, so "nearest at-or-above" is simply the greatest start.

use crate::annotate::{Annotation, Side, Target};

/// Which side of the diff a line-anchored cursor point sits on, extended with
/// the read-only file view's side-less worktree content. Kept local to this
/// module (rather than reusing [`Side`], which has no worktree case) so the
/// covering checks below can never confuse a diff line with a worktree line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AnchorSide {
    Diff(Side),
    Worktree,
}

/// The cursor's position reduced to the terms annotation targets are matched
/// against — no row-model or TUI types. Built from the cursor-derived
/// [`Target`] by [`CursorAnchor::from_target`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CursorAnchor {
    /// The cursor is on the file-header (or binary placeholder) row. Only a
    /// [`Target::File`] annotation matches here.
    FileHeader,
    /// The cursor is on a hunk-header row spanning new-side lines
    /// `[start, end]`. Only the [`Target::Hunk`] annotation on this exact span
    /// matches.
    HunkHeader { start: u32, end: u32 },
    /// The cursor is on a diff line at `line` on `side`.
    Line { line: u32, side: AnchorSide },
}

impl CursorAnchor {
    /// Reduces a cursor-derived [`Target`] (as produced by
    /// `App::target_for_cursor`) to a [`CursorAnchor`]. A cursor never yields a
    /// diff `Range` target in Normal mode, but the mapping stays total by
    /// treating any range-shaped target as a point at its start line.
    pub(super) fn from_target(target: &Target) -> CursorAnchor {
        match target {
            Target::File { .. } => CursorAnchor::FileHeader,
            Target::Hunk { start, end, .. } => CursorAnchor::HunkHeader {
                start: *start,
                end: *end,
            },
            Target::Line { line, side, .. } => CursorAnchor::Line {
                line: *line,
                side: AnchorSide::Diff(*side),
            },
            Target::Range { start, side, .. } => CursorAnchor::Line {
                line: *start,
                side: AnchorSide::Diff(*side),
            },
            Target::WorktreeLine { line, .. } => CursorAnchor::Line {
                line: *line,
                side: AnchorSide::Worktree,
            },
            Target::WorktreeRange { start, .. } => CursorAnchor::Line {
                line: *start,
                side: AnchorSide::Worktree,
            },
        }
    }
}

/// Whether `target` covers `anchor`, and if so the target's start line for the
/// nearest-at-or-above tie-break. `None` when the target does not cover the
/// cursor at all.
fn covering_start(anchor: &CursorAnchor, target: &Target) -> Option<u32> {
    match (anchor, target) {
        (CursorAnchor::FileHeader, Target::File { .. }) => Some(0),
        (
            CursorAnchor::HunkHeader { start, end },
            Target::Hunk {
                start: hs, end: he, ..
            },
        ) => (hs == start && he == end).then_some(*hs),
        (CursorAnchor::Line { line, side }, target) => line_covering_start(*line, *side, target),
        _ => None,
    }
}

/// The covering-start for a line anchor: a diff line matches diff-side
/// targets, a worktree line matches worktree targets; a hunk target counts as
/// a new-side span covering the lines within it.
fn line_covering_start(line: u32, side: AnchorSide, target: &Target) -> Option<u32> {
    match (side, target) {
        (
            AnchorSide::Diff(s),
            Target::Line {
                line: l, side: ts, ..
            },
        ) => (*ts == s && *l == line).then_some(*l),
        (
            AnchorSide::Diff(s),
            Target::Range {
                start,
                end,
                side: ts,
                ..
            },
        ) => (*ts == s && *start <= line && line <= *end).then_some(*start),
        (AnchorSide::Diff(Side::New), Target::Hunk { start, end, .. }) => {
            (*start <= line && line <= *end).then_some(*start)
        }
        (AnchorSide::Worktree, Target::WorktreeLine { line: l, .. }) => (*l == line).then_some(*l),
        (AnchorSide::Worktree, Target::WorktreeRange { start, end, .. }) => {
            (*start <= line && line <= *end).then_some(*start)
        }
        _ => None,
    }
}

/// The id of the annotation under `anchor`, or `None` when nothing covers the
/// cursor. `annotations` are the store's annotations targeting the cursor's
/// file (any order); the rule keys off each annotation's own id, so the
/// iteration order does not affect the result.
///
/// Among covering annotations the one whose target starts nearest at-or-above
/// the cursor wins (greatest start); ties break to the oldest (lowest id).
pub(super) fn overlapping_annotation<'a>(
    anchor: &CursorAnchor,
    annotations: impl IntoIterator<Item = &'a Annotation>,
) -> Option<usize> {
    annotations
        .into_iter()
        .filter_map(|a| covering_start(anchor, &a.target).map(|start| (start, a.id)))
        // Greatest start wins; on an equal start the lower id (older) wins, so
        // negate the id in the max key.
        .max_by_key(|&(start, id)| (start, std::cmp::Reverse(id)))
        .map(|(_, id)| id)
}

#[cfg(test)]
#[path = "annotation_overlap_tests.rs"]
mod tests;
