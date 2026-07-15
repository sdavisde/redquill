# 06-spec-project-search.md

## Introduction/Overview

While reviewing a diff, a reviewer constantly needs to chase context that is not in the diff: a function referenced by a changed line, a config key, a file they half-remember the name of. Today redquill offers no way to do this — the reviewer has to open a separate editor, which breaks the review flow. This spec adds two navigation features: **Project Search** (`g/`) — a full-screen, live, regex-capable text search across the project — and a **fuzzy file finder** (`gp`) — an overlay for jumping to a file by name. Both land on a shared new capability: a **read-only whole-file view** that can display any file in the repository (not just files with a diff), with annotations allowed on its lines.

The performance bar, set by the user and validated by benchmarks on this machine: search must feel **instantaneous on codebases with thousands of files** — first results visible well under 100ms.

## Goals

- From any diff view, find every occurrence of a text/regex pattern across the project and browse results without leaving redquill.
- Jump to any file in the repository by fuzzy name match in a couple of keystrokes.
- Open any search/finder result in a read-only, syntax-highlighted whole-file view at the target line, and return to exactly where you were with `Esc`.
- Annotate lines in files that have no diff, emitted through the existing stdout markdown contract with a new `(=)` marker.
- First search results render in <100ms on a ~5,000-file repository (benchmarked headroom: full scans complete in ~25–30ms on a 2,336-file corpus).

## User Stories

- **As a reviewer of an agentic change**, I want to grep the project for a function I see referenced in the diff so that I can judge whether the change is used correctly, without opening a second terminal with neovim.
- **As a reviewer**, I want to filter search results live as I refine my query so that I can narrow thousands of hits down to the relevant handful.
- **As a reviewer**, I want to open a file by fuzzy-typing part of its name so that I can check a related module the diff doesn't touch.
- **As a reviewer**, I want to open a search hit, read the surrounding code with syntax highlighting, and press `Esc` to get back to my review so that exploration never costs me my place.
- **As a reviewer**, I want to leave an annotation on a line the diff doesn't touch (e.g. "this caller also needs updating") so that my review output captures findings outside the changed lines.

## Demoable Units of Work

### Unit 1: Fuzzy file finder + read-only file view (`gp`)

**Purpose:** The smaller feature ships first and carries the shared foundation — the read-only whole-file view — that Unit 2 also needs. Serves reviewers who know roughly which file they want.

**Functional Requirements:**

- The system shall open a fuzzy file finder overlay when the user presses `gp` in the diff view (two-key sequence, joining the existing `gg`/`gd`/`gr` family in the shared keymap tables).
- The system shall populate the candidate list from `git ls-files -z` plus `git ls-files -z --others --exclude-standard` (tracked + untracked-but-unignored), loaded on a background thread — never blocking the render loop.
- The system shall rank candidates with `nucleo-matcher` (path-aware scoring config) as the user types, re-ranking on every keystroke; ranking completes in well under one frame at the target scale (benchmarked: <1ms at 2,336 paths).
- The user shall navigate results with the same motion keys used by existing pickers, and `Esc` shall close the finder returning to the prior view unchanged.
- On `Enter`, the system shall open the selected file in a **read-only whole-file view**: full worktree file content, syntax highlighted, navigable with the existing scroll/jump motions; staging, commit, and other diff-mutating keys are inert and hidden from the footer in this view (capability-gated, following the spec 05 pattern).
- `Esc` from the file view shall return to wherever the user was (finder already closed → back to the diff view, restoring cursor/scroll position via the existing suspend/restore mechanism).
- All new keys shall live in the shared keymap/modal-key tables and appear in the `?` help overlay, with the existing bidirectional drift-test pattern extended to the new mode.

**Proof Artifacts:**

- Test: unit tests for the file-list parser (`-z` NUL splitting) and ranking glue pass, demonstrating the pure core is TDD-covered.
- Test: modal-key drift tests for the finder mode pass, demonstrating every finder key is documented and every documented key acts.
- CLI: recorded acceptance journey (persisted under `proofs/`) — open redquill on a real repo, `gp`, type a partial name, open the file, scroll, `Esc` back to the diff with position intact — demonstrating the end-to-end flow a user actually performs.

### Unit 2: Project Search (`g/`) — live full-screen search

**Purpose:** The headline feature: project-wide live text/regex search with results streaming into a full-screen view. Serves the "chase a reference seen in a PR" journey.

**Functional Requirements:**

