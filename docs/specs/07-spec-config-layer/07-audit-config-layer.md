# 07-audit-config-layer.md

## Executive Summary

- Overall Status: PASS (run 2 — user waiver recorded)
- Required Gate Failures: 0 (1 waived by explicit user decision)
- Flagged Risks: 2

## Gateboard

| Gate | Status | Why it failed (<=10 words) | Exact fix target |
| --- | --- | --- | --- |
| Requirement-to-test traceability | PASS (waived) | User declined absence-test for no-reload FR | — |
| Proof artifact verifiability | PASS | — | — |
| Repository standards consistency | PASS | — | — |
| Open question resolution | PASS | — | — |
| Regression-risk blind spots | FLAG | See FLAG findings | — |
| Non-goal leakage | PASS | — | — |

## Standards Evidence Table (Required)

| Source File | Read | Standards Extracted | Conflicts |
| --- | --- | --- | --- |
| `AGENTS.md` | not found | — | — |
| `CONTRIBUTING.md` | not found | — | — |
| `README.md` (root) | yes | stdout annotation format frozen; LSP never blocks review; static-binary scope | none |
| `CLAUDE.md` | yes | 4 gates before done; keymap via shared tables; lean deps justified per commit; perf tripwires unloosened | none |
| `docs/rust-best-practices.md` | yes | No prod panics; typed errors in modules; refactor/behavior commits separate; tempdir tests; drift tests for data-driven invariants | none |
| `.github/workflows/ci.yml` | yes | Same 4 gates on ubuntu+macos; clippy `--all-targets` | none |
| `.cargo-husky/hooks/pre-commit`, `pre-push` | yes (present) | pre-commit fmt; pre-push all gates | none |

## Findings (Only include when non-empty)

### REQUIRED Failures (max 3 in main report)

None open. Run-1 failure resolved by user waiver:

1. ~~Spec Unit 1 FR "read the config file exactly once, at startup ... there is no reload mechanism" maps to sub-task 1.5 (wiring) but no planned test artifact demonstrates it.~~
   - Resolution (2026-07-16): User explicitly declined adding a test artifact for this FR — "I don't care to add a test to prove we aren't completing a feature." The no-reload FR is an intentional non-feature; absence-of-behavior is not test-covered by user decision. Task 1.5 is unchanged. This waiver is the documented explicit assumption satisfying the gate.

### FLAG Findings (max 2 in main report)

1. Sub-task 1.3 treats unknown-key *collection* as routine serde work, but serde's standard options are all-or-nothing (`deny_unknown_fields` fails the parse; default silently ignores). Collecting unknown keys for the warning surface needs a two-pass parse (e.g. `toml::Value` walk against known keys) or similar.
   - Risk: underestimated effort lands as silent-ignore, violating the spec's warning contract for unknown keys.
   - Suggested remediation: note the two-pass approach explicitly in 1.3 so the implementer doesn't discover it mid-task.
2. Sub-task 4.6 assumes `help.rs` is verify-only because the overlay derives from `Keymap::bindings()`. If any footer/hint site hardcodes a key label outside the tables, task 4 scope grows.
   - Risk: hidden hardcoded key labels make "help stays truthful" silently false for that site.
   - Suggested remediation: add a grep sweep for hardcoded key literals in `ui/` to 4.6's checklist; promote findings into the task if any appear.

## User-Approved Remediation Plan

- Completed (user-modified): user rejected the proposed 1.5 test artifact and directed no task-file edits; waiver recorded above. FLAG findings 1–2 remain advisory notes for the implementer (no task edits approved).

## Re-Audit Delta (Runs 2+ only)

- Changed gate statuses since previous run: Requirement-to-test traceability FAIL → PASS (waived by explicit user decision, 2026-07-16).
- Still-failing REQUIRED gates: none.
- Newly introduced findings: none.
