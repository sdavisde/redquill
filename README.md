<p align="center">
  <img src="redquill-logo.png" alt="redquill logo" width="300" height="300">
</p>

<p align="center">
  A portable, efficeint tool for reviewing code
</p>

## Vision

redquill is the human checkpoint between agent output and commit. Every hunk in the working tree gets one of two verdicts:

- **Keep it** → stage it (file, hunk, or line granularity)
- **Fix it** → annotate it, and batch the annotations back to the agent as its next prompt

What makes this tool unique is **code intelligence during review**: a limited language-server client so that go-to-definition, find-references, and hover docs are one keystroke away from any symbol in the diff — because the most common problem when reviewing code: not understanding how the changes impact other areas in the codebase.

## Getting Started

1. Install the redquill application

```bash
brew install sdavisde/tap/redquill
```

2. Run `redquill` in the git repo you want to review
3. Press \` to open the git panel, and `?` to see the list of keybinds.
4. When viewing the diff, press `c` to leave a comment which can be piped out to an agent when the session ends.

## Core features

**v1 — the review loop**
- Working-tree diff viewer: all changed files in one scrollable multibuffer of collapsible per-file sections, unified view, syntax highlighting, word-level intra-line diff, file tree sidebar (hidden by default; opens when the git panel is focused)
- Hunk/line navigation: jump between files, hunks, and changed regions without touching the mouse
- Staging: stage/unstage at file, hunk, and line granularity; staging a file collapses its section out of the way (hidden panel by default; toggle it in)
- Annotations: comment on any line, range, hunk, or file; classify (issue / question / nit / praise); browse all annotations in one list
- Batch output: on quit, emit annotations as structured markdown to stdout (and optionally a file) so any agent or script can consume them
- Diff targets: working tree (default), staged, commit ranges, arbitrary refs

**v1.x — code intelligence** (the differentiator)
- Embedded LSP client with a deliberately small surface: go-to-definition, find-references, hover/signature docs
- Peek windows: results render in an overlay without leaving the diff
- Graceful degradation: no language server configured → everything else still works

**Later**
- Agent-side plugins (Claude Code / Codex / OpenCode) that launch redquill as an overlay and feed annotations straight back into the session, looping until the review comes back clean
- Review sessions persisted across runs; re-review shows only what changed since your last pass
- Blame gutter, git log context, jj support

## Interaction model

Vim-grammar keybindings, designed to be remappable (config layer planned — see `docs/config-layer.md`).

### Design principles

1. **Two verdicts, minimum keystrokes.** Every feature is judged by whether it speeds up keep-vs-send-back.
2. **Composability over integration.** stdout/stdin first; plugins are thin wrappers around the same interface. redquill should work with an agent nobody has heard of yet.
3. **Never block review on intelligence.** LSP is progressive enhancement. Slow or missing servers must never make the diff feel slow.
4. **The terminal is the product.** No web views, no daemon, no account. One static binary.
5. **Respect user git config.** redquill reads and writes repo state the same way `git` on their PATH would.

Quit with `q` and annotations print to stdout — pipe them anywhere:

```sh
redquill | claude -p "address this review feedback"
```

### Annotation output format (public API)

Each annotation renders as a header naming its target, a blank line, then a
`[classification] body` line (subsequent body lines follow unindented):

```text
## src/auth/session.rs:44 (+)

[question] where does keystore get rotated?
```

Headers vary by target granularity — `path:line`, `path:start-end`, or just
`path` for a whole-file comment — with a trailing `(+)`/`(-)` marking which
side of the diff a line/range/hunk refers to (a hunk comment always shows
`(+)`, since a hunk is anchored to its new-side span; a whole-file comment
has no side marker at all). Multiple annotations are separated by
exactly one blank line; the whole document ends with a single trailing
newline.

A session that never leaves the working-tree view renders exactly as above,
byte-for-byte, with no extra lines — this is unchanged and will stay
unchanged. Annotations made against any other diff target (a commit, an
explicit range, or the staged index) are grouped after the working-tree
annotations, each group preceded by exactly one metadata line:

- a commit: `Reviewing: <short-sha>`
- a range: `Reviewing: <range-as-typed>` (e.g. `Reviewing: main..feature`)
- the staged index: `Reviewing: staged`

so a script or agent consuming the output always knows which revision a
group's line numbers resolve against.

Annotations made in the read-only whole-file view (any file opened via
Project Search or the fuzzy file finder, not just files with a diff) use a
third marker, `(=)`, meaning "current file content, not a diff side" — there
is no `+`/`-` to report since the file view shows no diff at all:

```text
## docs/notes.md:44 (=)

[question] should this doc mention the new flag?
```

A `(=)` annotation always reads live worktree content, so it groups with the
working-tree annotations above (no `Reviewing:` line of its own), never as
its own group.

### Diff targets

Any diff shown in the multibuffer — working tree (default), staged, an
explicit range/ref, or a single commit opened from the git panel's History
tab — can be annotated. Staging and code-intelligence keys are only ever
shown and active for the working tree/staged targets they apply to; a
read-only or historical target simply omits those keys from the footer and
the `?` overlay rather than showing an inert one.

## Prior art

Standing on the shoulders of: **lazygit** (staging ergonomics), **revdiff** and **tuicr** (the annotate-to-agent loop and its output conventions), **Zed** (diff-viewer quality bar), **Helix** (LSP configuration model). redquill exists because no one tool combines staging, annotation, and code navigation in a single review surface.
