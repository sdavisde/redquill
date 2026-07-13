# 02-spec-git-panel.md

## Introduction/Overview

redquill's current file sidebar is a passive list: it mirrors the diff's selected file and shows staged markers, but knows nothing about the repository beyond the changed-file list. This spec evolves it into a Zed-style **git panel**: a focusable pane showing the active branch with ahead/behind counts, changed and untracked files, and the stash list — plus non-blocking fetch/pull/push reachable from the panel, with git's output visible in a toggleable lazygit-style command log.

The goal is that a reviewer never leaves redquill to answer "what branch am I on, am I behind the remote, what's stashed?" or to sync with the remote before/after a review pass.

## Goals

- Show repository sync state (branch name, ahead/behind vs upstream, stash list) live in the panel, updating after every staging or remote operation.
- Make the panel focusable and keyboard-navigable: move a cursor through CHANGES / UNTRACKED / STASHES entries, jump the diff to any file with Enter.
- Run fetch, pull, and push from panel-scoped keybinds on a background thread — the render loop must never freeze during a network operation.
- Surface every git command redquill runs (and its output) in a toggleable command log pane, so the tool's git activity is fully transparent.
- Keep the existing review loop untouched: when the panel is not focused, every current keybinding behaves exactly as today.

## User Stories

- **As a reviewer**, I want to see my branch and how far it is ahead/behind its upstream while reviewing, so that I know whether I need to sync before acting on the diff.
- **As a reviewer**, I want to fetch, pull, or push with one keypress without leaving the tool, so that syncing doesn't break my review flow.
- **As a reviewer**, I want to navigate the changed/untracked file list with a cursor and jump the diff to any file, so that I can move through a large review in the order I choose.
- **As a git user**, I want to see my stashes listed in the panel, so that I remember work I've set aside without running `git stash list` in another terminal.
- **As a cautious user**, I want to see exactly which git commands redquill ran and what they printed, so that I trust the tool with write and network operations.

## Demoable Units of Work

### Unit 1: Repository state read models

**Purpose:** Give the `git/` module typed, tested knowledge of branch, upstream sync state, and stashes — the data backbone of the panel.

**Functional Requirements:**
- The system shall obtain the current branch name, upstream ref, and ahead/behind counts by extending the existing porcelain-v2 status call with `--branch` and parsing its `# branch.*` headers into a typed struct.
- The system shall parse `git stash list` (using an explicit `--format` for stable parsing) into a typed list of entries carrying the stash ref (e.g. `stash@{0}`), source branch, and message.
- The system shall degrade gracefully: detached HEAD shows the short commit id in place of a branch name; no upstream shows the branch with no ahead/behind counts; zero stashes yields an empty list — none of these are errors.
- The system shall keep `git/` free of TUI types, with errors via `thiserror`, following the existing `status.rs` parsing pattern.

**Proof Artifacts:**
- Test: unit tests over recorded porcelain `--branch` headers and stash `--format` output (including detached-HEAD, no-upstream, and empty cases) demonstrate correct parsing.
- Test: integration test building a tempdir repo with a local upstream and a stash, asserting parsed branch/ahead-behind/stash values, demonstrates the read models work against real git.

### Unit 2: Focusable git panel UI

**Purpose:** Replace the passive sidebar with the navigable panel a Zed user expects.

