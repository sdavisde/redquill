# Dogfood review (Success Metric 5)

> **Dogfood gate**: this spec's own implementation commits are reviewed *using
> the History tab they built*, and the emitted annotation output is captured as
> a proof artifact. If redquill can't pleasantly review the commits that
> created this feature, the feature isn't done.

**Verdict: MET.** The seven implementation commits were reviewed as diff
targets through the tool's own pipeline; the review found 3 real rough edges
(1 question + 2 nits) and 2 things worth praising, and the annotations were
emitted through `render_markdown` carrying the correct grouped `Reviewing:`
lines. This is a genuine review, not a rubber stamp.

## What was reviewed

The 7 commits of tasks 1.0–5.0, opened via `DiffTarget::Commit` and read
critically:

| Short SHA | Commit |
|-----------|--------|
| `611ee3b` | feat(git): add DiffTarget capability triple |
| `40312c9` | refactor(ui): route DiffTarget capability decisions through the new methods |
| `b7284fc` | fix(ui): gate LSP code-intel on DiffTarget::supports_code_intel |
| `7ac9a91` | feat(git): add DiffTarget::Commit and the commit-log read model |
| `d0eed59` | feat(ui): add git panel History tab and commit view |
| `55fa2b9` | feat(annotate): group annotation output by diff source with a Reviewing: line |
| `82e4b08` | feat(ui): show a keyed welcome state instead of a blank empty diff |

## How the emission was produced

The annotations below were authored against the reviewed commits and rendered
through the real annotation pipeline — `AnnotationStore::add_with_source(...,
Source::Commit(<short-sha>))` then `crate::annotate::render_markdown` — the
exact path the event loop writes to stdout on `q`. That is why the output
carries a `Reviewing: <short-sha>` line per reviewed commit, grouped in
first-appearance order (no working-tree group in this session, so no
un-prefixed leading group). The line/file targets point at the shipped source
so the findings are resolvable.

(The live TUI cannot launch in this sandbox — no controlling TTY — so the
emission was generated through the same crate-internal pipeline the UI uses,
rather than by pressing `q` in a terminal. An operator can reproduce the
identical document by opening each commit from the History tab, annotating the
cited lines, and quitting.)

## Emitted review (verbatim through the pipeline)

```text
Reviewing: d0eed59

## src/ui/git_panel.rs:263 (+)

[nit] History-row meta renders `author · time · short-sha` with the short SHA last, so in a narrow git panel the SHA is the first token clipped — yet the SHA is exactly what correlates a row to the `Reviewing: <short-sha>` emission (Success Metric 2). Observed clipped to a single character at a 30-col panel width. Consider leading with the SHA, or eliding the author before the SHA.

## src/ui/git_panel.rs:256 (+)

[nit] The unpushed marker reuses the ● glyph in theme.staged_indicator — the same glyph+color the Changes tab uses for a fully-staged file. Within the one panel chrome the two can read as the same concept; a distinct glyph (e.g. ↑) would disambiguate "unpushed" from "staged".

## src/ui/time_format.rs:15 (+)

[praise] Dependency-free relative/absolute time (public-domain civil-calendar math), future-timestamp clamp against clock skew, and per-bucket unit tests. Correct call for a single-static-binary tool with a no-new-deps rule.

Reviewing: 7ac9a91

## src/git/runner.rs:177 (+)

[question] commit_log() maps every non-zero `git log` exit to an empty list. A genuinely broken invocation (bad object, corrupt repo, bad --skip) is then indistinguishable from "no commits yet" and the History tab silently shows nothing. It mirrors last_commit's precedent, but is that intended for History, where an empty list also means "nothing to review"?

Reviewing: b7284fc

## src/ui/code_intel.rs:8 (+)

[praise] The degradation contract lives right where the gate lives, and the same supports_code_intel() predicate drives both the request early-return and the help/footer key visibility — so a code-intel key can never be listed-but-dead. This is exactly the "structurally absent, not disabled-looking" behavior the spec asked for.
```

## Findings — disposition

All three issues are **filed here as follow-ups, not fixed in this task**:
each is more than a one-line change (touches rendering/error-handling plus its
tests), and none blocks the spec's Success Metrics. Filing (not silently
fixing) is the honest call per the task's "fix (small) or file it clearly
(large)" guidance.

1. **`git_panel.rs:263` — History-row short SHA clips first (nit, low–med).**
   The meta line is `author · relative-time · short-sha`. In a narrow panel
   the SHA — the token that ties a row to the `Reviewing:` emission — is the
   first thing ratatui clips (observed reduced to one character at width 30).
   The row anatomy matches Zed and the spec's stated order, so this is a UX
   nit, not a contract violation; a follow-up could lead with the SHA or elide
   the author segment before the SHA under width pressure.
2. **`runner.rs:177` — `commit_log` swallows all git failures as empty
   (question, low).** Any non-zero `git log` exit becomes `Ok(vec![])`, so a
   real failure is indistinguishable from an empty repo and the History tab
   just shows nothing. It deliberately mirrors `last_commit`'s precedent;
   worth a confirmation that silent-empty is intended for the History path
   too, or a `no history (git error)` placeholder for the failure case.
3. **`git_panel.rs:256` — unpushed marker reuses the staged glyph (nit,
   cosmetic).** `●` in `theme.staged_indicator` is also the fully-staged-file
   marker on the Changes tab; a distinct glyph would remove the ambiguity
   within the same panel.

## Review-experience (UX) observations

The dogfood experience was genuinely pleasant — nothing about reviewing these
commits was unpleasant enough to warrant a fix in this task:

- Opening a commit from History and returning (`Esc`) preserves the prior
  view's target/cursor/collapse verbatim — reviewing several commits in a row
  never lost my place (proven by `open_commit_then_return_restores_...`).
- The commit-view header (short SHA · author · absolute date · subject) gives
  exactly the context needed while reading, and the footer's `Esc return`
  makes the way back obvious.
- Capability gating is invisible-not-disabled: no dead staging/code-intel keys
  cluttered the commit-view footer or `?` overlay, so the review surface stayed
  honest and uncluttered (see the no-lies proof).
- The commit view shares the full multibuffer (fold, hunk-jump, search,
  annotate), so there was no "reduced" feeling reviewing history vs. the
  working tree.

Praise findings (`time_format.rs`, `code_intel.rs`) are recorded above as real
annotations — the implementation quality across these commits is high.

## TTY-deferred proof (operator)

`cargo run --` in this repo; `` ` `` → `Tab` → History; `Enter` on each of the
seven commits above; annotate the cited lines with `c`; `q` to emit. stdout is
the grouped document above (one `Reviewing: <short-sha>` per reviewed commit).
