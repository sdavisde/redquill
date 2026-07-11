# Task 04 Proofs - Async remote operations (fetch/pull/push) and command log pane

## Task Summary

This task completed spec 02: panel-scoped `f`/`p`/`P` run plain `git fetch`/`pull`/`push` on a background thread (the first production use of spec 01's `BackgroundTasks` poller), with a running indicator, a single-in-flight guard, and an automatic state refresh on completion. Every remote operation — command line, exit status, stdout, stderr — lands in a bounded 50-entry command log rendered in the bottom-panel slot and toggled with `@` from either scope.

*Provenance note: the implementation sub-agent stalled before delivering its final report (twice); the orchestrator ran the gates, inspected the implementation, and assembled this evidence directly from the working tree — every claim below was verified first-hand, not relayed.*

## What This Task Proves

- Remote operations run without freezing the render loop: a 2-second operation was in flight while keypresses processed in single-digit milliseconds through the real dispatch path.
- Security invariants hold by construction: `RemoteOp` is a closed, payload-free enum whose argv is a hard-coded `&'static [&'static str]` — `--force` (or any flag) is unrepresentable; no shell is involved; `GIT_TERMINAL_PROMPT=0` is set on every child.
- At most one remote operation runs at a time; a second request is rejected with a message and does not spawn.
- Failures are transparent, not fatal: a real non-fast-forward push rejection appears in the command log with `exit 1` and git's verbatim stderr.
- Real remote semantics work against a `file://` bare remote: fetch reveals a behind count, push advances the remote ref and clears the ahead count, fast-forward pull integrates, and a divergent pull surfaces conflicted files as unmerged entries with no conflict-resolution attempted.

## Evidence Summary

- +25 tests (532 total green: 500 lib + 14 + 4 remote-integration + 10 + 4): remote-argv construction tests, command-log model tests (append/eviction/ordering), in-flight-guard test, refresh-on-completion test, log-pane render test, and 4 new integration tests in `tests/git_remote_integration.rs`.
- All four gates verified independently by the orchestrator: build clean, 532/532 passing (2 ignored = the two transcript-generator tests, run explicitly), clippy clean under `-D warnings`, fmt clean.
- README ratified `f`/`p`/`P` (panel-scoped) and `@` (both scopes) with no conflicts; `?` overlay updated in the scope groups.

## Artifact: Security-by-construction tests in `src/git/remote.rs`

**What it proves:** Each variant's argv is exactly one hard-coded subcommand (`["fetch"]`/`["pull"]`/`["push"]`); tests assert the exact argv, the working directory, the `GIT_TERMINAL_PROMPT=0` env entry, and that no variant can carry `--force`, `-f`, or any flag.

**Why it matters:** The spec's security considerations (fixed argv, no shell, never force, credential prompts fail fast) are the trust foundation for giving a review tool network write access.

**Command:**

~~~bash
cargo test --lib git::remote
~~~

**Result summary:** 6 tests pass, including `no_variant_can_carry_force_or_any_extra_flag` (asserts argv length is exactly 1) and `remote_command_disables_the_terminal_prompt`.

## Artifact: Non-blocking render loop (smoke transcript, part a)

**What it proves:** While a 2000ms operation ran on the real `BackgroundTasks` poller, four `j`/`k` keypresses drove the diff cursor through `dispatch_key` — the exact event-loop handler — all completing at t=+0ms with the operation still pending; the log drained at t=+2041ms.

**Why it matters:** This is the spec's "the render loop must never freeze during a network operation" goal and success metric #2, observed with timestamps on the real code path (tmux unavailable, so headless capture through the production dispatch function).

**Artifact path:** `docs/specs/02-spec-git-panel/02-proofs/02-task-04-smoke.txt`

**Result summary:** All keypresses processed in single-digit milliseconds while the op was in flight; regenerable via the `#[ignore]` transcript test in `src/ui/mod.rs`.

## Artifact: Failure transparency (smoke transcript, part b)

**What it proves:** A push rejected non-fast-forward by a `file://` bare remote (advanced by a second clone) flowed through the real spawn→poll→log pipeline and rendered in the command-log pane: `command_line="git push"`, `exit 1`, and git's verbatim rejection stderr — with no crash.

**Why it matters:** The spec's transparency goal: a cautious user sees exactly which command ran and what git printed, especially on failure.

**Artifact path:** `docs/specs/02-spec-git-panel/02-proofs/02-task-04-smoke.txt`

**Result summary:** The pane was visible and showed the rejection text; the tool continued running.

## Artifact: Remote semantics against a real `file://` bare remote

**What it proves:** `tests/git_remote_integration.rs` covers `fetch_after_remote_movement_reveals_a_behind_count`, `push_advances_the_remote_ref_and_clears_the_ahead_count`, `fast_forward_pull_integrates_the_remote_commit`, and `pull_with_divergent_edits_surfaces_conflicted_files_as_unmerged`.

**Why it matters:** These prove refreshed branch/ahead-behind state after each operation and that pull conflicts surface through the existing unmerged-status parsing (redquill attempts no resolution), per the spec's FRs.

**Command:**

~~~bash
cargo test --test git_remote_integration
~~~

**Result summary:** 4/4 pass in throwaway tempdir repos; the host repo is never touched.

## Artifact: Command-log model and single-in-flight guard

**What it proves:** The log is bounded at 50 entries with oldest-first eviction and newest-last ordering (`evicts_oldest_at_capacity`, `eviction_holds_the_cap_across_many_pushes`); a second remote request while one is in flight is rejected with a status message and does not spawn (`second_remote_request_while_one_in_flight_is_rejected_and_does_not_spawn`); completion appends the entry and re-runs the refresh path with staged markers and annotations surviving.

**Why it matters:** These are the operation-safety FRs — bounded memory, no concurrent remote ops, and state that stays fresh after every operation.

**Result summary:** All model, guard, and refresh tests pass; `#![allow(dead_code)]` was removed from `src/ui/background.rs` as it gained its first production caller.

## Artifact: Discoverability (README + `?` overlay)

**What it proves:** README's canonical keymap gained panel-scoped `f`/`p`/`P` rows and `@` in both the main table and the panel section (7 added lines, ratified before wiring, no conflicts); help renders the new actions in their scope groups.

**Result summary:** 100% of new actions are documented — the spec's discoverability success metric.

## Artifact: Four quality gates

**Command:**

~~~bash
cargo build && cargo test && cargo clippy -- -D warnings && cargo fmt --check
~~~

**Result summary:** Verified independently by the orchestrator: build clean, 532/532 tests green (2 deliberately-ignored transcript generators), clippy clean, no formatting drift. Final spec-wide test growth: 455 → 532 (+77).

## Reviewer Conclusion

Spec 02 is functionally complete: remote operations are async, singular, transparent, and safe by construction; the command log makes every remote command auditable; and real-git integration tests plus timestamped non-blocking evidence back every functional requirement — landed with all four gates green.
