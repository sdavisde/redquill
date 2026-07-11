# 03-audit-multibuffer-review.md

## Executive Summary

- Overall Status: PASS
- Required Gate Failures: 0
- Flagged Risks: 2

## Gateboard

| Gate | Status | Why it failed (<=10 words) | Exact fix target |
| --- | --- | --- | --- |
| Requirement-to-test traceability | PASS | — | — |
| Proof artifact verifiability | PASS | — | — |
| Repository standards consistency | PASS | — | — |
| Open question resolution | PASS | — | — |
| Regression-risk blind spots | FLAG | see Findings 1 | — |
| Non-goal leakage | FLAG | see Findings 2 | — |

Traceability notes: Unit 1 FRs → tasks 2.1/2.2 test artifacts; Unit 2 FRs → 3.1–3.4 `FakeGit` app-level tests; Unit 3 FRs → 4.1–4.5 tests (annotations/search/LSP/select-by-path/target gating) plus unchanged `annotate/markdown.rs` assertions; side-by-side removal → 1.3 keymap test + README diff; performance FR → 5.1 cache tests + `proofs/perf-5k-diff.md` (manual bar, per spec).

Open questions resolved as explicit assumptions (recorded in tasks Notes): keys `S`/`za` ratified via README update, `zM`/`zR` optional; `+N −M` collapsed-header summary optional; initial collapse = fully-staged files collapsed.

## Standards Evidence Table (Required)

| Source File | Read | Standards Extracted | Conflicts |
| --- | --- | --- | --- |
| `AGENTS.md` | not found | — | — |
| `CONTRIBUTING.md` | not found | — | — |
| `.github/pull_request_template.md` | not found | — | — |
| `CLAUDE.md` | yes | Four cargo gates per task; TDD for pure code; no `unwrap()`/`expect()` outside tests; keymap is data, every action in `?` overlay; conventional commits | none |
| `README.md` | yes | Keymap table is the public contract; annotation stdout format is a public API; vim-grammar keys | none |
| `.github/workflows/ci.yml` | yes | CI enforces the same four gates on ubuntu + macos | none |

## Findings (Only include when non-empty)

### FLAG Findings (max 2 in main report)

1. Performance acceptance is manual-only
   - Risk: the 5k-line instant-feel bar is validated by transcript, not an automated regression test; a later change could regress silently.
   - Suggested remediation: none required by spec (it defines a manual bar); task 5.2 records measured numbers in `proofs/perf-5k-diff.md` so future work has a baseline.

2. Task 5.1 changes refresh-time cache invalidation behavior
   - Risk: per-file invalidation is an implementation improvement the spec only implies ("computed lazily per file … and cached"); scope is adjacent to, not named by, an FR.
   - Suggested remediation: keep 5.1 minimal — invalidate by changed file only; no persistence or config surface.

## User-Approved Remediation Plan

- Not required: all REQUIRED gates pass; FLAG findings are informational.
