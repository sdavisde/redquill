# 02-tasks-git-panel.md

Task list for `02-spec-git-panel.md`. Sequencing: spec 01 (app decomposition) is merged; `BackgroundTasks<T>` (`src/ui/background.rs`) is the purpose-built, currently-unused seam for task 4.0. Shared files with spec 03 (`keymap.rs`, `help.rs`, `ui/mod.rs`) receive additive changes only, per spec 01's disjointness constraint.

## Relevant Files

| File | Why It Is Relevant |
| --- | --- |
| `src/git/branch.rs` (new) | `BranchStatus` struct and parser for porcelain-v2 `# branch.head` / `# branch.upstream` / `# branch.ab` headers. |
| `src/git/stash.rs` (new) | `StashEntry` struct and parser for `git stash list --format=...` output. |
| `src/git/remote.rs` (new) | Fixed-argv construction for `git fetch`/`pull`/`push` with `GIT_TERMINAL_PROMPT=0`; never `--force`. |
| `src/git/runner.rs` | Extend the existing status call with `--branch`; add stash-list and remote command execution methods. |
| `src/git/status.rs` | `parse_porcelain_v2` must route/skip `#` header lines it does not already handle. |
| `src/git/mod.rs` | Export the new types and functions. |
| `src/ui/git_panel.rs` (new) | The panel widget: branch header, CHANGES/UNTRACKED/STASHES sections, cursor state and navigation model. Evolves `sidebar.rs`. |
| `src/ui/command_log.rs` (new) | Bounded (~50-entry) in-memory command log model and bottom-pane render. |
| `src/ui/sidebar.rs` | Current passive sidebar; its row rendering (staged `●`, change-kind letters, path split, footer counts) migrates into `git_panel.rs`. |
| `src/ui/keymap.rs` | Additive scope dimension on `Binding` (diff vs. panel); new `Action` variants for focus toggle, fetch/pull/push, command-log toggle. |
| `src/ui/help.rs` | Group bindings by scope in the `?` overlay; hints for panel focus. |
| `src/ui/mod.rs` | Event loop: dispatch panel-scoped keys when panel is focused; drain `BackgroundTasks::poll` each tick; layout: focused-pane border emphasis, command-log bottom-pane slot. |
| `src/ui/app.rs` | Hold branch/stash state, `BackgroundTasks` field, in-flight-op guard, focus state; extend `refresh()`; narrow `select_file_by_path` call. |
| `src/ui/modes.rs` | Key handling for panel-focused input, following the existing modal-handler pattern. |
| `src/ui/background.rs` | First production caller lands; remove `#![allow(dead_code)]`. |
| `src/ui/stage_ops.rs` | The `StageOps` seam gains branch/stash read methods so `App` tests can fake them (mirrors existing `FakeGit` pattern). |
| `tests/git_integration.rs` | Integration coverage for branch/ahead-behind/stash read models against a real tempdir repo. |
| `tests/git_remote_integration.rs` (new) | Integration coverage for fetch/pull/push against a `file://` bare remote, including a conflict-producing pull. |
| `README.md` | Ratify and document the new keybindings in the canonical keymap table. |

### Notes

- Unit tests are colocated in `#[cfg(test)] mod tests` blocks inside each source file, per repo convention — no separate unit-test files.
- Integration tests reuse the existing helpers (`init_repo()`, `git(dir, args)`, `write(dir, rel, contents)`, `git_out`) from `tests/git_integration.rs` / `tests/git_stage_integration.rs`; never test against the host repo.
- TDD is required for the pure parsing code (branch headers, stash list, command-log model): write the failing test first, commit tests with the code.
- All four gates pass at every commit: `cargo build`, `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check`.
- Keybinding ratification happens in README.md first; any conflict discovered there is resolved in README before wiring the key.

## Tasks

### [x] 1.0 Repository state read models (branch, upstream, ahead/behind, stashes)

Extend `git/` with typed, TDD-tested knowledge of repository sync state: extend the existing `status --porcelain=v2 -z` call with `--branch` and parse the `# branch.head` / `# branch.upstream` / `# branch.ab` headers into a typed struct; add a `git stash list --format=...` parser producing typed entries (ref, source branch, message). Degrade gracefully (detached HEAD → short commit id, no upstream → no counts, zero stashes → empty list); errors via `thiserror`; no TUI types in `git/`.

