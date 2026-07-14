# Task 06 Proofs — User acceptance (Success Metrics)

Parent task 6.0 is the user-acceptance gate: it grades spec 05's six
user-verifiable Success Metrics, not internal test counts. Each acceptance
scenario is driven through the **real key-dispatch pipeline** (`dispatch_key`,
the same handler the blocking event loop calls) against throwaway tempdir
repos, with screens captured by rendering the real `draw` into a
`TestBackend` — the screenshot stand-in this sandbox requires (no controlling
TTY: `enable_raw_mode` → os error 6). Every acceptance test also `eprintln!`s
the artifact it captures, invisible in a normal `cargo test` run but
reprintable with `-- --nocapture` — that is how the verbatim text in the
evidence files was captured, reproducibly, with no throwaway debug edits.

## Acceptance evidence files

Under `docs/specs/05-spec-diff-sources/proofs/`:

- [`dead-end-journey.md`](../proofs/dead-end-journey.md) — Success Metric 1
- [`round-trip-emission.md`](../proofs/round-trip-emission.md) — Success Metric 2 (producer half; consumer half run by the orchestrator)
- [`no-lies-overlay.md`](../proofs/no-lies-overlay.md) — Success Metric 3
- [`dogfood-review.md`](../proofs/dogfood-review.md) — Success Metric 5

## Per-metric verdicts

| # | Success Metric | Verdict | Evidence |
|---|----------------|---------|----------|
| 1 | The dead-end disappears (≤5 keys launch→newest commit's diff) | **MET** — 3 keystrokes (`` ` `` → `Tab` → `Enter`), each named on-screen | `proofs/dead-end-journey.md`; test `dead_end_journey_reaches_the_newest_commit_in_a_handful_of_keys` |
| 2 | The fix-loop works on history (agent can resolve every site) | **MET (producer half)** — 3 sites / 2 files / one `Reviewing: <short-sha>`; consumer half is the orchestrator's | `proofs/round-trip-emission.md`; test `history_round_trip_producer_emits_three_sites_across_two_files_under_one_reviewing_line` |
| 3 | The tool never lies (listed keys work; absent caps inert) | **MET** | `proofs/no-lies-overlay.md`; new test `commit_view_help_overlay_shows_only_truthful_keys` + task-3.6 behavior tests |
| 4 | Existing habits unbroken (working-tree stdout byte-identical) | **MET** (inherited) — byte-exact fixtures unedited, explicit backward-compat test | `annotate::markdown` tests (task 4.0); full suite green |
| 5 | Dogfood gate (review this spec's own commits via the History tab) | **MET** — 3 real issues + 2 praise, emitted with grouped `Reviewing:` lines | `proofs/dogfood-review.md` |
| 6 | Still instant (perf tripwires pass, unmodified budgets) | **MET** (inherited) — `src/ui/perf_tests.rs` unmodified and green | full `cargo test` |

## Tests added by this task (all in `src/ui/history_integration_tests.rs`)

- `dead_end_journey_reaches_the_newest_commit_in_a_handful_of_keys` (6.1)
- `history_round_trip_producer_emits_three_sites_across_two_files_under_one_reviewing_line` (6.2)
- `commit_view_help_overlay_shows_only_truthful_keys` (6.3)

Task 3.6's existing tests already cover the no-lies *behavior* (staging/
code-intel inert + hidden, no auto-refresh, annotations functional), so 6.3
adds the overlay-buffer cross-check rather than extending them.

## Notable observations / follow-ups filed (not blocking)

From the dogfood (see `proofs/dogfood-review.md` for the full annotations):

1. `git_panel.rs:263` (nit) — History-row short SHA is the first token clipped
   in a narrow panel, though it's the token that ties a row to the
   `Reviewing:` emission.
2. `runner.rs:177` (question) — `commit_log` maps every non-zero `git log`
   exit to an empty list, so a real git error is indistinguishable from an
   empty repo.
3. `git_panel.rs:256` (nit) — the unpushed marker reuses the staged-file
   glyph/color.

All three are filed (more than a one-liner + tests each; none affects a
Success Metric), not fixed in this task — the honest call per the task's
"fix small / file large" guidance.

## Four gates

```
cargo build                                # Finished, no warnings
cargo test                                 # 886 lib + 45 integration, all pass
cargo clippy --all-targets -- -D warnings  # clean
cargo fmt --check                          # clean
```

`src/ui/perf_tests.rs` is untouched by this task (regression contract intact).
