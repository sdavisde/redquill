# 11-spec-panel-action-parity.md

## Introduction/Overview

A parity audit found several places where the same user intent works in one context but not a sibling context that presents the same object. The worst offender, reported directly from dogfooding: the git panel highlights a file, but `Space`/`S` (stage/unstage — or accept in a review session) do nothing there; the user must leave the panel to act on the file they're pointing at. In review mode the panel even *renders* per-file tri-state markers (`●` accepted, `~` deferred) that no panel key can change. This spec closes the mechanical action-parity gaps: panel file actions (stage/accept/defer), panel coherence keys (`Esc`, `s`, `/`), and in-place annotation edit/delete from the diff view.

Guiding principle (shared with specs 09/10/12): the tool should feel holistic — where two contexts show the same object, they answer to the same verbs.

## Goals

- Stage/unstage, and in review mode accept/defer, the highlighted file directly from the git panel using the same keys the diff view uses — zero new git-layer code, reusing the existing file-cursor sync.
- Make the git panel a coherent citizen: `Esc` leaves it, `s` and `/` work from it, and its footer advertises the file actions it now supports.
- Let a reviewer edit or delete an annotation from the diff view where it is visibly rendered, instead of detouring through the annotation list (and stop `c` from silently creating duplicates as the only option).
- Keep every new key in the shared keymap tables with help/footer drift tests passing — no loose match arms, no hidden features.

## User Stories

- **As a reviewer scanning the git panel's file tree**, I want to press `Space` or `S` on the highlighted file to stage or unstage it so that I can stage a changeset without bouncing between panel and diff.
- **As a reviewer in a branch-review session**, I want `Space`/`S`/`d` in the panel to accept or defer the highlighted file so that the tri-state markers I can see are also markers I can change.
- **As a user anywhere in the app**, I want `Esc` to back out of the git panel like it backs out of everything else so that leaving a context never requires a context-specific key.
- **As a reviewer reading an annotation inline in the diff**, I want to edit or delete it right there so that fixing a typo in my own comment doesn't require opening a separate panel and finding it again.

## Demoable Units of Work

### Unit 1: Panel file actions — stage, unstage, accept, defer

**Purpose:** The motivating fix. Serves reviewers driving a changeset from the panel.

**Functional Requirements:**

