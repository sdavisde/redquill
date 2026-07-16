# 08 Questions Round 2 - Branch Review Mode

Two follow-ups from your round-1 answers. Please answer directly in this file.

## 1. In-app ref-range entry, explained — include it or not?

**What it means.** Today you can launch redquill on any ref range from the shell — `redquill main...feature-x` or `redquill v1.2...v1.3` already works (this landed with spec 05). What you *cannot* do is change the range once the app is open: there is no key that lets you type a range and re-target the diff in place. "In-app ref-range entry" is exactly that — press a key, a small input modal opens (same pattern as `/` search), you type `main...v2.1`, and the diff view re-targets to that range without restarting the app.

**How it relates to review mode: barely.** It creates no worktree, no review states, no banner, no persistence — it's a pure viewer convenience for "let me quickly look at what changed between these two refs." The only connection is thematic (both are about ranges) and one shared caveat: when viewing a range whose head doesn't match your checkout, LSP navigation explores *your* files, not the range's — so it gets a small "exploration follows your checkout" indicator, whereas worktree-backed review mode gets truthful LSP for free.

**Impact on the rest of this spec if included: none structurally.** It would be its own demoable unit, severable at task-planning time. The cost is one input modal + the indicator.

- [ ] (A) Include it as a demoable unit in this spec
- [x] (B) Exclude it — file it as its own small future spec

**Recommended answer(s):** [(A)]

**Why these are recommended:**

- It closes the spec-05 dogfood gap you've already hit, and it's the cheapest unit in the spec.
- Choose `(B)` only if you want this spec as lean as possible; nothing else here depends on it.

## 2. Making `q` the end-review key: proposed semantics

Your suggestion to reuse `q` is sound — one fewer key to learn, and the banner can say `REVIEWING <branch> — q to end review`. But `q` currently means "quit and emit annotations to stdout," so its review-mode behavior needs to be exact. Proposal:

- **`q` in review mode** opens the end-review modal with three exits:
  - **pause** — emit annotations to stdout (unchanged contract), quit; worktree and review state are kept for next time
  - **finish** — emit annotations, remove the worktree (no `--force`), delete this range's review state, quit
  - **cancel** — back to reviewing
- **`Q` in review mode** stays what it is everywhere: quit immediately, emit nothing. Review state survives (it's written as you go), worktree survives — it's effectively pause-without-ceremony.

One wrinkle this exposed: **annotations are in-memory only** (by design — stdout on quit is the output contract). In a multi-session review, each session emits its own annotations when you quit; pausing does not carry unemitted annotations into the next session. Carrying annotations across sessions is the "persisted review sessions" roadmap item and the output-mechanisms spec you're planning — I propose declaring it a Non-Goal here rather than growing this spec.

- [x] (A) Yes — `q` opens the pause/finish modal as described, `Q` stays instant-quit, annotation persistence is a Non-Goal
- [ ] (B) Keep `q` as plain pause-and-quit (no modal), and put end-review behind a separate key (e.g. `X`) as originally proposed
- [ ] (C) Other (describe)

**Recommended answer(s):** [(A)]

**Why these are recommended:**

- It matches your instinct: the quit gesture and the end-review gesture are the same thing, and the modal makes the pause/finish distinction explicit exactly once, at the moment it matters.
- The modal is a one-keypress speed bump on quit only while reviewing; `Q` remains the zero-friction escape hatch.
- `(B)` preserves a frictionless `q` but reintroduces a second key the banner must teach, which is what you were trying to avoid.
