# 03-spec-multibuffer-review.md

## Introduction/Overview

redquill currently shows one file's diff at a time: the sidebar picks a file, the diff pane renders it. This spec replaces that with a Zed-style **multibuffer**: every changed file rendered as a collapsible section in one continuously scrollable buffer, for all diff targets (working tree, `--staged`, ref ranges).

The review flow this enables is the heart of the pivot: read a file's changes, stage it with one keypress — it collapses to a one-line header and gets out of the way — and keep scrolling. Unstaging is one keypress on the header. Files that change again after being staged re-expand automatically, so a collapsed header can never silently hide unreviewed changes. The multibuffer is unified-view only.

## Goals

- Render all changed files as one scrollable buffer of collapsible per-file sections, replacing the one-file-at-a-time view for every diff target.
- Make "done reviewing this file" a single keypress: stage-and-collapse, with unstage equally cheap from the collapsed header.
- Guarantee collapsed sections never hide unreviewed work: new unstaged changes auto-expand their file and mark it partially staged.
- Preserve the full existing review surface — annotations, search, hunk/line staging, and LSP peek (`gd`/`gr`/`K`) — working seamlessly across file boundaries.
- Hold the performance bar: scrolling and hunk-jumping stay instant on a 5k-line multi-file diff.

## User Stories

- **As a reviewer of agentic changes**, I want all changed files in one scrollable buffer so that I can read a whole working-tree diff top to bottom without a file-switching ritual.
- **As a reviewer**, I want staging a file to collapse it so that my buffer shrinks to only what still needs review, and I can see my remaining work at a glance.
- **As a reviewer who changes their mind**, I want to unstage a collapsed file with one keypress on its header so that pulling something back into review is painless.
- **As a reviewer in a live agent session**, I want a staged-then-edited file to pop back open with a partial-staged marker so that nothing the agent changed after my review can slip past me.
- **As a reviewer**, I want `gd`/`gr`/`K` to work from any line in the buffer so that "what does this touch?" never requires leaving the tool.

## Demoable Units of Work

### Unit 1: Multi-file row model with collapsible sections

**Purpose:** The structural core — one row buffer spanning all files, with per-file collapse.

**Functional Requirements:**
- The system shall provide a new row derivation (in the style of the existing `SbsRow` pattern in `src/ui/rows.rs`) that concatenates all files' rows into one buffer, each file preceded by a section-header row showing: collapse indicator (`▾`/`▸`), change-kind letter, path (rename arrow where applicable), staged `●` or partially-staged `±` marker.
- The system shall maintain a per-file collapse map; a collapsed file contributes only its header row to the buffer, and untracked files appear as sections via the existing synthetic-added diff support.
- The user shall scroll and move the cursor continuously across file boundaries with all existing motion keys; `]`/`[` (hunk jumps) shall cross into neighboring expanded files, and `Tab`/`Shift-Tab` shall jump between file section headers (repurposing their current next/previous-file meaning).
- The user shall toggle the collapse state of the file under the cursor with a dedicated keybind (proposed: `za`, vim fold grammar; final key ratified via the README keymap update).
- The system shall keep section-header rows and diff rows correctly addressable so that cursor clamping, visual selection, and annotation anchoring behave exactly as they do today within any expanded section.

**Proof Artifacts:**
- Test: unit tests on the multi-file row builder (concatenation order, collapse filtering, header content, addressability flags, synthetic untracked sections) demonstrate the model is correct.
- Manual smoke transcript: scrolling a multi-file working-tree diff end to end, collapsing/expanding files mid-scroll, demonstrates the buffer behaves as one continuous document.

### Unit 2: Staging-driven review flow

**Purpose:** The keep-it verdict becomes "stage and it gets out of the way."

**Functional Requirements:**
- The user shall stage the entire file under the cursor with a dedicated keybind (proposed: `S`; ratified via the README update); on success the file's section shall auto-collapse.
- The user shall, with the cursor on a collapsed (or expanded) staged file's header, press the same keybind to unstage the file; unstaging shall auto-expand the section.
- The system shall keep existing hunk/line staging (`space`, visual-mode line staging) working within expanded sections, updating the header marker to `±` when a file becomes partially staged.
- The system shall, on refresh, auto-expand any collapsed file that has new unstaged changes, while fully-staged collapsed files stay collapsed; collapse state for unchanged files survives refresh.
- The system shall reuse the existing `StageOps` gestures for all staging operations — no new git-layer code.

**Proof Artifacts:**
- Test: app-level tests covering stage→collapse, unstage-from-header→expand, partial-stage marker transitions, and the refresh auto-expand rule demonstrate the flow's state machine.
- Manual smoke transcript: review three files, stage two (watch them collapse), edit one staged file externally, refresh, and watch it re-expand with `±`, demonstrates the "nothing hides" guarantee end to end.

### Unit 3: Full-surface integration

**Purpose:** Everything that worked in the single-file view works in the multibuffer, for every target.

