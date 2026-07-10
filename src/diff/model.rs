//! Diff model core types and patch parsing.
//!
//! Turns a [`crate::git::RawFilePatch`] (raw, unparsed patch text for one
//! file) into a typed [`DiffFile`] tree: hunks, then lines, with old/new line
//! numbers and per-line kind. Pure data + a total parser — no I/O, no TUI
//! types, never panics on valid UTF-8 input (`git/` guarantees that upstream).
//!
//! Only two-way (`diff --git`) patches are supported; `@@@` combined-diff
//! (merge) headers are out of scope for this task (spec §6/§7) and are
//! best-effort treated as unrecognized body content rather than causing a
//! panic.

/// One file's parsed diff: metadata plus ordered hunks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffFile {
    pub path: String,
    pub old_path: Option<String>,
    pub status: ChangeStatus,
    /// Orthogonal to `status`: a renamed or modified file may also change mode.
    pub mode_change: Option<(String, String)>,
    /// Orthogonal to `status`: binary-ness is carried through from `git/`; a
    /// binary file is still Added/Modified/Deleted. Binary files carry zero
    /// hunks.
    pub is_binary: bool,
    pub hunks: Vec<Hunk>,
}

/// File-level change classification, richer than a porcelain letter.
/// Deliberately excludes binary and mode-change — those are orthogonal flags
/// on `DiffFile` (they co-occur with every variant), not statuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeStatus {
    Modified,
    Added,
    Deleted,
    Renamed { similarity: Option<u8> },
}

/// One `@@ ... @@` region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_count: u32,
    pub new_start: u32,
    pub new_count: u32,
    /// Text after the closing `@@` (function/section context), if any.
    pub section: Option<String>,
    pub lines: Vec<Line>,
}

/// One line of hunk body content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub kind: LineKind,
    pub old_lineno: Option<u32>,
    pub new_lineno: Option<u32>,
    /// Without the leading `+`/`-`/space marker.
    pub content: String,
    /// Preceded a `\ No newline at end of file` marker.
    pub no_newline: bool,
    /// Word-diff spans over `content` (char indices). Empty unless paired by
    /// `word::attach_word_spans`; always empty as produced by this module.
    pub changed_spans: Vec<std::ops::Range<usize>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

/// A cursor into a parsed model for navigation. Indices are stable for a
/// given parsed set (`Vec<DiffFile>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiffPosition {
    pub file: usize,
    pub hunk: usize,
    pub line: usize,
}

/// Aggregate counts across a parsed set, for the `main.rs` summary line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DiffSummary {
    pub files: usize,
    pub hunks: usize,
    pub added: usize,
    pub removed: usize,
}

/// Parses one `@@ -a[,b] +c[,d] @@[ section]` header line.
///
/// Returns `(old_start, old_count, new_start, new_count, section)`. An
/// omitted count defaults to 1 (spec §6 / FR-diff-parse-2). Returns `None`
/// for a line that doesn't match the expected shape; callers treat that as a
/// best-effort degradation rather than panicking.
// FR-diff-parse-2
fn parse_hunk_header(line: &str) -> Option<(u32, u32, u32, u32, Option<String>)> {
    let rest = line.strip_prefix("@@ ")?;
    let close_idx = rest.find(" @@")?;
    let ranges = &rest[..close_idx];
    let section_part = rest.get(close_idx + 3..)?;

    let mut parts = ranges.split_whitespace();
    let old = parts.next()?;
    let new = parts.next()?;
    let (old_start, old_count) = parse_range(old.strip_prefix('-')?)?;
    let (new_start, new_count) = parse_range(new.strip_prefix('+')?)?;

    let section = section_part.trim();
    let section = if section.is_empty() {
        None
    } else {
        Some(section.to_string())
    };

    Some((old_start, old_count, new_start, new_count, section))
}

/// Parses one side of a hunk header range: `N` (count defaults to 1) or
/// `N,M`.
fn parse_range(s: &str) -> Option<(u32, u32)> {
    match s.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
        None => Some((s.parse().ok()?, 1)),
    }
}

/// Metadata accumulated while scanning lines before/around hunk bodies.
#[derive(Default)]
struct Metadata {
    is_new: bool,
    is_deleted: bool,
    old_mode: Option<String>,
    new_mode: Option<String>,
    similarity: Option<u8>,
}

