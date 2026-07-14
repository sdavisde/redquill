//! The commit-log read model: lists a branch's history for the git panel's
//! History tab, newest first.
//!
//! Pure text-in / structs-out, mirroring `branch.rs` and `commit.rs`:
//! [`parse_commit_log`] takes the raw `git log --format=<COMMIT_LOG_FORMAT>`
//! payload (one NUL-delimited record per line — never scraped from human
//! -readable `git log` output) and returns typed [`CommitLogEntry`] records.
//! Pagination (page size / offset) is a parameter of the git invocation
//! itself (see [`super::runner::GitRunner::commit_log`]), not of this parser.

use super::error::GitError;

/// Format string passed to `git log --format=`: full hash, abbreviated hash,
/// single-line subject, author name, and author-date Unix timestamp,
/// `%x00`-separated. `%s` is already single-line (git strips the subject to
/// its first line), so a record never spans more than one line, and NUL
/// never appears in git text output, so splitting fields on `\0` is
/// unambiguous even for a hostile subject containing `:`, quotes, or
/// internal whitespace.
pub const COMMIT_LOG_FORMAT: &str = "%H%x00%h%x00%s%x00%an%x00%at";

/// One commit in the log, parsed from a single `git log
/// --format=<COMMIT_LOG_FORMAT>` record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitLogEntry {
    /// The full commit hash (`%H`).
    pub sha: String,
    /// The abbreviated commit hash (`%h`), respecting the user's
    /// `core.abbrev`.
    pub short_sha: String,
    /// The commit subject (`%s`) — the first line of the message, verbatim
    /// (no truncation; presentation-side truncation is the UI layer's job).
    pub subject: String,
    /// The author's display name (`%an`).
    pub author_name: String,
    /// The author date as a Unix timestamp (`%at`), for relative/absolute
    /// time formatting downstream.
    pub timestamp: i64,
}

/// Parses `git log --format=<COMMIT_LOG_FORMAT>` output into
/// [`CommitLogEntry`] records, one per line, in the order git emitted them
/// (newest first, when the invocation used no `--reverse`). Empty input (an
/// empty repository with no commits, or a page past the end of history)
/// yields an empty list — not an error.
pub fn parse_commit_log(input: &str) -> Result<Vec<CommitLogEntry>, GitError> {
    let mut out = Vec::new();
    for line in input.lines() {
        if line.is_empty() {
            continue;
        }
        let mut fields = line.splitn(5, '\0');
        let (Some(sha), Some(short_sha), Some(subject), Some(author_name), Some(timestamp)) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) else {
            return Err(GitError::Parse(format!(
                "malformed commit log line: {line:?}"
            )));
        };
        let timestamp = timestamp
            .parse::<i64>()
            .map_err(|_| GitError::Parse(format!("invalid commit timestamp: {timestamp:?}")))?;
        out.push(CommitLogEntry {
            sha: sha.to_string(),
            short_sha: short_sha.to_string(),
            subject: subject.to_string(),
            author_name: author_name.to_string(),
            timestamp,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_single_record() {
        let input = "abc123full\0abc123\0fix: parser bug\0Jane Dev\x001700000000\n";
        let entries = parse_commit_log(input).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].sha, "abc123full");
        assert_eq!(entries[0].short_sha, "abc123");
        assert_eq!(entries[0].subject, "fix: parser bug");
        assert_eq!(entries[0].author_name, "Jane Dev");
        assert_eq!(entries[0].timestamp, 1700000000);
    }

    #[test]
    fn parses_multiple_records_preserving_order() {
        let input = concat!(
            "sha1full\0sha1\0first commit\0Ann\x001700000100\n",
            "sha0full\0sha0\0second commit\0Bob\x001700000000\n",
        );
        let entries = parse_commit_log(input).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].subject, "first commit");
        assert_eq!(entries[1].subject, "second commit");
    }

    #[test]
    fn subject_containing_a_colon_is_preserved() {
        let input = "sha\0sha\0feat: add async: remote ops\0A\x001700000000\n";
        let entries = parse_commit_log(input).unwrap();
        assert_eq!(entries[0].subject, "feat: add async: remote ops");
    }

    #[test]
    fn subject_containing_internal_whitespace_is_preserved() {
        let input = "sha\0sha\0fix   weird   spacing\0A\x001700000000\n";
        let entries = parse_commit_log(input).unwrap();
        assert_eq!(entries[0].subject, "fix   weird   spacing");
    }

    #[test]
    fn subject_containing_quote_characters_is_preserved() {
        let input = "sha\0sha\0fix: handle \"quoted\" and 'single' text\0A\x001700000000\n";
        let entries = parse_commit_log(input).unwrap();
        assert_eq!(
            entries[0].subject,
            "fix: handle \"quoted\" and 'single' text"
        );
    }

    #[test]
    fn empty_repo_output_yields_no_entries() {
        assert!(parse_commit_log("").unwrap().is_empty());
    }

    #[test]
    fn missing_fields_errors() {
        let input = "sha\0sha\0only three fields\0author\n";
        assert!(matches!(parse_commit_log(input), Err(GitError::Parse(_))));
    }

    #[test]
    fn invalid_timestamp_errors() {
        let input = "sha\0sha\0subject\0author\0not-a-number\n";
        assert!(matches!(parse_commit_log(input), Err(GitError::Parse(_))));
    }
}
