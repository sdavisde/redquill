# 05-validation-diff-sources.md

SDD Phase 4 validation of spec 05 (diff-sources). Independent, evidence-based
verification of the implementation on branch `worktree-diff-sources`
(`main..HEAD`, 12 commits `68cf660..066fcba`) against the spec, task list, and
repository standards. Every gate below was re-run by the validator, not read
from the proof files.

## 1. Executive Summary

**Overall: PASS.** No gate tripped.

- **GATE A (CRITICAL/HIGH blocker):** PASS — zero CRITICAL or HIGH issues.
- **GATE B (FR coverage, no Unknowns):** PASS — all 25 Functional Requirements Verified, 0 Unknown.
- **GATE C (proof artifacts accessible/functional):** PASS — all 11 proof files present on disk and their backing tests/commands re-run green.
- **GATE D (file integrity, tiered):** PASS — every `src/` change maps to a task/FR; the two files not on the Relevant Files list (`switcher.rs`, `commit_message.rs`) are mechanical consequences of the ratified `Mode::Panel { tab }` change (task 3.1).
- **GATE E (repository standards):** PASS — layering, TDD, subprocess hygiene, conventional commits, refactor/behavior separation all honored.
- **GATE F (security — no real secrets in proofs):** PASS — only prose "token" and test-count arithmetic; no credentials.

**Implementation Ready: Yes.** The feature meets every spec requirement, all four cargo gates pass clean, and the six Success Metrics are demonstrated with reproducible evidence.

**Key metrics:**
- Requirements verified: 25/25 (100%).
- Proof artifacts working: 11/11 (100%).
- Files changed: 34 (24 core `src/`, 10 supporting — tests/docs/proofs), all in scope. Relevant-Files entries are all covered; `background.rs`-planned history loading landed in a dedicated `history.rs` on the same background poller (functionally equivalent, tested) — noted, not an issue.
- Gates: `cargo build` OK · `cargo test` 886 lib passed / 3 ignored / 0 failed + all integration binaries 0 failed · `cargo clippy --all-targets -- -D warnings` clean · `cargo fmt --check` clean.

### Rubric scores

| Rubric | Score | Severity |
|---|---|---|
| R1 spec coverage | 3 | OK |
| R2 proof artifacts demonstrate claims | 3 | OK |
| R3 file integrity | 3 | OK |
| R4 git traceability | 3 | OK |
| R5 evidence quality | 3 | OK |
| R6 repository compliance | 3 | OK |

## 2. Coverage Matrix

### 2.0 Success Metrics / Task 6.0 (graded first — user-facing definition of done)

