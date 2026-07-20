//! The annotation data model: what an annotation is attached to
//! ([`Target`]), how it's classified ([`Classification`]), and the
//! annotation record itself ([`Annotation`]).
//!
//! [`Classification`], [`Side`], [`Source`], and [`Target`] additionally
//! derive `Serialize`/`Deserialize` so
//! [`super::persist::PersistedAnnotation`] can compose them directly rather
//! than defining a shadow copy of each shape that could drift from the
//! domain type it mirrors. `Annotation` itself does not derive them — its
//! `id` is a store-assigned ordinal (see [`super::store::AnnotationStore`]),
//! not persistable data; [`super::persist::PersistedAnnotation`] carries
//! everything else.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors produced while constructing or mutating annotation data.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AnnotateError {
    /// An annotation body was empty (or all whitespace) after trimming.
    #[error("annotation body must not be empty")]
    EmptyBody,

    /// A [`Target::Range`] or [`Target::Hunk`] was constructed with
    /// `start > end`.
    #[error("range start ({start}) must be <= end ({end})")]
    InvalidRange {
        /// The requested start line.
        start: u32,
        /// The requested end line.
        end: u32,
    },

    /// [`AnnotationStore::edit`] or [`AnnotationStore::remove`] was called
    /// with an id that doesn't exist in the store.
    #[error("no annotation with id {0}")]
    NotFound(usize),
}

/// The reviewer's classification of an annotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Classification {
    /// Something that must change before this is acceptable.
    Issue,
    /// A clarifying question for the author.
    Question,
    /// A minor, non-blocking suggestion.
    Nit,
    /// Positive feedback.
    Praise,
}

impl Classification {
    /// The lowercase label used in markdown output (e.g. `"issue"`).
    pub fn label(self) -> &'static str {
        match self {
            Classification::Issue => "issue",
            Classification::Question => "question",
            Classification::Nit => "nit",
            Classification::Praise => "praise",
        }
    }

    /// Parses a [`Classification`] from its lowercase label. Returns `None`
    /// for anything else.
    pub fn from_label(label: &str) -> Option<Classification> {
        match label {
            "issue" => Some(Classification::Issue),
            "question" => Some(Classification::Question),
            "nit" => Some(Classification::Nit),
            "praise" => Some(Classification::Praise),
            _ => None,
        }
    }

    /// The next classification in the compose modal's `Ctrl-t` cycle:
    /// Issue -> Question -> Nit -> Praise -> Issue.
    pub fn cycle(self) -> Classification {
        match self {
            Classification::Issue => Classification::Question,
            Classification::Question => Classification::Nit,
            Classification::Nit => Classification::Praise,
            Classification::Praise => Classification::Issue,
        }
    }
}

/// Which side of the diff a line-anchored annotation refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// The old (removed, `-`) side of the diff.
    Old,
    /// The new (added or context) side of the diff.
    New,
}

/// Which diff source an annotation was authored against.
///
/// This is a plain, `annotate`-owned snapshot of the reviewed source, not a
/// re-export of `git::DiffTarget`: `annotate/` has no business depending on
/// that type's `ui`-facing capability methods, so this carries only the
/// sliver of information [`crate::annotate::markdown`] needs to print the
/// `Reviewing:` metadata line — the source's kind, plus an already-resolved
/// display string for the non-worktree variants. The `ui` layer derives a
/// `Source` from the live `DiffTarget` at annotation time.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// The working tree vs. the index — the default, and the only source
    /// that produces no `Reviewing:` metadata line.
    #[default]
    WorkingTree,
    /// The index vs. `HEAD`.
    Staged,
    /// An explicit range or ref expression, stored exactly as the user
    /// typed/selected it (e.g. `"main..feature"`).
    Range(String),
    /// A single commit, stored as its short SHA (already resolved by the
    /// caller — `annotate` never talks to git to compute one).
    Commit(String),
}

impl Source {
    /// The label printed after `Reviewing: ` for a non-worktree source.
    /// Callers should not print a metadata line at all for
    /// [`Source::WorkingTree`] — see the markdown module's grouping logic.
    pub fn label(&self) -> &str {
        match self {
            Source::WorkingTree => "working tree",
            Source::Staged => "staged",
            Source::Range(spec) => spec,
            Source::Commit(short_sha) => short_sha,
        }
    }
}

