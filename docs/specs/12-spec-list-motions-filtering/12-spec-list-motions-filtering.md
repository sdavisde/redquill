# 12-spec-list-motions-filtering.md

## Introduction/Overview

Movement and filtering are inconsistent across the app's list-like contexts: the diff view has half/full-page motions, `gg`/`G`, and count prefixes; the git panel has only `j`/`k`; the annotation list, staging panel, accepted panel, and switcher have step-only navigation and no way to filter, even though the help overlay (paging + `/` filter) and the file finder (fuzzy matching) prove both patterns in-app. This spec introduces two shared mechanisms and applies them everywhere: a **motion layer** — the motion set (step, half/full page, top/bottom, count prefixes) defined once and consumed by every buffer-like context — and a **`/` filter mode** — a reusable fuzzy-filter component for list contexts.

Ratified principle (questions round 1): the git panel is a type of buffer that should support vim motions the same way the diff view does, and a future motion addition should apply everywhere without per-context wiring.

## Goals

- One definition of "how you move in a buffer": every consuming context (diff view, git panel, modal lists) supports the full motion set including count prefixes, enforced by a drift test that fails when a context misses a motion.
- Adding a new motion to the shared set makes it work in every consuming context with no per-context code.
- Any list a user can face at real-world size (annotations, staged files, accepted files, branches, worktrees, launcher commits) is fuzzy-filterable via `/` with one consistent interaction: type to narrow, `Esc` clears/exits, `Enter` locks.
- The diff view's existing motion behavior is preserved exactly through the refactor (move-only invariant on its motion semantics).
- Spec 09's Review-launcher tabs adopt both mechanisms, closing the loop so the newest list in the app is also the best-behaved one.

## User Stories

- **As a reviewer with a 200-file changeset**, I want `Ctrl-d`, `G`, and `3j` in the git panel so that reaching a file doesn't take fifty keystrokes.
- **As a reviewer with dozens of annotations**, I want to press `/` in the annotation list and type a few characters so that I can find my comment without stepping through the whole list.
- **As a user with many branches**, I want the switcher and the Review launcher's Branches tab to filter fuzzily like the file finder so that picking a branch feels like the finder I already know.
- **As a future contributor**, I want to add a motion once and have it work in every list so that movement improvements never create new parity gaps.

## Demoable Units of Work

### Unit 1: Shared motion layer

**Purpose:** Motions become data consumed by every buffer-like context. Serves all users; prevents future parity gaps structurally.

**Functional Requirements:**

- FR-1: A shared motion set shall be defined once (as data, alongside the keymap tables): cursor step down/up, half-page, full-page, jump-to-top (`gg`), jump-to-bottom (`G`), each accepting an optional count prefix (`3j`, `2Ctrl-d`). Default keys match the diff view's current bindings.
- FR-2: The diff view shall be refactored to consume the shared layer with behavior preserved exactly — same keys, same semantics, same count handling; its motion-related tests pass unchanged (move-only invariant on motion semantics).
- FR-3: The git panel (both tabs) shall consume the layer: paging and jumps clamp against the panel row count, and the History tab's lazy prefetch triggers on layer-driven moves exactly as it does on `j`/`k` today. Count-digit interception shall move out of the `Normal`/`Visual`-only dispatch arm into the shared layer so counts work in every consuming context.
- FR-4: The modal list contexts — annotation list, staging panel, accepted panel, switcher (both tabs), and LSP peek — shall consume the layer for their navigation, replacing their hand-rolled `j`/`k` rows. The help overlay's existing paging keys shall be reconciled onto the layer where behavior-identical.
- FR-5: A drift test shall enumerate every consuming context and assert it dispatches the complete motion set; adding a motion to the set without full coverage fails the build.
- FR-6: All motion keys remain visible in help/footer per context via the existing shared-table rendering, and remain config-remappable wherever their host table already is.

**Proof Artifacts:**

