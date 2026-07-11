# 01-tasks-app-decomposition.md

Task list for `01-spec-app-decomposition.md`. Each demoable unit is a parent
task with checkbox subtasks and its proof artifact. Behavior-preserving moves
only; the full gate set (`cargo build` / `cargo test` / `cargo clippy -- -D
warnings` / `cargo fmt --check`) stays green before every commit, and the test
count never decreases (baseline: 412 lib unit tests + 3 integration files).

## Unit 1: `DiffViewState` extraction

Isolate the "one view over one diff" state and its motion/clamp/visibility/
hunk-jump logic into a new `src/ui/diff_view_state.rs` module (the render
module already owns `diff_view.rs`). `App` delegates; behavior identical.

- [x] Create `src/ui/diff_view_state.rs` with a `DiffViewState` struct owning:
      `files`, `selected_file`, `rows`, `sbs_rows`, `sbs_visual_of`, `cursor`,
      `scroll`, `sbs_scroll`, `layout` (ViewMode), `viewport_height`,
      `cursor_col`; wire it into `mod.rs`.
- [x] Move pure motion/clamp/visibility logic onto it: `max_cursor`,
      `nearest_addressable`, `ensure_visible`, `half_page`, viewport get/set,
      `toggle_view`, the four cursor-motion bodies, and the column-cursor
      methods (`effective_column`, `cursor_line_content`, `move_column_*`,
      `move_word_*`).
- [x] Move hunk navigation (`hunk_header_rows`, in-file jump, and the file
      probes `probe_first_hunk_row`/`probe_last_hunk_row`) onto it; `App`
      keeps `switch_file`/`next_hunk`/`prev_hunk` orchestration so the
      cross-file rebuild still highlights identically.
- [x] Repoint `App` fields to `self.view.*` so the render modules
      (`diff_view`, `sbs_view`, `sidebar`, panels) and all call sites keep
      compiling and behaving identically.
- [x] Relocate the moved logic's unit tests alongside `DiffViewState` where
      they exercise it directly; keep `App`-level behavior tests in `app.rs`.

**Proof:** `cargo test` still shows the full suite passing (>= 412 lib tests);
`cargo clippy -- -D warnings` + `cargo fmt --check` green; `wc -l
src/ui/app.rs src/ui/diff_view_state.rs` shows state moved, not duplicated.

## Unit 2: Modal states and service-glue extraction

Empty the remaining concerns out of `App` so it becomes a coordinator.

- [x] Extract modal key handlers (`handle_compose_key`, `handle_list_key`,
      `handle_staging_key`, `handle_search_key`, `handle_peek_key`) into a
      `modes` module grouping, driving `App` through its public methods.
- [x] Extract the staging gesture logic (`toggle_stage`, `run_stage_gesture`,
      `visual_stage_selection`, `StageGesture`) into a UI-side `staging`
      module operating through the existing `StageOps` trait seam (`refresh`
      stays in `App` as the highlighting rebuild coordinator, called via
      `pub(super)`).
- [x] Extract the code-intelligence glue (`code_intel_position`, `request`,
      `poll`, `handle_event`, `open_peek_locations`, peek preview/navigation)
      into a dedicated `code_intel` module that takes a view cursor position
      as input rather than reaching into `App` internals.
- [~] `src/ui/app.rs` coordinator *code* is ~880 lines (near the advisory
      ~800; the target is advisory per Open Question 3). The file total
      remains larger because App-behavior integration tests (navigation,
      compose, list, search, staging-via-Action, target derivation) that
      share fixtures were retained with `App`. Help overlay, keymap table,
      and all dispatch behavior unchanged.
- [x] Relocate the moved code's unit tests alongside it (search-handler
      tests → `modes`; LSP/peek/utf16 tests → `code_intel`; a colocated
      pure-selection test → `staging`).

**Proof:** `cargo test` / `cargo clippy -- -D warnings` / `cargo fmt --check`
all green; `wc -l src/ui/app.rs` at or under the target; no keybinding,
rendering, or layout change.

## Unit 3: Background task poller utility

Provide the reusable "spawn work on a background thread, drain results per
render tick" primitive spec 02 needs, generalized from `LspManager`'s thread +
mpsc channel + non-blocking poll pattern. Transport-agnostic (runs closures /
commands); no git or LSP types. Ships with no production callers.

- [ ] Create a background-task module under `src/ui/` with a poller that
      `spawn`s a task on a background thread, returns a task id immediately,
      and drains completed results via a non-blocking `poll()`.
- [ ] Report failure (command exit status + stderr, or a panicked closure) as
      a value, never a panic; no `unwrap`/`expect` outside tests.
- [ ] Unit tests with synthetic tasks covering success, failure, and
      not-yet-complete (pending) polling.
- [ ] Allow `dead_code` narrowly if needed so it compiles clean with no
      callers other than tests.

**Proof:** new poller unit tests passing under `cargo test`; utility compiles
with no non-test callers under `cargo clippy -- -D warnings`.
