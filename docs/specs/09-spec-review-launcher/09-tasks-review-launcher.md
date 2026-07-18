# 09-tasks-review-launcher.md

Tasks for `09-spec-review-launcher.md` (FR-1..FR-14). Each parent task is a thin vertical slice, independently verifiable from the user's perspective, with persisted proof artifacts per this repo's UX-outcome verification convention (journey transcripts captured into `docs/specs/09-spec-review-launcher/proofs/`, gitignored per repo convention). Worktree-touching flows are exercised only against agent-created scratch repos in tempdirs, per the repo guardrails.

## Standards Evidence

| Source File | Read | Standards Extracted | Conflicts |
| --- | --- | --- | --- |
| `CLAUDE.md` | Yes | (1) Keymap and modal keys are data in the shared tables (`src/ui/keymap.rs`, `src/ui/modal_keys.rs`), never loose match arms; every user-visible action reachable from the keymap and listed in `?` help. (2) Agent write ceiling during tasks: staging/unstaging only — no worktree add/remove/prune, fetch/pull/push, or product-commit against the user's real repo; `--review`/worktree testing only in scratch tempdir repos. (3) Perf tripwires in `src/ui/perf_tests.rs` enforce the complexity class — keep passing, never loosen budgets. | None |
| `README.md` | Yes | (1) Product promise: `?` shows the list of keybinds — help must stay truthful as bindings move. (2) Vision: redquill is the human checkpoint between agent output and commit — the launcher serves the review-agent-commit journey. (3) Don't promise unbuilt features in present tense (no dead PR-tab UI). | None |
| `docs/rust-best-practices.md` | Yes | (1) No `unwrap`/`expect`/panic macros in production code; typed errors in `git/`. (2) Four gates before every commit (`cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`); conventional commits; refactors and behavior changes never share a commit. (3) Data-driven invariants with bidirectional drift tests (keybindings ↔ help ↔ footer); background work off the render loop with generation guards; TDD for pure parsers; integration tests in canonicalized tempdirs only. | None |
| `CONTRIBUTING.md` | Not found | — | — |
| `.github/pull_request_template.md` | Not found (`.github/` contains only `workflows/`) | — | — |
| `docs/specs/08-spec-branch-review-mode/08-tasks-branch-review-mode.md` | Yes (format precedent) | (1) Parent-task format: checkbox title, "Covers:" line, Proof Artifact(s) with FR references, numbered sub-task list. (2) Proofs captured into a spec-local `proofs/` directory (gitignored). (3) Tempdir isolation is a first-class blocking requirement (2026-07-16 incident): canonicalize paths, pin git calls inside the tempdir, shared isolation-assertion helper before any mutating git call. | None |

## Relevant Files

