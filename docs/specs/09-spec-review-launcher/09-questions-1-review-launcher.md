# 09 Questions Round 1 - Review Launcher

This round was asked and answered interactively in-session on 2026-07-17 and codified here for the record. Selected answers are checked. Prior context: the launcher's high-level shape was already ratified in the same session — global `R` opens a tabbed "Start review" modal (Branches | Commits, future Pull Requests), diff-scope refresh moves to `r`, a real `Global` keymap scope is introduced, and closing the modal restores the invocation origin (the `EndReview` origin pattern).

## 1. Commits Tab Contents

What should the Commits tab list?

- [ ] (A) Recent commits of HEAD — same data source the git panel's History tab already loads. Predictable, reuses existing lazy-loading machinery, works on any branch.
- [ ] (B) Only commits ahead of the auto-resolved base (`origin/HEAD` → `main` → `master`). Focused on unreviewed work, but empty when the current branch is the base itself.
- [ ] (C) Ahead-of-base on a feature branch, falling back to recent HEAD log otherwise. Smartest but least predictable; two data paths to test.
- [x] (E) Other: **Ahead-of-base by default, with a keybind inside the modal to expand to all commits on the branch. "Let's try it and we'll see if it feels good."**

**Recommended answer(s):** [(A)]

**Why these were recommended:**

- `(A)` reuses the History tab's existing loader and is never empty, covering the agent-committed-to-main case with zero new git-layer code.
- `(B)` matches "what's unreviewed" intent but fails exactly on the base branch; `(C)` fixes that at the cost of unpredictable behavior.
- The chosen `(E)` combines `(B)`'s focus with an explicit, user-visible escape hatch instead of `(C)`'s implicit switching — a deliberate dogfood experiment.

## 2. Default Tab

Which tab should the launcher open on?

- [x] (A) Remember the last-used tab within a session (Branches on first open). Matches the existing `last_panel_tab` pattern; repeat workflows (successive agent-commit reviews) become `R` + `Enter`.
- [ ] (B) Always Branches — fixed and fully predictable; Commits is always one `Tab` away.
- [ ] (C) Always Commits — optimizes commit review at the cost of making branch review second-class.

**Recommended answer(s):** [(A)] — accepted.

**Why this was recommended:**

- Session-scoped tab memory has in-repo precedent (`last_panel_tab`) and makes repeated flows cheap without persisting anything.
- `(B)` costs a keystroke on every repeat commit review; `(C)` inverts the primary use case.

## 3. Behavior During an Active Review Session

How should the launcher behave while a branch-review session is active?

- [x] (A) Commits tab works (a commit peek is read-only and Esc-restorable, so it is safe mid-session); Branches tab selection is blocked with a status hint to finish/pause first (`q` → the existing EndReview modal). No implicit session teardown.
- [ ] (B) Launcher fully blocked in-session — simplest rules, but loses the useful mid-review commit peek.
- [ ] (C) Selecting a branch implicitly pauses the current session and starts the new one — fewest keystrokes, but implicit mutations of persisted review state.

**Recommended answer(s):** [(A)] — accepted.

**Why this was recommended:**

- Preserves a genuinely useful capability (peeking a commit mid-review) while routing session lifecycle changes through the existing explicit EndReview flow.
- `(C)` would silently write review-state persistence as a side effect of a list selection.

## 4. Enter on a Commit

What does `Enter` on a commit in the Commits tab do?

- [x] (A) Opens the existing read-only single-commit view (commit vs its first parent); `Esc` returns exactly to the prior view. Zero new diff machinery.
- [ ] (B) Opens a `commit..HEAD` range diff — closes the known in-app range-entry gap from spec 05, but requires a new Esc-restorable range view and more design decisions.
- [ ] (C) Both: `Enter` = single commit, a second keybind = `commit..HEAD` range.

**Recommended answer(s):** [(A)] — accepted.

**Why this was recommended:**

- `open_commit_view` already exists, is panel-independent, and is Esc-restorable; this keeps the spec small.
- In-app range entry remains deliberately deferred (spec 05 follow-up) rather than smuggled in here.
