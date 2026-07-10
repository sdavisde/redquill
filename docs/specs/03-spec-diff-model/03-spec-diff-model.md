# Task 3 — Diff Model Spec

## 1. Overview

Task 3 turns the raw per-file patch strings produced by `git/` into a typed,
navigable diff model living in `diff/`. It is pure data plus transforms — no
I/O, no TUI, heavily unit-tested — and is the substrate every later rendering,
navigation, staging, and annotation task builds on. Definition of done: it
parses this repo's own real diffs without panicking and all four CI gates pass.

## 2. Depends on

- `git::RawFilePatch { path, old_path, raw, is_binary }` — the per-file input.
  Task 3 parses the `raw` string; `path`/`old_path`/`is_binary` are carried
  through so the model doesn't re-derive what `git/` already resolved.
- `git::DiffTarget` — carried on the parsed set for context (which target the
  model represents), not re-interpreted here.
- `git::GitRunner::diff` — the source of `RawFilePatch`es; used only from
  `main.rs` and integration tests, never from inside `diff/`.

## 3. Goals

- Parse every `RawFilePatch.raw` into `DiffFile → Hunk → Line` covering all the
  header/hunk shapes git emits (multi-hunk, no-newline marker, mode change,
  rename with similarity, new/deleted).
- Attach word-level changed spans to paired removed/added lines via a single
  swappable function.
- Provide pure next/prev hunk and next/prev file navigation over a position.
- Achieve round-trip: re-serializing a parsed simple file reproduces its patch
  body byte-for-byte (excluding intentionally normalized cases, see Edge Cases).
- Replace `main.rs`'s Task 2 placeholder with a parsed summary (file / hunk /
  +/- counts) driven by this model.

## 4. Demoable Units of Work

### DUW 3.1 — Header and hunk parsing

**Purpose:** Convert one raw patch into a `DiffFile` with its hunks and lines,
preserving old/new line numbers and per-line kind.

- FR-diff-parse-1: The system shall parse a `RawFilePatch` into a `DiffFile`
  carrying `path`, `old_path`, `is_binary`, a `ChangeStatus`, and an ordered
  list of `Hunk`s.
- FR-diff-parse-2: The system shall parse each `@@ -a,b +c,d @@` header into a
  `Hunk` recording old-start/old-count and new-start/new-count, and shall
  retain the header's trailing section-heading text when present.
- FR-diff-parse-3: The system shall classify each body line as
  `Context`, `Added`, or `Removed`, and shall assign correct `old_lineno` /
  `new_lineno` values (context lines carry both; added carry only new; removed
  carry only old).
- FR-diff-parse-4: The system shall detect and represent file-level metadata —
  new file, deleted file, and rename (with similarity index) as a
  `ChangeStatus`; mode change as the orthogonal `mode_change` field; binary as
  the carried-through `is_binary` flag — without treating metadata lines as
  body lines. A mode-only change is `Modified` with `mode_change: Some(..)` and
  zero hunks.
- FR-diff-parse-5: The system shall represent a `\ No newline at end of file`
  marker as a flag on the preceding line rather than as its own `Line`.

**Proof Artifacts:**
- Unit test: a two-hunk patch yields 2 `Hunk`s with the exact start/count from
  each `@@` header.
- Unit test: an added line and a removed line in one hunk get correct disjoint
  line numbers; a following context line's numbers advance both sides.
- Unit test: new-file, deleted-file, mode-only, and rename fixtures each produce
  the expected `ChangeStatus` and zero mis-parsed body lines.
- Unit test: a fixture ending in `\ No newline at end of file` sets the flag on
  the last content line and adds no extra line.

### DUW 3.2 — Intra-line word diff

**Purpose:** For paired removed/added lines, compute word-level changed spans so
`ui/` can emphasize what actually changed.

- FR-diff-word-1: The system shall pair removed/added lines positionally within
  each contiguous change run (git emits N removed lines followed by M added
  lines; the i-th removed pairs with the i-th added; the excess `|N-M|` lines
  stay unpaired) and compute changed character-range spans on both lines of
  each pair using a single function (`word_diff_spans` or equivalent) that is
  the only place the diff algorithm lives.
