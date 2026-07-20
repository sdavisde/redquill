//! Turns a raw [`ForgeError`] into the one line a status message or tab body
//! shows a reviewer. [`error_headline`] is the shared "first non-empty
//! stderr line, else the error's own `Display`" extraction every call site
//! that surfaces a forge failure used to duplicate locally.

use super::ForgeError;

/// The one-line diagnostic a [`ForgeError`] contributes to a stopped run: a
/// `Command` error's first non-empty stderr line (CLI stderr is often
/// multi-line; one actionable line is what the status line shows), or the
/// error's own `Display` otherwise.
pub(crate) fn error_headline(e: &ForgeError) -> String {
    match e {
        ForgeError::Command { stderr, .. } if !stderr.trim().is_empty() => stderr
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or(stderr)
            .trim()
            .to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
#[path = "diagnose_tests.rs"]
mod tests;
