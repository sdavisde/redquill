# 08-spec-branch-review-mode.md

## Introduction/Overview

Branch review mode lets a developer review someone else's branch (typically a pull request) inside redquill without disturbing their own checkout. redquill creates a dedicated git worktree for the branch under review, shows the `base...branch` diff (merge-base semantics), and — because the worktree's files match the diff's post-state — LSP navigation and codebase exploration are truthful, which is the core advantage over web-based review.

On top of the existing diff/annotation experience, review mode adds a per-file review status (unreviewed / deferred / accepted) that generalizes the staging gesture, persists that status across sessions keyed by file content (blob SHA), and shows a high-contrast banner so the user always knows they are in a review session and how to leave it.

The core stays forge-agnostic: redquill reviews *a local branch*, not "a GitHub PR." Fetching the branch, posting comments, and PR verdicts remain outside this spec.

## Goals

- A developer can start reviewing any local branch in one step (CLI flag or in-app modal) and redquill handles all worktree plumbing.
- Review progress (accepted / deferred per file) survives quitting the app and resuming days later, and self-invalidates per file when the branch's content changes.
- LSP definition/references/hover work truthfully during a review because the checkout matches the diff head.
- The user can never confuse a review session with their normal working-tree session (persistent high-contrast banner), and can always end one safely (pause keeps everything; finish cleans everything up).
- The user's own checkout, index, and working tree are never touched by starting, pausing, or finishing a review.

## User Stories

- **As a developer reviewing a teammate's PR**, I want redquill to check the branch out into its own worktree so that I can review it without stashing or losing my own work in progress.
- **As a reviewer**, I want to mark files as accepted or "come back later" with the same gestures I already use for staging so that working through a large diff feels like the flow I already know.
- **As a reviewer returning for round two**, I want files I already accepted to stay accepted — unless their content actually changed — so that I only re-read what the author touched since my last pass.
- **As a reviewer exploring unfamiliar code**, I want go-to-definition and find-references to operate on the branch's real files so that I can answer "where is this used?" without leaving the terminal.
- **As a developer**, I want an unmissable banner while reviewing so that I never mistake a review worktree session for my own working tree and accidentally reason about the wrong checkout.

## Demoable Units of Work

### Unit 1: Start a review (CLI and in-app), worktree plumbing

**Purpose:** One-step entry into a review session, with redquill owning the `git worktree` lifecycle, for developers who want to review a branch without touching their checkout.

**Functional Requirements:**

- The system shall accept a new CLI flag `--review <branch>` (conflicts with `--staged` and the positional range argument) plus an optional `--base <ref>`.
- When `--base` is not given, the system shall resolve the base ref as: the branch `origin/HEAD` points to, else `main`, else `master`; if none exists, exit with a clear error naming the `--base` flag.
- The system shall create the review worktree with fixed argv `git worktree add <path> <branch>` (never `--force`) at `<git-common-dir>/redquill/worktrees/<sanitized-branch>`, where `<git-common-dir>` comes from a new runner method wrapping `git rev-parse --git-common-dir`, and `<sanitized-branch>` replaces every character outside `[A-Za-z0-9._-]` with `-` and appends a short hash of the original branch name to prevent collisions.
- If the worktree for that branch already exists (a paused review), the system shall reuse it instead of creating a new one.
- The system shall represent the session with a new `DiffTarget::Review { base, branch }` variant whose capability answers are compiler-forced at the three existing exhaustive-match sites: `is_live() == false`, `staging_mode() == ReadOnly`, `supports_code_intel() == true`.
- The diff shown shall be the merge-base range (`base...branch` three-dot semantics), computed and rendered from within the review worktree, so LSP servers and `g<Space>` open the branch's files.
- The user shall be able to start the same flow in-app: a new panel-scope keybinding opens a "review branch" modal listing local branches (reusing the spec-03 branch read models), excluding the branch currently checked out in the user's worktree; selecting one starts the review session in place (re-rooting via the existing `App::reroot` machinery).
- When `git worktree add` fails (branch already checked out elsewhere, missing branch, path collision), the system shall surface git's message in the UI/stderr and leave all existing state untouched — never crash, never retry with force.
- Starting, pausing, and finishing a review shall never modify the user's original worktree, index, or HEAD.

