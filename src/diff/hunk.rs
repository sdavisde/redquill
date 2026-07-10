//! Hunk parsing: turns one file's raw unified-diff patch text (as produced
//! by [`crate::git::split_patches`]) into structured [`Hunk`]s with per-side
//! line numbers.

use super::error::DiffParseError;
use super::line::{DiffLine, LineOrigin};

/// One `@@ -a,b +c,d @@` hunk: a contiguous run of context/added/removed
/// lines, plus the header's line-range and optional section text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// First affected line on the old side (1-based; `0` for an empty old
    /// side, e.g. a newly added file).
    pub old_start: u32,
    /// Number of lines the hunk spans on the old side.
    pub old_count: u32,
    /// First affected line on the new side (1-based; `0` for an empty new
    /// side, e.g. a deleted file).
    pub new_start: u32,
    /// Number of lines the hunk spans on the new side.
    pub new_count: u32,
    /// The function/section context text trailing the header's final `@@`,
    /// if git included one.
    pub section: Option<String>,
    /// The hunk's body lines, in order.
    pub lines: Vec<DiffLine>,
}

/// Parses every hunk out of one file's raw patch text.
///
/// Everything before the first `@@` header (the `diff --git` header, mode
/// lines, `---`/`+++` paths) is ignored. Patches with no hunks at all —
/// binary files, or files with no textual change — yield an empty `Vec`,
/// not an error.
pub fn parse_hunks(raw: &str) -> Result<Vec<Hunk>, DiffParseError> {
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut current: Option<Hunk> = None;
    let mut old_line = 0u32;
    let mut new_line = 0u32;

    for line in raw.lines() {
        if line.starts_with("@@ -") {
            if let Some(hunk) = current.take() {
                hunks.push(hunk);
            }
            let (old_start, old_count, new_start, new_count, section) = parse_header(line)?;
            old_line = old_start;
            new_line = new_start;
            current = Some(Hunk {
                old_start,
                old_count,
                new_start,
                new_count,
                section,
                lines: Vec::new(),
            });
            continue;
        }

        let Some(hunk) = current.as_mut() else {
            // Preamble before the first hunk header; not part of any hunk.
            continue;
        };

        if line.starts_with('\\') {
            // `\ No newline at end of file` (or similar) applies to the
            // line immediately preceding it in the patch.
            if let Some(last) = hunk.lines.last_mut() {
                last.no_newline = true;
            }
            continue;
        }

        let (origin, text) = if let Some(rest) = line.strip_prefix('+') {
            (LineOrigin::Added, rest)
        } else if let Some(rest) = line.strip_prefix('-') {
            (LineOrigin::Removed, rest)
        } else if let Some(rest) = line.strip_prefix(' ') {
            (LineOrigin::Context, rest)
        } else {
            // A blank context line whose leading space marker was trimmed.
            (LineOrigin::Context, line)
        };

        let (old_num, new_num) = match origin {
            LineOrigin::Added => (None, Some(new_line)),
            LineOrigin::Removed => (Some(old_line), None),
            LineOrigin::Context => (Some(old_line), Some(new_line)),
        };
        match origin {
            LineOrigin::Added => new_line += 1,
            LineOrigin::Removed => old_line += 1,
            LineOrigin::Context => {
                old_line += 1;
                new_line += 1;
            }
        }

        hunk.lines.push(DiffLine {
            origin,
            old_line: old_num,
            new_line: new_num,
            content: text.to_string(),
            no_newline: false,
        });
    }

    if let Some(hunk) = current.take() {
        hunks.push(hunk);
    }

    Ok(hunks)
}

/// Parses a `@@ -a[,b] +c[,d] @@[ section]` header line.
fn parse_header(line: &str) -> Result<(u32, u32, u32, u32, Option<String>), DiffParseError> {
    let malformed = || DiffParseError::MalformedHeader(line.to_string());

    let rest = line.strip_prefix("@@ -").ok_or_else(malformed)?;
    let plus_idx = rest.find(" +").ok_or_else(malformed)?;
    let old_part = &rest[..plus_idx];
    let after_plus = &rest[plus_idx + 2..];
    let end_idx = after_plus.find(" @@").ok_or_else(malformed)?;
    let new_part = &after_plus[..end_idx];
    let remainder = after_plus[end_idx + 3..].trim_start();
    let section = if remainder.is_empty() {
        None
    } else {
        Some(remainder.to_string())
    };

    let (old_start, old_count) = parse_range(old_part).ok_or_else(malformed)?;
    let (new_start, new_count) = parse_range(new_part).ok_or_else(malformed)?;

    Ok((old_start, old_count, new_start, new_count, section))
}

