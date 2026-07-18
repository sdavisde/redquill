# 11-tasks-panel-action-parity.md

Task list for `11-spec-panel-action-parity.md`. Parent tasks mirror the spec's three demoable units; each is a thin end-to-end slice demonstrable on its own.

Precondition (workspace hygiene, not a task): the uncommitted which-key withdrawal in the working tree (`src/ui/keymap.rs`, `src/ui/mod.rs`, `src/ui/mod_tests.rs`, `src/ui/keymap_config_tests.rs`, deleted `src/ui/which_key.rs`) must be committed before implementation begins — spec 11 edits the same files and the diffs must not tangle.

## Relevant Files

| File | Why It Is Relevant |
| --- | --- |
| `src/ui/keymap.rs` | Shared keymap tables: new `Scope::Panel` rows (`Space`, `S`, `d`, `Esc`, `s`, `/`) and new diff-scope rows (`e`, `x`) with `FooterHint`s; panel review "phantom" rows for help/footer documentation. |
| `src/ui/keymap_config_tests.rs` | Config-remap and bidirectional help/dispatch drift tests covering every new row (FR-5, FR-8, FR-12). |
| `src/ui/modes.rs` | `handle_panel_key` (lines ~174–198): panel dispatch of the new rows, review-session translation for panel keys, directory/History-tab gating. |
| `src/ui/mod.rs` | `dispatch_key`: existing review translation pattern (~476–482) mirrored for panel scope; help-overlay shadowing of panel `Esc` (already structural, ~355–367). |
| `src/ui/mod_tests.rs` | End-to-end dispatch tests through the real key path for all three units. |
| `src/ui/staging.rs` | `toggle_stage` mode guard (line ~37) — the one refactor seam: relax to include `Mode::Panel` or extract the file-targeted core. |
| `src/ui/app.rs` | `apply` dispatch: arms for the new `EditAnnotation`/`DeleteAnnotation` actions; existing `open_compose_for` (~1203) reused for in-place edit. |
| `src/ui/app_tests.rs` | Unit tests for the new apply arms (edit pre-fill, delete parity, no-op hint). |
| `src/ui/annotation_list.rs` | Existing `delete_focused_annotation` (~117): extract a delete-by-id core shared with the diff-view `x` path so list-parity is structural, not copied. |
| `src/ui/annotation_overlap.rs` (new) | Pure overlap-resolution function (FR-11): nearest target start above-or-at cursor wins, ties oldest-first. No app/TUI types in its signature. |
| `src/ui/annotation_overlap_tests.rs` (new) | TDD unit tests for the overlap rule: line/range/hunk/file targets, multi-overlap, file-header-row case. |
| `src/ui/footer.rs` | `panel_hints` (~198–235): remove the deliberate suppression of review-status hints in panel scope; capability-gate like `normal_hints`/`visual_hints`. |
| `src/ui/footer_tests.rs` | Footer tests for panel stage/accept/defer hints and their capability gating (FR-4). |
| `src/ui/git_panel.rs` | `PanelRow`/`panel_follow`/`panel_select`: row-kind inspection for file-vs-directory gating; no cursor-sync changes expected. |
| `src/ui/git_panel_tests.rs` | Panel dispatch tests: stage/accept/defer on file rows, inert directory/History/read-only cases. |
| `docs/specs/11-spec-panel-action-parity/proofs/` (new) | Journey transcripts and per-unit UI demo scripts + recorded user verdicts. |

### Notes

- Run tests with `cargo test`; all four gates (`cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`) must pass before every commit, and `src/ui/perf_tests.rs` tripwires stay green.
- Conventional commits; the `toggle_stage` guard change (refactor) commits separately from the new bindings (behavior).
- TDD for the pure overlap rule; integration/journey tests build scratch repos in canonicalized tempdirs only — never the host repo.
- **User UI checkpoint (applies to every parent task):** the final sub-task of each parent is a hands-on demo the *user* runs in the real TUI (`cargo run` against a scratch repo prepared by a script in `proofs/`). The parent task's `[ ]` box is only checked after the user confirms the behavior through the UI; the verdict is recorded in the demo script file. Automated tests alone do not close a parent task.

## Tasks

### [~] 1.0 Panel file actions — stage, unstage, accept, defer from the git panel (FR-1..FR-5)

Route `Space`/`S` (and in review sessions `d`) from the panel's highlighted file row to the existing stage/accept/defer operations — zero new git-layer code.

#### 1.0 Proof Artifact(s)

