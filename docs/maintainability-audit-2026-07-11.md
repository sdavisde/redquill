# Maintainability Audit — 2026-07-11

Scope: the whole repo at commit `9169151` (`perf(diff): poll the working tree off the render thread`).
Method: three parallel deep-read audits (vision/UX, core layers, UI layer), synthesized here.
Deliverable: the abstraction and architecture changes that would most improve maintainability, plus every issue rated critical/high found along the way.

---

## 1. Executive summary

The codebase is in genuinely good shape. Layer boundaries are honored strictly (zero TUI types in core modules, zero git parsing in the UI), error-handling conventions are followed to the letter (no production `unwrap`/`expect` anywhere), the async design is exemplary (failures-as-values, `catch_unwind`, single-flight guards, generation counters against stale snapshots), and test discipline is real (tempdir integration repos, exhaustive unit tests on the pure layers).

**No critical code defects were found.** No data-loss risk in the staging path, no correctness bug in git parsing, no panic reachable from user input.

The maintainability risks are instead structural and contractual:

1. **`App` in `src/ui/app.rs` is the god-object** — 44 public/`pub(super)` fields, every subsystem funneling through two shared mutation points, and sibling modules reaching into its fields. It is the one place where each new feature makes the next one harder.
2. **The project's written contracts have drifted from the code** — the CLAUDE.md "write ceiling" forbids `push` while the shipped git panel offers it; spec 02 declares branch switching a non-goal while `git switch` support has landed; the README promises "fully remappable" keybindings that don't exist yet.
3. **Two stated invariants have partial enforcement gaps** — keymap-as-data is bypassed in five modal handlers, and the 5k-line performance bar has no automated guard.

None of these is urgent in isolation. Together they define the work that keeps the codebase matching its own documentation as the git-panel/multibuffer workstream continues.

---

## 2. The vision and the review loop (mental model)

Everything in the audit is judged against this model, so it's worth stating precisely.

redquill is **the human checkpoint between agent output and commit**. Coding agents produce working-tree diffs faster than humans can review them; redquill collapses the fragmented review loop (diff reader + staging tool + "who calls this?" IDE window + copy-paste back to the agent) into one keyboard-driven surface. Every hunk gets exactly one of two verdicts:

- **Keep it** → stage it (file / hunk / line granularity). Staging a file collapses its section out of the multibuffer — the buffer shrinks to only what still needs review.
- **Fix it** → annotate it (issue / question / nit / praise). On `q`, annotations emit as structured markdown on stdout: `redquill | claude -p "address this review feedback"`.

The ideal session: launch against the working tree → scroll one continuous multibuffer of collapsible per-file sections (`j`/`k` across file boundaries, `]`/`[` between hunks, `Tab` between files) → answer "what does this touch?" without leaving via `gd`/`gr`/`K` peek overlays → `c` to comment, `space`/`S` to stage (sections auto-collapse) → `` ` `` into the git panel for orientation and fetch/pull/push → `q` to quit and pipe the review back to the agent. Meanwhile a background poll picks up the agent's live edits every ~2s, and any staged file that gains new unstaged changes auto-expands — the **"nothing hides" guarantee**.

Load-bearing design constraints:

- **Two verdicts, minimum keystrokes** — every feature is judged by whether it speeds keep-vs-send-back.
- **The annotation stdout format is a public API** (stdout reserved for it; TUI on stderr).
- **LSP is progressive enhancement** — never blocks or slows the diff.
- **Instant feel on a 5k-line diff** is a hard regression bar.
- **Keymap is data**, remappable, with every action in the `?` overlay.
- **One static binary; shell out to `git` on PATH**; respect user config.

The differentiating bet is code intelligence during review. Worth noting: recent commit energy is almost entirely in git-panel/multibuffer parity (the Zed pivot), while the LSP differentiator is static — a portfolio observation, not a defect.

---

## 3. Findings — ranked

### 3.1 HIGH — `App` god-object in `src/ui/app.rs`

The raw file is 4,474 lines, but 67% is a single `#[cfg(test)]` module; production code is ~1,476 lines. The real problem is shape, not length:

