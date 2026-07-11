# redquill

**A terminal UI for reviewing agentic code — read it, question it, stage it, or send it back.**

Coding agents produce large working-tree diffs faster than humans can review them. The review loop today is fragmented: one tool to read the diff, another to stage what's good, a third window to answer "wait, who else calls this function?", and copy-paste to get feedback back to the agent. redquill collapses that into a single keyboard-driven surface.

## Vision

redquill is the human checkpoint between agent output and commit. Every hunk in the working tree gets one of two verdicts:

- **Keep it** → stage it (file, hunk, or line granularity)
- **Fix it** → annotate it, and batch the annotations back to the agent as its next prompt

Everything else in the tool exists to make those two verdicts fast and well-informed. The differentiating bet is **code intelligence during review**: a limited language-server client so that go-to-definition, find-references, and hover docs are one keystroke away from any symbol in the diff — because the most common reason a reviewer leaves their review tool is to answer "what does this touch?"

Zed's git panel and diff viewer are the quality bar for the review experience. lazygit is the quality bar for staging ergonomics. The annotate-and-send loop should feel native to Claude Code, Codex CLI, and OpenCode sessions.

## Core features

**v1 — the review loop**
- Working-tree diff viewer: unified and side-by-side, syntax highlighting, word-level intra-line diff, file tree sidebar
- Hunk/line navigation: jump between files, hunks, and changed regions without touching the mouse
- Staging: stage/unstage at file, hunk, and line granularity (hidden panel by default; toggle it in)
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

Vim-grammar keybindings, fully remappable. Draft default map:

| Key | Action |
|---|---|
| `j` / `k`, `Ctrl-d` / `Ctrl-u` | Move / scroll |
| `h` / `l`, `w` / `b` | Move / word-jump the column cursor (needed for `gd`/`gr`/`K`) |
| `]` / `[` | Next / previous hunk |
| `Tab` / `Shift-Tab` | Next / previous file |
| `t` | Toggle side-by-side view |
| `/` then `n` / `N` | Search |
| `c` | Comment on line (visual select `v` for ranges) |
| `space` | Stage/unstage hunk (line in visual mode) |
| `s` | Toggle staging panel |
| `` ` `` | Focus / unfocus the git panel |
| `gd` / `gr` / `K` | Go to definition / references / hover docs |
| `a` | Annotation list |
| `?` | Help |
| `q` / `Q` | Quit and emit annotations / quit and discard |

**Git panel** (while focused, after `` ` ``):

| Key | Action |
|---|---|
| `` ` `` | Return focus to the diff view |
| `j` / `k` | Move the panel cursor through CHANGES / UNTRACKED / STASHES |
| `Enter` | Open the cursor's file in the diff (stash / header rows: no-op) |

Layout sketch:

```
┌─ files ──────┬─ diff: src/auth/session.rs ────────────────────────────┐
│ M src/auth/  │  42 │  fn validate(token: &Token) -> Result<Claims> {  │
│  ▸ session.rs│  43 │-     let key = env::var("SECRET")?;              │
│  ▸ mod.rs    │  44 │+     let key = self.keystore.current()?;         │
│ A src/keys.rs│     │  ● [question] where does keystore get rotated?   │
│ M tests/…    │  45 │      decode(token, &key)                         │
│              │ ┌─ references: keystore.current() ── 3 results ──┐     │
│ [2 staged]   │ │ src/keys.rs:81   src/auth/mod.rs:12   tests/…  │     │
│ [4 notes]    │ └────────────────────────────────────────────────┘     │
└──────────────┴────────────────────────────────────────────────────────┘
```

## Architecture

- **Language:** Rust
- **TUI:** [ratatui](https://ratatui.rs) + crossterm — the de facto standard for modern review/diff TUIs; large ecosystem, immediate-mode rendering suits a diff viewer well
- **Git:** shell out to `git` for diffs/staging (matches what the ecosystem does; avoids libgit2 divergence from user config), `git2` crate only where it clearly wins
- **Syntax highlighting:** tree-sitter (accurate, incremental, and the same infrastructure the LSP layer benefits from); syntect acceptable as a stopgap
- **LSP:** thin JSON-RPC client (`lsp-types` crate) managing external servers per language, configured like Helix's `languages.toml`; only `definition`, `references`, `hover` in scope for v1.x
- **Annotation output:** structured markdown on stdout — `## path/to/file.rs:LINE (+)` header followed by the comment body — a format agents parse trivially and humans can read raw

### Design principles

1. **Two verdicts, minimum keystrokes.** Every feature is judged by whether it speeds up keep-vs-send-back.
2. **Composability over integration.** stdout/stdin first; plugins are thin wrappers around the same interface. redquill should work with an agent nobody has heard of yet.
3. **Never block review on intelligence.** LSP is progressive enhancement. Slow or missing servers must never make the diff feel slow.
4. **The terminal is the product.** No web views, no daemon, no account. One static binary.
5. **Respect user git config.** redquill reads and writes repo state the same way `git` on their PATH would.

## Installation (planned)

Follow ecosystem conventions from day one:

```sh
curl -fsSL redquill.dev/install.sh | sh   # or:
brew install sdavisde/tap/redquill
cargo install redquill
```

Prebuilt binaries (linux/darwin, amd64/arm64) attached to GitHub Releases.

## Usage

```sh
redquill                 # review the working tree
redquill --staged        # review the index
redquill main..HEAD      # review a range
redquill -o review.md    # also write annotations to a file
```

Quit with `q` and annotations print to stdout — pipe them anywhere:

```sh
redquill | claude -p "address this review feedback"
```

## Prior art

Standing on the shoulders of: **lazygit** (staging ergonomics), **revdiff** and **tuicr** (the annotate-to-agent loop and its output conventions), **Zed** (diff-viewer quality bar), **Helix** (LSP configuration model). redquill exists because no one tool combines staging, annotation, and code navigation in a single review surface.

## Status

Pre-alpha, but the v1 review loop is implemented and usable via `cargo run --`: diff viewer (unified and side-by-side), annotations with markdown-on-quit, and file/hunk/line staging. LSP peek (`gd`/`gr`/`K`) from the v1.x milestone is implemented too. Installation (prebuilt binaries, package manager taps) is still planned — for now, build from source. Roadmap order: diff viewer → annotations + stdout → staging → LSP peek → agent plugins.
