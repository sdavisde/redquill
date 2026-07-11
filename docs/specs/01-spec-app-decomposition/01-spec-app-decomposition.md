# 01-spec-app-decomposition.md

## Introduction/Overview

`src/ui/app.rs` is a 3,921-line single-view "god object": it holds the file list, one selected file, one row buffer, one cursor/scroll pair, all modal states, staging gestures, and LSP glue in a single struct. Two upcoming workstreams — the Zed-style git panel (spec 02) and the multi-file collapsible diff buffer (spec 03) — both collide with this structure directly and cannot proceed in parallel while it stands.

This spec decomposes `app.rs` into small, per-concern components with **zero user-visible behavior change**, and introduces one new reusable utility (a background-task poller, generalized from the existing LSP threading pattern) that spec 02 requires for non-blocking fetch/pull/push. The goal is to make `App` a thin coordinator so specs 02 and 03 can be implemented by separate agents with a nearly disjoint file-contact map.

## Goals

- Extract all per-view state and navigation logic out of `App` into a dedicated `DiffViewState` component that the application can, in the future, hold more than one of (or generalize to a multi-file view).
- Extract each modal mode's state and key handling (compose, list, search, peek, staging panel) and the service-glue concerns (staging gestures, LSP/code-intel correlation) into their own modules.
- Introduce a small, unit-tested background-task poller utility modeled on the existing LSP manager's thread + channel + per-tick poll pattern.
- Reduce `src/ui/app.rs` to a thin coordinator of roughly 800 lines or fewer (mode dispatch, component wiring, top-level state).
- Preserve behavior exactly: every existing test passes (relocated as needed), no keybinding or rendering changes, no perceptible performance change.

## User Stories

- **As the project maintainer**, I want per-view state isolated in its own component so that the git-panel and multibuffer workstreams can be built in parallel by separate agents without both rewriting the same 3,900-line file.
- **As a future contributor**, I want each UI concern (view navigation, modal states, staging gestures, code intelligence) in a small focused module so that I can understand and modify one concern without holding the whole application in my head.
- **As a redquill user**, I want this refactor to be invisible — every keybinding, view, and workflow behaves exactly as it did before — so that I can keep reviewing diffs without relearning anything.

## Demoable Units of Work

### Unit 1: DiffViewState extraction

**Purpose:** Isolate the "one view over one diff" state so the application stops assuming exactly one file's rows exist at a time. This is the enabling step for the multibuffer (spec 03).

**Functional Requirements:**
- The system shall provide a `DiffViewState` (name indicative) component in a new module under `src/ui/` that owns: the file list, selected-file index, row buffer, cursor (row and column), unified scroll, side-by-side scroll, and viewport height (currently `app.rs:72-173`).
- The system shall move the motion, clamping, and visibility logic (`nearest_addressable`, `max_cursor`, `ensure_visible`, hunk-jump probing — currently `app.rs:365-581`) onto that component.
- The system shall keep `App` delegating to the component such that all keybindings and rendering behave identically to before the change.
- The system shall relocate the existing unit tests covering moved logic alongside the new component, with all of them passing unmodified in substance.

**Proof Artifacts:**
- CLI: `cargo test` output showing the full suite (currently 412 unit tests + 3 integration files) passing demonstrates behavior preservation.
- CLI: `cargo clippy -- -D warnings` and `cargo fmt --check` passing demonstrates the repo's quality gates hold.
- CLI: `wc -l src/ui/app.rs src/ui/<new module>.rs` before/after demonstrates the state actually moved rather than being duplicated.

### Unit 2: Modal states and service glue extraction

**Purpose:** Empty the remaining concerns out of `App` so it becomes a coordinator, shrinking the merge surface that specs 02 and 03 would otherwise contend over.

**Functional Requirements:**
- The system shall move each modal mode's state fields and its `handle_*_key` function (compose, list, staging panel, search, peek — currently `src/ui/mod.rs:210-320` plus fields on `App`) into per-mode modules or one `modes/` module grouping.
- The system shall move the staging gesture logic (`run_stage_gesture`, `toggle_stage` — currently `app.rs:846-956`) into a UI-side staging module that operates through the existing `StageOps` trait seam.
- The system shall move the code-intelligence glue (`code_intel_position`, `poll_lsp`, `pending_lsp` correlation — currently `app.rs:1297-1361`) into a dedicated module that takes a view's cursor position as input rather than reaching into `App` internals.
- The system shall reduce `src/ui/app.rs` to approximately 800 lines or fewer while keeping the `?` help overlay, keymap table, and all dispatch behavior unchanged.

**Proof Artifacts:**
- CLI: `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check` all passing demonstrates the gates hold after the full decomposition.
- CLI: `wc -l src/ui/app.rs` output at or under the target demonstrates the coordinator goal was met.
- Manual smoke transcript: a short recorded keybinding walkthrough (`j/k`, `]`/`[`, `Tab`, `t`, `/`+`n`, `c`, `space`, `s`, `gd`/`gr`/`K`, `a`, `?`, `q`) against a sample diff demonstrates zero user-visible change.