- **44 fields on `App`** (`app.rs:64–215`), many `pub(super)` specifically so sibling modules (`code_intel`, `staging`, `git_panel`, `help`) can reach into them. That field-level coupling is the true cost — sibling modules depend on `App`'s internals, not on an interface.
- Every subsystem funnels through two shared mutation points: `rebuild_rows` (`app.rs:521`) and `apply_snapshot` (`app.rs:1053`).
- The recent async-refresh work (`9169151`) is structurally excellent in itself (mirrors the remote-op pattern, adds staleness guards) but added 3 more fields and a third poller to `App` — the concentration is still growing.

The responsibility map of the production lines: ~18% refresh subsystem, ~12% state definitions, ~8% row/highlight assembly, ~6% annotation targeting, ~6% annotation-list panel, ~6% remote/command-log, plus panel/compose/search glue that is already mostly thin delegation.

**Recommended decomposition, lowest-risk first** (`App` stays the aggregate root; extract cohesive method clusters as `pub(super) impl App` blocks in sibling files, or into owned sub-state structs):

1. **Split the test module out** to `app/tests.rs`. Near-zero risk; 4,474 → ~1,476 lines with no logic change. Do this first.
2. **Extract the refresh subsystem** (~260 lines, `app.rs:893–1151`: `refresh`, `rebuild_from_git`, `manual_refresh`, `auto_refresh`, `maybe_auto_refresh`, `spawn_auto_refresh`, `poll_refresh`, `apply_snapshot`, plus the `InFlight*` structs) into `app/refresh.rs`. Most independent cluster, heavily tested.
3. **Extract row/highlight assembly** (`refresh_rows`, `rebuild_rows`, `app.rs:505–619`) into `app/render_glue.rs`, isolating the `highlight_cache` + `build_multibuffer` seam.
4. **Move each panel's handlers into its existing module** (staging handlers → `staging.rs`, panel handlers → `git_panel.rs`, list handlers → a new `annotation_list.rs`). Combine with the cursor fix in §3.3 so each cursor moves with its handler.
5. **Relocate annotation targeting** (`target_for_cursor`, `hunk_target`, `target_for_visual`, `line_target`, `app.rs:642–707`) into `diff`/`annotate` (or `ui/targeting.rs`). This is the one genuine business-logic-in-UI chunk — annotation targets are a diff-model concept, and moving them makes that logic unit-testable without an `App`. Touched by Visual + Compose paths; do last.

After steps 1–4, `app.rs` lands at ~700–800 lines of genuine aggregate-root wiring — a defensible size for the central state object.

### 3.2 HIGH — written contracts have drifted from the code

Three separate documents now disagree with the code or each other. This is the highest-leverage *documentation* fix because agents (and humans) work from these files:

- **The write ceiling.** CLAUDE.md Guardrails: "Never run destructive git commands (… push); staging/unstaging is the write ceiling." But spec 02 shipped panel-scoped fetch/pull/push (`src/git/remote.rs`, README panel table), and `02-audit-git-panel.md` already flags and resolves the conflict by precedence. The implementation is well-guarded (closed enum, no `--force`, no shell, `GIT_TERMINAL_PROMPT=0`) — the *text* is what's stale. **Fix:** rewrite the guardrail to state the actual ceiling: index writes plus the three sanctioned plain remote ops; force-push/reset/checkout --/clean remain forbidden. Also draw the line the current text blurs: what an *agent may do during a task* vs. what the *tool offers its user*.
- **Branch switching.** Spec 02 lists it as a non-goal, yet `9c98d97` added branch/worktree read models and `git switch` (`src/git/runner.rs:150–153`, `src/git/branch.rs`). Either ratify the scope expansion in a spec/README update or mark the capability as staged-for-a-future-spec. Right now it's an undocumented write capability.
- **"Fully remappable" keybindings.** README sells this; `docs/config-layer.md` is an explicit unbuilt skeleton and both specs defer keymap config. **Fix:** soften the README claim to "designed for remapping (config layer planned)" until the config layer exists. Cheap honesty now beats a broken promise discovered by a user.