/// What an [`Annotation`] is attached to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    /// A single line in a file.
    Line {
        /// Path of the annotated file, relative to the repo root.
        path: String,
        /// 1-based line number.
        line: u32,
        /// Which side of the diff `line` refers to.
        side: Side,
        /// The same physical line's 1-based number on the *opposite* diff
        /// side, when it exists there (a context line appears on both
        /// sides; added/removed lines don't). Captured at annotation time
        /// because some forge position formats (GitLab) require both
        /// numbers for a context line. `None` for single-sided lines and
        /// for annotations persisted before this field existed
        /// (`#[serde(default)]`), which keep single-sided behavior.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        other_line: Option<u32>,
    },
    /// An inclusive span of lines in a file.
    Range {
        /// Path of the annotated file, relative to the repo root.
        path: String,
        /// 1-based, inclusive start line.
        start: u32,
        /// 1-based, inclusive end line. Must be `>= start`.
        end: u32,
        /// Which side of the diff the span refers to.
        side: Side,
        /// `end`'s 1-based number on the opposite diff side, when `end` is
        /// a context line — the only endpoint forge submission anchors on
        /// (a span collapses to its end line). Same compatibility contract
        /// as [`Target::Line`]'s `other_line`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        other_end: Option<u32>,
    },
    /// An entire hunk, anchored to its new-side line span.
    Hunk {
        /// Path of the annotated file, relative to the repo root.
        path: String,
        /// 1-based, inclusive new-side start line.
        start: u32,
        /// 1-based, inclusive new-side end line. Must be `>= start`.
        end: u32,
        /// `end`'s 1-based old-side number when the hunk's last line is a
        /// context line (the common case — hunks end on trailing context).
        /// Same compatibility contract as [`Target::Line`]'s `other_line`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        other_end: Option<u32>,
    },
    /// An entire file.
    File {
        /// Path of the annotated file, relative to the repo root.
        path: String,
    },
    /// A single line in a file's *current worktree content* — not anchored
    /// to either side of a diff (the read-only file view). Serializes with
    /// the `(=)` marker; see `docs/annotation-format.md` for the full
    /// format contract, including how this composes with `Reviewing:`
    /// grouping.
    WorktreeLine {
        /// Path of the annotated file, relative to the repo root.
        path: String,
        /// 1-based line number.
        line: u32,
    },
    /// An inclusive span of lines in a file's current worktree content —
    /// the range counterpart to [`Target::WorktreeLine`]; see its doc for
    /// how it composes with `Reviewing:` grouping.
    WorktreeRange {
        /// Path of the annotated file, relative to the repo root.
        path: String,
        /// 1-based, inclusive start line.
        start: u32,
        /// 1-based, inclusive end line. Must be `>= start`.
        end: u32,
    },
}

impl Target {
    /// Builds a [`Target::Line`] with no opposite-side counterpart.
    pub fn line(path: impl Into<String>, line: u32, side: Side) -> Target {
        Target::line_with_other(path, line, side, None)
    }

    /// Builds a [`Target::Line`], recording the line's number on the
    /// opposite diff side when it has one (a context line).
    pub fn line_with_other(
        path: impl Into<String>,
        line: u32,
        side: Side,
        other_line: Option<u32>,
    ) -> Target {
        Target::Line {
            path: path.into(),
            line,
            side,
            other_line,
        }
    }

    /// Builds a [`Target::Range`] with no opposite-side counterpart,
    /// validating `start <= end`.
    pub fn range(
        path: impl Into<String>,
        start: u32,
        end: u32,
        side: Side,
    ) -> Result<Target, AnnotateError> {
        Target::range_with_other_end(path, start, end, side, None)
    }

    /// Builds a [`Target::Range`], validating `start <= end` and recording
    /// `end`'s opposite-side number when it has one (a context line).
    pub fn range_with_other_end(
        path: impl Into<String>,
        start: u32,
        end: u32,
        side: Side,
        other_end: Option<u32>,
    ) -> Result<Target, AnnotateError> {
        if start > end {
            return Err(AnnotateError::InvalidRange { start, end });
        }
        Ok(Target::Range {
            path: path.into(),
            start,
            end,
            side,
            other_end,
        })
    }

    /// Builds a [`Target::Hunk`] with no opposite-side counterpart,
    /// validating `start <= end`.
    pub fn hunk(path: impl Into<String>, start: u32, end: u32) -> Result<Target, AnnotateError> {
        Target::hunk_with_other_end(path, start, end, None)
    }