| # | Success Metric | Status | Evidence (independently re-verified) |
|---|---|---|---|
| SM1 | Dead-end disappears (≤5 keys launch→newest commit's diff) | **Verified** | `proofs/dead-end-journey.md`: 3 keys (`` ` ``→`Tab`→`Enter`), each named on-screen. Backing test `dead_end_journey_reaches_the_newest_commit_in_a_handful_of_keys` re-run: PASS. |
| SM2 | Fix-loop works on history (agent resolves every site) | **Verified** | Producer half: `proofs/round-trip-emission.md` + test `history_round_trip_producer_emits_three_sites_across_two_files_under_one_reviewing_line` re-run PASS (one `Reviewing:` line, 3 sites/2 files). Consumer half: orchestrator-run Haiku agent resolved all three sites (recorded in same file) — accepted per validation context #3. |
| SM3 | Tool never lies (listed keys work; absent caps inert) | **Verified** | `proofs/no-lies-overlay.md` + test `commit_view_help_overlay_shows_only_truthful_keys` re-run PASS; exhaustive `binding_hidden` cross-check + task-3.6 inert-behavior tests. |
| SM4 | Existing habits unbroken (working-tree stdout byte-identical) | **Verified** | `annotate/markdown` byte-exact fixtures (21 tests) re-run PASS; `add()` still defaults to `Source::WorkingTree` so existing fixtures unedited. |
| SM5 | Dogfood gate (review spec's own commits via History tab) | **Verified** | `proofs/dogfood-review.md`: 3 real issues (1 question, 2 nits) + 2 praise notes emitted through real `render_markdown` with grouped `Reviewing:` lines; issues honestly filed as non-blocking follow-ups. |
| SM6 | Still instant (perf tripwires pass, unmodified budgets) | **Verified** | `git diff main..HEAD -- src/ui/perf_tests.rs` is **empty** (byte-for-byte unmodified); tripwires pass in full `cargo test`. |

Task 6.0 sub-tasks (6.1–6.4) all check out: acceptance tests exist in `src/ui/history_integration_tests.rs`, each with a TTY-deferred operator section carrying exact reproduction steps and a headless rendered-buffer / UI-state test standing in for the screenshot — graded **Verified (TTY-deferred)** because the deferred steps are exact and the standing-in tests assert the same claims.

### 2.1 Functional Requirements

| FR (Unit) | Status | Evidence |
|---|---|---|
| U1: capability methods `is_live`/`staging_mode`/`supports_code_intel` | Verified | `src/git/diff.rs`; tests `working_tree/staged/range/commit_capability_triple` re-run PASS (4/4). |
| U1: route all 5 call sites through methods, no `matches!` capability checks remain | Verified | Commit `40312c9`; grep `matches!(.*DiffTarget` in `src/ui/` returns 5 hits, **all** non-capability (4 test assertions in `history_integration_tests.rs`, 1 FakeGit data-selection in `app_tests.rs`) — no production capability check remains. |
| U1: disable code-intel + hide keys when `!supports_code_intel()` | Verified | `src/ui/code_intel.rs` `request`/`refresh_peek_preview` early-return; `help::binding_hidden(code_intel)` + footer threading; commit `b7284fc`; tests `commit_view_hides_and_disarms_code_intel_keys` PASS. |
| U1: document degradation contract in code-intel module doc | Verified | `src/ui/code_intel.rs:9-` "Degradation contract: code-intel is silently absent off the live working tree". |
| U1: working-tree/staged behavior observably unchanged | Verified | Refactor commit `40312c9` states identical test counts / zero assertion edits; full suite green. |
| U2: `DiffTarget::Commit(rev)` = commit's own changes vs first parent | Verified | `src/git/runner.rs`; integration `commit_target_normal_commit_shows_only_that_commits_own_changes` + `..merge_commit_diffs_against_first_parent` PASS. |
| U2: root commit rendered all-added (empty-tree diff) | Verified | `commit_target_root_commit_is_an_all_added_diff_against_the_empty_tree` PASS. |
| U2: commit content from git objects (`<rev>^:<path>`/`<rev>:<path>`), never disk | Verified | `src/ui/syntax.rs` Commit arms; `content_source` unit tests PASS. |
| U2: commit capability triple `false`/`ReadOnly`/`false` | Verified | `commit_capability_triple` PASS. |
| U2: commit-log read model (sha/short_sha/subject/author/timestamp), NUL-delimited, newest-first | Verified | `src/git/log.rs`; `log::` tests re-run PASS (15/15) incl. hostile subjects. |
| U2: incremental fetch (count/skip pagination) | Verified | `runner.rs commit_log(count, skip)`; `git_log_integration` pagination tests PASS (4/4). |
| U2: closed-type argv, no shell interpolation | Verified | `runner.rs` builds discrete argv; `GIT_TERMINAL_PROMPT=0`; empty-tree fallback. |
| U3: git panel Changes⇄History tabs, panel-scoped toggle in shared tables + `?` | Verified | `TogglePanelTab` in `modal_keys.rs`/`keymap.rs` (`Tab`); `git_panel.rs` tab strip; footer drift test PASS. |
| U3: two-line rows (subject+unpushed marker; dimmed author·rel-time·short-sha) | Verified | `git_panel.rs` rendering tests; `time_format.rs` pure fn tests PASS; screenshot in `05-task-03-proofs.md`. |
| U3: async page load via background pattern, loading placeholder, page-on-scroll | Verified | `src/ui/history.rs` single-flight + generation counter on `background.rs` poller; `history_tests.rs` stale-drop + placeholder tests PASS. |
| U3: Enter opens highlighted commit in multibuffer | Verified | `app::open_commit_view`; `history_integration_tests` round-trip PASS. |
| U3: commit-view header block (short SHA/author/abs date/subject) | Verified | `diff_view.rs` header; dead-end + no-lies screenshots show it. |
| U3: commit view = same multibuffer nav + annotations | Verified | `commit_view_annotations_are_fully_functional` PASS. |
| U3: no staging/code-intel/auto-refresh in commit view, via capability model | Verified | `commit_view_hides_and_disarms_staging_keys` / `..code_intel_keys` / `..never_auto_refreshes` PASS. |
| U3: return via shared-table key (`Esc`); `q` quits+emits | Verified | `app::return_from_commit_view` restores `SuspendedView`; round-trip tests PASS. |
| U4: each annotation records its diff source | Verified | `annotate/model.rs Source`; `store::add_with_source`; `add()` defaults WorkingTree. |
| U4: working-tree group first, byte-identical to today | Verified | byte-exact backward-compat fixture unedited; markdown tests PASS. |
| U4: non-worktree groups after, one `Reviewing:` metadata line each | Verified | `markdown.rs group_by_source`; mixed-session byte-exact test PASS; producer emission shows one `Reviewing: <short-sha>`. |
| U4: metadata syntax documented (module doc + README) + byte-exact tests | Verified | `README.md:91-96` documents commit/range/staged forms; annotate module doc; 21 markdown tests. |
| U5: welcome state on zero-file target; situation text; ≥3 keyed hints from keymap; disappears on content | Verified | `src/ui/welcome.rs`; drift test `welcome_hints_resolve_for_every_spec` PASS; `mod_tests.rs` lifecycle tests; `05-task-05-welcome-buffer.txt` capture. |

### 2.2 Repository Standards

| Area | Status | Evidence |
|---|---|---|
| Four cargo gates pass | Verified | build OK; test 886 lib + integration, 0 failed; clippy `--all-targets` clean; fmt clean (re-run by validator). |
| Layering (no TUI types in `git/`) | Verified | `git/log.rs`, `runner.rs`, `diff.rs` carry no ratatui/ui types; `Source` is annotate-owned, not a `DiffTarget` re-export (documented cross-layer choice, commit `55fa2b9`). |
| TDD for pure code | Verified | log parser, capability triple, content_source, byte-exact annotation fixtures written test-first per commit messages; tests committed with code. |
| Tempdir integration tests, no host repo touch | Verified | `tests/git_integration.rs`, `git_log_integration.rs`, `history_integration_tests.rs` use tempdirs, canonicalized. |
| Background work pattern (single-flight + generation + non-blocking poll) | Verified | `history.rs` mirrors `refresh.rs InFlightRefresh` on `background.rs`. |
| Subprocess hygiene (closed argv, machine-readable, no shell) | Verified | discrete argv, NUL-delimited log format, `GIT_TERMINAL_PROMPT=0`. |
| Conventional commits; refactor vs behavior separated | Verified | `refactor(ui)` 40312c9 (move-only) and `fix(ui)` b7284fc (behavior) are separate commits, as spec requires. |
| Keymap/help drift protection | Verified | new keys in shared tables; footer + welcome drift tests present and green. |
| Perf tripwires unmodified | Verified | `git diff main..HEAD -- src/ui/perf_tests.rs` empty. |
| Write ceiling unchanged (commit view read-only) | Verified | no new git write ops; `staging_mode()==ReadOnly` on Commit; Non-Goals (restore-hunk/checkout) not implemented. |

### 2.3 Proof Artifacts

| Unit/Task | Artifact | Status | Verification result |
|---|---|---|---|
| 1.0 | `05-proofs/05-task-01-proofs.md` + capability tests, grep, code-intel gate | Working | tests re-run PASS; grep confirms no production capability `matches!`. |
| 2.0 | `05-proofs/05-task-02-proofs.md` + log parser, commit-shape, content_source tests | Working | `log::` 15 PASS, `git_integration` commit/merge/root PASS. |
| 3.0 | `05-proofs/05-task-03-proofs.md` (screenshots TTY-deferred) + round-trip/perf/drift tests | Working | round-trip UI-state tests PASS; TTY-deferred section has exact steps; standing-in rendered-buffer tests cover the same assertions. |
| 4.0 | `05-proofs/05-task-04-proofs.md` + byte-exact fixtures | Working | 21 markdown tests PASS incl. working-tree-only compat + mixed grouped. |
| 5.0 | `05-proofs/05-task-05-proofs.md` + `05-task-05-welcome-buffer.txt` | Working | welcome drift + lifecycle tests PASS; buffer capture matches. |
| 6.0 | `05-task-06-proofs.md` + `proofs/{dead-end-journey,round-trip-emission,no-lies-overlay,dogfood-review}.md` | Working | all four acceptance tests re-run PASS; each proof has front-loaded context, verbatim evidence, and a TTY-deferred operator section. |

## 3. Validation Issues

No CRITICAL, HIGH, or MEDIUM issues. The following are informational (LOW / no action required for readiness):

- **LOW — grep count evolved since task 1.2's proof.** Commit `40312c9`'s message states the grep returned "a single hit" (the FakeGit double). After task 3.0 added `history_integration_tests.rs`, the same grep now returns 5 hits — but the 4 new ones are test assertions (`matches!(app.target, DiffTarget::Commit(_))`) and the fifth is unchanged data-selection. The FR's actual requirement ("no capability-decision `matches!` at call sites") remains fully satisfied in production code. No fix needed; the proof's substantive claim holds.
- **LOW — planned file location deviation.** Task list Relevant Files anticipated background history loading in `src/ui/background.rs`; it shipped in a new `src/ui/history.rs` built on the generic `background.rs` poller. Functionally equivalent, tested (`history_tests.rs`), and consistent with the existing `refresh.rs` pattern. Documented here for traceability.
- **LOW — 3 dogfood follow-ups filed, not fixed** (`git_panel.rs:263` SHA-clip nit, `runner.rs:177` swallowed-git-error question, `git_panel.rs:256` glyph-reuse nit). This is the honest disposition the task sanctioned ("fix small / file large"); none affects a Success Metric or spec FR. Recorded as future work, not a validation defect.

## 4. Evidence Appendix

### Commits analyzed (`main..HEAD`, 12 commits)

| Commit | Type | Task/FR | Key files |
|---|---|---|---|
| `68cf660` | docs(specs) | scaffolding | spec/tasks/audit/questions |
| `611ee3b` | feat(git) | 1.0 | `git/diff.rs` (capability triple) |
| `40312c9` | refactor(ui) | 1.2 | 5 call sites → methods (move-only) |
| `b7284fc` | fix(ui) | 1.3/1.4 | `code_intel.rs`, `footer.rs`, `help.rs`, `mod.rs` |
| `7ac9a91` | feat(git) | 2.0 | `diff.rs` Commit, `log.rs` (new), `runner.rs`, `syntax.rs` |
| `d0eed59` | feat(ui) | 3.0 | `git_panel.rs`, `history.rs` (new), `time_format.rs` (new), `app.rs`, `diff_view.rs`, `modal_keys.rs`, `keymap.rs`, `modes.rs`, `switcher.rs`, `commit_message.rs` |
| `55fa2b9` | feat(annotate) | 4.0 | `annotate/{model,markdown,store,mod}.rs`, `app.rs`, `README.md` |
| `82e4b08` | feat(ui) | 5.0 | `welcome.rs` (new), `diff_view.rs`, `help.rs`, `mod.rs` |
| `664005e` | test(acceptance) | 6.0 | `history_integration_tests.rs` (+426) |
| `9fd8ce6` | docs(proofs) | 6.0 | `proofs/*.md`, `05-task-06-proofs.md` |
| `06b23ff` | docs(proofs) | 6.2 | round-trip consumer half |
| `066fcba` | chore(git) | 6.0 | gitignore proof folders (user-directed; not a traceability gap) |

File-integrity classification: 24 core `src/` files (all traced above), 10 supporting (tests, `README.md`, proofs, task file). `switcher.rs` + `commit_message.rs` — not on the Relevant Files list — changed only to add `tab: self.last_panel_tab` when reconstructing `Mode::Panel` (the field added by task 3.1); mechanically required, fully linked. GATE D1: no unexplained core change.

### Commands executed by the validator (actual results)

```
cargo build                                   -> Finished, exit 0
cargo test                                    -> 886 lib passed / 3 ignored / 0 failed; all integration binaries 0 failed
cargo clippy --all-targets -- -D warnings     -> Finished, exit 0 (clean)
cargo fmt --check                             -> exit 0 (clean)
cargo test --lib capability                   -> 4 passed (working_tree/staged/range/commit_capability_triple)
cargo test --lib log::                        -> 15 passed (incl. hostile subjects)
cargo test --lib markdown::                   -> 21 passed (byte-exact fixtures)
cargo test --lib welcome                      -> welcome_hints_resolve_for_every_spec passed
cargo test --lib history_integration          -> 14 passed (dead_end / round_trip / truthful-keys)
cargo test --test git_integration             -> 17 passed (normal/merge/root commit shapes)
cargo test --test git_log_integration         -> 4 passed (pagination)
git diff main..HEAD -- src/ui/perf_tests.rs   -> empty (unmodified)
grep -rn "matches!(.*DiffTarget" src/ui/      -> 5 hits, all in test files (assertions + FakeGit data selection)
grep -rniE "(api_key|secret|password|token|bearer|BEGIN|AKIA|ghp_|sk-)" <proof dirs> -> only prose "token"/test-count math; no credentials
```

### File-existence checks

All present on disk (untracked per user directive, commit `066fcba` — not a traceability gap):
`05-proofs/05-task-01..06-proofs.md`, `05-proofs/05-task-05-welcome-buffer.txt`,
`proofs/dead-end-journey.md`, `proofs/round-trip-emission.md`,
`proofs/no-lies-overlay.md`, `proofs/dogfood-review.md`.

New source files confirmed present: `src/git/log.rs`, `src/ui/history.rs`,
`src/ui/time_format.rs`, `src/ui/welcome.rs`,
`src/ui/history_integration_tests.rs`, `src/ui/history_tests.rs`.

**Validation Completed:** 2026-07-14 20:15 UTC
**Validation Performed By:** Claude Opus 4.8 (SDD Phase 4 validator)
