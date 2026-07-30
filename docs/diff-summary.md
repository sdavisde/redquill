# Review summary model

`diff::summarize` rolls a whole review up into one `ReviewSummary`, so the
question "how big is this, and where should I start?" is answerable without
walking hunks at a render surface.

## What it reports

| Field | Meaning |
| --- | --- |
| `files` | Every file in the review — binary and rename-only included. |
| `binary_files` | How many of those are binary. |
| `content_files` | Files whose content actually changed (added, deleted, modified) and that carry at least one changed line. |
| `stat` | Added/removed lines summed across the review. |
| `largest_file` | The file with the most changed lines, as a `Hotspot { path, stat }`. |
| `largest_hunk` | The biggest single hunk's changed-line count, across all files. |

## Counting rules

**Churn, not net.** `DiffStat::total()` is `added + removed`, so a line
rewritten in place counts twice — once on each side. That's deliberate: the
measure exists to estimate how much there is to *read*, and rewriting a line
means reading two. `DiffStat::net()` is there for the cases that want the
signed difference instead.

**Context lines never count.** A hunk with two changed lines and forty lines
of context is a two-line hunk as far as the summary is concerned. This
matches `Hunk::stats`, which has always excluded context.

**Binary files count as files, never as lines.** A line count over binary
content is meaningless, so binary files land in `files` and `binary_files`
and are skipped entirely for `stat`, `largest_file`, and `largest_hunk`.
This is the same call `stat_display` makes when it renders `bin` instead of
`+0 -0`.

**Rename-only changes count as files, not content changes.** A pure rename
carries no hunks, so it contributes to `files` but not to `content_files` —
`FileChangeKind::is_content_change` draws that line.

**Ties keep the earlier file.** When two files carry identical churn,
`largest_file` reports whichever came first in the input. Callers keep the
file list path-sorted, so the tiebreak is stable and path-ordered rather
than arbitrary.

## Where the numbers come from

`build_review` computes the summary once, on the background snapshot build,
and hands it to the UI on `ReviewSnapshot`. Render surfaces read the
precomputed value; nothing recomputes it per frame.
