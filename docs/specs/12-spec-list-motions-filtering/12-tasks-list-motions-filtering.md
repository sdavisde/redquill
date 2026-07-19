# 12-tasks-list-motions-filtering.md

Task list for `12-spec-list-motions-filtering.md`. Generated in batch mode (parent tasks + sub-tasks approved upfront by the user's batch-mode invocation).

## Relevant Files

| File | Why It Is Relevant |
| --- | --- |
| `src/ui/motion.rs` (new) | Shared motion layer: motion-set data + pure count parsing/repeat semantics (FR-1); unit-testable without the app. |
| `src/ui/mod.rs` | `dispatch_key` hosts count-digit interception in the `Normal`/`Visual` arm (lines ~400–417); relocation target for FR-3; event loop threads `pending_count`. |
| `src/ui/keymap.rs` | Keymap tables and `Action` enum; motion set defined alongside; `Scope::Panel` gains full motion bindings. |
| `src/ui/modes.rs` | `handle_panel_key` and every modal handler (`handle_list_key`, `handle_staging_key`, `handle_switcher_key`, `handle_peek_key`, `handle_review_launcher_key`) route through the shared layer. |
| `src/ui/git_panel.rs` | Panel cursor/row-count/clamp (`panel_row_count`, `moved_cursor`); `maybe_prefetch_history` must fire on layer-driven moves (FR-3). |
| `src/ui/app.rs` | Aggregate-root wiring for new nav methods and filter state per list. |
| `src/ui/list_filter.rs` (new) | Reusable `/` filter component: query state, fuzzy delegation to `search::rank`, lock/clear/empty-state semantics (FR-7, FR-9). |
| `src/ui/modal_keys.rs` | Shared modal-key tables gain motion rows and filter-mode rows with footer hints; drift tests live here (FR-4, FR-10, FR-13). |
| `src/ui/annotation_list.rs` | Adopts motion layer + filter (FR-4, FR-8). |
| `src/ui/staging.rs` | Staging + accepted panels adopt motion layer + filter; verbs act on filtered selection (FR-4, FR-8). |
| `src/ui/switcher.rs` | Both switcher tabs adopt motion layer + filter (FR-4, FR-8). |
| `src/ui/review_launcher.rs` | Launcher tabs adopt both mechanisms; filtered `Enter` obeys the in-session guard (FR-12, FR-13). |
| `src/ui/code_intel.rs` | LSP peek reconciled onto the layer where behavior-identical (FR-4). |
| `src/ui/help.rs` | Help paging reconciled where behavior-identical; help `/` filter reconciled or divergence documented (FR-4, FR-11); keymap↔help drift coverage. |
| `src/ui/footer.rs` | Footer hints for motions and filter mode; active-filter indicator styling reuse (FR-6, FR-9, FR-10). |
| `src/ui/modal_keys_config.rs` + `src/ui/modal_keys_config_tests.rs` | Config-remap registration and drift coverage for new rows (FR-6, FR-10, FR-13). |
| `src/ui/diff_view_state.rs` | Motion implementations the diff view keeps; consumed through the layer with behavior preserved (FR-2). |
| `src/ui/perf_tests.rs` | Cursor/hunk-nav tripwires must stay green; new 5k-row filter budget test. |
| `src/ui/mod_tests.rs`, `src/ui/git_panel_tests.rs`, `src/ui/footer_tests.rs`, `src/ui/review_launcher_integration_tests.rs` | Behavior-preservation, drift, and integration tests (FR-2, FR-5, FR-8, FR-12). |
| `docs/specs/12-spec-list-motions-filtering/12-proofs/` (new) | Journey transcripts and per-task proof records. |

### Notes

- Test commands: `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` — all four gates before every commit.
- Unit tests inline (`#[cfg(test)]`) or in sibling `*_tests.rs` files wired via `#[path]` per existing convention; integration tests use tempdir scratch repos only.
- Refactor commits (diff-view migration, count relocation, per-list `j`/`k` replacement) strictly separate from behavior commits (new motions in new places, filter mode).
- The fuzzy matcher (`src/search/fuzzy.rs::rank`) is already a shared pure module — no extraction refactor needed; the filter component delegates to it.
- The `g`/`z` pending-prefix machine and spec 10's which-key footer are NOT moving; count relocation must preserve their observable behavior (count survives the `g` of `3gg`, Esc cancels a count, pending-prefix footer strips unchanged).

## Tasks

### [x] 1.0 Move through every list with the diff view's vim motions

A user can open the git panel (or any list: annotations, staging, accepted, switcher, LSP peek) and page with `Ctrl-d`/`Ctrl-u`/`Ctrl-f`/`Ctrl-b`, jump with `gg`/`G`, and prefix counts like `3j` — exactly the gestures the diff view already has — while the diff view itself behaves precisely as before. Covers FR-1..FR-6.

#### 1.0 Proof Artifact(s)

- Test: `src/ui/motion.rs` unit tests pass, demonstrating count parsing and each motion's semantics as pure functions (FR-1).
- Test: pre-existing diff-view motion tests pass unchanged post-refactor (zero assertion edits, identical test count), demonstrating behavior preservation (FR-2).
- Test: motion coverage drift test enumerates every consuming context × the complete motion set; a negative-proof test shows a dummy motion added to the set fails coverage (FR-5).
- Test: git panel tests demonstrate paging/jump clamping to `panel_row_count` and History lazy prefetch firing on layer-driven moves (FR-3).
- CLI: journey transcript `12-proofs/12-journey-a-big-changeset.txt` — 200-file scratch repo: git panel `Ctrl-d` pages, `G` jumps to bottom, `3j` steps three; same gestures in the annotation list (FR-3, FR-4).
- CLI: `cargo test` output showing perf tripwires (`cursor_navigation_is_bounded`, `hunk_navigation_is_bounded`) still green.

#### 1.0 Tasks

- [x] 1.1 TDD: create `src/ui/motion.rs` — the shared motion set as data (cursor step down/up, half-page down/up, full-page down/up, jump-to-top, jump-to-bottom), each accepting an optional count, plus pure count-accumulation logic (digits, `MAX_COUNT` cap, leading-zero rule, cancel semantics) relocated from `dispatch_key`'s constants. Failing tests first; no app types.
- [x] 1.2 Refactor (behavior-preserving, own commit): route diff-view motion dispatch through the shared layer — same keys, same semantics, same count handling. Move-only invariant: motion-related tests pass unchanged, identical test counts, zero assertion edits; state the invariant in the commit message (FR-2).
- [x] 1.3 Refactor (own commit): relocate count-digit interception out of the `Normal`/`Visual`-only arm of `dispatch_key` (`src/ui/mod.rs` ~400–417) into the shared layer so counts are available to every consuming context. Preserve: count survives `just_started_sequence` (`3gg`), Esc cancels an in-progress count, bare `0` falls through to `CursorLineStart` when no count is pending, the `g`/`z` pending-prefix machine and which-key footer are untouched (existing `footer_tests.rs` pending-prefix tests pass unchanged) (FR-3).
- [x] 1.4 Feat: git panel (both tabs) consumes the layer — add panel-scope motion bindings (half/full page, counts) clamped against `panel_row_count()`; the History tab's `maybe_prefetch_history` triggers on layer-driven moves exactly as on `j`/`k`; motions compose with spec 11's panel verbs in `handle_panel_key` (FR-3). **Deviation**: jump-to-top binds a single `g`/`Home`, not the diff view's two-key `gg` — panel scope's keymap *could* support a two-key sequence, but every other non-diff context (below) genuinely cannot (see 1.5's deviation note), so panel scope follows the same single-`g` convention for consistency across the whole non-diff surface rather than being the one context with a different jump-to-top gesture. Documented in `motion.rs`'s module doc.
- [x] 1.5 Feat: modal list contexts consume the layer — annotation list, staging panel, accepted panel, switcher (both tabs), LSP peek replace hand-rolled `j`/`k` with layer dispatch; help-overlay paging keys reconciled onto the layer only where behavior-identical, divergence documented in the module doc otherwise (FR-4). **Deviation**: modal-key tables (`ModalBinding`) have no two-key-sequence support (a real, pre-existing structural limit — every row matches exactly one physical key), so jump-to-top uses a single `g` (plus `Home`) instead of `gg`, mirroring the precedent the help overlay's own scroll keys already set (`modal_keys::HELP_KEYS`). Help-overlay paging itself was *not* reconciled onto the shared layer — its `Cell<u16>`-plus-render-side-clamp scroll model is structurally unlike every other context's clamped-cursor-against-a-known-length model; the divergence is documented in `help.rs`'s module doc rather than forcing a restructure.
- [x] 1.6 Test: motion coverage drift test — enumerate every consuming context (diff view, git panel, annotation list, staging, accepted, switcher×2, peek) and assert each dispatches the complete motion set; add the dummy-motion negative proof (add a motion in a test, assert coverage fails) (FR-5). Folded into 1.5's commit per this task list's explicit TDD allowance.
- [x] 1.7 Help/footer/config coverage: all motion keys visible per context via the shared-table rendering (`?` help + footer hints), config-remappable wherever the host table already is; keymap↔help and footer drift tests extended and green; perf tripwires unchanged (FR-6).
- [x] 1.8 Proof: build a 200-file scratch repo (tempdir), capture journey transcript A into `12-proofs/`, record all-gates output in `12-proofs/12-task-01-proofs.md`. Both files are gitignored per the repo's `docs/specs/*/*-proofs/` convention (kept local, not committed) — the generating test (`big_changeset_motion_journey_transcript` in `src/ui/git_panel_tests.rs`) is committed and regression-tested.

### [x] 2.0 Narrow any list by pressing `/` and typing a few characters

A user in the annotation list, staging panel, accepted panel, or switcher presses `/`, types a fragment, and the list fuzzily narrows as they type; `Esc` clears and exits, `Enter` locks the filter so the list's normal verbs (`e`, `d`, `Space`, `Enter`) act on the narrowed rows; an on-screen indicator shows the active query, and an empty result shows a hint instead of a blank list. Covers FR-7..FR-11.

#### 2.0 Proof Artifact(s)

- Test: `src/ui/list_filter.rs` unit tests demonstrate query editing, fuzzy ranking via the finder's matcher, `Esc` clear / `Enter` lock semantics, and empty-state (FR-7, FR-9).
- Test: per-context integration tests demonstrate filter + motion + verb composition — filter the staging panel, `j`, `Space` unstages the correct filtered entry (FR-8).
- Test: modal-key drift tests cover the filter-mode rows in every gaining table; no-shadow assertions verify `/` collides with nothing existing per table (FR-10).
- Test: new perf budget test — filtering a 5k-row candidate list stays within budget (render loop never stalls).
- CLI: journey transcript `12-proofs/12-journey-b-find-annotation.txt` — 30 annotations, `/` + three characters narrows, `Enter` locks, `e` edits the right one (FR-7, FR-8).

#### 2.0 Tasks

- [x] 2.1 TDD: create `src/ui/list_filter.rs` — reusable filter component: `/` enters filter mode, printable chars build the query, matching/ranking delegates to `crate::search::rank` (fuzzy, smartcase), `Esc` clears query and exits, `Enter` locks and returns key handling to the list's verbs; exposes filtered-index mapping so motions and verbs operate on the filtered view; empty-result state carries the query for the hint line. Failing tests first; pure module, no app types (FR-7).
- [x] 2.2 Feat: annotation list adopts the component — filter indicator with query text in the list chrome (reusing the help overlay's filter-line styling), empty-state hint ("no matches — Esc to clear"), motions move within filtered results, `Enter`/`e`/`d` act on the filtered selection (FR-8, FR-9).
- [x] 2.3 Feat: staging panel and accepted panel adopt the component; `Space`/verbs act on the filtered selection (FR-8, FR-9).
- [x] 2.4 Feat: switcher (both tabs) adopts the component; tab toggle interacts sanely with an active filter (filter is per-open, transient) (FR-8, FR-9). **Decision**: toggling tabs clears the active filter — the simplest sane behavior, since a query typed against branch names carries no meaning over to worktree names; documented on `SwitcherState::filter`.
- [x] 2.5 Filter-mode keys land in the shared modal-key machinery: rows with footer hints and help coverage for every gaining table; drift tests extended; no-shadow verification against each table's existing bindings; config-remap registration where the host table is remappable (FR-10). Folded into 2.2's commit per this task list's explicit allowance, since the shared `FILTER_EDIT_KEYS` table and `modes.rs`'s shared `intercept_filter` dispatcher genuinely span all four gaining contexts at once.
- [x] 2.6 FR-11 reconciliation: compare the help overlay's `/` filter (substring smartcase via `search::smartcase_contains`) against the shared component (fuzzy); adopt only if behavior-identical, otherwise leave help as-is and document the divergence in the module doc — no user-visible change either way; the existing help-filter tests (`help_filter_enter_locks_and_two_escapes_close`, `help_filter_narrows_rendered_bindings_to_matching_rows`, `closing_help_resets_the_filter`) pass unchanged as the preservation proof.
- [x] 2.7 Perf: add a 5k-row filter budget tripwire to `src/ui/perf_tests.rs` following the existing loop-amortized wall-clock pattern; confirm all existing tripwires stay green.
- [x] 2.8 Proof: 30-annotation scratch session, capture journey transcript B into `12-proofs/`, record all-gates output in `12-proofs/12-task-02-proofs.md`.

### [ ] 3.0 Filter and move through the Review launcher like every other list

A user presses `R`, lands on the launcher's Branches or Commits tab, pages/jumps/counts with the same motions as everywhere else, presses `/` and types a fragment to fuzzily narrow branches or commits, and a filtered `Enter` starts the right review — unless a review session is already active, in which case the same guard hint appears as today. Covers FR-12..FR-13.

#### 3.0 Proof Artifact(s)

- Test: launcher integration tests demonstrate fuzzy-filtering branches and commits, motion within filtered results, and the in-session guard holding under a filtered `Enter` (FR-12).
- Test: the motion coverage drift test includes both launcher tabs as consuming contexts (FR-13, extends FR-5).
- CLI: journey transcript `12-proofs/12-journey-c-branch-pick.txt` — many-branch scratch repo: `R`, `/`, type fragment, `Enter` starts review of the right branch (FR-12).

#### 3.0 Tasks

- [ ] 3.1 Feat: launcher Branches and Commits tabs consume the motion layer (including counts), clamped to `review_launcher_row_count()`; Commits-tab lazy prefetch (`poll_launcher_commits`) fires on layer-driven moves; both tabs added to the motion coverage drift test (FR-12, FR-13).
- [ ] 3.2 Feat: both launcher tabs adopt the `/` filter component with indicator and empty-state hint; a filtered `Enter` routes through `confirm_launcher_branch_review`'s existing `in_review_session()` guard unchanged (FR-12).
- [ ] 3.3 Launcher modal key table gains the filter-mode rows with footer hints, help coverage, config remapping, and drift-test coverage; no-shadow verification against `REVIEW_LAUNCHER_KEYS` (FR-13).
- [ ] 3.4 Proof: many-branch scratch repo, capture journey transcript C into `12-proofs/`; integration test proving the guard holds under filtered `Enter`; record all-gates output in `12-proofs/12-task-03-proofs.md`.
