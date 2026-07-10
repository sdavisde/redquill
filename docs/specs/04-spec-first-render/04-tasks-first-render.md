# 04-tasks-first-render

Task list for **Task 4 — First Render** (`docs/specs/04-spec-first-render/04-spec-first-render.md`).
Brings the parsed diff on screen: adds ratatui + crossterm, a panic-safe terminal
guard, a data-driven keymap, and a scrollable colored unified-diff render, wired
from `main.rs` into a clean `ui/` entry point. Scope is deliberately minimal —
scroll and quit only (spec §1). Interactive rendering itself is NOT unit-tested
(spec §8 / project CLAUDE.md); the pure layers (keymap resolution, layout/scroll
helpers, guard teardown) ARE test-first.

## Repo-specific execution context (binding)

- **Rust, edition 2024, stable toolchain.** Quality gates are cargo, not Node:
  - `cargo build`
  - `cargo test`
  - `cargo clippy -- -D warnings`
  - `cargo fmt --check`
  - All four MUST pass before a task is considered done. There is no ESLint /
    tsc / pnpm step — those Phase-0 sub-checks do not apply to this repo.
- **No `unwrap()` / `expect()` outside `#[cfg(test)]`.** `ui/` is library code
  (lives under `src/lib.rs`), so it returns errors via **`thiserror`** (a new
  `ui::UiError` wrapping `std::io::Error`); `anyhow` stays at the binary edge
  (`main.rs`), which converts `UiError` via `?`. crossterm/ratatui I/O calls that
  can fail must propagate, never `.unwrap()`.
