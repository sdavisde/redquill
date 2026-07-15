# 06-tasks-project-search.md

Task list for `06-spec-project-search.md`.

Implementation happens on a dedicated worktree branch (repo convention for concurrent sessions), merging to `main` only with all four gates green. Proof artifacts live under `docs/specs/06-spec-project-search/proofs/` (gitignored per repo convention, commit `066fcba`).

## Relevant Files

| File | Why It Is Relevant |
| --- | --- |
| `Cargo.toml` | Adds `nucleo-matcher` (task 1.0) and `grep-searcher`/`grep-regex`/`ignore` (task 2.0), `default-features = false` where features permit; justification in the adding commits. |
| `src/search/mod.rs` | New non-UI module root: query/hit/candidate types shared by engine and UI. No TUI types (layering rule). |
| `src/search/files.rs` | New: file-candidate model; merge/dedupe of tracked + untracked-unignored lists. Pure, TDD. |
| `src/search/fuzzy.rs` | New: `nucleo-matcher` glue — rank candidates for a query with path-aware config, stable ordering. Pure, TDD. |
| `src/search/engine.rs` | New: in-process grep engine — matcher build (regex/smartcase/word/literal), parallel gitignore-aware walk, streaming sink, abort flag, caps, binary/large-file skip. |
| `src/git/runner.rs` + `src/git/` (new `ls_files.rs`) | Fixed-argv `git ls-files -z` (tracked) and `--others --exclude-standard` (untracked) runners + NUL-split parser (TDD on fixture bytes; tempdir integration test). |
| `src/git/diff.rs` | Read-only file target: `DiffTarget` variant (or equivalent capability flag) so staging/commit/LSP capabilities switch off for whole-file views. |
| `src/ui/stage_ops.rs` | Async seam methods for file-list fetch (existing `AsyncReviewBuilder`-style closures); whole-file content already available via `read_worktree_file`. |
| `src/ui/file_view.rs` | New: read-only whole-file view — synthesize all-context body from worktree content, open-at-line, suspend/restore glue. |
| `src/ui/file_finder.rs` / `src/ui/file_finder_modal.rs` | New: finder mode state + handlers / centered modal render (switcher pattern + live input). |
| `src/ui/project_search.rs` / `src/ui/project_search_view.rs` | New: search mode state (query, toggles, generation, debounce, results) + full-screen render. |
| `src/ui/app.rs` | New `Mode` variants + suspended-view bookkeeping for the two new modes. |
| `src/ui/mod.rs` | Key dispatch, per-mode render calls, per-tick poll drains for finder load + search results. |
| `src/ui/modes.rs` | Modal key handlers for the new modes (following switcher handler placement). |
| `src/ui/keymap.rs` | `gp` and `g/` two-key sequences + new actions in the shared table. |
| `src/ui/modal_keys.rs` | Key tables for finder, file-view, and search modes (incl. `Alt-c`/`Alt-w`/`Alt-r` toggles) — the `?` overlay and drift tests derive from these. |
| `src/ui/footer.rs` / `src/ui/help.rs` | Mode footers and help sections; read-only file view omits staging/commit/LSP keys (README convention). |
| `src/ui/search.rs` | Existing smartcase helper — reference for case semantics; do not duplicate logic. |
| `src/ui/perf_tests.rs` | New wall-clock tripwire for query→first-results complexity class. |
| `src/annotate/model.rs` | Target/side representation for current-file-content annotations (the `(=)` side). |
| `src/annotate/markdown.rs` | `(=)` header serialization + byte-exact tests; module doc (format contract) update. |
| `src/ui/targeting.rs` / `src/ui/annotation_list.rs` | Map file-view cursor → annotation target; list panel navigation back to file-view locations. |
| `README.md` | "Annotation output format" section gains the `(=)` marker (same commit as serialization change). |

### Notes

