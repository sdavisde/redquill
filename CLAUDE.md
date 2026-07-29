# CLAUDE.md

Operational guide for agents working in this repo. Read README.md first — it owns the vision, feature scope, and design principles. This file covers how to work here; don't duplicate the README into it.


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

Git hooks are installed automatically by `cargo-husky` the first time `cargo test` builds dev-dependencies (scripts live in `.cargo-husky/hooks/`, checked into the repo). `pre-commit` runs `cargo fmt --check`; `pre-push` runs all four gates (build, test, clippy, fmt).

## Architecture map

Keep these boundaries clean; they're the seams for testing and for future work:

- `git/` — runs git commands, parses porcelain/diff output into typed structs. No TUI types leak in here.
- `diff/` — diff model: files, hunks, lines, intra-line word diff. Pure data + transforms; heavily unit-tested.
- `annotate/` — annotation model, persistence, stdout serialization.
- `lsp/` — server lifecycle + the three requests. Must be fully async and never block the render loop; missing/slow servers degrade silently.
- `ui/` — ratatui widgets, layout, event loop, keymap. Keymap is data (remappable), not hardcoded match arms scattered through widgets.
- `review/` — per-file review-status model (spec 08 Unit 3, docs/specs/08-spec-branch-review-mode/08-spec-branch-review-mode.md): pure `ReviewStatus` tri-state and transition functions, no TUI types. Persistence (`review-state.json`, blob-SHA reconciliation) lands as a `review::store` submodule in spec 08 task 4.0.
- `main.rs` — CLI args (working tree default, `--staged`, ref ranges, `--review`/`--base`, `-o file`), wiring.

## Conventions

Generic Rust discipline (error handling, layering, state design, testing, concurrency, subprocess hygiene, commit gates) lives in the imported file below and applies in full:

@docs/rust-best-practices.md

Project-specific rules on top of it:

- TDD applies to this repo's pure code: git output parsing, diff model, annotation serialization.
- Before adding a test, name the defect it would catch (Testing, in the imported file above, is the bar). This suite has been pruned once already for tests that echoed tables, pinned cosmetic rendering, or verified call arguments — don't regrow them.
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

- **Local repo writes: destructive ones go behind a confirm modal, and every op builds its argv from closed types.** Beyond that, redquill is a working tool sitting between an IDE and a review tool — it does what a developer reviewing changes needs it to do. The one operation that destroys unrecoverable work (the file restore on `d`) names the exact file in its confirmation; nothing destructive is ever reachable from a bare keypress.
- **Forge writes are a different matter — they land on other people's PRs, so the ceiling there is real and narrow.** Exactly two things: (a) submitting **one review** on the PR under review — its line comments, file comments, thread replies, and verdict — via the forge CLI, and only from behind the submit confirm modal (nothing is ever sent without that confirm); and (b) forced ref update and branch deletion confined structurally to `refs/heads/redquill/pr/*` (the fetch-on-open `+` refspec and the finished-review cleanup delete — the update path cannot name any other ref). Forbidden on the forge: PR/MR merge, close, or reopen; editing or deleting any forge comment (including your own already-published ones); thread resolve/unresolve; and any forge write outside the submit flow (there is no generic "run arbitrary api call" path reachable from a UI action). Forge **reads** are separate and unrestricted by this ceiling: the PR listing, thread import, and `gx`'s `gh pr view --web` / `glab mr view --web` (which hands the PR's URL to the platform's browser and writes nothing) are all fine.
- **What an agent working in this repo may run during a task is narrower than what the product offers its user.** Staging/unstaging only. Agents must not fetch, pull, push, restore, or commit against the user's repo state, even though the product offers those to its human user through the git panel — an agent-run task is not the same context as a user pressing a keybind. (Committing an agent's own task work under the commit gates is a separate, unchanged workflow.) Agents must not run `git worktree add`/`remove`/`prune` against the user's real repo either — `--review` testing happens only against scratch repos an agent creates itself in a tempdir. Likewise agents never invoke a forge write (no `gh`/`glab` review submit, comment, reply, verdict, or any `api` POST/PATCH/DELETE) and never fetch/force-update/delete a `redquill/pr/*` ref against the user's real repo — forge testing uses fakes and scratch repos only, and live-write dogfood proofs are performed by the user, never the agent.
- Branch/worktree read models and a `git switch` runner exist in `src/git/` (commit `9c98d97`) as the git layer for the ratified branch/worktree switcher — see spec 03, docs/specs/03-spec-branch-worktree-switcher.md — implemented on the `worktree-git-switcher` branch.
- Don't add dependencies casually — this ships as one lean static binary. Justify anything beyond the stack above in the PR/commit description.
- Don't invent new keybindings that conflict with the shared keymap tables in `src/ui/modal_keys.rs` (defaults in `src/ui/keymap.rs`); propose changes there so the `?` help overlay stays in sync.
- If a task seems to require a web view, daemon, or network call, stop and ask — it's out of scope by design.