#### 1.0 Proof Artifact(s)

- Test: unit tests in `src/git/` over recorded porcelain `--branch` header fixtures (normal, detached-HEAD, no-upstream, ahead/behind) and stash `--format` output (empty, multi-entry, message containing separators) pass via `cargo test`, demonstrating correct parsing of every degradation case.
- Test: integration test in `tests/git_integration.rs` building a tempdir repo with a local upstream (ahead 2 / behind 1 arranged via fixture commits) and one stash, asserting the parsed branch name, ahead/behind counts, and stash entry, demonstrating the read models against real git.
- CLI: `cargo test` summary showing the lib + integration test count strictly increased from the pre-task baseline, demonstrating the repo's test-growth standard.

#### 1.0 Tasks

- [x] 1.1 Write failing unit tests for branch-header parsing in a new `src/git/branch.rs`: fixtures for a normal branch with upstream and `# branch.ab +2 -1`, a detached HEAD (`# branch.head (detached)` plus `# branch.oid`), and a branch with no upstream (no `branch.upstream`/`branch.ab` lines).
- [x] 1.2 Implement `BranchStatus` (name-or-short-oid, `Option<upstream>`, `Option<(ahead, behind)>`) and its header parser in `src/git/branch.rs`, errors via the existing `GitError` (`thiserror`), no TUI types; export from `src/git/mod.rs`; tests from 1.1 pass.
- [x] 1.3 Add `--branch` to the status invocation in `src/git/runner.rs`, feed `#`-prefixed header lines to the branch parser, and confirm `parse_porcelain_v2` still passes all existing tests with headers present in the stream.
- [x] 1.4 Write failing unit tests for stash-list parsing: choose an explicit `--format` with an unambiguous field separator (e.g. `%gd%x00%gs`), fixtures for empty output, multiple entries, and a stash message containing spaces/colons.
- [x] 1.5 Implement `StashEntry` (ref such as `stash@{0}`, source branch, message) and its parser in `src/git/stash.rs`; add a `stash_list` method to `GitRunner`; export from `src/git/mod.rs`; tests from 1.4 pass.
- [x] 1.6 Add an integration test to `tests/git_integration.rs` using the existing tempdir helpers: create a local bare upstream, arrange ahead-2/behind-1 via fixture commits, create one stash, and assert the parsed branch name, upstream, counts, and stash entry. Also cover the detached-HEAD and no-upstream cases against real git.
- [x] 1.7 Run all four gates (`cargo build`, `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check`) and record the test-count delta against the pre-task baseline.

### [ ] 2.0 Git panel rendering: branch header and sectioned file/stash display

Evolve the passive `sidebar.rs` into the git panel's visual form in the same 32-column slot: branch header (`git: main ↑2↓1`, detached/no-upstream variants), CHANGES section preserving the existing staged `●` markers and change-kind letters, UNTRACKED section, STASHES section (ref, branch, message), and the existing footer counts. Panel remains passive in this task — no focus or cursor yet. `App` gains branch/stash state populated on startup and on `refresh()`.

#### 2.0 Proof Artifact(s)

- Test: `TestBackend` render tests asserting the branch header text (including `↑N↓M` glyphs, detached-HEAD and no-upstream variants), the three section headers, preserved `●` staged markers, and stash rows, demonstrating the layout matches the spec's design mock.
- Test: unit test asserting `App::refresh` repopulates branch/stash state alongside the existing snapshot (staged markers and annotations preserved), demonstrating refresh integration.
- Manual smoke transcript: `cargo run` in a fixture repo with an upstream and two stashes showing the populated panel, demonstrating live end-to-end rendering.

#### 2.0 Tasks

