# 03 Questions Round 1 - Multibuffer Review

Please answer each question below (select one or more options, or add your own notes). Feel free to add additional context under any question.

## 1. Fate of the Side-by-Side View

Today `t` toggles unified ↔ side-by-side for the selected file. A side-by-side rendering of a *multi-file* buffer is a significant extra lift (Zed's project diff is unified-style blocks, not two panes). What should happen to side-by-side when the multibuffer becomes the main view?

- [ ] (A) Unified-only multibuffer; side-by-side is removed entirely (delete `sbs_view`/`SbsRow`).
- [ ] (B) Keep side-by-side via a per-file focus mode: a keybind on a file section opens that single file in the existing full-width view, where `t` works exactly as today; leaving focus returns to the multibuffer.
- [ ] (C) Implement side-by-side rendering inside the multibuffer itself (both views work multi-file).
- [x] (E) Other (describe) - instead of allowing the diff view to be toggled with a keybind, lets have this be a config option for now. In a future spec, we can create a "settings" modal to allow easily changing config options like `herdr` has.

> **Superseded (user, in conversation):** side-by-side in a multi-file buffer would be significant extra work, so it is dropped from scope for now — the multibuffer is unified-only ("I only use the unified view anyway"). No view-mode config option is needed in this spec; side-by-side may return with the future settings/config spec.

**Recommended answer(s):** [(B)]

**Why these are recommended:**

- `(B)` preserves a shipped, tested feature at near-zero cost (the single-file view and `SbsRow` derivation already exist and keep working), and "focus one file full-width" is a useful affordance in its own right — Zed offers the same gesture (open a project-diff entry as its own tab).
- `(C)` roughly doubles the rendering work of this spec for a view you didn't mention needing multi-file, and can always be a later enhancement behind the same `t` binding.
- `(A)` deletes something that works and that a reviewer comparing large rewritten hunks genuinely uses; only pick it if you never use side-by-side.

## 2. Which Diff Targets Use the Multibuffer

redquill reviews the working tree (default), the index (`--staged`), and ref ranges (`main..HEAD`). Should the multibuffer become the main view for all targets, or only for the working-tree "uncommitted changes" flow?

- [x] (A) All targets: one view system; `--staged` and ranges render the same collapsible multi-file buffer (staging keybinds simply disabled where they don't apply, e.g. ranges).
- [ ] (B) Working tree only: ranges and `--staged` keep the current one-file-at-a-time view, giving redquill two parallel view systems.
- [ ] (E) Other (describe)

**Recommended answer(s):** [(A)]

**Why these are recommended:**

- `(A)` keeps one code path — after the spec-01 refactor there will be exactly one view-state component, and maintaining a legacy second view indefinitely contradicts the point of the pivot.
- A multi-file scrollable buffer is arguably *most* valuable for range review (reading a whole branch top-to-bottom like a PR).
- `(B)` only makes sense as a temporary migration stage; if you want that, note it under Other and the spec will sequence working-tree first with ranges following in the same spec.

## 3. Auto-Expand on New Changes

You chose "stage auto-collapses a file; a separate keybind toggles collapse." A staged-and-collapsed file can change again (the agent edits it, or you edit it after review) — it becomes partially staged with fresh unstaged hunks. On refresh, what should happen to its collapsed state?

- [ ] (A) Auto-expand any collapsed file that has new unstaged changes; fully staged files stay collapsed.
- [ ] (B) Collapse state is sticky: files stay collapsed until you manually expand them, regardless of new changes.
- [x] (C) Auto-expand as in (A), plus a visual marker (e.g. `±` partially-staged indicator) on the section header either way.
- [ ] (E) Other (describe)

**Recommended answer(s):** [(C)]

**Why these are recommended:**

- Your stated goal is "collapsed = done reviewing." A file with fresh unstaged changes is no longer done — leaving it collapsed (`(B)`) silently hides exactly the changes the tool exists to surface, which is the one failure mode a review tool must not have.
- `(C)` adds the partial-staged marker so the panel and section headers tell the truth even at a glance, matching the staged `●` marker convention that already exists in the sidebar.
- `(B)` is predictable but unsafe-by-default for the agentic workflow where files change under you mid-review.
