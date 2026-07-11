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

- [ ] Create `src/ui/diff_view_state.rs` with a `DiffViewState` struct owning:
      `files`, `selected_file`, `rows`, `sbs_rows`, `sbs_visual_of`, `cursor`,
      `scroll`, `sbs_scroll`, `view`, `viewport_height`, `cursor_col`; wire it
      into `mod.rs`.
- [ ] Move pure motion/clamp/visibility logic onto it: `max_cursor`,
      `nearest_addressable`, `ensure_visible`, `half_page`, viewport get/set,
      `toggle_view`, the four cursor-motion bodies, and the column-cursor
      methods (`effective_column`, `cursor_line_content`, `move_column_*`,
      `move_word_*`).
- [ ] Move hunk navigation (`hunk_header_rows`, `next_hunk`, `prev_hunk`) and
      `switch_file` onto it, with `App` supplying the row-rebuild step so
      cross-file jumps still highlight identically.
- [ ] Repoint `App` fields to `self.view.*` (or delegating accessors) so the
      render modules (`diff_view`, `sbs_view`, `sidebar`, panels) and all
      call sites keep compiling and behaving identically.
- [ ] Relocate the moved logic's unit tests alongside `DiffViewState` where
      they exercise it directly; keep `App`-level behavior tests in `app.rs`.

**Proof:** `cargo test` still shows the full suite passing (>= 412 lib tests);
`cargo clippy -- -D warnings` + `cargo fmt --check` green; `wc -l
src/ui/app.rs src/ui/diff_view_state.rs` shows state moved, not duplicated.

## Unit 2: Modal states and service-glue extraction

Empty the remaining concerns out of `App` so it becomes a coordinator.

- [ ] Extract modal key handlers (`handle_compose_key`, `handle_list_key`,
      `handle_staging_key`, `handle_search_key`, `handle_peek_key`, currently
      `src/ui/mod.rs:210-320`) into a `modes` module grouping, driving `App`
      through its public methods.
- [ ] Extract the staging gesture logic (`toggle_stage`, `run_stage_gesture`,
      `visual_stage_selection`, `StageGesture`, `refresh`) into a UI-side
      staging module operating through the existing `StageOps` trait seam.
- [ ] Extract the code-intelligence glue (`code_intel_position`,
      `request_code_intel`, `poll_lsp`, `handle_lsp_event`,
      `open_peek_locations`, peek-preview correlation) into a dedicated module
      that takes a view cursor position as input rather than reaching into
      `App` internals.
- [ ] Reduce `src/ui/app.rs` to ~800 lines or fewer while keeping the `?`
      help overlay, keymap table, and all dispatch behavior unchanged.
- [ ] Relocate the corresponding unit tests alongside the moved code.

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