/// Classifies a single non-header metadata line (FR-diff-parse-4). Lines that
/// don't match a known metadata shape (e.g. `diff --git`, `index`, `---`,
/// `+++`, `rename from/to`, `copy from/to`) are ignored here — their
/// information (paths) is already carried through on `RawFilePatch`.
// FR-diff-parse-4
fn apply_metadata_line(line: &str, meta: &mut Metadata) {
    if line.starts_with("new file mode ") {
        meta.is_new = true;
    } else if line.starts_with("deleted file mode ") {
        meta.is_deleted = true;
    } else if let Some(rest) = line.strip_prefix("old mode ") {
        meta.old_mode = Some(rest.trim().to_string());
    } else if let Some(rest) = line.strip_prefix("new mode ") {
        meta.new_mode = Some(rest.trim().to_string());
    } else if let Some(rest) = line.strip_prefix("similarity index ") {
        meta.similarity = rest.trim().trim_end_matches('%').parse().ok();
    }
}

/// Derives the file-level [`ChangeStatus`] from accumulated metadata and the
/// carried-through `old_path`. Priority: explicit new/deleted markers win
/// over an implied rename, which wins over plain `Modified`.
// FR-diff-parse-4
fn derive_status(meta: &Metadata, old_path: &Option<String>) -> ChangeStatus {
    if meta.is_new {
        ChangeStatus::Added
    } else if meta.is_deleted {
        ChangeStatus::Deleted
    } else if old_path.is_some() {
        ChangeStatus::Renamed {
            similarity: meta.similarity,
        }
    } else {
        ChangeStatus::Modified
    }
}

/// Classifies and appends one hunk body line (FR-diff-parse-3), advancing
/// the running old/new line counters. A `\ No newline at end of file`
/// marker (FR-diff-parse-5) sets a flag on the previously pushed line
/// instead of producing its own `Line`. Unrecognized line shapes (should not
/// occur in well-formed git output) degrade to a best-effort `Context` line
/// covering the whole line, rather than panicking — parsing is total.
/// Counter increments saturate at `u32::MAX` rather than overflowing, so an
/// adversarial hunk header (e.g. a start line near `u32::MAX`) can never
/// panic in debug or silently wrap in release.
// FR-diff-parse-3
// FR-diff-parse-5
fn apply_body_line(line: &str, hunk: &mut Hunk, old_lineno: &mut u32, new_lineno: &mut u32) {
    if let Some(_rest) = line.strip_prefix('\\') {
        // e.g. "\ No newline at end of file" — flag the preceding line.
        if let Some(last) = hunk.lines.last_mut() {
            last.no_newline = true;
        }
        return;
    }

    let (kind, content, old_no, new_no) = if let Some(rest) = line.strip_prefix('+') {
        let no = *new_lineno;
        // Saturate rather than wrap/panic: a crafted hunk header can start a
        // counter at u32::MAX (spec totality invariant — never panics on
        // valid UTF-8 input, even adversarial header values).
        *new_lineno = new_lineno.saturating_add(1);
        (LineKind::Added, rest, None, Some(no))
    } else if let Some(rest) = line.strip_prefix('-') {
        let no = *old_lineno;
        *old_lineno = old_lineno.saturating_add(1);
        (LineKind::Removed, rest, Some(no), None)
    } else if let Some(rest) = line.strip_prefix(' ') {
        let old_no = *old_lineno;
        let new_no = *new_lineno;
        *old_lineno = old_lineno.saturating_add(1);
        *new_lineno = new_lineno.saturating_add(1);
        (LineKind::Context, rest, Some(old_no), Some(new_no))
    } else {
        // Best-effort: an empty line inside a hunk body (some tools trim the
        // mandatory leading space off a blank context line) or any other
        // unexpected shape is treated as a context line rather than
        // panicking or dropping data.
        let old_no = *old_lineno;
        let new_no = *new_lineno;
        *old_lineno = old_lineno.saturating_add(1);
        *new_lineno = new_lineno.saturating_add(1);
        (LineKind::Context, line, Some(old_no), Some(new_no))
    };

    hunk.lines.push(Line {
        kind,
        old_lineno: old_no,
        new_lineno: new_no,
        content: content.to_string(),
        no_newline: false,
        changed_spans: Vec::new(),
    });
}

