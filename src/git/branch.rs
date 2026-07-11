//! Model and parser for the `# branch.*` headers of
//! `git status --porcelain=v2 --branch -z`.
//!
//! Pure text-in / structs-out, mirroring `status.rs`: [`parse_branch_headers`]
//! reads the same NUL-separated payload the status parser consumes, but only
//! looks at the `# branch.*` header fields, ignoring record lines.

use super::error::GitError;

/// Number of hex characters used for the short oid shown in place of a
/// branch name when `HEAD` is detached.
const SHORT_OID_LEN: usize = 7;

/// Current branch / sync state, parsed from porcelain-v2 branch headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchStatus {
    /// The branch name, or a short commit id when `HEAD` is detached.
    pub name: String,
    /// Whether `HEAD` is detached (in which case `name` is a short oid).
    pub detached: bool,
    /// The upstream ref (e.g. `origin/main`), if one is configured.
    pub upstream: Option<String>,
    /// `(ahead, behind)` commit counts vs. the upstream, if one is configured.
    pub ahead_behind: Option<(u32, u32)>,
}

/// Parses the `# branch.*` headers out of a `git status --porcelain=v2
/// --branch -z` payload into a [`BranchStatus`].
///
/// Non-header fields (ordinary/rename/unmerged/untracked/ignored records)
/// are ignored, so this can be called on the same raw payload passed to
/// [`super::status::parse_porcelain_v2`].
pub fn parse_branch_headers(input: &str) -> Result<BranchStatus, GitError> {
    let mut oid: Option<&str> = None;
    let mut head: Option<&str> = None;
    let mut upstream: Option<&str> = None;
    let mut ahead_behind: Option<(u32, u32)> = None;

    for field in input.split('\0').filter(|s| !s.is_empty()) {
        if let Some(rest) = field.strip_prefix("# branch.oid ") {
            oid = Some(rest);
        } else if let Some(rest) = field.strip_prefix("# branch.head ") {
            head = Some(rest);
        } else if let Some(rest) = field.strip_prefix("# branch.upstream ") {
            upstream = Some(rest);
        } else if let Some(rest) = field.strip_prefix("# branch.ab ") {
            ahead_behind = Some(parse_ab(rest)?);
        }
        // Any other header (e.g. future additions) or record line is not
        // relevant to branch state and is ignored here.
    }

    let head = head.ok_or_else(|| GitError::Parse("missing branch.head header".into()))?;
    let (name, detached) = if head == "(detached)" {
        let oid =
            oid.ok_or_else(|| GitError::Parse("detached HEAD missing branch.oid header".into()))?;
        let short = oid.get(..SHORT_OID_LEN).unwrap_or(oid);
        (short.to_string(), true)
    } else {
        (head.to_string(), false)
    };

    Ok(BranchStatus {
        name,
        detached,
        upstream: upstream.map(|s| s.to_string()),
        ahead_behind,
    })
}

/// Parses a `branch.ab` value of the form `+N -M`.
fn parse_ab(rest: &str) -> Result<(u32, u32), GitError> {
    let mut parts = rest.split_whitespace();
    let (Some(ahead_field), Some(behind_field)) = (parts.next(), parts.next()) else {
        return Err(GitError::Parse(format!("malformed branch.ab: {rest:?}")));
    };
    let ahead = ahead_field
        .strip_prefix('+')
        .ok_or_else(|| GitError::Parse(format!("malformed ahead count: {ahead_field:?}")))?
        .parse::<u32>()
        .map_err(|_| GitError::Parse(format!("invalid ahead count: {ahead_field:?}")))?;
    let behind = behind_field
        .strip_prefix('-')
        .ok_or_else(|| GitError::Parse(format!("malformed behind count: {behind_field:?}")))?
        .parse::<u32>()
        .map_err(|_| GitError::Parse(format!("invalid behind count: {behind_field:?}")))?;
    Ok((ahead, behind))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_branch_with_upstream_and_ahead_behind() {
        let input = concat!(
            "# branch.oid 85d7cc5dd1cf49f6abe1c81439fbb5deae4124ab\0",
            "# branch.head main\0",
            "# branch.upstream origin/main\0",
            "# branch.ab +2 -1\0",
        );
        let status = parse_branch_headers(input).unwrap();
        assert_eq!(status.name, "main");
        assert!(!status.detached);
        assert_eq!(status.upstream.as_deref(), Some("origin/main"));
        assert_eq!(status.ahead_behind, Some((2, 1)));
    }

    #[test]
    fn detached_head_uses_short_oid_from_branch_oid() {
        let input = concat!(
            "# branch.oid 85d7cc5dd1cf49f6abe1c81439fbb5deae4124ab\0",
            "# branch.head (detached)\0",
        );
        let status = parse_branch_headers(input).unwrap();
        assert_eq!(status.name, "85d7cc5");
        assert!(status.detached);
        assert_eq!(status.upstream, None);
        assert_eq!(status.ahead_behind, None);
    }

    #[test]
    fn branch_with_no_upstream_has_no_counts() {
        let input = concat!(
            "# branch.oid 85d7cc5dd1cf49f6abe1c81439fbb5deae4124ab\0",
            "# branch.head feature\0",
        );
        let status = parse_branch_headers(input).unwrap();
        assert_eq!(status.name, "feature");
        assert!(!status.detached);
        assert_eq!(status.upstream, None);
        assert_eq!(status.ahead_behind, None);
    }

    #[test]
    fn headers_are_parsed_when_record_lines_are_interspersed() {
        let input = concat!(
            "# branch.oid 85d7cc5dd1cf49f6abe1c81439fbb5deae4124ab\0",
            "# branch.head main\0",
            "# branch.upstream origin/main\0",
            "# branch.ab +0 -0\0",
            "1 .M N... 100644 100644 100644 aaa bbb src/main.rs\0",
            "? untracked.txt\0",
        );
        let status = parse_branch_headers(input).unwrap();
        assert_eq!(status.name, "main");
        assert_eq!(status.ahead_behind, Some((0, 0)));
    }

    #[test]
    fn missing_head_header_errors() {
        let input = "# branch.oid 85d7cc5dd1cf49f6abe1c81439fbb5deae4124ab\0";
        assert!(matches!(
            parse_branch_headers(input),
            Err(GitError::Parse(_))
        ));
    }

    #[test]
    fn malformed_ab_errors() {
        let input = concat!("# branch.head main\0", "# branch.ab bogus\0",);
        assert!(matches!(
            parse_branch_headers(input),
            Err(GitError::Parse(_))
        ));
    }
}
