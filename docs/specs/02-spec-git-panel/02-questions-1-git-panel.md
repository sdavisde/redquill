# 02 Questions Round 1 - Git Panel

Please answer each question below (select one or more options, or add your own notes). Feel free to add additional context under any question.

## 1. Stash Scope

You said "view stashes." Should the panel only display the stash list, or also let you act on stashes? Acting on stashes (apply/pop/drop) mutates the working tree, which is beyond redquill's current write ceiling (staging only), so this materially changes the tool's safety posture.

- [ ] (A) View-only: show `git stash list` entries (name, branch, message) in a STASHES section; no actions.
- [ ] (B) View + apply/pop: keybinds to apply or pop the selected stash; drop excluded.
- [ ] (C) Full stash management: create, apply, pop, and drop from the panel.
- [x] (D) View-only now, with the panel structured so apply/pop can be added in a later spec.
- [ ] (E) Other (describe)

**Recommended answer(s):** [(D)]

**Why these are recommended:**

- `(D)` matches your literal request ("view stashes") and keeps this spec's write surface limited to the index plus explicit remote ops, while making the follow-on obvious and cheap.
- `(B)`/`(C)` change the working tree, which today nothing in redquill does — that deserves its own deliberate spec (conflict handling on apply, discard semantics for drop) rather than riding along here.
- `(A)` is nearly identical to `(D)` in build cost but loses the forward-looking structure.

## 2. Panel Focus Model

Zed's git panel is a focusable pane with its own cursor (navigate entries, Enter jumps to the file). redquill's current sidebar is passive — selection just mirrors the diff's selected file. Which model should the new panel use?

- [x] (A) Focusable panel: a keybind moves focus between panel and diff; the panel has its own cursor for CHANGES/UNTRACKED/STASHES entries; Enter on a file entry selects it in the diff view.
- [ ] (B) Passive display: the panel only renders state (branch, ahead/behind, sections, stashes); all actions remain global keybinds from the diff view.
- [ ] (C) Focusable, but only the file sections are navigable; stashes/branch are display-only rows.
- [ ] (E) Other (describe)

**Recommended answer(s):** [(A)]

**Why these are recommended:**

- `(A)` is the Zed-parity behavior you asked for and becomes more valuable once the multibuffer lands (panel entry → scroll multibuffer to that file's section).
- `(B)` is cheaper but makes stash viewing awkward (a list you can't scroll if it's long) and caps the panel's usefulness at roughly what the sidebar already does.
- `(C)` is a reasonable middle ground, but the incremental cost of making all sections navigable over just one is small once the panel has a cursor at all; the split isn't worth the inconsistency.

## 3. Remote Operation UX (fetch / pull / push)

How should fetch, pull, and push behave when invoked? They run plain `git fetch` / `git pull` / `git push` on your PATH (respecting your git config, per the repo's design principles) on a background thread with progress/result shown in the status footer. The open question is confirmation and failure UX.

- [x] (A) No confirmations: keypress runs the command immediately; success/failure (with git's stderr summary) shown in a command output pane like lazygit's "Command Log" which can be visibility toggled with a keybind. Pull conflicts simply surface as conflicted files in the changes list (the status parser already understands unmerged entries).
- [ ] (B) Confirm before push only (push is the only action visible to others); fetch/pull run immediately.
- [ ] (C) Confirm before push and pull (both mutate more than the index); fetch runs immediately.
- [ ] (E) Other (describe)

**Recommended answer(s):** [(A)]

**Why these are recommended:**

- `(A)` matches the tool's keyboard-speed ethos and lazygit's convention (single keypress, immediate feedback); a deliberate keybind is itself the confirmation in a modal-free TUI.
- Plain `git push` (never force) to the configured upstream is hard to damage — the worst case is a rejected push, which is just a footer message.
- If you routinely work on shared branches where an accidental push is costly, `(B)` is the sensible override.

## 4. Keybindings for the New Actions

Proposed additions to the README keymap (none conflict with the existing map; `f`/`p` are currently unused). Per CLAUDE.md, the winning choices get proposed in README.md itself as part of this spec's work.

- [ ] (A) `g`-prefix family: `gf` fetch, `gp` pull, `gP` push (mnemonic "git-fetch/pull/push"; consistent with the existing `gd`/`gr` two-key pattern).
- [ ] (B) Bare keys: `f` fetch, `p` pull, `P` push (lazygit-style; fastest, but burns three top-level keys).
- [x] (C) Panel-scoped: bare `f`/`p`/`P` only while the git panel is focused (keeps the global namespace clean; requires question 2 = focusable).
- [ ] (E) Other (describe)

**Recommended answer(s):** [(A)]

**Why these are recommended:**

- `(A)` reuses the established two-key prefix machinery from `gd`/`gr` (no new keymap capability needed), reads mnemonically, and preserves scarce top-level keys for the multibuffer spec (which needs collapse/toggle bindings).
- `(C)` is elegant but makes remote ops unreachable while reviewing the diff, adding a focus-switch keystroke to every fetch.
- `(B)` is fastest in hand but spends `f`/`p`/`P` globally, which the multibuffer and future staging flows may want more.