- Test: panel dispatch tests in `src/ui/git_panel_tests.rs` / `src/ui/mod_tests.rs` demonstrate `Space` toggle-stages and `S` stages the highlighted file in working-tree mode, translate to accept and `d` toggle-defer in a review session with tri-state markers updating, and are inert with a status hint on directory rows, inert on the History tab, and inert with hidden hints on read-only targets (FR-1, FR-2, FR-3).
- Test: footer tests in `src/ui/footer_tests.rs` demonstrate panel hints now include stage/accept/defer, capability-gated (FR-4); drift tests in `src/ui/keymap_config_tests.rs` pass (FR-5).
- CLI: journey transcript on a scratch tempdir repo — open panel, `j` to a file, `Space` stages it, staged marker updates in place; same flow in a review session accepts the file and the `●` marker appears — persisted to `proofs/` (FR-1, FR-2).
- Demo: user runs `proofs/demo-1-panel-actions.md` script in the live TUI (`cargo run` on a scratch repo) and confirms staging and review triage from the panel; verdict recorded in the script file (FR-1..FR-4).

#### 1.0 Tasks

- [x] 1.1 Confirm the which-key withdrawal precondition is met (working tree clean of `src/ui/*` changes); if not, stop and coordinate with the user before touching shared files.
- [x] 1.2 Refactor `src/ui/staging.rs::toggle_stage`: relax the `Mode::Normal | Visual` early-return to legitimately admit panel invocations (include `Mode::Panel`, or extract the file-targeted core the guard wraps). Behavior-preserving for existing paths — identical test counts, zero assertion edits; commit as `refactor:` on its own.
- [x] 1.3 TDD: add failing dispatch tests (in `src/ui/git_panel_tests.rs` and `src/ui/mod_tests.rs`) for panel `Space` toggle-stage and `S` stage-file on a highlighted file row in working-tree mode, asserting the staged marker updates via the existing refresh path (FR-1).
- [x] 1.4 Add `Scope::Panel` keymap rows `Space` → `ToggleStage` and `S` → `StageFile` in `src/ui/keymap.rs` with footer hints, and route them in `src/ui/modes.rs::handle_panel_key`, relying on `panel_follow`'s existing cursor sync. Panel invocations must force the whole-file gesture (`StageGesture::WholeFile`) — never the hunk/line gesture the diff cursor's row kind would otherwise imply (FR-1).
- [x] 1.5 Mirror the diff view's review-session translation for panel scope (`ToggleStage`→`ToggleAccept`, `StageFile`→`AcceptFile` when `in_review_session()`), add the panel `d` → `ToggleDefer` row, and add panel review phantom rows for help/footer documentation following the existing diff-scope pattern in `src/ui/keymap.rs` (~677–697); assert tri-state markers (`●`/`~`) update immediately (FR-2).
- [x] 1.6 Gate the new keys: no-op with a status-line hint on `PanelRow::Dir`, inert on the History tab, inert with hidden hints when `app.target.staging_mode()` is read-only — reusing the diff view's capability-gating checks (FR-3).
- [x] 1.7 Remove the deliberate suppression of review-status hints in `src/ui/footer.rs::panel_hints` (~208–211) and capability-gate panel hints like `normal_hints`/`visual_hints`; cover with `src/ui/footer_tests.rs` (FR-4).
- [x] 1.8 Add the new rows to the `?` help overlay's panel section via the shared tables, extend config-remap coverage, and keep bidirectional drift tests passing in `src/ui/keymap_config_tests.rs` (FR-5).
- [x] 1.9 Produce the CLI journey transcript on a scratch tempdir repo (working-tree staging flow + review-session accept flow) and persist it to `docs/specs/11-spec-panel-action-parity/proofs/` (FR-1, FR-2).
- [x] 1.10 Run all four cargo gates plus perf tripwires; commit the behavior change as `feat:` separate from 1.2's refactor.
- [~] 1.11 **USER UI CHECKPOINT:** write `proofs/demo-1-panel-actions.md` — a script that builds a scratch repo (setup commands included) and lists the exact key sequence: `cargo run`, `` ` ``, `j` to a file, `Space` (stages, marker updates), `Space` (unstages), `S`; then `--review` on a scratch branch: accept two files, `d`-defer one, watch `●`/`~` update; also verify a directory row shows the hint and the History tab is inert. Pause here — the user runs it and their verdict is recorded in the file before 1.0 is checked off.

### [ ] 2.0 Panel coherence — `Esc` leaves, `s` and `/` reach through (FR-6..FR-8)

Make the git panel answer to the app's universal verbs: `Esc` backs out, `s` and `/` behave as if the panel were closed first.

#### 2.0 Proof Artifact(s)

- Test: dispatch tests demonstrate `Esc` closes the panel and returns to `Normal`, does not close it while the help overlay is open, and `s` / `/` from the panel land in the staging panel and search respectively with focus returning to `Normal` on exit (FR-6, FR-7).
- Test: panel-scope rows appear in help and footer; bidirectional drift tests pass (FR-8).
- CLI: journey transcript — `` ` `` open panel, `Esc` back out; `` ` ``, `/`, type a query, land on a match — persisted to `proofs/` (FR-6, FR-7).
- Demo: user runs `proofs/demo-2-panel-coherence.md` in the live TUI and confirms `Esc`/`s`/`/` from the panel; verdict recorded in the script file (FR-6, FR-7).

#### 2.0 Tasks

- [ ] 2.1 TDD: add failing dispatch tests for panel `Esc` (closes panel → `Normal`; shadowed while the help overlay is open — the existing `dispatch_key` help-shadow at `src/ui/mod.rs` ~355–367 should already provide this, assert it), and for `s`/`/` landing in the staging panel and search with correct exit focus (FR-6, FR-7).
- [ ] 2.2 Add `Scope::Panel` rows in `src/ui/keymap.rs`: `Esc` closing the panel (reuse or introduce the appropriate action so the `` ` `` toggle stays untouched), `s` → `ToggleStagingPanel`, `/` → `Search`; route in `handle_panel_key` so each behaves as if the panel were closed first (FR-6, FR-7).
- [ ] 2.3 Help/footer coverage for the three rows via the shared tables; config-remap and bidirectional drift tests pass in `src/ui/keymap_config_tests.rs` (FR-8).
- [ ] 2.4 Produce the CLI journey transcript (`` ` `` → `Esc`; `` ` `` → `/` → query → match) and persist to `proofs/` (FR-6, FR-7).
- [ ] 2.5 Run all four cargo gates plus perf tripwires; commit as `feat:`.
- [ ] 2.6 **USER UI CHECKPOINT:** write `proofs/demo-2-panel-coherence.md` — scratch-repo script: open panel, `Esc` backs out; open panel with help overlay up, `Esc` closes help not panel; from panel press `s` (staging panel opens) and `/` (search works, exit returns to Normal). Pause for the user's UI verdict before 2.0 is checked off.

