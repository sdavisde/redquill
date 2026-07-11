# Task 02 Proofs — Multi-file row model with collapsible sections

## Task Summary

Parent task 2.0 replaces the one-file-at-a-time diff view with a Zed-style
**multibuffer**: every changed file's rows are concatenated into one
scrollable buffer of collapsible per-file sections.

- `src/ui/rows.rs` gains `StagedMarker`, an extended section-header
  `Row::FileHeader { …, file_index, staged_marker, collapsed }`, the
  `MultibufferRows { rows, file_of_row, header_row_of_file }` struct, and
  `build_multibuffer(files, collapsed, staged_markers, annotations, syntax)`.
  The per-file build logic is factored into a shared `append_file_rows`
  helper; `build_rows` (single file) is now a thin wrapper over it.
- `src/ui/diff_view_state.rs` stores the whole-buffer rows plus the
  `file_of_row`/`header_row_of_file` maps and a path-keyed collapse map;
  adds `file_of_cursor()`, `section_span()`, `toggle_collapse_at_cursor()`,
  whole-buffer `next_hunk`/`prev_hunk` (crossing file boundaries), and
  `next_section`/`prev_section` header jumps; `selected_file` is now a
  derived value re-synced from the cursor in `ensure_visible`. The
  throwaway `probe_first_hunk_row`/`probe_last_hunk_row` builders are gone.
- `src/ui/keymap.rs` adds `Action::ToggleCollapse` bound to `za` (extending
  the two-key prefix machine to `z`) and repurposes `Tab`/`Shift-Tab` to
  the section-header jumps.
- `src/ui/app.rs` rewrites `rebuild_rows` to build the whole multibuffer,
  lazily highlighting only expanded files; routes `target_for_cursor` /
  `target_for_visual` / jump / peek / refresh through `file_of_cursor()` and
  section spans; sets initial collapse from `staged` membership.
- `src/ui/diff_view.rs` renders section headers with the `▾`/`▸` indicator,
  kind letter, path (rename arrow), and a `●`/`±` marker slot; collapsed
  sections render exactly one line. `src/ui/sidebar.rs` follows
  `file_of_cursor()`. `src/ui/help.rs` + `README.md` document `za` and the
  repurposed `Tab`.

## What This Task Proves

- The multi-file row builder is correct: concatenation order, per-row file
  identity, header-row indices, collapse filtering (a collapsed file
  contributes exactly its header row), section-header content and markers,
  addressability, synthetic untracked sections, and header-only zero-content
  files.
- Navigation is correct across file boundaries: `j`/`Ctrl-d` cross sections,
  `]`/`[` cross into neighboring expanded files and skip collapsed ones,
  `Tab`/`Shift-Tab` jump between section headers, `za` toggles collapse, and
  the cursor stays clamped/addressable after rebuilds.
- The review surface is preserved: annotations still splice per file, search
  spans the buffer, LSP peek and staging derive their file from the cursor's
  owning section — with all pre-existing app/code-intel/staging tests still
  green.
- All four repository gates pass and the test count strictly increased.

## Evidence Summary

| Check | Result |
| --- | --- |
| `cargo build` | pass |
| `cargo test` | pass — 450 tests (425 unit + 25 integration), 0 failed |
| `cargo clippy -- -D warnings` | pass |
| `cargo fmt --check` | pass |
| Test count vs 430 baseline | 450 — strictly increased (+20 unit tests) |
| Smoke transcript | TestBackend frames (below); tmux unavailable in env |

Unit-test count moved from 405 to 425 (+20); integration tests unchanged at
25 (11 + 10 + 4). A handful of pre-existing tests were renamed/rewritten to
match the new semantics (`Tab` now jumps section headers; the multibuffer
highlights every expanded file at build time), not deleted.

## Artifact: Four cargo gates green

**What it proves:** The multibuffer rewrite compiles, every test passes,
clippy is warning-free under `-D warnings`, and formatting is canonical.

**Why it matters:** These four commands are the repo's blocking quality bar
(CLAUDE.md). The change touches the row model, view state, app wiring,
keymap, and three renderers; a green build+test run demonstrates the seams
are consistent end to end.

**Command:** `cargo build && cargo test && cargo clippy -- -D warnings && cargo fmt --check`

**Result summary:** All four gates exit 0. Test output trimmed to the
`test result` summary lines (every test `ok`).

```
$ cargo build
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.80s

$ cargo test          # trimmed to "test result" lines
test result: ok. 425 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo clippy -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.64s

$ cargo fmt --check
FMT_OK
```

