# 10-audit-help-discoverability.md

Phase-4 planning audit of `10-tasks-help-discoverability.md` against `10-spec-help-discoverability.md` (FR-1..FR-13), the ratified questions round, and repository standards. Chain-of-Verification performed: every gate claim below was re-checked against the spec text, the finished task file, and the standards sources before finalizing.

## Executive Summary

- **Overall Status:** PASS
- **Required Gate Failures:** 0
- **Flagged Risks:** 0

All four REQUIRED gates pass; per audit convention the report is kept minimal (no Findings section). Verification notes per gate:

- **Traceability:** FR-1→2.2; FR-2→2.3+2.5(a); FR-3→2.5(b); FR-4→2.1+2.4+2.5(c); FR-5→2.5; FR-6→3.1; FR-7→3.3+3.6; FR-8→3.1 (build-breaker)+3.3 (omission); FR-9→3.4; FR-10→4.1+4.5; FR-11→4.4; FR-12→4.3+4.4+4.5; FR-13→4.5. Every FR maps to at least one named planned test artifact.
- **Proof verifiability:** every proof artifact is a named test or a named transcript file under `docs/specs/10-spec-help-discoverability/proofs/` (gitignored per repo convention), labeled with terminal size (80×24 for Journey A) and the Scope::Global-vs-fallback path in effect; journeys are scripted (reproducible); scratch repos in tempdirs where repo state matters (sanitized); each artifact states which FR it demonstrates (scope-linked). Which-key timing proofs use injected elapsed values, no real sleeps.
- **Standards consistency:** CLAUDE.md, README.md, and docs/rust-best-practices.md read with standards extracted (table below). AGENTS.md, CONTRIBUTING.md, and .github/pull_request_template.md absent — recorded. The single conflict found (FR-6 example phrase vs. shipped clipboard-on-quit behavior, commit `4322f5a`) is documented in the Standards Evidence table and remediated by dedicated sub-task 3.2 — no undocumented conflicts.
- **Open question resolution:** all three spec Open Questions are explicitly non-blocking with recorded assumptions (OQ1 wording → sub-tasks 3.1/3.2; OQ2 500 ms compile-time const → sub-task 4.2; OQ3 rare-mode asymmetry accepted → "This context" origin handling in 2.3). The spec-09 ordering dependency is an explicit documented assumption, not implicit: stated in the task-file intro, annotated on sub-tasks 2.3, 3.1, 3.7, and 5.2, with the documented fallback for FR-2's "Works everywhere" section and an explicit no-fallback/09-must-land-first statement for Journey B.
- **FLAG gates:** Regression-risk blind spots — none flagged: the two riskiest touches (help-state consolidation; event-loop `Instant`-beside-pending signature change) each carry explicit regression machinery (1.1/1.5 move-only invariant with baseline test-count comparison; 4.4 byte-identical resolution pins; perf tripwires required unmodified in 1.5/4.6/5.3). Non-goal leakage — none: no config delay knob (compile-time const, 4.2), no startup nudge, no new bindings beyond the help modal's tab keys (2.2), which-key limited to table-derived two-key prefixes (4.1), no command palette; sub-task 5.4 re-checks Non-Goals at closure.

## Gateboard

| Gate | Status | Why it failed (<=10 words) | Exact fix target |
| --- | --- | --- | --- |
| Requirement-to-test traceability (REQUIRED) | PASS | — | — |
| Proof artifact verifiability (REQUIRED) | PASS | — | — |
| Repository standards consistency (REQUIRED) | PASS | — | — |
| Open question resolution (REQUIRED) | PASS | — | — |
| Regression-risk blind spots (FLAG) | PASS | — | — |
| Non-goal leakage (FLAG) | PASS | — | — |

## Standards Evidence Table

| Source File | Read | Standards Extracted | Conflicts |
| --- | --- | --- | --- |
| `CLAUDE.md` | Yes | (1) Keymap/modal keys are data in the shared tables (`src/ui/keymap.rs`, `src/ui/modal_keys.rs`), never loose match arms; every user-visible action reachable from the keymap and listed in `?`. (2) All four gates (`cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`) before any task is done. (3) Perf tripwires in `src/ui/perf_tests.rs` enforce the complexity class — never loosen budgets. Agent write ceiling during tasks: staging only. | None |
| `README.md` | Yes | (1) `?` is the promised discoverability surface ("press `?` to see the list of keybinds"). (2) Session end copies annotations to the clipboard (commit `4322f5a`) in addition to stdout emission. | Documented: spec FR-6's example phrase "Quit and print annotations" predates clipboard-on-quit; editorial per spec Open Question 1; remediated by sub-task 3.2. |
| `docs/rust-best-practices.md` | Yes | (1) Data-driven invariants: behavior and documentation render from one const table with bidirectional drift tests. (2) Nothing blocks the render loop; time-dependent logic tested with injected values, not sleeps; no panic macros in production code; presentation logic factored into pure functions. (3) Refactors and behavior changes never share a commit; move-only refactors prove identical test counts and zero assertion edits. | None |
| `AGENTS.md` | Not found | — | — |
| `CONTRIBUTING.md` | Not found | — | — |
| `.github/pull_request_template.md` | Not found | — | — |
| `docs/specs/08-spec-branch-review-mode/08-tasks-branch-review-mode.md` | Yes (format precedent) | (1) Parent tasks are user-verifiable vertical slices with "Covers:" notes and per-task proof artifacts. (2) Transcripts/screenshots persist under `docs/specs/<spec>/proofs/` (gitignored — confirmed in `.gitignore` line 6). (3) Gates + conventional commits restated per task. | None |
