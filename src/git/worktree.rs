//! Model and parser for `git worktree list --porcelain`.
//!
//! Pure text-in / structs-out, mirroring `stash.rs` and `branch.rs`:
//! [`parse_worktree_list`] takes the raw porcelain payload and returns typed
//! [`WorktreeEntry`] records, in the order git lists them (the main worktree
//! first).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use super::error::GitError;

/// One worktree, parsed from a blank-line-separated block of
/// `git worktree list --porcelain` output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeEntry {
    /// Absolute path to the worktree's root.
    pub path: PathBuf,
    /// The checked-out commit's oid. `None` for a `bare` entry, which has no
    /// `HEAD` line.
    pub head: Option<String>,
    /// The short branch name (`refs/heads/` stripped), if one is checked
    /// out here. `None` when `detached` or `bare`.
    pub branch: Option<String>,
    /// Whether this is the repository's bare entry.
    pub bare: bool,
    /// Whether `HEAD` is detached in this worktree.
    pub detached: bool,
    /// `Some(reason)` when locked; `reason` is `""` when git gave none.
    pub locked: Option<String>,
    /// `Some(reason)` when prunable; `reason` is `""` when git gave none.
    pub prunable: Option<String>,
}

impl WorktreeEntry {
    fn new(path: PathBuf) -> Self {
        WorktreeEntry {
            path,
            head: None,
            branch: None,
            bare: false,
            detached: false,
            locked: None,
            prunable: None,
        }
    }
}

/// Parses `git worktree list --porcelain` output into [`WorktreeEntry`]
/// records, in git's own order (the main worktree first).
///
/// Each entry is a block of attribute lines starting with a `worktree
/// <path>` line, separated from the next entry by a blank line; a
/// `worktree` line always starts a new entry, so an unterminated trailing
/// block is flushed at EOF too. Any attribute line seen before the first
/// `worktree` line is a parse error. Unknown attributes are ignored, for
/// forward compatibility with future git versions (mirroring
/// `parse_branch_headers`).
pub fn parse_worktree_list(input: &str) -> Result<Vec<WorktreeEntry>, GitError> {
    let mut out = Vec::new();
    let mut current: Option<WorktreeEntry> = None;

    for line in input.lines() {
        if line.is_empty() {
            if let Some(entry) = current.take() {
                out.push(entry);
            }
            continue;
        }

        if let Some(path) = line.strip_prefix("worktree ") {
            if let Some(entry) = current.take() {
                out.push(entry);
            }
            current = Some(WorktreeEntry::new(PathBuf::from(path)));
            continue;
        }

        let entry = current.as_mut().ok_or_else(|| {
            GitError::Parse(format!(
                "worktree attribute before `worktree` line: {line:?}"
            ))
        })?;

        if let Some(oid) = line.strip_prefix("HEAD ") {
            entry.head = Some(oid.to_string());
        } else if let Some(branch_ref) = line.strip_prefix("branch ") {
            let name = branch_ref.strip_prefix("refs/heads/").unwrap_or(branch_ref);
            entry.branch = Some(name.to_string());
        } else if line == "bare" {
            entry.bare = true;
        } else if line == "detached" {
            entry.detached = true;
        } else if let Some(rest) = line.strip_prefix("locked") {
            entry.locked = Some(rest.strip_prefix(' ').unwrap_or(rest).to_string());
        } else if let Some(rest) = line.strip_prefix("prunable") {
            entry.prunable = Some(rest.strip_prefix(' ').unwrap_or(rest).to_string());
        }
        // Any other attribute (future additions) is not relevant here and
        // is ignored.
    }

    if let Some(entry) = current.take() {
        out.push(entry);
    }

    Ok(out)
}

