//! The annotation data model: what an annotation is attached to
//! ([`Target`]), how it's classified ([`Classification`]), and the
//! annotation record itself ([`Annotation`]).

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    /// The old (removed, `-`) side of the diff.
    Old,
    /// The new (added or context) side of the diff.
    New,
}

/// Which diff source an annotation was authored against.
///
/// This is a plain, `annotate`-owned snapshot of the reviewed source — not a
/// re-export of `git::DiffTarget`. `annotate/` deliberately does not import
/// `git::DiffTarget`: that type also carries capability methods
/// (`is_live`/`staging_mode`/`supports_code_intel`) that are a `ui`-facing
/// concern annotate has no business depending on, and its `Range`/`Commit`
/// payloads are raw git rev-specs the UI layer already has to interpret
/// (e.g. resolving a commit's short SHA via the commit-log read model). This
/// type carries only the sliver of information [`crate::annotate::markdown`]
/// needs to print the `Reviewing:` metadata line: the source's kind, plus an
/// already-resolved display string for the non-worktree variants. The `ui`
/// layer (which already depends on both `git` and `annotate`) is responsible
/// for deriving a `Source` from the live `DiffTarget` at annotation time.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A single line in a file.
    Line {
        /// Path of the annotated file, relative to the repo root.
        path: String,
        /// 1-based line number.
        line: u32,
        /// Which side of the diff `line` refers to.
        side: Side,
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
    },
    /// An entire hunk, anchored to its new-side line span.
    Hunk {
        /// Path of the annotated file, relative to the repo root.
        path: String,
        /// 1-based, inclusive new-side start line.
        start: u32,
        /// 1-based, inclusive new-side end line. Must be `>= start`.
        end: u32,
    },
    /// An entire file.
    File {
        /// Path of the annotated file, relative to the repo root.
        path: String,
    },
}

impl Target {
    /// Builds a [`Target::Line`].
    pub fn line(path: impl Into<String>, line: u32, side: Side) -> Target {
        Target::Line {
            path: path.into(),
            line,
            side,
        }
    }

    /// Builds a [`Target::Range`], validating `start <= end`.
    pub fn range(
        path: impl Into<String>,
        start: u32,
        end: u32,
        side: Side,
    ) -> Result<Target, AnnotateError> {
        if start > end {
            return Err(AnnotateError::InvalidRange { start, end });
        }
        Ok(Target::Range {
            path: path.into(),
            start,
            end,
            side,
        })
    }

    /// Builds a [`Target::Hunk`], validating `start <= end`.
    pub fn hunk(path: impl Into<String>, start: u32, end: u32) -> Result<Target, AnnotateError> {
        if start > end {
            return Err(AnnotateError::InvalidRange { start, end });
        }
        Ok(Target::Hunk {
            path: path.into(),
            start,
            end,
        })
    }

    /// Builds a [`Target::File`].
    pub fn file(path: impl Into<String>) -> Target {
        Target::File { path: path.into() }
    }

    /// The path this target is attached to, regardless of variant.
    pub fn path(&self) -> &str {
        match self {
            Target::Line { path, .. }
            | Target::Range { path, .. }
            | Target::Hunk { path, .. }
            | Target::File { path } => path,
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