## Artifact: New row-builder tests (`src/ui/rows.rs`)

**What it proves:** The pure `build_multibuffer` derivation is correct across
every property Unit 1 requires. Written failing-first (TDD) against the
new struct, then implemented to green.

**Why it matters:** This builder is the structural core the staging flow
(3.0) and surface integration (4.0) build on; its correctness is what keeps
addressability (annotations/staging/LSP) intact across the buffer.

| Test | One-line description |
| --- | --- |
| `multibuffer_concatenates_files_in_order_with_header_indices` | Files concatenate in order; `header_row_of_file` marks each section header; `file_index` on each header matches. |
| `multibuffer_file_of_row_maps_every_row_to_its_file` | `file_of_row` maps each row to its owning file index. |
| `multibuffer_collapsed_file_contributes_exactly_its_header_row` | A collapsed file yields exactly one row (its header, `collapsed: true`); the maps stay consistent. |
| `multibuffer_header_carries_staged_marker` | The section header carries the file's `StagedMarker`. |
| `multibuffer_preserves_addressability_of_rows` | Header/hunk/line rows are addressable; annotation display rows are not. |
| `multibuffer_synthetic_untracked_file_is_a_normal_section` | A `FileDiff::synthetic_added` untracked file enters as a normal addressable section. |
| `multibuffer_zero_content_file_is_header_only_but_addressable` | A file with no hunks renders header-only yet stays expandable and addressable. |

## Artifact: New navigation tests

**What it proves:** Cross-file motion, hunk jumps across sections, section
header jumps, collapse toggling, and cursor clamping all behave correctly on
the whole-buffer row model.

**Why it matters:** These pin the "one continuous document" behavior — the
buffer moves like a single scrollable file even though it spans many.

`src/ui/diff_view_state.rs`:

| Test | One-line description |
| --- | --- |
| `next_hunk_advances_then_stops_at_last` | `]` advances hunk-to-hunk and stops at the last hunk. |
| `cursor_down_crosses_into_the_next_file_section` | `j` past a file's last row lands in the next file's section; `file_of_cursor`/`selected_file` follow. |
| `next_hunk_crosses_the_file_boundary` | `]` crosses from one file's hunk into the next file's hunk. |
| `next_hunk_skips_a_collapsed_section` | `]` skips a collapsed section (it contributes no hunk headers). |
| `next_and_prev_section_jump_between_headers` | `Tab`/`Shift-Tab` jump forward/back between section headers and clamp at the ends. |
| `toggle_collapse_at_cursor_flips_state_for_the_cursor_file` | `za` toggles collapse for the cursor's file only. |
| `cursor_clamps_into_range_after_a_smaller_rebuild` | The cursor stays in range and addressable after the buffer shrinks. |

`src/ui/app.rs`:

| Test | One-line description |
| --- | --- |
| `toggle_collapse_collapses_and_expands_file_under_cursor` | `ToggleCollapse` collapses the cursor file to a header row, keeps the cursor addressable, then expands it back. |
| `toggle_collapse_targets_the_cursor_file_not_the_first` | Collapse targets the cursor's file, not file 0. |
| `next_file_jumps_to_next_section_header` | Repurposed `Tab` lands the cursor on the next file's section header. |
| `prev_file_jumps_back_across_sections` | Repurposed `Shift-Tab` jumps back across section headers. |
| `multibuffer_highlights_every_expanded_file_once` | Every expanded file's in-use sides are highlighted once at build; later motions re-fetch nothing. |
| `collapsed_file_is_not_highlighted_until_expanded` | A file that starts collapsed is not highlighted until expanded (lazy per-file population). |

`src/ui/keymap.rs`:

| Test | One-line description |
| --- | --- |
| `z_starts_a_sequence_and_za_toggles_collapse` | `z` is a two-key prefix; `za` resolves to `ToggleCollapse`. |
| `resolve_completes_za_across_two_events` | The pending-prefix state machine completes `za` across two key events. |

`src/ui/mod.rs` (TestBackend render):

| Test | One-line description |
| --- | --- |
| `multibuffer_renders_all_section_headers` | All section headers render (▾ indicator, `M` kind letter, paths) with bodies visible. |
| `collapsed_section_renders_header_only_with_collapsed_indicator` | A collapsed section renders exactly one line (▸ indicator) with no body rows. |
| `staged_file_section_header_shows_marker` | A staged file's header shows the `●` marker slot. |

## Artifact: Smoke transcript — multibuffer in action

**What it proves:** The multibuffer behaves as one continuous document:
three files render as concatenated collapsible sections, the cursor scrolls
across a file boundary, and `za` collapses/expands a section in place.

