# 04-validation-first-render

Validation report for spec 04 (First Render), cycle `04-spec-first-render-e3d5bf1-20260710T1224Z`.
Two zero-overlap read-only auditors (sector A: keymap/terminal/mod/Cargo.toml; sector B:
app/main/tests + folded /trace data-flow pass per OP-040). Full sector reports:
`pipeline-state/auditor-A-report.md`, `pipeline-state/auditor-B-report.md`.

## Verdict: PASS

- CRITICAL: 0 · HIGH: 0 · MEDIUM: 1 · LOW: 7 (synthesis parity-checked against the
  canonical sector reports — counts match, GATE 9 clean)
- No autonomous-retry cycle needed.

## Coverage matrix (synthesized; no Unknown entries)

| Requirement | Sector | Verdict | Evidence anchor |
| --- | --- | --- | --- |
| FR-render-term-1 (raw mode + alt screen enter/exit) | A | Covered | `TerminalGuard::enter` / `restore_terminal` |
| FR-render-term-2 (panic hook restores first) | A | Covered | `Once`-installed chained hook; restore before previous hook runs |
| FR-render-term-3 (guard Drop restores) | A | Covered | `impl Drop`; counter-instrumented routing test |
| FR-render-keymap-1 (KeyEvent→Action via data) | A | Covered | `Keymap::resolve` HashMap lookup; no widget match arms |
| FR-render-keymap-2 (exact README map) | A | Covered | binding-by-binding diff: 21 README bindings + 4 §9 aliases, none missing/extra |
| FR-render-keymap-3 (only j/k, Ctrl-d/u, q live) | A | Covered | `apply_action` exhaustive match; deferred Actions explicit no-ops |
| FR-render-keymap-4 (unbound → Noop) | A | Covered | resolve default + unit test |
| FR-render-keymap-5 (one-line rebind invariant) | A+B | Covered | only non-test key literal lives in `default_map()`; empty-state hint derives via `chord_for(Action::Quit)` |
| FR-render-view-1 (all files, one scrollable buffer) | B | Covered | flattened `Row` model; boundary windows traced correct |
| FR-render-view-2 (gutter + old/new line numbers) | B | Covered | `gutter_marker` / `lineno_col_width` + unit tests |
| FR-render-view-3 (added/removed colors) | B | Covered | green/red/default styling |
| FR-render-view-4 (changed_spans emphasis) | B | Covered | `Modifier::REVERSED` on span ranges; OOB-safe under truncation |
| FR-render-view-5 (line/half-page scroll, clamped, render-on-event) | B | Covered | saturating clamp math; blocking `event::read()`; half-page = half current viewport per event |
| FR-render-wire-1 (ui never touches git) | B | Covered | zero `git` imports under `src/ui/`; `main.rs` sole `GitRunner` caller |
| FR-render-wire-2 (empty diff → TUI empty state, clean quit) | B | Covered | no bypass; derived quit hint |
| §6 edge cases (panic-restore, empty, long lines, 5k perf slice, resize, binary placeholder, UTF-8 as-is, narrow term) | A+B | 8/8 Covered | two carry automated-test gaps but are implementation+observable-covered per spec §8 |

## Quality gates (auditor-run and orchestrator-run, both green)

- `cargo build` ✓ · `cargo test` ✓ (114 passed, 0 failed) · `cargo clippy -- -D warnings` ✓ (also `--all-targets`) · `cargo fmt --check` ✓

## Findings (all non-blocking)

1. **MEDIUM (A)** — `chord_for` returns a nondeterministic (HashMap-order) chord for
   alias-bound actions (scroll/half-page). Harmless now (sole consumer is single-bound
   `Action::Quit`); MUST prefer the primary chord before the Task-5 help overlay
   displays bindings. → carried to Task 5.
2. **LOW (A)** — bare `d`/`r` gd/gr placeholders must be replaced by real `g`-chord
   sequencing before GotoDefinition/FindReferences go live (Task 5+). Doc-commented.
3. **LOW (A)** — `mod` declaration order alphabetical vs contract literal order; no
   semantic effect.
4. **LOW (B)** — no unit test asserts the changed_spans→emphasis style mapping.
5. **LOW (B)** — `render_frame` recomputes `total_rows` per frame (O(#hunks) — cheap,
   not a whole-buffer build; still worth caching when Task 5 touches the loop).
6. **LOW (B)** — truncation is char-count, not display-width (CJK/wide-glyph cosmetic).
7. **LOW (B)** — `diff::summarize`/`DiffSummary` orphaned by the main.rs rewire; stale
   doc-comment. Candidate for reuse or removal in Task 5 sidebar work (improve-not-remove:
   sidebar file list is its natural consumer).
8. **LOW (B)** — the `REDQUILL_PANIC_TEST` trigger ships env-gated in the render path;
   track removal after the operator's interactive verification (TO-DO Part A).

## Operator follow-ups (→ TO-DO.txt Part A)

- Interactive TTY verification (sandbox had no controlling TTY): visual render on a dirty
  repo, clean-repo empty state + `q`, and `REDQUILL_PANIC_TEST=1` panic-restore check —
  exact steps in `docs/specs/04-spec-first-render/04-proofs/T4.0-proof.md`.
- `cargo-audit` still not installed — dep-CVE gates ran as logged-skip this cycle.