### Unit 3: Background task poller utility

**Purpose:** Provide the reusable "spawn work on a background thread, drain results per render tick" primitive that spec 02's fetch/pull/push needs, generalized from the pattern the LSP manager already proves out.

**Functional Requirements:**
- The system shall provide a small background-task utility module under `src/ui/` (or a shared location consistent with module boundaries) that can spawn a task on a background thread, return a task identifier immediately, and deliver the task's result through a non-blocking `poll()` drained once per event-loop tick (mirroring `LspManager`'s channel + poll design in `src/lsp/manager.rs`).
- The system shall ensure the utility never blocks the render loop: spawning returns immediately and polling returns only completed results.
- The system shall support reporting task failure (e.g., a command exiting nonzero with its stderr) as a value, not a panic, consistent with the repo's no-`unwrap` error conventions.
- The system shall cover the utility with unit tests using fake/synthetic tasks (no network, no git), including success, failure, and not-yet-complete polling.

**Proof Artifacts:**
- Test: new unit tests for the poller passing under `cargo test` demonstrates the spawn/poll contract works for success, failure, and pending states.
- Code: the utility compiling with no callers other than tests (dead-code-allowed as appropriate) demonstrates it is a ready seam for spec 02 without behavior change in this spec.

## Non-Goals (Out of Scope)

1. **No behavior, keybinding, layout, or rendering changes**: this spec is invisible to users; any visible diff is a defect.
2. **No feature work**: the git panel (spec 02) and multibuffer (spec 03) are separate specs; this spec only prepares seams.
3. **No LSP manager rewrite**: the existing `LspManager` threading stays as-is; migrating it onto the new poller utility is deferred (recorded in Open Questions).
4. **No config layer**: `docs/config-layer.md` (remappable keymap loading, sidebar config) remains untouched.
5. **No performance work**: performance must simply not regress (instant feel on a 5k-line diff per CLAUDE.md); no optimization effort beyond that.

## Design Considerations

No user-visible design changes. The terminal UI must render identically before and after this refactor.

## Repository Standards

- All four gates must pass: `cargo build`, `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check`.
- No `unwrap()`/`expect()` outside tests; `thiserror` in library code, `anyhow` at the binary edge.
- Module boundaries per CLAUDE.md: no TUI types leak into `git/`; `diff/` stays pure data; `lsp/` stays async and non-blocking.
- Unit tests colocated with the code they cover; moved code carries its tests with it.
- Conventional commits (`refactor:` for these changes), with tests committed alongside the code.

## Technical Considerations

- The decomposition targets are already mapped: `App` fields at `app.rs:72-173`; motion/clamping at `app.rs:365-581`; staging gestures at `app.rs:846-956`; code-intel glue at `app.rs:1297-1361`; modal key handlers at `src/ui/mod.rs:210-320`. The keymap (`src/ui/keymap.rs`) and row model (`src/ui/rows.rs`) are consumed, not modified.
- `DiffViewState` should own exactly the state spec 03 will need to generalize to a multi-file buffer (rows, cursor, scrolls, viewport, addressability clamping). Design its API against that consumer: spec 03 replaces "rows for one file" with "rows for many files with collapse state" without touching callers again.
- The side-by-side view (`SbsRow` derivation in `rows.rs`) is the in-repo precedent for views over shared row state; the extraction must not fork or duplicate the row model.
- The background poller should be transport-agnostic (it runs closures/commands, not git specifically) so `git/` stays free of threading concerns and `ui/` stays free of git parsing, preserving module boundaries.
- Work proceeds as a sequence of small behavior-preserving moves, each leaving the suite green — not one big-bang rewrite commit.

## Security Considerations

No specific security considerations identified: no new inputs, credentials, network calls, or data handling are introduced by this refactor.

## Success Metrics

1. **`src/ui/app.rs` line count**: reduced from 3,921 to ≤ ~800 lines, with no single extracted module exceeding ~1,000 lines.
2. **Test integrity**: 100% of the pre-existing test suite passes after each demoable unit, with test count not decreasing.
3. **Zero user-visible change**: the manual keybinding walkthrough produces identical behavior before and after.
4. **Parallelization achieved (trailing metric)**: specs 02 and 03 can subsequently be implemented with no overlapping edits to any file except additive `keymap.rs`/`help.rs`/`mod.rs` touches.

## Open Questions

1. Exact module names and grouping (`view.rs` vs `diff_view.rs` conflict with the existing render module name; `modes/` as one directory vs sibling files) are left to implementation, following existing naming conventions.
2. Whether `LspManager` should later migrate onto the shared background poller — deferred; not needed for specs 02/03.
3. Whether the ~800-line coordinator target is met exactly is advisory; the hard requirement is that view state, modal states, staging gestures, and code-intel glue no longer live in `App`.
