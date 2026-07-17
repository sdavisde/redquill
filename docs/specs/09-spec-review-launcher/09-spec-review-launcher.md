# 09-spec-review-launcher.md

## Introduction/Overview

Today, starting a review requires navigating into the git panel first: branch review lives behind a panel-only `R` binding, and reviewing a commit takes four steps (open panel, switch to History, move cursor, Enter). This spec adds a global **Review launcher**: pressing `R` from anywhere opens a tabbed modal (**Branches** | **Commits**) from which the user starts a branch review or opens a commit's diff, then returns exactly where they were. It also introduces a real `Global` keymap scope so "works everywhere" keys are one table row instead of duplicated per-scope rows.

The guiding principle: navigation keys stay contextual, but *launcher* actions (starting a review) must not be location-gated.

## Goals

- Start reviewing the most recent commit from anywhere in 2 keystrokes (`R`, `Enter`), and a branch review in 3 (`R`, move, `Enter`).
- Replace the panel-only branch-review entry with a single global entry point that also hosts commit review, and is shaped to accept a future Pull Requests tab without changing the entry point.
- Introduce `Scope::Global` in the keymap so global bindings are defined once, dispatched in both table-driven scopes, and rendered as a distinct "works everywhere" help section.
- Preserve the existing review machinery unchanged beneath the new entry point: the worktree-backed branch-review session flow and the read-only, Esc-restorable commit view.
- Keep the keymap, footer hints, and `?` help overlay in drift-tested sync, and ship the new modal's key table config-remappable from day one.

## User Stories

- **As a developer whose coding agent just committed**, I want to open that commit's diff with one keybind from the diff view so that I can review the change without navigating panel → History → commit.
- **As a reviewer**, I want one memorable key that starts any review so that I don't have to remember which panel and tab a given review type lives in.
- **As a new user**, I want the app's core review workflows presented together in one launcher so that discovering one key teaches me what the tool can do.
- **As a user mid-branch-review**, I want to peek at an individual commit without disturbing my session so that I can check context and return safely.
- **As a keymap customizer**, I want the launcher's bindings (global key and in-modal keys) remappable through the existing config layer so that the new surface follows the same rules as every other mode.

## Demoable Units of Work

### Unit 1: Global keymap scope, `R`/`r` rebind, and the launcher shell

**Purpose:** Establish the `Global` scope, move the affected bindings, and ship the empty-but-navigable tabbed modal with origin restore. Serves everyone by making "works everywhere" keys a first-class concept.

**Functional Requirements:**

- FR-1: The keymap `Scope` enum shall gain a `Global` variant. During key dispatch in the table-driven modes (`Normal`/`Visual` diff scope and `Panel` scope), a key shall resolve first against the active scope's bindings, then against `Global` bindings; a scope-specific binding shadows a global one for the same key.
- FR-2: The existing cross-scope duplicated rows (`?` help, `@` command log, `!` dismiss warning, `q` quit, `Q`/`Ctrl-C` quit-discard) shall migrate to single `Global` rows with no observable behavior change (including the review-session `q` → EndReview translation, which remains in dispatch).
- FR-3: The keymap config shall accept a `[keys.global]` section with the same merge semantics as `[keys.diff]`/`[keys.panel]` (exact-keys replacement, `= []` unbinds, unknown-action warning, same-scope collision → user wins).
- FR-4: `R` shall be bound in `Global` scope to a new `open-review-launcher` action. The diff-scope `R` (refresh) shall rebind to `r`, and the panel-scope `R` (open review-branch modal) row shall be removed. Modal free-text contexts (compose, commit message, search, finder) are unaffected: they do not consult the keymap table, so `R` still types a literal character there.
- FR-5: A new `Mode::ReviewLauncher` shall carry its tab, cursor, and an origin field. `Tab`/`Shift-Tab` and `h`/`l` shall switch tabs (parity with the Switcher modal); `j`/`k` and arrow keys shall move the cursor; `Esc` shall close the modal and restore the invocation origin exactly — `Normal`/`Visual` returns to `Normal`; `Panel` returns to `Panel` with its cursor and tab intact (the `EndReviewOrigin` pattern).
- FR-6: The launcher shall remember its last-used tab for the lifetime of the process and reopen on it; the first open of a session shall show the Branches tab.
- FR-7: The launcher's key table shall live in `src/ui/modal_keys.rs`, drive both dispatch and footer hints, be included in the config-remappable modal set, and appear as its own section in the `?` help overlay. The help overlay shall render `Global` bindings as a distinct "works everywhere" section instead of repeating them per scope.

