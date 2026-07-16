# 08-tasks-branch-review-mode.md

Tasks for `08-spec-branch-review-mode.md`. Each parent task is a thin vertical slice, independently verifiable from the user's perspective — every slice ends with something a user can run, see, or do, not just passing tests.

## Relevant Files

| File | Why It Is Relevant |
| --- | --- |
| `src/main.rs` | CLI args (`--review`, `--base`), session wiring, review-session start path. |
| `src/git/diff.rs` | `DiffTarget` enum — new `Review { base, branch }` variant and its three capability answers. |
| `src/git/runner.rs` | New fixed-argv runner methods: `git_common_dir`, `default_base`, `worktree_add`, `worktree_remove`, `worktree_prune`, `blob_sha`. |
| `src/git/runner_tests.rs` (or module tests) | TDD fixtures for the new runner methods against tempdir repos. |
| `src/review/mod.rs` | New module: review-status domain model + persistence (pure, no TUI types); add to CLAUDE.md architecture map. |
| `src/review/model.rs` | `ReviewStatus` enum + pure transition functions (TDD). |
| `src/review/store.rs` | `review-state.json` schema, serde round-trip, atomic write, reconciliation, GC, corrupt-file recovery. |
| `src/review/store_tests.rs` | TDD tests for schema/round-trip/reconciliation/GC/corruption. |
| `src/ui/app.rs` | Review-session state on `App`, accept/defer handlers, auto-collapse wiring, save-on-change hooks. |
| `src/ui/keymap.rs` | New `Action` variants + `Binding` rows (review keys, review-branch modal opener). |
| `src/ui/modal_keys.rs` | Key tables for the end-review modal and review-branch modal (+ drift tests). |
| `src/ui/modes.rs` | New `Mode` variants and handlers for the two modals. |
| `src/ui/mod.rs` | Layout: banner band in `draw()` AND the viewport-measurement mirror (~line 747). |
| `src/ui/theme.rs` | `review_banner_bg`/`review_banner_fg` + contrast drift-guard test. |
| `src/ui/rows.rs` | Section-header review markers (O(1) lookups in the row-build path). |
| `src/ui/switcher.rs` | Reroot flow reference; branch read models reused by the review-branch modal. |
| `src/ui/perf_tests.rs` | Must pass unmodified — tripwire for marker lookups in the hot path. |
| `CLAUDE.md` | Write-ceiling amendment (worktree add / no-force remove / prune) + architecture-map entry for `review/`. |
| `docs/specs/08-spec-branch-review-mode/proofs/` | Captured transcripts/screenshots (gitignored per repo convention). |

### Notes

- All four gates before every commit: `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`. Conventional commits; each parent task lands as one or a few self-contained commits; refactors and behavior changes never share a commit.
- **Tempdir isolation is a first-class requirement** (2026-07-16 incident: a test escaped its tempdir and mutated a real repo; switcher-shaped tests suspected). Every integration test in this spec: build repos with `tempfile`, canonicalize paths (macOS `/var` symlink), pin git calls inside the tempdir (`-C <tempdir>` / cwd), and use a shared isolation-assertion helper that verifies the fixture path is under the tempdir root before any mutating git call. Worktree tests are exactly the incident's risk shape — the isolation sub-tasks are blocking, not hygiene.
- TDD order for pure code (runner output parsing, sanitization, schema, reconciliation): failing test first, tests committed with the code.
- Agents must not run worktree/fetch/pull/push/commit operations against the user's real repo; user-perspective proofs are captured by the user or in scratch repos, per the guardrails.

## Tasks

### [x] 1.0 Start a review from the CLI: `redquill --review <branch>` opens the branch in its own worktree

**User can verify:** run `redquill --review <branch>` in any repo → the `base...branch` diff opens, `git worktree list` shows the new worktree under `.git/redquill/worktrees/`, and `git status`/HEAD in their own checkout are untouched. `gd` on a symbol lands in the worktree's file, proving LSP is truthful. Bad inputs (unknown branch, branch checked out elsewhere) produce a readable error, not a crash.

Covers: spec Unit 1 (CLI path), `DiffTarget::Review` variant, base resolution (`origin/HEAD` → `main` → `master`), worktree add plumbing, write-ceiling amendment in CLAUDE.md (add/remove/prune, no `--force`).

#### 1.0 Proof Artifact(s)