- FR-diff-word-2: The system shall express results as byte/char `Range`s stored
  on the `Line` (`changed_spans: Vec<Range<usize>>`), never as pre-styled text.
- FR-diff-word-3: The system shall leave `changed_spans` empty for unpaired
  lines (lone additions, lone deletions, context) and for identical pairs.

**Proof Artifacts:**
- Unit test: `-let key = foo;` / `+let key = bar;` yields a single span covering
  `foo` on the old line and `bar` on the new line, with the shared prefix/suffix
  excluded.
- Unit test: an unpaired added line has empty `changed_spans`.
- Unit test: swapping the algorithm behind `word_diff_spans` (asserted by it
  being called from exactly one site) leaves callers unchanged.

### DUW 3.3 — Navigation primitives

**Purpose:** Pure lookups for the moves `ui/` will bind in Tasks 4–5.

- FR-diff-nav-1: The system shall, given a `DiffPosition`, return the position
  of the next / previous hunk (across file boundaries), or `None` at the ends.
- FR-diff-nav-2: The system shall, given a `DiffPosition`, return the first
  position of the next / previous file, or `None` at the ends.
- FR-diff-nav-3: Navigation functions shall be pure (no mutation of the model)
  and defined over a stable index/position type.

**Proof Artifacts:**
- Unit test: from the last hunk of file 0 in a 2-file model, `next_hunk` lands
  on the first hunk of file 1.
- Unit test: `prev_file` from file 0 returns `None`.

### DUW 3.4 — main.rs summary wiring + real-diff integration

**Purpose:** Prove the model against reality and give the operator a visible
signal before any TUI exists.

- FR-diff-wire-1: The system shall replace `main.rs`'s per-file placeholder with
  a summary line reporting total file count, total hunk count, and aggregate
  added/removed line counts, computed from the parsed model.
- FR-diff-wire-2: The system shall parse a real diff drawn from this repo's own
  history (an integration test running `git` in a tempdir clone / against a real
  commit range) without panicking.

**Proof Artifacts:**
- Integration test: build a throwaway repo (or diff a known commit range),
  parse every `RawFilePatch`, assert no panic and file/hunk counts > 0.
- Observable: `cargo run` in a dirty repo prints e.g. `3 files, 7 hunks,
  +42 -18` instead of the Task 2 listing.

## 5. Data Model / Key Types

Proposals — the operator may rename/reshape. Field names mirror git vocabulary
and the existing `git/` types.

```rust
/// One file's parsed diff: metadata plus ordered hunks.
pub struct DiffFile {
    pub path: String,
    pub old_path: Option<String>,
    pub status: ChangeStatus,
    /// Orthogonal to `status`: a renamed or modified file may also change mode.
    pub mode_change: Option<(String, String)>, // (old_mode, new_mode)
    /// Orthogonal to `status`: binary-ness comes from `git/`; a binary file is
    /// still Added/Modified/Deleted. Binary files carry zero hunks.
    pub is_binary: bool,
    pub hunks: Vec<Hunk>,
}

/// File-level change classification, richer than a porcelain letter.
/// Deliberately excludes binary and mode-change — those are orthogonal flags
/// on `DiffFile` (they co-occur with every variant), not statuses.
pub enum ChangeStatus {
    Modified,
    Added,
    Deleted,
    Renamed { similarity: Option<u8> },
}

/// One `@@ ... @@` region.
pub struct Hunk {
    pub old_start: u32,
    pub old_count: u32,
    pub new_start: u32,
    pub new_count: u32,
    /// Text after the closing `@@` (function/section context), if any.
    pub section: Option<String>,
    pub lines: Vec<Line>,
}

pub struct Line {
    pub kind: LineKind,
    pub old_lineno: Option<u32>,
    pub new_lineno: Option<u32>,
    pub content: String,             // without the leading +/-/space marker
    pub no_newline: bool,            // preceded a `\ No newline` marker
    /// Word-diff spans over `content` (char indices). Empty unless paired.
    pub changed_spans: Vec<std::ops::Range<usize>>,
}

pub enum LineKind {
    Context,
    Added,
    Removed,
}

/// A cursor into the model for navigation. Indices are stable for a given
/// parsed set.
pub struct DiffPosition {
    pub file: usize,
    pub hunk: usize,
    pub line: usize,
}
```