**Proof Artifacts:**

- Test: integration tests in tempdir repos demonstrate worktree creation at the specified path, reuse on second launch, and each failure case (checked-out branch, unknown branch) surfacing an error without side effects.
- CLI: transcript of `redquill --review <branch>` followed by `git worktree list` and `git status` in the original checkout demonstrates the session starts and the user's checkout is untouched.
- Screenshot: the in-app review-branch modal listing branches demonstrates the in-app entry point exists and is keymap-reachable.

### Unit 2: Review banner and end-of-review lifecycle

**Purpose:** Make the review session unmistakable and give it a safe, explicit ending, for reviewers who pause and resume across days.

**Functional Requirements:**

- Whenever a review session is active, the system shall render a full-width, single-row banner at the top of the layout reading `REVIEWING <branch> — q to end review` (gracefully truncated on narrow terminals), including a progress count of accepted files (e.g. `4/12`).
- Banner colors shall come from two new `Theme` fields (`review_banner_bg`, a dark red; `review_banner_fg`, a light foreground), with a contrast drift-guard test in `theme.rs` following the existing brightness-assertion pattern.
- The banner row shall be accounted for in both layout computations (the `draw()` split and the viewport-measurement mirror) so cursor/scroll math stays correct.
- In review mode, `q` shall open an end-review modal with three exits, driven from a `modal_keys.rs` table with the standard bidirectional drift test:
  - **pause**: emit annotations to stdout (unchanged output contract), quit; worktree and review state are kept.
  - **finish**: emit annotations, remove the worktree with fixed argv `git worktree remove <path>` (never `--force`), delete this review's persisted state entry, quit. If removal fails (e.g. the worktree is dirty), surface git's message, keep the state entry, and stay in the app.
  - **cancel**: close the modal, continue reviewing.
- `Q` in review mode shall keep its global meaning: quit immediately, emit nothing; worktree and review state survive.
- Outside review mode, `q` and `Q` behavior shall be byte-for-byte unchanged.
- The system shall run `git worktree prune` (fixed argv) only as part of a successful finish, to clear administrative records.
- All new keys and the banner hint shall appear in the `?` help overlay via the shared keymap/modal tables — no loose match arms.

**Proof Artifacts:**

- Test: theme contrast drift-guard test passes demonstrates the banner is provably high-contrast; modal drift tests pass demonstrates help/behavior stay in sync.
- Screenshot: review session showing the dark-red banner above the diff demonstrates the banner renders and the layout is intact.
- Test: integration test demonstrates finish removes the worktree and the state entry while pause/`Q` leave both in place, and a dirty worktree makes finish fail safely with state intact.

### Unit 3: Per-file review tri-state

**Purpose:** Let reviewers work through a large diff the way they check off files on a forge — accept, defer, continue — using existing staging muscle memory.

**Functional Requirements:**

- The system shall model per-file review status as an exhaustively-matched enum: `Unreviewed` (default), `Deferred`, `Accepted`, `ChangedSinceAccepted`, stored as a path-keyed map on `App` mirroring `staged_states`.
- In review mode, `Space` shall toggle the cursor file between `Accepted` and `Unreviewed`; accepting shall auto-collapse the file's section and un-accepting shall expand it (mirroring stage-auto-collapse).
- In review mode, `S` shall accept the file under the cursor from anywhere inside it, mirroring `StageFile` fully — including its toggle direction: `S` on an already-`Accepted` file un-accepts it back to `Unreviewed` and re-expands the section (amended per Unit 5, user decision 2026-07-16).
- In review mode, `d` shall toggle the cursor file between `Deferred` and `Unreviewed`; deferring shall collapse the section.
- Each non-`Unreviewed` status shall render a distinct single-cell marker on the file's sidebar row and multibuffer section header, via O(1) map lookups inside the existing row-build path. `Accepted` shall render as a green `●` circle in the section header's marker slot (the filename row in the buffer) and on the sidebar row — deliberately mirroring the staged-file affordance so an accepted-and-collapsed file is unmistakably different from a merely-collapsed one (user decision, 2026-07-16). `~` deferred and `!` changed-since-accepted remain suggestions (final glyphs per the Open Question).
- Review-status keys shall be active only when the diff target is a review session; in all other targets `Space`/`S`/`d` keep their current behavior (which on `Range` targets is already inert for staging), and the `?` overlay shall show the mode-appropriate descriptions.
- Local working-tree and `--staged` behavior, including the existing `StagedState` display, shall be completely unchanged.
- The wall-clock perf tripwires in `src/ui/perf_tests.rs` shall pass unmodified — review-status lookups must not change the row-build complexity class.

