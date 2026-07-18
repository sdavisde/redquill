# 10-tasks-help-discoverability.md

Tasks for `docs/specs/10-spec-help-discoverability/10-spec-help-discoverability.md` (FR-1..FR-13). Each parent task is a thin vertical slice, demoable from the user's perspective, with persisted UX-journey transcripts as evidence per the spec's Success Metrics. **Ordering dependency:** spec 09 (`docs/specs/09-spec-review-launcher/09-spec-review-launcher.md`) introduces `Scope::Global` and the `open-review-launcher` action; spec 10 lands second. As of planning time spec 09 has **not** landed on `main` (no `Scope::Global` in `src/ui/keymap.rs`; `Mode::ReviewBranch` and panel-scope `R` still present) — if that is still true at build time, FR-2's "Works everywhere" section takes the spec's documented fallback (render the currently-duplicated cross-scope rows under the same heading) and the proof artifacts must state which path was taken.

## Standards Evidence

| Source File | Read | Standards Extracted | Conflicts |
| --- | --- | --- | --- |
| `CLAUDE.md` | Yes | (1) Keymap/modal keys are data in the shared tables (`src/ui/keymap.rs`, `src/ui/modal_keys.rs`), never loose match arms; every user-visible action reachable from the keymap and listed in `?`. (2) All four gates (`cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`) before any task is done. (3) Perf tripwires in `src/ui/perf_tests.rs` enforce the complexity class — never loosen budgets to fit a regression. Agent write ceiling during tasks: staging only. | None |
| `README.md` | Yes | (1) `?` is the promised discoverability surface ("press `?` to see the list of keybinds"). (2) Session end copies annotations to the clipboard (commit `4322f5a`) in addition to stdout emission. | Minor wording drift only: spec FR-6's example intent phrase "Quit and print annotations" predates the clipboard-on-quit feature; phrase is editorial per spec Open Question 1 — reword at implementation, no structural change. |
| `docs/rust-best-practices.md` | Yes | (1) Data-driven invariants: behavior and documentation render from one const table with bidirectional drift tests. (2) Nothing blocks the render loop; time-dependent logic tested with injected values, not sleeps; no panic macros in production code; presentation logic factored into pure functions testable without constructing the app. (3) Refactors and behavior changes never share a commit; move-only refactors prove identical test counts and zero assertion edits. | None |
| `CONTRIBUTING.md` | Not found | — | — |
| `.github/pull_request_template.md` | Not found | — | — |
| `docs/specs/08-spec-branch-review-mode/08-tasks-branch-review-mode.md` | Yes (format precedent) | (1) Parent tasks are user-verifiable vertical slices with a "Covers:" note and per-task proof artifacts. (2) Transcripts/screenshots persist under `docs/specs/<spec>/proofs/` (gitignored per repo convention). (3) Gates + conventional commits restated per task. | None |

### Verified Code Anchors

- Help overlay `src/ui/help.rs` (`group_of` ~53, `binding_hidden` ~130, `modal_sections` ~178, `/` filter via `row_matches`/`HelpViewState` ~230–260, drift test `help_overlay_covers_every_keymap_binding` ~528); help state lives as fields on `App` (`help_open`/`help_scroll`/`help_viewport`/`help_search`, `src/ui/app.rs` ~178–197) — help is an overlay flag, **not** a `Mode`; `HELP_KEYS` ~1313 and `HELP_SEARCH_HINTS` ~1426 in `src/ui/modal_keys.rs` (help table is in the config-remappable set); pending-prefix machine in `src/ui/mod.rs` ~335–430 (`pending: &mut Option<KeyEvent>`, `pending_count`); `KeySeq::Two` two-key sequences and `FooterHint` in `src/ui/keymap.rs` ~382–462; `draw()` already receives `pending` (`src/ui/mod.rs` ~682); event loop tick/pollers ~955–1035.

## Relevant Files