**Proof Artifacts:**

- Test: keymap unit tests demonstrate scope-then-global resolution order and shadowing.
- Test: existing bidirectional help/dispatch drift tests pass with the new scope and modal table, demonstrating help, footer, and dispatch stay in sync.
- CLI: `cargo run` session — `R` from the diff view and from the git panel opens the launcher; `Esc` returns to the exact prior focus (panel cursor/tab preserved) — captured as a journey transcript, demonstrating origin restore.
- Test: config test demonstrates `[keys.global]` remapping of `open-review-launcher` and `[keys.diff]` remapping of refresh on `r`.

### Unit 2: Branches tab (migrate the existing branch-review modal)

**Purpose:** Make branch review reachable from anywhere by hosting the existing spec-08 flow in the launcher, and retire the panel-coupled modal. Serves reviewers starting worktree-backed review sessions.

**Functional Requirements:**

- FR-8: The Branches tab shall list local branches excluding the current one (identical to the current review-branch modal), and `Enter` shall start a branch review through the existing unchanged flow: single-in-flight guard, `ensure_review_worktree`, review-state reconciliation, and re-root onto `DiffTarget::Review` with base auto-resolved (`origin/HEAD` → `main` → `master`).
- FR-9: `Mode::ReviewBranch` and its panel-only entry shall be removed; the launcher is the sole in-app entry point. Closing after a successful review start behaves as today (the re-rooted review view), and closing without starting restores the invocation origin per FR-5.
- FR-10: While a branch-review session is active, the Branches tab shall remain visible but `Enter` shall not start a new review; it shall show a status message directing the user to finish or pause first (`q` → the existing EndReview modal). No implicit pause, finish, or state write occurs.

**Proof Artifacts:**

- Test: migration-parity tests demonstrate the branch list contents and confirm-flow calls match the pre-migration modal (including the in-flight guard).
- Test: in-session block test demonstrates `Enter` on the Branches tab during an active session emits the hint and mutates nothing.
- CLI: journey transcript — from the diff view of a scratch repo, `R`, `j`, `Enter` lands in a review session of the chosen branch, demonstrating the ≤3-keystroke branch-review journey. (Per repo guardrails, worktree flows are exercised only against agent-created scratch repos in a tempdir.)

### Unit 3: Commits tab

**Purpose:** Make "review a commit that just landed" a 2-keystroke action. Serves developers reviewing agent-made or recent commits.

**Functional Requirements:**

- FR-11: The Commits tab shall list, by default, commits ahead of the auto-resolved base (`base..HEAD`), newest first, cursor starting on the newest. The git layer shall gain a typed, machine-parsed log-range query alongside the existing history loader; loading is lazy and off the render loop with the same generation-guard discipline as the History tab.
- FR-12: A key in the launcher's table (default `a`, "all commits") shall toggle the list between ahead-of-base and the recent-HEAD-log source the History tab uses. The toggle state is remembered with the tab for the process lifetime. This default-filtered-with-expand behavior is an explicit dogfood experiment (questions round 1).
- FR-13: When the filtered list is empty (e.g., the current branch is the base itself), the tab shall render an empty-state line that names the toggle key rather than showing a blank list.
- FR-14: `Enter` shall open the selected commit in the existing read-only single-commit view (`open_commit_view`: commit vs first parent, staging read-only, `Esc` restores the suspended prior view). This works both in normal operation and during an active branch-review session.

**Proof Artifacts:**

- Test: git-layer unit tests (TDD, per repo convention for parsers) demonstrate correct `base..HEAD` listing, ordering, and empty-range behavior on fixture repos in tempdirs.
- Test: UI tests demonstrate the toggle switches data sources, state persists across reopen, and the empty state renders with the toggle hint.
- CLI: journey transcript — commit made on a scratch repo, then `R`, `Enter` shows that commit's diff and `Esc` returns to the prior view, demonstrating the 2-keystroke commit-review journey.

## Non-Goals (Out of Scope)

