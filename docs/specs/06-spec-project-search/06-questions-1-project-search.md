# 06 Questions Round 1 - Project Search

Please answer each question below (select one or more options, or add your own notes). Feel free to add additional context under any question.

Context: this spec covers two features — a project-wide text search ("Project Search", Zed cmd+shift+f analog) and a fuzzy file finder (Zed cmd+p analog). Backend benchmarks were run on this machine (Apple M2) against real corpora (rust-analyzer, 2,336 files; TypeScript repo, 81,369 files as an extreme outlier); numbers cited below are medians from those runs.

## 1. Does the LSP error-messaging improvement fold into this spec?

You also asked for better LSP failure messaging (e.g. "TypeScript language server is not installed" instead of "No code intelligence here", possibly a modal with a copyable install command). Should that ship inside spec 06 or separately?

- [ ] (A) Separate small spec (07) — keep spec 06 purely about search/navigation
- [ ] (B) Fold into spec 06 as an additional demoable unit
- [x] (C) Skip the spec process for it — implement directly as a small standalone change
- [ ] (D) Other (describe)

**Recommended answer(s):** [(A)]

**Why these are recommended:**

- The LSP messaging work touches a completely different subsystem (`src/lsp/` transport/manager/events + one UI module) and shares no code, UI surface, or validation story with search. Folding it in makes spec 06's demoable units and validation muddier without any synergy.
- It is small but not trivial: un-nulling server stderr, distinguishing "server missing/crashed" from "no results" in the event model, a status message, and (if you want it) an install-hint modal with a copy action — enough moving parts that a small spec keeps it honest, and it can proceed in parallel with spec 06.
- (C) is defensible if you'd rather have it fixed this week than speced; the risk is the modal + keymap + help-overlay obligations get done ad hoc.

## 2. What surface does Project Search live on?

You described it as a "Project Search tab." In redquill, the only existing tabs (Changes/History) live inside the narrow right sidebar panel — too narrow for `path:line  matched text` rows. Zed's project search actually occupies the full editor pane.

- [x] (A) Full-screen search view: query input on top, results list below, occupying the main content area; Esc returns to the diff exactly where you were
- [ ] (B) A third tab inside the right sidebar git panel (narrow column)
- [ ] (C) Other (describe)

**Recommended answer(s):** [(A)]

**Why these are recommended:**

- (A) matches what Zed actually does with cmd+shift+f (the results consume the main pane) and gives results room for file grouping and match previews.
- (B) fits the literal word "tab" but the sidebar is ~40 columns wide — unusable for grep results.
- The suspend/restore mechanism the History tab already uses (open commit → Esc back) is reused for (A), so "feels like a tab you can leave and return from" is preserved.

## 3. What happens when you select a search result?

This is the decision that drives most of the scope. Every view in redquill today renders a diff; most grep hits will be in files with no diff at all.

- [x] (A) Open a read-only whole-file view scrolled to the hit line, with syntax highlighting; Esc returns to the search results (and Esc again to the diff)
- [ ] (B) Navigation-only v1: search results are browsable with inline match context, but selecting a result does not open the file yet (follow-up spec adds opening)
- [ ] (C) Jump into the existing diff view when the hit file is part of the current diff; show "not in current diff" otherwise
- [ ] (D) Other (describe)

**Current best-practice context:** Spec 05 already added whole-file content plumbing (`show_file`, `read_worktree_file`) for syntax highlighting, and untracked files are already rendered by synthesizing a whole-file diff — so (A) is buildable from existing seams, but it is the only part of this spec that is not a copy of an existing pattern (new read-only diff-target variant, capability gating so stage/commit keys disable correctly).

**Recommended answer(s):** [(A)]

**Why these are recommended:**

- (A) is the entire point of your use case — "I see a function in a PR and want to chase it through the codebase." Without opening files, search is a dead end; with (C), most hits (anything outside the diff) are dead ends.
- (A) is the highest-risk unit in the spec, but it is shared by both features (the file finder needs the same "open a file that has no diff" behavior), so it pays for itself twice.
- (B) is the honest fallback if you want to ship the search UI sooner and take file-opening as a fast follow — but it makes the first release feel broken for the primary journey.

## 4. Can you annotate lines in a searched (no-diff) file?

Annotations are redquill's core output (markdown on stdout, a public format). Should the read-only file view opened from search support annotating lines?

- [ ] (A) No — read-only navigation in v1; annotations stay diff-scoped (revisit later)
- [x] (B) Yes — annotate any line in any opened file; the stdout format gains a marker distinguishing non-diff annotations
- [ ] (C) Other (describe)

**Recommended answer(s):** [(A)]

**Why these are recommended:**

- The stdout markdown format is treated as a public API once shipped; (B) extends that contract and deserves its own deliberate design (how does a consumer distinguish "comment on changed line" from "comment on unrelated context file"?) rather than riding along.
- (A) keeps spec 06's scope to search + navigation, which is what you asked for. If you find yourself wanting (B) while dogfooding, it becomes a clean follow-up spec.

## 5. Text-search backend

Benchmarks on this machine: `git grep -n -I` completes fully (spawn to all results parsed) in 27–31ms on a 2,336-file repo across common/rare/pathological queries; `rg` is comparable (29–43ms) and pulls ahead on larger trees; embedding ripgrep's library crates (`grep-searcher` + `ignore`) saves only ~3–5ms of process-spawn time while adding ~2MB to the binary and 21 transitive dependencies. Ripgrep 15.1 is installed on your machine but is not guaranteed present for other users. Helix itself shells out to the `rg` binary rather than embedding the crates.