    /// Builds a [`Target::Hunk`], validating `start <= end` and recording
    /// `end`'s old-side number when the hunk's last line is a context line.
    pub fn hunk_with_other_end(
        path: impl Into<String>,
        start: u32,
        end: u32,
        other_end: Option<u32>,
    ) -> Result<Target, AnnotateError> {
        if start > end {
            return Err(AnnotateError::InvalidRange { start, end });
        }
        Ok(Target::Hunk {
            path: path.into(),
            start,
            end,
            other_end,
        })
    }

    /// Builds a [`Target::File`].
    pub fn file(path: impl Into<String>) -> Target {
        Target::File { path: path.into() }
    }

    /// Builds a [`Target::WorktreeLine`].
    pub fn worktree_line(path: impl Into<String>, line: u32) -> Target {
        Target::WorktreeLine {
            path: path.into(),
            line,
        }
    }

    /// Builds a [`Target::WorktreeRange`], validating `start <= end`.
    pub fn worktree_range(
        path: impl Into<String>,
        start: u32,
        end: u32,
    ) -> Result<Target, AnnotateError> {
        if start > end {
            return Err(AnnotateError::InvalidRange { start, end });
        }
        Ok(Target::WorktreeRange {
            path: path.into(),
            start,
            end,
        })
    }

    /// The path this target is attached to, regardless of variant.
    pub fn path(&self) -> &str {
        match self {
            Target::Line { path, .. }
            | Target::Range { path, .. }
            | Target::Hunk { path, .. }
            | Target::File { path }
            | Target::WorktreeLine { path, .. }
            | Target::WorktreeRange { path, .. } => path,
        }
    }

    /// The anchor line to land on when navigating back to this target's
    /// location: the line itself for [`Target::WorktreeLine`], the start
    /// line for [`Target::WorktreeRange`]. `None` for every other variant —
    /// callers use `path()` position lookups plus row-model anchoring for
    /// those instead (see `App::jump_to_annotation`).
    pub fn worktree_anchor_line(&self) -> Option<u32> {
        match self {
            Target::WorktreeLine { line, .. } => Some(*line),
            Target::WorktreeRange { start, .. } => Some(*start),
            _ => None,
        }
    }
}

/// A single reviewer annotation: what it's attached to, how it's
/// classified, its body text, and a stable ordinal assigned by the store
/// that created it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Annotation {
    /// Stable id assigned by the owning [`AnnotationStore`], in insertion
    /// order.
    pub id: usize,
    /// What this annotation is attached to.
    pub target: Target,
    /// The reviewer's classification.
    pub classification: Classification,
    /// The comment body, guaranteed non-empty after trimming.
    pub body: String,
    /// The diff source this annotation was authored against. Defaults to
    /// [`Source::WorkingTree`]; see [`AnnotationStore::add_with_source`].
    pub source: Source,
    /// Whether this annotation has already been published to the forge as a
    /// review comment. `false` for a freshly authored annotation; set once
    /// the review is submitted so it is excluded from future submits and —
    /// when the forge's own copy is present in the fetched thread overlay —
    /// not re-drawn as a local annotation at the same anchor (the forge copy
    /// is authoritative on screen). Never affects the stdout markdown, which
    /// includes every annotation regardless of published state.
    pub published: bool,
    /// Whether a private GitLab draft note for this annotation already
    /// exists server-side from a stopped submit run — created but not yet
    /// bulk-published. A resubmit skips re-creating such drafts and lets
    /// its bulk publish flip them; cleared once published. Always `false`
    /// on the GitHub path, which stages no drafts.
    pub draft_created: bool,
}

