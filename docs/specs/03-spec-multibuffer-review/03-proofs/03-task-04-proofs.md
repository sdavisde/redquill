# Task 04 Proofs — Full-surface integration across all diff targets

## Task Summary

Parent task 4.0 makes the whole review surface — annotations, search, LSP
peek, and panel selection — work across the multibuffer's file sections, for
every diff target, and gates the staging gestures where they don't apply.

- `src/ui/diff_view_state.rs` gains `anchor_row_in_buffer(target)`, the
  multibuffer-native anchor resolver: a `Target::File` maps to its
  section-header row, and line/hunk/range targets resolve *within the owning
  file's row span* (so a line number that also appears in another section can
  never be mismatched). `jump_to_annotation` (in `app.rs`) now routes through
  it and still expands a collapsed target section first.
- `src/ui/app.rs` adds the `select_file_by_path(path) -> bool` seam (spec
  02's interface): it expands a collapsed target, moves the cursor to the
  file's section header, scrolls it into view, and returns `false` for an
  unknown path. The sidebar highlight already follows `file_of_cursor()`
  (task 2.0), so moving the cursor is what "selects" the file everywhere.
- Search already recomputes over the whole row `Vec` on every rebuild; task
  4.2 pins that with cross-boundary / collapsed-section / wrap / collapse-
  toggle-recompute tests (no code change needed — `za` → `toggle_collapse`
  → `rebuild_rows` → `search.recompute` was already wired in task 2.0).
- `src/ui/code_intel.rs` `code_intel_position` already derives its path from
  `file_of_cursor()` and `peek_enter` already expands + scrolls to the target
  section; task 4.3 pins both with multi-file tests.
- `src/ui/help.rs` gains target-aware filtering: `render` takes a
  `staging_allowed` flag (`false` on a `Range` target), hiding the inert
  file/hunk stage gestures (`S`, `space`) from the overlay while keeping the
  still-working staging-panel toggle. `src/ui/mod.rs` derives the flag from
  `app.target` at the call site.

## What This Task Proves

- Annotation anchors resolve against the whole buffer (File → section header;
  line/hunk/range → within the owning section), gutter rows splice per
  section, and `jump_to_annotation` scrolls + expands a collapsed target.
- The markdown-on-quit stdout output is byte-identical: `src/annotate/` is
  untouched, and a multi-section review emits exactly the expected bytes.
- Search spans the buffer across file boundaries, collapsed sections
  contribute no matches, `n`/`N` wrap over the whole buffer, and a collapse
  toggle recomputes matches with no stale row indices.
- LSP peek derives its path from the cursor's owning file, and peek
  jump-to-location expands + scrolls to a collapsed target section.
- The select-by-path seam expands/scrolls/selects, and no-ops on unknown
  paths.
- The multibuffer renders for `--staged` and ref-range targets; staging keys
  are inert *and* absent from the `?` overlay on ranges, while the `Staged`
  target keeps its existing unstage semantics.
- All four repository gates pass and the test count strictly increased.

## Evidence Summary

| Check | Result |
| --- | --- |
| `cargo build` | pass |
| `cargo test` | pass — 494 tests (469 unit + 25 integration), 0 failed |
| `cargo clippy --all-targets -- -D warnings` | pass |
| `cargo fmt --check` | pass |
| Test count vs 478 baseline | 494 — strictly increased (+16 unit tests) |
| `git diff --stat src/annotate/` | empty (markdown stdout API untouched) |
| Smoke transcript | TestBackend frames (below); tmux unavailable in env |

Unit-test count moved from 453 to 469 (+16); integration tests unchanged at
25 (11 + 10 + 4). No pre-existing tests were changed or deleted — every
addition is net-new.

## Artifact: Four cargo gates green

**What it proves:** The full-surface integration compiles, every test passes,
clippy is warning-free under `-D warnings`, and formatting is canonical.

**Why it matters:** These four commands are the repo's blocking quality bar
(CLAUDE.md). The change touches the view-state anchor resolver, app wiring,
help rendering, and its call site; a green run demonstrates the seams are
consistent end to end.

**Command:** `cargo build && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`

**Result summary:** All four gates exit 0. Test output trimmed to the
`test result` summary lines (every test `ok`).

```
$ cargo build
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.43s

$ cargo test          # trimmed to "test result" lines
test result: ok. 469 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo clippy --all-targets -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.94s

$ cargo fmt --check
FMT_OK
```

## Artifact: Byte-identical markdown-on-quit output (public API)

**What it proves:** The multibuffer never touches the stdout markdown
contract. The format is keyed purely off the [`AnnotationStore`]'s insertion
order, independent of the row model.

**Why it matters:** The markdown-on-stdout format is a public API (CLAUDE.md /
spec Non-Goal 6). The whole point of this task is "no surface regression,
byte-identical output for equivalent reviews."

**Command:** `git diff --stat src/annotate/` and
`cargo test --lib multi_section_annotations_emit_unchanged_markdown`

**Result summary:** `git diff --stat src/annotate/` prints nothing — no
serialization code or its assertions changed. A new app-level test composes
file/line/hunk annotations across three separate sections and asserts the
exact rendered bytes.

```
$ git diff --stat src/annotate/
$          # (empty — the annotate/ tree is untouched)
```

`ui::app::multi_section_annotations_emit_unchanged_markdown` builds a
three-file `App`, adds a `File` annotation on `a.rs`, a `Line` annotation on
`b.rs`, and a `Hunk` annotation on `c.rs`, then asserts:

```
## a.rs

[praise] clean module

## b.rs:1 (+)

[question] why new0?

## c.rs:1-1 (+)

[nit] tidy this hunk
```

The eleven existing `src/annotate/markdown.rs` assertions all pass unchanged
(part of the 469-test run above).

## Artifact: New anchor / search / peek / select-by-path / gating tests

**What it proves:** Each surface generalizes correctly to the multibuffer,
pinned failing-first where the task calls for TDD (anchor resolution).

**Why it matters:** These are the review-surface invariants Unit 3 requires;
each test locks one down against the whole-buffer row model.

**4.1 — anchor resolution + annotations**

| Test | One-line description |
| --- | --- |
| `ui::rows::multibuffer_splices_annotation_gutter_rows_in_each_section` | Annotations in two files each splice their display row inside their own section span; every spliced row maps to its owning file. |
| `ui::diff_view_state::anchor_row_in_buffer_resolves_targets_within_owning_section` | `File` target → section-header row; `Line`/`Hunk` resolve within the owning file's span (never a neighbor's identically-numbered line); unknown path → `None`. |
| `ui::app::jump_to_annotation_expands_a_collapsed_target_section` | Jumping to an annotation in a collapsed file re-expands it and lands the cursor on the anchor line. |
| `ui::app::multi_section_annotations_emit_unchanged_markdown` | A 3-file review with file/line/hunk annotations across sections emits byte-identical markdown. |

