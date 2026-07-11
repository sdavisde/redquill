# Task 03 Proofs - Panel focus and keyboard navigation

## Task Summary

This task made the git panel focusable and keyboard-navigable: the keymap gained an additive scope dimension (diff vs. panel), `` ` `` toggles focus between the diff and the panel (ratified in README first — no conflicts), a cursor traverses all CHANGES/UNTRACKED/STASHES entries with `j`/`k`, and Enter on a file entry jumps the diff to that file and returns focus. The focused pane is marked by an emphasized border.

## What This Task Proves

- Panel-scoped bindings resolve only while the panel is focused; every pre-existing diff-scope binding resolves byte-for-byte identically when unfocused — the existing review loop is untouched.
- The cursor model flattens the three sections into navigable rows, clamps at both ends, crosses section boundaries, skips header rows, and handles empty sections.
- Enter on a file selects it in the diff view via the narrow `App::select_file_by_path` seam (spec 03's future reroute point) and returns focus; Enter on stash/branch rows is a deliberate no-op.
- Focus is modeled as `Mode::Panel` — mutually exclusive with the other modal panels by construction, mirroring the existing `List`/`Staging` pattern.
- All new actions appear in the `?` overlay, grouped by scope, and in README's canonical keymap table.

## Evidence Summary

- +23 tests (507 total green): 6 keymap scope tests, 6 cursor-model tests, 9 `App` focus/navigation tests, 2 dispatch/border tests.
- TDD honored: cursor-model and scope tests referenced not-yet-existing types and were confirmed failing (compile errors) before implementation.
- Headless smoke transcript drives the real `dispatch_key` event-loop path with real crossterm `KeyEvent`s.

## Artifact: Keymap scope-resolution tests

**What it proves:** `Scope::Diff` / `Scope::Panel` resolution is correct, `FocusGitPanel` is bound in both scopes so `` ` `` toggles both ways, and legacy `lookup`/`resolve` callers (defaulting to diff scope) behave identically to before the change.

**Why it matters:** This is the "when the panel is not focused, every current keybinding behaves exactly as today" guarantee from the spec's goals, enforced at the keymap layer.

**Command:**

~~~bash
cargo test --lib ui::keymap
~~~

**Result summary:** All scope tests pass; the old resolution API delegates to the scope-aware variants with unchanged results.

## Artifact: Panel cursor-model and navigation tests

**What it proves:** Section flattening, `j`/`k` clamping at both ends and across boundaries, header-row skipping, empty-section behavior, Enter-on-file selection with focus return, and Enter-on-stash no-op.

**Why it matters:** These are the exact navigation correctness cases the spec's Unit 2 proof artifacts call for.

**Command:**

~~~bash
cargo test --lib ui::git_panel && cargo test --lib ui::app
~~~

**Result summary:** 6 cursor-model tests (written failing-first) plus 9 `App`-level focus/navigation tests pass.

## Artifact: Headless smoke transcript through the real dispatch path

**What it proves:** The end-to-end focus model works: `` ` `` focuses the panel, `j`×4 traverses CHANGES→UNTRACKED→STASHES with bottom clamp, Enter on a stash stays put, `k`/`Enter` on a file jumps the diff (`selected_file` changes) and returns focus, then unfocused `j`, `s`, `space`, `gd` dispatch exactly as before.

**Why it matters:** tmux is unavailable on this machine, so instead of a PTY capture the transcript feeds real crossterm `KeyEvent`s through `dispatch_key` — the same function the live event loop now calls — making it a true behavioral trace, not a simulation of a copy. A permanent regression test keeps the unfocused-dispatch guarantee under `cargo test`.

**Artifact path:** `docs/specs/02-spec-git-panel/02-proofs/02-task-03-smoke.txt`

**Result summary:** 14 stepwise key→observation entries, regenerable via the `#[ignore]` transcript-capture test.

## Artifact: Keymap discoverability (`?` overlay + README)

**What it proves:** The `?` overlay groups bindings by scope with a "Git panel (focused)" section; README's canonical keymap table gained the `` ` ``/`j`/`k`/`Enter` panel rows (9 added lines, nothing else touched).

**Why it matters:** Repo guardrail — every user-visible action must be reachable from the keymap and listed in help; README owns the canonical map and was updated first (no conflict found for `` ` ``).

**Result summary:** Ratification confirmed against both the README table and `default_map()` before wiring; help renders from the keymap data itself, so the sections cannot drift.

## Artifact: Four quality gates

**Command:**

~~~bash
cargo build && cargo test && cargo clippy -- -D warnings && cargo fmt --check
~~~

**Result summary:** Verified independently by the orchestrator: build clean, 507/507 passing (1 ignored = the transcript generator, run explicitly), clippy clean under `-D warnings`, no formatting drift.

## Notes for Reviewers

- `dispatch_key`/`Flow` were extracted from the event-loop body (additive, behavior-identical) so tests exercise the real dispatch path — this was required by the smoke sub-task and doubles as a testability improvement.
- No hidden `Esc`-to-close was added: only the documented `` ` `` toggles focus, per the no-hidden-features guardrail.

## Reviewer Conclusion

The focus model is complete and safe: panel navigation works end to end through the real event path, the untouched-review-loop guarantee is enforced by tests at both the keymap and dispatch layers, and all bindings are discoverable — landed with all four gates green and a strict +23 test increase.
