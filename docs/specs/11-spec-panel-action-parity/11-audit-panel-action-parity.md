# 11-audit-panel-action-parity.md

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
| Regression-risk blind spots | FLAG | See Findings | `## Tasks > 1.2`, `## Tasks > 1.11` |
| Non-goal leakage | PASS | — | — |

### Gate evidence

- **Traceability:** FR-1→1.3/1.4, FR-2→1.5, FR-3→1.6, FR-4→1.7, FR-5→1.8, FR-6/7→2.1/2.2, FR-8→2.3, FR-9/10→3.3/3.4, FR-11→3.1, FR-12→3.2/3.4/3.5. Every FR has at least one planned test artifact.
- **Verifiability:** proof artifacts name exact test files, the `cargo test` command, persisted transcript paths under `proofs/`, and per-unit user-run UI demo scripts with recorded verdicts — observable, reproducible, scope-linked, sanitized (scratch tempdir repos only, no secrets).
- **Standards:** 6 sources read (root `README.md`, `CLAUDE.md`, `docs/rust-best-practices.md`, `Cargo.toml`, `.cargo-husky/hooks/*`, `.github/workflows/ci.yml`); `AGENTS.md`/`CONTRIBUTING.md`/PR template not found (recorded). One nuance documented and resolved: main-table keys route to `src/ui/keymap.rs`, modal keys to `src/ui/modal_keys.rs` — all spec 11 rows are main-table (Diff/Panel scope), so `keymap.rs` is correct.
- **Open questions:** both spec open questions (directory-row bulk apply; `x` mnemonic) are explicitly non-blocking with definite defaults ratified in FR-3 and FR-12.

## Standards Evidence Table (Required)

| Source File | Read | Standards Extracted | Conflicts |
| --- | --- | --- | --- |
| `AGENTS.md` | not found | — (agent guidance lives in `CLAUDE.md`) | none |
| `README.md` (root) | yes | README key table is a contract the keymap must match; stage=keep / annotate=fix model | none |
| `CONTRIBUTING.md` / `.github/pull_request_template.md` | not found | — | none |
| `CLAUDE.md` (root) | yes | four cargo gates; keymap-as-data in shared tables, no loose match arms; perf tripwires never loosened; agent write ceiling = staging only | routing nuance keymap.rs vs modal_keys.rs — resolved above |
| `docs/rust-best-practices.md` | yes | no production panics; bidirectional drift tests; TDD for pure code; refactor and behavior never share a commit; tempdir-only integration tests | none |
| `Cargo.toml` / `.cargo-husky/hooks/*` / `.github/workflows/ci.yml` | yes | pre-push + CI run the same four gates, clippy `--all-targets -- -D warnings`; no new dependencies | none |

## Findings (Only include when non-empty)

### FLAG Findings (max 2 in main report)

1. Guard relaxation touches the diff view's staging hot path.
   - Risk: relaxing `staging::toggle_stage`'s `Mode::Normal|Visual` guard (task 1.2) could subtly alter diff-view staging behavior; existing tests cover current semantics but no task asserts them *unchanged* post-refactor.
   - Suggested remediation: task 1.2 already carries the move-only invariant (identical test counts, zero assertion edits) — during 1.2, additionally run the existing `tests/git_stage_integration.rs` suite before and after and note identical results in the commit message. No task-file change required.
2. Uncommitted which-key withdrawal shares files with this spec.
   - Risk: if implementation starts before the withdrawal is committed, spec 11 diffs tangle with an unrelated removal, breaking the one-change-per-commit convention.
   - Suggested remediation: task 1.1 is the explicit gate; treated as a hard stop, not advisory.

## User-Approved Remediation Plan

- Not required — no REQUIRED gate failures. FLAG mitigations are already embedded in tasks 1.1 and 1.2.