**Proof Artifacts:**

- Test: unit tests over the status enum transitions (accept/un-accept/defer cycles, collapse side effects) demonstrate the state machine matches this spec.
- Screenshot: sidebar and section headers showing all three markers plus the banner progress count demonstrates the reviewer-facing rendering.
- Test: existing perf tripwires pass unmodified demonstrates no hot-path regression.

### Unit 4: Persistence, staleness, and cleanup

**Purpose:** Make review progress durable across sessions and honest across the author's pushes, for multi-day, multi-round reviews.

**Functional Requirements:**

- The system shall persist review state to `<git-common-dir>/redquill/review-state.json` (serde_json; both crates already in-tree), written atomically (temp file + rename) after every status change so a crash or `Q` loses nothing.
- The file shall store one entry per review keyed by branch, containing the base ref, the worktree path, and a per-file map of `{ status, blob_sha }`, where `blob_sha` is the file's blob SHA on the branch head at the moment of acceptance, obtained via a new runner method wrapping `git rev-parse <branch>:<path>` (following the `verify_rev` precedent).
- On starting or resuming a review, the system shall reconcile each persisted `Accepted` file against the branch's current blob SHA: on mismatch the file becomes `ChangedSinceAccepted` — visibly marked, not collapsed, and one `Space` press re-accepts it at the new blob SHA. `Deferred` status carries over as-is.
- On every launch, the system shall garbage-collect state entries whose branch no longer exists (and prune their worktree records); GC shall never touch entries for existing branches.
- A missing state file shall behave as empty. A corrupt state file shall be renamed aside (e.g. `review-state.json.corrupt`), reported on stderr, and treated as empty — reviews degrade to unreviewed rather than crashing. This is the module's documented silent-degradation contract.
- Serialization shall be developed test-first (TDD applies: this is pure serialization/parsing code), including a byte-exact round-trip test of the schema.

**Proof Artifacts:**

- Test: round-trip and reconciliation tests in tempdir repos demonstrate persistence, blob-SHA staleness demotion, deferred carry-over, GC of deleted branches, and corrupt-file recovery.
- CLI: a scripted two-session transcript (accept files → quit → commit a change to one file on the branch → relaunch) demonstrates exactly the touched file shows `ChangedSinceAccepted` while the rest stay accepted.

### Unit 5: Local-mode parity surfaces (added by 2026-07-16 parity audit, user-ratified)

**Purpose:** Every construct the user knows from local staging sessions gets a deliberate analogue — or a deliberate, recorded omission — in a review session, so the two modes feel like one tool. Added after a code audit found the staging panel, `S` toggle semantics, and the git panel's write operations were never re-examined for review mode.

**Functional Requirements:**

