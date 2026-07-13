//! The commit read model and the commit write command.
//!
//! The read side is pure text-in / structs-out, mirroring `branch.rs` and
//! `stash.rs`: [`parse_commit_summary`] takes the raw
//! `git log -1 --format=<COMMIT_SUMMARY_FORMAT>` output for the tip commit
//! and returns a typed [`CommitSummary`], or `None` when there is no commit
//! to summarize (an empty payload — e.g. a repository with no commits yet).
//!
//! The write side ([`commit_command`], spec 04:
//! `docs/specs/04-spec-commit-staged.md`) mirrors `remote.rs`: security by
//! construction, not call-site discipline. The argument vector is closed at
//! `["commit", "-m", message]` — the message is passed verbatim (newlines
//! preserved) as a single argv element, no shell is ever invoked, and no
//! flag beyond `-m` can be attached, so `--amend`, `--no-verify`,
//! `--allow-empty`, and every other flag are impossible. Every child runs
//! with `GIT_TERMINAL_PROMPT=0` so a credential/signing prompt fails fast in
//! the background instead of blocking a worker thread; hooks and the user's
//! git config (signing, sign-off) apply exactly as `git` on their `PATH`
//! would apply them.

use std::path::Path;
use std::process::Command;

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

/// Builds the `git commit -m <message>` [`Command`], to be run at the
/// repository `root` (spec 04).
///
/// The command is a fixed argument vector: exactly `["commit", "-m",
/// message]`, the message verbatim (newlines included) as one argv element —
/// no shell, no user-controlled interpolation, and never `--amend`,
/// `--no-verify`, `--allow-empty`, or any flag beyond `-m`. Sets
/// `GIT_TERMINAL_PROMPT=0` in the child environment so a credential/signing
/// prompt fails fast in the background rather than hanging the worker
/// thread on a blocked read.
pub fn commit_command(message: &str, root: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(root);
    cmd.args(["commit", "-m", message]);
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd
}

/// The commit command line as shown in the command log, e.g.
/// `git commit -m "fix: parser\n\nbody"` — the message is debug-quoted so a
/// multi-line message still logs as one unambiguous line.
pub fn commit_command_line(message: &str) -> String {
    format!("git commit -m {message:?}")
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

    // -- commit_command: the spec 04 write command ---------------------------

    use std::ffi::OsStr;
    use std::path::PathBuf;

    #[test]
    fn commit_command_is_exactly_commit_dash_m_message_at_root() {
        let root = PathBuf::from("/tmp/redquill-commit-test");
        let cmd = commit_command("fix: parser bug", &root);
        assert_eq!(cmd.get_program(), OsStr::new("git"));
        let args: Vec<&OsStr> = cmd.get_args().collect();
        assert_eq!(
            args,
            vec![
                OsStr::new("commit"),
                OsStr::new("-m"),
                OsStr::new("fix: parser bug"),
            ]
        );
        assert_eq!(cmd.get_current_dir(), Some(root.as_path()));
    }

    #[test]
    fn commit_command_passes_a_multiline_message_verbatim_as_one_argv_element() {
        let message = "feat: subject line\n\nbody paragraph\nwith a second line";
        let cmd = commit_command(message, Path::new("."));
        let args: Vec<&OsStr> = cmd.get_args().collect();
        // Exactly three argv elements: newlines never split the message.
        assert_eq!(args.len(), 3);
        assert_eq!(args[2], OsStr::new(message));
    }

    #[test]
    fn commit_command_never_carries_a_flag_beyond_dash_m() {
        // Even a hostile message that *looks* like flags stays a single
        // argv element after `-m`, so git can only ever read it as the
        // message — argv is closed by construction.
        let hostile = "--amend --no-verify";
        let cmd = commit_command(hostile, Path::new("."));
        let args: Vec<&OsStr> = cmd.get_args().collect();
        assert_eq!(
            args,
            vec![OsStr::new("commit"), OsStr::new("-m"), OsStr::new(hostile)]
        );
    }

    #[test]
    fn commit_command_disables_the_terminal_prompt() {
        let cmd = commit_command("msg", Path::new("."));
        let prompt = cmd
            .get_envs()
            .find(|(k, _)| *k == OsStr::new("GIT_TERMINAL_PROMPT"))
            .and_then(|(_, v)| v);
        assert_eq!(prompt, Some(OsStr::new("0")));
    }

    // -- commit_command_line: the command-log display line -------------------

    #[test]
    fn commit_command_line_debug_quotes_the_message() {
        assert_eq!(
            commit_command_line("fix: parser"),
            "git commit -m \"fix: parser\""
        );
    }

    #[test]
    fn commit_command_line_keeps_a_multiline_message_on_one_log_line() {
        let line = commit_command_line("subject\n\nbody");
        assert_eq!(line, "git commit -m \"subject\\n\\nbody\"");
        assert!(!line.contains('\n'));
    }
}
