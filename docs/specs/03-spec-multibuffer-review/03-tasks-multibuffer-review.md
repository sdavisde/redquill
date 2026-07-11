# 03-tasks-multibuffer-review.md

Task list for `03-spec-multibuffer-review.md`.

Ordering rationale: the side-by-side removal comes first because `rows` and `sbs_rows` are rebuilt in lockstep today (`app.rs` `rebuild_rows`), and deleting `SbsRow`/`sbs_view.rs`/parity tests up front halves the surface every later task must generalize. The multi-file row model (2.0) is the structural core the staging flow (3.0) and surface integration (4.0) build on; performance and docs finalization (5.0) close out the spec's hard regression bar and README keymap contract.

## Relevant Files

| File | Why It Is Relevant |
| --- | --- |
| `src/ui/rows.rs` | Row model; gains the multi-file builder, section-header rows, and per-row file identity. Sheds `SbsRow`/`build_sbs_rows`. Inline unit tests (TDD). |
| `src/ui/diff_view_state.rs` | View state; gains collapse map, whole-buffer rows, cursor→file derivation, cross-file motions, header jumps. Sheds `ViewMode`/sbs state. Inline unit tests. |
| `src/ui/app.rs` | Wiring: `apply()` action arms, `rebuild_rows`, `refresh` collapse/auto-expand rules, jump logic, `FakeGit` app-level tests. |
| `src/ui/diff_view.rs` | Renders section headers (collapse indicator, kind letter, path, `●`/`±` marker) and collapsed single-line headers. |
| `src/ui/sbs_view.rs` | Deleted (side-by-side retired). |
| `src/ui/keymap.rs` | Add `Action::StageFile` (`S`), `Action::ToggleCollapse` (`za`, new `z` prefix), repurpose `Tab`/`Shift-Tab`, retire `t`/`Action::ToggleView`. Inline tests. |
| `src/ui/help.rs` | Help overlay reflects new/removed bindings; target-conditional hiding of staging keys on ranges. |
| `src/ui/sidebar.rs` | `±` partial-staged marker; highlight follows cursor-derived file. |
| `src/ui/staging.rs` | `toggle_stage` gesture resolution keyed off the cursor row's owning file instead of `selected_file`. Inline tests. |
| `src/ui/stage_ops.rs` | Per-file `StagedState` (None/Partial/Full) derivation from `git status` codes. Inline tests (TDD). |
| `src/ui/code_intel.rs` | `code_intel_position` derives path from the owning file; peek jump-to-location scrolls to and expands the target section. Inline tests. |
| `src/ui/search.rs` | Matches computed over the whole multibuffer row Vec (collapsed sections contribute no rows). Inline tests. |
| `src/ui/syntax.rs` | Lazy per-file highlight population for visible/expanded sections; per-file (not wholesale) invalidation on refresh. Inline tests. |
| `src/ui/modes.rs` | Modal key handling touch-ups where actions moved. |
| `src/ui/mod.rs` | Event loop/render call sites; `TestBackend` render tests updated. |
| `src/ui/list_panel.rs` | Annotation list jump targets a row in the unified buffer. |
| `README.md` | Keymap table updated: add `S`, `za`; repurpose `Tab`; remove `t`. |
| `docs/specs/03-spec-multibuffer-review/03-proofs/` | Committed smoke transcripts and performance-run notes (proof artifacts). |

### Notes

- Tests live inline in `#[cfg(test)] mod tests` next to the code (repo convention); app-level tests use the `FakeGit`/`app_with_fake` harness in `app.rs`; render tests use `ratatui::backend::TestBackend`.
- TDD for the pure row-derivation and staged-state code: failing test first, tests commit with the code.
- Run all four gates before every commit: `cargo build`, `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check`.
- No `unwrap()`/`expect()` outside tests. Conventional commits (`refactor:` for 1.0, `feat:` for 2.0–4.0, `perf:`/`docs:` as fits for 5.0).
- Assumptions ratified from spec Open Questions: keys are `S` and `za` (`zM`/`zR` optional); collapsed-header `+N −M` summary is optional; initial collapse state = fully-staged files start collapsed, everything else expanded.

## Tasks

### [x] 1.0 Retire the side-by-side view

#### 1.0 Proof Artifact(s)

- CLI: `cargo build && cargo test && cargo clippy -- -D warnings && cargo fmt --check` all green after the removal demonstrates no dangling references.
- CLI: `grep -riE "sbs|side.?by.?side|ToggleView" src/ README.md` returns no hits demonstrates the code path is fully deleted, not orphaned.
- Diff: README.md keymap table no longer lists `t`, and a help-overlay test asserts `t`/toggle-view is absent, demonstrates the binding is retired from the public map.

#### 1.0 Tasks

