# History round-trip — producer half (Success Metric 2)

> **The fix-loop works on history**: annotating lines of a historical commit
> and quitting produces stdout an agent can act on unambiguously — verified by
> running the README's own pipe (`redquill | <agent> -p "address this review
> feedback"`) against a committed change and confirming the agent locates
> every annotated site.

**Verdict (producer half): MET.** Annotating 3 lines across 2 files of a
historical commit and rendering the store produces exactly one
`Reviewing: <short-sha>` line for the whole commit group, followed by three
`## path:line (+)` sites resolvable against that revision.

**The consumer half (handing this emission to a separate agent and confirming
it locates all three sites) is run by the orchestrator.** This file is the
producer output + the ground truth the orchestrator checks the agent against.

## How this was produced

Driven through the real key-dispatch pipeline against a throwaway tempdir
repo (no TTY in this sandbox), then `crate::annotate::render_markdown` is
called on the resulting store — the exact function the event loop writes to
stdout on `q`. Backing test:
`src/ui/history_integration_tests.rs::history_round_trip_producer_emits_three_sites_across_two_files_under_one_reviewing_line`.

Reproduce verbatim:

```sh
cargo test --lib -- --nocapture --test-threads=1 \
  history_integration_tests::history_round_trip
```

## Tempdir repo structure (ground truth)

Fixture `repo_with_two_file_commit()`:

- **Commit 1** `base: two files` — creates:
  - `alpha.txt` = `alpha 1 / alpha 2 / alpha 3`
  - `beta.txt` = `beta 1 / beta 2 / beta 3`
- **Commit 2** `feat: extend both files` (the newest commit, the one opened
  and annotated) — appends:
  - `alpha.txt` gains new-side lines **4** (`alpha 4`) and **5** (`alpha 5`)
  - `beta.txt` gains new-side line **4** (`beta 4`)

The commit's own diff (`<rev>^..<rev>`) therefore introduces exactly those
three added lines across the two files.

The session: open the newest commit from the History tab (`` ` `` → `Tab` →
`Enter`), then steer the cursor onto each added line and comment (`c`, type,
`Enter`).

## The three annotated sites (ground truth for the consumer)

| # | File | New-side line | Line content | Comment |
|---|------|---------------|--------------|---------|
| 1 | `alpha.txt` | 4 | `alpha 4` | rename this to something clearer |
| 2 | `alpha.txt` | 5 | `alpha 5` | possible off-by-one here |
| 3 | `beta.txt` | 4 | `beta 4` | add a test covering this |

All three are `(+)` (new-side) annotations against the commit whose short SHA
appears in the `Reviewing:` line below. A consumer resolves each site as
`<short-sha>:<path>` line `<line>`.

## Verbatim stdout emission

Captured byte-for-byte from the run (the short SHA is the tempdir repo's real
`git`-resolved abbreviation for that run — `aa44e7c` in the captured run; it
varies per fresh tempdir, which is exactly why the emission self-describes the
revision):

```text
Reviewing: aa44e7c

## alpha.txt:4 (+)

[issue] rename this to something clearer

## alpha.txt:5 (+)

[issue] possible off-by-one here

## beta.txt:4 (+)

[issue] add a test covering this
```

Asserted by the test: exactly **one** `Reviewing:` line; the emission
**starts with** `Reviewing: <that commit's short SHA>` (no stray working-tree
group); and all three `## alpha.txt:4 (+)`, `## alpha.txt:5 (+)`,
`## beta.txt:4 (+)` headers are present — three sites, two files, one
revision.

## Orchestrator note — consumer half

Hand the emission above to a separate agent with the README's pipe framing
(`address this review feedback`), pointing it at the same commit, and confirm
it locates all three sites: `alpha.txt` lines 4 and 5, `beta.txt` line 4,
resolved against the commit named on the `Reviewing:` line. Because the tempdir
short SHA is per-run, the consumer check should use the repo/commit produced by
the same run (or re-derive the fixture); the *structure* (one Reviewing line +
the three `path:line` sites) is stable.

## TTY-deferred proof (operator)

In a real terminal, in a repo with a commit that changed ≥2 files:
`cargo run --`, then `` ` `` → `Tab` → `Enter` on that commit, move to a
changed line (`j`/`k`), `c`, type a comment, `Enter`; repeat for two more
lines in a second file; then `q`. stdout begins with `Reviewing: <short-sha>`
followed by the three annotations in the standard per-annotation format —
pipe it to an agent per the README example.
