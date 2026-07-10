# Task 6 — Syntax Highlighting Spec

## 1. Overview

Task 6 layers tree-sitter syntax highlighting onto the diff for Rust, Python,
TypeScript/TSX, and Go, composing token colors with the existing +/- coloring and
word-diff emphasis rather than replacing them. Unknown languages fall back to
plain rendering, never an error, and highlighting must not perceptibly slow
scrolling on a 5k-line diff. This completes Milestone 1.

## 2. Depends on

- Task 4's unified render and +/- coloring / word-diff emphasis — highlighting
  composes with these layers; the gutter and background-tint signals stay owned
  by the diff render.
- Task 3's `diff::{DiffFile, Hunk, Line, LineKind}` — highlighting reconstructs
  pre-image / post-image text per hunk or file from these lines.
- Task 5's cursor + viewport/visible-range machinery — reused to highlight lazily
  (visible range + margin).
- `DiffFile.path` — extension-based language detection.

## 3. Goals

- Add tree-sitter with grammars for **only** Rust, Python, TypeScript/TSX, Go.
- Detect language by file extension; unknown → plain rendering (no error path).
- Color tokens per line for both the post-image (added/context) and pre-image
  (removed) sides.
- Compose highlight foreground colors with +/- gutter + background tint and
  word-diff emphasis so all three signals remain readable on dark terminals.
- Keep scrolling instant on a 5k-line diff via lazy/cached highlighting; record
  measured numbers in the PR description.

## 4. Demoable Units of Work

### DUW 6.1 — Language detection + grammar registry

**Purpose:** Map files to grammars with a hard-coded, closed set and a safe
fallback.

- FR-hl-lang-1: The system shall map a `DiffFile.path` extension to one of
  {Rust, Python, TypeScript, TSX, Go} or to `None` (plain).
- FR-hl-lang-2: A `None` language shall render exactly as Task 4/5 did — plain,
  no error, no panic.
- FR-hl-lang-3: The grammar set shall be closed to the four languages this task;
  adding others is out of scope.

**Proof Artifacts:**
- Unit test: `.rs`→Rust, `.py`→Python, `.ts`→TypeScript, `.tsx`→TSX, `.go`→Go,
  `.md`/no-extension→None.
- Observable: a `.md` or unknown file in the diff renders plainly alongside
  highlighted code files.

### DUW 6.2 — Per-line token coloring

**Purpose:** Actually color the code.

- FR-hl-color-1: The system shall parse enough of a file's/hunk's reconstructed
  text to assign token color spans to each rendered line.
- FR-hl-color-2: Removed lines shall be highlighted from the **pre-image** text
  and added/context lines from the **post-image** text, so each side highlights
  against valid source.
- FR-hl-color-3: The chosen parse granularity (per-hunk vs. per-file of
  reconstructed text) and its known limitations (e.g. a hunk that slices through
  a multi-line construct may mis-tokenize at edges) shall be documented in a doc
  comment at the parse site. "Reconstructed text" means the concatenation of the
  relevant hunk lines per side (post-image = context + added; pre-image =
  context + removed) — NOT the full file: a diff cannot recover the unchanged
  regions between hunks, so inter-hunk gaps remain parse boundaries either way.
  Loading true full-file blobs (working tree / `git show`) would fix those
  edges but requires `git/` access, which `ui/` must not have; if ever wanted,
  `main.rs` must pre-load blob contents and pass them in. Deferred, documented.
- FR-hl-color-4: A parse failure or partial/truncated hunk shall degrade to plain
  rendering for the affected region, never error.

**Proof Artifacts:**
- Observable: a Rust file in the diff shows keyword/type/string/comment colors.
- Observable: a removed line renders highlighted (from pre-image), not just as
  flat red.
- Unit test: the highlight function returns non-empty color spans for a known
  Rust snippet and empty (plain) for an unknown language.

### DUW 6.3 — Layer composition

**Purpose:** Three signals at once — token color, add/remove, word diff.

- FR-hl-compose-1: Token colors shall apply to the foreground; add/remove shall be
  signaled by the gutter marker plus a background tint (not by overriding the
  foreground token color).
- FR-hl-compose-2: Word-diff emphasis (Task 3 `changed_spans`) shall remain
  visible on top of token coloring (e.g. via a stronger background/underline on
  the changed span).
- FR-hl-compose-3: The combined result shall be readable on a dark terminal;
  full theming is deferred but defaults must not produce illegible combinations
  (e.g. dark-on-dark).

**Proof Artifacts:**
- Observable: an edited line shows syntax colors, a red/green background tint, and
  a distinctly emphasized changed word simultaneously, all legible.
- Manual review: a dark-terminal screenshot in the PR shows the three layers
  coexisting.

### DUW 6.4 — Performance guard

**Purpose:** No scroll regression on large diffs.