/// Parses one [`RawFilePatch`] into a [`DiffFile`].
///
/// Total: never panics on valid UTF-8 input. Walks `patch.raw` in a single
/// pass, classifying each line as hunk-header, hunk-body, or file metadata.
/// `path` / `old_path` / `is_binary` are carried through verbatim, never
/// re-derived (per the git contract). `changed_spans` is left empty — it is
/// populated later by `word::attach_word_spans`.
// FR-diff-parse-1
pub fn parse_patch(patch: &crate::git::RawFilePatch) -> DiffFile {
    let mut meta = Metadata::default();
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut current: Option<Hunk> = None;
    let mut old_lineno: u32 = 0;
    let mut new_lineno: u32 = 0;

    // Split on the `\n` record separator only — NOT `str::lines()`, which
    // also swallows a trailing `\r` and would corrupt CRLF file content that
    // git preserves as part of a line's body (spec §6). At most one trailing
    // `\n` (the final line terminator) is stripped; embedded `\r` bytes are
    // left untouched as ordinary content.
    let text = patch.raw.strip_suffix('\n').unwrap_or(&patch.raw);
    for line in text.split('\n') {
        let is_hunk_header = !patch.is_binary && line.starts_with("@@ ");
        if is_hunk_header {
            if let Some(h) = current.take() {
                hunks.push(h);
            }
            match parse_hunk_header(line) {
                Some((old_start, old_count, new_start, new_count, section)) => {
                    old_lineno = old_start;
                    new_lineno = new_start;
                    current = Some(Hunk {
                        old_start,
                        old_count,
                        new_start,
                        new_count,
                        section,
                        lines: Vec::new(),
                    });
                }
                None => {
                    // Malformed header: best-effort, drop this hunk boundary
                    // rather than panicking; subsequent lines fall back to
                    // metadata scanning (harmless — they won't match any
                    // metadata prefix and are simply ignored).
                    current = None;
                }
            }
            continue;
        }

        match current.as_mut() {
            Some(hunk) => apply_body_line(line, hunk, &mut old_lineno, &mut new_lineno),
            None => apply_metadata_line(line, &mut meta),
        }
    }
    if let Some(h) = current.take() {
        hunks.push(h);
    }

    // Binary files are pass-through with zero hunks; never attempt to parse
    // a body (spec §6). `is_binary` patches have no "@@ " lines recognized
    // above, so `hunks` is already empty in that case.
    let status = derive_status(&meta, &patch.old_path);
    let mode_change = match (meta.old_mode, meta.new_mode) {
        (Some(old), Some(new)) => Some((old, new)),
        _ => None,
    };

    DiffFile {
        path: patch.path.clone(),
        old_path: patch.old_path.clone(),
        status,
        mode_change,
        is_binary: patch.is_binary,
        hunks,
    }
}

/// Parses every patch in `patches`, preserving order.
// FR-diff-parse-1
pub fn parse_patches(patches: &[crate::git::RawFilePatch]) -> Vec<DiffFile> {
    patches.iter().map(parse_patch).collect()
}

