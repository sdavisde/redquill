# 12-audit-list-motions-filtering.md

## Executive Summary

- Overall Status: PASS
- Required Gate Failures: 0
- Flagged Risks: 2

## Gateboard

| Gate | Status | Why it failed (<=10 words) | Exact fix target |
| --- | --- | --- | --- |
| Parent-task user-verifiability | PASS | — | — |
| Parent-task vertical slicing | PASS | — | — |
| Requirement-to-test traceability | PASS | — | — |
| Proof artifact verifiability | PASS | — | — |
| Repository standards consistency | PASS | — | — |
| Open question resolution | PASS | — | — |

Gate evidence, compact:

- **User-verifiability**: each parent task opens with an observable behavior a non-technical user can perform (`Ctrl-d` pages the git panel; `/` + typing narrows a list; `R` → `/` → `Enter` starts the right review). No code-level phrasing.
- **Vertical slicing**: parent tasks map 1:1 to the spec's three demoable units (motion layer everywhere / filter mode everywhere / launcher adoption); refactors (count relocation, diff-view migration) are sub-tasks inside the vertical they serve, not parents.
- **Traceability**: FR-1→1.1, FR-2→1.2, FR-3→1.3/1.4, FR-4→1.5+1.6, FR-5→1.6, FR-6→1.7, FR-7→2.1, FR-8→2.2–2.4+integration tests, FR-9→2.1/2.2, FR-10→2.5, FR-11→2.6 (existing help-filter tests as preservation proof), FR-12→3.1/3.2+guard test, FR-13→3.1/3.3 (drift coverage). Every FR has ≥1 planned test artifact.
- **Proof artifacts**: all name exact test files/functions, transcript paths under `12-proofs/`, and the four cargo gates — observable, reproducible, scope-linked, sanitized (scratch repos in tempdirs only, no secrets).
- **Open questions**: all three spec open questions are explicitly non-blocking with shipped assumptions (transient-per-open filters; adopt-if-identical help reconciliation; count+paging free by construction).

## Standards Evidence Table (Required)

| Source File | Read | Standards Extracted | Conflicts |
| --- | --- | --- | --- |
| `CLAUDE.md` (repo root) | yes | Four gates per commit; keymap/modal tables as data with drift tests; perf tripwires are contracts; agent write ceiling = staging only | clippy invocation (see below) |
| `docs/rust-best-practices.md` | yes | No panics in production code; refactor and behavior commits never share; move-only refactors keep identical test counts; tempdir-only integration tests | clippy invocation (see below) |
| `README.md` (repo root) | yes | Product vision (review checkpoint, `?` help contract, `/` idioms); user-facing key contracts to preserve | none |
| `AGENTS.md` | not found | — | — |
| `CONTRIBUTING.md` | not found | — | — |
| `.github/pull_request_template.md` | not found | — | — |

**Conflict + precedence decision**: `CLAUDE.md` shows `cargo clippy -- -D warnings`; `docs/rust-best-practices.md` requires `--all-targets` so test code is linted. Precedence: `cargo clippy --all-targets -- -D warnings` (the superset — satisfies both). Recorded in the task list Notes.

## Findings (Only include when non-empty)

### FLAG Findings (max 2 in main report)

1. Count-machinery relocation (task 1.3) is the highest regression-risk refactor in the plan.
   - Risk: count/pending-prefix/which-key interplay (`3gg` count survival, Esc cancel, footer strips) breaks subtly while unit tests stay green.
   - Suggested remediation: already mitigated in-plan — task 1.3 pins the exact preservation invariants and requires the existing `footer_tests.rs` pending-prefix tests and diff-view motion tests to pass unchanged; keep it a standalone commit so a bisect isolates it. No task edit required.

2. Peek/help motion reconciliation (task 1.5) risks user-visible drift under "adopt where behavior-identical".
   - Risk: peek scrolls hover text as well as list rows; forcing it onto the layer could change scroll feel.
   - Suggested remediation: already bounded — FR-4/FR-11 make reconciliation opportunistic with documented divergence as the sanctioned fallback; treat "no user-visible change from reconciliation alone" as the acceptance bar. No task edit required.

## User-Approved Remediation Plan

- Not applicable — no REQUIRED failures; both FLAGs are accepted risks with in-plan mitigations (batch-mode invocation pre-approved proceeding).

## Chain-of-Verification (Phase 4A)

1. Initial assessment complete (gateboard above).
2. Self-questioning: all REQUIRED gates pass with explicit evidence — yes; evidence lines cite task IDs and file paths.
3. Fact-check: FR map re-walked against spec §Functional Requirements and task file §Tasks; standards rows re-checked against the three read files; launcher guard function (`confirm_launcher_branch_review` / `in_review_session()`) confirmed present on main.
4. Inconsistencies: none found after re-walk.
5. Final synthesis: PASS — planning is ready for Phase 3 implementation.