| File | Why It Is Relevant |
| --- | --- |
| `src/ui/help.rs` | Center of the spec: `HelpOverlayState` (task 1.2), pure section-builders (1.3), tab logic and "This context"/"All keys" views (2.x), curated workflows const table + header render (3.x), extended drift tests (inline `#[cfg(test)]` module — split per repo convention if it dwarfs production code). |
| `src/ui/app.rs` | Replace the four loose help fields (~178–197) with one `HelpOverlayState`; `ToggleHelp` handler (~942) records the origin mode/scope on open. |
| `src/ui/modal_keys.rs` | `HELP_KEYS` gains tab-switch actions (`Tab`/`Shift-Tab`/`h`/`l`) with kebab-case action names and footer hints; existing modal drift tests extended. |
| `src/ui/modal_keys_config_tests.rs` | Prove the new help-tab actions are config-remappable (help table is already in the remappable set). |
| `src/ui/keymap.rs` | Pure prefix-discovery helpers over `KeySeq::Two` (which-key prefixes + continuation listing, task 4.1); source of `Scope::Global` rows for the "Works everywhere" section once spec 09 lands. |
| `src/ui/keymap_config_tests.rs` | FR-7 remap test: a `[keys.global]`/`[keys.diff]` override changes the key displayed in the workflows header. |
| `src/ui/which_key.rs` | New module: pure popup visibility decision over injected elapsed time (`WHICH_KEY_DELAY` compile-time const) + popup render anchored near the footer; unit tests with no real sleeps. |
| `src/ui/mod.rs` | Event loop: `pending_since: Option<Instant>` stored beside `pending` (~416–418), elapsed threaded into `draw()` (~682); popup render call while a prefix is pending. |
| `src/ui/mod_tests.rs` | Regression pins: pending-prefix resolution byte-identical with the popup feature present; fluent chords never render it. |
| `src/ui/perf_tests.rs` | Must pass unmodified — tripwire for the O(1)-per-tick delay check and popup render cost. |
| `README.md` | Docs-as-contract sweep (task 5.4): sync the `?` description if the redesign changes what it promises. |
| `docs/specs/10-spec-help-discoverability/proofs/` | Persisted journey transcripts (A/B/C), refactor-invisibility proof, gates transcript — gitignored per repo convention (`docs/specs/*/proofs/`). |

### Notes

- All four gates before every commit: `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`. Conventional commits; refactors and behavior changes never share a commit (task 1.0 is the dedicated refactor commit; 2.0+ are behavior commits).
- TDD for pure code per repo convention: prefix-discovery helpers, which-key visibility decision, workflows drift test — failing test first, tests committed with the code.
- Which-key timing tests use injected elapsed values (pure decision function over `(pending, elapsed, threshold)`), **never real sleeps** — flake-free per repo convention.
- Journey transcripts are captured against scratch repos in tempdirs where repo state matters; agents must not fetch/pull/push/commit against the user's repo during tasks. Every transcript is labeled with the terminal size and the Scope::Global-vs-fallback path in effect.
- Spec-09 dependency annotations appear on the affected sub-tasks (2.3, 3.1, 3.7, 5.2): FR-2's "Works everywhere" section has a documented fallback; Journey B has none — spec 09 must land first.

## Tasks

### [x] 1.0 Refactor: consolidate help-overlay state and extract pure section-builders (no behavior change)

Covers: no FR closes here — structural enabler for FR-1..FR-5 (and the FR-7/FR-9 render hooks). Exists to honor the repo rule that refactors and behavior changes never share a commit: the loose `App` fields (`help_open`, `help_scroll`, `help_viewport`, `help_search`) consolidate into one help-overlay state struct that records the mode/scope it was opened over (origin capture per spec Design Considerations), and `help.rs` section assembly is factored into pure functions taking explicit table/context arguments, ready to grow tabs without touching dispatch.

#### 1.0 Proof Artifact(s)