- FR-1: In the git panel's Changes tab with a file row highlighted, `Space` shall toggle-stage and `S` shall stage that file, with semantics identical to the diff view's whole-file gestures, reusing the existing stage machinery (the panel already syncs the diff cursor to the highlighted file via `panel_follow`). The stage functions' current `Normal`/`Visual` mode guard shall be relaxed or routed rather than duplicated.
- FR-2: During an active review session, panel `Space`/`S` shall translate to accept-file gestures and `d` shall toggle-defer, mirroring the diff view's existing review-session dispatch translation, and the panel's tri-state markers shall update immediately.
- FR-3: On a directory row, these keys shall be a no-op with a status-line hint (no recursive apply); on the History tab they shall be inert. Capability gating applies: in read-only diff targets (commit view, ranges) the keys are inert and their hints hidden, exactly as in the diff view.
- FR-4: The panel footer shall advertise stage/accept/defer per the active mode by removing the current deliberate suppression of review-status hints in panel scope; hints come from the shared tables and are capability-gated.
- FR-5: All new bindings live in the `Scope::Panel` keymap table (or ride spec 09's dispatch translation pattern), appear in the `?` help overlay's panel section, and keep the bidirectional drift tests passing.

**Proof Artifacts:**

- Test: panel dispatch tests demonstrate `Space`/`S` stage/unstage the highlighted file in working-tree mode and translate to accept/`d` defer in a review session, with directory rows and History tab inert (FR-1, FR-2, FR-3).
- Test: footer tests demonstrate panel hints now include stage/accept/defer, capability-gated (FR-4); drift tests pass (FR-5).
- CLI: journey transcript on a scratch tempdir repo — open panel, `j` to a file, `Space` stages it, staged marker updates in place; same flow in a review session accepts the file and the `●` marker appears — persisted to `proofs/` (FR-1, FR-2).

### Unit 2: Panel coherence — `Esc` leaves, `s` and `/` reach through

**Purpose:** Remove the small frictions that make the panel feel like a separate app. Serves everyone.

**Functional Requirements:**

- FR-6: `Esc` in the git panel shall close the panel and return to `Normal` mode, with the same overlay shadowing as elsewhere (an open help overlay consumes `Esc` first). The existing `` ` `` toggle remains.
- FR-7: `s` in the panel shall open the staging panel and `/` shall enter search, each behaving as if the panel were closed first (focus lands where the diff-view invocation of the same key lands); on exit, focus returns to `Normal` as those features already do.
- FR-8: Both keys are panel-scope table rows with help/footer coverage and passing drift tests.

**Proof Artifacts:**

- Test: dispatch tests demonstrate `Esc` closes the panel (and does not when the help overlay is open), and `s`/`/` from the panel land in the staging panel and search respectively (FR-6, FR-7).
- CLI: journey transcript — `` ` `` open panel, `Esc` back out; `` ` ``, `/`, type a query, land on a match — persisted to `proofs/` (FR-6, FR-7).

### Unit 3: Edit and delete annotations from the diff view

**Purpose:** Close the reverse gap — annotations are visible inline but only mutable from the list. Serves annotating reviewers.

**Functional Requirements:**

- FR-9: With the diff cursor on a line overlapped by an annotation's target (line, range, hunk, or file for file-level annotations when on the file-header row), `e` shall open the existing in-place edit compose (`open_compose_for`) for that annotation. If no annotation overlaps, `e` is a no-op with a status hint.
- FR-10: `x` in the same circumstances shall delete the overlapping annotation, with behavior (confirmation or not) identical to the annotation list's existing `d` delete path.
- FR-11: When multiple annotations overlap the cursor line, the deterministic rule is: the annotation whose target starts nearest above-or-at the cursor line wins; ties broken by creation order (oldest first). The rule shall be a pure, unit-tested function.
- FR-12: `e` and `x` are diff-scope keymap rows (both currently unbound), config-remappable like all main-table rows, present in help and footer with drift tests passing. `c` behavior is unchanged (always composes a new annotation).

**Proof Artifacts:**

- Test: unit tests for the overlap-resolution rule (pure function) demonstrate deterministic selection including multi-overlap and file-level cases (FR-11).
- Test: dispatch tests demonstrate `e` opens edit-in-place pre-filled with the existing body, `x` deletes with list-parity semantics, and both no-op with a hint when nothing overlaps (FR-9, FR-10).
- CLI: journey transcript — annotate a line, move away, return, `e`, change the text, submit; then `x` deletes it and the inline row disappears — persisted to `proofs/` (FR-9, FR-10).

## Non-Goals (Out of Scope)

1. **Annotating from the git panel**: explicitly rejected in questions round 1 (gap 9) — `c` keeps meaning commit in panel scope.
2. **Panel fast navigation (paging, `gg`/`G`, counts) and list filtering**: spec 12 (shared motion layer + `/` filter component).
3. **Hunk- or line-level staging from the panel**: panel actions are whole-file only; finer granularity stays in the diff view where the hunks are visible.
4. **Recursive directory stage/accept**: directory rows are a hinted no-op this round; bulk-apply is a possible follow-up after dogfooding.
5. **Changing existing stage/accept semantics**: only new entry points to existing operations; the operations themselves are untouched.

## Design Considerations

- Panel file actions must feel like the *same* gesture as in the diff, not a parallel feature: same keys, same footer wording, same capability gating.
- The status hints for no-op cases (directory row, no overlapping annotation) use the existing status-line pattern — quiet, non-modal.
- `x` for delete follows vim's "excise" mnemonic; `e` for edit matches the annotation list's existing `e`. Both are remappable if the user disagrees.

## Repository Standards

- All keys land in the shared tables (`src/ui/keymap.rs`, footer hints via `FooterHint`), never loose match arms; stable kebab-case action names; bidirectional help/dispatch drift tests must pass.
- `docs/rust-best-practices.md` applies in full: no production panics, pure functions for the overlap rule (unit-testable without the app), no render-loop blocking, integration tests in canonicalized tempdirs only.
- All four gates before every commit; conventional commits; the mode-guard relaxation (refactor) commits separately from the new bindings (behavior).
- Perf tripwires stay green.

## Technical Considerations

- **Ordering**: lands after spec 09. Spec 09's task 2.0 rewrites the same dispatch region (`R` rebind, launcher arm) and its `Scope::Global` machinery changes `handle_panel_key`'s resolution path; building 11 on top avoids a rebase-heavy collision. The review-session translation for panel keys (FR-2) should mirror whatever final shape 09 gives the diff-scope translation.
- **The one real seam**: `staging::toggle_stage` early-returns unless `Mode::Normal|Visual`; the fix routes panel invocations legitimately rather than spoofing modes — either relax the guard to include `Panel` or extract the file-targeted core the guard wraps.
- **Review ops need no guard change**: `accept_file`/`toggle_defer_file` already self-guard only on `in_review_session()` and key off `cursor_file_path()`, which `panel_follow` keeps correct.
- **Footer**: the current suppression of review hints in panel scope is a one-line deliberate exclusion; removing it is the intended fix, not a workaround.
- No new dependencies. No latest-standards research needed — internal parity work on the established stack.

## Security Considerations

No new security surface: new entry points to already-sanctioned index writes (stage/unstage) and in-memory review-state transitions, all within the existing write ceiling. Agent testing stays in scratch tempdir repos. Proof artifacts contain no secrets.

## Success Metrics

1. **Journey A (panel staging)**: open panel, highlight file, `Space` — file stages and the marker updates, without leaving the panel; transcript persisted.
2. **Journey B (panel review triage)**: in a review session, accept two files and defer one entirely from the panel; markers update live; transcript persisted.
3. **Journey C (annotation round-trip)**: annotate, edit in place with `e`, delete with `x`, all from the diff view; transcript persisted.
4. **Zero drift**: help/footer drift tests, perf tripwires, and all four cargo gates pass.

## Open Questions

1. **Directory-row bulk apply**: no-op-with-hint this round; whether `Space` on a directory should stage its subtree is deferred until dogfooding shows demand. Non-blocking (FR-3 is definite).
2. **`x` mnemonic**: if `x` proves wanted for something else later (e.g., future vim-style delete motions), the binding is config-remappable and the row is data — trivial to move. Non-blocking.
