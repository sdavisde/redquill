//! Index staging plumbing.
//!
//! File-level staging goes through plain `git` porcelain (`add`, `restore
//! --staged` / `rm --cached`). Hunk- and line-level staging is built on one
//! primitive: constructing a minimal, valid patch and piping it to
//! `git apply --cached` (or `--reverse` to unstage). Every operation here
//! writes to the index only — the working tree is never touched.

use std::collections::HashSet;
use std::io::Write;
use std::process::{Command, Stdio};

use super::diff::RawFilePatch;
use super::error::GitError;
use super::runner::GitRunner;

impl GitRunner {
    /// Stages `path` in its entirety, including untracked and deleted files.
    ///
    /// Equivalent to `git add -A -- <path>`: unlike plain `git add`, `-A`
    /// also stages deletions for the given pathspec.
    pub fn stage_file(&self, path: &str) -> Result<(), GitError> {
        self.run_index(&["add", "-A", "--", path])
    }

    /// Unstages `path`, reverting its index entry to match `HEAD`.
    ///
    /// `git restore --staged` requires a `HEAD` commit to restore from. In a
    /// repository with no commits yet there is no `HEAD`, so this falls back
    /// to dropping the path from the index directly (`git rm --cached`),
    /// which is the correct "unstaged" state pre-first-commit.
    pub fn unstage_file(&self, path: &str) -> Result<(), GitError> {
        if self.has_head() {
            self.run_index(&["restore", "--staged", "--", path])
        } else {
            self.run_index(&["rm", "--cached", "--ignore-unmatch", "--", path])
        }
    }

    /// Applies `patch` to the index only (`git apply --cached`). The working
    /// tree is never touched.
    pub fn apply_cached(&self, patch: &str) -> Result<(), GitError> {
        self.run_index_with_stdin(&["apply", "--cached", "--unidiff-zero", "-"], patch)
    }

    /// Reverses `patch` against the index only (`git apply --cached
    /// --reverse`). The working tree is never touched.
    pub fn unapply_cached(&self, patch: &str) -> Result<(), GitError> {
        self.run_index_with_stdin(
            &["apply", "--cached", "--unidiff-zero", "--reverse", "-"],
            patch,
        )
    }

