# 03-tasks-diff-model

Task list for **Task 3 — Diff Model** (`docs/specs/03-spec-diff-model/03-spec-diff-model.md`).
Turns raw per-file patch strings from `git/` into a typed, navigable, pure diff
model in `diff/`. No I/O, no TUI. TDD is mandatory (the diff model is pure code —
per project CLAUDE.md, write the failing test first and commit tests with the code).

## Repo-specific execution context (binding)

- **Rust, edition 2024, stable toolchain.** Quality gates are cargo, not Node:
  - `cargo build`
  - `cargo test`
  - `cargo clippy -- -D warnings`
  - `cargo fmt --check`
  - All four MUST pass before a task is considered done. There is no ESLint /
    tsc / pnpm step — those Phase-0 sub-checks do not apply to this repo.
- **No `unwrap()` / `expect()` outside `#[cfg(test)]`.** Library errors via
  `thiserror`; `anyhow` only at the binary edge (`main.rs`). Parsing here is
  total (never panics on valid UTF-8 input) — prefer returning a best-effort
  model over panicking. `git/` guarantees valid UTF-8 upstream (`GitError::Utf8`),
  so `diff/` never sees invalid sequences.
- **Performance target:** instant feel on a 5k-line diff. Do not introduce
  per-line allocations or quadratic scans in the parser hot path.