- **New dependencies — ratatui + crossterm, pre-approved.** These two (and ONLY
  these two) are added this cycle. They are named in the project CLAUDE.md stack
  section and README Architecture as the decided TUI stack, and are the explicit
  subject of spec §3/§Goals. The single commit that adds them MUST justify them
  per the CLAUDE.md dependency guardrail (one-line rationale: "ratatui+crossterm
  are the decided TUI stack, spec 04 §3; crossterm backend only"). No other new
  dependency without a blocker. `Cargo.toml` is owned solely by T1.0 (Wave 1).
- **Performance target:** instant feel on a 5k-line diff. The render path MUST
  slice only the visible row range each frame (spec §6 "very large diff"), never
  build/style the whole buffer per frame. Render **on event** (block on the next
  key/resize), never a busy loop (FR-render-view-5).
- **crossterm backend only** (spec §9 Open-Question default): `ratatui::backend::
  CrosstermBackend<Stdout>`. No termion / other backends.
- **FR-render-keymap-5 invariant (cross-cutting — applies to EVERY task below).**
  All default bindings are provisional; the operator will tune them by feel, so a
  rebind must stay a one-line change. Therefore: **no widget, prompt string, doc
  comment, status line, or empty-state message may hardcode a key name.** Anything
  that displays or reacts to a key derives it from the `Keymap` (reverse-lookup an
  `Action` → its bound chord for display; forward-resolve a `KeyEvent` → `Action`
  for behavior). Changing a default binding must require editing exactly one entry
  in `Keymap::default_map()` (plus the README table row and at most one default-map
  test assertion) — nothing else.

## Depends-on contract (verified present at cycle start — HEAD `e3d5bf1` / `diff/` + `git/`)

The spec §2 "Depends on" symbols were confirmed by grep/read in `src/` before task
generation (content-anchored, landed by spec 03):

```rust
// src/diff/model.rs — re-exported from `diff` (src/diff/mod.rs)
pub struct DiffFile { pub path: String, pub old_path: Option<String>, pub status: ChangeStatus,
    pub mode_change: Option<(String, String)>, pub is_binary: bool, pub hunks: Vec<Hunk> }
pub enum ChangeStatus { Modified, Added, Deleted, Renamed { similarity: Option<u8> } }
pub struct Hunk { pub old_start: u32, pub old_count: u32, pub new_start: u32, pub new_count: u32,
    pub section: Option<String>, pub lines: Vec<Line> }
pub struct Line { pub kind: LineKind, pub old_lineno: Option<u32>, pub new_lineno: Option<u32>,
    pub content: String, pub no_newline: bool, pub changed_spans: Vec<std::ops::Range<usize>> }
pub enum LineKind { Context, Added, Removed }
pub fn parse_patches(patches: &[crate::git::RawFilePatch]) -> Vec<DiffFile>; // src/diff
// src/git — re-exported from `git` (src/git/mod.rs)
pub struct GitRunner; impl GitRunner { pub fn discover() -> Result<Self, GitError>;
    pub fn diff(&self, target: &DiffTarget) -> Result<Vec<RawFilePatch>, GitError> }
pub enum DiffTarget { WorkingTree, Staged, Range(String) }
// src/main.rs — existing Config + run() (prints a summary today; T4.0 routes it into ui/)
```

`ui/` renders the ALREADY-parsed `Vec<DiffFile>` and its `Line.changed_spans`; it
never calls `git/` (FR-render-wire-1). `main.rs` is the only site that touches
`GitRunner`.

## Wave Schedule

| Wave | Tasks     | Dependencies          | Parallel Agents |
| ---- | --------- | --------------------- | --------------- |
| 1    | 1.0       | None (zero in-degree) | 1               |
| 2    | 2.0, 3.0  | Wave 1 completion     | 2               |
| 3    | 4.0       | Wave 2 completion     | 1               |

**Isolation decision (mandatory gate — file-ownership partitioning, NOT worktrees).**
Full rationale + feedback source in `pipeline-state/wave-isolation-decision.md`.
Summary: the three conflict-prone shared files are single-agent-owned across the
schedule — **`Cargo.toml`** (T1.0, Wave 1, sole writer — first-to-add-deps-wins),
**`src/ui/mod.rs`** (T1.0, Wave 1, sole writer — module registration + re-exports +
`UiError`; never touched again because the `run` entry point body lives in
`app.rs`), and **`src/main.rs`** (T4.0, Wave 3, sole writer). Wave 2's two agents
fill disjoint whole files (`keymap.rs`, `terminal.rs`) and never touch `mod.rs`.
No wave has two agents contending for any file. Worktrees are omitted per
`memory/feedback-worktree-stale-base.md` (impl agents write to main directly;
stale-base risk on a fast-moving small module outweighs isolation benefit).

## Quality Gates (Applied After Every Parent Task)

- `cargo build` — compiles clean
- `cargo test` — all unit + integration tests pass
- `cargo clippy -- -D warnings` — zero warnings
- `cargo fmt --check` — formatting clean
- /trace after each wave: `ui/` imports no `git` types (FR-render-wire-1);
  key-matching lives only in the `Keymap` (no `match ev.code` arms in widgets);
  no hardcoded key names outside `default_map()` (FR-render-keymap-5).

## Relevant Files

- `Cargo.toml` — add `ratatui` + `crossterm` (T1.0 owns solely)
- `src/ui/mod.rs` — module registration, `UiError`, crate-facing re-exports (T1.0 owns solely)
- `src/ui/keymap.rs` — `Action`, `KeyChord`, `Keymap`; in-module unit tests (stub by T1.0, filled by T2.0)
- `src/ui/terminal.rs` — `TerminalGuard`, `restore_terminal`; teardown unit test (stub by T1.0, filled by T3.0)
- `src/ui/app.rs` — `App`, `run` entry point, pure layout helpers, event/render loop; in-module unit tests (stub by T1.0, filled by T4.0)
- `src/main.rs` — route `run()` into `ui::run(files)` (T4.0 owns solely)

### Notes

- Unit tests live in-module under `#[cfg(test)] mod tests` (Rust convention).
  Interactive rendering itself is NOT unit-tested (spec §8) — only the pure layers.
- Spec §5 data-model shapes are normative for field semantics; T1.0 may refine
  names but MUST keep: `Keymap` bindings as plain data (no closures/trait objects
  in the map — remap-readiness, spec §5 note), `App.scroll` as `usize` (not `u16`),
  and the `resolve(&self, ev) -> Action` signature.
- `gd`/`gr` are two-key chords in the README; per spec §9 default they are
  registered as **placeholder single-entry no-ops** this task (real chord
  sequencing deferred to Task 5). `K` (Hover) is a real single-key no-op binding.
- Arrow/page aliases (`Down`/`Up` → scroll, `PgDn`/`PgUp` → half-page) are added
  as **extra `default_map()` entries** per spec §9 default (aliases of existing
  actions, zero extra code — they are not new features).

## Interface Contracts (shared across waves — Check 4)

Wave 1 (T1.0) publishes these frozen signatures from `ui/`; Wave 2/3 consumers code
against them verbatim. If a consumer's reading disagrees with the spec §5 literal,
the spec wins — flag the conflict in the return.

```rust
// src/ui/mod.rs (T1.0) — library error + registration + re-exports
pub mod terminal; pub mod keymap; pub mod app;
pub use app::{App, run};
pub use keymap::{Action, KeyChord, Keymap};
pub use terminal::{TerminalGuard, restore_terminal};

#[derive(Debug, thiserror::Error)]
pub enum UiError {
    #[error("terminal I/O error: {0}")]
    Io(#[from] std::io::Error),
}

// src/ui/keymap.rs (T1.0 stubs shapes; T2.0 fills bodies)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action { ScrollDown, ScrollUp, HalfPageDown, HalfPageUp,
    NextHunk, PrevHunk, NextFile, PrevFile,           // no-op this task
    Search, SearchNext, SearchPrev,                   // no-op
    Comment, VisualSelect, StageToggle, ToggleStagePanel, // no-op
    GotoDefinition, FindReferences, Hover,            // no-op
    AnnotationList, Help,                             // no-op
    Quit, QuitDiscard, Noop }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyChord { pub code: crossterm::event::KeyCode, pub modifiers: crossterm::event::KeyModifiers }
impl KeyChord { pub fn from_event(ev: crossterm::event::KeyEvent) -> Self; }
pub struct Keymap { bindings: std::collections::HashMap<KeyChord, Action> } // plain data — no closures
impl Keymap {
    pub fn default_map() -> Self;                                  // README draft bindings + aliases
    pub fn resolve(&self, ev: crossterm::event::KeyEvent) -> Action; // unbound → Action::Noop
    pub fn chord_for(&self, action: Action) -> Option<KeyChord>;   // reverse lookup for display (FR-render-keymap-5)
}

// src/ui/terminal.rs (T1.0 stubs; T3.0 fills)
pub struct TerminalGuard { /* owns Terminal<CrosstermBackend<Stdout>> */ }
impl TerminalGuard {
    pub fn enter() -> Result<Self, crate::ui::UiError>;           // raw mode + alt screen + panic hook
    pub fn terminal_mut(&mut self) -> &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>;
}
impl Drop for TerminalGuard { /* calls restore_terminal() */ }
pub fn restore_terminal();  // idempotent: disable raw mode + leave alt screen; callable without a live TTY

// src/ui/app.rs (T1.0 stubs `run` + helpers; T4.0 fills)
pub struct App { files: Vec<crate::diff::DiffFile>, scroll: usize, should_quit: bool }
pub fn run(files: Vec<crate::diff::DiffFile>) -> Result<(), crate::ui::UiError>; // the ui/ entry point
// pure layout helpers (unit-tested in-module):
pub fn clamp_scroll(offset: usize, total_rows: usize, viewport: usize) -> usize;
pub fn visible_range(scroll: usize, total_rows: usize, viewport: usize) -> std::ops::Range<usize>;
pub fn gutter_marker(kind: crate::diff::LineKind) -> char;       // '+' / '-' / ' '
pub fn lineno_col_width(files: &[crate::diff::DiffFile]) -> usize;
```

## Tasks

### Wave 1 (No Dependencies)

### [x] 1.0 Foundation: deps + `ui/` module skeleton + frozen stubs (DUW 4.1/4.2/4.3/4.4 scaffolding)

**Wave:** 1 | **Agent Scope:** `Cargo.toml`, `src/ui/mod.rs`, `src/ui/keymap.rs` (compiling stub only), `src/ui/terminal.rs` (compiling stub only), `src/ui/app.rs` (compiling stub only). Do NOT touch `src/main.rs`.
**FRs:** none land behaviorally here — T1.0 establishes the frozen interface the FR-bearing Wave-2/3 tasks fill. (Enables FR-render-term-*, FR-render-keymap-*, FR-render-view-*, FR-render-wire-*.)
**wiring_caller:** `src/main.rs::run()` calls `ui::run` (via T4.0, Wave 3); `keymap.rs`/`terminal.rs`/`app.rs` bodies consume these types (T2.0/T3.0/T4.0).
**wired_path_test:** compile-time — `cargo build` + all four gates green with stubs in place; behavioral wiring proven by T2.0/T3.0/T4.0 tests. (Foundation/scaffold task: it is not itself a "build X that does Y" unit; the named future tasks T2.0/T3.0/T4.0 exist in the wave schedule and carry the real wiring proofs.)
**FR-render-keymap-5 invariant:** the stubs MUST NOT hardcode any key name outside `default_map()`; the `chord_for` reverse-lookup accessor is stubbed here precisely so displays derive keys from the map.

#### 1.0 Proof Artifact(s)

- Build: `cargo build` compiles with ratatui + crossterm resolved and all three `ui/` submodules registered.
- Grep: `src/ui/mod.rs` declares `pub mod terminal; pub mod keymap; pub mod app;` and re-exports every symbol in the Interface Contracts block; `UiError` is defined here.
- Gates: `cargo clippy -- -D warnings` + `cargo fmt --check` clean with stubs (stubs silence unused params via `let _ = ...;`, no `#[allow(dead_code)]`).

#### 1.0 Quality Verification

- [x] `cargo build` clean
- [x] `cargo test` — all passing (existing tests unaffected)
- [x] `cargo clippy -- -D warnings` — zero warnings
- [x] `cargo fmt --check` clean
- [x] /trace: `ui/mod.rs` imports no `git` types; `UiError` uses `thiserror`; stubs contain no key-name literals outside `keymap.rs`

#### 1.0 Tasks

- [x] 1.1 Add `ratatui` and `crossterm` to `Cargo.toml` `[dependencies]` (latest compatible stable; crossterm backend only). This task is the SOLE writer of `Cargo.toml` this cycle. The commit that lands this MUST include the one-line dependency justification (see execution context) per the CLAUDE.md guardrail.
- [x] 1.2 Define `UiError` (thiserror, `Io(#[from] std::io::Error)`) in `src/ui/mod.rs`, register `pub mod terminal; pub mod keymap; pub mod app;`, and add the `pub use` re-exports from the Interface Contracts block so `ui::Action`, `ui::Keymap`, `ui::TerminalGuard`, `ui::run`, etc. resolve from the crate root.
- [x] 1.3 Stub `src/ui/keymap.rs`: define the full `Action` enum, `KeyChord { code, modifiers }` (derive `Hash, Eq, PartialEq, Clone, Copy, Debug`) with `from_event`, and `Keymap { bindings: HashMap<KeyChord, Action> }` with `default_map()` (return `Keymap { bindings: HashMap::new() }`), `resolve()` (return `Action::Noop`), and `chord_for()` (return `None`). Real behavior fills in T2.0. Clippy-clean.
- [x] 1.4 Stub `src/ui/terminal.rs`: define `TerminalGuard` (holding the ratatui `Terminal<CrosstermBackend<Stdout>>`), `TerminalGuard::enter() -> Result<Self, UiError>`, `terminal_mut()`, `impl Drop`, and a free `restore_terminal()` fn. Stub bodies may construct the terminal but the panic hook + real enter/restore fill in T3.0. Clippy-clean; no `unwrap`/`expect`.
- [x] 1.5 Stub `src/ui/app.rs`: define `App { files, scroll, should_quit }` (private fields, `usize` scroll), `pub fn run(files) -> Result<(), UiError>` (stub body: `let _ = files; Ok(())`), and the pure helpers `clamp_scroll`, `visible_range`, `gutter_marker`, `lineno_col_width` with minimal correct-typed bodies (real logic fills in T4.0). Clippy-clean.
- [x] 1.6 Confirm all four gates green with the full skeleton in place; the crate root exposes the frozen contract for Wave 2/3.

---

### Wave 2 (Requires Wave 1)

### [ ] 2.0 Keymap as data (DUW 4.2)

**Wave:** 2 | **Agent Scope:** `src/ui/keymap.rs` (fill body only — do NOT touch `mod.rs`, `Cargo.toml`, or any other file)
**FRs:** FR-render-keymap-1, FR-render-keymap-2, FR-render-keymap-3, FR-render-keymap-4, FR-render-keymap-5
**wiring_caller:** `src/ui/app.rs::run` event loop (T4.0, Wave 3) calls `Keymap::default_map()` once and `keymap.resolve(ev)` per key event to route every key; the future help overlay (Task 5+) calls `chord_for` for display.
**wired_path_test:** `src/ui/keymap.rs` `#[cfg(test)]` unit tests (this task); end-to-end key routing exercised by T4.0's loop.
**FR-render-keymap-5 invariant:** `default_map()` is the single source of truth for bindings; `chord_for` is the reverse-lookup any display uses. No key name is hardcoded anywhere else.

**Open-Question defaults baked in (spec §9):** register `gd`/`gr` as placeholder
single-entry no-ops (doc-comment the deferral to Task 5); `K` → `Action::Hover`
(real no-op binding). Add arrow/page aliases as EXTRA entries: `Down`→`ScrollDown`,
`Up`→`ScrollUp`, `PgDn`→`HalfPageDown`, `PgUp`→`HalfPageUp`.

#### 2.0 Proof Artifact(s)

- Test: `keymap.resolve(KeyEvent(char 'j'))` → `Action::ScrollDown`; `'k'` → `ScrollUp`; `Ctrl-d` → `HalfPageDown`; `Ctrl-u` → `HalfPageUp`; `'q'` → `Quit`; `'Q'` → `QuitDiscard` (FR-render-keymap-1/2).
- Test: `']'` resolves to `Action::NextHunk` even though it is a no-op this task (binding present, behavior deferred) — FR-render-keymap-3.
- Test: an unbound key (e.g. `'z'`) resolves to `Action::Noop` (FR-render-keymap-4).
- Test: arrow/page aliases resolve to the same actions as their vim counterparts (`Down`→`ScrollDown`, `PgDn`→`HalfPageDown`) — spec §9 default.
- Test: `chord_for(Action::Quit)` returns the chord bound to `'q'` — proves display derives keys from the map (FR-render-keymap-5).

#### 2.0 Quality Verification

- [ ] `cargo build` clean
- [ ] `cargo test` — all passing
- [ ] `cargo clippy -- -D warnings` — zero warnings
- [ ] `cargo fmt --check` clean
- [ ] /trace: key resolution is table lookup over `bindings` data — no `match ev.code` behavior arms; bindings are plain data (no closures/trait objects)

#### 2.0 Tasks

- [ ] 2.1 (test-first) Write failing unit tests in `src/ui/keymap.rs` for the live bindings (`j`/`k`, `Ctrl-d`/`Ctrl-u`, `q`/`Q`), a present-but-no-op binding (`]`→`NextHunk`), an unbound key (→`Noop`), the arrow/page aliases, and the `chord_for` reverse lookup.
- [ ] 2.2 Implement `KeyChord::from_event` (normalize a `crossterm` `KeyEvent` to `{ code, modifiers }`; strip irrelevant modifier bits so `'j'` with no modifiers is stable as a `HashMap` key).
- [ ] 2.3 Implement `Keymap::default_map()` binding EXACTLY the README draft map (spec FR-render-keymap-2): `j`/`k`, `Ctrl-d`/`Ctrl-u`, `]`/`[`, `Tab`/`Shift-Tab`, `/`, `n`/`N`, `c`, `v`, `space`, `s`, `gd`/`gr`/`K`, `a`, `?`, `q`/`Q` → their `Action`s; plus the arrow/page aliases. `gd`/`gr` as placeholder single-entry no-ops (doc-comment).
- [ ] 2.4 Implement `resolve()` (lookup `KeyChord::from_event(ev)` in `bindings`, default `Action::Noop`) and `chord_for()` (reverse scan for the first chord bound to a given `Action`).
- [ ] 2.5 Add `// FR-render-keymap-N` traceability comments at the implementing sites; doc-comment the `gd`/`gr` deferral and the "all bindings provisional / one-line rebind" invariant (FR-render-keymap-5).

### [ ] 3.0 Panic-safe terminal guard (DUW 4.1 — do this first)

**Wave:** 2 | **Agent Scope:** `src/ui/terminal.rs` (fill body only — do NOT touch `mod.rs`, `Cargo.toml`, or any other file)
**FRs:** FR-render-term-1, FR-render-term-2, FR-render-term-3
**wiring_caller:** `src/ui/app.rs::run` (T4.0, Wave 3) constructs `TerminalGuard::enter()` before the render loop and relies on its `Drop` (and the panic hook) for restore.
**wired_path_test:** `src/ui/terminal.rs` `#[cfg(test)]` unit test that `restore_terminal()` is callable and idempotent without a live TTY (spec §4.1 proof); end-to-end panic-restore verified observably by T4.0 (temporary panic hook).
**FR-render-keymap-5 invariant:** the guard displays/handles no key names; N/A here beyond not introducing any hardcoded-key literal.

#### 3.0 Proof Artifact(s)

- Unit test: `restore_terminal()` is callable twice in a row without panicking and without a live TTY (idempotent teardown) — spec §4.1 "teardown callable without a live TTY".
- Observable (manual, recorded in proofs): forcing a panic mid-render returns a usable shell prompt (no stuck raw mode, cursor visible, echo on) — FR-render-term-2. Verified via T4.0's temporary panic trigger.
- Observable: `q` exits and the terminal is clean (raw mode disabled, alt screen left) — FR-render-term-1/3.

#### 3.0 Quality Verification

- [ ] `cargo build` clean
- [ ] `cargo test` — all passing
- [ ] `cargo clippy -- -D warnings` — zero warnings
- [ ] `cargo fmt --check` clean
- [ ] /trace: setup/teardown is encapsulated in `TerminalGuard`; `Drop` restores; no `unwrap`/`expect` outside tests; panic hook chains to the previous hook

#### 3.0 Tasks

- [ ] 3.1 (test-first) Write a failing unit test asserting `restore_terminal()` is idempotent and TTY-independent (call it twice; no panic; returns `()`), and that `TerminalGuard`'s teardown path routes through `restore_terminal()`.
- [ ] 3.2 Implement `restore_terminal()`: `crossterm::terminal::disable_raw_mode()` + `LeaveAlternateScreen` (+ show cursor); ignore "already restored" errors so it is idempotent (FR-render-term-1/3).
- [ ] 3.3 Implement `TerminalGuard::enter()`: enable raw mode, enter the alternate screen, build the `Terminal<CrosstermBackend<Stdout>>`, and **install a panic hook** (chaining the previous hook) that calls `restore_terminal()` BEFORE the default panic message prints (FR-render-term-2). Return `Result<Self, UiError>`, no `unwrap`.
- [ ] 3.4 Implement `impl Drop for TerminalGuard` calling `restore_terminal()` so early returns and `?` also restore (FR-render-term-3), plus `terminal_mut()` for the app loop's draw access.
- [ ] 3.5 Add `// FR-render-term-N` traceability comments at the implementing sites.

---

### Wave 3 (Requires Wave 2)

### [ ] 4.0 Unified diff render + scrolling + main→ui wiring (DUW 4.3 + 4.4)

**Wave:** 3 | **Agent Scope:** `src/ui/app.rs` (fill body only) + `src/main.rs` (route into `ui::run`). Do NOT touch `mod.rs`, `Cargo.toml`, `keymap.rs`, or `terminal.rs`.
**FRs:** FR-render-view-1, FR-render-view-2, FR-render-view-3, FR-render-view-4, FR-render-view-5, FR-render-wire-1, FR-render-wire-2
**wiring_caller:** this task IS the production wiring — `src/main.rs::run()` loads `RawFilePatch`es via `GitRunner`, parses via `diff::parse_patches`, and calls `ui::run(files)`; the loop drives `Keymap` (T2.0) inside a `TerminalGuard` (T3.0).
**wired_path_test:** `src/ui/app.rs` `#[cfg(test)]` unit tests for the pure layout/scroll helpers (this task); observable `cargo run` in dirty + clean repos. Interactive render itself is not unit-tested (spec §8).
**FR-render-keymap-5 invariant:** the empty-state message and any key-referencing UI text (e.g. "press <q> to quit") MUST derive the key from `keymap.chord_for(Action::Quit)` — no hardcoded `'q'` in `app.rs`. Behavior routes through `keymap.resolve(ev)`; the loop contains no `match ev.code` arms.

**Open-Question defaults baked in (spec §9):** long lines render **truncated to
width with a trailing marker** (no wrap; gutters stay aligned); half-page = **half
the current viewport height, recomputed per event** (FR-render-view-5). Binary /
zero-hunk files render the header + a **"binary / no textual diff" placeholder
row**, not an empty gap (spec §6). Terminal resize re-renders to the new size
without panic. Only the visible row slice is built/styled per frame (spec §6 large-diff).

#### 4.0 Proof Artifact(s)

- Unit test: `clamp_scroll(offset, total, viewport)` clamps past-end and past-top to bounds; `visible_range` returns the correct clamped `scroll..scroll+viewport` window; `gutter_marker(LineKind::Added)` → `'+'`, `Removed` → `'-'`, `Context` → `' '`; `lineno_col_width` returns the digit width of the largest line number across all files (representative inputs) — FR-render-view-2/5.
- Observable: `cargo run` in a dirty repo shows a colored, scrollable unified diff spanning all changed files (file header per file, hunk header per hunk, one row per `Line` with gutter + old/new line numbers + content); added green / removed red / context default; `changed_spans` regions emphasized — FR-render-view-1/2/3/4.
- Observable: scrolling past the end/top clamps rather than panicking; `j`/`k` move one line, `Ctrl-d`/`Ctrl-u` a half page — FR-render-view-5.
- Observable: `cargo run` in a CLEAN repo (empty diff) enters the TUI, shows an explicit empty-state message, and `q` exits cleanly (no special-case bypass) — FR-render-wire-2.
- Grep: `src/ui/` has no `use ...git` imports — FR-render-wire-1.

#### 4.0 Quality Verification

- [ ] `cargo build` clean
- [ ] `cargo test` — all passing (incl. new layout-helper unit tests)
- [ ] `cargo clippy -- -D warnings` — zero warnings
- [ ] `cargo fmt --check` clean
- [ ] /trace: `ui/` imports no `git` types (wire-1); key handling is `keymap.resolve` only (no widget match arms); no hardcoded key names outside `default_map()` (keymap-5); render slices the visible range only (no whole-buffer per-frame build)

#### 4.0 Tasks

- [ ] 4.1 (test-first) Write failing unit tests in `src/ui/app.rs` for the pure helpers: `clamp_scroll` (past-end, past-top, exact-fit), `visible_range` (clamped window, viewport larger than content), `gutter_marker` (all three `LineKind`s), `lineno_col_width` (single-digit vs multi-digit, multi-file max). These compile against T1.0's stubbed signatures and fail until 4.2 lands.
- [ ] 4.2 Implement the pure helpers and a flattened-row model: build the ordered render rows (file header, hunk header, one per `Line`; binary/zero-hunk → header + placeholder row) from `&[DiffFile]`; `visible_range`/`clamp_scroll` select the slice; `gutter_marker` + `lineno_col_width` format the gutter/number columns. Long lines truncate-to-width with a trailing marker (FR-render-view-1/2/3/4; spec §9 default).
- [ ] 4.3 Implement `run(files)`: construct `Keymap::default_map()` and a `TerminalGuard::enter()?`; loop = draw the visible slice, then **block on the next crossterm event** (key or resize), `keymap.resolve(ev)` → mutate `App` (`ScrollDown`/`Up` ±1 clamped; `HalfPageDown`/`Up` ± half viewport height recomputed per event; `Quit`/`QuitDiscard` set `should_quit`; every other `Action` is a no-op), re-draw only on change; exit the loop when `should_quit`. Render on event, never busy-loop (FR-render-view-5). Handle resize by re-rendering to the new size (spec §6). No `unwrap`/`expect`.
- [ ] 4.4 Render styling: added lines in an added color, removed in a removed color, context default (plain colors, no syntax highlighting); within paired lines, render `changed_spans` regions with distinct emphasis (brighter/inverted) — FR-render-view-3/4. Empty-state row when `files` is empty (FR-render-wire-2), with any key hint derived via `keymap.chord_for(Action::Quit)` (FR-render-keymap-5).
- [ ] 4.5 Wire `src/main.rs::run()`: keep `GitRunner::discover()` + `runner.diff(&target)`, replace the summary `println!` (and the untracked-file counting, which was summary-specific) with `let files = diff::parse_patches(&patches); ui::run(files)?;`. `run()` stays `anyhow::Result<()>` and free of `unwrap`/`expect` (`UiError` converts via `?`). `ui/` is never imported by `git/` and never imports `git/` (FR-render-wire-1).
- [ ] 4.6 Add a temporary panic-trigger path (behind a hidden/test-only flag or documented manual step) to observe FR-render-term-2 restore end-to-end, then record the observation in `03-proofs`-style proof notes; add `// FR-render-view-N` / `// FR-render-wire-N` traceability comments.

## Post-Generation Verification (recorded at task-gen time)

- **Check 1 — Requirement coverage:** all 15 FR-IDs mapped — keymap-1..5 → T2.0; term-1..3 → T3.0; view-1..5 + wire-1..2 → T4.0; T1.0 scaffolds all. Spec defines **no** ERR-IDs (confirmed by grep). Every spec §6 edge case has an owner: panic-restore (T3.0 + T4.0 §4.6), empty diff (T4.0 §4.4), long lines truncate-to-width (T4.0 §4.2), large diff visible-slice (T4.0 §4.2/4.3), resize (T4.0 §4.3), binary/zero-hunk placeholder (T4.0 §4.2), narrow terminal clamp (T4.0 §4.2 helpers). UTF-8 validity is guaranteed upstream by `git/` (no `ui/` handling needed).
- **Check 2 — Dependency ordering:** no task consumes a same/later-wave output at runtime. Wave 1 stubs the frozen contract; Wave 2 (keymap, terminal) are mutually independent and depend only on Wave 1 stubs; Wave 3 (app/render + main wiring) depends on real keymap + terminal bodies from Wave 2. No forward references.
- **Check 3 — Agent scope / shared-file ownership:** `Cargo.toml` → T1.0 only; `src/ui/mod.rs` → T1.0 only (never re-touched — `run` body lives in `app.rs`); `src/main.rs` → T4.0 only. Within each wave every file has exactly one writer (Wave 2's `keymap.rs`/`terminal.rs` are disjoint). Max scope: T4.0 = 2 files; all others ≤ 5 trivial-stub files (T1.0) or 1 file. Well under split thresholds.
- **Check 4 — Interface contract extraction:** the T1.0 → T2.0/T3.0/T4.0 seams are pinned verbatim in the Interface Contracts block and echoed in each consumer task.
- **Check 6 — Infrastructure ordering:** the terminal guard (panic safety) and keymap land in Wave 2 BEFORE the render loop (Wave 3) that could panic and that routes keys — infrastructure precedes dependents.
- **Check 6d — Path/symbol grounding:** `Cargo.toml` (no ratatui/crossterm yet — confirmed), `src/ui/mod.rs` (empty stub — confirmed), `src/lib.rs` (`pub mod ui` — confirmed), `src/main.rs::run()` (summary println — confirmed), and every `diff`/`git` symbol in the Depends-on block were read/grepped at HEAD `e3d5bf1` before locking task bodies. `keymap.rs`/`terminal.rs`/`app.rs` are new files (noted). No cited path is a phantom.
- **Check 7 — Bare-repo safety:** no task touches `.git/` internals, hooks, or config.
- **Check 9 — Build-task wiring:** T2.0 (keymap) wired by T4.0's loop; T3.0 (terminal) wired by T4.0's `run`; T4.0 IS the production `main.rs → ui::run` wiring. T1.0 is a scaffold task whose wiring is carried by the named future tasks (all present in the schedule). No built-but-unwired module.