- [ ] 2.1 Extend the `StageOps` seam (`src/ui/stage_ops.rs`) with branch-status and stash-list reads, implement them on `GitRunner`, and extend the `FakeGit` test double in `src/ui/app.rs` tests to serve canned values.
- [ ] 2.2 Add branch and stash state to `App`, populated at startup and inside `refresh()`; write a unit test with `FakeGit` asserting `refresh()` repopulates branch/stash state while staged markers and annotations survive exactly as today.
- [ ] 2.3 Write failing `TestBackend` render tests for the panel: header `git: main ↑2↓1`, detached-HEAD variant (short oid), no-upstream variant (no arrows), the three section headers, `●` staged markers, change-kind letters, stash rows (`0 wip: parser` style), and the footer counts line.
- [ ] 2.4 Create `src/ui/git_panel.rs` rendering the branch header and CHANGES/UNTRACKED/STASHES sections in the existing 32-column slot, migrating the row rendering (staged marker, letter colors, dir/basename split, footer) from `sidebar.rs`; wire `draw()` in `src/ui/mod.rs` to the new widget; tests from 2.3 pass.
- [ ] 2.5 Empty-state handling: zero stashes hides or shows an empty STASHES section per the design mock (`STASHES (2)` count in header row), no untracked files shows no UNTRACKED rows; add render assertions for both.
- [ ] 2.6 Manual smoke: `cargo run` in a scratch fixture repo (upstream + two stashes + mixed staged/unstaged/untracked files) and record the transcript/screenshot of the populated panel.
- [ ] 2.7 Run all four gates.

### [ ] 3.0 Panel focus and keyboard navigation

Add a scope/context dimension to `keymap.rs`'s `Binding` (diff scope vs. panel scope — additive), a `` ` `` focus toggle between diff and panel, a panel cursor moving through all CHANGES/UNTRACKED/STASHES entries with `j`/`k`, and Enter-on-file selecting that file in the diff view via a narrow "select file by path" call and returning focus. Focused pane gets border emphasis; stash/branch rows are navigable no-ops. Help overlay groups bindings by scope; README keymap table updated with the ratified focus-toggle key.

#### 3.0 Proof Artifact(s)

- Test: unit tests on panel navigation state (section flattening, cursor clamping across section boundaries and empty sections, Enter-on-file selection, Enter-on-stash no-op) demonstrating navigation correctness.
- Test: keymap unit tests asserting panel-scoped bindings resolve only while the panel is focused and that every pre-existing diff-scope binding resolves unchanged when unfocused, demonstrating the untouched-review-loop guarantee.
- Manual smoke transcript: focus toggle → traverse all three sections → Enter on a file jumps the diff and returns focus → unfocused keybinds behave exactly as before, demonstrating the focus model end to end.
- CLI: `?` overlay text showing the new bindings grouped by scope, plus the README.md keymap-table diff, demonstrating no hidden features and ratified keys.

#### 3.0 Tasks

- [ ] 3.1 Ratify the focus-toggle key in README.md's keymap table first: confirm `` ` `` conflicts with nothing in the existing map, add a "Git panel" section to the table documenting `` ` `` (focus toggle), `j`/`k` (move cursor), and Enter (open file in diff).
- [ ] 3.2 Add a scope dimension to `Binding` in `src/ui/keymap.rs` (e.g. `Scope::Diff` / `Scope::Panel`), defaulting every existing binding to diff scope; write keymap unit tests proving resolution respects scope and that all pre-existing bindings resolve unchanged in diff scope.
- [ ] 3.3 Add panel focus state to `App` (following the existing mode-based handling pattern in `modes.rs`), a `FocusGitPanel` toggle action bound to `` ` `` in both scopes, and focused-pane border emphasis consistent with existing overlay styling; add a `TestBackend` assertion for the emphasized border on each side of the toggle.
- [ ] 3.4 Write failing unit tests for the panel cursor model in `src/ui/git_panel.rs`: flattening the three sections into navigable rows, `j`/`k` clamping at both ends and across section boundaries, skipping section-header rows, and behavior with an empty section.
- [ ] 3.5 Implement the cursor model and panel-focused key handling (in `modes.rs`, following the existing modal-handler pattern), rendering the cursor row with the `ListState`/`REVERSED` pattern used by `staging_panel.rs`/`list_panel.rs`; tests from 3.4 pass.
- [ ] 3.6 Implement Enter-on-file: a narrow `select_file_by_path(&mut self, path)` on `App` that selects the file in the diff view and returns focus to the diff; Enter on stash/branch/header rows is a no-op; unit tests for both.
- [ ] 3.7 Update the `?` help overlay (`src/ui/help.rs`) to group bindings by scope, including the panel-scope section; verify every new action appears.
- [ ] 3.8 Manual smoke transcript: toggle focus, traverse all three sections, Enter on a file jumps the diff and returns focus, then confirm a sample of pre-existing bindings (`j`/`k` scroll, `space` stage, `s` staging panel, `gd`) behave exactly as before when the panel is unfocused.
- [ ] 3.9 Run all four gates.

### [ ] 4.0 Async remote operations (fetch/pull/push) and command log pane

Wire `BackgroundTasks::poll` into the event-loop tick; add panel-scoped `f`/`p`/`P` running plain `git fetch`/`pull`/`push` (fixed argv, no shell, never `--force`, `GIT_TERMINAL_PROMPT=0`) on the background thread; running indicator; at most one in-flight op (further requests rejected with a message); on completion refresh changes list, branch header, and ahead/behind. Every remote op (command line, exit status, stdout/stderr) appends to a bounded (~50-entry) in-memory command log rendered in the existing bottom-panel slot, toggled with `@`. Pull conflicts surface via existing unmerged-status parsing. Help overlay and README keymap updated.

#### 4.0 Proof Artifact(s)

- Test: integration test using a tempdir repo with a `file://` bare remote exercising fetch and push, a pull that creates ahead/behind movement, and a pull producing a merge conflict whose files surface as unmerged entries, demonstrating real remote ops against real git.
- Test: unit tests on the command-log model (append, bounded eviction at capacity, newest-last ordering) and on the single-in-flight guard (second request rejected with a message), demonstrating operation-safety logic.
- Manual smoke transcript: trigger a slow fetch and continue scrolling the diff during it with no dropped frames, demonstrating the render loop never blocks.
- Manual: command log pane showing a failed push (rejected non-fast-forward) with its stderr and nonzero exit status, demonstrating failure transparency without a crash.
- CLI: `cargo build && cargo test && cargo clippy -- -D warnings && cargo fmt --check` all green, demonstrating the four quality gates at completion.

