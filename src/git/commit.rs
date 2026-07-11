//! Model and parser for `git log -1 --format=<COMMIT_SUMMARY_FORMAT>`.
//!
//! Pure text-in / structs-out, mirroring `branch.rs` and `stash.rs`:
//! [`parse_commit_summary`] takes the raw formatted output for the tip commit
//! and returns a typed [`CommitSummary`], or `None` when there is no commit to
//! summarize (an empty payload — e.g. a repository with no commits yet).

use super::error::GitError;

/// Format string passed to `git log -1 --format=`, using `%x00` as an
/// unambiguous separator between the abbreviated hash and the subject line.
/// `%h` respects the user's `core.abbrev`; `%s` is the single-line subject.
pub const COMMIT_SUMMARY_FORMAT: &str = "%h%x00%s";

/// A one-line summary of the current tip commit (`HEAD`), shown in the git
/// panel's bottom section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitSummary {
    /// The abbreviated commit hash (`%h`), e.g. `a1b2c3d`.
    pub short_hash: String,
    /// The commit subject (`%s`) — the first line of the message.
    pub subject: String,
}

/// Parses `git log -1 --format=<COMMIT_SUMMARY_FORMAT>` output into a
/// [`CommitSummary`]. An empty payload (no commits yet) yields `Ok(None)`; a
/// present-but-malformed record (missing the `%x00` separator) is an error.
pub fn parse_commit_summary(input: &str) -> Result<Option<CommitSummary>, GitError> {
    let record = input.strip_suffix('\n').unwrap_or(input);
    if record.is_empty() {
        return Ok(None);
    }
    let mut fields = record.splitn(2, '\0');
    match (fields.next(), fields.next()) {
        (Some(hash), Some(subject)) if !hash.is_empty() => Ok(Some(CommitSummary {
            short_hash: hash.to_string(),
            subject: subject.to_string(),
        })),
        _ => Err(GitError::Parse(format!(
            "malformed commit summary: {input:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_output_yields_no_commit() {
        assert_eq!(parse_commit_summary("").unwrap(), None);
        assert_eq!(parse_commit_summary("\n").unwrap(), None);
    }

    #[test]
    fn parses_hash_and_subject() {
        let summary = parse_commit_summary("a1b2c3d\0fix: parser bug\n")
            .unwrap()
            .unwrap();
        assert_eq!(summary.short_hash, "a1b2c3d");
        assert_eq!(summary.subject, "fix: parser bug");
    }

    #[test]
    fn subject_containing_a_colon_is_preserved() {
        let summary = parse_commit_summary("a1b2c3d\0feat: add async: remote ops")
            .unwrap()
            .unwrap();
        assert_eq!(summary.subject, "feat: add async: remote ops");
    }

    #[test]
    fn empty_subject_is_allowed() {
        // A commit with an empty subject line still summarizes.
        let summary = parse_commit_summary("a1b2c3d\0").unwrap().unwrap();
        assert_eq!(summary.short_hash, "a1b2c3d");
        assert_eq!(summary.subject, "");
    }

    #[test]
    fn missing_separator_errors() {
        assert!(matches!(
            parse_commit_summary("a1b2c3d fix: no separator"),
            Err(GitError::Parse(_))
        ));
    }
}