- Repo test conventions: TDD for pure code; big test modules split as `#[cfg(test)] #[path = "foo_tests.rs"]`; integration tests in tempdirs with canonicalized paths; run everything with `cargo test`.
- Gates before every commit: `cargo build && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`. Conventional commits; refactors never share a commit with behavior.
- Every user-visible key goes through the shared tables in `src/ui/keymap.rs` / `src/ui/modal_keys.rs` — never loose match arms — and must appear in `?` (drift tests enforce both directions).
- Background work: bounded channels drained once per tick via the existing `BackgroundTasks` poller; single-flight + generation counters copied from the History loader; never block the render loop.

## Tasks

### [x] 1.0 Fuzzy file finder (`gp`) opening a read-only whole-file view

Ships spec Unit 1: the `gp` overlay (git-ls-files candidates, `nucleo-matcher` ranking) and the shared read-only file view — whole-file, syntax-highlighted, capability-gated, lossless `Esc` unwind. Foundation for tasks 3.0 and 4.0.

#### 1.0 Proof Artifact(s)

- Test: `cargo test search::` passes — file-list parsing (NUL-split, tracked+untracked merge) and ranking glue written TDD-first, demonstrating the pure core meets its contract.
- Test: modal-key drift tests for the finder mode and file-view mode pass, demonstrating every key is documented in `?` and every documented key acts (keymap guardrail).
- Test: footer/help assertions that the file view omits staging/commit/LSP keys (README read-only-target convention), demonstrating capability gating.
- CLI: recorded journey in `proofs/task-1-file-finder.md` — `gp` → type partial name → open a file with no diff → scroll → `Esc` back to prior view with position intact — demonstrating Unit 1 FRs end to end on a real repo.

#### 1.0 Tasks

- [x] 1.1 Add `nucleo-matcher` to `Cargo.toml` (`default-features = false` if features permit); commit message carries the dependency justification (benchmarked <1ms at 2.3k paths, fzf-consistent ranking, 3 transitive deps, MPL-2.0 noted).
- [x] 1.2 TDD in `src/git/`: `ls_files` (tracked) and `ls_files_untracked` (`--others --exclude-standard`) — fixed-argv runners + NUL-split parser tested on fixture bytes first, then a tempdir-repo integration test (canonicalized paths).
- [x] 1.3 TDD `src/search/files.rs`: `FileCandidate` model; merge tracked + untracked lists (dedupe, stable order).
- [x] 1.4 TDD `src/search/fuzzy.rs`: `rank(candidates, query) -> ranked matches with positions` using `nucleo-matcher` path config; smartcase behavior consistent with `src/ui/search.rs` conventions; deterministic tie-breaking.
- [x] 1.5 Read-only file target: extend `DiffTarget`/capability model in `src/git/diff.rs` with a worktree-file variant — staging, commit, and code-intel capabilities all report unavailable; TDD the capability predicates alongside the existing target tests.
- [x] 1.6 `src/ui/file_view.rs`: open a worktree file as a synthesized all-context body (via the `read_worktree_file` seam), syntax highlighted, existing scroll/jump motions working, open-at-line support; suspend/restore the prior view via the existing suspended-view mechanism (app-state test: open → `Esc` restores the prior mode, cursor, and scroll); footer/help omit gated keys (assertion test).
- [x] 1.7 Finder mode: `Mode` variant + state in `src/ui/app.rs`/`src/ui/file_finder.rs` (input buffer, cursor, ranked list); candidates loaded through `BackgroundTasks` on open (single-flight); re-rank per keystroke; render in `src/ui/file_finder_modal.rs` (switcher modal pattern + input line + match-position highlighting); `Enter` opens the file view; `Esc` closes losslessly.
- [x] 1.8 Keymap: `gp` sequence + actions in `src/ui/keymap.rs`; finder and file-view key tables in `src/ui/modal_keys.rs`; extend the bidirectional drift tests; `?` overlay and footers show the new modes.
- [x] 1.9 Gates green; record the acceptance journey (open finder on this repo, open an un-diffed file, unwind) with observations in `proofs/task-1-file-finder.md`.

### [x] 2.0 In-process search engine core (`src/search/engine.rs`)

The embedded ripgrep engine behind Project Search — pure module, no TUI types, fully contract-tested before any UI consumes it.

