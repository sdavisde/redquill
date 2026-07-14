# 05-tasks-diff-sources.md

Task list for `05-spec-diff-sources.md`. Parent tasks 1.0–5.0 map 1:1 to the spec's Demoable Units and should be implemented in order (1.0 is the foundation the others consume). Task 6.0 is the user-acceptance gate: it grades the spec's Success Metrics — the user-facing definition of done — after everything else lands, and validation reviews it first.

## Relevant Files

| File | Why It Is Relevant |
| --- | --- |
| `src/git/diff.rs` | `DiffTarget` enum: add `Commit(String)` variant and the capability methods (`is_live`, `staging_mode`, `supports_code_intel`) with unit tests. |
| `src/git/runner.rs` | Builds git argv: commit-diff invocation (`<rev>^ <rev>` / empty-tree fallback) and the NUL-delimited `git log` invocation. |
| `src/git/log.rs` (new) | Commit-log read model: `CommitLogEntry`, NUL-delimited parser, pagination (count/skip). |
| `src/git/mod.rs` | Export the new `log` module. |
| `src/ui/syntax.rs` | `content_source` gains `Commit` arms (`<rev>^:<path>` / `<rev>:<path>`), keeping historical content off the disk. |
| `src/ui/refresh.rs` | Auto-refresh gate switches from `matches!(target, Range)` to `target.is_live()`. |
| `src/ui/stage_ops.rs` | Untracked-file injection gated by `is_live()`; `build_review` handles the `Commit` target. |
| `src/ui/staging.rs` | Staging direction/read-only guards route through `staging_mode()`. |
| `src/ui/app.rs` | `stage_file` guard via `staging_mode()`; view-suspend/restore state for open-commit/return navigation. |
| `src/ui/mod.rs` | `staging_allowed` computation replaced by capability queries. |
| `src/ui/footer.rs` | Footer hint visibility driven by `staging_mode()` / `supports_code_intel()`. |
| `src/ui/code_intel.rs` | Gate requests and peek previews on `supports_code_intel()`; module doc documents the degradation contract. |
| `src/ui/git_panel.rs` | History tab: tab state, row rendering, cursor, unpushed markers, Enter-to-open. |
| `src/ui/modal_keys.rs` | New semantic keys (panel tab toggle, open commit, return from commit view) in the shared tables. |
| `src/ui/keymap.rs` | Default bindings for the new keys (suggested: `Tab` panel-scoped, `Enter`, `Esc`). |
| `src/ui/background.rs` | Background commit-log fetch (single-flight + generation counter, non-blocking poll). |
| `src/ui/diff_view.rs` | Commit-view header block (short SHA, author, absolute date, subject) above the multibuffer. |
| `src/annotate/model.rs` | Annotations record the diff target they were created against. |
| `src/annotate/markdown.rs` | Grouped emission with the `Reviewing:` metadata line; byte-exact tests. |
| `src/ui/welcome.rs` (new, or within `diff_view.rs`) | Empty-diff welcome state: situation text + keyed action hints sourced from the shared keymap tables. |
| `src/ui/perf_tests.rs` | Existing wall-clock tripwires must pass unmodified (regression contract). |
| `docs/specs/05-spec-diff-sources/proofs/` | User-acceptance evidence: journey transcripts, round-trip output, dogfood review annotations. |
| `README.md` | Annotation-format documentation gains the `Reviewing:` line contract. |

### Notes

- Test commands: `cargo test` (unit + integration), plus the full gate set before every commit: `cargo build`, `cargo test`, `cargo clippy -- -D warnings` (`--all-targets`), `cargo fmt --check`.
- TDD applies to the pure code in this feature: capability methods, log parsing, commit-diff target semantics, annotation serialization — failing test first.
- Integration tests build throwaway repos in tempdirs (canonicalize paths for the macOS `/var` symlink); never touch the host repo.
- Refactor commits (1.0) and behavior commits (2.0–4.0) stay separate; conventional commit prefixes throughout.
- New keybindings go in the shared tables (`modal_keys.rs` / `keymap.rs`) only — never loose match arms — so the existing help-overlay drift tests cover them.

## Tasks

### [~] 1.0 Capability model on `DiffTarget` (behavior-preserving refactor + code-intel gate)

