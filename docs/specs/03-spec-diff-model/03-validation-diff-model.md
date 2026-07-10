# 03-validation-diff-model

Validation report for **Task 3 — Diff Model** (`03-spec-diff-model.md`), produced by the
/md pipeline Phase 3 (2 zero-overlap auditors + folded /trace duties per OP-040, plus a
witness-lite compliance pass per OP-041).

**Verdict: PASS** (no CRITICAL/HIGH findings; all actionable findings closed in-cycle)

## Quality gates (verified at HEAD ff17d7f)

| Gate | Result |
| --- | --- |
| `cargo build` | ✓ clean |
| `cargo test` | ✓ 73 passed / 0 failed (59 lib + 3 diff_integration + 11 git_integration) |
| `cargo clippy -- -D warnings` | ✓ zero warnings |
| `cargo fmt --check` | ✓ clean |

## Coverage matrix (synthesized from auditor sector reports — no Unknown entries)

| FR-ID | Implemented | Tested | Evidence (auditor) |
| --- | --- | --- | --- |
| FR-diff-parse-1 | ✓ | ✓ | `parse_patch` total walk, model.rs (A) |
| FR-diff-parse-2 | ✓ | ✓ | `@@` header parse incl. omitted-count→1, section text retained (A) |
| FR-diff-parse-3 | ✓ | ✓ | line classification + disjoint lineno assignment, zero-context hunks (A) |
| FR-diff-parse-4 | ✓ | ✓ | new/deleted/rename±edits/mode-only/binary fixtures; metadata never body Lines (A) |
| FR-diff-parse-5 | ✓ | ✓ | `\ No newline` marker old/new/both, no extra Line (A) |
| FR-diff-word-1 | ✓ | ✓ | single-seam `word_diff_spans`, one production call site (A) |
| FR-diff-word-2 | ✓ | ✓ | shared prefix/suffix excluded; char-offset spans incl. multibyte/emoji (A) |
| FR-diff-word-3 | ✓ | ✓ | i-th↔i-th pairing, excess/context/identical → empty spans (A) |
| FR-diff-nav-1 | ✓ | ✓ | cross-file hunk nav, zero-hunk skip, None at ends (B) |
| FR-diff-nav-2 | ✓ | ✓ | file nav lands on zero-hunk files, None at ends (B) |
| FR-diff-nav-3 | ✓ | ✓ | all four fns pure — `&[DiffFile]` + `&DiffPosition` → `Option<DiffPosition>` (B) |
| FR-diff-wire-1 | ✓ | ✓ | `run()` → `parse_patches` + `summarize` summary line; untracked folded into files (B) |
| FR-diff-wire-2 | ✓ | ✓ | tempdir-isolated integration tests, host repo untouched, files>0 && hunks>0 (B) |

## Findings (canonical tally: CRITICAL 0 · HIGH 0 · MEDIUM 1 · LOW 2 · INFO 1 — parity-verified against sector reports)

| ID | Sev | Finding | Status | fix_commit |
| --- | --- | --- | --- | --- |
| A-MED-1 | MEDIUM | `parse_patch` u32 lineno overflow on crafted `@@ -4294967295 +1 @@` (debug panic / release wrap) — violated stated totality invariant; not reachable via real git | CLOSED — `saturating_add`, regression test `overflowing_hunk_header_start_saturates_instead_of_panicking` | ff17d7f |
| B-LOW-1 | LOW | `next_file`/`next_hunk` unguarded `pos.file + 1` (overflow only at `usize::MAX` stale pos) | CLOSED — saturating arithmetic → None; regression test `stale_out_of_range_position_never_panics_and_stays_valid` | ff17d7f |
| B-LOW-2 | LOW | `prev_hunk` returns still-invalid index for stale out-of-range `pos.hunk` | CLOSED — clamp to valid range before decrement; covered by same regression test | ff17d7f |
| B-INFO-1 | INFO | T3.0 TDD is a disclosed "written-then-stub-reverted" variant, failing-first verified for all substantive cases | ACCEPTED — no action (honest disclosure, property holds) | — |

Note: validation verdict was PASS pre-fix (no CRITICAL/HIGH); the fixer ran as don't-defer
in-cycle closure, not an autonomous-retry cycle. Retry cycles used: 0 of 2.

## Folded /trace results

- `diff/` purity: no ratatui / crossterm / GitRunner / process / fs imports (A).
- `GitRunner::diff` call sites confined to `main.rs` + tests (B).
- Single word-diff algorithm seam confirmed; spans are `Range<usize>` char offsets, never pre-styled text (A).
- Parser hot path linear; `MAX_LINE_TOKENS=400` LCS guard degrades to whole-line span (A).
- Adversarial battery: 11 crafted inputs (malformed headers, `@@`-in-section, CRLF, multibyte
  UTF-8 spans, inflated counts, binary) — only the overflow case broke pre-fix (A); crafted
  stale positions + zero-hunk-only and empty models safe (B).

## Witness-lite compliance (OP-041)

R1 TDD ✓ · R2 no-unwrap ✓ · R3 conventional commits + 4 gates ✓ · R4 no new deps / no destructive git ✓ · R5 FR traceability + honest proofs ✓ — overall PASS
(`pipeline-state/witness-matrix.md`)

## Sector reports

Full evidence: `pipeline-state/auditor-A-report.md` (model.rs + word.rs, 8 FR-IDs),
`pipeline-state/auditor-B-report.md` (nav.rs + mod.rs + main.rs + integration, 5 FR-IDs).
Note: `pipeline-state/` is gitignored per-clone observability surface; this report is the
durable synthesis.