**4.2 — search across the buffer**

| Test | One-line description |
| --- | --- |
| `ui::app::search_matches_span_file_boundaries` | A search spans the whole buffer: one match per file, in different sections. |
| `ui::app::collapsed_section_contributes_no_search_matches` | A collapsed file's rows are absent, so it yields no matches. |
| `ui::app::search_next_wraps_across_the_whole_buffer` | `n`/`N` advance across sections and wrap around either end. |
| `ui::app::toggling_collapse_recomputes_search_matches_without_stale_indices` | `za` on a matched file recomputes matches (drops it) with no stale row indices. |

**4.3 — LSP peek in the multibuffer**

| Test | One-line description |
| --- | --- |
| `ui::code_intel::code_intel_position_derives_path_from_the_cursor_row_owning_file` | The cursor in the *second* section issues its request against the second file's path. |
| `ui::code_intel::peek_enter_expands_a_collapsed_target_section` | Peek jump-to-location re-expands a collapsed target section and lands on the target line within its span. |

**4.4 — select-by-path seam**

| Test | One-line description |
| --- | --- |
| `ui::app::select_file_by_path_moves_cursor_to_section_header` | Moves the cursor to the file's header row; sidebar selection follows. |
| `ui::app::select_file_by_path_expands_a_collapsed_target` | Expands a collapsed target before selecting. |
| `ui::app::select_file_by_path_unknown_path_is_a_noop_returning_false` | Unknown path returns `false` and changes nothing. |

**4.5 — all-targets parity + staging gating**

| Test | One-line description |
| --- | --- |
| `ui::mod::multibuffer_renders_for_a_range_target` | A multi-file `Range` review renders every section header + body (TestBackend). |
| `ui::mod::help_overlay_hides_staging_rows_on_a_range_target` | Help omits `S`/`space` staging gestures on `Range` but keeps the panel toggle. |
| `ui::mod::help_overlay_shows_staging_rows_on_the_working_tree_target` | Help lists all staging gestures on `WorkingTree`. |

