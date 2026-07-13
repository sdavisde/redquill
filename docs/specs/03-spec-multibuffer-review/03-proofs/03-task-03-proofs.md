# Task 03 Proofs — Staging-driven review flow

## Task Summary

Parent task 3.0 turns the multibuffer's "keep it" verdict into a single
keypress: `S` stages the file under the cursor and its section collapses out
of the way; `S` on a fully-staged file unstages it and re-expands. A
refresh can never let a collapsed header hide unreviewed work — a file that
is staged and then edited again comes back partially staged (`±`) and
auto-expands.

- `src/ui/stage_ops.rs` gains the pure `StagedState { Unstaged, Partial,
  Full }` derivation (`staged_state` / `staged_states_from_status`) from a
  file's porcelain index-side (`X`) and working-tree-side (`Y`) codes, plus
  `kind_from_staged_code`. `ReviewSnapshot` now carries a path-keyed
  `staged_states` map, and `build_review` unions fully-staged files into the
  working-tree review as header-only sections (decision A).
- `src/ui/app.rs` stores `staged_states`, adds `App::stage_file` (the `S`
  gesture: stage+collapse / unstage+expand via the existing `StageOps`
  methods), derives the `●`/`±` markers from `staged_states` in
  `rebuild_rows`, and — in `refresh` — drops collapse-map entries for
  departed files and auto-expands any collapsed file that is now *partially*
  staged. Initial launch collapses only fully-staged files.
- `src/ui/keymap.rs` adds `Action::StageFile` bound to `S` (SHIFT-stripped,
  like `K`/`Q`/`N`). `src/ui/help.rs` lists it under "Stage".
- `src/ui/staging.rs` derives the staged file from `file_of_cursor()` and
  rejects a Visual selection that spans more than one file section
  (decision B), scoping the per-file `hunk_index` body count to the
  selected section.
- `src/ui/sidebar.rs` renders `●`/`±`/blank from `staged_states`.
- `README.md` documents the `S` binding.

## What This Task Proves

- The pure `StagedState` derivation is correct across every `XY`
  combination — working-tree-only, staged-only, both-sides, added,
  added-then-edited, deleted, untracked, rename, rename-then-edited, copy,
  and no-change — and `staged_states_from_status` omits `Unstaged` files.
- The `S` state machine: an unstaged/partial file stages and collapses; a
  fully-staged file unstages and re-expands; failures leave state unchanged;
  a read-only range is a no-op with a message; only `stage_file`/
  `unstage_file` are ever called (no new git-layer code).
- The `±`/`●` markers transition live: staging one hunk marks the file
  Partial (`±`) and keeps its section expanded.
- The refresh rules: a partially-staged collapsed file auto-expands
  ("nothing hides"), a still-fully-staged collapsed file stays collapsed, a
  manually-collapsed unstaged file survives refresh, and collapse-map
  entries for departed files are dropped.
- Visual line-staging still works from the cursor's file and rejects
  cross-section spans.
- All four repository gates pass and the test count strictly increased.

## Evidence Summary

| Check | Result |
| --- | --- |
| `cargo build` | pass |
| `cargo test` | pass — 478 tests (453 unit + 25 integration), 0 failed |
| `cargo clippy --all-targets -- -D warnings` | pass |
| `cargo fmt --check` | pass |
| Test count vs 450 baseline | 478 — strictly increased (+28 unit tests) |
| Smoke transcript | FakeGit harness (below); tmux unavailable in env |

Unit-test count moved from 425 to 453 (+28); integration tests unchanged at
25. No pre-existing tests were deleted; a few had `staged_states` threaded in
or their staged-marker assertions moved from `app.staged` to
`app.staged_states` to match the real derivation.

## Artifact: Four cargo gates green

**What it proves:** The stage-and-collapse flow compiles, every test
passes, clippy is warning-free under `-D warnings`, and formatting is
canonical.

**Why it matters:** These four commands are the repo's blocking quality bar
(CLAUDE.md). The change touches the staged-state derivation, app wiring, the
keymap, the staging gesture, and two renderers; a green run demonstrates the
seams are consistent end to end.

**Command:** `cargo build && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`

**Result summary:** All four gates exit 0. Test output trimmed to the
`test result` summary lines (every test `ok`).

```
$ cargo build
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.18s

$ cargo test          # trimmed to "test result" lines
test result: ok. 453 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo clippy --all-targets -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.11s

$ cargo fmt --check
FMT_OK
```

## Artifact: New `StagedState` derivation tests (`src/ui/stage_ops.rs`)