#### 4.0 Tasks

- [ ] 4.1 Ratify the remaining keys in README.md's keymap table: panel-scoped `f` (fetch), `p` (pull), `P` (push) and the command-log toggle `@`; resolve any conflict in README before wiring.
- [ ] 4.2 Write failing unit tests, then implement remote command construction in `src/git/remote.rs`: fixed argv for plain `git fetch`/`git pull`/`git push` with `GIT_TERMINAL_PROMPT=0` in the child environment; assert no `--force` and no shell invocation is possible by construction.
- [ ] 4.3 Write failing unit tests, then implement the command-log model in `src/ui/command_log.rs`: entry = command line, exit status, stdout, stderr; bounded at 50 entries with oldest-first eviction; newest-last ordering; no persistence.
- [ ] 4.4 Wire `BackgroundTasks::poll` draining into the event-loop tick in `src/ui/mod.rs` (alongside the existing `code_intel::poll`), remove `#![allow(dead_code)]` from `src/ui/background.rs`, and add a `BackgroundTasks` field plus a single-in-flight-op guard to `App`; unit test: a second remote request while one is in flight is rejected with a status message.
- [ ] 4.5 Bind panel-scoped `f`/`p`/`P` actions that spawn the remote command via `BackgroundTasks` and `run_command`, and render a running indicator (spinner or `fetching…` in the panel header/footer) while in flight.
- [ ] 4.6 On completion: append the command-log entry, re-run `App::refresh` plus the branch/stash reads, and show a success/failure summary in the footer; unit test with `FakeGit` asserting the refresh path runs and staged markers/annotations survive.
- [ ] 4.7 Render the command log in the existing bottom-panel slot (same 60/40 split as the staging panel), toggled with `@` from both scopes, newest-last; `TestBackend` render test for entries including a nonzero-exit entry with stderr.
- [ ] 4.8 Add `tests/git_remote_integration.rs` using the tempdir helpers: create a `file://` bare remote; exercise fetch (after remote-side movement, assert behind count), push (assert remote ref advanced and ahead count cleared), a pull that fast-forwards, and a pull that produces a merge conflict — asserting conflicted files appear as unmerged entries in the parsed status.
- [ ] 4.9 Update the `?` help overlay with the remote-op and command-log bindings in their scope groups.
- [ ] 4.10 Manual smoke: trigger a slow fetch (large or throttled remote) and scroll the diff during it (no freeze); force a non-fast-forward push rejection and capture the command-log pane showing its stderr and exit status.
- [ ] 4.11 Run all four gates and record the final test-count delta.
