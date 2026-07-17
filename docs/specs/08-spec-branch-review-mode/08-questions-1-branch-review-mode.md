# 08 Questions Round 1 - Branch Review Mode

Please answer each question below (select one or more options, or add your own notes). Feel free to add additional context under any question.

## 1. How does a branch review start?

What is the entry surface for "review this branch" (the flow that creates the worktree, computes `merge-base...head`, and opens the review)?

- [ ] (A) CLI flag only: `redquill --review <branch>` does all the plumbing
- [ ] (B) In-app only: a "review branch" action using a new modal in the UI
- [x] (C) Both A and B
- [ ] (D) In-app ref-range modal only — no worktree plumbing in this spec

**Recommended answer(s):** [(C)]

**Why these are recommended:**

- `(B)` reuses the merged spec-03 branch picker and matches spec 05's precedent that new diff sources are reached in-app; it's the flow you'll use day-to-day.
- The CLI flag in `(A)` is nearly free once the plumbing exists, and it is the hook the future forge-integration wrapper needs (a script that fetches a PR branch and launches redquill on it). Spec 05's "no new CLI surface" non-goal was scoped to that spec, not a standing rule.
- `(D)` would ship a review mode whose LSP navigation lies about the code on disk — it defeats the point of the feature.

## 2. Should bare in-app ref-range entry (no worktree) be included?

This is the known spec-05 dogfood gap: a modal to type any ref range (e.g. `v1.2...v1.3`) and re-target the diff in place, without a worktree. Codebase exploration/LSP would be disabled or visibly marked as navigating your current checkout, since disk state doesn't match the range head.

- [ ] (A) Include it as a demoable unit in this spec (small: one input modal reusing the search-modal pattern; `DiffTarget::Range` already works end-to-end)
- [ ] (B) Exclude it — this spec does the worktree-backed review flow only; range entry gets its own later spec
- [x] (C) I'm not sure what this ref-range entry really means, or whether this would impact the other changes in this spec

**Recommended answer(s):** [(A)]

**Why these are recommended:**

- It closes a gap you've already hit while dogfooding, and the review flow builds on the same range plumbing anyway — the marginal cost is one modal plus an "exploration disabled" indicator.
- `(B)` is the right call only if you want this spec strictly minimal; the unit is cleanly severable at task-planning time if it grows.

## 3. Where do review worktrees live on disk?

- [x] (A) Inside the git dir: `.git/redquill/worktrees/<sanitized-branch>` — hidden from file browsers, no `.gitignore` pollution, co-located with the review-state file in `.git/redquill/`
- [ ] (B) Sibling directory: `../<repo>-review/<branch>`
- [ ] (C) System cache dir (XDG / `~/Library/Caches`)
- [ ] (D) Configurable via the spec-07 config layer, defaulting to (A)

**Recommended answer(s):** [(A)]

**Why these are recommended:**

- Git supports worktrees at any path; putting them under `.git/` keeps the user's project neighborhood clean and makes "redquill's stuff" one directory to reason about (state file + worktrees together).
- `(B)` clutters the parent directory and risks collision with real checkouts; `(C)` puts repo-coupled state far from the repo.
- `(D)` is attractive but spec 07 is mid-flight on a concurrent branch — depending on it would couple the two specs. A config knob can be added later without migration pain.

## 4. Write-ceiling amendment scope

Branch review requires git operations outside the current product write ceiling. Which worktree operations should the amendment sanction?

- [ ] (A) `git worktree add` only — finish-review leaves the worktree for the user to remove manually
- [x] (B) `git worktree add` + `git worktree remove` (never `--force`; a dirty worktree makes removal fail and surface a message) + `git worktree prune`
- [ ] (C) (B) plus forced removal when the worktree is dirty

**Current best-practice context:** `git worktree remove` without `--force` refuses to delete a worktree containing uncommitted changes — git itself provides the safety property the guardrails care about.

**Recommended answer(s):** [(B)]

**Why these are recommended:**

- Finish-review's whole point is cleanup; `(A)` leaves a manual chore and stale worktrees accumulate.
- No-force `remove` is non-destructive by construction (git refuses if anything would be lost), which fits the existing ceiling's spirit; `prune` only clears administrative records for already-deleted directories.
- `(C)` can silently destroy edits made in the review worktree and belongs on the forbidden list.