**What it proves:** The pure three-state derivation is correct across every
porcelain `XY` combination the spec names, including untracked and renames.
Written failing-first (TDD) against the new `staged_state` /
`staged_states_from_status` functions, then implemented to green.

**Why it matters:** These states drive the `●`/`±` markers, the launch/S
collapse decisions, and the refresh auto-expand rule — the whole flow keys
off this one derivation.

| Test | One-line description |
| --- | --- |
| `unstaged_when_working_tree_only_modification` | `.M` → `Unstaged`. |
| `full_when_staged_modification_only` | `M.` → `Full`. |
| `partial_when_both_staged_and_unstaged_modification` | `MM` → `Partial`. |
| `full_when_staged_addition` | `A.` → `Full`. |
| `partial_when_added_then_modified` | `AM` → `Partial`. |
| `full_when_staged_deletion` | `D.` → `Full`. |
| `unstaged_when_untracked` | `??` → `Unstaged`. |
| `full_when_staged_rename` | `R.` → `Full` (keyed by the new path). |
| `partial_when_renamed_then_modified` | `RM` → `Partial`. |
| `full_when_staged_copy` | `C.` → `Full`. |
| `unstaged_when_no_changes_on_either_side` | `..` → `Unstaged`. |
| `states_map_omits_unstaged_and_keys_partial_full_by_path` | Map keeps `Partial`/`Full` by path, omits `Unstaged`/untracked. |
| `kind_from_staged_code_maps_letters` | Index-side code → `FileChangeKind` for the header-only section letter. |

## Artifact: `S` state-machine + refresh tests (`src/ui/app.rs`)

**What it proves:** The stage/unstage/collapse/expand state machine and the
refresh collapse-map rules behave exactly as Unit 2 specifies, all through
the existing `StageOps` gestures recorded by the `FakeGit` harness.

| Test | One-line description |
| --- | --- |
| `stage_file_stages_the_file_and_collapses_its_section` | `S` on an unstaged file records `StageFile`, collapses it, keeps it as a `Full` section. |
| `stage_file_on_fully_staged_file_unstages_and_expands` | `S` on a fully-staged (collapsed) file records `UnstageFile` and re-expands it. |
| `stage_file_records_only_stageops_methods` | The `S` gesture only ever calls `stage_file`/`unstage_file` — no new git-layer code. |
| `stage_file_on_read_only_range_is_a_noop_with_message` | `S` on a `Range` target records nothing and sets "read-only diff target". |
| `stage_file_error_sets_message_and_leaves_state_unchanged` | A git failure surfaces the error and collapses/removes nothing. |
| `hunk_stage_marks_file_partial_and_keeps_it_expanded` | `space` on a hunk → file `Partial` (`±` header marker), section stays expanded. |
| `refresh_auto_expands_a_partially_staged_collapsed_file` | A staged-then-edited collapsed file re-expands with `±` (nothing hides). |
| `refresh_keeps_a_still_fully_staged_collapsed_file_collapsed` | A still-`Full` collapsed file stays collapsed. |
| `refresh_preserves_a_manually_collapsed_unstaged_file` | A `za`-collapsed unstaged file survives refresh. |
| `refresh_drops_collapse_entries_for_departed_files` | A file that leaves the review has its collapse-map entry dropped. |
| `nothing_hides_smoke_stage_two_then_edit_one_reexpands` | The end-to-end Unit-2 smoke (see below). |

`src/ui/keymap.rs`:

| Test | One-line description |
| --- | --- |
| `shift_s_resolves_to_stage_file_regardless_of_shift_bit` | `S` (with or without the SHIFT bit) resolves to `StageFile`; `s` still opens the panel. |

`src/ui/staging.rs` (decision B):

| Test | One-line description |
| --- | --- |
| `visual_stage_selection_rejects_cross_section_span` | A Visual span crossing a file boundary is rejected ("selection spans multiple files"). |
| `visual_stage_selection_scopes_body_index_to_second_file_section` | Body-line indices for a second file's hunk are scoped to that file's section, not offset by the first file's identically-indexed hunk. |

`src/ui/mod.rs` (TestBackend render):

| Test | One-line description |
| --- | --- |
| `partial_file_section_header_shows_partial_marker` | A `Partial` file's header renders the `±` marker. |

## Artifact: "Nothing hides" smoke — stage two, edit one, re-expand

**What it proves:** The full Unit-2 guarantee end to end: review three
files, stage two (they collapse), an external edit lands on a staged file,
and the next refresh re-expands that file with `±` — a collapsed header can
never silently hold unreviewed changes.