- Test: `cargo test` before/after shows identical test counts with zero assertion edits (move-only invariant, `docs/rust-best-practices.md`), demonstrating behavior preservation.
- Test: existing help drift tests (`help_overlay_covers_every_keymap_binding`, capability-gating tests) and `src/ui/perf_tests.rs` pass unmodified, demonstrating no observable change.
- CLI: `?` overlay opened from diff view and git panel renders identically to pre-refactor (transcript/screenshot pair persisted to `docs/specs/10-spec-help-discoverability/proofs/`), demonstrating the refactor is invisible to the user.

#### 1.0 Tasks

- [x] 1.1 Capture the move-only baseline: run `cargo test` on a clean tree and record the total test count plus the full test-name list (`cargo test -- --list`) to a scratch note outside the repo; this is the comparison basis for sub-task 1.5.
- [x] 1.2 Define `HelpOverlayState` in `src/ui/help.rs` — open flag, scroll `Cell<u16>`, viewport `Cell<u16>`, search `Option<(String, bool)>`, plus an `origin` field recording the mode/scope the overlay was opened over — and replace the four loose `App` fields (`help_open`/`help_scroll`/`help_viewport`/`help_search`, `src/ui/app.rs` ~178–197) with one field, mechanically updating all readers (the `ToggleHelp` handler ~942, the open-predicate ~850, render call sites in `src/ui/mod.rs`). `origin` is written on open but read by nothing yet; render output stays byte-identical.
- [x] 1.3 Factor section assembly in `src/ui/help.rs` into pure functions taking explicit arguments: extract the diff-group row builder (~318), the panel-section builder (~345), and the `modal_sections` consumption into functions returning plain `(title, rows)` section data that `render` consumes. No logic changes, no new filtering, identical output order.
- [x] 1.4 Run all four gates (`cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`).
- [x] 1.5 Verify and record the move-only invariant: test count identical to 1.1's baseline and `git diff` shows zero assertion edits (test-side changes limited to mechanical field/path renames); state the invariant explicitly in the commit message. Capture the pre/post-identical `?` render (diff view + git panel) to `docs/specs/10-spec-help-discoverability/proofs/10-1-refactor-invisibility.txt`; commit as a pure `refactor:` commit.

### [ ] 2.0 Context-first tabbed help overlay: `?` opens on "This context", `Tab` reaches "All keys" (Unit 1)