    /// Whether the repository has a `HEAD` commit.
    fn has_head(&self) -> bool {
        Command::new("git")
            .current_dir(self.root())
            .args(["rev-parse", "--verify", "-q", "HEAD"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Runs a git subcommand at the repo root, discarding stdout, erroring on
    /// a non-zero exit.
    fn run_index(&self, args: &[&str]) -> Result<(), GitError> {
        let output = Command::new("git")
            .current_dir(self.root())
            .args(args)
            .output()
            .map_err(map_spawn_err)?;
        check_status(args, &output.status, &output.stderr)
    }

    /// Runs a git subcommand at the repo root, writing `stdin_data` to its
    /// stdin and erroring on a non-zero exit.
    fn run_index_with_stdin(&self, args: &[&str], stdin_data: &str) -> Result<(), GitError> {
        let mut child = Command::new("git")
            .current_dir(self.root())
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(map_spawn_err)?;

        let mut stdin = child.stdin.take().ok_or_else(|| GitError::Command {
            command: args.join(" "),
            code: "spawn".to_string(),
            stderr: "failed to open git stdin".to_string(),
        })?;
        stdin
            .write_all(stdin_data.as_bytes())
            .map_err(GitError::Spawn)?;
        drop(stdin); // Close stdin so git sees EOF.

        let output = child.wait_with_output().map_err(GitError::Spawn)?;
        check_status(args, &output.status, &output.stderr)
    }
}

/// Maps a spawn `io::Error` to a `GitNotFound` when git is absent, else `Spawn`.
fn map_spawn_err(e: std::io::Error) -> GitError {
    if e.kind() == std::io::ErrorKind::NotFound {
        GitError::GitNotFound
    } else {
        GitError::Spawn(e)
    }
}

/// Turns a non-zero exit status into a [`GitError::Command`].
fn check_status(
    args: &[&str],
    status: &std::process::ExitStatus,
    stderr: &[u8],
) -> Result<(), GitError> {
    if status.success() {
        return Ok(());
    }
    Err(GitError::Command {
        command: args.join(" "),
        code: status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "signal".to_string()),
        stderr: String::from_utf8_lossy(stderr).trim().to_string(),
    })
}

/// Splits a raw file patch into its header (everything before the first
/// `@@` hunk line) and the verbatim text of each hunk, in order.
fn split_header_and_hunks(raw: &str) -> (String, Vec<String>) {
    let mut starts = Vec::new();
    let mut offset = 0usize;
    for line in raw.split_inclusive('\n') {
        if line.starts_with("@@ -") {
            starts.push(offset);
        }
        offset += line.len();
    }
    let header_end = starts.first().copied().unwrap_or(raw.len());
    let header = raw[..header_end].to_string();

    let mut hunks = Vec::with_capacity(starts.len());
    for (i, &start) in starts.iter().enumerate() {
        let end = starts.get(i + 1).copied().unwrap_or(raw.len());
        hunks.push(raw[start..end].to_string());
    }
    (header, hunks)
}

/// The parsed pieces of a `@@ -old_start,old_count +new_start,new_count @@
/// section` header line that a rebuilt hunk needs to preserve.
struct HunkHeader {
    old_start: u32,
    new_start: u32,
    /// The trailing section text, including its leading space if present
    /// (empty string when git included no section).
    section: String,
}

/// Parses one `@@ ... @@` header line. Only the starts and the trailing
/// section text are kept; counts are recomputed by the caller.
fn parse_hunk_header_line(line: &str) -> Result<HunkHeader, GitError> {
    let malformed = || GitError::Parse(format!("malformed hunk header: {line:?}"));

    let rest = line.strip_prefix("@@ -").ok_or_else(malformed)?;
    let plus_idx = rest.find(" +").ok_or_else(malformed)?;
    let old_part = &rest[..plus_idx];
    let after_plus = &rest[plus_idx + 2..];
    let end_idx = after_plus.find(" @@").ok_or_else(malformed)?;
    let new_part = &after_plus[..end_idx];
    let remainder = &after_plus[end_idx + 3..];
    let section_end = remainder.find('\n').unwrap_or(remainder.len());
    let section = remainder[..section_end].to_string();

    let old_start_str = old_part.split_once(',').map_or(old_part, |(s, _)| s);
    let new_start_str = new_part.split_once(',').map_or(new_part, |(s, _)| s);
    let old_start: u32 = old_start_str.parse().map_err(|_| malformed())?;
    let new_start: u32 = new_start_str.parse().map_err(|_| malformed())?;

    Ok(HunkHeader {
        old_start,
        new_start,
        section,
    })
}

/// One line of a hunk body: its marker (`' '`, `'+'`, or `'-'`), its text
/// content (without the marker or trailing newline), and whether a
/// `\ No newline at end of file` marker immediately follows it in the
/// source patch.
struct BodyLine {
    marker: char,
    content: String,
    no_newline: bool,
}

/// Parses the body lines following a hunk's `@@` header line out of the
/// hunk's verbatim text (header line included at the start).
fn parse_body_lines(hunk_text: &str) -> Vec<BodyLine> {
    let mut lines = hunk_text.split_inclusive('\n');
    lines.next(); // Skip the `@@ ... @@` header line itself.

    let mut bodies: Vec<BodyLine> = Vec::new();
    for raw_line in lines {
        let stripped = raw_line.strip_suffix('\n').unwrap_or(raw_line);
        if stripped.starts_with('\\') {
            // `\ No newline at end of file` applies to the previous line.
            if let Some(prev) = bodies.last_mut() {
                prev.no_newline = true;
            }
            continue;
        }
        let mut chars = stripped.chars();
        let marker = chars.next().unwrap_or(' ');
        let content = chars.as_str().to_string();
        let marker = if marker == '+' || marker == '-' {
            marker
        } else {
            // A literal space marker, or a blank context line whose leading
            // space was trimmed — either way, context.
            ' '
        };
        bodies.push(BodyLine {
            marker,
            content,
            no_newline: false,
        });
    }
    bodies
}

/// Builds a minimal, valid patch containing the file headers of `patch` plus
/// exactly hunk `hunk_index`, verbatim. Applying this via [`GitRunner::apply_cached`]
/// stages just that hunk.
///
/// Errors if `hunk_index` is out of range for the file's hunks.
pub fn build_hunk_patch(patch: &RawFilePatch, hunk_index: usize) -> Result<String, GitError> {
    let (header, hunks) = split_header_and_hunks(&patch.raw);
    let hunk = hunks.get(hunk_index).ok_or_else(|| {
        GitError::Parse(format!(
            "hunk index {hunk_index} out of range: {} hunk(s) in {}",
            hunks.len(),
            patch.path
        ))
    })?;

    let mut out = header;
    out.push_str(hunk);
    Ok(out)
}

/// Builds a patch that stages only the selected `+`/`-` lines within hunk
/// `hunk_index` of `patch`. `line_indices` are indices into that hunk's body
/// lines, counting every context/added/removed line starting at 0.
///
/// Non-selected `-` lines become context lines (kept, marker switched to
/// space); non-selected `+` lines are dropped entirely. The hunk header's
/// `old_count`/`new_count` are recomputed to match; the starts are
/// unchanged. Indices pointing at context lines are ignored (context is
/// always kept). If the resulting hunk would contain no `+`/`-` lines at all
/// (nothing selected, or only context indices were given), this returns
/// `Err(GitError::Parse(_))` rather than silently emitting a no-op patch —
/// callers should treat "nothing to stage" as a decision made before calling
/// this, not a valid patch to apply.
pub fn build_line_patch(
    patch: &RawFilePatch,
    hunk_index: usize,
    line_indices: &[usize],
) -> Result<String, GitError> {
    let (header, hunks) = split_header_and_hunks(&patch.raw);
    let hunk_text = hunks.get(hunk_index).ok_or_else(|| {
        GitError::Parse(format!(
            "hunk index {hunk_index} out of range: {} hunk(s) in {}",
            hunks.len(),
            patch.path
        ))
    })?;

    let header_line = hunk_text
        .split_inclusive('\n')
        .next()
        .ok_or_else(|| GitError::Parse(format!("empty hunk body at index {hunk_index}")))?;
    let hunk_header = parse_hunk_header_line(header_line)?;

    let bodies = parse_body_lines(hunk_text);
    if bodies.is_empty() {
        return Err(GitError::Parse(format!(
            "hunk {hunk_index} in {} has no body lines",
            patch.path
        )));
    }

    let selected: HashSet<usize> = line_indices.iter().copied().collect();

    let mut out_lines: Vec<(char, &str, bool)> = Vec::with_capacity(bodies.len());
    let mut old_count = 0u32;
    let mut new_count = 0u32;
    let mut any_change = false;

    for (i, body) in bodies.iter().enumerate() {
        match body.marker {
            ' ' => {
                out_lines.push((' ', &body.content, body.no_newline));
                old_count += 1;
                new_count += 1;
            }
            '-' => {
                if selected.contains(&i) {
                    out_lines.push(('-', &body.content, body.no_newline));
                    old_count += 1;
                    any_change = true;
                } else {
                    // Unselected removal: keep the text, but as context.
                    out_lines.push((' ', &body.content, body.no_newline));
                    old_count += 1;
                    new_count += 1;
                }
            }
            '+' => {
                if selected.contains(&i) {
                    out_lines.push(('+', &body.content, body.no_newline));
                    new_count += 1;
                    any_change = true;
                }
                // Unselected addition: dropped entirely.
            }
            _ => unreachable!("markers are normalized to ' ', '-', or '+'"),
        }
    }

    if !any_change {
        return Err(GitError::Parse(
            "line selection produced an all-context hunk with nothing to stage".to_string(),
        ));
    }

    let mut result = header;
    result.push_str(&format!(
        "@@ -{},{} +{},{} @@{}\n",
        hunk_header.old_start, old_count, hunk_header.new_start, new_count, hunk_header.section
    ));
    for (marker, content, no_newline) in out_lines {
        result.push(marker);
        result.push_str(content);
        result.push('\n');
        if no_newline {
            result.push_str("\\ No newline at end of file\n");
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(raw: &str) -> RawFilePatch {
        RawFilePatch {
            path: "f.rs".to_string(),
            old_path: None,
            raw: raw.to_string(),
            is_binary: false,
        }
    }

    const MULTI_HUNK: &str = "\
diff --git a/f.rs b/f.rs
index 111..222 100644
--- a/f.rs
+++ b/f.rs
@@ -1,2 +1,2 @@
 top context
-old top
+new top
@@ -10,2 +10,2 @@
 bottom context
-old bottom
+new bottom
";

    #[test]
    fn build_hunk_patch_extracts_first_hunk_only() {
        let p = patch(MULTI_HUNK);
        let out = build_hunk_patch(&p, 0).unwrap();
        assert!(out.contains("diff --git a/f.rs b/f.rs"));
        assert!(out.contains("--- a/f.rs"));
        assert!(out.contains("@@ -1,2 +1,2 @@"));
        assert!(out.contains("-old top"));
        assert!(out.contains("+new top"));
        assert!(!out.contains("bottom"));
    }

    #[test]
    fn build_hunk_patch_extracts_second_hunk_only() {
        let p = patch(MULTI_HUNK);
        let out = build_hunk_patch(&p, 1).unwrap();
        assert!(out.contains("diff --git a/f.rs b/f.rs"));
        assert!(out.contains("@@ -10,2 +10,2 @@"));
        assert!(out.contains("-old bottom"));
        assert!(out.contains("+new bottom"));
        assert!(!out.contains("top context"));
        assert!(!out.contains("old top"));
    }

    #[test]
    fn build_hunk_patch_out_of_range_errors() {
        let p = patch(MULTI_HUNK);
        let err = build_hunk_patch(&p, 2).unwrap_err();
        assert!(matches!(err, GitError::Parse(_)));
    }

    #[test]
    fn build_hunk_patch_preserves_no_newline_marker() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1 +1 @@
-old last
\\ No newline at end of file
+new last
\\ No newline at end of file
";
        let p = patch(raw);
        let out = build_hunk_patch(&p, 0).unwrap();
        assert_eq!(out.matches("\\ No newline at end of file").count(), 2);
    }

    // A hunk with balanced old/new counts (4/4), used to exercise line-level
    // patch construction: `ctx1` and `ctx2` are context, `old1`/`old2` are
    // removed, `new1`/`new2` are added.
    const LINE_HUNK: &str = "\
diff --git a/f.rs b/f.rs
index 111..222 100644
--- a/f.rs
+++ b/f.rs
@@ -1,4 +1,4 @@
 ctx1
-old1
-old2
+new1
+new2
 ctx2
";

    #[test]
    fn build_line_patch_selects_single_added_line() {
        let p = patch(LINE_HUNK);
        // Indices: 0=ctx1, 1=old1(-), 2=old2(-), 3=new1(+), 4=new2(+), 5=ctx2.
        let out = build_line_patch(&p, 0, &[3]).unwrap();

        assert!(out.contains("@@ -1,4 +1,5 @@"));
        // Unselected removals become context (kept, not removed).
        assert!(out.contains(" old1\n"));
        assert!(out.contains(" old2\n"));
        // Selected addition stays an addition.
        assert!(out.contains("+new1\n"));
        // Unselected addition is dropped entirely.
        assert!(!out.contains("new2"));
        assert!(out.contains(" ctx1\n"));
        assert!(out.contains(" ctx2\n"));
    }

    #[test]
    fn build_line_patch_selects_single_removed_line() {
        let p = patch(LINE_HUNK);
        let out = build_line_patch(&p, 0, &[1]).unwrap();

        // old1 selected for removal stays removed; old2 becomes context.
        assert!(out.contains("-old1\n"));
        assert!(out.contains(" old2\n"));
        // Neither addition was selected, so both are dropped.
        assert!(!out.contains("new1"));
        assert!(!out.contains("new2"));
        // old_count: ctx1 + old1(removed) + old2(context) + ctx2 = 4
        // new_count: ctx1 + old2(context) + ctx2 = 3
        assert!(out.contains("@@ -1,4 +1,3 @@"));
    }

    #[test]
    fn build_line_patch_context_index_is_ignored() {
        let p = patch(LINE_HUNK);
        // Index 0 is ctx1, a context line — selecting it plus a real
        // addition should behave identically to selecting just the addition.
        let with_ctx = build_line_patch(&p, 0, &[0, 3]).unwrap();
        let without_ctx = build_line_patch(&p, 0, &[3]).unwrap();
        assert_eq!(with_ctx, without_ctx);
    }

    #[test]
    fn build_line_patch_all_context_selection_errors() {
        let p = patch(LINE_HUNK);
        // Nothing selected at all.
        let err = build_line_patch(&p, 0, &[]).unwrap_err();
        assert!(matches!(err, GitError::Parse(_)));

        // Only a context index selected — still a no-op.
        let err = build_line_patch(&p, 0, &[0]).unwrap_err();
        assert!(matches!(err, GitError::Parse(_)));
    }

    #[test]
    fn build_line_patch_out_of_range_hunk_index_errors() {
        let p = patch(LINE_HUNK);
        let err = build_line_patch(&p, 5, &[0]).unwrap_err();
        assert!(matches!(err, GitError::Parse(_)));
    }

    #[test]
    fn build_line_patch_preserves_no_newline_marker() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,2 +1,2 @@
 ctx
-old last
+new last
\\ No newline at end of file
";
        let p = patch(raw);
        // Indices: 0=ctx, 1=old last(-), 2=new last(+).
        let out = build_line_patch(&p, 0, &[1, 2]).unwrap();
        assert!(out.contains("+new last\n\\ No newline at end of file\n"));
    }

    #[test]
    fn build_line_patch_selecting_both_removed_and_added_matches_full_hunk_shape() {
        let p = patch(LINE_HUNK);
        let out = build_line_patch(&p, 0, &[1, 2, 3, 4]).unwrap();
        assert!(out.contains("@@ -1,4 +1,4 @@"));
        assert!(out.contains("-old1\n"));
        assert!(out.contains("-old2\n"));
        assert!(out.contains("+new1\n"));
        assert!(out.contains("+new2\n"));
    }

    #[test]
    fn split_header_and_hunks_handles_no_hunks() {
        let raw = "\
diff --git a/img.png b/img.png
index 111..222 100644
Binary files a/img.png and b/img.png differ
";
        let (header, hunks) = split_header_and_hunks(raw);
        assert_eq!(header, raw);
        assert!(hunks.is_empty());
    }
}