**Method:** `tmux` is not installed in this environment, so the smoke is
driven through the real `App`/`apply`/`refresh` path with the `FakeGit`
harness, whose refresh diff/status are read through mutable handles so the
test can mutate what the next `refresh` sees (the "external edit"). This is
the `nothing_hides_smoke_stage_two_then_edit_one_reexpands` test; the steps
and assertions it drives:

1. Three files `a.rs`, `b.rs`, `c.rs`, nothing staged — all three sections
   start **expanded**.
2. `S` on `a.rs` → records `StageFile("a.rs")`, `a.rs` **collapses**.
3. Mutate the fake to the post-(a,b)-stage state, move the cursor onto
   `b.rs`, `S` → records `StageFile("b.rs")`, `b.rs` **collapses** (a.rs
   stays collapsed). The recorded call log is exactly
   `[StageFile("a.rs"), StageFile("b.rs")]`.
4. Mutate the fake so `a.rs` is now `MM` (partially staged) and back in the
   working diff — the external edit — then `refresh()`.
5. Assertions: `a.rs` is **re-expanded**, its header marker is `±`
   (`StagedMarker::Partial`), and `b.rs` (still fully staged) **stays
   collapsed**.

## Decision (A): fully-staged files persist as header-only sections

**Which option I took:** the **header-only** realization of decision (A),
not the "render the file's staged diff inline" option.

`build_review`, for the `WorkingTree` target, unions the unstaged diff with
the fully-staged files from `git status` (those with staged changes and no
unstaged changes, not already present in the unstaged diff), merged into
the one flat list that is **sorted by path** (byte-wise ascending) with
every other entry. (Originally these sections were appended in status
order after the unstaged/untracked entries; that ordering was superseded
by the stable path sort so staging a file never moves it in the list —
mirroring Zed's git-panel feel.) Each is a
`FileDiff` with no hunks — `build_multibuffer` already renders a
zero-content file as a single addressable header row (covered by task 2.0's
`multibuffer_zero_content_file_is_header_only_but_addressable`), so the
section shows `▸/▾ <kind> <path> ●` and no body.

**Why this option:** it is the most localized change and avoids a
stage-direction ambiguity that belongs to task 4.0. Rendering the *staged*
diff inline (the preferred option in the task) would require a second
`ops.diff(Staged)` call and would leave `space` on those hunks trying to
`apply_cached` an already-staged hunk while the target is `WorkingTree` —
target-aware gating of the staging gestures is exactly task 4.0's job
(all-targets integration). With the header-only section there are no
line/hunk rows to mis-stage: the only gestures are `S` (unstage) and
`space`-on-header (whole-file), both correct. The `●` marker on the header
already communicates "fully staged"; I did **not** add a separate dim
placeholder row, to avoid introducing a new `Row` variant that would ripple
through the shared row-render/target-derivation matches — keeping the choice
localized to `build_review` + `rebuild_rows` per the task's guidance. Once a
fully-staged file is edited again it re-enters via the real unstaged diff
with full content, so no information is ever lost.

Ordering is stable, status-independent, and explicitly tested: all entries
(diff-parsed, untracked, fully-staged header-only) sort together by path,
so a file keeps its position when its staged state changes (see
`build_review_sorts_unstaged_untracked_and_fully_staged_by_path` and
`staging_a_file_keeps_its_position_in_the_list`).

## Decision (B): Visual selections are single-section

`visual_stage_selection` rejects a span whose line rows belong to more than
one file (`file_of_row`), before the per-file `hunk_index` is trusted, and
scopes its body-line counting to the selected file's `section_span` so a
second file's identically-numbered hunk can't shift the first file's
indices. Cursor motion in Visual mode still crosses sections freely; only
staging the span is constrained. Covered by
`visual_stage_selection_rejects_cross_section_span` and
`visual_stage_selection_scopes_body_index_to_second_file_section`.

## Reviewer Conclusion

The keep-it verdict is now one keypress: `S` stages the file under the
cursor and collapses it; `S` on a fully-staged header unstages and
re-expands. Per-file `StagedState` drives live `●`/`±` markers in both the
section header and the sidebar, derived by a TDD'd pure function covering
every porcelain combination. `refresh` guarantees nothing hides — a
staged-then-edited file re-expands with `±` — while fully-staged and
manually-collapsed files keep their state and departed files' entries are
cleaned up. Fully-staged files persist as header-only sections (decision A,
header-only option) so unstaging is always one keypress away, and Visual
staging is constrained to a single section (decision B). All four cargo
gates are green and the test count rose from 450 to 478 (+28). The staging
flow reused only the existing `StageOps` gestures — no new git-layer code —
leaving the all-targets surface integration (annotations, search, LSP,
target-aware gating) for task 4.0.