- [ ] (A) Fallback chain, zero new dependencies: use `rg --json` when ripgrep is on PATH, else `git grep -n -I --untracked` — both streamed from a background thread
- [ ] (B) `git grep` only — one code path, zero assumptions beyond git itself
- [x] (C) Embed the ripgrep crates in-process (+2MB binary, 21 deps, ~3ms faster)
- [ ] (D) Other (describe)

**Current best-practice context:** All subprocess options are ~3× under a 100ms "instantaneous" bar at your stated scale, and with streamed output the first results render within single-digit milliseconds of the scan starting. The 81k-file outlier corpus took ~1.3s with every backend, including the embedded crates — at that scale the work is I/O-bound and backend choice does not differentiate; result caps and streaming are the mitigation.

**Recommended answer(s):** [(A)]

**Why these are recommended:**

- (A) gets the best available speed on every machine with zero new dependencies, consistent with the repo's "shell out to git on PATH" principle and its lean-binary guardrail. `rg` additionally searches untracked-but-unignored files by default — the right universe when reviewing agentic changes that create new files (the `git grep` fallback gets `--untracked` for parity).
- (B) is simpler (one parser instead of two) at a modest cost in large-tree speed; a reasonable second choice if you'd rather not maintain the rg JSON parser.
- (C) fails the dependency guardrail while not advancing your speed goal — the benchmark shows the spawn cost it eliminates is noise next to the debounce interval. Its author describes the crates as not ready for wide library use.

## 6. Fuzzy-matching engine for the file finder

Benchmarks: `nucleo-matcher` (the engine behind Helix and Television) ranks 2,336 paths in under 1ms and 81k paths in ~10–30ms; it is 2–6× faster than `fuzzy-matcher` (unmaintained since 2020) with best-in-class fzf-style ranking, 3 transitive deps, and negligible binary-size impact. A hand-rolled subsequence scorer is faster still in raw terms but ranks results visibly worse (no optimal alignment, weaker filename bonuses). File list sourced from `git ls-files -z` (5.5ms at 2.3k files, ~6× faster than a directory walker at 81k files).

- [x] (A) Add `nucleo-matcher` as a dependency (MPL-2.0 license — file-level copyleft; linking is fine and does not affect redquill's own license, but note redquill currently declares no license)
- [ ] (B) Hand-rolled smartcase subsequence scorer — zero dependencies, noticeably worse ranking quality
- [ ] (C) Other (describe)

**Recommended answer(s):** [(A)]

**Why these are recommended:**

- Ranking quality is what makes a fuzzy finder feel good — fzf-consistent scoring is the difference between the file you meant appearing first vs. fifth. This is the one place in this spec where a dependency buys real user-visible quality at near-zero size cost, which satisfies the repo's "justify every dependency" bar.
- (B) is the right call only if you want a strict zero-dependency policy or the MPL-2.0 license is a problem; speed is not a differentiator (all options are sub-frame at your scale).

## 7. Keybindings

Terminals cannot see cmd-based chords (cmd+shift+f / cmd+p never reach the app), so terminal-friendly defaults are needed. Currently free: `Ctrl-p` (everywhere), `Ctrl-f` (diff scope), and the `g` prefix already dispatches two-key sequences (`gg`/`gd`/`gr`).

- [ ] (A) `Ctrl-p` opens the file finder; `g/` opens Project Search (extends the existing `g`-prefix family; mnemonic next to the local `/` search)
- [ ] (B) `Ctrl-p` opens the file finder; `Ctrl-f` opens Project Search (closer to a GUI-editor reflex, but collides with vim/less "page forward" muscle memory alongside the existing Ctrl-d/Ctrl-u scrolling)
- [x] (C) Other (describe) lets use g/ for grepping and gp for project file search

**Recommended answer(s):** [(A)]

**Why these are recommended:**

- `Ctrl-p` is the canonical fuzzy-finder chord (fzf, ctrlp.vim, Telescope) and is free in every scope.
- `g/` reuses the existing two-key dispatch machinery with zero conflicts, and reads as "go search the project" next to `gd`/`gr`; since you navigate in neovim, `Ctrl-f` = page-forward is a reflex worth not fighting.
- Both land in the shared keymap tables and the `?` help overlay per the repo guardrail.

## 8. Match semantics for Project Search v1

- [ ] (A) Literal (fixed-string) smartcase search — case-insensitive unless the query contains an uppercase letter; regex and whole-word toggles are explicit non-goals for v1
- [x] (B) Regex search from day one, with toggles for case sensitivity / whole word
- [ ] (C) Other (describe)

**Recommended answer(s):** [(A)]

**Why these are recommended:**

- Your driving use case is pasting/typing an identifier you saw in a diff — literal matching serves it exactly, avoids regex-escaping surprises (`foo()` as a regex is not what you typed), and matches the smartcase convention the in-diff `/` search already uses.
- (A) keeps the v1 UI to a single input with no toggle keys; regex/word toggles are an easy additive follow-up once the surface exists, and both backends (`rg -F`, `git grep -F`) support literal mode natively.