- **Integration tests** that write build throwaway git repos in tempdirs via
  `std::process::Command` git calls — never mutate the host repo. The one
  read-only exception (diffing this repo's committed history) is explicitly
  permitted by the spec's Testing Strategy §8.
- **No new dependencies** without justification. The word-diff algorithm is
  hand-rolled over `std` (spec Open-Question default: LCS over whitespace+
  punctuation token runs). Do NOT add a diff crate. `Cargo.toml` is NOT in any
  task's scope this cycle.

## Git contract (verified present at cycle start — commit 0150343 / `git/`)

The `Depends on` section of the spec references these symbols; all confirmed to
exist in `src/git/` before task generation:

```rust
// src/git/diff.rs
pub struct RawFilePatch { pub path: String, pub old_path: Option<String>, pub raw: String, pub is_binary: bool }
pub enum DiffTarget { WorkingTree, Staged, Range(String) }
// src/git/runner.rs
impl GitRunner { pub fn diff(&self, target: &DiffTarget) -> Result<Vec<RawFilePatch>, GitError> }
```

`diff/` parses `RawFilePatch.raw`; `path` / `old_path` / `is_binary` are carried
through, never re-derived. `GitRunner::diff` is used only from `main.rs` and the
integration test, never from inside `diff/`.

## Wave Schedule

| Wave | Tasks              | Dependencies          | Parallel Agents |
| ---- | ------------------ | --------------------- | --------------- |
| 1    | 1.0                | None (zero in-degree) | 1               |
| 2    | 2.0, 3.0, 4.0      | Wave 1 completion     | 3               |

**Isolation decision (mandatory gate, per binding context #8):** file-ownership
partitioning, **NOT** worktree isolation. Rationale: the codebase is small and
`diff/` is a single module. The only conflict-prone shared file is
`src/diff/mod.rs` (module registration + re-exports). Wave 1 (T1.0) owns it
solely and pre-declares every submodule (`model`, `word`, `nav`) plus all
re-exports — creating compiling stubs for `word.rs` and `nav.rs`. Wave-2 agents
therefore fill their own file's body ONLY and never touch `mod.rs`, giving
disjoint whole-file ownership with zero shared-file seam. Worktrees add no value
at this scale and are omitted (also per `feedback-worktree-stale-base.md`: impl
agents write to main directly).

## Quality Gates (Applied After Every Parent Task)

- `cargo build` — compiles clean
- `cargo test` — all unit + integration tests pass
- `cargo clippy -- -D warnings` — zero warnings
- `cargo fmt --check` — formatting clean
- /trace: architectural compliance verified after each wave (diff/ stays pure —
  no ratatui / crossterm / git-runner types leak into the model)

## Relevant Files

- `src/diff/mod.rs` — module declarations + crate-facing re-exports (T1.0 owns)
- `src/diff/model.rs` — data types + patch parsing; inline `#[cfg(test)]` unit tests (T1.0)
- `src/diff/word.rs` — `word_diff_spans` seam + `attach_word_spans` transform; unit tests (stub by T1.0, filled by T2.0)
- `src/diff/nav.rs` — next/prev hunk & file navigation over `DiffPosition`; unit tests (stub by T1.0, filled by T3.0)
- `src/main.rs` — `run()` summary wiring (T4.0)
- `tests/diff_integration.rs` — real-diff / tempdir integration test (T4.0, new file)

### Notes

- Unit tests live in-module under `#[cfg(test)] mod tests` alongside the code
  they test (Rust convention). Integration tests live in `tests/`.
- Field names mirror git vocabulary and the existing `git/` types (spec §5).
- The data-model in spec §5 is a proposal; T1.0 may refine names but MUST keep
  the field semantics (orthogonal `mode_change` / `is_binary` vs the
  `ChangeStatus` enum) and the single `word_diff_spans` seam.

## Interface Contracts (shared across waves — Check 4)

Wave 1 (T1.0) publishes these from `diff/`; wave-2 consumers code against them
verbatim. If a wave-2 agent's reading disagrees with the spec §5 literal, the
spec wins — flag the conflict in the return.

```rust
// Core types (T1.0, src/diff/model.rs) — per spec §5, re-exported from diff/
pub struct DiffFile { pub path: String, pub old_path: Option<String>, pub status: ChangeStatus,
    pub mode_change: Option<(String, String)>, pub is_binary: bool, pub hunks: Vec<Hunk> }
pub enum ChangeStatus { Modified, Added, Deleted, Renamed { similarity: Option<u8> } }
pub struct Hunk { pub old_start: u32, pub old_count: u32, pub new_start: u32, pub new_count: u32,
    pub section: Option<String>, pub lines: Vec<Line> }
pub struct Line { pub kind: LineKind, pub old_lineno: Option<u32>, pub new_lineno: Option<u32>,
    pub content: String, pub no_newline: bool, pub changed_spans: Vec<std::ops::Range<usize>> }
pub enum LineKind { Context, Added, Removed }
pub struct DiffPosition { pub file: usize, pub hunk: usize, pub line: usize }

// Parse + summary entry points (T1.0) — consumed by T4.0
pub fn parse_patch(patch: &crate::git::RawFilePatch) -> DiffFile;
pub fn parse_patches(patches: &[crate::git::RawFilePatch]) -> Vec<DiffFile>;
pub struct DiffSummary { pub files: usize, pub hunks: usize, pub added: usize, pub removed: usize }
pub fn summarize(files: &[DiffFile]) -> DiffSummary;

// Word-diff seam (T2.0 fills; T1.0 stubs, src/diff/word.rs)
pub fn word_diff_spans(old: &str, new: &str)
    -> (Vec<std::ops::Range<usize>>, Vec<std::ops::Range<usize>>); // the ONE algorithm site
pub fn attach_word_spans(file: &mut DiffFile); // pairs lines, calls word_diff_spans exactly once per pair

// Navigation (T3.0 fills; T1.0 stubs, src/diff/nav.rs) — pure, no mutation
pub fn next_hunk(files: &[DiffFile], pos: &DiffPosition) -> Option<DiffPosition>;
pub fn prev_hunk(files: &[DiffFile], pos: &DiffPosition) -> Option<DiffPosition>;
pub fn next_file(files: &[DiffFile], pos: &DiffPosition) -> Option<DiffPosition>;
pub fn prev_file(files: &[DiffFile], pos: &DiffPosition) -> Option<DiffPosition>;
```

## Tasks

### Wave 1 (No Dependencies)

### [x] 1.0 Diff model core types + patch parsing (DUW 3.1)

**Wave:** 1 | **Agent Scope:** `src/diff/mod.rs`, `src/diff/model.rs`, `src/diff/word.rs` (compiling stub only), `src/diff/nav.rs` (compiling stub only)
**FRs:** FR-diff-parse-1, FR-diff-parse-2, FR-diff-parse-3, FR-diff-parse-4, FR-diff-parse-5
**wiring_caller:** `src/main.rs::run()` (via T4.0, Wave 2) + `src/diff/word.rs` + `src/diff/nav.rs` consume these types
**wired_path_test:** `src/diff/model.rs` `#[cfg(test)]` unit tests (this task) + `tests/diff_integration.rs` (T4.0)

**Contract (per spec §5 literal — normative):** see the Interface Contracts block
above for the exact type shapes. Carry `path` / `old_path` / `is_binary` through
from `RawFilePatch`; do not re-derive.

#### 1.0 Proof Artifact(s)

- Test: two-hunk patch yields exactly 2 `Hunk`s with the start/count from each `@@` header (FR-diff-parse-2).
- Test: an added line + a removed line in one hunk get correct disjoint line numbers; a following context line advances both sides (FR-diff-parse-3).
- Test: new-file (`--- /dev/null`), deleted-file (`+++ /dev/null`), mode-only, and rename fixtures each produce the expected `ChangeStatus`, correct `mode_change`, and zero mis-parsed body lines. A mode-only change is `Modified` with `mode_change: Some(..)` and zero hunks (FR-diff-parse-4).
- Test: a fixture ending in `\ No newline at end of file` sets `no_newline` on the last content line and adds no extra `Line` (FR-diff-parse-5).
- CLI: `cargo test diff::model` — all green.

#### 1.0 Quality Verification

- [x] `cargo build` clean
- [x] `cargo test` — all passing
- [x] `cargo clippy -- -D warnings` — zero warnings
- [x] `cargo fmt --check` clean
- [x] /trace: `diff/` imports no ratatui/crossterm/git-runner types; model is pure data

#### 1.0 Tasks

- [x] 1.1 (test-first) Write failing unit tests in `src/diff/model.rs` for the two-hunk header parse (FR-diff-parse-2), including the retained trailing section-heading text after `@@`, and the omitted-count case `@@ -3 +3 @@` (absent count defaults to 1, spec §6).
- [x] 1.2 (test-first) Write failing tests for line classification + line-number assignment (FR-diff-parse-3): context carries both `old_lineno`/`new_lineno`; added carries only new; removed carries only old; a zero-context hunk (count 0 on a side) parses correctly.
- [x] 1.3 (test-first) Write failing tests for file-level metadata (FR-diff-parse-4): new file, deleted file, rename-with-similarity (`ChangeStatus::Renamed { similarity }`), rename-with-edits (similarity < 100, hunks present), mode-only change (`Modified`, `mode_change: Some`, zero hunks), and binary pass-through (`is_binary` carried, zero hunks, body not parsed). Metadata lines are NOT emitted as body `Line`s.
- [x] 1.4 (test-first) Write failing test for the `\ No newline at end of file` marker (FR-diff-parse-5) — flag on the preceding line, on old side / new side / both; no extra `Line` emitted.
- [x] 1.5 Define the data types in `src/diff/model.rs` per spec §5 (`DiffFile`, `ChangeStatus`, `Hunk`, `Line`, `LineKind`, `DiffPosition`). Derive `Debug, Clone, PartialEq, Eq` to match `git/` conventions and enable test asserts.
- [x] 1.6 Implement the parser `parse_patch(&RawFilePatch) -> DiffFile` (FR-diff-parse-1..5): walk the raw patch, classify header/metadata lines, parse `@@ -a,b +c,d @@` headers (handle omitted counts → default 1), assign old/new line numbers, set `no_newline`, detect `ChangeStatus` + `mode_change`. Leave `changed_spans` empty (populated later by the word module). Total function — never panics on valid UTF-8; treat unexpected shapes as best-effort context lines.
- [x] 1.7 Add `parse_patches(&[RawFilePatch]) -> Vec<DiffFile>` and `summarize(&[DiffFile]) -> DiffSummary { files, hunks, added, removed }` (added = count of `LineKind::Added`, removed = count of `LineKind::Removed`) for the T4.0 summary line.
- [x] 1.8 Create compiling stubs `src/diff/word.rs` (`word_diff_spans` returning `(Vec::new(), Vec::new())`, `attach_word_spans` a no-op) and `src/diff/nav.rs` (four fns returning `None`). Stubs MUST be clippy-clean (silence unused params via `let _ = ...;`, do not add `#[allow(dead_code)]`). Bodies are filled in Wave 2.
- [x] 1.9 Wire `src/diff/mod.rs`: `pub mod model; pub mod word; pub mod nav;` and `pub use` re-exports of every public symbol in the Interface Contracts block, so the crate root (`diff::DiffFile`, `diff::parse_patches`, `diff::next_hunk`, …) resolves. Confirm `cargo build` + all four gates green with the stubs in place.

---

### Wave 2 (Requires Wave 1)

### [x] 2.0 Intra-line word diff (DUW 3.2)

**Wave:** 2 | **Agent Scope:** `src/diff/word.rs` (fill body only — do NOT touch `mod.rs`)
**FRs:** FR-diff-word-1, FR-diff-word-2, FR-diff-word-3
**wiring_caller:** `attach_word_spans` calls `word_diff_spans` (the single algorithm site); production consumption by `ui/` emphasis is **deferred — see spec `04-spec-first-render` (Task 4)** per spec §7 Non-Goals.
**wired_path_test:** `src/diff/word.rs` `#[cfg(test)]` unit tests (this task)

**Open-Question defaults (spec §9, apply unless spec text overrides):** token
granularity = split on whitespace **and** punctuation runs (identifiers stay
whole; `foo.bar()` breaks at `.` / `(`), LCS over those tokens; span index units
= **char** offsets into `content` (so `ui/` slices without UTF-8 boundary math).

#### 2.0 Proof Artifact(s)

- Test: `-let key = foo;` / `+let key = bar;` yields a single span covering `foo` on the old line and `bar` on the new line, shared prefix/suffix excluded (FR-diff-word-1/2).
- Test: an unpaired added line (lone addition) has empty `changed_spans`; likewise lone deletion, context, and identical pairs (FR-diff-word-3).
- Test: `word_diff_spans` is called from exactly one site (`attach_word_spans`), so swapping the algorithm body leaves callers unchanged — assert the seam by structure/comment + a test that only exercises it through `attach_word_spans` (FR-diff-word-1 proof).

#### 2.0 Quality Verification

- [x] `cargo build` clean
- [x] `cargo test` — all passing
- [x] `cargo clippy -- -D warnings` — zero warnings
- [x] `cargo fmt --check` clean
- [x] /trace: `word_diff_spans` is the ONLY place the intra-line algorithm lives; results are `Range`s on `Line`, never pre-styled text

#### 2.0 Tasks

- [x] 2.1 (test-first) Write failing tests for the `foo`→`bar` single-span case, the unpaired/identical empty-span cases, and the "excess `|N-M|` unpaired lines" pairing rule (git emits N removed then M added; i-th removed pairs with i-th added; excess stays unpaired) — FR-diff-word-1/3.
- [x] 2.2 Implement `word_diff_spans(old, new) -> (old_spans, new_spans)` as char-range `Range<usize>` output via LCS over whitespace+punctuation token runs (spec §9 default). This is the single algorithm seam — no other function may compute spans.
- [x] 2.3 Implement `attach_word_spans(&mut DiffFile)`: within each contiguous change run in every hunk, positionally pair removed/added lines, call `word_diff_spans` once per pair, and store the returned char ranges into `Line.changed_spans` on both lines. Leave unpaired lines and context/identical pairs with empty `changed_spans` (FR-diff-word-3).
- [x] 2.4 Add FR-ID traceability comments (`// FR-diff-word-N`) at the implementing sites.

### [x] 3.0 Navigation primitives (DUW 3.3)

**Wave:** 2 | **Agent Scope:** `src/diff/nav.rs` (fill body only — do NOT touch `mod.rs`)
**FRs:** FR-diff-nav-1, FR-diff-nav-2, FR-diff-nav-3
**wiring_caller:** production key-binding of these moves is **deferred — see spec `04-spec-first-render` / `05-spec-navigation-sidebar` (Tasks 4-5)** per spec DUW 3.3 purpose ("moves `ui/` will bind in Tasks 4-5").
**wired_path_test:** `src/diff/nav.rs` `#[cfg(test)]` unit tests (this task)

#### 3.0 Proof Artifact(s)

- Test: from the last hunk of file 0 in a 2-file model, `next_hunk` lands on the first hunk of file 1 (crosses the file boundary) — FR-diff-nav-1.
- Test: `prev_file` from file 0 returns `None`; `next_hunk`/`prev_hunk` at the model ends return `None` — FR-diff-nav-1/2.
- Test: zero-hunk files (binary / pure rename / mode-only) are SKIPPED by `next_hunk`/`prev_hunk` but LANDED ON by `next_file`/`prev_file` (their header is a valid position with no line component) — spec §6 edge case.

#### 3.0 Quality Verification

- [x] `cargo build` clean
- [x] `cargo test` — all passing
- [x] `cargo clippy -- -D warnings` — zero warnings
- [x] `cargo fmt --check` clean
- [x] /trace: navigation fns are pure (take `&[DiffFile]` + `&DiffPosition`, return `Option<DiffPosition>`, no mutation)

#### 3.0 Tasks

- [x] 3.1 (test-first) Write failing tests for the cross-file `next_hunk`, the `None`-at-ends cases, and the zero-hunk-file skip/land behavior (FR-diff-nav-1/2 + spec §6).
- [x] 3.2 Implement `next_hunk` / `prev_hunk` over `DiffPosition` (across file boundaries; skip zero-hunk files; `None` at the ends) — FR-diff-nav-1.
- [x] 3.3 Implement `next_file` / `prev_file` returning the first position of the adjacent file (landing on zero-hunk files; `None` at the ends) — FR-diff-nav-2.
- [x] 3.4 Keep all four functions pure over the stable index/position type (no mutation of the model) and add `// FR-diff-nav-N` traceability comments — FR-diff-nav-3.

### [x] 4.0 main.rs summary wiring + real-diff integration (DUW 3.4)

**Wave:** 2 | **Agent Scope:** `src/main.rs`, `tests/diff_integration.rs` (new file) — do NOT touch any `src/diff/` file
**FRs:** FR-diff-wire-1, FR-diff-wire-2
**wiring_caller:** this task IS the production wiring — `main.rs::run()` calls `diff::parse_patches` + `diff::summarize`.
**wired_path_test:** `tests/diff_integration.rs` (this task)

**Contract consumed (from T1.0):** `diff::parse_patches(&[RawFilePatch]) -> Vec<DiffFile>` and `diff::summarize(&[DiffFile]) -> DiffSummary { files, hunks, added, removed }`. Does NOT depend on T2.0/T3.0 — the summary needs only parsing + counts, so this task runs in parallel with them.

#### 4.0 Proof Artifact(s)

- Observable: `cargo run` in a dirty repo prints e.g. `3 files, 7 hunks, +42 -18` instead of the Task-2 per-file listing (FR-diff-wire-1).
- Integration test: `tests/diff_integration.rs` builds a throwaway tempdir repo (or diffs a known committed range of THIS repo, read-only), parses every `RawFilePatch` via `diff::parse_patches`, and asserts no panic with file/hunk counts > 0 (FR-diff-wire-2).
- CLI: `cargo test --test diff_integration` — green.

#### 4.0 Quality Verification

- [x] `cargo build` clean
- [x] `cargo test` — all passing (incl. the new integration test)
- [x] `cargo clippy -- -D warnings` — zero warnings
- [x] `cargo fmt --check` clean
- [x] /trace: `GitRunner::diff` is called only from `main.rs`/tests, never from inside `diff/`

#### 4.0 Tasks

- [x] 4.1 (test-first) Write `tests/diff_integration.rs`: build a throwaway git repo in a tempdir via `std::process::Command` git calls (init, write files, commit, edit), run `GitRunner::diff`, `diff::parse_patches` the result, assert no panic and `summarize(...).files > 0 && .hunks > 0`. (A read-only diff of this repo's own committed history — e.g. `git diff <sha>^ <sha>` — is the permitted host-repo exception per spec §8; any test that WRITES must use a tempdir.) Test fails until 4.2 wiring lands / compiles.
- [x] 4.2 Replace the per-file placeholder loop in `main.rs::run()` with a single summary line built from `diff::parse_patches(&patches)` + `diff::summarize(&files)`: `println!("{} files, {} hunks, +{} -{}", s.files, s.hunks, s.added, s.removed)`. Preserve the working-tree untracked-file handling only if it still makes sense for a summary (otherwise fold untracked count into `files`); keep `run()` returning `anyhow::Result<()>` and free of `unwrap()`/`expect()`.
- [x] 4.3 Add `// FR-diff-wire-1` / `// FR-diff-wire-2` traceability comments.

## Post-Generation Verification (recorded at task-gen time)

- **Check 1 — Requirement coverage:** all 13 FR-IDs mapped — parse-1..5 → T1.0; word-1..3 → T2.0; nav-1..3 → T3.0; wire-1..2 → T4.0. Spec defines **no** ERR-IDs. Every spec §6 edge case is covered by a named test (omitted counts, zero-context hunks, no-newline both sides, rename-with-edits, mode-only, binary, zero-hunk navigation, empty diff).
- **Check 3 — Agent scope size:** max 4 files (T1.0, two of them trivial stubs); all others ≤ 2 files. Well under the 6-file / 15-subtask split thresholds.
- **Check 4 — Interface contract extraction:** the producer-consumer seams (T1.0 → T2.0/T3.0/T4.0) are pinned verbatim in the Interface Contracts block and echoed in each consumer task.
- **Check 6d — Path/symbol grounding:** `src/diff/mod.rs`, `src/main.rs`, `src/lib.rs` (`pub mod diff`), and the `git/` symbols were all confirmed present via grep/read before locking task bodies. `tests/diff_integration.rs` is a new file (noted). No cited path is a phantom.
- **Check 9 — build-task wiring:** T1.0's model is wired by T4.0 (production) + unit/integration tests; T2.0/T3.0 primitives are wired into the model/tests now, with production `ui/` consumption explicitly deferred to the already-authored specs 04/05 (deferral form satisfied — the named future tasks exist).