#### 2.0 Proof Artifact(s)

- Test: `cargo test search::engine` passes on tempdir corpora — regex/smartcase/word/literal matrix, gitignore respected, untracked-unignored included, binary + oversized files skipped with counters, cap behavior, prompt mid-scan abort — demonstrating every Unit 2 engine FR has a mapped test.
- Test: perf tripwire in `src/ui/perf_tests.rs` style passes (generated multi-thousand-file corpus, loop-amortized, 10–20× debug budget), demonstrating the instant-feel complexity class is enforced.
- CLI: `cargo tree -e normal | wc -l` before/after + release binary size delta recorded in `proofs/task-2-engine.md`, demonstrating dependency cost matches what was ratified in questions round 1 (~+2MB, ~21 transitive deps).

#### 2.0 Tasks

- [x] 2.1 Record baseline (`cargo tree -e normal | wc -l`, release binary size), then add `grep-searcher`, `grep-regex`, `ignore` (`default-features = false` where features permit); justification in the commit message; record the after-numbers in `proofs/task-2-engine.md`.
- [x] 2.2 TDD query model: `SearchQuery { pattern, case: CaseMode, whole_word: bool, literal: bool }` → `grep-regex` matcher construction (smartcase = case-insensitive unless pattern has an uppercase letter; literal via fixed-string escaping/builder option); typed error for invalid regex (no panics).
- [x] 2.3 TDD engine scan: parallel `ignore` walk rooted at the repo worktree (respects `.gitignore`, includes untracked-unignored, skips `.git/`), `grep-searcher` per file with binary detection and a large-file skip threshold; sink emits `SearchHit { path, line_number, line_text, match_spans }` into a bounded channel in small batches; skip counters reported in a scan summary.
- [x] 2.4 TDD cancellation + limits: `AtomicBool` abort checked in the sink (assert prompt stop mid-scan); results tagged with a caller-supplied generation; global cap (default 10,000 hits) with an explicit `capped` flag in the summary.
- [x] 2.5 Perf tripwire: generate a several-thousand-file corpus in a tempdir, measure query→first-batch and full-scan in debug, budget 10–20× measured, loop-amortize; add alongside the existing tripwires in `src/ui/perf_tests.rs` (or `src/search/` if a purer seam fits — keep the established style).
- [x] 2.6 Gates green; finish `proofs/task-2-engine.md` (dependency cost + test run summary).

### [x] 3.0 Project Search view (`g/`): live query, toggles, result navigation

Ships spec Unit 2's UI on top of task 2.0's engine.

#### 3.0 Proof Artifact(s)

- Test: mode dispatch + drift tests for the search mode pass (query editing, toggles, navigation, open-result, unwind), demonstrating keymap/help integration.
- Test: generation/debounce unit tests pass — stale results dropped, in-flight scan aborted on query change, invalid regex shows inline error without wiping prior results — demonstrating the concurrency contract.
- CLI: recorded journey in `proofs/task-3-project-search.md` — review a diff, `g/` an identifier from it, results stream, refine + toggle whole-word, open a hit in an untouched file, `Esc` `Esc` back to the exact diff position — with timing notes against the <100ms first-results bar.

#### 3.0 Tasks

- [x] 3.1 Search mode: `Mode` variant + state in `src/ui/app.rs`/`src/ui/project_search.rs` — query buffer, toggle states (case/word/literal), results grouped by file, selection + scroll, generation counter, debounce deadline.
- [x] 3.2 TDD background wiring: debounce (~120–150ms after last keystroke, min query length 2), spawn engine scan on a worker with the abort flag + generation; drain result batches once per tick next to the existing polls in `src/ui/mod.rs`; drop stale generations; abort the in-flight scan on query change or mode exit; single-flight guard.
- [x] 3.3 Full-screen render in `src/ui/project_search_view.rs`: input line with toggle indicators (e.g. `[re] [Cc] [w]`), file-grouped results with match-span emphasis (reuse existing highlight/theme utilities), summary line ("N matches in M files", cap + skip indicators), inline regex-error line that never wipes the previous good results.
- [x] 3.4 Result navigation: list motions through grouped results; `Enter` opens the task-1.0 file view at the hit line (cursor on it); `Esc` from the file view returns to the search view with query/toggles/results/selection intact; `Esc` from the search view restores the exact prior diff position.
- [x] 3.5 Keymap: `g/` sequence + action in `src/ui/keymap.rs`; search-mode table in `src/ui/modal_keys.rs` including `Alt-c` (case), `Alt-w` (word), `Alt-r` (regex↔literal); drift tests both directions; footer + `?` updated.
- [x] 3.6 Gates green; record the primary journey with timing notes in `proofs/task-3-project-search.md`.