/// Maps a branch name to a filesystem-safe worktree directory name (spec 08
/// Unit 1): every character outside `[A-Za-z0-9._-]` becomes `-`, then a
/// short (8 hex digit) hash of the *original* branch name is appended.
///
/// The hash suffix is what makes this collision-safe: `feat/x` and
/// `feat-x` both sanitize their body to `feat-x`, but hash to different
/// suffixes (they're different original strings), so they land in distinct
/// directories rather than one silently clobbering the other's worktree.
/// [`DefaultHasher::new`] is a fixed (not per-process-randomized) starting
/// state, so this is deterministic across runs — required for "reuse the
/// existing worktree on relaunch" to find the same path every time.
pub fn sanitize_branch_dir_name(branch: &str) -> String {
    let sanitized: String = branch
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();

    let mut hasher = DefaultHasher::new();
    branch.hash(&mut hasher);
    let short_hash = hasher.finish() as u32;

    format!("{sanitized}-{short_hash:08x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_output_yields_no_entries() {
        assert!(parse_worktree_list("").unwrap().is_empty());
    }

    #[test]
    fn parses_single_worktree_with_branch() {
        let input = concat!(
            "worktree /repo\n",
            "HEAD 85d7cc5dd1cf49f6abe1c81439fbb5deae4124ab\n",
            "branch refs/heads/main\n",
        );
        let entries = parse_worktree_list(input).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, PathBuf::from("/repo"));
        assert_eq!(
            entries[0].head.as_deref(),
            Some("85d7cc5dd1cf49f6abe1c81439fbb5deae4124ab")
        );
        assert_eq!(entries[0].branch.as_deref(), Some("main"));
        assert!(!entries[0].bare);
        assert!(!entries[0].detached);
        assert_eq!(entries[0].locked, None);
        assert_eq!(entries[0].prunable, None);
    }

    #[test]
    fn parses_detached_worktree() {
        let input = concat!(
            "worktree /repo/wt\n",
            "HEAD 85d7cc5dd1cf49f6abe1c81439fbb5deae4124ab\n",
            "detached\n",
        );
        let entries = parse_worktree_list(input).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].detached);
        assert_eq!(entries[0].branch, None);
    }

    #[test]
    fn parses_bare_entry() {
        let input = concat!("worktree /repo\n", "bare\n");
        let entries = parse_worktree_list(input).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].bare);
        assert_eq!(entries[0].head, None);
        assert_eq!(entries[0].branch, None);
    }

    #[test]
    fn parses_locked_with_and_without_reason() {
        let input = concat!(
            "worktree /repo/wt-a\n",
            "HEAD 85d7cc5dd1cf49f6abe1c81439fbb5deae4124ab\n",
            "detached\n",
            "locked\n",
            "\n",
            "worktree /repo/wt-b\n",
            "HEAD 85d7cc5dd1cf49f6abe1c81439fbb5deae4124ab\n",
            "detached\n",
            "locked gone away\n",
        );
        let entries = parse_worktree_list(input).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].locked.as_deref(), Some(""));
        assert_eq!(entries[1].locked.as_deref(), Some("gone away"));
    }

    #[test]
    fn parses_prunable_with_reason() {
        let input = concat!(
            "worktree /repo/wt\n",
            "HEAD 85d7cc5dd1cf49f6abe1c81439fbb5deae4124ab\n",
            "detached\n",
            "prunable gitdir file points to non-existent location\n",
        );
        let entries = parse_worktree_list(input).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].prunable.as_deref(),
            Some("gitdir file points to non-existent location")
        );
    }

    #[test]
    fn parses_multiple_entries_in_order() {
        let input = concat!(
            "worktree /repo\n",
            "HEAD 85d7cc5dd1cf49f6abe1c81439fbb5deae4124ab\n",
            "branch refs/heads/main\n",
            "\n",
            "worktree /repo/.worktrees/feature\n",
            "HEAD 4a6bb10ac4f9e3a33041b7f2db360b0a296f3d9c\n",
            "branch refs/heads/feature\n",
        );
        let entries = parse_worktree_list(input).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, PathBuf::from("/repo"));
        assert_eq!(entries[0].branch.as_deref(), Some("main"));
        assert_eq!(entries[1].path, PathBuf::from("/repo/.worktrees/feature"));
        assert_eq!(entries[1].branch.as_deref(), Some("feature"));
    }

    #[test]
    fn attribute_before_worktree_line_errors() {
        let input = "HEAD 85d7cc5dd1cf49f6abe1c81439fbb5deae4124ab\n";
        assert!(matches!(
            parse_worktree_list(input),
            Err(GitError::Parse(_))
        ));
    }

    #[test]
    fn strips_refs_heads_prefix_from_branch() {
        let input = concat!(
            "worktree /repo\n",
            "HEAD 85d7cc5dd1cf49f6abe1c81439fbb5deae4124ab\n",
            "branch refs/heads/feature/nested\n",
        );
        let entries = parse_worktree_list(input).unwrap();
        assert_eq!(entries[0].branch.as_deref(), Some("feature/nested"));
    }

    // -- sanitize_branch_dir_name -------------------------------------------

    #[test]
    fn keeps_simple_names_unchanged_up_to_the_hash_suffix() {
        let name = sanitize_branch_dir_name("feature");
        assert!(name.starts_with("feature-"));
        // 8 hex digits after the separating dash.
        assert_eq!(name.len(), "feature-".len() + 8);
    }

    #[test]
    fn replaces_characters_outside_the_allowed_set() {
        let name = sanitize_branch_dir_name("feat/awesome thing!");
        assert!(name.starts_with("feat-awesome-thing--"));
    }

    #[test]
    fn keeps_dots_underscores_and_hyphens_as_is() {
        let name = sanitize_branch_dir_name("release/v1.2.3_rc-1");
        assert!(name.starts_with("release-v1.2.3_rc-1-"));
    }

    #[test]
    fn is_deterministic_across_calls() {
        assert_eq!(
            sanitize_branch_dir_name("feature/x"),
            sanitize_branch_dir_name("feature/x")
        );
    }

    #[test]
    fn colliding_sanitized_bodies_still_get_distinct_directories() {
        // Both sanitize their body to "feat-x", but are different original
        // branch names, so the hash suffix must disambiguate them.
        let a = sanitize_branch_dir_name("feat/x");
        let b = sanitize_branch_dir_name("feat-x");
        assert_ne!(a, b);
    }
}