- CLI: transcript of `redquill --review <branch>` then `git worktree list` and `git status` in the original checkout demonstrates the session opens on the range diff and the user's checkout is untouched (FR: Unit 1).
- Screenshot: `gd` (go-to-definition) inside the review session opening a file under `.git/redquill/worktrees/` demonstrates code intel navigates the branch's real files (FR: Unit 1 LSP truthfulness).
- CLI: transcript of `redquill --review no-such-branch` and `--review <currently-checked-out-branch>` demonstrates each failure surfaces git's message and exits cleanly with no side effects (FR: Unit 1 error handling).
- Test: tempdir integration tests for worktree creation path/sanitization, reuse on relaunch, base-resolution fallback chain, and both failure cases demonstrate the plumbing contract.
- Diff: CLAUDE.md guardrails section showing the amended product write ceiling demonstrates docs-as-contract is upheld.

#### 1.0 Tasks

- [x] 1.1 TDD new `GitRunner` methods against tempdir fixture repos: `git_common_dir()` (`rev-parse --git-common-dir`, canonicalized absolute path), `default_base()` (`symbolic-ref refs/remotes/origin/HEAD` → `main` → `master`, typed error naming `--base` when none resolve), `worktree_add(path, branch)` (fixed argv `worktree add <path> <branch>`, never `--force`, typed error carrying git's stderr). Follow the `verify_rev`/`switch_branch` precedents; `GIT_TERMINAL_PROMPT=0`.
- [x] 1.2 TDD a pure sanitization helper: branch name → worktree directory name (keep `[A-Za-z0-9._-]`, others → `-`, append short std-hash suffix of the original name). Collision test: `feat/x` vs `feat-x` map to different directories.
- [x] 1.3 Add `DiffTarget::Review { base, branch }` in `src/git/diff.rs`; the compiler forces decisions at the three capability matches — `is_live() == false`, `staging_mode() == ReadOnly`, `supports_code_intel() == true` — plus diff argv in the runner using three-dot `base...branch`. Unit-test all four sites.
- [x] 1.4 CLI wiring in `src/main.rs`: `--review <branch>` (conflicts with `--staged` and the positional range) and `--base <ref>`; flow = resolve base → ensure worktree (create, or reuse a paused review's) → `discover_in(worktree_path)` → review snapshot → `App` with the `Review` target so LSP and `g<Space>` root in the worktree. Every failure exits with a readable message and zero side effects.
- [x] 1.5 Tempdir integration tests for the full CLI flow per the isolation notes: creation at the sanitized path, reuse on second launch, unknown-branch and already-checked-out failures — plus the shared isolation-assertion helper, introduced here and used by every mutating test in this spec.
- [x] 1.6 Amend CLAUDE.md: product write ceiling gains `git worktree add <path> <branch>`, `git worktree remove <path>` (never `--force`), `git worktree prune`, with forced removal explicitly forbidden; agent ceiling unchanged; architecture map gains the `review/` module line (landed in 3.1 but documented once here).
- [x] 1.7 Run gates; capture the transcripts + `gd` screenshot into `proofs/`; commit.

### [x] 2.0 Review banner and ending a review: `q` → pause or finish

**User can verify:** while reviewing, a dark-red full-width banner reads `REVIEWING <branch> — q to end review`; pressing `q` opens a modal whose choices are labeled with consequences; **pause** quits with annotations on stdout and the worktree still present; **finish** quits with annotations on stdout and `git worktree list` no longer shows the worktree; `Q` quits instantly with nothing emitted; a deliberately dirtied worktree makes finish fail with a message instead of deleting anything. In a plain working-tree session, `q`/`Q` are byte-for-byte unchanged.

Covers: spec Unit 2 — banner + theme fields + contrast guard, layout accounting in both split sites, end-review modal, worktree remove/prune, annotation emission under the existing `Reviewing:` group-line contract.

#### 2.0 Proof Artifact(s)

- Screenshot: review session showing the banner above the diff demonstrates the mode is unmistakable and layout is intact (FR: Unit 2 banner).
- CLI: transcript of pause (worktree survives) vs finish (worktree gone) vs `Q` (no stdout emission) demonstrates all three exits behave as specified (FR: Unit 2 lifecycle).
- CLI: transcript of finish against a dirtied worktree demonstrates no-force removal fails safely with the session continuing (FR: Unit 2 safety).
- Test: theme contrast drift-guard test and end-review modal drift tests pass demonstrates high contrast is enforced and help/behavior stay in sync; a regression test pins `q`/`Q` behavior outside review mode.

#### 2.0 Tasks

- [x] 2.1 TDD theme additions: write the contrast drift-guard assertions first (brightness-delta pattern already in `theme.rs`), then add `review_banner_bg` (dark red) and `review_banner_fg` (light) to `Theme` and its default.
- [x] 2.2 TDD a pure banner-content helper: `(branch, accepted, total, width) → line`, truncating the branch name first, never wrapping. Render as a `Constraint::Length(1)` top band in `draw()` **and** subtract the same row in the viewport-measurement mirror in `src/ui/mod.rs` — add a test or debug assertion that the two split computations agree when the banner is active.
- [x] 2.3 End-review modal: new `Mode` variant + handler in `modes.rs`, key table in `modal_keys.rs` (pause / finish / cancel, labels naming consequences: "keep worktree" / "remove worktree"), bidirectional drift test. In review mode `q` opens this modal instead of quitting; `Q` keeps its global instant-quit; outside review mode `q`/`Q` are untouched — pin that with an explicit regression test.
- [x] 2.4 Runner methods `worktree_remove(path)` (fixed argv, never `--force`) and `worktree_prune()` (TDD in tempdirs, isolation helper from 1.5). Wire finish = emit annotations (existing `Reviewing: <range>` grouping, zero format changes) → remove worktree → prune → quit; on removal failure surface git's message and stay in the session. Pause = emit + quit, touching nothing.
- [x] 2.5 Tempdir integration tests: pause leaves the worktree, finish removes it, `Q` emits nothing, dirty-worktree finish fails with the worktree and session intact.
- [x] 2.6 Footer + `?` overlay entries for the review-mode `q` meaning via the shared tables; run gates; capture banner screenshot + lifecycle transcripts into `proofs/`; commit.
- [x] 2.7 Polish pass from user dogfood feedback: banner padding/weight + right-aligned progress; end-review modal compacted to content, single-line options with accented keys, j/k/Enter selection (lazygit-style).

### [ ] 3.0 Accept / defer files while reviewing

**User can verify:** during a review, `Space` accepts the cursor file (section collapses, `✓`-style marker appears in sidebar and section header, banner count increments), `Space` again un-accepts (expands, count decrements), `S` accepts from anywhere in the file, `d` defers (collapses with its own marker); `?` shows all three with review-specific descriptions; in a normal working-tree session these keys behave exactly as before.

Covers: spec Unit 3 — review-status enum + App map, key handling gated to review targets, markers via O(1) lookups, banner progress count, per-target key visibility in footer/`?`, perf tripwires unchanged.

#### 3.0 Proof Artifact(s)

- Screenshot: sidebar + section headers showing accepted and deferred markers with the banner count demonstrates the reviewer-facing tri-state (FR: Unit 3 rendering).
- CLI: transcript/screenshot of the `?` overlay in review mode vs working-tree mode demonstrates mode-appropriate key descriptions and unchanged local behavior (FR: Unit 3 gating).
- Test: unit tests over status transitions and collapse side effects demonstrate the state machine matches the spec.
- Test: existing perf tripwires pass unmodified demonstrates no hot-path complexity regression.

#### 3.0 Tasks

- [ ] 3.1 Create the `src/review/` module: `model.rs` with `ReviewStatus { Unreviewed (default), Deferred, Accepted, ChangedSinceAccepted }` and pure transition functions (`toggle_accept`, `toggle_defer`, including `ChangedSinceAccepted` + accept → `Accepted`). TDD the full transition table; exhaustive matches only; no TUI types.
- [ ] 3.2 `App` state: path-keyed `HashMap<String, ReviewStatus>` mirroring `staged_states` (missing = `Unreviewed`), populated only for review sessions.
- [ ] 3.3 Key handling: new `Action` variants (`ToggleAccept`, `AcceptFile`, `ToggleDefer`) bound to `Space`/`S`/`d`, active only on the `Review` target; other targets keep existing behavior untouched (staging keys on `Range` stay inert as shipped). Accept auto-collapses via `set_collapsed`, un-accept expands (mirroring stage-auto-collapse); defer collapses.
- [ ] 3.4 Rendering: distinct single-cell markers (working glyphs `✓` accepted / `~` deferred / `!` changed-since-accepted; finalize against terminal font fallback) on sidebar rows and `rows.rs` section headers via O(1) map lookups, visually distinct from staging's `±`/`●`; wire the banner's `accepted/total` count from 2.2.
- [ ] 3.5 Per-target key visibility: review keys appear in footer/`?` only in review sessions with review-specific descriptions, per the README rule that inapplicable keys are omitted rather than inert; extend the drift tests both directions.
- [ ] 3.6 Run the perf tripwires unmodified (marker lookups must not change row-build complexity); run gates; capture marker screenshot + dual `?` overlay proof into `proofs/`; commit.

### [ ] 4.0 Review progress survives sessions and self-invalidates when files change

**User can verify:** accept files, quit with pause; relaunch `--review <branch>` → accepted files are still accepted and collapsed; commit a change to one accepted file on the branch, relaunch → exactly that file shows the changed-since-accepted marker, un-collapsed, and one `Space` re-accepts it; deferred files carry over; finish deletes the state entry; deleting the branch then launching redquill GCs the entry; a hand-corrupted state file is set aside with a stderr note instead of crashing.

Covers: spec Unit 4 — `review-state.json` in the common git dir, atomic writes, blob-SHA capture via `rev-parse <branch>:<path>`, reconciliation, GC, corrupt-file recovery, finish-time state deletion (closing the loop with task 2.0).

#### 4.0 Proof Artifact(s)

- CLI: scripted two-session transcript (accept → pause → author commits to one file → resume) demonstrates per-file staleness demotion and everything else staying accepted (FR: Unit 4 reconciliation).
- CLI: transcript of finish followed by inspecting `review-state.json`, and of launch-after-branch-delete, demonstrates state deletion and GC (FR: Unit 4 cleanup).
- Test: TDD round-trip serialization tests (byte-exact), reconciliation/GC/corrupt-file tempdir tests demonstrate the persistence contract.

#### 4.0 Tasks

- [ ] 4.1 TDD the schema in `src/review/store.rs`: serde types (schema `version` field; per-branch entry = base ref, worktree path, per-file `{ status, blob_sha }`), byte-exact round-trip test, atomic write (temp file + rename in the same directory), path = `<git_common_dir>/redquill/review-state.json`. Typed errors; no TUI types.
- [ ] 4.2 TDD runner method `blob_sha(branch, path)` wrapping `git rev-parse <branch>:<path>` (full SHA; typed handling for paths absent on the branch — an accepted deleted file records its absence, not a SHA).
- [ ] 4.3 TDD pure reconciliation: `(persisted entry, current blob SHAs) → status map` — matching `Accepted` stays; mismatch → `ChangedSinceAccepted`; `Deferred` carries over; files new on the branch since last session are `Unreviewed`.
- [ ] 4.4 Wire persistence into the session: save after every status change (crash/`Q`-safe, off the render loop per the concurrency rules); on session start load + reconcile; `ChangedSinceAccepted` renders un-collapsed with its marker and `Space` re-accepts at the fresh SHA.
- [ ] 4.5 Launch-time GC + corruption handling: drop entries whose branch no longer exists (never touching live entries); corrupt file → rename to `review-state.json.corrupt`, one stderr line, continue empty. Document this as the module's silent-degradation contract in the module doc. Tempdir tests for both.
- [ ] 4.6 Finish (2.4) additionally deletes the branch's state entry; two-session tempdir integration test covering resume → staleness → re-accept → finish; run gates; capture the two-session and cleanup transcripts into `proofs/`; commit.

### [ ] 5.0 Start a review without leaving the app: review-branch modal

**User can verify:** from a normal redquill session, open the git panel, press the new review key → a modal lists local branches (current branch excluded), Enter on one lands in the same banner-topped review session as the CLI path (worktree created or reused, review states restored); Esc dismisses; a branch that can't be worktree'd shows the git error in-app; the action appears in the `?` overlay.

Covers: spec Unit 1 (in-app path) — panel-scope binding, modal reusing spec-03 branch read models, session start via `App::reroot`, error surfacing.

#### 5.0 Proof Artifact(s)

- Screenshot: the review-branch modal over the git panel demonstrates the in-app entry point exists and is keymap-discoverable (FR: Unit 1 in-app).
- Screenshot/CLI: selecting a branch landing in the bannered review session with previously-persisted marks restored demonstrates parity with the CLI path (FR: Units 1+4 integration).
- Test: modal drift tests and a reroot-into-review integration test demonstrate keymap sync and the session-start path.

#### 5.0 Tasks

- [ ] 5.1 New panel-scope `Action` + binding opening `Mode::ReviewBranch`: modal lists local branches via the existing `branch_list` read model, excluding the branch checked out in the user's worktree; cursor/Enter/Esc handling and a `modal_keys.rs` table with drift test, styled like the spec-03 switcher list.
- [ ] 5.2 In-app session start sharing the CLI path's core: resolve base → ensure worktree → re-root via `App::reroot` (build-before-swap, LSP re-create, annotation preservation) → `Review` target snapshot → load + reconcile persisted state (4.4). One "ensure review session" code path, two entry points.
- [ ] 5.3 In-app failure surfacing: `worktree_add`/reroot errors render in the modal or panel message area (existing error-surface pattern), never crash, never mutate state.
- [ ] 5.4 Reroot-into-review tempdir integration test (isolation helper mandatory — this is the switcher-adjacent shape the incident implicates); `?` overlay entry; run gates; capture modal + parity screenshots into `proofs/`; commit.
