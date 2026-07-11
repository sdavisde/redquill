//! Model and parser for `git stash list --format=<STASH_LIST_FORMAT>`.
//!
//! Pure text-in / structs-out, mirroring `status.rs` and `branch.rs`:
//! [`parse_stash_list`] takes the raw formatted output and returns typed
//! [`StashEntry`] records.

use super::error::GitError;

/// Format string passed to `git stash list --format=`, using `%x00` as an
/// unambiguous field separator between the stash ref and its subject line.
pub const STASH_LIST_FORMAT: &str = "%gd%x00%gs";

/// A single stashed change, parsed from one line of `git stash list
/// --format=<STASH_LIST_FORMAT>` output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StashEntry {
    /// The stash ref, e.g. `stash@{0}`.
    pub stash_ref: String,
    /// The branch the stash was taken from, if git recorded one
    /// (`None` for a stash taken with a detached `HEAD`, i.e. `(no branch)`).
    pub branch: Option<String>,
    /// The stash message: either the user-supplied `-m` message, or the
    /// auto-generated `<short-oid> <commit subject>` for an unnamed stash.
    pub message: String,
}

/// Parses `git stash list --format=<STASH_LIST_FORMAT>` output into
/// [`StashEntry`] records, newest first (git's own list order).
pub fn parse_stash_list(input: &str) -> Result<Vec<StashEntry>, GitError> {
    let mut out = Vec::new();
    for line in input.lines() {
        if line.is_empty() {
            continue;
        }
        let mut fields = line.splitn(2, '\0');
        let (Some(stash_ref), Some(subject)) = (fields.next(), fields.next()) else {
            return Err(GitError::Parse(format!("malformed stash entry: {line:?}")));
        };
        let (branch, message) = parse_subject(subject)?;
        out.push(StashEntry {
            stash_ref: stash_ref.to_string(),
            branch,
            message,
        });
    }
    Ok(out)
}

/// Parses a stash subject of the form `WIP on <branch>: <rest>` (the
/// auto-generated case) or `On <branch>: <message>` (an explicit `-m`
/// message), returning the branch (`None` for `(no branch)`, i.e. a
/// detached-`HEAD` stash) and the remainder as the message.
fn parse_subject(subject: &str) -> Result<(Option<String>, String), GitError> {
    let rest = subject
        .strip_prefix("WIP on ")
        .or_else(|| subject.strip_prefix("On "))
        .ok_or_else(|| GitError::Parse(format!("unrecognized stash subject: {subject:?}")))?;

    let (branch_part, message) = rest
        .split_once(": ")
        .ok_or_else(|| GitError::Parse(format!("unrecognized stash subject: {subject:?}")))?;

    let branch = if branch_part == "(no branch)" {
        None
    } else {
        Some(branch_part.to_string())
    };
    Ok((branch, message.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_output_yields_no_entries() {
        assert!(parse_stash_list("").unwrap().is_empty());
    }

    #[test]
    fn parses_auto_generated_wip_message() {
        let input = "stash@{0}\0WIP on main: 85d7cc5 second\n";
        let entries = parse_stash_list(input).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].stash_ref, "stash@{0}");
        assert_eq!(entries[0].branch.as_deref(), Some("main"));
        assert_eq!(entries[0].message, "85d7cc5 second");
    }

    #[test]
    fn parses_explicit_message_containing_a_colon() {
        // `-m "spike: tabs"` produces `On <branch>: spike: tabs`; the
        // message itself contains a colon and must not be truncated.
        let input = "stash@{1}\0On main: spike: tabs and stuff\n";
        let entries = parse_stash_list(input).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].stash_ref, "stash@{1}");
        assert_eq!(entries[0].branch.as_deref(), Some("main"));
        assert_eq!(entries[0].message, "spike: tabs and stuff");
    }

    #[test]
    fn parses_multiple_entries_newest_first() {
        let input = concat!(
            "stash@{0}\0On main: newest\n",
            "stash@{1}\0On main: oldest\n",
        );
        let entries = parse_stash_list(input).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].message, "newest");
        assert_eq!(entries[1].message, "oldest");
    }

    #[test]
    fn detached_head_stash_has_no_branch() {
        let input = "stash@{0}\0WIP on (no branch): 85d7cc5 second\n";
        let entries = parse_stash_list(input).unwrap();
        assert_eq!(entries[0].branch, None);
        assert_eq!(entries[0].message, "85d7cc5 second");
    }

    #[test]
    fn message_with_spaces_is_preserved() {
        let input = "stash@{0}\0On feature/foo: wip mid review please hold\n";
        let entries = parse_stash_list(input).unwrap();
        assert_eq!(entries[0].branch.as_deref(), Some("feature/foo"));
        assert_eq!(entries[0].message, "wip mid review please hold");
    }

    #[test]
    fn malformed_entry_missing_separator_errors() {
        let input = "stash@{0}\0malformed subject with no colon\n";
        assert!(matches!(parse_stash_list(input), Err(GitError::Parse(_))));
    }
}