- The system shall open a full-screen search view when the user presses `g/` in the diff view: a query input line on top, a scrollable results list below, grouped by file with `path:line` and the matched line text (match span visually emphasized).
- The system shall execute searches **in-process** using the ripgrep engine crates (`grep-searcher` + `grep-regex` + `ignore`): parallel, `.gitignore`-respecting walk of the worktree including untracked-but-unignored files, skipping binary files. (Deliberate, user-ratified deviation from the zero-dependency default — see Technical Considerations.)
- The system shall treat the query as a **regex by default**, with smartcase (case-insensitive unless the query contains an uppercase letter), and shall provide toggles for case sensitivity, whole-word, and regex-vs-literal, with the active toggle states always visible in the search view.
- The system shall search live as the user types: debounced (~120–150ms), minimum query length 2, each keystroke bumping a generation counter; results from a superseded query generation shall be dropped, and the in-flight scan shall be cancelled promptly via an abort flag checked in the search sink.
- The system shall stream results incrementally over a bounded channel drained once per tick (the existing background-task pattern), so first hits render while the scan continues; total collected results shall be capped (default 10,000 hits) with an explicit "capped — refine your query" indicator when truncation occurs.
- An invalid regex shall show an inline, non-blocking error under the input (no results wiped until a valid query produces new ones); it shall never panic.
- On `Enter` on a result, the system shall open the Unit 1 read-only file view scrolled to the hit line (cursor on it); `Esc` shall return to the search view with query, toggles, results, and selection intact; `Esc` from the search view shall return to the diff view position the user left.
- Search shall always scan the **worktree on disk** (current checked-out content), regardless of which diff source is being reviewed.
- The wall-clock perf tripwire suite shall gain a search-path test in the style of `src/ui/perf_tests.rs`, enforcing the complexity class of query→first-results on a generated corpus.

**Proof Artifacts:**

- Test: engine unit tests (query→matches on a tempdir corpus: regex, smartcase, whole-word, literal mode, binary-skip, gitignore-respect, cap behavior, cancellation) pass, demonstrating the search core meets its contract.
- Test: perf tripwire passes, demonstrating the instant-feel complexity class is enforced, not aspirational.
- CLI: recorded acceptance journey (persisted under `proofs/`) — review a diff, `g/`, type an identifier from the diff, watch results stream, refine query, toggle whole-word, open a hit in a file the diff doesn't touch, `Esc` `Esc` back to the diff — demonstrating the primary user journey end to end with timing notes.

### Unit 3: Annotations on non-diff lines + stdout `(=)` marker

**Purpose:** Makes exploration actionable: findings in unchanged files become part of the review output. Serves consumers of redquill's stdout contract (agents reading the review).

**Functional Requirements:**

- The user shall annotate a line or range in the read-only file view using the same annotation keys as the diff view.
- The system shall serialize such annotations with a new side marker `(=)` — `## path/to/file.rs:44 (=)` (and `:10-20 (=)` for ranges) — meaning "current file content, not a diff side"; existing `(+)`/`(-)`/file-target header shapes are byte-for-byte unchanged.
- The stdout format documentation (module doc of `src/annotate/markdown.rs` and any README examples) shall be updated in the same change; the format is a public API, so serialization is covered by byte-exact tests.
- Annotations on non-diff lines shall appear in the annotation list panel alongside diff annotations, navigable back to their file-view location.

**Proof Artifacts:**

- Test: byte-exact serialization tests for `(=)` line and range headers pass, and existing serialization tests pass unmodified, demonstrating the public API is extended without breaking existing consumers.
- CLI: stdout capture from an acceptance run (persisted under `proofs/`) showing a mixed review — diff annotations and a `(=)` annotation — demonstrating the full output contract in one artifact.
- Round-trip: an agent consumer reads the captured output and correctly distinguishes the non-diff annotation (same consumer-half pattern used for spec 05 proofs).

## Non-Goals (Out of Scope)

1. **Search-and-replace / editing**: redquill remains read-only outside its sanctioned git operations; the file view never edits files.
2. **Searching historical trees**: search and the file finder always operate on the worktree on disk, even when reviewing a commit from the History tab. Searching a commit's tree is a possible follow-up.
3. **LSP integration in the file view**: `gd`/`gr`/`K` remain gated to working-tree diffs; extending code intelligence to the file view is future work.
4. **Persisted search history** across sessions.
5. **A sidebar-tab variant** of search (ruled out in questions round 1 — the sidebar is too narrow).
6. **Regex features beyond the engine**: `grep-regex` is a finite-automata engine — no backreferences or lookaround; this is accepted (same engine and limits as ripgrep itself).
7. **Non-git directories**: redquill only runs inside git repositories; the search walk assumes the repo root.

## Design Considerations

- **Project Search** is a full-screen view (Zed-like): one query input line (with toggle-state indicators, e.g. `[re] [Cc] [w]`), results grouped under file-path headings, count summary ("1,204 matches in 87 files"), and a footer showing the mode's keys. Entering it, opening results, and `Esc`-unwinding must preserve every prior position (search view state included) — the "feels like a tab" quality the user asked for comes from lossless suspend/restore, per the History tab's mechanism.
- **File finder** is a centered modal overlay in the style of the branch/worktree switcher: input on top, ranked list below, match positions highlighted.
- **Read-only file view** reuses the diff rendering surface (a whole-file, all-context body with syntax highlighting — same seam that already renders untracked files as synthesized whole-file diffs). Footer must not advertise staging/commit keys; annotation keys are advertised (Unit 3).
- Proposed in-mode toggle keys (confirm at spec review, final say at implementation review): `Alt-c` cycle case mode, `Alt-w` whole-word, `Alt-r` regex↔literal. Alt-chords avoid collisions with text input and are deliverable by crossterm.