**Functional Requirements:**
- The system shall render the multibuffer for all diff targets — working tree (default), `--staged`, and ref ranges — with staging keybinds disabled (and absent from contextual help) where they don't apply (ranges).
- The system shall keep annotations fully functional across the buffer: composing on lines/ranges/hunks/files in any section, annotation gutter rows rendering in place, the annotation list jumping to any file's rows, and markdown-on-quit output unchanged (the stdout format is a public API).
- The system shall make search (`/`, `n`/`N`) span the entire buffer across expanded sections.
- The system shall keep LSP peek (`gd`/`gr`/`K`) working from any addressable line (rows retain their new-file line mapping), and peek jump-to-location shall scroll the multibuffer to the target file's section, expanding it if collapsed.
- The system shall route sidebar/git-panel file selection to "scroll the multibuffer to that file's section (expanding if collapsed)" through the narrow select-by-path interface established in spec 02.
- The system shall remove the side-by-side view from the reachable UI (retire the `t` binding; delete code that would otherwise be dead), and shall update README.md's keybinding map and the `?` help overlay to reflect the final bindings (`S`, `za`, repurposed `Tab`, retired `t`).

**Proof Artifacts:**
- Test: `cargo test` including annotation-anchoring and search tests generalized to multi-file rows demonstrates no review-surface regression; annotation output snapshot tests unchanged demonstrates the stdout API is intact.
- Manual smoke transcript: `redquill main..HEAD` rendering a branch as one scrollable buffer with annotations and LSP peek working, demonstrates all-targets parity.
- Manual: a ~5k-line multi-file diff scrolled with held-down `j` and `Ctrl-d` demonstrates the instant-feel performance target holds.

## Non-Goals (Out of Scope)

1. **Side-by-side view**: dropped for now (unified-only, per user decision); may return via the future settings/config spec.
2. **Tabs**: no tab abstraction; the panel + multibuffer is the whole layout.
3. **Config layer / settings modal**: deferred to a future spec (`docs/config-layer.md` remains the sketch).
4. **Git-layer changes**: no new git commands; staging plumbing is reused as-is.
5. **Collapse-state persistence across runs**: collapse state is in-memory only; persisted review sessions remain a roadmap item.
6. **Annotation output format changes**: the markdown-on-stdout contract is untouched.

## Design Considerations

Target layout (working with spec 02's git panel):

```
┌─ git: main ↑2 ──┬─ uncommitted changes ───────────────────┐
│ CHANGES         │ ▾ M src/auth/session.rs              ±  │
│ ± M session.rs  │   42 │  fn validate(token: &Token) ...  │
│   M mod.rs      │   43 │- let key = env::var("SECRET")?;  │
│ ● A keys.rs     │   44 │+ let key = keystore.current()?;  │
│ UNTRACKED       │ ▸ A src/keys.rs                      ●  │
│   notes.md      │ ▾ M src/auth/mod.rs                     │
│ STASHES (1)     │   12 │+ pub mod keystore;               │
└─────────────────┴─────────────────────────────────────────┘
```

Section headers are visually distinct (styled like the current file header bar), collapsed headers render exactly one line. Collapsed headers may show a summary (e.g. `+12 −4`) at the implementer's discretion.

## Repository Standards

- All four gates at every commit: `cargo build`, `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check`.
- TDD for the pure row-derivation code: failing tests first, tests commit with the code.
- No `unwrap()`/`expect()` outside tests.
- Every new action in the keymap table and `?` overlay; keybinding changes land in README.md's map as part of this spec.
- Conventional commits (`feat:`, `refactor:` for the sbs removal, `docs:` for README).

## Technical Considerations

- **Hard dependency on spec 01**: this spec consumes the extracted `DiffViewState`, generalizing "rows for one file" to "rows for many files with collapse state." Do not start before spec 01 merges.
- **Parallel with spec 02**: file contact is disjoint except additive touches to `keymap.rs`, `help.rs`, and `ui/mod.rs`; the panel↔multibuffer boundary is spec 02's select-by-path interface.
- **Performance**: today rows are built for one file at a time; the multibuffer builds them for all files. Syntax highlighting must be computed lazily per file (on first expansion/visibility) and cached — the existing peek-overlay per-path highlight cache is precedent. Stage/collapse operations should rebuild incrementally or fast enough to feel instant; the 5k-line target is a hard regression bar.
- **Cursor/scroll invariants**: `code_intel_position`, annotation anchoring, and visual-range staging all key off addressable rows with per-side line numbers — the multi-file builder must preserve those invariants so `lsp/`, `annotate/`, and `git/` need no changes.
- **`Tab` repurposing** (next/previous file → jump between section headers) preserves its muscle-memory meaning ("go to next file") in the new model.

## Security Considerations

No specific security considerations identified: no new inputs, credentials, network calls, or output formats.

## Success Metrics

1. **One-keystroke verdicts**: staging-and-dismissing a reviewed file is exactly one keypress; recovering it is one keypress on its header.
2. **Nothing hides**: 100% of refreshes that introduce unstaged changes to a collapsed file result in that file expanded with `±` (covered by tests).
3. **No surface regression**: annotations, search, staging, and LSP peek all demonstrably work across file boundaries; annotation stdout output is byte-identical for equivalent reviews.
4. **Performance**: instant-feel scrolling/hunk-jumping on a 5k-line multi-file diff (manual bar, plus no perceptible regression vs the single-file view).
5. **Quality gates**: all four cargo gates green; test count strictly increases.

## Open Questions

1. Final key glyphs (`S` stage-file, `za` collapse toggle) are proposals ratified in the README keymap update; `zM`/`zR` (collapse-all/expand-all) are a cheap optional addition left to implementation judgment.
2. Whether collapsed headers show a `+N −M` summary is an implementer choice (nice-to-have, no requirement).
3. Initial collapse state on launch: all expanded, except files already fully staged at startup, which start collapsed — assumed; flag if you'd rather everything start expanded.
