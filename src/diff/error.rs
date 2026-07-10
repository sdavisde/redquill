//! Error types for the diff module.

use thiserror::Error;

/// Errors produced while parsing diff hunks.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DiffParseError {
    /// A `@@ ... @@` hunk header did not match the expected format.
    #[error("malformed hunk header: {0}")]
    MalformedHeader(String),
}