**Functional Requirements:**
- The system shall render the panel with a branch header (branch name, ahead/behind indicators such as `↑2 ↓1` when an upstream exists) and three sections: CHANGES (tracked modifications, preserving the existing staged `●` markers and change-kind letters), UNTRACKED, and STASHES (view-only entries: ref, branch, message).
- The user shall move focus between the panel and the diff view with a dedicated keybind (proposed: `` ` ``; final key ratified via the README keymap update in this spec).
- The user shall, while the panel is focused, move a cursor through all section entries with the existing motion keys (`j`/`k`), and pressing Enter on a file entry shall select that file in the diff view and return focus to it.
- The system shall visibly distinguish the focused pane (e.g. border emphasis) and the panel's cursor row, and stash/branch rows shall be cursor-navigable but carry no actions in this spec.
- The system shall list every new action in the keymap table and the `?` help overlay, and shall update README.md's keybinding map with the ratified keys as part of this unit.

**Proof Artifacts:**
- Test: unit tests on panel state (section flattening, cursor clamping across sections, Enter-on-file selection) demonstrate navigation correctness.
- Manual smoke transcript: focus toggle → navigate all three sections → Enter on a file jumps the diff → unfocused keybinds unchanged, demonstrates the focus model end to end.
- CLI: `?` overlay screenshot/text showing the new bindings demonstrates no hidden features.

### Unit 3: Async remote operations and command log

**Purpose:** Fetch/pull/push without leaving the tool or freezing it, with full transparency of what ran.

**Functional Requirements:**
- The user shall, while the panel is focused, trigger fetch (`f`), pull (`p`), and push (`P`) — these bindings are panel-scoped and shall not exist in diff-view scope.
- The system shall run remote operations as plain `git fetch` / `git pull` / `git push` (fixed argv, no shell, never `--force`) on a background thread via the background-task poller introduced by spec 01, with `GIT_TERMINAL_PROMPT=0` set so a credential prompt fails fast instead of hanging the thread.
- The system shall show a running indicator while an operation is in flight, allow at most one remote operation at a time (further requests are rejected with a message), and refresh the changes list, branch header, and ahead/behind counts when the operation completes.
- The system shall append every remote operation (command line, exit status, stdout/stderr) to a command log rendered in a toggleable bottom pane (proposed toggle: `@`, matching lazygit; final key ratified via the README update), reusing the existing bottom-panel layout slot.
- The system shall surface pull conflicts naturally: conflicted files appear via the existing unmerged-status parsing in the CHANGES section; redquill shall not attempt any conflict resolution.

**Proof Artifacts:**
- Test: integration test using a tempdir repo with a `file://` bare remote exercising fetch and push (and a pull that creates ahead/behind movement), asserting refreshed state, demonstrates real remote ops work.
- Manual smoke transcript: trigger a slow fetch and continue scrolling the diff during it, demonstrates the render loop never blocks.
- Manual: command log pane showing a failed push (rejected non-fast-forward) with its stderr, demonstrates failure transparency without a crash.

## Non-Goals (Out of Scope)

1. **Stash mutations**: no create/apply/pop/drop — the stash section is view-only, structured so actions can be added in a later spec.
2. **Commit creation, branch switching, checkout, or remote management**: the write ceiling remains the index plus the three explicit remote operations. (Branch switching was later ratified as its own feature — see spec 03, docs/specs/03-spec-branch-worktree-switcher.md. Commit creation likewise — see spec 04, docs/specs/04-spec-commit-staged.md.)
3. **Force push or any destructive git operation**: never, under any binding.
4. **Confirmation dialogs**: remote ops run immediately on keypress by design; no modal prompts.
5. **Credential handling**: redquill never reads, stores, or prompts for credentials; git's own machinery (ssh-agent, credential helpers) is the only path.
6. **Config layer / remappable keys**: keybindings ship as defaults in the keymap table; the config file work remains deferred per `docs/config-layer.md`.
7. **Multibuffer integration**: the panel targets the current diff view; scroll-to-section behavior against the multibuffer belongs to spec 03.

## Design Considerations

Panel layout (replacing the current sidebar in its existing slot and width). **Ratified update:** the panel occupies that slot only while focused (`Mode::Panel`) — it is hidden by default, showing/hiding exactly follows focus (backtick to open+focus, backtick or Enter-on-file to close+hide), and the diff pane takes the full width whenever the panel is closed. A persistent footer hint (`` ` git panel``) keeps the binding discoverable while the panel is hidden.

```
┌─ git: main ↑2↓1 ─┐
│ CHANGES          │
│ ● M session.rs   │
│   M mod.rs       │
│   A keys.rs      │
│ UNTRACKED        │
│   notes.md       │
│ STASHES (2)      │
│   0 wip: parser  │
│   1 spike: tabs  │
│ [3 files][1 ●]   │
└──────────────────┘
```

The command log pane reuses the bottom-panel slot (same split as the staging panel), showing the most recent commands newest-last. Focused-pane borders use emphasis consistent with existing overlay styling. No other visual redesign.

## Repository Standards

- All four gates pass at every commit: `cargo build`, `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check`.
- TDD for the pure parsing code (porcelain `--branch` headers, stash list): failing test first.
- Integration tests build throwaway repos in tempdirs via `std::process::Command` git calls; never against the host repo.
- No `unwrap()`/`expect()` outside tests; `thiserror` in `git/`, `anyhow` at the binary edge.
- Every user-visible action reachable from the keymap and listed in `?` help; keybinding changes proposed in README.md's map, not invented ad hoc.
- Conventional commits (`feat:` for panel/ops, `test:`, `docs:` for the README map update).

## Technical Considerations

- **Ahead/behind** comes free from `git status --porcelain=v2 --branch -z` header lines (`# branch.head`, `# branch.upstream`, `# branch.ab +N -M`) — one process call already being made, extended rather than added.
- **Sequencing**: Unit 3 depends on spec 01's background-task poller; Units 1–2 have no dependency on spec 01 beyond merge hygiene. Implement this spec after spec 01 merges to avoid rebasing panel state onto the decomposed `App`.
- **Keymap scope**: panel-scoped bindings require the keymap's `Binding` to gain a scope/context dimension (diff vs panel focus). This is an additive change to `src/ui/keymap.rs` and the dispatch path; the help overlay should group bindings by scope.
- **Refresh**: after any remote op completes, re-run the existing snapshot/refresh path (`App::refresh`) plus the new branch/stash reads; staged markers and annotations must survive refresh exactly as they do today.
- **Command log** stores a bounded in-memory history (e.g. last 50 entries) — no persistence.
- The interface between panel selection and the diff view should be a narrow "select file by path" call, so spec 03 can later reroute it to "scroll multibuffer to section" without reworking the panel.

## Security Considerations

- Remote operations inherit the user's git authentication (ssh-agent, credential helpers); redquill never handles secrets. `GIT_TERMINAL_PROMPT=0` prevents interactive credential prompts from hanging background threads — such operations fail visibly in the command log instead.
- Commands are executed with fixed argument vectors (no shell interpolation of any user-controlled string).
- Never `--force`, never rewriting remote state beyond a plain fast-forward-checked `git push`.
- The command log renders git's own output only; git does not echo credentials, and redquill must not log environment contents.

## Success Metrics

1. **State visibility**: branch, ahead/behind, and stash list are correct and update within one refresh after staging or remote ops (verified by the integration tests).
2. **Responsiveness**: scrolling and hunk navigation remain instant (per the repo's 5k-line-diff target) while a fetch/pull/push is in flight — zero dropped-frame freezes.
3. **Discoverability**: 100% of new actions appear in the `?` overlay and the README keymap table.
4. **Quality gates**: all four cargo gates green; test count strictly increases.

## Open Questions

1. Final key glyphs (`` ` `` for focus toggle, `@` for command log) are proposals; ratified in the README keymap update task within Unit 2/3 — any conflict discovered there is resolved in README first.
2. Panel width stays at the current fixed 32 columns; making it configurable is deferred to the config-layer spec.
3. Enter on a stash entry is reserved (no-op) for the future stash-actions spec.