/// Parses one side of a header (`start` or `start,count`); an omitted count
/// means `1`.
fn parse_range(s: &str) -> Option<(u32, u32)> {
    match s.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
        None => Some((s.parse().ok()?, 1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(hunk: &Hunk, i: usize) -> &DiffLine {
        &hunk.lines[i]
    }

    #[test]
    fn parses_single_hunk_with_context_add_remove() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 111..222 100644
--- a/f.rs
+++ b/f.rs
@@ -1,3 +1,4 @@
 line1
-line2
+line2 mod
+line new
 line3
";
        let hunks = parse_hunks(raw).unwrap();
        assert_eq!(hunks.len(), 1);
        let hunk = &hunks[0];
        assert_eq!(hunk.old_start, 1);
        assert_eq!(hunk.old_count, 3);
        assert_eq!(hunk.new_start, 1);
        assert_eq!(hunk.new_count, 4);
        assert_eq!(hunk.section, None);
        assert_eq!(hunk.lines.len(), 5);

        assert_eq!(line(hunk, 0).origin, LineOrigin::Context);
        assert_eq!(line(hunk, 0).old_line, Some(1));
        assert_eq!(line(hunk, 0).new_line, Some(1));
        assert_eq!(line(hunk, 0).content, "line1");

        assert_eq!(line(hunk, 1).origin, LineOrigin::Removed);
        assert_eq!(line(hunk, 1).old_line, Some(2));
        assert_eq!(line(hunk, 1).new_line, None);
        assert_eq!(line(hunk, 1).content, "line2");

        assert_eq!(line(hunk, 2).origin, LineOrigin::Added);
        assert_eq!(line(hunk, 2).old_line, None);
        assert_eq!(line(hunk, 2).new_line, Some(2));
        assert_eq!(line(hunk, 2).content, "line2 mod");

        assert_eq!(line(hunk, 3).origin, LineOrigin::Added);
        assert_eq!(line(hunk, 3).old_line, None);
        assert_eq!(line(hunk, 3).new_line, Some(3));
        assert_eq!(line(hunk, 3).content, "line new");

        assert_eq!(line(hunk, 4).origin, LineOrigin::Context);
        assert_eq!(line(hunk, 4).old_line, Some(3));
        assert_eq!(line(hunk, 4).new_line, Some(4));
        assert_eq!(line(hunk, 4).content, "line3");
    }

    #[test]
    fn parses_multiple_hunks_with_independent_counters() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 111..222 100644
--- a/f.rs
+++ b/f.rs
@@ -1,2 +1,2 @@
 a
-b
+B
@@ -10,2 +10,3 @@
 j
+k
 l
";
        let hunks = parse_hunks(raw).unwrap();
        assert_eq!(hunks.len(), 2);

        assert_eq!(hunks[0].old_start, 1);
        assert_eq!(hunks[0].new_start, 1);
        assert_eq!(line(&hunks[0], 0).old_line, Some(1));

        assert_eq!(hunks[1].old_start, 10);
        assert_eq!(hunks[1].new_start, 10);
        assert_eq!(line(&hunks[1], 0).old_line, Some(10));
        assert_eq!(line(&hunks[1], 0).new_line, Some(10));
        assert_eq!(line(&hunks[1], 1).origin, LineOrigin::Added);
        assert_eq!(line(&hunks[1], 1).new_line, Some(11));
        assert_eq!(line(&hunks[1], 2).old_line, Some(11));
        assert_eq!(line(&hunks[1], 2).new_line, Some(12));
    }

    #[test]
    fn count_omitted_header_implies_count_one() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -5 +5 @@
-x
+y
";
        let hunks = parse_hunks(raw).unwrap();
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].old_start, 5);
        assert_eq!(hunks[0].old_count, 1);
        assert_eq!(hunks[0].new_start, 5);
        assert_eq!(hunks[0].new_count, 1);
    }

    #[test]
    fn added_file_starts_at_zero_old_count() {
        let raw = "\
diff --git a/new.rs b/new.rs
new file mode 100644
index 000..111
--- /dev/null
+++ b/new.rs
@@ -0,0 +1,5 @@
+a
+b
+c
+d
+e
";
        let hunks = parse_hunks(raw).unwrap();
        assert_eq!(hunks.len(), 1);
        let hunk = &hunks[0];
        assert_eq!(hunk.old_start, 0);
        assert_eq!(hunk.old_count, 0);
        assert_eq!(hunk.new_start, 1);
        assert_eq!(hunk.new_count, 5);
        assert_eq!(hunk.lines.len(), 5);
        for (i, l) in hunk.lines.iter().enumerate() {
            assert_eq!(l.origin, LineOrigin::Added);
            assert_eq!(l.old_line, None);
            assert_eq!(l.new_line, Some(1 + i as u32));
        }
    }

    #[test]
    fn no_newline_marker_attaches_to_removed_line() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,2 +1,2 @@
 line1
