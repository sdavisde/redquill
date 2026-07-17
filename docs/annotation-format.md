# Annotation markdown format

This is the public-contract format `render_markdown` (`src/annotate/markdown.rs`) emits on quit to stdout. Treat it as a public API once shipped — annotation output is a stable interface for downstream tooling, not incidental logging.

## Basic shape

```text
## src/auth/session.rs:44 (+)

[question] where does keystore get rotated?
```

Each annotation is a `## <header>` line, a blank line, then the classification tag (`[issue]`, `[question]`, `[nit]`, or `[praise]`) followed by the comment body. Multiple annotations are separated by one blank line each. An empty annotation store renders to an empty string; otherwise the output ends with exactly one trailing newline.

## Side markers

The header's trailing marker tells you which side of the diff (or which non-diff view) the annotation targets:

- `(+)` — the new (added or context) side of a line, range, or hunk.
- `(-)` — the old (removed) side of a line or range.
- `(=)` — the current worktree file content, not a diff side at all (see below). There is no `+`/`-` to report because this view shows no diff.
- No marker at all — a whole-file annotation (`Target::File`), diffed or not.

## The `Reviewing:` metadata line

Annotations authored against the default working-tree source are always emitted first, in the basic format above, with **no** metadata line — a session that never leaves the working-tree view is byte-identical to the format before source-grouping existed.

Annotations authored against any other source are grouped by that source (in order of first appearance, the working-tree group excluded since it's always first) and each group is preceded by exactly one metadata line of the form `Reviewing: <spec>`, where `<spec>` is:

- a commit: the short SHA (e.g. `Reviewing: abc1234`)
- a range: the range expression exactly as typed/selected (e.g. `Reviewing: main..feature`)
- the index: the literal word `staged` (e.g. `Reviewing: staged`)

Example mixed session (one working-tree annotation, then one against a historical commit):

```text
## src/lib.rs:10-20 (+)

[nit] extract this into a helper

Reviewing: abc1234

## src/auth/session.rs:44 (+)

[question] where does keystore get rotated?
```

## The `(=)` marker (current file content, not a diff side)

Annotations made in the read-only whole-file view (any file opened via the project search or fuzzy file finder, not just files with a diff) target `Target::WorktreeLine` or `Target::WorktreeRange` instead of `Target::Line`/`Target::Range`, and serialize with the `(=)` marker:

```text
## docs/notes.md:44 (=)

[question] should this doc mention the new flag?
```

The file view always reads live worktree content (never a historical revision), so a `(=)` annotation always composes with the working-tree group above: it is emitted in the same always-first, metadata-line-free group as ordinary working-tree diff annotations, never its own `Reviewing:` group. A `Target::File` (whole-file) comment made from the file view is unaffected by this section — it already has no side marker at all, diffed or not.