The pre-existing `ui::app::staged_target_space_unstages_hunk` /
`staged_target_file_header_unstages_file` continue to pass, so the `Staged`
target keeps its unstage semantics (no regression); and the pre-existing
`range_target_space_is_noop_with_message` /
`stage_file_on_read_only_range_is_a_noop_with_message` prove `space`/`S` are
already inert no-ops on a range target (setting only a status message).

## Artifact: Range-review smoke transcript (TestBackend)

**What it proves:** A ref-range review renders as one scrollable multibuffer
of collapsible sections, and its `?` help overlay omits the inert staging
gestures.

**Why it matters:** This is the all-targets-parity manual evidence Unit 3's
proof artifacts call for.

**Method:** `tmux` is not installed in this environment, so these frames were
produced by driving the real `App`/`draw` render path through
`ratatui::backend::TestBackend`, with `app.target = DiffTarget::Range(…)`,
and dumping the composited cells. Each frame is the actual UI, not a mock.

**Frame 1 — a two-file range multibuffer (72×14):**

```
┌a.rs──────────────────────────────────┐┌files─────────────────────────┐
│  ▾ M a.rs                            ││  M a.rs                      │
│  @@ -1,1 +1,1 @@                     ││  M b.rs                      │
│      1       -old                    ││                              │
│            1 +new                    ││                              │
│  ▾ M b.rs                            ││                              │
│  @@ -1,1 +1,1 @@                     ││                              │
│      1       -old                    ││                              │
│            1 +new                    ││                              │
│                                      ││                              │
│                                      ││                              │
│                                      │└──────────────────────────────┘
└──────────────────────────────────────┘ [2 files]
```

**Frame 2 — the same range review's `?` help overlay, Stage group only
(72×40, trimmed to the relevant rows):** the `Stage` group shows *only* the
still-working panel toggle — the `S` and `space` gestures are absent:

```
│Stage                                                                │
│                 s  Toggle staging panel                             │
│                                                                     │
│Search                                                               │
```

For contrast, on the `WorkingTree` target the same group renders three rows
(`space` Stage/unstage hunk, `S` Stage/unstage file under cursor, `s` Toggle
staging panel) — pinned by
`help_overlay_shows_staging_rows_on_the_working_tree_target`.

## Decision: fully-staged expanded sections get no dim placeholder row

The task's optional last bullet asks for a dim placeholder row (e.g.
`(fully staged — S to unstage)`) in a manually-expanded fully-staged section,
"if it doesn't ripple". **It ripples, so it is skipped**, per the task's
explicit fallback.

A fully-staged file is a header-only section (task 3.0, decision A). Adding a
placeholder body requires a new `Row` variant, which would fan out through
every *exhaustive* `match` on `Row` — `App::target_for_cursor` (no wildcard
arm), the `diff_view` renderer, `Row::is_addressable`, and the visual-staging
/ search paths — the exact ripple task 3.0's decision (A) chose the
header-only realization to avoid. The `●` marker on the header already
communicates "fully staged", and such a section is only ever seen after a
deliberate `za` on an already-collapsed staged file (fully-staged files start
*collapsed*), so the empty body is a rare, self-inflicted edge. The safe,
spec-sanctioned choice is to leave it out and preserve the addressability /
byte-identical invariants this task exists to protect.

## Reviewer Conclusion

The whole review surface now works across the multibuffer's sections for
every diff target. Annotation anchors resolve against the whole buffer via
the new `anchor_row_in_buffer` (File → section header; line/hunk/range →
within the owning span), gutter rows splice per section, and
`jump_to_annotation` scrolls + expands a collapsed target — while the
markdown-on-quit stdout API is byte-identical (`src/annotate/` untouched).
Search spans file boundaries, skips collapsed sections, wraps over the whole
buffer, and recomputes on collapse toggle. LSP peek derives its path from the
cursor's owning file and expands + scrolls to a collapsed target. The
`select_file_by_path` seam expands/scrolls/selects and no-ops on unknown
paths. The multibuffer renders for `--staged` and ref-range targets; on
ranges the staging gestures are inert *and* hidden from the `?` overlay,
while the `Staged` target keeps its unstage semantics. All four cargo gates
are green and the test count rose from 478 to 494 (+16). The one optional
placeholder-row nicety is skipped with a documented ripple rationale — the
performance hardening and final keymap/docs sweep remain for task 5.0.
