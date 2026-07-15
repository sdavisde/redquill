# 05-spec-diff-sources.md

## Introduction/Overview

redquill works well when the repo is actively being edited, but it is not yet a functional viewer for diffs that are not the working tree: a single historical commit cannot be reviewed at all, and features like staging and LSP code-intel behave as if every diff were the working tree. This spec abstracts the viewer over a "diff source": it introduces a single-commit diff target, replaces the scattered per-target checks with a named capability model, adds a Zed-style History tab to the git panel as the in-app entry point for commit review, and makes annotation output self-describing when the reviewed source is not the working tree.

The design follows how modern Zed structures the same feature (verified against Zed's `main` branch, mid-2026): one source-agnostic diff surface, a small source type in the domain layer, capabilities that are structurally absent rather than best-effort-wrong, and a git panel with a Changes ⇄ History tab pair where Enter on a commit opens it in the same multibuffer used for uncommitted changes.

## Goals

- A user can open any commit from the git panel and review it in the same multibuffer viewer (collapsible file sections, hunk navigation, annotations) used for working-tree diffs.
- "What can I do with this diff?" (stage, auto-refresh, code-intel) is answered by named capability methods on the diff target — one place to consult, exhaustively handled — instead of five independent `matches!` checks.
- LSP code-intel is cleanly unavailable when the displayed text is not the live working tree, fixing the existing silent wrongness on `--staged` and range views.
- Annotation stdout output remains byte-identical for working-tree-only sessions, and gains an additive `Reviewing:` metadata line for annotations made against non-working-tree sources.
- All existing behavior (working tree, `--staged`, `A..B`/`A...B` ranges) is preserved unchanged.

## User Stories

- **As a developer reviewing agentic changes**, I want to open a recent commit from the panel and read its diff with the same navigation I already use, so that I can review work that was committed before I got to it.
- **As a developer investigating a regression**, I want to browse the branch's commit history inside redquill and inspect what any single commit changed, so that I don't have to leave the tool for `git show`.
- **As a reviewer annotating a historical commit**, I want the emitted markdown to state which commit the annotations refer to, so that an agent consuming the review resolves my line comments against the right revision.
- **As a user of the existing working-tree flow**, I want nothing about my current workflow or its stdout output to change, so that scripts and habits keep working.

## Demoable Units of Work

### Unit 1: Capability Model on the Diff Target

**Purpose:** Replace the scattered per-target `matches!` checks with named capability methods on `DiffTarget`, so every current and future source answers "can I stage / should I refresh / is code-intel valid?" in exactly one place. This unit is behavior-preserving for working-tree and staged views, and fixes a real bug for range views (code-intel currently resolves against on-disk files that may not match the displayed text).

**Functional Requirements:**
- The system shall provide capability methods on `DiffTarget`: `is_live()` (does the source change under the app, driving auto-refresh and untracked-file injection), `staging_mode()` returning one of `Stage`, `Unstage`, or `ReadOnly`, and `supports_code_intel()` (true only when the diff's new side is the live working tree).
- The system shall route all existing per-target decisions through these methods: the auto-refresh gate (`refresh.rs`), untracked-file injection (`stage_ops.rs`), the staging direction/read-only guards (`staging.rs`, `app.rs`), and the help/footer key visibility (`mod.rs`, `footer.rs`). No `matches!(target, ...)` capability checks remain at call sites.
- The system shall disable LSP code-intel requests and peek previews whenever `supports_code_intel()` is false, and the corresponding keybindings shall be absent from the `?` help overlay and footer hints in that state (same mechanism as the existing staging-key visibility).
- The system shall document the degradation contract (code-intel silently absent on non-worktree sources, and why) in the code-intel module doc, per the repository's error-handling rules.
- Working-tree and `--staged` behavior shall be observably unchanged (same staging directions, same refresh behavior, same help contents).

**Proof Artifacts:**
- Test: unit tests asserting each `DiffTarget` variant's capability triple (`is_live`, `staging_mode`, `supports_code_intel`) demonstrates the model is exhaustive and matches current behavior for existing variants.
- CLI: `redquill main..HEAD` with an LSP server configured shows no code-intel keys in the `?` overlay or footer, and pressing them does nothing, demonstrates the gate works end-to-end.
- Test: existing staging/refresh test suites pass unchanged, demonstrates the refactor is behavior-preserving.

### Unit 2: Single-Commit Diff Target and Commit-Log Read Model

**Purpose:** The git layer learns two new things: "produce the diff a commit introduced" and "list the current branch's commit history." Pure domain work, fully unit-testable, no UI.

**Functional Requirements:**
- The system shall support a `DiffTarget::Commit(rev)` variant whose diff is what that commit changed relative to its first parent (`git diff <rev>^ <rev>` semantics; merge commits diff against the first parent).
- The system shall render a root commit (no parent) as an all-added diff (diff against git's empty tree).
- The system shall resolve syntax-highlighting/full-file content for a commit target from git objects (`<rev>^:<path>` / `<rev>:<path>` via the existing content-source seam), never from the working tree on disk.
- A commit target shall report capabilities `is_live() == false`, `staging_mode() == ReadOnly`, `supports_code_intel() == false`.
- The system shall provide a commit-log read model in `src/git/` that lists commits for the current branch (or HEAD when detached), newest first, with per-commit: full SHA, short SHA, subject, author name, and commit timestamp — parsed from a NUL-delimited machine-readable `git log` format, never scraped from human-readable output.
- The commit-log read model shall support incremental fetching (an initial page plus "fetch more" with count/skip), so the panel never needs the whole history up front.
- Git invocations shall be built from closed types (no string interpolation into a shell; rev arguments passed as discrete argv elements).

**Proof Artifacts:**
- Test: parser unit tests (TDD) for the commit-log format, including subjects containing separators and multi-line-safe handling, demonstrates the read model is robust.
- Test: integration test in a tempdir repo asserting `DiffTarget::Commit` produces the commit's own changes (not worktree-vs-rev), including a merge commit (first parent) and a root commit (all-added), demonstrates correct diff semantics.
- Test: content-source tests asserting commit-target sides map to `<rev>^:<path>` / `<rev>:<path>`, demonstrates historical content never reads the disk.

### Unit 3: Git Panel History Tab and Commit View

**Purpose:** The user-facing entry point, mirroring modern Zed: the git panel gains a Changes ⇄ History tab pair; Enter on a history row opens that commit in the multibuffer viewer, read-only, with a way back.

**Functional Requirements:**
- The git panel shall have two tabs, Changes (the existing panel content) and History, toggled by a panel-scoped keybinding registered in the shared keymap tables (`modal_keys.rs` / `keymap.rs`) and listed in the `?` help overlay. Suggested default: `Tab` while the panel is focused.
- The History tab shall list commits newest-first as two-line rows: line 1 the subject (truncated to fit); line 2, dimmed: author name, relative time (e.g. "2 days ago"), and short SHA. Commits not yet pushed to the upstream (within the existing ahead count) shall show an unpushed marker on line 1.
- History rows shall load asynchronously via the existing background-operation pattern (the render loop never blocks on `git log`); a loading placeholder shows until the first page arrives, and scrolling near the end fetches the next page.
- The user shall move a highlighted row with the panel's existing cursor keys and open the highlighted commit with Enter; opening replaces the main multibuffer content with that commit's diff.
- The commit view shall show a header block above the diff: short SHA, author name, absolute date, and the commit subject.
- The commit view shall support the same multibuffer navigation as the working-tree view (file expand/collapse, hunk/file jumping, search) and shall support annotations (line/range/hunk/file).
- The commit view shall expose no staging affordances (keys inert and absent from help/footer, driven by `staging_mode() == ReadOnly`), no code-intel, and no auto-refresh — all via the Unit 1 capability model, not view-local checks.
- The user shall return from the commit view to the previous view (working tree/staged/range, with its state) via a keybinding registered in the shared tables. Suggested default: `Esc`.
- Quitting with `q` from a commit view shall behave as it does today: emit annotations and exit.

**Proof Artifacts:**
- Demo/screenshots: panel History tab with commit rows, an opened commit view with header and collapsed/expanded files, and the `?` overlay showing History/return keys but no staging or code-intel keys, demonstrates the end-to-end flow and capability gating.
- Test: UI-state tests asserting open-commit/return round-trips preserve the prior view's target and cursor state, demonstrates navigation correctness.
- Test: existing wall-clock perf tripwires (`src/ui/perf_tests.rs`) still pass, demonstrates the History tab and commit view don't regress the render loop's complexity class.

### Unit 4: Source-Aware Annotation Output

**Purpose:** Annotation stdout stays byte-identical for the existing flow, and becomes self-describing when annotations were made against a non-working-tree source — so downstream agents know which revision line numbers refer to.

**Functional Requirements:**
- The system shall record, with each annotation, the diff target it was created against.
- On quit, annotations made against the default working-tree target shall be emitted first, in the existing format, byte-identical to today's output.
- Annotations made against any non-working-tree target shall be emitted after the working-tree group, each group preceded by a single metadata line identifying the source (e.g. `Reviewing: abc1234` for a commit, `Reviewing: main..feature` for a range, `Reviewing: staged` for the index), followed by that group's annotations in the existing per-annotation format.
- A session that never leaves the working-tree view shall produce output with no metadata lines — byte-identical to current output.
- The exact metadata-line syntax shall be documented alongside the existing format contract (annotate module doc and README) and covered by byte-exact tests, since the stdout format is a public API.

**Proof Artifacts:**
- Test: byte-exact serialization tests for (a) a working-tree-only session (identical to current fixtures) and (b) a mixed session (working-tree + commit annotations, grouped with one `Reviewing:` line), demonstrates both the compatibility guarantee and the extension.
- CLI: a scripted session annotating a commit shows the `Reviewing: <short-sha>` line in stdout, demonstrates consumers can resolve the revision.
- Docs: updated format documentation demonstrates the public API contract includes the extension.

### Unit 5: Empty-Diff Welcome State

**Purpose:** A user who launches redquill with nothing to review (most commonly: the agent already committed, so the working tree is clean) currently gets a blank screen — the exact moment the tool should be teaching them what they *can* do. Replace the void with a small welcome state that points to real actions, the way Zed's project diff shows "No uncommitted changes" and lazygit surfaces its keybinds.

**Functional Requirements:**
- The system shall render a welcome/empty state in the diff area whenever the current diff target yields zero files, instead of a blank buffer.
- The welcome state shall name the situation for the active target (e.g. "No uncommitted changes" for the working tree; equivalent wording for an empty staged/range/commit diff).
- The welcome state shall list a small set of actionable next steps with their keys — at minimum: open the git panel, switch to the History tab to review recent commits, and open the `?` help overlay.
- Hint keys shall be rendered from the shared keymap tables, never hardcoded literals, so remapped keys display correctly and the existing drift-test mechanism covers the hints.
- The welcome state shall disappear as soon as the target has content (e.g. a working-tree edit arrives via auto-refresh).

**Proof Artifacts:**
- Screenshot: launching in a clean repo shows the welcome state with situation text and keyed hints, demonstrates the dead-end screen now teaches the escape route.
- Test: UI-state tests pass asserting empty target → welcome rendered / non-empty target → absent, and that hint keys come from the keymap tables (drift-style), demonstrates correctness and remap safety.

## Non-Goals (Out of Scope)

1. **New CLI surface**: no `--commit` flag, no `show` subcommand, no second positional (`redquill A B`). The existing CLI (bare, `--staged`, `A..B`/`A...B`) is unchanged; commit review is reached in-app. (Scripting a commit diff remains possible today via the range path, e.g. `rev^..rev`.)
2. **Hunk restore/revert in the commit view**: Zed offers restore-hunk there; redquill must not — it is a `checkout --`-class destructive operation, forbidden by the repository write ceiling (CLAUDE.md Guardrails).
3. **Checkout/revert/cherry-pick of a commit, or any history-mutating action** from the History tab or commit view.
4. **Commit search/filtering, a full git-graph view, tag chips, avatars, "open on GitHub" links**: Zed pushes search into a separate Git Graph view; redquill defers all of these.
5. **Feeding historical text to LSP servers** so code-intel works on non-worktree sources — deliberately deferred; the capability gate leaves room to add it later without rework.
6. **Full commit message body rendering/expansion** in the commit-view header (subject only for now).
7. **Stash-entry diff targets and stash actions** — the stash section remains view-only per the earlier panel decisions.

## Design Considerations

Mirror modern Zed's shapes, translated to a TUI:

- **Panel tabs**: Zed's git panel is `Changes ⇄ History`; redquill renders the same two-tab header inside the existing panel chrome. The History list is virtualized/paged (Zed uses a uniform list over lazily hydrated entries) — in redquill terms: render only visible rows, fetch pages in the background.
- **Row anatomy** (Zed-faithful, two lines): subject + unpushed marker; then dimmed `author · relative-time · short-sha`.
- **Commit view**: Zed opens a tab; redquill has a single main pane (no top-level tabs, per the ratified layout decision), so opening a commit swaps the multibuffer content and `Esc` returns to the previous view with its state intact. The header block stands in for Zed's commit header (author, date, SHA, subject).
- **Capability absence is invisible, not disabled-looking**: keys that don't apply are absent from help/footer (existing staging-key pattern), matching Zed's "structurally absent" approach.
- All new keys live in the shared keymap tables and the `?` overlay — no hidden features.

## Repository Standards

- Layering: `git/` gains the commit-log read model and commit diff acquisition with no TUI types; `ui/` consumes them. No presentation types in domain signatures.
- TDD for the pure code: log parsing, commit diff target semantics, annotation serialization (byte-exact fixtures).
- Integration tests build throwaway repos in tempdirs (canonicalized paths); never touch the host repo.
- Background work follows the existing pattern: spawn on a background thread, re-enter via non-blocking polling, single-flight + generation guards; the render loop never blocks on git.
- Subprocess hygiene: closed-type argv, machine-readable formats (NUL-delimited log format), `GIT_TERMINAL_PROMPT=0`.
- All four gates (`cargo build`, `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check`) pass before any task is considered done; conventional commits; refactor (Unit 1) and behavior changes (Units 2–4) in separate commits.
- Keymap/help drift protection: new keys added via the shared tables so the existing drift tests cover them.

## Technical Considerations

- **Extend, don't rebuild**: `DiffTarget` (`src/git/diff.rs`), the content-source seam (`src/ui/syntax.rs`), the `StageOps` trait, `build_review`, and the multibuffer row builder (`src/ui/rows.rs`) are already source-parametric. This spec adds a variant, capability methods, and new arms — the row/render model needs no changes.
- **Zed's verified architecture** is the reference: a small source enum in the domain layer (Zed's `DiffBase`), the same multibuffer for every source, capabilities gated by construction (Zed's commit view installs a restore-only delegate and read-only buffers; staging is impossible because no index comparison exists). Zed's History tab: `crates/git_ui/src/git_panel.rs` (`GitPanelTab`, `render_commit_history`); commit view: `crates/git_ui/src/commit_view.rs`.
- **Capability exhaustiveness**: implement the capability methods as exhaustive matches on `DiffTarget` so adding a future variant forces a compiler-driven decision at every capability, per the data-driven-invariants rule.
- **View-stack for return navigation**: returning from a commit view must restore the previous target's full view state (files, collapse, cursor, staged markers). Prefer suspending/restoring the previous view state over re-deriving it, but re-running `build_review` for the previous target on return is acceptable if state restoration proves equivalent.
- **Unpushed marker** reuses the existing ahead/behind read model from the git panel work (first `ahead` commits in the list are unpushed).
- **Performance**: History paging keeps `git log` cost bounded; commit diffs load once (no refresh polling — `is_live() == false` means the auto-refresh tick skips them entirely, which is a small win for the perf budget). Perf tripwires in `src/ui/perf_tests.rs` must keep passing unmodified.
- No new dependencies are anticipated; everything shells out to git on PATH per the stack rules.

## Security Considerations

- Rev strings and paths are passed as discrete argv elements to `git` (no shell, no interpolation), consistent with existing subprocess hygiene; commit SHAs shown in the panel come from parsed `git log` output.
- The feature adds no network calls, credentials, or new write operations; the commit view is strictly read-only, and the repository write ceiling is unchanged.
- Proof artifacts must not include contents of private repos other than the throwaway tempdir fixtures.

## Success Metrics

These are user-verifiable acceptance scenarios, not internal test counts — they define "done," and validation grades them before any technical gate. The problem being solved: the keep-or-fix review loop currently only works if the user catches the agent before it commits, and on non-default diff sources the tool can actively mislead (code-intel resolving against the wrong text).

1. **The dead-end disappears**: in a repo where an agent has just committed (clean working tree — today an empty, useless viewer), a user discovering controls only from the empty-state hints and `?` gets from launch to reading that commit's diff in a handful of obvious keystrokes (target: panel → History → select → open, ≤5 keys).
2. **The fix-loop works on history**: annotating lines of a historical commit and quitting produces stdout an agent can act on unambiguously — verified by running the README's own pipe (`redquill | <agent> -p "address this review feedback"`) against a committed change and confirming the agent locates every annotated site.
3. **The tool never lies**: in any non-working-tree view, every key shown in `?`/footer does what it says, and no absent capability (staging, code-intel) has any effect. No wrong jumps, no misleading affordances during review.
4. **Existing habits unbroken**: working-tree-only sessions are byte-identical on stdout and unchanged in keys and behavior — existing scripts and muscle memory require zero adjustment.
5. **Dogfood gate**: this spec's own implementation commits are reviewed *using the History tab they built*, and the emitted annotation output is captured as a proof artifact. If redquill can't pleasantly review the commits that created this feature, the feature isn't done.
6. **Still instant**: all wall-clock perf tripwires pass with unmodified budgets — history browsing and commit opening keep the instant feel on a 5k-line diff.

## Open Questions

1. Suggested keybinding defaults (`Tab` for panel tab toggle, `Esc` for returning from a commit view) are placeholders pending a conflict check against the shared keymap tables at implementation time; the semantic actions, not the specific keys, are the requirement.
2. The exact `Reviewing:` metadata-line syntax (e.g. short vs. full SHA, whether `staged` is spelled `staged` or `index`) will be fixed in Unit 4's format documentation and byte-exact tests; any reasonable, documented choice is acceptable.
3. History page size (e.g. 100 commits per fetch) is an implementation tuning choice, not a contract.