Covers: FR-1 (two tabs, `Tab`/`Shift-Tab`/`h`/`l` in the config-remappable `HELP_KEYS` table + footer hints), FR-2 (context bindings by origin mode/scope + "Works everywhere" section from `Scope::Global`, with capability gating intact — workflows-header slot renders empty until 3.0, where FR-2 fully closes; if spec 09 is unlanded, use the spec's documented fallback and say so in the proofs), FR-3 (full grouped reference unchanged on "All keys"), FR-4 (`/` filter live on both tabs, filter + scroll reset on tab switch), FR-5 (drift tests extended: per-context completeness of "This context", total completeness of "All keys").

#### 2.0 Proof Artifact(s)

- Test: extended bidirectional drift tests demonstrate every binding reachable in a context appears on that context's "This context" view and the "All keys" tab remains complete (FR-5).
- Test: filter tests demonstrate `/` narrows only the active tab and resets on tab switch (FR-4).
- CLI: journey transcript (Journey A, first capture) — `?` from the diff view, from the git panel, and from a modal that binds `?` shows three different context views, each fitting one 80×24 screen without scrolling; transcript states which "Works everywhere" data path (Scope::Global vs fallback) was taken; persisted to `proofs/` (FR-1, FR-2, FR-3).

#### 2.0 Tasks

- [ ] 2.1 Add `HelpTab { ThisContext, AllKeys }` and a `tab` field to `HelpOverlayState`; opening `?` always starts on `ThisContext`; switching tabs resets `search` to `None` and scroll to 0 (FR-4's reset half). Unit-test the open-default and the reset.
- [ ] 2.2 Add tab-switch actions to `HELP_KEYS` in `src/ui/modal_keys.rs` (`Tab`/`l` → next tab, `Shift-Tab`/`h` → previous tab) with stable kebab-case action names and footer hints; extend the existing modal drift tests and `src/ui/modal_keys_config_tests.rs` so the new actions are provably config-remappable (FR-1).
- [ ] 2.3 Build the "This context" view as a pure function of `(origin, tables, capability flags, query)` returning ordered sections: the workflows-header slot (renders empty until task 3.0 closes FR-2), the origin context's bindings (Diff scope for `Normal`/`Visual` origin, Panel scope for `Panel`, the origin modal's table when opened from a modal that binds `?`), then a "Works everywhere" section. **Depends on spec 09**: source the section from `Scope::Global` bindings if 09 has landed; otherwise use the spec's documented fallback — the currently-duplicated cross-scope rows (`?`, `@`, `!`, `q`, `Q`/`Ctrl-C`) under the same heading. Capability gating via the existing `binding_hidden`. Unit-test the sections produced for each origin.
- [ ] 2.4 Wire tab dispatch into `render`: the "All keys" tab renders today's full grouped reference unchanged in content (FR-3); the `/` filter applies only to the visible tab's sections (FR-4's live-filter half). Test that a query narrowing one tab leaves the other tab complete after a switch (which also resets the filter).
- [ ] 2.5 Extend the bidirectional drift tests (FR-5): (a) every binding dispatchable in a context appears on that context's "This context" view unless capability-hidden; (b) `help_overlay_covers_every_keymap_binding` continues to prove "All keys" completeness; (c) a filter-reset-on-tab-switch regression test.
- [ ] 2.6 Run gates; capture Journey A (first pass): scripted `?` from the diff view, from the git panel, and from one modal that binds `?`, at 80×24, each view fitting one screen without scrolling; persist to `proofs/10-2-journey-a-context-help.txt` labeled with the terminal size and the Scope::Global-vs-fallback path taken; commit (behavior commit, separate from task 1.0's refactor).

### [ ] 3.0 Common-workflows header: intent phrases resolve live to keys (Unit 2)

Covers: FR-6 (curated const table, target 5 / hard cap 7, intent phrase → `Action`), FR-7 (header at top of "This context", keys resolved from the effective post-config-merge keymap), FR-8 (unbound-in-effective-keymap entries omitted; drift test fails the build if a curated entry is unbound in the *default* keymap), FR-9 (capability-gated entries hidden by the same gating as regular rows). Closes FR-2 completely (header now occupies its slot). Intent-phrase wording finalized here per spec Open Question 1 (including updating the "Quit and print annotations" example to match shipped clipboard-on-quit behavior).

#### 3.0 Proof Artifact(s)

- Test: drift test demonstrates every curated entry resolves to a bound default-keymap action (FR-8) and gated entries disappear in a gated context (FR-9).
- Test: remap test demonstrates a `[keys.global]`/`[keys.diff]` override changes the key displayed in the header (FR-7).
- CLI: journey transcript (Journey B) — starting from the workflows header alone, "Review a branch or commit" executes end-to-end: header names the key, pressing it opens the review launcher; persisted to `proofs/` (FR-6, FR-7).

#### 3.0 Tasks

- [ ] 3.1 TDD the curated workflows table in `src/ui/help.rs`: write the drift test first (every entry's `Action` is bound in the *default* keymap — a build-breaking test per FR-8 — and the entry count is ≤ 7), then land the const table (target 5 entries pairing intent phrase → `Action` per FR-6's examples). **Depends on spec 09** for the "Review a branch or commit" → `open-review-launcher` entry; if 09 has not landed, bind that entry to the existing review-branch opener action as the documented interim fallback and swap to `open-review-launcher` when 09 lands.
- [ ] 3.2 Fix the stale FR-6 example wording: replace "Quit and print annotations" with phrasing matching shipped behavior since commit `4322f5a` (annotations are copied to the clipboard on quit, in addition to stdout emission), e.g. "Quit and copy annotations". Editorial change per spec Open Question 1 — wording only, no structural change.
- [ ] 3.3 Pure resolution function: `Action` → displayed key label(s) from the *effective* (post-config-merge) keymap, scope-aware for the current context; entries whose action is unbound in the effective keymap are omitted from display (FR-7, plus FR-8's graceful-degradation half). Unit-test resolution, remap display, and omission.
- [ ] 3.4 Apply capability gating to header entries through the same `binding_hidden` context flags as regular help rows (FR-9); test that a gated entry (e.g., staging in a read-only view) disappears from the header.
- [ ] 3.5 Render the header at the top of the "This context" tab: "Common workflows" title, intent phrase left / key right, never more than 7 lines, visually distinct from the key sections — this closes FR-2.
- [ ] 3.6 Remap display test in `src/ui/keymap_config_tests.rs`: a `[keys.global]` (or `[keys.diff]`) override changes the key shown in the header (FR-7).
- [ ] 3.7 Run gates; capture Journey B: starting from the workflows header alone, "Review a branch or commit" names the key and pressing it opens the review launcher end-to-end. **Hard dependency on spec 09 — this journey has no fallback; 09 must land first.** Persist to `proofs/10-3-journey-b-intent-to-action.txt` labeled with the terminal size and the Scope::Global-vs-fallback path; commit.

### [ ] 4.0 Which-key popup: pause on a pending `g`/`z` prefix reveals its continuations (Unit 3)

Covers: FR-10 (prefixes derived from the keymap's `KeySeq::Two` sequences, not hardcoded; popup after ~500 ms compile-time delay listing bound continuations with descriptions from the effective keymap), FR-11 (pending-prefix state machine byte-identical; dismissal exactly mirrors existing prefix resolution; delay check piggybacks on the existing render tick — stored `Instant` compared on tick, no threads/timers), FR-12 (effective keymap reflected, count-prefixed chords like `3g` show the same continuations), FR-13 (drift test: popup contents equal the keymap's two-key bindings for that prefix). Popup anchored near the footer per spec Design Considerations; renders only while a prefix is pending (perf tripwires unaffected).

#### 4.0 Proof Artifact(s)

- Test: drift test demonstrates popup contents are derived from the keymap table, including under a config remap that adds/removes a continuation (FR-10, FR-12, FR-13).
- Test: state-machine tests with **injected elapsed values (no real sleeps)** demonstrate pending-prefix resolution is byte-identical with the feature present, the popup appears only past the threshold, and fluent chords never render it (FR-11).
- Test: `src/ui/perf_tests.rs` passes unmodified, demonstrating the O(1)-per-tick delay check adds no hot-path cost.
- CLI: journey transcript (Journey C) — press `g`, pause, popup lists continuations, press `d` to jump to definition (explore-then-execute); same session shows a fluent `gd` producing no popup; persisted to `proofs/` (FR-10, FR-11).

#### 4.0 Tasks

- [ ] 4.1 TDD pure prefix-discovery helpers in `src/ui/keymap.rs`: (a) the set of which-key prefixes = the distinct first chords of `KeySeq::Two` bindings in the effective keymap (FR-10 — derived, never hardcoded, so a future prefix or a user remap creating one gets the popup for free); (b) `continuations(prefix)` → `(key label, description)` rows in table order. Failing tests first, including a config remap that adds and removes a two-key binding.
- [ ] 4.2 New module `src/ui/which_key.rs`: pure visibility decision `should_show(pending, elapsed, threshold)` taking **injected elapsed time** — no `Instant::now()` inside the function; the caller supplies elapsed (this is the test seam) — with `WHICH_KEY_DELAY: Duration` (500 ms) as a compile-time const per spec Non-Goal 1. Unit tests at below/at/above threshold, **no real sleeps**.
- [ ] 4.3 Event-loop signature change (planning surprise #5, explicit): store `pending_since: Option<Instant>` beside `pending` in the run loop in `src/ui/mod.rs` (~955–1035) — set at the moment a prefix goes pending (`just_started_sequence`, ~416–418), cleared whenever `pending` clears, and unaffected by `pending_count` so a pending `3g` shows the same continuations as `g` (FR-12); thread `pending` plus the computed elapsed into `draw()` (~682). The pending-prefix state machine's transitions are untouched (FR-11).
- [ ] 4.4 Regression-pin FR-11: tests demonstrating pending-prefix resolution (typing a continuation, `Esc`, an invalid key, and count-prefixed chords) is byte-identical with the feature present, and that a fluent chord (continuation arriving before the threshold) never renders the popup — all via injected elapsed values, no sleeps.
- [ ] 4.5 Popup render: a small overlay anchored near the footer (not centered, per spec Design Considerations), listing `key — description` rows in table order from 4.1, rendered only while a prefix is pending and elapsed ≥ `WHICH_KEY_DELAY`; drift test that the popup's contents equal the set of two-key bindings for that prefix in the effective keymap, including under a remap (FR-13, FR-12).
- [ ] 4.6 Confirm `src/ui/perf_tests.rs` passes unmodified (the delay check is O(1) per tick); run gates; capture Journey C: `g` + pause → popup lists continuations → `d` jumps to definition (explore-then-execute), then a fluent `gd` in the same session with no popup; persist to `proofs/10-4-journey-c-which-key.txt` labeled with the terminal size; commit.

### [ ] 5.0 Success-metric journeys and zero-drift/zero-regression closure

Covers: no new FR — end-to-end verification of the spec's Success Metrics over the finished feature set: Journey A re-captured with the workflows header present (its 2.0 capture predates 3.0), Journeys B and C confirmed against the final build, and metric 4 (all extended drift tests, all pre-existing help tests, perf tripwires, and all four cargo gates green in one run). Also reconciles docs-as-contract: help modal footer hints list the new tab keys, and any README/spec wording touched by the redesign is synced.

#### 5.0 Proof Artifact(s)

- CLI: final persisted journey transcripts A, B, and C in `docs/specs/10-spec-help-discoverability/proofs/`, each labeled with the terminal size (80×24 for A) and the Scope::Global-vs-fallback path in effect, demonstrating the spec's three Success Metric journeys on the shipped build.
- Test: one clean run of the full gate set (`cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`) plus the perf tripwires, captured as a transcript, demonstrating zero drift and zero regression (Success Metric 4).
- CLI: `?` overlay footer showing the tab-switch hints from the shared modal table, demonstrating the no-hidden-features rule holds for the new keys.

#### 5.0 Tasks

- [ ] 5.1 Re-capture Journey A on the finished build (workflows header now present — the 2.6 capture predates task 3.0): `?` from the diff view, the git panel, and one modal, at 80×24, each fitting one screen; persist to `proofs/10-5-journey-a-final.txt` labeled with the terminal size and the Scope::Global-vs-fallback path in effect.
- [ ] 5.2 Confirm Journeys B and C against the final build and persist final transcripts to `proofs/10-5-journey-b-final.txt` and `proofs/10-5-journey-c-final.txt` with the same labeling. **Journey B retains the hard spec-09 dependency (review launcher) — no fallback; 09 must have landed.**
- [ ] 5.3 One clean full run of all four gates plus the perf tripwires, captured as a transcript to `proofs/10-5-gates.txt` (Success Metric 4: zero drift, zero regression).
- [ ] 5.4 Docs-as-contract sweep: verify the help modal footer shows the tab-switch hints from the shared `HELP_KEYS` table (capture to `proofs/10-5-help-footer.txt`), and sync any README/spec wording the redesign touched (e.g., the `?` description in README's Getting Started). No new keybinds and no scope beyond the spec (Non-Goals hold: no config delay knob, no startup nudge, no command palette, no modal which-key).
- [ ] 5.5 Final traceability review: confirm every FR-1..FR-13 maps to at least one landed, named test from tasks 2.x–4.x; land any doc sync as its own `docs:` commit.