- `S` on an already-`Accepted` file shall un-accept it back to `Unreviewed` and re-expand its section — the full `StageFile` toggle analogue (this amends Unit 3's `S` requirement).
- In a review session, the `s` staging panel shall become the **accepted-files panel**: it lists the review's accepted files, `Space`/`Enter` on an entry un-accepts it (its diff section re-expands), and the empty state says no files are accepted yet. Outside review sessions the staging panel is byte-for-byte unchanged. Footer and `?` overlay describe the panel session-appropriately via the shared key tables, with bidirectional drift tests.
- In a review session, `p` (pull) and `P` (push) in the git panel shall require confirmation through a modal that names the branch under review (e.g. `Push <branch> — the branch under review?`); confirm runs the existing sanctioned op, Esc cancels. `f` (fetch) stays unprompted — reviewers are expected to fetch. Outside review sessions all three are unchanged. (Confirm-first over hard-block: user decision, 2026-07-16, preserving the maintainer workflow of pulling an author's new commits mid-review.)
- In a review session, the commit modal (`c`) shall show a prominent warning line naming the branch under review before the user can confirm; the existing nothing-staged gate is unchanged. (Same confirm-first policy as pull/push.)
- Deliberate omissions, recorded so they read as decisions rather than gaps: no hunk/line-level accept (review status is per-file by design); no "partially accepted" state; no `[N accepted]` count in the git panel (progress lives in the banner).

**Proof Artifacts:**

- Test: `S` toggle transition tests (accepted → un-accept + expand) mirroring the existing `StageFile` toggle tests.
- Screenshot/test-render: the accepted-files panel listing accepted files and un-accepting one via `Space`, demonstrating the unstage-panel analogue.
- Test/render: the pull/push confirm modal naming the reviewed branch, fetch running unprompted, and the commit modal's review warning line; drift tests for every new modal table.
- Test: regressions pinning staging panel, pull/push, and commit behavior outside review sessions as byte-for-byte unchanged.

## Non-Goals (Out of Scope)

1. **Forge integration**: no GitHub/GitLab awareness — no PR listing, no comment posting, no verdicts (approve/request-changes). A future spec may add this as a thin adapter; the core stays forge-agnostic.
2. **In-app ref-range entry**: typing an arbitrary range to re-target the open diff is excluded (user decision, round 2) and will be filed as its own small spec. CLI range launch continues to work as shipped in spec 05.
3. **Annotation persistence across sessions**: annotations remain in-memory and are emitted to stdout on quit (pause and finish both emit). Carrying unemitted annotations between sessions belongs to the future output-mechanisms / persisted-sessions spec.
4. **Local-mode changes**: working-tree and `--staged` review keep today's staging model untouched; no "deferred" marker in local mode.
5. **Remote operations on the user's behalf**: redquill does not fetch the branch being reviewed; the user fetches via the existing git panel or their shell.
6. **Configurability of worktree/state locations**: fixed paths under `.git/redquill/` for now; a spec-07 config knob can come later without migration pain.
7. **Reviewing remote-only branches**: the review-branch modal lists local branches only in this spec.

## Design Considerations

- **Banner**: single top row, full width, dark-red background with light foreground (exact colors via `Theme` fields, guarded by a contrast drift test). Text: `REVIEWING <branch> — q to end review`, plus an accepted-files progress count. Truncate the branch name first on narrow terminals; never wrap to a second row.
- **Markers**: deferred and changed-since-accepted glyphs must be visually distinct from the staging glyphs (`±`, `●`) so a user who uses both modes never confuses them. Deliberate exception (user decision, 2026-07-16): `Accepted` reuses the green `●` staged affordance on the filename row/sidebar — staging markers never render inside a review session (staging is read-only there), so the reuse is unambiguous in context and makes "accepted" as instantly legible in review mode as "staged" is in local mode.
- **End-review modal**: mirrors existing confirm modals (commit, switcher) in style and key handling; the three exits must be labeled with their consequences ("keep worktree" / "remove worktree"), not just "pause"/"finish".
- **Review-branch modal**: visually consistent with the spec-03 switcher list (current-branch exclusion, cursor navigation, Enter to select, Esc to dismiss).

## Repository Standards

- All four gates before every commit: `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`; conventional commits; refactors and behavior changes never share a commit.
- Keymap: every new action is a `Binding` row in `Keymap::default_map()`; modal keys live in `modal_keys.rs` tables with bidirectional drift tests; nothing user-visible outside the `?` overlay.
- Layering: worktree/rev-parse/prune operations are `GitRunner` methods with fixed argv built from closed types (no string interpolation, no `sh -c`), behind the existing runner/`StageOps` seam; no TUI types in `git/`; persistence schema types live outside `ui/`.
- Errors: typed errors in `git/` and the persistence module; no `unwrap()`/`expect()`/panics in production code; background git calls stay off the render loop (existing poller pattern) with `GIT_TERMINAL_PROMPT=0`.
- TDD for the pure parts: state-file serialization, blob-SHA reconciliation logic, branch-name sanitization.
- Integration tests build throwaway repos in tempdirs (canonicalize paths for macOS `/var`); never touch the host repo.

## Technical Considerations

- **Write-ceiling amendment (required, part of this spec's docs work)**: CLAUDE.md's guardrails section must be amended to add `git worktree add <path> <branch>`, `git worktree remove <path>` (never `--force`), and `git worktree prune` to the product's runtime write ceiling, with forced removal explicitly forbidden. The agent write ceiling (stage/unstage only) is unchanged — agents never run worktree operations against the user's repo, so this spec's acceptance journey is user-run.
- **`DiffTarget::Review` variant** is the plug-in point spec 05 designed for: adding it forces a compiler-driven decision at `is_live`/`staging_mode`/`supports_code_intel`. Code intel is truthful here precisely because the session runs inside the branch's worktree.
- **Common git dir vs toplevel**: `root()` returns the worktree toplevel, which is wrong for linked worktrees; all persistence and worktree paths must resolve through the new `--git-common-dir` runner method so state is shared no matter which worktree redquill runs in.
- **Blob SHAs** come from `git rev-parse <branch>:<path>` at acceptance time (full SHA, robust) rather than parsing abbreviated `index` lines from the diff.
- **Reuse over rebuild**: session start-in-app rides `App::reroot` (build-new-runner-then-swap, LSP re-create, annotation preservation); accept-auto-collapse rides `set_collapsed`; the review-branch modal rides the spec-03 branch read models.
- **Concurrent spec 07 (config layer)** is mid-flight on another branch; this spec deliberately takes no dependency on it.
- No new crates: serde/serde_json/toml already present; std for hashing the sanitized-name suffix is sufficient.
- No latest-standards research was needed: git worktree plumbing and blob addressing are stable git fundamentals, and all UI decisions are governed by in-repo conventions.

## Security Considerations

- Worktree removal is never forced; git's own dirty-tree refusal is the data-loss guard, and the failure path must keep review state so nothing is silently dropped.
- Fixed argv everywhere; branch names are passed as single argv elements (never interpolated into a shell string) and directory names are sanitized as specified.
- The state file contains only paths, refs, and SHAs — no secrets; it lives inside `.git/` and is never committed.
- No network operations are introduced.

## Success Metrics

1. **End-to-end review journey (acceptance task, user-run, evidence persisted)**: in a scratch repo, start `redquill --review <branch>`, accept some files and defer one, pause via `q`; commit a change to one accepted file on the branch; resume — exactly that file shows changed-since-accepted, others stay accepted; finish via `q` — annotations emit, `git worktree list` shows the worktree gone, the state entry is deleted, and the original checkout was never modified. Evidence: terminal transcript/screenshots plus before/after `git worktree list` and state-file snapshots.
2. **Isolation invariant**: `git status` and `git rev-parse HEAD` in the user's original worktree are identical before starting and after finishing a review.
3. **No regressions**: all existing tests including the perf tripwires pass unmodified; local-mode staging behavior is unchanged.
4. **Discoverability**: every new action appears in the `?` overlay, verified by the existing drift tests extended to the new tables.

## Open Questions

1. ~~Final marker glyph for accepted~~ Resolved 2026-07-16: accepted = green `●` (staged-affordance reuse, see Design Considerations). Final glyphs for deferred/changed-since-accepted (suggested `~`/`!`) — cosmetic, decide during implementation against terminal-width and font-fallback behavior.
2. Banner progress-count format (`4/12` vs `4 of 12 accepted`) and truncation order on very narrow terminals — cosmetic.
3. Whether the review-branch modal should offer remote-tracking branches with a "create local branch first" affordance — deferred to the forge-integration follow-up; local-only is acceptable for v1.