/// Aggregates file/hunk/added/removed counts across a parsed set, for the
/// `main.rs` summary line (T4.0).
pub fn summarize(files: &[DiffFile]) -> DiffSummary {
    let mut summary = DiffSummary {
        files: files.len(),
        ..DiffSummary::default()
    };
    for file in files {
        summary.hunks += file.hunks.len();
        for hunk in &file.hunks {
            for line in &hunk.lines {
                match line.kind {
                    LineKind::Added => summary.added += 1,
                    LineKind::Removed => summary.removed += 1,
                    LineKind::Context => {}
                }
            }
        }
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::split_patches;

    /// Parses a single-file raw diff string through the same `git/`
    /// splitting logic `main.rs` uses, returning the one `DiffFile`.
    fn parse_one(raw: &str) -> DiffFile {
        let patches = split_patches(raw);
        assert_eq!(patches.len(), 1, "fixture must contain exactly one file");
        parse_patch(&patches[0])
    }

    // --- 1.1: hunk header parsing (FR-diff-parse-2) ---------------------

    #[test]
    fn two_hunk_header_parse_with_section_text() {
        let diff = "\
diff --git a/src/foo.rs b/src/foo.rs
index 111..222 100644
--- a/src/foo.rs
+++ b/src/foo.rs
@@ -1,3 +1,3 @@ fn foo() {
 line1
-line2
+line2b
 line3
@@ -10 +10 @@
 line10
";
        let file = parse_one(diff);
        assert_eq!(file.hunks.len(), 2);

        let h0 = &file.hunks[0];
        assert_eq!(h0.old_start, 1);
        assert_eq!(h0.old_count, 3);
        assert_eq!(h0.new_start, 1);
        assert_eq!(h0.new_count, 3);
        assert_eq!(h0.section.as_deref(), Some("fn foo() {"));

        // Omitted count case: `@@ -10 +10 @@` — absent count defaults to 1.
        let h1 = &file.hunks[1];
        assert_eq!(h1.old_start, 10);
        assert_eq!(h1.old_count, 1);
        assert_eq!(h1.new_start, 10);
        assert_eq!(h1.new_count, 1);
        assert_eq!(h1.section, None);
    }

    // --- 1.2: line classification + line-number assignment (FR-diff-parse-3) ---

    #[test]
    fn add_remove_context_get_disjoint_line_numbers() {
        let diff = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1,2 +1,2 @@
-old_line
+new_line
 context_line
";
        let file = parse_one(diff);
        assert_eq!(file.hunks.len(), 1);
        let lines = &file.hunks[0].lines;
        assert_eq!(lines.len(), 3);

        assert_eq!(lines[0].kind, LineKind::Removed);
        assert_eq!(lines[0].old_lineno, Some(1));
        assert_eq!(lines[0].new_lineno, None);
        assert_eq!(lines[0].content, "old_line");

        assert_eq!(lines[1].kind, LineKind::Added);
        assert_eq!(lines[1].old_lineno, None);
        assert_eq!(lines[1].new_lineno, Some(1));
        assert_eq!(lines[1].content, "new_line");

        // Following context line advances both sides from where each left off.
        assert_eq!(lines[2].kind, LineKind::Context);
        assert_eq!(lines[2].old_lineno, Some(2));
        assert_eq!(lines[2].new_lineno, Some(2));
        assert_eq!(lines[2].content, "context_line");
    }

    #[test]
    fn zero_context_hunk_pure_addition_parses_correctly() {
        let diff = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -5,0 +6,2 @@
+added1
+added2
";
        let file = parse_one(diff);
        assert_eq!(file.hunks.len(), 1);
        let hunk = &file.hunks[0];
        assert_eq!(hunk.old_count, 0);
        assert_eq!(hunk.new_count, 2);
        assert_eq!(hunk.lines.len(), 2);
        assert_eq!(hunk.lines[0].new_lineno, Some(6));
        assert_eq!(hunk.lines[0].old_lineno, None);
        assert_eq!(hunk.lines[1].new_lineno, Some(7));
    }

    // --- 1.3: file-level metadata (FR-diff-parse-4) ----------------------

    #[test]
    fn new_file_status_and_zero_mis_parsed_body_lines() {
        let diff = "\
diff --git a/new.rs b/new.rs
new file mode 100644
index 0000000..abc1234
--- /dev/null
+++ b/new.rs
@@ -0,0 +1,2 @@
+line1
+line2
";
        let file = parse_one(diff);
        assert_eq!(file.status, ChangeStatus::Added);
        assert_eq!(file.mode_change, None);
        assert_eq!(file.hunks.len(), 1);
        assert_eq!(file.hunks[0].lines.len(), 2);
    }

    #[test]
    fn deleted_file_status() {
        let diff = "\
diff --git a/gone.rs b/gone.rs
deleted file mode 100644
index abc1234..0000000
--- a/gone.rs
+++ /dev/null
@@ -1,2 +0,0 @@
-line1
-line2
";
        let file = parse_one(diff);
        assert_eq!(file.status, ChangeStatus::Deleted);
        assert_eq!(file.hunks.len(), 1);
        assert_eq!(file.hunks[0].lines.len(), 2);
    }

    #[test]
    fn pure_rename_with_similarity_and_zero_hunks() {
        let diff = "\
diff --git a/old.rs b/new.rs
similarity index 100%
rename from old.rs
rename to new.rs
";
        let file = parse_one(diff);
        assert_eq!(
            file.status,
            ChangeStatus::Renamed {
                similarity: Some(100)
            }
        );
        assert_eq!(file.old_path.as_deref(), Some("old.rs"));
        assert_eq!(file.path, "new.rs");
        assert_eq!(file.hunks.len(), 0);
    }

    #[test]
    fn rename_with_edits_has_hunks_and_lower_similarity() {
        let diff = "\
diff --git a/old2.rs b/new2.rs
similarity index 90%
rename from old2.rs
rename to new2.rs
index 111..222 100644
--- a/old2.rs
+++ b/new2.rs
@@ -1 +1 @@
-x
+y
";
        let file = parse_one(diff);
        assert_eq!(
            file.status,
            ChangeStatus::Renamed {
                similarity: Some(90)
            }
        );
        assert_eq!(file.hunks.len(), 1);
        assert_eq!(file.hunks[0].lines.len(), 2);
    }

    #[test]
    fn mode_only_change_is_modified_with_mode_change_and_zero_hunks() {
        let diff = "\
diff --git a/script.sh b/script.sh
old mode 100644
new mode 100755
";
        let file = parse_one(diff);
        assert_eq!(file.status, ChangeStatus::Modified);
        assert_eq!(
            file.mode_change,
            Some(("100644".to_string(), "100755".to_string()))
        );
        assert_eq!(file.hunks.len(), 0);
    }

    #[test]
    fn binary_file_carries_flag_through_with_zero_hunks() {
        let diff = "\
diff --git a/img.png b/img.png
new file mode 100644
index 0000000..abc1234
Binary files /dev/null and b/img.png differ
";
        let file = parse_one(diff);
        assert!(file.is_binary);
        assert_eq!(file.status, ChangeStatus::Added);
        assert_eq!(file.hunks.len(), 0);
    }

    // --- 1.4: no-newline marker (FR-diff-parse-5) ------------------------

    #[test]
    fn no_newline_marker_on_both_sides_sets_flag_no_extra_line() {
        let diff = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1 +1 @@
-old_last
\\ No newline at end of file
+new_last
\\ No newline at end of file
";
        let file = parse_one(diff);
        let lines = &file.hunks[0].lines;
        assert_eq!(lines.len(), 2, "backslash markers must not add Lines");
        assert!(lines[0].no_newline);
        assert!(lines[1].no_newline);
    }

    #[test]
    fn no_newline_marker_on_old_side_only() {
        let diff = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1 +1 @@
-old_last
\\ No newline at end of file
+new_last
";
        let file = parse_one(diff);
        let lines = &file.hunks[0].lines;
        assert_eq!(lines.len(), 2);
        assert!(lines[0].no_newline);
        assert!(!lines[1].no_newline);
    }

    #[test]
    fn no_newline_marker_on_new_side_only() {
        let diff = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -1 +1 @@
-old_last
+new_last
\\ No newline at end of file
";
        let file = parse_one(diff);
        let lines = &file.hunks[0].lines;
        assert_eq!(lines.len(), 2);
        assert!(!lines[0].no_newline);
        assert!(lines[1].no_newline);
    }

    // --- 1.7: parse_patches + summarize -----------------------------------

    #[test]
    fn parse_patches_and_summarize_aggregate_counts() {
        let diff = "\
diff --git a/a.rs b/a.rs
index 1..2 100644
--- a/a.rs
+++ b/a.rs
@@ -1,2 +1,2 @@
-a1
+a1b
 a2
diff --git a/b.rs b/b.rs
index 3..4 100644
--- a/b.rs
+++ b/b.rs
@@ -1 +1,2 @@
 b1
+b2
";
        let patches = split_patches(diff);
        let files = parse_patches(&patches);
        assert_eq!(files.len(), 2);

        let summary = summarize(&files);
        assert_eq!(summary.files, 2);
        assert_eq!(summary.hunks, 2);
        assert_eq!(summary.added, 2); // a1b + b2
        assert_eq!(summary.removed, 1); // a1
    }

    #[test]
    fn empty_diff_yields_empty_summary() {
        let files = parse_patches(&[]);
        assert!(files.is_empty());
        let summary = summarize(&files);
        assert_eq!(
            summary,
            DiffSummary {
                files: 0,
                hunks: 0,
                added: 0,
                removed: 0
            }
        );
    }

    // --- spec §6 edge case: CRLF / control chars preserved verbatim -------

    #[test]
    fn crlf_in_content_preserved_verbatim() {
        let diff = "diff --git a/f.rs b/f.rs\nindex 1..2 100644\n--- a/f.rs\n+++ b/f.rs\n@@ -1 +1 @@\n-old\r\n+new\r\n";
        let file = parse_one(diff);
        let lines = &file.hunks[0].lines;
        assert_eq!(lines[0].content, "old\r");
        assert_eq!(lines[1].content, "new\r");
    }

    // --- Regression: audit finding A-MED-1 — overflow on a crafted header ---

    /// A crafted hunk header starting at `u32::MAX` must not panic (debug)
    /// or silently wrap (release) when the body-line counters advance;
    /// they saturate instead, so repeated removed lines all report the
    /// same saturated line number rather than corrupting data.
    #[test]
    fn overflowing_hunk_header_start_saturates_instead_of_panicking() {
        let diff = "\
diff --git a/f.rs b/f.rs
index 1..2 100644
--- a/f.rs
+++ b/f.rs
@@ -4294967295 +1 @@
-x
-y
";
        let file = parse_one(diff);
        assert_eq!(file.hunks.len(), 1);
        let hunk = &file.hunks[0];
        assert_eq!(hunk.old_start, u32::MAX);

        let lines = &hunk.lines;
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].old_lineno, Some(u32::MAX));
        assert_eq!(
            lines[1].old_lineno,
            Some(u32::MAX),
            "counter saturates at u32::MAX rather than wrapping/panicking"
        );
    }
}