-old last
\\ No newline at end of file
+new last
";
        let hunks = parse_hunks(raw).unwrap();
        let hunk = &hunks[0];
        assert!(line(hunk, 1).no_newline);
        assert!(!line(hunk, 0).no_newline);
        assert!(!line(hunk, 2).no_newline);
    }

    #[test]
    fn no_newline_marker_attaches_to_added_line() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,2 +1,2 @@
 line1
-old last
+new last
\\ No newline at end of file
";
        let hunks = parse_hunks(raw).unwrap();
        let hunk = &hunks[0];
        assert!(!line(hunk, 1).no_newline);
        assert!(line(hunk, 2).no_newline);
    }

    #[test]
    fn no_newline_marker_on_both_sides() {
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
        let hunks = parse_hunks(raw).unwrap();
        let hunk = &hunks[0];
        assert!(line(hunk, 0).no_newline);
        assert!(line(hunk, 1).no_newline);
    }

    #[test]
    fn section_text_after_trailing_at_at() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -10,2 +10,3 @@ fn foo() {
 context
+added
 context2
";
        let hunks = parse_hunks(raw).unwrap();
        assert_eq!(hunks[0].section.as_deref(), Some("fn foo() {"));
    }

    #[test]
    fn no_section_text_is_none() {
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,1 +1,1 @@
-x
+y
";
        let hunks = parse_hunks(raw).unwrap();
        assert_eq!(hunks[0].section, None);
    }

    #[test]
    fn malformed_header_is_an_error() {
        // Starts with "@@ -" (so it's recognized as a header attempt) but
        // the old-side count isn't a valid integer.
        let raw = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,x +1 @@
+x
";
        let err = parse_hunks(raw).unwrap_err();
        assert!(matches!(err, DiffParseError::MalformedHeader(_)));
    }

    #[test]
    fn binary_patch_yields_no_hunks_not_an_error() {
        let raw = "\
diff --git a/img.png b/img.png
index 111..222 100644
Binary files a/img.png and b/img.png differ
";
        let hunks = parse_hunks(raw).unwrap();
        assert!(hunks.is_empty());
    }

    #[test]
    fn deleted_file_zero_new_count() {
        let raw = "\
diff --git a/gone.rs b/gone.rs
deleted file mode 100644
index 111..000
--- a/gone.rs
+++ /dev/null
@@ -1,3 +0,0 @@
-a
-b
-c
";
        let hunks = parse_hunks(raw).unwrap();
        let hunk = &hunks[0];
        assert_eq!(hunk.new_start, 0);
        assert_eq!(hunk.new_count, 0);
        assert_eq!(hunk.lines.len(), 3);
        for l in &hunk.lines {
            assert_eq!(l.origin, LineOrigin::Removed);
            assert_eq!(l.new_line, None);
        }
    }
}
