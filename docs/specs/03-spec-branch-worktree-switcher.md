# 03-spec-branch-worktree-switcher.md

## Introduction/Overview

Spec 02 shipped the git panel with a passive but navigable branch/changes/stash view, plus (in `142e996`) auto-follow: the diff view now tracks the panel cursor as it moves over file rows. This spec ratifies the next step the project owner has since designed interactively: a branch/worktree switcher modal, opened from the git panel, that lets a reviewer change branch or re-root the whole app onto a different worktree without leaving redquill.

The git-layer capability for this already exists — `9c98d97` added local-branch and worktree read models plus a `git switch` runner on the `worktree-git-switcher` branch, staged ahead of UI wiring. This spec is that ratification: it supersedes spec 02's non-goal on branch switching and gives the scaffolding a home.

## Goals

- Let a reviewer switch local branches from within redquill via a centered modal opened from the git panel.
- Let a reviewer re-root the app onto a different worktree from the same modal, without restarting the process.
- Preserve annotations across both a branch switch and a worktree re-root.
- Keep failures visible (command log) and non-fatal (modal-stays/closes semantics, never a crash).

## User Stories

- **As a reviewer**, I want to switch branches from the git panel with a couple of keystrokes, so that I don't have to leave redquill to `git switch` and restart.
- **As a reviewer working across worktrees**, I want to jump the whole review session to another worktree, so that I can review multiple in-flight branches without spawning separate processes.
- **As a cautious user**, I want a failed switch (dirty tree, already checked out elsewhere) to show up in the command log rather than silently do nothing or crash the app.

## Demoable Units of Work

### Unit 1: Switcher modal shell

**Functional Requirements:**
- The user shall, while the git panel is focused, press `b` to open a centered modal with two tabs, **Branches** (default) and **Worktrees**.
- The user shall switch tabs with `Tab`/`h`/`l`/arrow keys, move the cursor with `j`/`k`, and act on the selected row with `Enter`.
- The user shall close the modal with `Esc`, returning focus to the git panel at the same cursor row it had before the modal opened.
- The system shall treat `q` as inert while the modal is open, per the existing overlay rule (no accidental quit-through-modal).

### Unit 2: Branches tab

**Functional Requirements:**
- The system shall list local branches, marking the current branch and annotating any branch checked out in another worktree.
- The user shall, on `Enter`, trigger `git switch -- <name>` followed by a full review rebuild (diff, panel, annotation targets).
- The system shall record switch failures (dirty working tree, branch checked out elsewhere) in the command log pane and point the footer at `@` to view it; the modal stays open on failure and closes on success.
- The system shall never crash on a failed switch, and shall block branch-switch attempts while a remote operation (fetch/pull/push) is in flight.

### Unit 3: Worktrees tab

**Functional Requirements:**
- The system shall list entries parsed from `git worktree list --porcelain`, with badges for branch, detached, bare, locked, and prunable states, and a marker for the current worktree.
- The user shall, on `Enter`, re-root the app onto the selected worktree: construct a new `GitRunner` rooted there, swap it into the backend, lazily re-create any LSP state, and rebuild the review snapshot build-first-then-swap so a failed rebuild never leaves the app half-switched.
- The system shall refuse to act on bare or already-current worktree entries, showing a footer message instead of attempting the switch.

### Unit 4: Annotation continuity

**Functional Requirements:**
- The system shall preserve existing annotations across both a branch switch and a worktree re-root, re-targeting them against the rebuilt review state rather than discarding them.

## Non-Goals (Out of Scope)

1. **Creating or deleting branches or worktrees**: this spec only switches between existing ones.
2. **Force switching**: no `--force` or discard-changes variant of `git switch`.
3. **Remote branch checkout**: the Branches tab lists local branches only.
4. **Anything else spec 02 excluded**: stash mutations, commit creation, and additional remote management remain out of scope here too. (Commit creation was later ratified as its own feature — see spec 04, docs/specs/04-spec-commit-staged.md.)

## Write Ceiling

This spec extends the product's write ceiling (CLAUDE.md) to include `git switch`, run with a fixed argument vector — no shell interpolation, no `--force` variant, `--` always separating flags from the branch name. It supersedes spec 02 Non-Goal #2 ("commit creation, branch switching, checkout, or remote management") specifically for branch switching; the rest of that non-goal (commit creation, arbitrary checkout, remote management beyond fetch/pull/push) still stands. (Commit creation was later ratified separately — see spec 04, docs/specs/04-spec-commit-staged.md.)

## Technical Considerations

- Git-layer read models (`src/git/branch.rs`, `src/git/worktree.rs`) and the `git switch` runner already exist from `9c98d97` on `worktree-git-switcher`; this spec's job is UI wiring on top of that layer, not new git-layer work.
- Worktree re-root must build the new snapshot before swapping state in, so a failed rebuild leaves the current worktree's session intact.
- Reuses the existing command log pane and footer-hint pattern from spec 02's remote-ops unit for failure reporting.

## Success Metrics

1. Branch switch and worktree re-root both complete with a rebuilt, correct review state and preserved annotations.
2. Every failure path lands in the command log with no crash.
3. All four cargo gates green; test count strictly increases.