### 3.3 MEDIUM — mode-cursor state can drift; clamps are scattered

`App` carries four parallel cursor fields — `view.cursor` plus `list_cursor`, `staging_cursor`, `panel_cursor` (`app.rs:89, 119, 124`) — that live on `App` regardless of which mode is active. Inactive cursors can hold stale indices; correctness currently depends on ~15 scattered `.min(len-1)` / `saturating_sub` clamps (`app.rs:782–1246` passim). This is the main state-drift hazard in the UI.

**Fix:** move each cursor into its `Mode` variant payload (`List { cursor }`, `Staging { cursor }`, `Panel { cursor }`), matching the existing `Visual { anchor }` precedent. An inactive panel then *cannot* carry a stale index, and the clamp collapses to one place per panel. Do this together with decomposition step 4 above.

Related: overlay state is spread across `mode` plus two orthogonal booleans (`help_open`, `command_log_open`), so "is an overlay up?" consults three places (the recent `q`-inert-over-overlays commit had to do exactly this). Acceptable, but worth a single `overlay_active()` helper so the check can't be done inconsistently.

### 3.4 MEDIUM — keymap-as-data is bypassed in five modal handlers

Normal/Visual/Panel dispatch is fully data-driven through `Keymap::resolve` — good. But `handle_compose_key`, `handle_search_key`, `handle_peek_key`, `handle_list_key`, `handle_staging_key` (`modes.rs`) and `handle_help_key` (`mod.rs:212`) hardcode keys. For Compose/Search (free-text input) that's justified and documented. For **List (`j/k/e/d/a`), Staging (`j/k/Space/s`), and Help (`j/k/g/G`)** the keys are one-action-per-key, expressible as bindings, and currently not remappable — a partial violation of the stated invariant.

Compounding it: the help overlay's coverage test (`help.rs:344`) only checks keymap bindings; the modal-mode hints are hand-maintained string literals in `help::render` (`help.rs:203–229`). Adding a key to `handle_list_key` without updating the help text fails no test.

**Fix (incremental):** first, introduce per-mode `const` tables of `(key, action, description)` consumed by *both* the modal handlers and `help::render`, with a test cross-checking them — this closes the drift gap immediately without touching dispatch. Then, when the config layer lands, fold those tables into `Keymap` scopes so the modal keys become remappable like everything else.

### 3.5 MEDIUM — the 5k-line performance bar has no automated guard

Both spec audits (`02-audit`, `03-audit`) flag that "instant feel on a 5k-line diff" and "no dropped frames during remote ops" are validated only by manual smoke transcripts. The recent perf commits show the bar is being defended reactively.

**Fix:** add a benchmark-style integration test that generates a synthetic 5k-line diff in a tempdir repo and asserts wall-clock budgets on the hot paths that don't need a terminal: full `rebuild_rows`/`build_multibuffer`, `apply_snapshot`, highlight-cache population for one file, and cursor/hunk navigation over the built rows. Not a frame-rate test — a regression tripwire on the operations whose cost determines frame time.

### 3.6 LOW — core-layer polish (all cheap, none urgent)