## Repository Standards

- All four gates before every commit: `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`; conventional commits; refactors separate from behavior.
- TDD for the pure code: file-list parsing, ranking glue, search-engine contract tests, `(=)` serialization.
- Keymap/help guardrail: every new action in the shared tables (`src/ui/keymap.rs`, `src/ui/modal_keys.rs`) with drift tests; nothing hidden.
- Module boundaries: the search engine and file-list/fuzzy ranking live in a new non-UI module (e.g. `src/search/`) with no TUI types; `ui/` holds only the modes, rendering, and glue — matching the `git/`/`diff/`/`ui/` layering discipline.
- Background-work discipline: bounded channels, `try_recv` drain per tick, single-flight + generation counters, no blocking the render loop (patterns copied from the History loader).
- Perf tripwires in the established style: debug-build measured, 10–20× budget, loop-amortized.

## Technical Considerations

- **New dependencies (all justified in the commit that adds them):**
  - `grep-searcher`, `grep-regex`, `ignore` — the ripgrep engine, embedded. **This is a deliberate, user-ratified deviation from the repo's lean-dependency default** (questions round 1, Q5): measured cost ~+2MB binary / 21 transitive deps for ~3–5ms over subprocess `git grep` on realistic corpora. The user chose it for maximum speed with regex, smartcase, and whole-word natively, in-process cancellation mid-scan, no reliance on any external binary, and structured match data without output parsing. Benchmarks (Apple M2): in-process full scan 25–26ms on a 2,336-file corpus vs 27–31ms `git grep`; at an 81k-file outlier all approaches converge (~1.3s, I/O-bound) — mitigated by streaming + caps, not engine choice. Use `default-features = false` where features permit.
  - `nucleo-matcher` (~3 transitive deps, negligible binary delta, MPL-2.0 — file-level copyleft; linking does not affect redquill's own licensing, but flag if a permissive-only license policy is ever adopted). Benchmarked <1ms ranking at 2,336 paths, 2–6× faster than the unmaintained `fuzzy-matcher`, fzf-consistent scoring (Helix/Television use it).
- **File-list source is `git ls-files -z`**, not the `ignore` walker (5.5ms vs 8ms at 2.3k files, ~6× faster at 81k; exact fidelity to git's tracked set). The `ignore` walker is used only inside the grep scan, where its parallel gitignore-aware walk is the point.
- **Cancellation**: the search sink checks an `AtomicBool` abort flag per match/line; setting it on query change stops workers promptly. Generation counter still guards against any straggler results.
- **Read-only file view**: extend the diff-source/capability model from spec 05 (a read-only target variant; `StagingMode::ReadOnly`-style gating) rather than inventing a parallel view type; synthesize an all-context whole-file body from worktree content (`read_worktree_file` seam).
- **Large-file guard**: skip files over a size threshold in search (grep-searcher supports this cheaply) and note skips in the summary line, so a stray generated artifact can't blow the instant-feel budget.
- Prior-art notes: Helix shells out to `rg` for global search; Television embeds nucleo and shells for grep. Embedding the grep crates is the less common choice — acceptable here because the decision was made explicitly with measured costs on the table.

## Security Considerations

- User-supplied regexes are compiled by `grep-regex` (finite-automata family): no catastrophic backtracking, so a hostile pattern cannot hang the search thread; compile errors surface inline and never panic.
- Search and the file finder operate strictly within the repository worktree; no path from user input is opened directly — only paths produced by `git ls-files` or the gitignore-aware walk.
- No network access, no new subprocess surface (the feature removes a would-be subprocess), no credentials touched.
- Persisted proof artifacts must not capture repository contents beyond what the journey requires (they live under the gitignored `proofs/` convention).

## Success Metrics

1. **Instant feel, measured**: on a ~5,000-file corpus, first search results visible <100ms from keystroke-settle (target headroom says ~30ms full-scan is achievable); enforced structurally by the perf tripwire and evidenced with timing notes in the Unit 2 acceptance journey.
2. **The primary journey works end to end**: a user reviewing a real diff can `g/` an identifier seen in the diff, open a hit in an untouched file, annotate it, and unwind with `Esc` to their exact review position — persisted as acceptance evidence (user journey + artifacts, per this repo's UX-outcome verification practice), not just unit tests.
3. **Output contract intact**: all pre-existing stdout serialization tests pass unmodified; the `(=)` extension ships with byte-exact tests and a consumer round-trip proof.
4. **No regression to review flow**: existing perf tripwires stay green; keymap/help drift tests stay green with the new modes included.

## Open Questions

1. Toggle keybindings inside the search view are proposed as `Alt-c` / `Alt-w` / `Alt-r` — confirm or adjust at spec review; non-blocking because the toggle *behaviors* are fixed by this spec and the chords live in the remappable shared tables.
2. Result cap default (10,000 hits) and large-file skip threshold — proposed values; tune during implementation without affecting scope.
3. Whether `gp`/`g/` should also be reachable from panel scope (sidebar focused) — proposed yes for `g/`/`gp` both; trivial either way, decide at implementation review.