| File | Why It Is Relevant |
| --- | --- |
| `src/ui/keymap.rs` | `Scope` enum gains `Global` (~372); scope-then-global resolution in `lookup_in`/`resolve_in`/`starts_sequence_in`/`completions_for`/`label_for`; migrate duplicated rows (diff ~604/677/682/739/744, panel ~809/821/826/833/838); new `OpenReviewLauncher` action + `action_name` round-trip; `R`→launcher, refresh `R`→`r` (~687), panel `R` row removed (~816); existing test pins at ~1056–1082, ~1128/1132 (`R` ± SHIFT), ~1136 (`r` → None) must be rewritten. |
| `src/ui/keymap_config.rs` | `[keys.global]` section merged with the same semantics as `[keys.diff]`/`[keys.panel]`. |
| `src/ui/keymap_config_tests.rs` | Merge-semantics tests for `[keys.global]` (exact-keys replacement, `= []` unbind, unknown-action warning, collision → user wins) and `[keys.diff]` refresh-on-`r` remap. |
| `src/config/keys.rs` | `KeysConfig` gains the `global` field and the `"review-launcher"` entry in its hardcoded table-name list (parallel to `MODAL_MODE_NAMES` — layering note forbids importing `modal_keys`). |
| `src/config/keys_tests.rs` | Cross-check tests pinning the two parallel lists agree. |
| `src/ui/mod.rs` | `dispatch_key` (~340–490): scope-then-global resolution for `Normal`/`Visual` and `Panel` arms; new `Mode::ReviewLauncher` arm; remove `Mode::ReviewBranch` arm (~369) and its render hook (~830). |
| `src/ui/modes.rs` | New `handle_review_launcher_key`; remove `handle_review_branch_key` (~334). |
| `src/ui/app.rs` | `Mode::ReviewLauncher { tab, cursor, origin }` variant; generalize the `EndReviewOrigin` pattern (~161) for launcher origin restore; process-lifetime `last_launcher_tab` + commits-toggle memory (precedent: `last_panel_tab` ~395); remove `Mode::ReviewBranch` doc refs (~318). |
| `src/ui/modal_keys.rs` | New `REVIEW_LAUNCHER_KEYS` table (tab switch, cursor, Enter, Esc, `a` toggle); `ModalKeymaps` field; `MODAL_MODE_NAMES` gains `"review-launcher"` (~1973); remove `REVIEW_BRANCH_KEYS` (~825) and its `ModalKeymaps.review_branch` field; drift tests. |
| `src/ui/modal_keys_config.rs` / `src/ui/modal_keys_config_tests.rs` | Wire `[keys.review-launcher]` into the config-remappable modal set; cross-check test with `config::keys`. |
| `src/ui/review_launcher.rs` (new) | Launcher state (`LauncherTab`, cursor, origin, commits-toggle) + `App` handlers (open/close/tab/cursor/confirm per tab/in-session guard). |
| `src/ui/review_launcher_modal.rs` (new) | Launcher render, styled like `switcher_modal.rs`: centered overlay, tab headers, list, per-tab Enter-outcome footer hint, Commits empty-state line. |
| `src/ui/review_launcher_integration_tests.rs` (new) | Real-git launcher flows in canonicalized tempdir scratch repos: branch-review start parity, in-session guard, commit open/Esc restore, origin restore. |
| `src/ui/review_branch.rs` | Branch-list data path (`open_review_branch_modal` ~73) and `confirm_review_branch` (~146) reused by the Branches tab; handlers migrated into `review_launcher.rs`; file slimmed or folded in. |
| `src/ui/review_branch_modal.rs` | Deleted in task 3.0 (`Mode::ReviewBranch` retired, FR-9). |
| `src/ui/review_branch_integration_tests.rs` | Migrated to launcher-driven equivalents (parity evidence for FR-8/FR-9). |
| `src/ui/help.rs` | Trip hazard: scope-equality *filters* (`== Scope::Diff` ~318, `== Scope::Panel` ~345) compile fine while silently dropping `Global` rows — sweep them; add the "works everywhere" section and the launcher's `modal_sections` entry (~178); remove `OpenReviewBranch` from the action-group map (~68); extend bidirectional drift tests to `Scope::Global`. |
| `src/ui/footer.rs` | `Mode::ReviewLauncher` modal-hints arm; remove `Mode::ReviewBranch` arm (~435); migrated Global rows keep their `.footer()` hints. |
| `src/ui/refresh.rs`, `src/ui/annotation_list.rs`, `src/ui/git_panel.rs` | `Mode::ReviewBranch` match arms removed (refresh ~121/187, annotation_list ~25, git_panel ~543); `open_commit_view` (git_panel ~677) reused unchanged by the Commits tab. |
| `src/git/log.rs` | New typed `base..HEAD` log-range query (TDD) reusing `parse_commit_log`/`CommitLogEntry`; fixed-argv runner call, machine-readable format only. |
| `src/git/runner.rs` | Fixed-argv runner method for the range log if not colocated in `log.rs`; `GIT_TERMINAL_PROMPT=0` per precedent. |
| `src/ui/history.rs` | `InFlightHistory` single-flight + generation-guard precedent, reused/generalized for the launcher's lazy Commits load. |
| `src/ui/review_session.rs` | `resolve_review_base`, `ensure_review_worktree`, reconciliation — called unchanged by the Branches tab confirm flow (FR-8). |
| `src/ui/switcher.rs` / `src/ui/switcher_modal.rs` | Tab-switching and modal-layout parity precedent (`SwitcherTab`, `Tab`/`h`/`l`); reference only, no changes expected. |
| `src/ui/perf_tests.rs` | Must pass unmodified — no per-frame work proportional to history size. |
| `CLAUDE.md` / `README.md` | Docs-as-contract sweep in task 5.0: no stale "panel-only" review-branch statements; write ceilings unchanged. |
| `docs/specs/09-spec-review-launcher/proofs/` | Persisted journey transcripts and overlay captures (gitignored per repo convention). |

### Notes