- **Duplicated git error plumbing:** `map_spawn_err` defined verbatim in `src/git/runner.rs:181–187` and `src/git/stage.rs:104–110`; the non-zero-exit → `GitError::Command` construction duplicated between `runner.rs:67–77` and `stage.rs:113–129`. Hoist both into `git/error.rs` helpers.
- **`unreachable!` in the stage patch builder** (`src/git/stage.rs:330`): genuinely unreachable today, but it's a panic macro in the library that writes to the user's index, and the conventions forbid it. Replace with a defensive arm or `GitError::Parse` — zero behavior change.
- **LSP `dispatch_now` does a synchronous `fs::read_to_string`** on the render path (`src/lsp/manager.rs:391`). Bounded (single file, only on an explicit keypress), but it contradicts the module's "never blocks the render loop" doc. Either note the exception in the doc or move the read to the writer thread.
- **Unbounded mpsc channels** in `lsp/manager.rs:140` and `lsp/transport.rs:74`. Fine at LSP traffic volumes; document the assumption that `poll()` runs every frame.
- **`rebuild_rows` reconstructs the entire multibuffer on a single-file collapse toggle** (`rows.rs:303`). Fine at 5k lines; first thing to bite at very large file counts. Note it; don't fix it until the perf tripwire (§3.5) says so.
- **`diff` imports `git::RawFilePatch`** (`src/diff/file.rs:4`) — a deliberate, defensible directional coupling (git owns raw output; diff consumes it). No change recommended; recorded so nobody "fixes" it casually.

### 3.7 Portfolio observation — the differentiator is idle

Not a code finding, but the audit would be incomplete without it: the README names LSP code intelligence as *the* differentiating bet, and recent months of commits are almost entirely Zed-parity git-panel/multibuffer work, including capabilities (branch switch, remote ops) at or past the edge of the stated scope. The panel work is high quality and the pivot was deliberate (2026-07-10), but each parity feature added to the panel is a feature lazygit already has; nobody else has review-time LSP peek in a diff multibuffer. When the git-panel workstream completes, the roadmap should swing back to the differentiator rather than continuing toward general-git-client parity.

---

## 4. Recommended sequence

| Order | Work | Findings addressed | Risk |
|---|---|---|---|
| 1 | Update CLAUDE.md write ceiling, ratify/branch-switch scope, soften README remap claim | 3.2 | none (docs) |
| 2 | Split `app.rs` test module out; same for `ui/mod.rs` if desired | 3.1 step 1 | near-zero |
| 3 | Core-layer polish: dedupe git error helpers, remove `unreachable!` | 3.6 | trivial |
| 4 | Shared per-mode key tables + help cross-check test | 3.4 | low |
| 5 | Extract refresh subsystem, then row/render glue from `app.rs` | 3.1 steps 2–3 | low–medium |
| 6 | Move panel handlers into their modules + cursors into `Mode` variants | 3.1 step 4, 3.3 | medium |
| 7 | Performance tripwire test (5k-line synthetic diff) | 3.5 | low |
| 8 | Relocate annotation targeting out of `App` | 3.1 step 5 | medium |

Each step leaves the tree green (`cargo build && cargo test && cargo clippy -- -D warnings && cargo fmt --check`) and is a self-contained conventional commit.

---

## 5. What was checked and found clean

For the record, the audit explicitly verified and found **no issues** in: layer-boundary imports (no `ratatui`/`crossterm`/`ui` types in core; no `Command::new("git")` in production UI code — `stage_ops.rs` is a clean trait seam over `crate::git`); production `unwrap`/`expect` (zero, repo-wide); staging correctness (`build_line_patch` count recomputation, `--cached`-only index writes, never touches the working tree); porcelain v2 parsing (renames, spaces, conflicts, NUL separation); annotation stdout format vs. the documented public API; highlight-cache invalidation (per-path byte-equality, `retain_paths` bounding); async re-entry safety (single-flight, generation counters, snapshot-drop when the user is mid-Compose/Search/Visual, `catch_unwind` on background tasks); integration-test hygiene (all tempdir, no host-repo access); and dependency hygiene (every crate beyond the sanctioned stack justified: `clap`, `serde`/`serde_json` for JSON-RPC, per-language tree-sitter grammars, `tempfile` dev-only).