1. **Pull Requests / MR tab**: forge integration is a future spec. No placeholder or disabled third tab ships — no dead UI.
2. **Arbitrary range entry**: typing `X..Y` to re-target the diff stays deferred (spec 05 / spec 08 follow-up); this spec adds no range view.
3. **Commit-range review from the list**: `Enter` opens exactly one commit; no multi-select or `commit..HEAD` action.
4. **Help overlay redesign and which-key popups**: that is spec 10. This spec only adds the launcher's help section and the global-scope help section.
5. **Review-session semantics changes**: worktree lifecycle, tri-state persistence, blob-SHA reconciliation, and the EndReview flow are untouched.
6. **Command palette**: explicitly deferred; revisit only if the launcher plus spec 10 prove insufficient.

## Design Considerations

- Modal layout follows the Switcher modal: centered overlay, tab headers with the active tab highlighted, list below, footer hints from the modal key table.
- The two tabs deliberately have different weights — Commits opens a lightweight read-only peek; Branches starts a full worktree session. Each tab's footer hint line should make the outcome of `Enter` unambiguous (e.g., "review commit (read-only)" vs "start branch review").
- The in-session Branches block (FR-10) surfaces through the existing status-message line, consistent with other guarded operations.

## Repository Standards

- Keymap and modal keys are data: new bindings go in `src/ui/keymap.rs` / `src/ui/modal_keys.rs` shared tables, never loose match arms; every action gets a stable kebab-case `action_name` round-trip; drift tests must keep passing.
- Rust best practices per `docs/rust-best-practices.md`: no `unwrap`/`expect` in production code, typed errors in `git/`, background work off the render loop with generation guards, subprocess argv from closed types, machine-readable git output only.
- TDD for the pure git-layer range-log parsing; integration tests in canonicalized tempdirs only (never the host repo — see the 2026-07-16 tempdir-leak incident).
- All four gates before any commit: `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`. Conventional commits; refactor (scope migration) and behavior change (launcher) in separate commits.
- Perf tripwires in `src/ui/perf_tests.rs` must stay green; the launcher must not add per-frame work proportional to repo history size.

## Technical Considerations

- **Dispatch order** (FR-1): active scope first, then `Global`, is a hard rule so existing scope-specific keys can never be silently stolen by a global binding — and vice versa gives config users a predictable shadowing story.
- **Origin restore**: generalize the existing `EndReviewOrigin` pattern rather than inventing a parallel one; `after_panel_coherence` is already a no-op outside `Mode::Panel`, so no panel coupling remains.
- **Reuse over rebuild**: Branches tab wraps the existing `open_review_branch_modal` data path and `confirm_review_branch`; Commits tab reuses `CommitLogEntry` and `open_commit_view`. The only genuinely new machinery is the `base..HEAD` log query and the modal shell.
- **Remappability debt**: the retired review-branch modal's table was one of the four non-remappable ones; its replacement ships in the remappable set, reducing that debt rather than growing it.
- No new dependencies. No latest-standards research was needed — internal TUI design on the established in-repo stack.

## Security Considerations

No new security surface. The launcher triggers only already-sanctioned operations (worktree add/remove under the managed path via the existing session flow; read-only log/diff queries) with fixed argv from closed types. Agent-side testing of worktree flows happens only in scratch tempdir repos per the repo guardrails. Proof artifacts contain no secrets.

## Success Metrics

Per this repo's UX-outcome verification convention, metrics are user journeys with persisted evidence, not test counts alone:

1. **Journey A (commit review)**: from the diff view of a scratch repo with a fresh commit, `R` + `Enter` displays that commit's diff and `Esc` returns to the exact prior view — 2 keystrokes, transcript persisted as a proof artifact.
2. **Journey B (branch review)**: from anywhere (diff view and panel both), `R` + cursor + `Enter` starts a worktree review session — ≤3 keystrokes, transcript persisted.
3. **Journey C (no dead ends)**: on the base branch with zero ahead commits, the Commits tab shows the empty-state hint and `a` expands to the full log — persisted.
4. **Zero drift**: all keymap/help/footer drift tests and all four cargo gates pass; the `?` overlay shows the "works everywhere" section and the launcher section.

## Open Questions

1. **Auto-expand on empty?** When the ahead-of-base list is empty, should the Commits tab auto-toggle to all-commits instead of showing the empty-state hint? Shipping hint-first per the dogfood decision ("let's try it and we'll see"); revisit after use. Non-blocking — FR-13 is definite for implementation.
2. **Session-persisted tab/toggle memory**: tab and filter memory are process-lifetime only (FR-6, FR-12). Persisting them across runs is a possible follow-up once the config layer (spec 07) settles; assumed out for now.