#### 1.0 Proof Artifact(s)

- Test: unit tests in `src/git/diff.rs` asserting the full capability triple for every `DiffTarget` variant pass, demonstrates the model is exhaustive and encodes current behavior (FR: capability methods).
- CLI: `grep -rn "matches!(.*DiffTarget" src/ui/` returns no capability-decision hits, demonstrates all call sites route through the named methods (FR: no scattered checks).
- CLI: `redquill main..HEAD` in a test repo with an LSP server configured shows no code-intel keys in the `?` overlay or footer, and the keys are inert, demonstrates the gate works end-to-end (FR: code-intel gating).
- Test: full existing suite passes unchanged (`cargo test`), demonstrates the refactor is behavior-preserving for working-tree/staged flows.
- Diff: `src/ui/code_intel.rs` module doc stating the degradation contract, demonstrates the documented-contract requirement is met (FR: module doc).

#### 1.0 Tasks

- [x] 1.1 TDD: write failing unit tests in `src/git/diff.rs` asserting, per variant — `WorkingTree`: `is_live()==true`, `staging_mode()==Stage`, `supports_code_intel()==true`; `Staged`: `false`/`Unstage`/`false`; `Range`: `false`/`ReadOnly`/`false`. Then add the `StagingMode` enum and the three methods as exhaustive matches (no wildcard arms, per the data-driven-invariants rule).
- [ ] 1.2 Route the five existing capability decisions through the methods: `refresh.rs` auto-refresh gate and `stage_ops.rs` untracked-file injection → `is_live()`; `staging.rs` (read-only guard + direction) and `app.rs` `stage_file` guard → `staging_mode()`; `mod.rs` / `footer.rs` `staging_allowed` → `staging_mode() != ReadOnly`. Verify with grep that no capability `matches!` remains in `src/ui/`. Commit as `refactor:` (identical test counts, zero assertion edits — state this in the commit).
- [ ] 1.3 Gate code-intel: return early from LSP request dispatch and peek-preview reads in `code_intel.rs` when `!target.supports_code_intel()`, and drive the code-intel key visibility in help/footer from the same predicate via the shared key tables (mirror how staging keys hide today).
- [ ] 1.4 Write the degradation contract in the `code_intel.rs` module doc (code-intel silently absent when the new side isn't the live working tree, and why that's deliberate). Manually verify the `main..HEAD` proof, run all four gates, commit as `fix:` (this changes observable behavior on range/staged views).

### [ ] 2.0 Single-commit diff target and commit-log read model (git layer)

#### 2.0 Proof Artifact(s)

- Test: parser unit tests for the NUL-delimited log format in `src/git/log.rs` pass, including subjects containing `:`/whitespace/quote characters, demonstrates robust machine-readable parsing (FR: log read model).
- Test: tempdir integration tests pass asserting `DiffTarget::Commit` yields the commit's own changes for a normal commit, a merge commit (first parent), and a root commit (all-added), demonstrates correct diff semantics (FR: commit target, merge/root edge cases).
- Test: `content_source` tests asserting `Commit` sides resolve to `<rev>^:<path>` / `<rev>:<path>` git-object specs pass, demonstrates historical content never reads the working tree (FR: content from git objects).
- Test: capability tests for the `Commit` variant (`is_live()==false`, `staging_mode()==ReadOnly`, `supports_code_intel()==false`) pass, demonstrates the new source inherits correct gating (FR: commit capabilities).

#### 2.0 Tasks