### [ ] 3.0 Edit and delete annotations from the diff view (FR-9..FR-12)

Close the reverse gap: `e` edits and `x` deletes the annotation under the cursor, right where it's rendered.

#### 3.0 Proof Artifact(s)

- Test: unit tests for the pure overlap-resolution function demonstrate deterministic selection across line/range/hunk/file targets, multi-overlap, and the file-header-row case (FR-11).
- Test: dispatch tests demonstrate `e` opens edit-in-place pre-filled with the existing body, `x` deletes with list-parity (no-confirmation) semantics, both no-op with a status hint when nothing overlaps, both config-remappable, `c` unchanged, drift tests passing (FR-9, FR-10, FR-12).
- CLI: journey transcript — annotate a line, move away, return, `e`, change the text, submit; then `x` deletes it and the inline row disappears — persisted to `proofs/` (FR-9, FR-10).
- Demo: user runs `proofs/demo-3-annotation-roundtrip.md` in the live TUI and confirms the in-place edit/delete round-trip; verdict recorded in the script file (FR-9, FR-10).

#### 3.0 Tasks

- [ ] 3.1 TDD: create `src/ui/annotation_overlap.rs` with a pure function selecting the overlapping annotation for a cursor position (inputs: cursor file/line/row-kind + the annotations targeting that file; rule: target start nearest above-or-at the cursor wins, ties broken by creation order, oldest first; file-level targets match only the file-header row). Failing tests first in `src/ui/annotation_overlap_tests.rs` covering `Target::Line`/`Range`/`Hunk`/`File` (and worktree variants), multi-overlap, and ties (FR-11).
- [ ] 3.2 Add `EditAnnotation` and `DeleteAnnotation` actions with diff-scope rows `e` and `x` in `src/ui/keymap.rs` (both currently unbound), with footer hints and stable kebab-case names for config remapping (FR-12).
- [ ] 3.3 Wire `apply` arms in `src/ui/app.rs`: `EditAnnotation` resolves the overlap and calls the existing `open_compose_for(id)`; `DeleteAnnotation` resolves and deletes via a delete-by-id core extracted from `annotation_list.rs::delete_focused_annotation` so list and diff paths share one implementation (no confirmation, matching the list). No overlap → status-line hint, no mode change (FR-9, FR-10).
- [ ] 3.4 Dispatch tests in `src/ui/app_tests.rs` / `src/ui/mod_tests.rs`: `e` pre-fills the compose with the existing body and classification, `x` removes the annotation and the inline row disappears on rebuild, both no-op with a hint on non-overlapping lines, `c` still always composes new (FR-9, FR-10, FR-12).
- [ ] 3.5 Help/footer coverage for `e`/`x`; config-remap and bidirectional drift tests pass (FR-12).
- [ ] 3.6 Produce the CLI journey transcript (annotate → `e` edit → submit → `x` delete → inline row gone) and persist to `proofs/` (FR-9, FR-10).
- [ ] 3.7 Run all four cargo gates plus perf tripwires; commit (`feat:` for bindings/wiring; the `annotation_list.rs` delete-core extraction commits separately as `refactor:` if done as a distinct move).
- [ ] 3.8 **USER UI CHECKPOINT:** write `proofs/demo-3-annotation-roundtrip.md` — scratch-repo script: annotate a line with `c`, scroll away and back, `e` on the annotated line (compose opens pre-filled), edit and submit, see the new text inline; `x` deletes it; `e` on a bare line shows the no-op hint. Pause for the user's UI verdict before 3.0 is checked off.