**Why it matters:** This is the manual smoke evidence Unit 1's proof
artifacts call for — end-to-end scrolling and mid-scroll collapse/expand.

**Method:** `tmux` is not installed in this environment, so this transcript
was produced by driving the real `App`/`draw` render path through
`ratatui::backend::TestBackend` (a 72×18 buffer), applying the same
`CursorDown`/`ToggleCollapse` actions the keymap dispatches, and dumping the
rendered cells. Each frame is the actual composited UI (diff pane + sidebar +
footer), not a mock.

**Frame 1 — initial: three expanded sections, cursor on the first header:**

```
┌src/auth.rs───────────────────────────┐┌files─────────────────────────┐
│  ▾ M src/auth.rs                     ││  M src/auth.rs               │
│  @@ -1,3 +1,3 @@                     ││  M src/keys.rs               │
│      1     1  ctx                    ││  M src/mod.rs                │
│      2       -old in src/auth.rs     ││                              │
│            2 +new in src/auth.rs     ││                              │
│  ▾ M src/keys.rs                     ││                              │
│  @@ -1,3 +1,3 @@                     ││                              │
│      1     1  ctx                    ││                              │
│      2       -old in src/keys.rs     ││                              │
│            2 +new in src/keys.rs     ││                              │
│  ▾ M src/mod.rs                      ││                              │
│  @@ -1,3 +1,3 @@                     ││                              │
│      1     1  ctx                    ││                              │
│      2       -old in src/mod.rs      ││                              │
│            2 +new in src/mod.rs      │└──────────────────────────────┘
└──────────────────────────────────────┘ [3 files]
```

**Frame 2 — after 5× `j`: the cursor crossed from `src/auth.rs` into
`src/keys.rs`** (note the pane title and sidebar highlight now follow the
cursor's owning file — one buffer, no file-switch ritual):

```
┌src/keys.rs───────────────────────────┐┌files─────────────────────────┐
│  ▾ M src/auth.rs                     ││  M src/auth.rs               │
│  @@ -1,3 +1,3 @@                     ││  M src/keys.rs               │
│      1     1  ctx                    ││  M src/mod.rs                │
│      2       -old in src/auth.rs     ││                              │
│            2 +new in src/auth.rs     ││                              │
│  ▾ M src/keys.rs                     ││                              │
│  @@ -1,3 +1,3 @@                     ││                              │
│      1     1  ctx                    ││                              │
│      2       -old in src/keys.rs     ││                              │
│            2 +new in src/keys.rs     ││                              │
│  ▾ M src/mod.rs                      ││                              │
...
```

**Frame 3 — after `za`: `src/keys.rs` collapsed to a single `▸` header line**
(its body rows are gone; `src/mod.rs` slides up):

```
┌src/keys.rs───────────────────────────┐┌files─────────────────────────┐
│  ▾ M src/auth.rs                     ││  M src/auth.rs               │
│  @@ -1,3 +1,3 @@                     ││  M src/keys.rs               │
│      1     1  ctx                    ││  M src/mod.rs                │
│      2       -old in src/auth.rs     ││                              │
│            2 +new in src/auth.rs     ││                              │
│  ▸ M src/keys.rs                     ││                              │
│  ▾ M src/mod.rs                      ││                              │
│  @@ -1,3 +1,3 @@                     ││                              │
│      1     1  ctx                    ││                              │
│      2       -old in src/mod.rs      ││                              │
│            2 +new in src/mod.rs      ││                              │
│                                      ││                              │
└──────────────────────────────────────┘ [3 files]
```

**Frame 4 — after `za` again: `src/keys.rs` expands back** to its full
section (identical to Frame 1's layout), demonstrating collapse is a
reversible, in-place fold.

## Reviewer Conclusion

The one-file-at-a-time diff view is now a single scrollable multibuffer of
collapsible per-file sections. The pure `build_multibuffer` derivation is
covered by seven property tests (concatenation, identity maps, collapse
filtering, markers, addressability, synthetic sections, header-only files);
navigation across file boundaries, section jumps, and `za` collapse are
covered by view- and app-level tests; three TestBackend render tests pin the
section-header visuals. All four cargo gates are green and the test count
rose from 430 to 450. The smoke transcript shows the buffer scrolling across
a file boundary and a section collapsing/expanding in place. Addressability
(the per-side line numbers annotations/staging/LSP key off) is preserved
within every expanded section, so `annotate/`, `lsp/`, and `git/` needed no
changes — the seam the staging flow (3.0) and full-surface integration (4.0)
build on next.