- All four gates before every commit: `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`. Conventional commits; each parent task lands as one or a few self-contained commits; **refactors and behavior changes never share a commit** — task 1.0 and 2.0 sub-tasks call out the boundary explicitly.
- **Tempdir isolation is a first-class requirement** (2026-07-16 incident): every integration test builds repos with `tempfile`, canonicalizes paths (macOS `/var` symlink), pins git calls inside the tempdir, and uses the shared isolation-assertion helper (introduced in spec 08 task 1.5) before any mutating git call.
- TDD for pure code: the git-layer `base..HEAD` range query gets failing tests first, tests committed with the code.
- Agent guardrails: no worktree/fetch/pull/push/product-commit operations against the user's real repo. All worktree-exercising proofs (Journeys A/B/C) run in agent-created scratch repos in tempdirs; user-perspective captures against real repos are performed by the user.
- Drift tests are bidirectional: every documented key must observably act; every undocumented key must observably not. Extending them to `Scope::Global` and the launcher table is part of the definition of done, not polish.

## Tasks

### [x] 1.0 `Scope::Global` keymap scope: define global bindings once, dispatch scope-then-global, render a "works everywhere" help section

Covers: FR-1, FR-2, FR-3, FR-7 (global "works everywhere" help-section clause only — the launcher's own table lands in 2.0)

This is the scope-migration slice, deliberately separated from the launcher behavior change per the repo rule that refactors and behavior changes never share a commit: the `Scope::Global` variant + row migration (FR-1/FR-2) land as a behavior-preserving refactor commit(s); the `[keys.global]` config section (FR-3) and the new help section (FR-7 clause) land as their own commits on top. No `R` rebinding happens in this task — `R` still means refresh/review-branch until 2.0.

#### 1.0 Proof Artifact(s)

- Test: keymap unit tests demonstrate scope-then-global resolution order in both table-driven scopes (`Diff` and `Panel`) and that a scope-specific binding shadows a global one for the same key (FR-1).
- Test: behavior-pin tests demonstrate the migrated rows (`?`, `@`, `!`, `q`, `Q`/`Ctrl-C`) resolve to the same actions from both scopes before and after migration, including the review-session `q` → EndReview translation remaining in dispatch (FR-2).
- Test: config tests demonstrate `[keys.global]` merge semantics match `[keys.diff]`/`[keys.panel]` — exact-keys replacement, `= []` unbinds, unknown-action warning, same-scope collision → user wins (FR-3).
- Test: existing bidirectional help/dispatch drift tests pass with the new scope, demonstrating help stays truthful (FR-7 clause).
- CLI: `cargo run` screenshot/transcript of the `?` overlay showing global bindings rendered once in a distinct "works everywhere" section instead of repeated per scope, captured into `proofs/` (FR-7 clause).

#### 1.0 Tasks

- [x] 1.1 Write behavior-pin tests (green against current code) for the five cross-scope duplicated bindings — `?` ToggleHelp, `@` command log, `!` dismiss warning, `q` quit, `Q`/`Ctrl-C` quit-discard — asserting each resolves to the same action from `Scope::Diff` and `Scope::Panel`, extending the existing pins at `keymap.rs` ~1056–1082 and ~1585–1802. These are the before/after invariant for the migration.
- [x] 1.2 Add `Scope::Global` to the `Scope` enum (`keymap.rs` ~372) and implement scope-then-global resolution: in `lookup_in`, `resolve_in`, `starts_sequence_in`, `completions_for`, and `label_for`, a query in `Diff` or `Panel` consults the active scope's rows first, then `Global`; an active-scope row shadows a `Global` row for the same key. Unit-test resolution order and shadowing with synthetic bindings (FR-1). No rows move yet — behavior identical; this plus 1.3 is the refactor commit.
- [x] 1.3 Migrate the ten duplicated rows to five single `Global` rows: delete the diff rows (~604, 677, 682, 739, 744) and panel rows (~809, 821, 826, 833, 838), add `Global` equivalents preserving descriptions and `.footer()` hints. The review-session `q` → EndReview translation stays where it lives in dispatch, untouched (FR-2). The 1.1 pin tests and the full suite must pass unchanged. Commit as `refactor:` with the move-only invariant stated (identical resolution behavior, zero assertion edits in 1.1's pins).
- [x] 1.4 Sweep `help.rs` for the silent-drop trip hazard: the section builders use scope-equality *filters* (`b.scope == Scope::Diff` ~318, `== Scope::Panel` ~345), so `Global` rows vanish from help without a compile error. Add a "works everywhere" section rendered from `Scope::Global` bindings (placed before the per-scope sections), keep the per-scope sections free of duplicates, and verify `footer.rs` still renders the migrated rows' hints. Extend the bidirectional drift tests to cover `Scope::Global`: every Global binding appears in help, and every "works everywhere" entry dispatches from both scopes (FR-7 clause). Deviation: landed in the same commit as 1.2/1.3 rather than its own, because the row migration alone breaks pre-existing help/footer tests (the section is what keeps them green) — see commit message.
- [x] 1.5 Add the `[keys.global]` config section: `KeysConfig` in `src/config/keys.rs` gains a `global` field; `keymap_config.rs` applies it with the same merge semantics as `[keys.diff]`/`[keys.panel]`. Tests in `keymap_config_tests.rs`/`keys_tests.rs`: exact-keys replacement, `= []` unbinds, unknown-action warning, same-scope collision → user wins (FR-3). Separate `feat:` commit — this is the behavior change layered on the refactor.
- [x] 1.6 Run all four gates; capture the `?` overlay showing the "works everywhere" section into `docs/specs/09-spec-review-launcher/proofs/`; commit.

### [x] 2.0 Launcher shell: global `R` opens the tabbed Review launcher, `Esc` restores the exact origin, refresh moves to `r`

Covers: FR-4, FR-5, FR-6, FR-7 (launcher key table, footer hints, config-remappable set, launcher help section)

Note: after this task the launcher's tabs are navigable but `Enter` is inert (Branches confirm lands in 3.0, Commits data in 4.0) and the panel-`R` review-branch entry is already removed per FR-4 — an accepted intermediate state within this spec's sequence; the retired entry's replacement becomes functional in 3.0.

#### 2.0 Proof Artifact(s)

- Test: dispatch tests demonstrate `R` resolves to `open-review-launcher` from both `Diff` and `Panel` scopes via the Global row, diff-scope refresh now answers on `r`, and the panel-scope `R` review-branch row is gone; free-text modal contexts (compose, commit message, search, finder) still receive a literal `R` (FR-4).
- Test: `Mode::ReviewLauncher` state tests demonstrate `Tab`/`Shift-Tab`/`h`/`l` tab switching, `j`/`k`/arrow cursor movement, and `Esc` origin restore — `Normal`/`Visual` → `Normal`, `Panel` → `Panel` with cursor and tab intact, per the generalized `EndReviewOrigin` pattern (FR-5).
- Test: tab-memory test demonstrates first open lands on Branches and reopen lands on the last-used tab for the process lifetime (FR-6).
- Test: modal-table drift tests demonstrate the launcher table in `src/ui/modal_keys.rs` drives dispatch, footer hints, and its own `?` help section, and the `MODAL_MODE_NAMES`/`KeysConfig` parallel-list cross-check passes with the new remappable `[keys.review-launcher]` entry (FR-7).
- CLI: journey transcript — `R` from the diff view and from the git panel (cursor mid-list, History tab) opens the launcher; `Esc` returns to the exact prior focus with panel cursor/tab preserved — captured into `proofs/`, demonstrating origin restore (FR-5).

#### 2.0 Tasks

- [x] 2.1 Generalize the origin-restore pattern: introduce a launcher origin type modeled on `EndReviewOrigin` (`app.rs` ~161) — or generalize `EndReviewOrigin` itself if the payloads are identical — capturing `Normal`/`Visual`-family vs `Panel { cursor, tab }`. If this touches the existing EndReview flow it is a behavior-preserving `refactor:` commit, separate from the feature commits below.
- [x] 2.2 Add `Mode::ReviewLauncher { tab, cursor, origin }` and a new `src/ui/review_launcher.rs` with `LauncherTab { Branches, Commits }`, `App::open_review_launcher` / `close_review_launcher`, tab switching, cursor movement, and process-lifetime `last_launcher_tab` on `App` (precedent: `last_panel_tab` ~395; first open → Branches). Unit tests: open-from-`Normal`/`Visual`/`Panel`, `Esc` restores the exact origin (panel cursor + tab intact), tab memory across reopen (FR-5, FR-6).
- [x] 2.3 Create `REVIEW_LAUNCHER_KEYS` in `modal_keys.rs` (`Tab`/`Shift-Tab`/`h`/`l` switch tab, `j`/`k`/arrows move cursor, `Enter` confirm, `Esc` close — parity with `SWITCHER_KEYS`), wire `modes::handle_review_launcher_key` through it, add the `mod.rs` dispatch arm, the `footer.rs` hints arm, and the launcher section in `help.rs::modal_sections`. Bidirectional drift tests for the new table.
- [x] 2.4 Trip-hazard sub-task — remappable-set parallel lists: add the `review_launcher` field to `ModalKeymaps`, `"review-launcher"` to `MODAL_MODE_NAMES` (~1973), the matching entry in `KeysConfig::from_value`'s hardcoded list in `src/config/keys.rs` (it cannot import `modal_keys` — layering note), and the `modal_keys_config.rs` override wiring. The cross-check test pinning the two lists must pass; add a config test remapping a launcher key via `[keys.review-launcher]` (FR-7).
- [x] 2.5 Rebind (single `feat:` commit): add the `OpenReviewLauncher` action with kebab-case `action_name` round-trip (`"open-review-launcher"`); bind `R` in `Scope::Global`; move diff-scope `Refresh` from `R` (~687) to `r` (verified free — only the `gr` two-key sequence uses `r` today); delete the panel-scope `R` `OpenReviewBranch` row (~816) and retire the `OpenReviewBranch` action from the keymap table (the modal itself dies in 3.0). Add a config test for `[keys.global]` remap of `open-review-launcher` and `[keys.diff]` remap of refresh on `r` (FR-3/FR-4 interplay).
- [x] 2.6 Trip-hazard sub-task — rewrite the stale test pins: `keymap.rs` ~1128/1132 (`R` ± SHIFT → `Refresh`) and ~1136 (`r` → `None`) now assert the new bindings; verify the `action_names_are_total_and_bijective` test (~1821) covers the new action. Add free-text-context tests proving `R` still inserts a literal character in compose, commit-message, search, and finder modes (they bypass the keymap table) (FR-4).
- [x] 2.7 Render `src/ui/review_launcher_modal.rs` styled like `switcher_modal.rs`: centered overlay, tab headers with active-tab highlight, list area (placeholder content until 3.0/4.0), footer hints from the effective launcher table, and a per-tab Enter-outcome hint line ("start branch review" vs "review commit (read-only)") per the spec's Design Considerations.
- [x] 2.8 Run gates; capture the journey transcript — `R` from the diff view and from the git panel (cursor mid-list on a non-default tab), `Esc` restores the exact prior focus — into `proofs/`; commit.

### [x] 3.0 Branches tab: migrate branch review into the launcher, retire `Mode::ReviewBranch`, block in-session starts with a hint

Covers: FR-8, FR-9, FR-10

#### 3.0 Proof Artifact(s)

- Test: migration-parity tests demonstrate the Branches tab's list contents (local branches excluding current) and confirm-flow calls (single-in-flight guard, `ensure_review_worktree`, review-state reconciliation, re-root onto `DiffTarget::Review` with the auto-resolved base) match the pre-migration `review_branch` modal (FR-8).
- Test: removal tests demonstrate `Mode::ReviewBranch` and its panel-only entry are gone, the launcher is the sole in-app entry, successful start lands in the re-rooted review view, and close-without-start restores the invocation origin (FR-9).
- Test: in-session block test demonstrates `Enter` on the Branches tab during an active review session emits the finish-or-pause status hint and mutates nothing — no worktree, review-state, or mode change (FR-10).
- CLI: journey transcript on a scratch tempdir repo — from the diff view, `R`, `j`, `Enter` lands in a worktree-backed review session of the chosen branch (≤3 keystrokes, Journey B), and the same flow launched from the git panel — captured into `proofs/` (FR-8, FR-9).

#### 3.0 Tasks

- [x] 3.1 Wire the Branches tab's data to the existing branch-list path used by `open_review_branch_modal` (`review_branch.rs` ~73): populate on launcher open / tab switch, local branches excluding the current one. Parity test asserting the launcher's list equals the pre-migration modal's list on the same fixture repo (FR-8).
- [x] 3.2 Wire `Enter` on the Branches tab to the existing confirm flow (`confirm_review_branch`, ~146) unchanged: single-in-flight guard, `resolve_review_base` (`origin/HEAD` → `main` → `master`), `ensure_review_worktree`, review-state reconciliation, re-root onto `DiffTarget::Review`. Successful start lands in the re-rooted review view exactly as today; failure surfaces the same status message with the launcher's origin restored. Parity tests over the call sequence and the in-flight guard against tempdir fixture repos (FR-8).
- [x] 3.3 In-session guard (FR-10): when a branch-review session is active, the Branches tab stays visible but `Enter` emits the finish-or-pause status hint (existing status-message line, naming `q` → EndReview) and performs no mutation. Test: active session + `Enter` → hint set, mode unchanged, no worktree or review-state calls made (assert via the operations seam).
- [x] 3.4 Retire `Mode::ReviewBranch` (FR-9): delete `review_branch_modal.rs`, `modes::handle_review_branch_key` (~334), the `mod.rs` arms (~369, ~830), the `footer.rs` arm (~435), the mode-match mentions in `refresh.rs` (~121/187), `annotation_list.rs` (~25), `git_panel.rs` (~543), the `REVIEW_BRANCH_KEYS` table + `ModalKeymaps.review_branch` field, and the `OpenReviewBranch` entry in `help.rs`'s action-group map (~68). Fold the surviving handlers from `review_branch.rs` into `review_launcher.rs` (or slim the file to the shared data path). The compiler's exhaustive-match errors are the checklist — chase every one. Note in the commit that the non-remappable-modal debt drops from four tables to three.
- [x] 3.5 Migrate `review_branch_integration_tests.rs` to launcher-driven equivalents in `review_launcher_integration_tests.rs`: reroot-into-review happy path, failure path, and close-without-start origin restore from both `Normal` and `Panel` origins (FR-9/FR-5). Tempdir isolation helper on every mutating test.
- [x] 3.6 Run gates; capture Journey B transcript on an agent-created scratch tempdir repo — `R`, `j`, `Enter` from the diff view lands in a review session (≤3 keystrokes), then the same flow launched from the git panel — into `proofs/`; commit.

### [x] 4.0 Commits tab: ahead-of-base list with an all-commits toggle, `Enter` opens the read-only commit view

Covers: FR-11, FR-12, FR-13, FR-14

#### 4.0 Proof Artifact(s)

- Test: TDD git-layer unit tests on tempdir fixture repos demonstrate the new typed `base..HEAD` log-range query — correct listing, newest-first ordering, and empty-range behavior — parsed from machine-readable output alongside the existing history loader (FR-11).
- Test: UI tests demonstrate lazy off-render-loop loading with the History tab's generation-guard discipline, the `a` toggle switching between ahead-of-base and recent-HEAD-log sources, toggle+tab state persisting across reopen for the process lifetime, and the empty state rendering a line that names the toggle key (FR-11, FR-12, FR-13).
- Test: integration test demonstrates `Enter` opens the selected commit via the existing `open_commit_view` (commit vs first parent, staging read-only) and works during an active branch-review session, with `Esc` restoring the suspended prior view (FR-14).
- CLI: journey transcript on a scratch tempdir repo — fresh commit made, then `R`, `Enter` shows that commit's diff and `Esc` returns to the prior view (2 keystrokes, Journey A) — captured into `proofs/` (FR-11, FR-14).
- CLI: journey transcript on the base branch with zero ahead commits — Commits tab shows the empty-state hint and `a` expands to the full log (Journey C) — captured into `proofs/` (FR-12, FR-13).

#### 4.0 Tasks

- [x] 4.1 TDD the git-layer range query (failing tests first, committed with the code): a typed function in `src/git/log.rs` taking a closed range type (base + head refs, never interpolated strings) and returning `Vec<CommitLogEntry>` via `parse_commit_log`, newest first; runner call with fixed argv and the existing machine-readable log format, `GIT_TERMINAL_PROMPT=0`. Fixture tempdir repos cover: commits ahead of base listed newest-first, empty range (branch == base) → empty vec, base resolution handled by the caller. Isolation-assertion helper on every fixture.
- [x] 4.2 Lazy loading off the render loop: opening the Commits tab (or switching to it) kicks a background load reusing the `InFlightHistory` single-flight + generation-guard discipline (`history.rs`, `app.rs` ~379/~416); results drain via the existing non-blocking poll; a stale generation's result is dropped, not applied. Tests: single-flight (second open while in flight doesn't spawn), stale-drop on generation bump (FR-11).
- [x] 4.3 Commits tab render in `review_launcher_modal.rs`: list `CommitLogEntry` rows (subject + short sha + relative time, matching History-tab row style), cursor starting on the newest; loading placeholder while in flight; when the filtered list is empty, render the empty-state line that names the effective toggle key (read from the effective launcher table, not hardcoded, so remaps stay truthful) (FR-13). Render tests for populated, loading, and empty states.
- [x] 4.4 The `a` "all commits" toggle: add the row to `REVIEW_LAUNCHER_KEYS` (config-remappable like the rest of the table), toggling the tab's data source between ahead-of-base (4.1) and the recent-HEAD-log source the History tab uses; toggle state is remembered alongside `last_launcher_tab` for the process lifetime. Tests: toggle switches sources, state survives close/reopen, drift tests keep table/footer/help in sync (FR-12).
- [x] 4.5 `Enter` opens the selected commit via the existing `open_commit_view` (`git_panel.rs` ~677) — commit vs first parent, staging read-only, `Esc` restores the suspended prior view — including during an active branch-review session (the read-only peek ratified in questions round 1). Integration tests for both contexts in tempdir repos (FR-14).
- [x] 4.6 Run gates; capture Journey A (fresh commit on a scratch repo, `R`, `Enter`, `Esc` back — 2 keystrokes) and Journey C (on the base branch: empty-state hint, `a` expands) transcripts into `proofs/`; commit.

### [x] 5.0 Journey evidence, drift/perf verification, and docs sync: the launcher holds up end-to-end

Covers: Success Metrics 1–4 (Journeys A/B/C persisted, zero drift); end-to-end re-verification of FR-4..FR-14 from the user's perspective; no new FRs

#### 5.0 Proof Artifact(s)

- CLI: consolidated journey transcripts A (2-keystroke commit review), B (≤3-keystroke branch review from diff view and panel), and C (empty-state → `a` expand), persisted in `docs/specs/09-spec-review-launcher/proofs/`, demonstrating the spec's Success Metrics as user journeys with evidence (Metrics 1–3).
- Test: full gate run — `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` — plus all keymap/help/footer drift tests green, demonstrating zero drift across the new scope, rebinds, and launcher table (Metric 4).
- Test: `src/ui/perf_tests.rs` tripwires pass unmodified, demonstrating the launcher adds no per-frame work proportional to repo history size (spec Repository Standards).
- CLI: `?` overlay capture showing both the "works everywhere" section and the launcher section, and a docs diff (if any) showing CLAUDE.md/README statements still track shipped behavior (docs-as-contract; e.g., the retired review-branch modal no longer described as panel-only), captured into `proofs/` (Metric 4).

#### 5.0 Tasks

- [x] 5.1 Re-capture the three journeys end-to-end on the finished build (not the per-task interim captures): Journey A (fresh commit → `R`, `Enter`, `Esc` — 2 keystrokes), Journey B (`R`, move, `Enter` from both the diff view and the git panel — ≤3 keystrokes), Journey C (base branch → empty-state hint → `a` expands). Agent-created scratch tempdir repos only; persist the transcripts under `docs/specs/09-spec-review-launcher/proofs/` with a one-line index naming which metric each satisfies.
- [x] 5.2 Verification sweep: run all four gates plus the full drift-test suite (keymap ↔ help ↔ footer, modal-table cross-checks, `action_name` bijectivity) and confirm `src/ui/perf_tests.rs` passes unmodified; record the command outputs into `proofs/`.
- [x] 5.3 Docs-as-contract sweep: grep CLAUDE.md, README.md, and spec-08 docs for statements the launcher invalidates (panel-only `R` review-branch entry, refresh-on-`R`, the four-non-remappable-modals count) and fix any stale claims; write ceilings are untouched by this spec — verify no edit is needed there. `docs:` commit if changes are required. Deviation: sweep found no stale claims in CLAUDE.md/README.md (neither names a specific keybind or a modal-table count), so no `docs:` commit was needed — see `proofs/09-task-05-proofs.md`.
- [x] 5.4 Coverage cross-check: walk FR-1..FR-14 against the landed tests and transcripts, confirm each has its promised artifact, capture the `?` overlay showing the "works everywhere" and launcher sections, and record the FR-12 default-filtered-with-expand behavior as the open dogfood experiment for follow-up review (spec Open Question 1). Final commit.
