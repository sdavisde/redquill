# CLAUDE.md

Operational guide for agents working in this repo. Read README.md first — it owns the vision, feature scope, keybinding map, and design principles. This file covers how to work here; don't duplicate the README into it.


## Contributing

If you are a Fable-model agent, you should not be contributing directly. You should be designating all of your implementation responsibilities to a Sonnet sub-agent.

## What this is

redquill: a Rust TUI for reviewing agentic code changes. Diff viewer + annotations piped to stdout + git staging + limited LSP navigation. Single static binary, terminal only.

## Stack (decided — don't relitigate without asking)

- Rust, stable toolchain, edition 2024
- ratatui + crossterm for the TUI
- Shell out to `git` on PATH for diff/staging/status (respect user git config); avoid libgit2 unless there's a clear, isolated win
- tree-sitter for syntax highlighting
- LSP: external servers over JSON-RPC using the `lsp-types` crate; scope is definition, references, hover only
- Annotation output: markdown records on stdout (`## path/to/file.rs:LINE (+)` header, comment body below) — treat this format as a public API once shipped

## Commands

```sh
cargo build                # debug build
cargo run -- [args]        # run against the current repo's working tree
cargo test                 # unit + integration tests
cargo clippy -- -D warnings
cargo fmt --check
```

All four of build/test/clippy/fmt must pass before considering any task done.

## Architecture map

Keep these boundaries clean; they're the seams for testing and for future work:

- `git/` — runs git commands, parses porcelain/diff output into typed structs. No TUI types leak in here.
- `diff/` — diff model: files, hunks, lines, intra-line word diff. Pure data + transforms; heavily unit-tested.
- `annotate/` — annotation model, persistence, stdout serialization.
- `lsp/` — server lifecycle + the three requests. Must be fully async and never block the render loop; missing/slow servers degrade silently.
- `ui/` — ratatui widgets, layout, event loop, keymap. Keymap is data (remappable), not hardcoded match arms scattered through widgets.
- `main.rs` — CLI args (working tree default, `--staged`, ref ranges, `-o file`), wiring.

## Conventions

Generic Rust discipline (error handling, layering, state design, testing, concurrency, subprocess hygiene, commit gates) lives in the imported file below and applies in full:

@docs/rust-best-practices.md

Project-specific rules on top of it:

- TDD applies to this repo's pure code: git output parsing, diff model, annotation serialization.
- Every user-visible action must be reachable from the keymap and listed in the `?` help overlay — no hidden features. Modal key handlers and help hints are driven from the shared tables in `src/ui/modal_keys.rs`; add keys there, never as loose match arms.
- Performance target: instant feel on a 5k-line diff; if a change makes scrolling or hunk-jumping perceptibly slower, it's a regression. The wall-clock tripwire tests in `src/ui/perf_tests.rs` enforce the complexity class — keep them passing, don't loosen budgets to make a regression fit.

## Roadmap order (work in this order unless told otherwise)

1. Diff viewer: parse `git diff`, render unified view with syntax highlighting, file sidebar, hunk/file navigation
2. Annotations: comment on line/range/hunk/file, annotation list panel, emit markdown to stdout on `q`
3. Staging: file/hunk/line stage-unstage, toggleable staging panel
4. Side-by-side view, search, themes
5. LSP peek: definition/references/hover overlays
6. Agent plugins (Claude Code first), persisted review sessions

## Guardrails

- **The tool's write ceiling** (what the shipped product may do to the user's repo at runtime) is index writes (stage/unstage) plus the three sanctioned plain remote ops — fetch, pull, push — shipped in the git panel per spec 02. `push` is never `--force`. Forbidden, always: force-push, `reset --hard`, `checkout --`, `clean`, and any other destructive or history-rewriting operation.
- **What an agent working in this repo may run during a task** is narrower than the above: staging/unstaging only. Agents must not fetch, pull, or push on the user's behalf, even though the product offers those operations to its human user through the git panel — an agent-run task is not the same context as a user pressing a panel keybind.
- Branch/worktree read models and a `git switch` runner exist in `src/git/` (commit `9c98d97`) as the git layer for the ratified branch/worktree switcher — see spec 03, docs/specs/03-spec-branch-worktree-switcher.md — implemented on the `worktree-git-switcher` branch.
- Don't add dependencies casually — this ships as one lean static binary. Justify anything beyond the stack above in the PR/commit description.
- Don't invent new keybindings that conflict with the README's map; propose changes to the map in README.md itself.
- If a task seems to require a web view, daemon, or network call, stop and ask — it's out of scope by design.