- [x] 1.1 Delete `src/ui/sbs_view.rs` and its module declaration/render call sites in `src/ui/mod.rs` and `src/ui/diff_view.rs`; delete `SbsRow`, `build_sbs_rows`, `SbsRow::source_rows`, and their tests from `src/ui/rows.rs`.
- [x] 1.2 Remove `ViewMode`, `layout`, `toggle_view`, `sbs_rows`, `sbs_visual_of`, `sbs_scroll`, and the sbs branch of `ensure_visible` from `src/ui/diff_view_state.rs`; remove the lockstep sbs rebuild from `App::rebuild_rows` and the sbs-parity tests in `src/ui/app.rs`/`src/ui/mod.rs`.
- [x] 1.3 Remove `Action::ToggleView` and the `t` binding from `src/ui/keymap.rs`, its help grouping in `src/ui/help.rs`, and add a test asserting `t` resolves to no action.
- [x] 1.4 Update README.md (drop `t` row; note unified-only view), run the four gates, and commit `refactor: remove side-by-side view (multibuffer is unified-only)`.

### [x] 2.0 Multi-file row model with collapsible sections

#### 2.0 Proof Artifact(s)

- Test: unit tests on the multi-file builder in `src/ui/rows.rs` (concatenation order, collapse filtering, section-header content/markers, addressability, per-row file identity, synthetic untracked sections) pass via `cargo test` demonstrates the model is correct (FRs: Unit 1, all five).
- Test: `src/ui/diff_view_state.rs` tests covering cross-file cursor motion, `]`/`[` crossing expanded sections, `Tab`/`Shift-Tab` header jumps, `za` toggling, and cursor clamping pass demonstrates navigation correctness.
- Manual smoke transcript committed at `docs/specs/03-spec-multibuffer-review/03-proofs/03-task-02-proofs.md`: scrolling a multi-file working-tree diff end to end, collapsing/expanding mid-scroll, demonstrates one continuous document.

#### 2.0 Tasks

- [x] 2.1 (TDD) In `src/ui/rows.rs`, extend the header row to a section header carrying `{file_index, path, old_path, kind, staged_marker, collapsed, annotated}` and write failing tests for a new `build_multibuffer(files, collapse_state, staged_states, annotations, per-file syntax) -> MultibufferRows` where `MultibufferRows` holds `rows: Vec<Row>`, `file_of_row: Vec<usize>`, and `header_row_of_file: Vec<usize>`; collapsed files contribute exactly their header row; untracked files enter via the existing `FileDiff::synthetic_added` path; then implement to green.
- [x] 2.2 In `src/ui/diff_view_state.rs`, store `MultibufferRows` plus a path-keyed collapse map; add `file_of_cursor()`, make `nearest_addressable`/clamping/motions operate over the whole buffer, generalize `]`/`[` to cross into neighboring expanded files (deleting the `probe_*` throwaway-row dance), and add next/prev-section-header motions; keep `selected_file` only as a derived value for the sidebar highlight.
- [x] 2.3 In `src/ui/keymap.rs` + `src/ui/app.rs`, add `Action::ToggleCollapse` bound to `za` (extending the two-key prefix machinery to `z`), rebind `Tab`/`Shift-Tab` to the header-jump actions, and wire `App::rebuild_rows` to build the multibuffer with lazy per-file highlight population (only expanded files whose rows can be visible); initial collapse state on launch: fully-staged files collapsed, all else expanded.
- [x] 2.4 In `src/ui/diff_view.rs` (+ `src/ui/sidebar.rs`), render section headers visually distinct (current file-header-bar style) with `▾`/`▸`, kind letter, path/rename arrow, and marker slot; collapsed headers render exactly one line; sidebar highlight follows `file_of_cursor()`; add `TestBackend` render tests.
- [x] 2.5 Update `src/ui/help.rs` and README.md for `za` and the repurposed `Tab`, run the four gates, record `03-proofs/03-task-02-proofs.md`, and commit `feat: multi-file multibuffer with collapsible sections`.

### [x] 3.0 Staging-driven review flow

#### 3.0 Proof Artifact(s)

- Test: app-level tests with the existing `FakeGit` harness in `src/ui/app.rs` covering stage→collapse, unstage-from-header→expand, `±` marker transitions, and the refresh auto-expand rule pass via `cargo test` demonstrates the flow's state machine (FRs: Unit 2, all five).
- Test: `FakeGit` call-recording assertions show only existing `StageOps` methods are invoked demonstrates no new git-layer code.
- Manual smoke transcript committed at `docs/specs/03-spec-multibuffer-review/03-proofs/03-task-03-proofs.md`: review three files, stage two (collapse), edit one staged file externally, refresh, watch it re-expand with `±`, demonstrates the "nothing hides" guarantee.

#### 3.0 Tasks

