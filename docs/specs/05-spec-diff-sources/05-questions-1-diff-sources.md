# 05 Questions Round 1 - Diff Sources

Please answer each question below (select one or more options, or add your own notes). Feel free to add additional context under any question.

Context for all questions: the 2026-07-14 research session established that the viewer is already largely source-agnostic (`DiffTarget` enum exists, ref ranges render end-to-end, the multibuffer row builder and syntax-highlighting content seam are source-blind). This spec covers the remaining gaps: a single-commit target, named capability methods replacing five scattered `matches!` checks, and gating LSP code-intel. The questions below are the decisions that materially change the spec.

## 1. CLI Surface for Viewing a Single Commit

Today `redquill <rev>` passes the rev verbatim to `git diff`, meaning "working tree vs `<rev>`" — consistent with git, but not "what that commit changed." How should the user ask for a commit's own diff (`git diff rev^ rev` semantics)?

- [ ] (A) Add a `--commit <rev>` (short `-c`) flag; bare `redquill <rev>` keeps its current git-diff semantics
- [ ] (B) Reinterpret a bare revision as "what that commit changed"; working-tree-vs-rev moves behind a flag
- [ ] (C) No new surface — document that `redquill 'rev^!'` and `redquill rev^..rev` already work via the range path
- [ ] (D) Add a `show <rev>` subcommand (git-style `diff` vs `show` split)
- [x] (E) Other (describe) i don't really understand why we need so mcuh cli utility for this, i barely see users using the cli directly when launching it. i imagine most people just using 'redquill' directly.

**Current best-practice context:** git itself resolves this exact ambiguity by splitting the verbs: `git diff <rev>` is worktree-vs-rev, `git show <rev>` is the commit's own diff. Zed reaches its commit view only from UI affordances (blame, log), so it has no CLI precedent to copy. magit treats "a lone commit" as worktree-relative in its diff transient and has a separate commit-show command — same split as git.

**Recommended answer(s):** [(A)]

**Why these are recommended:**

- `(A)` preserves git muscle memory for `redquill main` ("what have I changed relative to main") — the behavior most likely already in your fingers — while making commit review discoverable and cheap to type.
- `(B)` would silently change the meaning of existing invocations; `redquill main` showing "the commit at main's tip" is rarely what anyone wants for a branch name.
- `(C)` costs nothing but is undiscoverable and leaves merge commits subtly wrong (`rev^..rev` picks first parent implicitly with no place to document it).
- `(D)` is the most git-faithful but introduces a subcommand structure the CLI doesn't have yet; heavier than the feature needs. Reasonable to revisit if more subcommands ever appear.

## 2. Two-Ref Positional Form

`git diff A B` (space-separated) is equivalent to `git diff A..B`. redquill's clap config accepts only one positional token, so only `A..B`/`A...B` work today. Should the spec add a second positional?

- [x] (A) No — exclude as a non-goal; `A..B` already covers it exactly
- [ ] (B) Yes — accept `redquill A B` for git muscle-memory parity
- [ ] (E) Other (describe)

**Recommended answer(s):** [(A)]

**Why these are recommended:**

- The two forms are exactly equivalent in git, so `(B)` adds parsing surface and help-text complexity for zero new capability.
- Listing it as an explicit non-goal prevents it from being "helpfully" added during implementation and keeps the CLI grammar simple (one positional = one diff spec).
- If muscle memory bites in practice, it's a trivial additive follow-up that breaks nothing.

## 3. In-App Commit Selection

Is any TUI surface for *choosing* a commit (a log/history list in the git panel, "open this commit" from a keybind) in scope for this spec, or is commit review reached from the CLI only?

- [ ] (A) CLI-only for this spec; an in-app commit log/picker is a separate future spec
- [x] (B) Include a minimal commit-log section in the git panel that opens a commit view
- [ ] (C) Include a full log view with its own navigation - remember, we're trying to mirror how modern zed works.
- [ ] (E) Other (describe)

**Current best-practice context:** Zed's commit view is reached from blame/log affordances, but architecturally its commit *loader* landed separately from those entry points. gitui/lazygit both treat the log as its own view that merely *links* to commit inspection.

**Recommended answer(s):** [(A)]

**Why these are recommended:**

- `(A)` keeps this spec a pure source-abstraction slice: once `--commit` works end-to-end, a future log-panel spec gets commit review "for free" by constructing the same target — exactly how Zed layered it.
- `(B)`/`(C)` drag in panel layout, log pagination, and new keymap territory — each a spec-sized concern on its own, and the git panel (spec 02) is still awaiting validation. Bundling risks both.

## 4. Code-Intel (LSP) on Non-Worktree Sources

`code_intel.rs` unconditionally reads on-disk worktree files — already silently wrong for `--staged` and range views (definitions/hover resolve against text that may differ from what's displayed), and meaningless for a commit view. What should this spec do?

- [x] (A) Capability-gate it: code-intel available only when the diff's new side is the live working tree; cleanly absent (help overlay included) elsewhere
- [ ] (B) Leave current behavior untouched; note the drift as a known issue
- [ ] (C) Route historical text through the LSP (feed `git show` content to servers) so gd/gr/K work on any source
- [ ] (E) Other (describe)

**Current best-practice context:** Zed gates by construction — commit-view buffers are synthetic read-only blobs with no worktree file, so LSP features simply don't attach. The lesson is to make the capability structurally absent rather than best-effort-wrong.

**Recommended answer(s):** [(A)]

**Why these are recommended:**

- `(A)` fixes an existing correctness bug and the new-source case with one named predicate (`supports_code_intel()`), and fits the repo's "silent degradation only when documented" rule — the module doc gets the contract.
- `(B)` ships a viewer that confidently jumps to the wrong line on historical diffs — worse than absence.
- `(C)` is real work (temp buffers or `didOpen` with historical text, position mapping) with modest payoff for a review tool; it can be layered later behind the same predicate without rework.

## 5. Annotation Output for Historical Sources

The stdout markdown format (`## path:LINE (+)`) is a frozen public API and carries no revision context. A review of commit `abc123` currently emits headers indistinguishable from a working-tree review — ambiguous for the roadmap's primary consumer (agent plugins). What should this spec do?

- [ ] (A) No format change; document in the module doc + README that line numbers refer to the displayed diff's sides, and consumers must know what they invoked
- [x] (B) Additive-only: emit one metadata line at the top of the output (e.g. `Reviewing: abc123` or `Reviewing: main..feature`) **only when** the source is not the working-tree default — existing invocations stay byte-identical
- [ ] (C) Extend every annotation header with revision context (e.g. `## path:44 (+) @abc123`)

**Current best-practice context:** GitHub PR reviews anchor comments to an immutable base..head pair precisely because line numbers alone are meaningless without knowing which revisions they index into — the same consumer problem your agent-plugin roadmap will hit.

**Recommended answer(s):** [(B)]

**Why these are recommended:**

- `(B)` makes historical reviews self-describing for downstream agents — the whole point of reviewing an old commit is handing the findings to something that wasn't watching — while keeping every existing invocation's output byte-identical, so "frozen API" is honored where it's already relied upon.
- `(A)` is the conservative floor, but it bakes in an ambiguity you'd have to break the format to fix later, after more consumers exist. Cheaper to define the extension now, while non-worktree output has zero consumers.
- `(C)` changes the shape of every header line for all sources — a true breaking change to the frozen format, disproportionate to the need.
- If you pick `(B)`, the exact metadata-line syntax becomes part of the public API and will be specified and byte-exact tested like the rest of the format.
