# 09-audit-review-launcher.md

Phase-4 planning audit of `09-tasks-review-launcher.md` against `09-spec-review-launcher.md` (FR-1..FR-14), the ratified questions round 1, and the repository standards sources. Chain-of-Verification performed: each gate claim below was re-checked against the spec text, the finished task file, and the standards sources before finalizing.

## Executive Summary

- **Overall Status: PASS**
- **Required Gate Failures: 0**
- **Flagged Risks: 0**

All four REQUIRED gates pass; both FLAG gates were evaluated and raised nothing actionable, so per the minimal-report rule no Findings section follows.

## Gateboard

| Gate | Status | Why it failed (<=10 words) | Exact fix target |
| --- | --- | --- | --- |
| Requirement-to-test traceability (FR-1..FR-14 → planned test artifact) | PASS | — | — |
| Proof artifact verifiability (observable, reproducible, scope-linked, sanitized) | PASS | — | — |
| Repository standards consistency (sources read, conflicts documented) | PASS | — | — |
| Open question resolution (non-blocking, assumptions recorded) | PASS | — | — |
| FLAG: regression-risk blind spots | PASS | — | — |
| FLAG: non-goal leakage | PASS | — | — |

Verification notes per gate:

- **Traceability**: FR-1→1.2, FR-2→1.1/1.3, FR-3→1.5 (+2.5 remap test), FR-4→2.5/2.6 (incl. free-text literal-`R` tests), FR-5→2.2/3.5, FR-6→2.2, FR-7→1.4/2.3/2.4, FR-8→3.1/3.2, FR-9→3.4/3.5, FR-10→3.3, FR-11→4.1/4.2, FR-12→4.4, FR-13→4.3, FR-14→4.5. Every FR maps to at least one planned test; journeys A/B/C additionally cover FR-4/5/8/11/12/13/14 with persisted transcripts.
- **Verifiability**: every artifact names a concrete mechanism (test module or exact key sequence), a persisted location (`docs/specs/09-spec-review-launcher/proofs/`), and its FR/metric link; worktree-exercising proofs are pinned to agent-created scratch tempdir repos (sanitized, reproducible); no "should work"-style language remains.
- **Standards consistency**: CLAUDE.md, README.md, and docs/rust-best-practices.md read with 3 standards extracted each (see Standards Evidence Table); AGENTS.md, CONTRIBUTING.md, and .github/pull_request_template.md are **absent — recorded**. No conflicts found between sources; none left undocumented. Task-file commit boundaries honor the refactor-vs-behavior rule explicitly (1.2/1.3 refactor vs 1.5 feat; 2.1 refactor vs 2.5 feat).
- **Open questions**: the spec's two Open Questions are explicitly non-blocking with recorded assumptions — Q1 (auto-expand on empty) ships hint-first per the ratified dogfood decision, FR-13 is definite, and task 5.4 records the experiment for follow-up; Q2 (cross-run tab/toggle persistence) is assumed out, memory is process-lifetime per FR-6/FR-12.
- **FLAG regression-risk**: the highest-risk changes (global-row migration, `R`→launcher, refresh→`r`) each carry explicit before/after regression pins (1.1 behavior pins committed green pre-migration; 2.6 rewrites the stale pins at keymap.rs ~1128/1132/1136; free-text contexts pinned to literal `R`); the help.rs silent-drop filter hazard and the parallel-list hazard are dedicated sub-tasks (1.4, 2.4), not incidental. The transient no-in-app-branch-review-entry state between tasks 2.0 and 3.0 is documented in the task file with the CLI `--review` path remaining available — a disclosed sequencing choice, not a blind spot.
- **FLAG non-goal leakage**: no sub-task ships a PR tab, arbitrary range entry, commit-range action, help redesign/which-key, session-semantics change, or command palette; 4.5 explicitly reuses `open_commit_view` single-commit-only per Non-Goal 3.

## Standards Evidence Table

| Source File | Read | Standards Extracted | Conflicts |
| --- | --- | --- | --- |
| `CLAUDE.md` | Yes | (1) Keymap and modal keys are data in the shared tables (`src/ui/keymap.rs`, `src/ui/modal_keys.rs`), never loose match arms; every user-visible action reachable from the keymap and listed in `?` help. (2) Agent write ceiling during tasks: staging/unstaging only — no worktree add/remove/prune, fetch/pull/push, or product-commit against the user's real repo; `--review`/worktree testing only in scratch tempdir repos. (3) Perf tripwires in `src/ui/perf_tests.rs` enforce the complexity class — keep passing, never loosen budgets. | None |
| `README.md` | Yes | (1) Product promise: `?` shows the list of keybinds — help must stay truthful as bindings move. (2) Vision: redquill is the human checkpoint between agent output and commit — the launcher serves the review-agent-commit journey. (3) Don't promise unbuilt features in present tense (no dead PR-tab UI). | None |
| `docs/rust-best-practices.md` | Yes | (1) No `unwrap`/`expect`/panic macros in production code; typed errors in `git/`. (2) Four gates before every commit (`cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`); conventional commits; refactors and behavior changes never share a commit. (3) Data-driven invariants with bidirectional drift tests (keybindings ↔ help ↔ footer); background work off the render loop with generation guards; TDD for pure parsers; integration tests in canonicalized tempdirs only. | None |
| `AGENTS.md` | Not found | — | — |
| `CONTRIBUTING.md` | Not found | — | — |
| `.github/pull_request_template.md` | Not found (`.github/` contains only `workflows/`) | — | — |
| `docs/specs/08-spec-branch-review-mode/08-tasks-branch-review-mode.md` | Yes (format precedent) | (1) Parent-task format: checkbox title, "Covers:" line, Proof Artifact(s) with FR references, numbered sub-task list. (2) Proofs captured into a spec-local `proofs/` directory (gitignored). (3) Tempdir isolation is a first-class blocking requirement (2026-07-16 incident): canonicalize paths, pin git calls inside the tempdir, shared isolation-assertion helper before any mutating git call. | None |