- [x] 3.1 (TDD) In `src/ui/stage_ops.rs`, derive a per-file `StagedState { Unstaged, Partial, Full }` from `FileStatus.staged`/`.unstaged` codes (failing tests first covering all code combinations, including untracked and renames), and thread it into the data `rebuild_rows`/sidebar consume.
- [x] 3.2 In `src/ui/keymap.rs` + `src/ui/app.rs`, add `Action::StageFile` bound to `S`: on an unstaged/partial file, `stage_file` via `StageOps` then auto-collapse its section; on a fully staged file (cursor on its header or body), `unstage_file` then auto-expand; status-line feedback on failure; app-level `FakeGit` tests for both directions.
- [x] 3.3 In `App::refresh`, preserve the collapse map by path across refreshes, auto-expand any collapsed file whose new status has unstaged changes, keep fully-staged collapsed files collapsed, and drop map entries for files that left the diff; app-level tests for the auto-expand rule and collapse-state survival.
- [x] 3.4 Keep `space`/visual-mode hunk-line staging working with the file derived from the cursor row (`src/ui/staging.rs`), update header + sidebar markers to `±`/`●` per `StagedState`, extend `src/ui/help.rs` + README.md with `S`, run the four gates, record `03-proofs/03-task-03-proofs.md`, and commit `feat: stage-and-collapse review flow`.

### [ ] 4.0 Full-surface integration across all diff targets

#### 4.0 Proof Artifact(s)

- Test: annotation-anchoring, search, and jump tests generalized to multi-file rows pass via `cargo test`, and the existing markdown-output assertions in `src/annotate/markdown.rs` pass unchanged, demonstrates no review-surface regression and an intact stdout API (FRs: Unit 3, requirements on annotations, search, LSP, panel routing).
- Test: keymap/help tests asserting staging actions are inert and absent from the `?` overlay under a `Range` target demonstrate target-conditional gating.
- Manual smoke transcript committed at `docs/specs/03-spec-multibuffer-review/03-proofs/03-task-04-proofs.md`: a ref-range review rendered as one scrollable buffer with an annotation composed and `gd`/`K` peek used across two files, demonstrates all-targets parity.

#### 4.0 Tasks

- [ ] 4.1 (TDD) Generalize `anchor_row_index` in `src/ui/rows.rs` to the multibuffer: `Target::File` maps to that file's section-header row (not row 0), line/range/hunk targets resolve within the owning file's row span; annotation gutter rows splice correctly in every section; `jump_to_annotation` scrolls the buffer and expands a collapsed target section; markdown-on-quit assertions unchanged.
- [ ] 4.2 Make search span the buffer: `SearchState::recompute` already runs over the full row Vec — add tests that matches cross file boundaries, skip collapsed sections (their rows are absent), and that `n`/`N` wrap over the whole buffer; recompute on collapse toggle.
- [ ] 4.3 In `src/ui/code_intel.rs`, derive `code_intel_position` path from the cursor row's owning file; make `peek_enter` jump-to-location scroll the multibuffer to the target file's section, expanding it if collapsed; extend inline tests.
- [ ] 4.4 Add the narrow select-by-path seam from spec 02: `App::select_file_by_path(&Path)` expands (if collapsed) and scrolls to that file's section header, and route sidebar/git-panel selection through it; unit tests including the unknown-path case.
- [ ] 4.5 Render the multibuffer for `--staged` and ref-range targets; for ranges, make staging actions no-ops absent from contextual help (`src/ui/help.rs` gains target-aware filtering); tests per target; run the four gates, record `03-proofs/03-task-04-proofs.md`, and commit `feat: multibuffer across all diff targets with full review surface`.

### [ ] 5.0 Performance hardening and keymap/docs finalization

#### 5.0 Proof Artifact(s)

- Test: unit tests asserting highlight-cache population happens only for visible/expanded files and survives refresh for unchanged files pass via `cargo test` demonstrates lazy highlighting.
- Manual performance transcript committed at `docs/specs/03-spec-multibuffer-review/03-proofs/03-task-05-proofs.md`: a ~5k-line multi-file diff (fixture generation commands recorded) scrolled with held `j` and `Ctrl-d`, plus stage/collapse latency notes, demonstrates the instant-feel bar.
- Diff: README.md keymap table and help-overlay tests reflect the final binding set (`S`, `za`, repurposed `Tab`, retired `t`, `zM`/`zR` if implemented) demonstrates the public keymap contract matches the implementation.
- CLI: `cargo build && cargo test && cargo clippy -- -D warnings && cargo fmt --check` green at spec completion demonstrates all repository quality gates pass.

#### 5.0 Tasks

- [ ] 5.1 Replace the wholesale highlight-cache clear in `App::refresh` with per-file invalidation (only files whose diff content changed), and populate the cache lazily on first visibility/expansion; unit tests in `src/ui/syntax.rs`/`app.rs`.
- [ ] 5.2 Make stage/collapse row rebuilds incremental or verify a full rebuild of a 5k-line buffer is imperceptible (measure; document the numbers in the perf transcript); remove any remaining redundant per-gesture work found while measuring.
- [ ] 5.3 Build a throwaway ~5k-line multi-file diff fixture repo (commands recorded), run the held-`j`/`Ctrl-d` scroll and stage/collapse checks, and commit `03-proofs/03-task-05-proofs.md`.
- [ ] 5.4 Final README.md keymap table + `?` overlay sweep (every new action listed, `zM`/`zR` included if implemented), run the four gates, and commit `docs: finalize multibuffer keymap and proofs`.
