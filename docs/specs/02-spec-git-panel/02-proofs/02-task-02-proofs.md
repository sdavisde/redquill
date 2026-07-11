# Task 02 Proofs - Git panel rendering: branch header and sectioned display

## Task Summary

This task replaced the passive file sidebar with the git panel's visual form in the same 32-column slot: a branch header with ahead/behind glyphs (`git: main ↑2↓1`), CHANGES / UNTRACKED / STASHES sections, preserved staged `●` markers and change-kind letters, and the existing footer counts. The panel is still passive — focus and navigation arrive in task 3.0.

## What This Task Proves

- The panel renders the branch header correctly in all state variants: upstream with counts (`↑2↓1`), detached HEAD (short oid), no upstream (no arrows), and zero ahead/behind.
- The three sections render per the spec mock, with empty sections hidden and the STASHES header carrying its count (`STASHES (2)`).
- The existing sidebar behaviors survive intact: staged `●` markers, change-kind letter colors, dir/basename path split, and the byte-identical footer (`[N files] [N staged] [N notes]`).
- `App::refresh` now repopulates branch/stash state alongside the snapshot, with staged markers and annotations preserved exactly as before.
- Live end-to-end rendering works against real git output from a fixture repository.

## Evidence Summary

- 10 new `TestBackend` render tests in `src/ui/git_panel.rs` (9 confirmed failing against the pre-panel rendering before implementation) plus 1 new `App::refresh` unit test.
- Full suite: 484 tests green (456 lib + 14 + 10 + 4), a strict +11 over the 473 baseline.
- All four gates pass; smoke frame captured from real git data at `02-task-02-smoke.txt`.

## Artifact: TestBackend render tests for every header/section variant

**What it proves:** Header variants (upstream+counts, detached, no-upstream, zero counts), section headers, `●` markers, change-kind letters, stash rows, footer, and both empty-section cases all render as the spec's design mock requires.

**Why it matters:** These lock the panel's visual contract so tasks 3.0/4.0 (cursor, focus borders, running indicator) cannot silently regress it.

**Command:**

~~~bash
cargo test --lib ui::git_panel
~~~

**Result summary:** All 10 render tests pass; TDD honored — 9 of 10 confirmed failing against the pre-panel rendering first (the 10th is an absence assertion that passes trivially by construction).

## Artifact: App::refresh repopulates branch/stash state

**What it proves:** After a refresh, `App` re-reads branch status and the stash list through the extended `StageOps` seam (`branch_status()`/`stash_list()`, faked by `FakeGit` in tests) while staged markers and annotations survive unchanged.

**Why it matters:** The spec requires panel state to update after every staging or remote operation via the existing refresh path; this test is what task 4.0's post-remote-op refresh relies on.

**Command:**

~~~bash
cargo test --lib ui::app
~~~

**Result summary:** The new refresh test passes alongside all pre-existing `App` tests; the `StageOps` additions use default bodies so unrelated test doubles needed no changes.

## Artifact: Live smoke frame from real git data

**What it proves:** The full panel — header `git: main ↑2↓1`, all three sections, `●` marker, view-only stash rows, footer — renders correctly from live `git status --porcelain=v2 --branch` and `git stash list` output in a fixture repo (bare `file://` upstream arranged ahead-2/behind-1, two stashes, mixed staged/unstaged/untracked files).

**Why it matters:** Render tests use canned structs; this proves the whole pipeline (GitRunner → parsers → App state → panel widget) against a real repository.

**Artifact path:** `docs/specs/02-spec-git-panel/02-proofs/02-task-02-smoke.txt`

**Result summary:** Captured via `TestBackend` full-frame snapshot driven by a real `GitRunner` against the fixture repo (tmux was unavailable on this machine for a PTY capture, so the frame is headless but the git data is real). Excerpt:

~~~
┌git: main ↑2↓1────────────────┐
│CHANGES                       │
│● M session.rs                │
│UNTRACKED                     │
│  notes.md                    │
│STASHES (2)                   │
│  0 spike: tabs               │
│  1 wip: parser               │
└──────────────────────────────┘
 [2 files] [2 staged]
~~~

## Artifact: Four quality gates

**What it proves:** The repository's mandatory quality bar holds at this commit.

**Command:**

~~~bash
cargo build && cargo test && cargo clippy -- -D warnings && cargo fmt --check
~~~

**Result summary:** Build clean, 484/484 tests pass, clippy clean under `-D warnings`, no formatting drift. Verified independently by the orchestrator after the implementation agent's own run.

## Notes for Reviewers

- `sidebar.rs` was removed outright; its helpers moved wholesale into `git_panel.rs` (no shim, no duplicate code).
- Empty sections are hidden entirely (matching the mock, which only shows populated sections); this choice is covered by render tests.
- A staged-only file (index add with no unstaged hunks) appears in the `[N staged]` footer count but not as a CHANGES row — this is the pre-existing sidebar behavior, deliberately preserved; the changes list derives from the working-tree diff.

## Reviewer Conclusion

The git panel's visual layer is complete and contract-locked by render tests, integrated with `App::refresh` through the `StageOps` seam, proven against real git data, and landed with all four gates green and a strict +11 test increase.