- [ ] 2.1 TDD: extend `DiffTarget` with `Commit(String)`; the compiler forces capability decisions (add the triple + tests from 1.1's table). Implement acquisition in `runner.rs`: `git diff --no-color --no-ext-diff -M <rev>^ <rev>` with rev passed as discrete argv elements; when `git rev-parse --verify <rev>^` fails (root commit), diff against git's empty tree instead.
- [ ] 2.2 Add `Commit` arms to `content_source` in `syntax.rs`: old side `<rev>^:<path>` (empty-content fallback for root commits), new side `<rev>:<path>`; unit tests per the existing `content_source` test pattern.
- [ ] 2.3 TDD: create `src/git/log.rs` with `CommitLogEntry { sha, short_sha, subject, author_name, timestamp }` and a parser for a NUL-delimited `git log --format` record layout; parser tests first, including hostile subjects and empty repos. No TUI types in signatures.
- [ ] 2.4 Add the log invocation to `runner.rs` with count/skip pagination parameters (closed types → fixed argv; `GIT_TERMINAL_PROMPT=0` as elsewhere), and export the module from `git/mod.rs`.
- [ ] 2.5 Tempdir integration tests for the three commit-shape cases (normal, merge, root) and for log pagination (two pages, stable ordering). Run gates; commit as `feat(git):`.

### [ ] 3.0 Git panel History tab and commit view (UI)

#### 3.0 Proof Artifact(s)

- Screenshot: git panel History tab showing two-line commit rows (subject + unpushed marker; dimmed author · relative time · short SHA) with a highlighted row, demonstrates the Zed-style list renders (FR: History tab, row anatomy).
- Screenshot: an opened commit view showing the header block (short SHA, author, absolute date, subject) above collapsible file sections, with the `?` overlay listing History/return keys but no staging or code-intel keys, demonstrates the end-to-end flow and capability gating (FR: commit view, gating).
- Test: UI-state tests pass asserting open-commit → return round-trips restore the previous target, cursor, and collapse state, demonstrates navigation correctness (FR: return navigation).
- Test: existing `src/ui/perf_tests.rs` tripwires pass with unmodified budgets, demonstrates no render-loop complexity regression (FR: performance).
- Test: keymap/help drift tests pass with the new keys present, demonstrates no hidden features (FR: keymap registration).

#### 3.0 Tasks

- [ ] 3.1 Add panel tab state (`Changes` / `History`) to the git panel; register the panel-scoped toggle key in `modal_keys.rs` + `keymap.rs` (suggested default `Tab`; verify no conflict in the shared tables first) so the `?` overlay updates via the existing drift-tested path.
- [ ] 3.2 Wire background history loading in `background.rs` following the existing fetch/pull/push pattern: spawn `git log` page fetch off-thread, single-flight flag, generation counter to drop stale results, drain via the per-tick poll. History tab shows a loading placeholder until page 1 arrives; scrolling near the end requests the next page. Add UI-state tests asserting (a) the placeholder state before the first page lands and (b) a stale-generation history result is dropped, not applied.
- [ ] 3.3 Render History rows in `git_panel.rs`: two lines per commit — subject truncated to panel width plus an unpushed marker for the first `ahead` commits (reuse the existing ahead/behind read model); dimmed second line `author · relative-time · short-sha`. Extract relative-time formatting as a pure function with unit tests.
- [ ] 3.4 Implement open-commit: Enter on the highlighted row suspends the current view state (target, files, cursor, collapse, staged markers) and rebuilds the multibuffer via `build_review` for `DiffTarget::Commit(sha)`; render the commit header block above the diff in `diff_view.rs`.
- [ ] 3.5 Implement return: `Esc` (registered in the shared tables) restores the suspended view state; `q` from a commit view quits and emits as usual. Write UI-state round-trip tests (open → navigate → return → assert prior state intact).
- [ ] 3.6 Assert capability gating in commit view via tests: staging keys inert and absent from help/footer, no code-intel, no auto-refresh tick for the commit target, annotations (line/range/hunk/file) fully functional.
- [ ] 3.7 Run perf tripwires and all four gates; capture the two proof screenshots; commit as `feat(ui):`.

### [ ] 4.0 Source-aware annotation output

#### 4.0 Proof Artifact(s)

- Test: byte-exact serialization test for a working-tree-only session passes against the existing fixtures unchanged, demonstrates backward compatibility of the public stdout format (FR: byte-identical default).
- Test: byte-exact serialization test for a mixed session (working-tree + commit annotations) passes, showing the working-tree group first and the commit group preceded by exactly one `Reviewing:` line, demonstrates the additive extension (FR: grouped output, metadata line).
- CLI: scripted session annotating a commit shows `Reviewing: <short-sha>` in stdout, demonstrates consumers can resolve the revision (FR: self-describing output).
- Diff: README + `annotate` module-doc changes documenting the metadata-line syntax, demonstrates the public-API contract is written down (FR: documented format).

#### 4.0 Tasks

- [ ] 4.1 Extend the annotation model (`annotate/model.rs`) so each annotation records the diff target it was created against; working-tree remains the default. Update persistence/construction call sites.
- [ ] 4.2 TDD: byte-exact tests first — (a) working-tree-only fixture identical to current output, (b) mixed fixture with grouped output (`Reviewing: <spec>` line preceding each non-worktree group; working-tree group first, no metadata line). Then implement grouped emission in `annotate/markdown.rs`.
- [ ] 4.3 Fix the metadata-line syntax as part of the format contract (commit → `Reviewing: <short-sha>`, range → `Reviewing: <range-as-typed>`, staged → `Reviewing: staged`) and document it in the `annotate` module doc and README format section.
- [ ] 4.4 Run the scripted CLI proof and all four gates; commit as `feat(annotate):`.

### [ ] 5.0 Empty-diff welcome state

#### 5.0 Proof Artifact(s)

- Screenshot: `redquill` launched in a clean tempdir repo shows the welcome state (situation text + keyed hints: open panel, History tab, `?` help) instead of a blank buffer, demonstrates the dead-end screen teaches the escape route (FR: welcome state, action hints).
- Test: UI-state tests pass asserting empty target → welcome rendered, non-empty target → welcome absent, and welcome cleared when auto-refresh delivers content, demonstrates correct lifecycle (FR: appears/disappears).
- Test: drift-style test asserting every hint key in the welcome text is sourced from the shared keymap tables passes, demonstrates remap safety and no hardcoded keys (FR: keymap-sourced hints).

#### 5.0 Tasks

- [ ] 5.1 Add an empty-state detection point at the review-build boundary (zero files for the active target) and render a centered welcome block in the diff area: one line naming the situation per target ("No uncommitted changes", "Nothing staged", "Empty diff for <range>"), then 3–4 action hints.
- [ ] 5.2 Source each hint's key from the shared keymap tables (`modal_keys.rs`/`keymap.rs`) at render time — no key literals in the welcome text — and add a drift-style test that fails if a hinted action's key is missing or renamed.
- [ ] 5.3 UI-state tests for the lifecycle (empty → welcome; content arrives via refresh → welcome gone; History-opened commit with empty diff → target-appropriate wording). Capture the screenshot proof; run gates; commit as `feat(ui):`.

### [ ] 6.0 User acceptance — prove the core problems are fixed

#### 6.0 Proof Artifact(s)

- Transcript + screenshot: the dead-end journey — launch in a clean tempdir repo containing fresh commits, follow only the welcome-state hints and `?`, reach the newest commit's diff in ≤5 keystrokes — demonstrates Success Metric 1 (saved under `docs/specs/05-spec-diff-sources/proofs/`).
- CLI: transcript of the README pipe (`redquill | <agent> -p "address this review feedback"`) for a session that annotated 3 lines across 2 files of a historical commit, showing the agent locating all three sites, demonstrates Success Metric 2 (round-trip works on history).
- Test/screenshot: commit-view `?` overlay cross-checked against actual behavior — everything listed works, staging/code-intel absent and inert — demonstrates Success Metric 3 (no lies).
- Annotations file: `proofs/dogfood-review.md` — the emitted output from reviewing this spec's own implementation commits via the History tab — demonstrates Success Metric 5 (dogfood gate).

#### 6.0 Tasks

- [ ] 6.1 Script the dead-end journey in a tempdir repo (fresh commits, clean tree); record the exact keystroke path and verify each step was discoverable from the welcome hints or `?` alone; save transcript + screenshot to `proofs/`.
- [ ] 6.2 Run the round-trip: annotate 3 lines across 2 files of a historical commit, quit, pipe stdout to an agent; verify the agent resolves all three sites (correct revision and lines); save the transcript to `proofs/`.
- [ ] 6.3 Cross-check the commit-view help overlay against behavior (confirm 3.6's tests cover the ReadOnly state; extend if not); capture the no-lies screenshot.
- [ ] 6.4 Dogfood: review the implementation commits of tasks 1.0–5.0 in redquill via the History tab; save the emitted annotations to `proofs/dogfood-review.md`; fix or file anything that made the review unpleasant before marking this spec done.
