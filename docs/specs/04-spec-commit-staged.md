# 04-spec-commit-staged.md

## Introduction/Overview

Spec 02 shipped staging and the git panel; spec 03 shipped branch/worktree switching. The remaining gap in the review loop is the last step: a reviewer who has staged the hunks they accept still has to quit redquill (or switch to another terminal) to run `git commit`. This spec ratifies lazygit-style commit creation from the git panel: press `c` while the panel is focused, type a message in an inline modal, and the commit runs in the background like the existing remote operations.

Scope is deliberately minimal: plain `git commit -m <message>` of whatever is already staged. No amend, no flags, no editor round-trip.

## Goals

- Let a reviewer commit staged changes from the git panel without leaving redquill, with a lazygit-style inline message input.
- Keep the render loop non-blocking: the commit runs on the existing background poller with the same single-flight discipline as fetch/pull/push.
- Keep failures visible (command log) and non-fatal (hook rejections, signing failures, and races surface as messages, never crashes).
- Leave the history-rewriting guardrail fully intact: plain commit only.

## User Stories

- **As a reviewer**, I want to stage the hunks I accept and commit them with a couple of keystrokes, so that the accept-half of a review doesn't require leaving the tool.
- **As a lazygit user**, I want `c` in the git panel to open a message input where Enter commits and Esc cancels, so the muscle memory transfers.
- **As a cautious user**, I want a failed commit (rejected hook, gpg failure) to land in the command log with its stderr rather than silently doing nothing or crashing.

## Demoable Units of Work

### Unit 1: Commit message modal

**Functional Requirements:**
- The user shall, while the git panel is focused, press `c` to open a centered commit-message modal — but only when at least one change is staged; with nothing staged, the system shall show a footer message ("nothing staged to commit") and not open the modal.
- The modal shall reuse the Compose text-entry behavior (`TextBuffer`): printable characters insert, arrows move the cursor, `Backspace` deletes, `Ctrl-j` inserts a newline (first line is the summary; subsequent lines are the body).
- The user shall submit with `Enter` and cancel with `Esc`, returning focus to the git panel at the cursor row it had before the modal opened.
- The system shall reject `Enter` on an empty or whitespace-only message with a footer message; the modal stays open.
- The system shall treat `q` as inert while the modal is open, per the existing overlay rule.
- The modal's keys shall be documented via a hint-only table (like `COMPOSE_HINTS`) with the bidirectional drift tests that pattern requires, and the `c` binding shall appear in the keymap table, the `?` help overlay, the panel footer hints, and the README git-panel map.

### Unit 2: Async commit execution

**Functional Requirements:**
- On submit, the system shall run `git commit -m <message>` with a fixed argument vector — the message passed verbatim (newlines included) as a single argv element, no shell interpolation, `GIT_TERMINAL_PROMPT=0` — on the existing background task poller, via a new write method on the git runner behind the operations trait seam.
- The system shall enforce single-flight across all mutating background git operations: a commit shall be rejected with a footer message while a remote op (or another commit) is in flight, and remote ops and branch switches shall likewise be blocked while a commit is in flight.
- The system shall show a running indicator while the commit is in flight, and on completion append the full command line, exit status, stdout, and stderr to the command log.
- On completion the system shall re-run the full review refresh, so committed files leave CHANGES, the last-commit line updates, and the ahead/behind counts update.
- On failure the footer shall read "commit failed — see command log (@)"; hooks run normally (never `--no-verify`) and the user's git config (signing, sign-off) is respected because the commit is the user's own `git` binary.

## Non-Goals (Out of Scope)

1. **Amend**: `commit --amend` rewrites history; the "no history-rewriting operations" guardrail stands untouched. If amend ever earns an exception it gets its own spec.
2. **Flag toggles**: no `--no-verify`, `--allow-empty`, `--signoff`, or any other flag from the UI; the argv is closed at `commit -m <message>`.
3. **Editor-based message entry**: no suspending to `$EDITOR`; consequently `commit.template` does not apply (that is inherent to `-m`). A later spec may add an editor variant.
4. **Staging from within the modal**: the modal commits what is already staged; staging stays where it lives today.
5. **Anything else specs 02/03 excluded**: stash mutations, arbitrary checkout, remote management beyond fetch/pull/push, force/destructive operations — all still out.

## Write Ceiling

This spec extends the product's write ceiling (CLAUDE.md) to include plain `git commit`, run with a fixed argument vector — `commit -m <message>`, the message passed verbatim as a single argv element, no shell interpolation, and never `--amend`, `--no-verify`, `--allow-empty`, or any flag beyond `-m`. It supersedes spec 02 Non-Goal #2 ("commit creation, branch switching, checkout, or remote management") specifically for commit creation; the rest of that non-goal (arbitrary checkout, remote management beyond fetch/pull/push) still stands, as does the ban on history-rewriting operations.

It also deliberately departs from spec 02 Non-Goal #4 ("no modal prompts") for this one operation: a commit inherently requires a message, so the modal is data entry, not a confirmation dialog. Remote ops keep their immediate-on-keypress behavior; no confirmation dialogs are introduced anywhere.

The agent-side write ceiling (CLAUDE.md) is unchanged: agents working in this repo still must not run the product's remote or commit operations on the user's behalf during a task.

## Technical Considerations

- Reuse `TextBuffer` from `src/ui/compose.rs` directly — it is not annotation-specific. A new mode variant (mirroring `Mode::Compose`) with its own key handler, hint table, and render function is the expected shape; mode-local state belongs in the variant/state struct, not loose `App` fields.
- The git-layer write method follows the `switch_branch` precedent (`src/git/runner.rs`): fixed argv through `run_raw`, exposed through the `StageOps` trait so the UI is testable with fakes. `src/git/commit.rs` is currently a read-only model (`CommitSummary`); the write method is new.
- Execution mirrors the remote-op pattern in `src/ui/app.rs` (`request_remote_op` / `poll_remote`): spawn on `BackgroundTasks`, single-flight guard, `CommandLogEntry` on drain, `self.refresh()`, footer message. Generalizing the existing in-flight guard so "at most one mutating background git op" is a single invariant (rather than parallel guards that must each check the other) is the preferred shape; naming is the implementer's call.
- Perf tripwires in `src/ui/perf_tests.rs` must stay green untouched — nothing here runs on the render path.

## Success Metrics

1. Stage → `c` → message → Enter produces a commit in the repo, the panel refreshes (CHANGES empties of committed files, last-commit line updates), and the annotations/review state survive.
2. Every failure path (nothing staged, empty message, hook rejection, concurrent op) lands in the footer and/or command log with no crash.
3. All four cargo gates green; test count strictly increases.