## 5. Accept / defer keybindings

On `Range` targets, staging is already `ReadOnly` (spec 05), so the staging keys are free in review mode.

- [x] (A) Reuse staging muscle memory: `Space` = toggle accept on cursor file (auto-collapses on accept, mirroring stage-auto-collapse), `S` = accept file from anywhere in it; `d` = toggle defer (collapse + "come back later" marker)
- [ ] (B) Dedicated new keys, leaving `Space`/`S` inert in review mode (e.g. `m` = accept, `e` = defer)

**Recommended answer(s):** [(A)]

**Why these are recommended:**

- The whole design insight is that accepting a file *is* the review-mode binding of the staging gesture — one gesture, mode-dependent meaning. Muscle memory transfers between local review and branch review.
- The keymap table (`src/ui/keymap.rs`) and `?` overlay can show the mode-appropriate description, so there's no discoverability cost.
- `d` is unbound in diff scope today (only `Ctrl-d` and `gd` are taken); if you'd rather reserve `d`, suggest an alternative under Other.

## 6. Ending a review: semantics and key

The banner will read like `REVIEWING <branch> — <key> to end review`. What should ending do?

- [x] (A) `<key>` opens a small confirm modal with two exits: **pause** (leave review mode, keep worktree and state — resume later) and **finish** (emit annotations, remove worktree, delete this range's review state); plain `q` while in review mode behaves like pause-and-quit
- [ ] (B) `<key>` always pauses; finish-review is a separate action in the git panel
- [ ] (C) `<key>` always finishes (destructive-ish: removes worktree immediately after confirm)

And the key itself: `Esc` is heavily used for dismissing modals/visual mode, so it would need to be context-sensitive. `X` is unbound in diff scope.

- [ ] (X1) `X`
- [ ] (X2) `Esc` (only when no modal/visual state is active)
- [x] (X3) Other (describe) should we just use "q" since you plan for that to work the same anyway?

**Recommended answer(s):** [(A), (X1)]

**Why these are recommended:**

- Pause vs finish is the review-lifetime distinction the whole persistence design hangs on; putting both behind one well-labeled modal makes the state file's lifecycle visible instead of implicit.
- `(C)` makes the common case (multi-round review) awkward; `(B)` hides finish somewhere the banner can't point to.
- `Esc` as a mode exit fights its existing dismiss semantics and will cause accidental exits mid-review; `X` is free, easy to show in the banner, and hard to hit by accident.

## 7. What happens to an accepted file whose content changes upstream?

When resuming a review, a previously accepted file whose blob SHA no longer matches:

- [ ] (A) Silently demotes to unreviewed (GitHub "Viewed" behavior)
- [x] (B) Moves to a distinct visible **changed-since-accepted** state (its own sidebar marker; un-collapses; one keypress re-accepts) so you know to look at just the delta
- [ ] (C) Stays accepted

**Recommended answer(s):** [(B)]

**Why these are recommended:**

- `(A)` loses information you already paid for — you can't distinguish "never looked" from "looked, then it changed," which is exactly the signal round two needs.
- `(C)` makes acceptance a lie after every push.
- `(B)` costs one more display state on an enum the compiler will exhaustively check, consistent with the repo's data-driven-invariant conventions.

## 8. Does the review tri-state touch local (working-tree/staged) mode in this spec?

- [x] (A) No — review states exist only on worktree-review and range targets; local mode keeps today's staging behavior unchanged (accepted = staged is already true there by construction)
- [ ] (B) Also add the "deferred" marker to local mode now
- [ ] (C) Full unification: replace the staging display model with the review tri-state everywhere

**Recommended answer(s):** [(A)]

**Why these are recommended:**

- Local mode already has a working tri-state (`StagedState`: Unstaged/Partial/Full) with its own semantics driven by git porcelain — the index *is* the persistence there, so review mode adds nothing but risk.
- `(B)` is a nice-to-have that's cleanly additive later; `(C)` is a refactor of shipped, dogfooded behavior mixed into a feature spec, which the repo conventions (refactors and behavior changes never share a commit) frown on at spec scale too.