- Test: motion-layer unit tests demonstrate count parsing and each motion's semantics as pure functions (FR-1).
- Test: diff-view motion tests pass unchanged post-refactor, demonstrating behavior preservation (FR-2).
- Test: the coverage drift test demonstrates every consuming context handles every motion, and a deliberately-added dummy motion fails it (FR-5).
- CLI: journey transcript — a scratch repo with 200 changed files: in the git panel, `Ctrl-d` pages, `G` jumps to bottom, `3j` steps three; same gestures shown in the annotation list — persisted to `proofs/` (FR-3, FR-4).

### Unit 2: Shared `/` filter mode for lists

**Purpose:** One consistent way to narrow any list. Serves reviewers on real-world-sized sessions.

**Functional Requirements:**

- FR-7: A reusable filter component shall provide: `/` enters filter mode, printable characters build a query, fuzzy matching reuses the file finder's existing matcher and ranking, `Esc` clears the query and exits filter mode, `Enter` locks the filter and returns key handling to the list's verbs (the help overlay's existing filter semantics).
- FR-8: The component shall be applied to: the annotation list, the staging panel, the accepted panel, and the switcher (both tabs). While a filter is active, motions (Unit 1) move within the filtered results and list verbs (`Enter`, `e`, `d`, `Space`, …) act on the filtered selection.
- FR-9: An active-filter indicator with the query text shall render in the list's chrome, and an empty result set shall render a hint line (query + "no matches — Esc to clear") rather than a blank list.
- FR-10: Filter-mode keys shall live in the shared modal-key machinery with footer hints, help coverage, and drift tests; contexts gaining `/` must not shadow an existing binding (verified against each table).
- FR-11: The help overlay's own `/` filter shall be reconciled onto the shared component only if behavior-identical; otherwise it remains as-is and the divergence is documented in the module doc (no user-visible change either way).

**Proof Artifacts:**

- Test: component unit tests demonstrate query editing, fuzzy ranking (delegating to the finder's matcher), lock/clear semantics, and empty-state (FR-7, FR-9).
- Test: per-context integration tests demonstrate filter + motion + verb composition (filter the staging panel, `j`, `Space` unstages the correct filtered entry) (FR-8).
- CLI: journey transcript — 30 annotations, `/`, type three characters, list narrows, `Enter` locks, `e` edits the right one — persisted to `proofs/` (FR-7, FR-8).

### Unit 3: Review-launcher adoption (after spec 09)

**Purpose:** The newest lists in the app get the shared mechanisms, closing the ordering contract from questions round 1.

**Functional Requirements:**

- FR-12: The Review launcher's Branches and Commits tabs shall consume the motion layer (including counts) and the `/` filter component. Filtering the Branches tab must not bypass the launcher's in-session guard (spec 09 FR-10): a filtered `Enter` obeys the same block.
- FR-13: The launcher's modal key table gains the filter-mode rows with footer hints and help coverage, staying config-remappable; the motion coverage drift test (FR-5) includes both launcher tabs as consuming contexts.

**Proof Artifacts:**

- Test: launcher integration tests demonstrate fuzzy-filtering branches and commits, motion within filtered results, and the in-session guard holding under a filtered `Enter` (FR-12).
- CLI: journey transcript — many-branch scratch repo: `R`, `/`, type fragment, `Enter` starts review of the right branch — persisted to `proofs/` (FR-12).

## Non-Goals (Out of Scope)

1. **New motions beyond the existing set**: no `w`/`b`, marks, or jump-list this round — the layer makes adding them cheap later; this spec only unifies what exists. (Jump-list was previously ruled out for terminal key-collision reasons; that decision stands.)
2. **Filtering free-text contexts**: the file finder and project search already are search; compose and other text inputs are untouched.
3. **Filtering the diff view itself**: `/` in the diff remains content search, not list filtering.
4. **Changing the finder's always-on typing UX**: the finder keeps its query-first model; `/`-mode is for lists with letter verbs (questions round 1).
5. **Persisted filter state**: filters are transient per-open; nothing is remembered across reopens.

## Design Considerations

- Filter-mode entry/exit must feel identical everywhere: same `/` trigger, same indicator styling, same `Esc`/`Enter` contract — one idiom, learned once.
- The active-filter indicator reuses the help overlay's existing filter-line styling for visual consistency.
- Motion behavior under an active filter follows the filtered view (motions move through what the user sees), never the underlying unfiltered list.

## Repository Standards

- Data-driven invariants throughout: the motion set and filter keys are shared tables; the FR-5 coverage drift test is the spec's structural heart. No loose match arms.
- `docs/rust-best-practices.md` applies in full; the motion layer and filter component are pure/presentation-side modules unit-testable without the app; no render-loop blocking; tempdir-only integration tests.
- Refactor commits (diff-view motion migration, per-list `j`/`k` replacement) strictly separate from behavior commits (new motions in new places, filter mode) — this spec is unusually refactor-heavy, so the boundary discipline matters more than usual.
- All four gates per commit; conventional commits; perf tripwires stay green — motion dispatch through the layer must not add per-keystroke overhead measurable by the existing tripwires, and filtering a 5k-row list must not stall the render loop.

## Technical Considerations

- **Ordering**: after spec 09 (Unit 3 targets the launcher; Unit 1's dispatch refactor should build on 09's final dispatch shape) and after spec 11 (11 adds panel verbs this spec's motions compose with; both touch `handle_panel_key`). Recommended landing order: 09 → 11 → 12. Spec 10 is independent (its help-overlay work is reconciled, not restructured, by FR-4/FR-11).
- **Count machinery relocation** (FR-3) is the riskiest refactor: digit interception currently lives in the `Normal`/`Visual` dispatch arm ahead of table resolution. Moving it into the shared layer must preserve the diff view's interaction between counts, pending prefixes (`g`/`z`), and spec 10's which-key (which keys off pending-prefix state — coordinate if 10 has landed; the pending-prefix machine itself is not moving).
- **Fuzzy matcher reuse**: the finder's matcher is the single matching implementation; if it needs extraction into a shared module, that's a move-only refactor commit.
- **Peek and help reconciliation** (FR-4) is opportunistic: adopt where behavior-identical, document divergence where not — no user-visible change is acceptable from reconciliation alone.
- No new dependencies (the fuzzy matcher already exists in-repo). No latest-standards research needed — internal design; prior art (vim count/motion model, telescope-style filtering) was considered in the design discussion.

## Security Considerations

No new security surface: navigation and in-memory filtering only; no new git operations, I/O, or persistence. Proof artifacts contain no secrets.

## Success Metrics

1. **Journey A (big changeset)**: 200-file scratch repo — git panel navigable with `Ctrl-d`/`G`/`3j`; transcript persisted.
2. **Journey B (find the annotation)**: 30 annotations — `/` + three characters + `Enter` + `e` edits the intended one; transcript persisted.
3. **Journey C (branch pick at scale)**: many-branch scratch repo — `R`, `/`, fragment, `Enter` starts the right review; transcript persisted.
4. **Structural guarantee**: the FR-5 coverage drift test is green across all consuming contexts, and a dummy-motion negative test proves it bites; all four cargo gates and perf tripwires pass.

## Open Questions

1. **Filter persistence while a list stays open**: locked filters survive within a single open (definite); whether the switcher should re-open pre-filtered is deferred — transient-per-open ships (Non-Goal 5). Non-blocking.
2. **Help-overlay reconciliation depth** (FR-11): adopt-if-identical is the contract; the implementation decides which side of it falls out, documented either way. Non-blocking.
3. **`2Ctrl-d` count-with-paging**: counts compose with all motions in the shared layer by construction; if dogfooding shows count+paging is never used, nothing needs removing — it's free. Non-blocking.