- FR-hl-perf-1: The system shall highlight lazily (visible range + margin) and/or
  cache highlight results per file so a 5k-line diff does not re-highlight
  everything per frame.
- FR-hl-perf-2: Scrolling a 5k-line mixed-language diff shall remain perceptibly
  instant (no visible per-frame hitch).
- FR-hl-perf-3: The chosen strategy and before/after measurements shall be
  recorded in the PR description.

**Proof Artifacts:**
- Measurement: timing of a full scroll pass on a 5k-line diff, with and without
  the cache/lazy path, recorded in the PR.
- Observable: scrolling the large diff stays smooth.

## 5. Data Model / Key Types

```rust
/// The closed grammar set for Task 6.
pub enum Language { Rust, Python, TypeScript, Tsx, Go }

pub fn detect_language(path: &str) -> Option<Language>;

/// A colored region within a single rendered line.
pub struct HighlightSpan {
    pub range: std::ops::Range<usize>,  // char range into line content
    pub kind: TokenKind,                // maps to a color, theming later
}

/// Coarse token classes; a small closed set kept theme-mappable.
pub enum TokenKind {
    Keyword, Type, Function, String, Number, Comment, Punctuation, Ident, Other,
}

/// Per-file highlight cache; keyed so re-scroll is free.
pub struct HighlightCache {
    // e.g. file index -> per-line Vec<HighlightSpan>, lazily populated.
}
impl HighlightCache {
    /// Returns spans for one rendered line, parsing on first demand.
    pub fn line_spans(&mut self, file: usize, side: Side, line: usize)
        -> &[HighlightSpan];
}

pub enum Side { PreImage, PostImage }
```

Render composition: the widget layers, in order, (1) `TokenKind` → foreground
color, (2) `LineKind` → gutter + background tint, (3) `changed_spans` → emphasis.

## 6. Edge Cases

- Unknown / no extension → plain, no error (FR-hl-lang-2).
- Mixed-language diff in one run — each file uses its own grammar; plain files
  interleave fine.
- Removed lines — highlight from pre-image, not post-image.
- Hunk slicing through a multi-line construct (unterminated string/block comment
  at a hunk edge) — mis-tokenization tolerated and documented; must not panic.
- Binary / zero-hunk files — no highlighting attempted.
- Very long lines — highlight only the rendered (truncated) width or skip
  highlighting beyond the cut; never block the frame.
- 5k-line diff — lazy/cached path holds the scroll performance target.
- Grammar load failure at startup — degrade that language to plain, keep others.
- TSX vs. TS — `.tsx` uses the TSX grammar, `.ts` the TypeScript grammar.

## 7. Non-Goals

- No configurable themes / no light-terminal tuning — a later roadmap step; ship
  dark-readable defaults only.
- No languages beyond the four named.
- No incremental re-parse on edit (the diff is static per run); LSP-driven
  semantic highlighting is out of scope (LSP is roadmap step 5).
- No side-by-side-specific highlighting work beyond what the unified view needs.
- STOP here — Milestone 1 is complete after this task.

## 8. Testing Strategy

- **Unit (pure):** `detect_language` extension mapping; the highlight function
  returning spans for a known snippet per language and empty for unknown; span
  ranges staying within line bounds.
- **Not tested (per CLAUDE.md):** interactive rendered appearance and color
  legibility — verified by observation/screenshots.
- **Measurement (not a unit test):** the 5k-line scroll timing, reported in the
  PR, not asserted in CI (timing tests are flaky).

## 9. Open Questions

- **Parse granularity** — per-hunk reconstructed text or per-file (all of a
  file's hunk lines concatenated per side — see FR-hl-color-3's definition;
  neither option is the full blob)? *Recommended default:* per-file
  concatenation (post-image for added/context, pre-image for removed) cached
  per file — fewer edge mis-tokens than per-hunk since same-hunk-run constructs
  stay intact, and the cache makes the cost one-time. Inter-hunk gaps remain
  mis-token edges under both options; full-blob loading via a `main.rs`-provided
  content seam is the documented future fix. Document the memory tradeoff.
- **Highlight vs. word-diff conflict on a changed span** — token color or emphasis
  wins the foreground? *Recommended default:* keep token color on the foreground;
  express word-diff emphasis via background/underline so both read.
- **Color source** — a hand-picked minimal palette mapped from `TokenKind`, or a
  tree-sitter highlight-query capture-name mapping? *Recommended default:* map a
  small closed `TokenKind` set to a fixed dark palette now; richer capture-name
  theming waits for the theming roadmap step.
- **tree-sitter crate vs. per-language grammar crates** — vendored grammars or
  crates.io grammar crates? *Recommended default:* use the published grammar
  crates for the four languages; justify the dependency count in the PR per
  CLAUDE.md's lean-binary guardrail.