### [x] 4.0 Annotations on non-diff lines: `(=)` marker + output contract

Ships spec Unit 3: annotation keys in the file view; public stdout format extended byte-exactly.

#### 4.0 Proof Artifact(s)

- Test: byte-exact serialization tests for `(=)` line/range headers pass AND the pre-existing serialization suite passes with zero assertion edits, demonstrating the public API is extended without breaking existing consumers.
- CLI: captured stdout in `proofs/task-4-output.md` from a session mixing diff annotations and a `(=)` annotation, demonstrating the full contract in one artifact.
- Round-trip: consumer-half exercise (agent reads the captured output and correctly resolves the `(=)` annotation to worktree content), recorded in `proofs/task-4-output.md` — same pattern as spec 05's proof.
- Diff: README "Annotation output format" section updated in the same commit as the serialization change, demonstrating docs-as-contract.

#### 4.0 Tasks

- [x] 4.1 TDD model: represent the "current file content" side in `src/annotate/model.rs` (e.g. a third side/target form) without disturbing existing variants; decide and document how it composes with the `Reviewing:` grouping (file-view annotations group with the working-tree group).
- [x] 4.2 TDD serialization: `(=)` marker for line and range headers in `src/annotate/markdown.rs`, byte-exact tests; run the full existing suite and assert zero assertion edits; update the module-doc format contract.
- [x] 4.3 File view annotation flow: annotation keys active in the file-view key table (drift tests extended); TDD the `src/ui/targeting.rs` mapping from file-view cursor/selection → the new target form; annotation list panel shows these entries and navigates back to the file-view location (app-state test for the navigate-back path).
- [x] 4.4 Update README "Annotation output format" (the `(=)` marker, its meaning, one example) in the same commit as 4.2.
- [x] 4.5 Gates green; produce the stdout capture + consumer round-trip evidence in `proofs/task-4-output.md`.

### [ ] 5.0 User-acceptance evidence and gate sweep

Repo-standard closing task (spec 05 precedent): UX outcomes verified by the user, evidence persisted, merge candidate fully green.

#### 5.0 Proof Artifact(s)

- Proof: `proofs/task-5-acceptance.md` — user-executed journeys for Success Metrics 1 and 2 (instant-feel timing note; grep→open→annotate→unwind journey) with observations, demonstrating UX outcomes rather than only technical artifacts.
- CLI: final four-gate sweep output captured in `proofs/task-5-acceptance.md` (all commands, exit 0), demonstrating merge readiness.
- Test: full `cargo test` summary comparison showing pre-existing test counts intact (no deleted/loosened tests), demonstrating no regression to existing contracts.

#### 5.0 Tasks

- [ ] 5.1 Assemble the merge candidate on the worktree branch: sync with current `main`, resolve, full gate sweep; capture the sweep output.
- [ ] 5.2 User-in-the-loop acceptance: the user runs the Success Metric 1 and 2 journeys on a real review in this repo; record observations, timings, and any friction verbatim in `proofs/task-5-acceptance.md` (follow-ups filed as notes, not silently fixed).
- [ ] 5.3 Regression check: compare `cargo test` totals against pre-branch baseline; assert no pre-existing test was deleted or loosened (move-only refactors keep identical counts).
- [ ] 5.4 Confirm every spec proof artifact exists under `proofs/`; hand off to Phase 4 validation.
