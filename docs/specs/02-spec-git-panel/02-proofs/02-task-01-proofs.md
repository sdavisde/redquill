# Task 01 Proofs - Repository state read models (branch, upstream, ahead/behind, stashes)

## Task Summary

This task gives the `git/` module typed, tested knowledge of repository sync state: the porcelain-v2 status call now runs with `--branch` and its `# branch.*` headers parse into a `BranchStatus`; `git stash list` (with an explicit NUL-separated `--format`) parses into typed `StashEntry` values. This is the data backbone the git panel renders in task 2.0.

## What This Task Proves

- Porcelain `--branch` headers parse correctly for a normal branch with upstream and ahead/behind counts, a detached HEAD (short oid, no upstream), and a branch with no upstream — none of the degraded cases are errors.
- Stash lists parse correctly for empty output, multiple entries, and messages containing spaces/colons; the detached-HEAD `(no branch)` case is represented as `branch: None`.
- The read models work against real git: an integration test builds a tempdir repo with a local bare upstream arranged ahead-2/behind-1 plus one stash and asserts every parsed field.
- The pre-existing status parsing is unaffected: `parse_porcelain_v2` skips header fields, `GitRunner::status()` keeps its exact signature, and all 455 baseline tests still pass.

## Evidence Summary

- TDD was followed for both parsers: test modules were written first and confirmed failing (compile errors / parse panic) before implementation.
- Full suite: 473 tests green (445 lib + 14 git integration + 10 stage integration + 4 LSP), a strict +18 over the 455 baseline.
- All four repository gates pass: build, test, `clippy -- -D warnings`, `fmt --check`.

## Artifact: Unit test coverage of every degradation case

**What it proves:** The branch-header parser (6 tests in `src/git/branch.rs`) covers normal/detached/no-upstream/ahead-behind fixtures, and the stash parser (7 tests in `src/git/stash.rs`) covers empty/multi-entry/separator-in-message fixtures — the spec's "degrade gracefully" requirement in full.

**Why it matters:** These are the exact cases where a naive parser turns an ordinary repository state (fresh branch, detached HEAD, no stashes) into a spurious error.

**Command:**

~~~bash
cargo test
~~~

**Result summary:** Lib suite grew from 430 to 445 (+15: 6 branch, 7 stash, 2 status-header tests), all passing.

~~~
running 445 tests
test result: ok. 445 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.43s
~~~

## Artifact: Integration tests against real git

**What it proves:** `tests/git_integration.rs` gained 3 tests exercising the read models end-to-end in throwaway tempdir repos: `branch_ahead_and_behind_with_upstream_and_a_stash` (bare "remote", `-u` push, divergence arranged via a second clone, asserts `main` / `origin/main` / `(2,1)` / stash ref+branch+message), `detached_head_branch_status_uses_short_oid`, and `branch_with_no_upstream_has_no_ahead_behind`.

**Why it matters:** Fixture-string unit tests cannot catch drift between recorded git output and what the installed git actually emits; these tests parse real `git status --porcelain=v2 --branch -z` and real `git stash list` output.

**Command:**

~~~bash
cargo test --test git_integration
~~~

**Result summary:** 14 integration tests pass (11 baseline + 3 new), never touching the host repository.

~~~
running 14 tests
test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.39s
~~~

## Artifact: Four quality gates

**What it proves:** The repository's mandatory quality bar holds at this commit.

**Why it matters:** CLAUDE.md requires all four gates green before any task is considered done; clippy `-D warnings` also enforces the no-`unwrap()` discipline indirectly flagged in review.

**Command:**

~~~bash
cargo build && cargo test && cargo clippy -- -D warnings && cargo fmt --check
~~~

**Result summary:** Build finished with no warnings, 473/473 tests pass, clippy is clean under `-D warnings`, and `fmt --check` produces no diff.

~~~
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.15s
test result: ok. 445 passed; 0 failed; ...
test result: ok. 14 passed; 0 failed; ...
test result: ok. 10 passed; 0 failed; ...
test result: ok. 4 passed; 0 failed; ...
clippy: Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.10s
fmt --check: (no output, exit 0)
~~~

## Artifact: Public API surface added to `git/`

**What it proves:** The new capability is exposed through typed, TUI-free structs with `thiserror` errors, following the existing `status.rs` pattern.

**Why it matters:** Task 2.0 consumes exactly this API through the `StageOps` seam; the spec requires `git/` to stay free of TUI types.

**Result summary:** `BranchStatus`, `StashEntry`, `StatusSnapshot`, `parse_branch_headers`, `parse_stash_list`, `parse_porcelain_v2_full`, `GitRunner::status_with_branch()`, `GitRunner::stash_list()`; `GitRunner::status()` signature unchanged for existing callers.

## Reviewer Conclusion

The read models are implemented TDD-first, cover every graceful-degradation case the spec names, are proven against real git in tempdir repos, and land with all four quality gates green and a strict +18 test-count increase — task 1.0's proof obligations are fully met.