The single word-diff seam:

```rust
/// The ONLY place the intra-line algorithm lives; swap the body freely.
/// Returns (old_spans, new_spans) as char ranges.
fn word_diff_spans(old: &str, new: &str)
    -> (Vec<std::ops::Range<usize>>, Vec<std::ops::Range<usize>>);
```

## 6. Edge Cases

- `\ No newline at end of file` — flag on preceding line; may appear on old
  side, new side, or both.
- New file (`--- /dev/null`) and deleted file (`+++ /dev/null`).
- Pure rename with no body (`similarity index 100%`, zero hunks).
- Rename **with** edits (similarity < 100%, hunks present).
- Mode-only change (`old mode` / `new mode`, no hunks).
- Binary (`is_binary` already set by `git/`) — pass through with zero hunks;
  do not attempt to parse a body.
- Multiple hunks in one file; hunks with zero context (count of 0 on a side).
- Omitted hunk counts — `@@ -3 +3 @@` is legal; an absent count defaults to 1.
- Zero-hunk files (binary, pure rename, mode-only) under navigation —
  `next_hunk`/`prev_hunk` skip them; `next_file`/`prev_file` still land on them
  (their header is a valid position with no line component).
- Empty diff (no files) and empty hunk lists.
- Input is guaranteed valid UTF-8: `git/` decodes strictly and surfaces invalid
  bytes as `GitError::Utf8` upstream, so `diff/` never sees invalid sequences.
  (Lossy decoding at the `git/` boundary is a possible future hardening item,
  out of scope here.)
- CRLF / embedded control chars in content — preserved verbatim in `content`.
- Very long lines — no truncation in the model (rendering handles width).
- `@@@` combined-diff (merge) headers — out of scope; document that Task 3
  assumes two-way diffs only (see Non-Goals).

## 7. Non-Goals

- No ratatui, no rendering, no color — STOP before any TUI. Coloring and word-
  span emphasis are Task 4.
- No syntax highlighting or token model — Task 6.
- No staging or annotation state on the model — Tasks 2/3 of the roadmap
  (staging, annotations) own those.
- No combined/merge (`diff --cc`) parsing this task.
- No side-by-side layout transforms — Task 4/roadmap step 4.

## 8. Testing Strategy

- **Unit (TDD, primary):** all parsing, word-diff, and navigation logic against
  inline raw-string fixtures; write the failing test first per CLAUDE.md. Cover
  every Edge Case bullet with at least one fixture.
- **Property:** round-trip re-serialization for simple modified files (parse →
  emit → compare body). Renames/mode-only/no-newline cases may be documented as
  normalized and excluded from the strict round-trip.
- **Integration:** one test that parses a real diff from this repo's own
  history (e.g. `git diff <sha>^ <sha>` over an early commit) asserting no
  panic and sane aggregate counts. Note on CLAUDE.md's "never test against the
  host repo" rule: its intent is to forbid *mutating* the host repo; this test
  is read-only (diff of committed history) and is explicitly required by the
  task's definition of done. Any test that writes still uses a tempdir repo.
- **Not tested:** nothing interactive exists yet; `main.rs`'s printed summary is
  covered indirectly by the integration test's count assertions.

## 9. Open Questions

- **Word-diff token granularity** — split on whitespace only, or whitespace +
  punctuation, or Unicode word boundaries? *Recommended default:* split on
  whitespace and punctuation runs (identifiers stay whole, `foo.bar()` breaks at
  `.`/`(`), LCS over those tokens. Cheap and readable; revisit if noisy.
- **Span index units** — byte offsets or char offsets into `content`? *Recommended
  default:* char offsets, so `ui/` can slice without UTF-8 boundary math.
- **Round-trip strictness** — must renames/mode-only round-trip too, or only
  modified files? *Recommended default:* only plain modified files round-trip
  strictly; others assert structural equality, not byte equality.
- **ChangeStatus vs. flags** — resolved in the sketch above: enum for the
  primary status (Modified/Added/Deleted/Renamed) plus orthogonal
  `mode_change: Option<(String, String)>` and `is_binary` fields, so a
  renamed-and-chmod'd binary file is representable. Reopen only if you'd
  rather collapse everything into one richer enum.