/// Validates and normalizes an annotation body: trims surrounding
/// whitespace and rejects an empty result.
pub(crate) fn validate_body(body: &str) -> Result<String, AnnotateError> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Err(AnnotateError::EmptyBody);
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_label_roundtrip() {
        for c in [
            Classification::Issue,
            Classification::Question,
            Classification::Nit,
            Classification::Praise,
        ] {
            assert_eq!(Classification::from_label(c.label()), Some(c));
        }
    }

    #[test]
    fn classification_cycle_wraps_around() {
        assert_eq!(Classification::Issue.cycle(), Classification::Question);
        assert_eq!(Classification::Question.cycle(), Classification::Nit);
        assert_eq!(Classification::Nit.cycle(), Classification::Praise);
        assert_eq!(Classification::Praise.cycle(), Classification::Issue);
    }

    #[test]
    fn classification_labels_are_lowercase() {
        assert_eq!(Classification::Issue.label(), "issue");
        assert_eq!(Classification::Question.label(), "question");
        assert_eq!(Classification::Nit.label(), "nit");
        assert_eq!(Classification::Praise.label(), "praise");
    }

    #[test]
    fn classification_from_unknown_label_is_none() {
        assert_eq!(Classification::from_label("blocker"), None);
        assert_eq!(Classification::from_label(""), None);
        assert_eq!(Classification::from_label("Issue"), None);
    }

    #[test]
    fn range_rejects_start_after_end() {
        let err = Target::range("src/a.rs", 10, 5, Side::New).unwrap_err();
        assert_eq!(err, AnnotateError::InvalidRange { start: 10, end: 5 });
    }

    #[test]
    fn range_allows_equal_start_and_end() {
        let target = Target::range("src/a.rs", 5, 5, Side::New).unwrap();
        assert_eq!(
            target,
            Target::Range {
                path: "src/a.rs".to_string(),
                start: 5,
                end: 5,
                side: Side::New,
                other_end: None,
            }
        );
    }

    #[test]
    fn hunk_rejects_start_after_end() {
        let err = Target::hunk("src/a.rs", 10, 5).unwrap_err();
        assert_eq!(err, AnnotateError::InvalidRange { start: 10, end: 5 });
    }

    #[test]
    fn target_path_returns_inner_path_for_every_variant() {
        assert_eq!(Target::line("a.rs", 1, Side::New).path(), "a.rs");
        assert_eq!(
            Target::range("a.rs", 1, 2, Side::New).unwrap().path(),
            "a.rs"
        );
        assert_eq!(Target::hunk("a.rs", 1, 2).unwrap().path(), "a.rs");
        assert_eq!(Target::file("a.rs").path(), "a.rs");
    }

    #[test]
    fn validate_body_trims_whitespace() {
        assert_eq!(validate_body("  hello  ").unwrap(), "hello");
    }

    #[test]
    fn validate_body_rejects_empty_and_whitespace_only() {
        assert_eq!(validate_body(""), Err(AnnotateError::EmptyBody));
        assert_eq!(validate_body("   \n\t "), Err(AnnotateError::EmptyBody));
    }

    // -- Target::WorktreeLine / Target::WorktreeRange -----------------------

    #[test]
    fn worktree_line_builds_expected_variant() {
        assert_eq!(
            Target::worktree_line("docs/notes.md", 44),
            Target::WorktreeLine {
                path: "docs/notes.md".to_string(),
                line: 44,
            }
        );
    }

    #[test]
    fn worktree_range_builds_expected_variant() {
        assert_eq!(
            Target::worktree_range("docs/notes.md", 10, 20).unwrap(),
            Target::WorktreeRange {
                path: "docs/notes.md".to_string(),
                start: 10,
                end: 20,
            }
        );
    }

    #[test]
    fn worktree_range_rejects_start_after_end() {
        let err = Target::worktree_range("docs/notes.md", 10, 5).unwrap_err();
        assert_eq!(err, AnnotateError::InvalidRange { start: 10, end: 5 });
    }

    #[test]
    fn worktree_range_allows_equal_start_and_end() {
        assert!(Target::worktree_range("docs/notes.md", 5, 5).is_ok());
    }

    #[test]
    fn worktree_targets_report_their_path() {
        assert_eq!(
            Target::worktree_line("docs/notes.md", 1).path(),
            "docs/notes.md"
        );
        assert_eq!(
            Target::worktree_range("docs/notes.md", 1, 2)
                .unwrap()
                .path(),
            "docs/notes.md"
        );
    }

    #[test]
    fn worktree_anchor_line_resolves_line_and_range_start_only() {
        assert_eq!(
            Target::worktree_line("a.rs", 44).worktree_anchor_line(),
            Some(44)
        );
        assert_eq!(
            Target::worktree_range("a.rs", 10, 20)
                .unwrap()
                .worktree_anchor_line(),
            Some(10)
        );
        assert_eq!(Target::file("a.rs").worktree_anchor_line(), None);
        assert_eq!(
            Target::line("a.rs", 1, Side::New).worktree_anchor_line(),
            None
        );
        assert_eq!(
            Target::range("a.rs", 1, 2, Side::New)
                .unwrap()
                .worktree_anchor_line(),
            None
        );
        assert_eq!(
            Target::hunk("a.rs", 1, 2).unwrap().worktree_anchor_line(),
            None
        );
    }

    // -- opposite-side counterpart capture (context lines) -------------------

    #[test]
    fn line_with_other_records_the_counterpart_and_plain_line_leaves_it_absent() {
        assert_eq!(
            Target::line_with_other("a.rs", 8, Side::New, Some(6)),
            Target::Line {
                path: "a.rs".to_string(),
                line: 8,
                side: Side::New,
                other_line: Some(6),
            }
        );
        assert_eq!(
            Target::line("a.rs", 8, Side::New),
            Target::line_with_other("a.rs", 8, Side::New, None)
        );
    }

    #[test]
    fn range_and_hunk_with_other_end_record_the_counterpart_and_still_validate() {
        assert_eq!(
            Target::range_with_other_end("a.rs", 1, 3, Side::New, Some(2)).unwrap(),
            Target::Range {
                path: "a.rs".to_string(),
                start: 1,
                end: 3,
                side: Side::New,
                other_end: Some(2),
            }
        );
        assert_eq!(
            Target::hunk_with_other_end("a.rs", 1, 3, Some(2)).unwrap(),
            Target::Hunk {
                path: "a.rs".to_string(),
                start: 1,
                end: 3,
                other_end: Some(2),
            }
        );
        assert!(Target::range_with_other_end("a.rs", 5, 2, Side::New, Some(1)).is_err());
        assert!(Target::hunk_with_other_end("a.rs", 5, 2, Some(1)).is_err());
    }

    #[test]
    fn targets_persisted_before_the_counterpart_field_still_deserialize() {
        // The exact JSON shapes written before `other_line`/`other_end`
        // existed must keep loading, with the counterpart absent.
        let line: Target =
            serde_json::from_str(r#"{"kind":"line","path":"a.rs","line":3,"side":"new"}"#).unwrap();
        assert_eq!(line, Target::line("a.rs", 3, Side::New));
        let range: Target = serde_json::from_str(
            r#"{"kind":"range","path":"a.rs","start":1,"end":4,"side":"old"}"#,
        )
        .unwrap();
        assert_eq!(range, Target::range("a.rs", 1, 4, Side::Old).unwrap());
        let hunk: Target =
            serde_json::from_str(r#"{"kind":"hunk","path":"a.rs","start":1,"end":4}"#).unwrap();
        assert_eq!(hunk, Target::hunk("a.rs", 1, 4).unwrap());
    }

    #[test]
    fn counterpart_free_targets_serialize_without_the_new_keys() {
        let json = serde_json::to_string(&Target::line("a.rs", 3, Side::New)).unwrap();
        assert!(!json.contains("other_line"), "unexpected key: {json}");
        let json = serde_json::to_string(&Target::range("a.rs", 1, 4, Side::New).unwrap()).unwrap();
        assert!(!json.contains("other_end"), "unexpected key: {json}");
    }

    #[test]
    fn counterpart_carrying_targets_round_trip_through_serde() {
        for target in [
            Target::line_with_other("a.rs", 8, Side::New, Some(6)),
            Target::range_with_other_end("a.rs", 1, 8, Side::New, Some(6)).unwrap(),
            Target::hunk_with_other_end("a.rs", 1, 8, Some(6)).unwrap(),
        ] {
            let json = serde_json::to_string(&target).unwrap();
            let back: Target = serde_json::from_str(&json).unwrap();
            assert_eq!(back, target, "round trip changed the target: {json}");
        }
    }

    #[test]
    fn source_default_is_working_tree() {
        assert_eq!(Source::default(), Source::WorkingTree);
    }

    #[test]
    fn source_label_matches_the_format_contract() {
        assert_eq!(Source::WorkingTree.label(), "working tree");
        assert_eq!(Source::Staged.label(), "staged");
        assert_eq!(
            Source::Range("main..feature".to_string()).label(),
            "main..feature"
        );
        assert_eq!(Source::Commit("abc1234".to_string()).label(), "abc1234");
    }
}
