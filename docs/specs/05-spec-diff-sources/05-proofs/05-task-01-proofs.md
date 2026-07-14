# Task 01 Proofs - `DiffTarget` capability model + code-intel gating

## Task Summary

Parent task 1.0 ("Capability model on `DiffTarget`", spec 05 Unit 1) replaced
five scattered `matches!(target, DiffTarget::...)` capability checks with
three named, exhaustively-matched methods on `DiffTarget` —
`is_live()`, `staging_mode()` (`Stage` / `Unstage` / `ReadOnly`), and
`supports_code_intel()` — then routed every existing call site through them,
and finally used `supports_code_intel()` to close a real bug: LSP code-intel
(`gd`/`gr`/`K`) previously had no target gate at all, so it would resolve
requests against on-disk content that doesn't match a `--staged` or range
diff's displayed text.

Landed as three commits on `worktree-diff-sources`:

1. `611ee3b` — `feat(git): add DiffTarget capability triple (is_live/staging_mode/supports_code_intel)`
2. `40312c9` — `refactor(ui): route DiffTarget capability decisions through the new methods`
3. (this proof file's commit) — `fix(ui): gate LSP code-intel on DiffTarget::supports_code_intel`

## What This Task Proves

- The capability model is exhaustive and encodes today's behavior for every
  existing `DiffTarget` variant (`WorkingTree`, `Staged`, `Range`).
- The refactor commit (`40312c9`) is move-only: identical test counts,
  zero assertion edits, only the five call sites changed.
- No capability-deciding `matches!(.*DiffTarget` construct remains in
  `src/ui/` — every capability question is answered by the named methods.
- Code-intel requests and peek-preview reads are now gated on
  `supports_code_intel()`, and the same predicate hides `gd`/`gr`/`K` from
  the `?` help overlay and footer strip, mirroring the existing staging-key
  visibility mechanism.
- The degradation is documented as a deliberate contract in the
  `code_intel.rs` module doc, per the repository's error-handling rules.

## Evidence Summary

| Proof | Result |
| --- | --- |
| Capability-triple unit tests | 3/3 pass (`git::diff::tests::{working_tree,staged,range}_capability_triple`) |
| `grep` for capability `matches!` in `src/ui/` | 1 hit, a test-fixture data selector (not a capability decision) — zero real hits |
| Full suite before/after the refactor commit | 802 passed / 2 ignored → 802 passed / 2 ignored (lib unit tests), identical; 38 passed across integration binaries, unchanged |
| Full suite after the code-intel gating commit | 810 passed / 2 ignored (lib unit tests; +8 new tests, 0 removed/edited existing assertions) |
| `code_intel.rs` module doc | Degradation contract section added, quoted below |
| Four gates (build/test/clippy/fmt) | All pass on every commit |

## Artifact: Capability-triple unit test run output

**What it proves:** Every `DiffTarget` variant's full capability triple
(`is_live`, `staging_mode`, `supports_code_intel`) is asserted in one place
and passes, demonstrating the model is exhaustive and matches the behavior
the five old call sites implemented individually.

**Why it matters:** This is the single source of truth the rest of the task
routes through; if this table were wrong, every downstream call site would
inherit the same wrong answer, which is exactly the bug class this task
closes (the old scattered `matches!` sites could — and for code-intel, did —
disagree).

**Command:**

```
cargo test --lib capability_triple
```

**Result summary:**

```
running 3 tests
test git::diff::tests::working_tree_capability_triple ... ok
test git::diff::tests::range_capability_triple ... ok
test git::diff::tests::staged_capability_triple ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 809 filtered out; finished in 0.00s
```

The three tests (`src/git/diff.rs`), verbatim:

```rust
#[test]
fn working_tree_capability_triple() {
    let target = DiffTarget::WorkingTree;
    assert!(target.is_live());
    assert_eq!(target.staging_mode(), StagingMode::Stage);
    assert!(target.supports_code_intel());
}

#[test]
fn staged_capability_triple() {
    let target = DiffTarget::Staged;
    assert!(!target.is_live());
    assert_eq!(target.staging_mode(), StagingMode::Unstage);
    assert!(!target.supports_code_intel());
}

#[test]
fn range_capability_triple() {
    let target = DiffTarget::Range("main..HEAD".to_string());
    assert!(!target.is_live());
    assert_eq!(target.staging_mode(), StagingMode::ReadOnly);
    assert!(!target.supports_code_intel());
}
```

## Artifact: grep verification — no capability `matches!(...DiffTarget...)` remains in `src/ui/`

**What it proves:** Every capability-deciding call site (auto-refresh gate,
untracked-file injection, staging read-only guard + direction, the
`stage_file` guard, and the `staging_allowed`/help/footer computations) now
routes through `is_live()` / `staging_mode()` instead of matching
`DiffTarget` variants directly.

**Why it matters:** This is the task's explicit acceptance grep from the
task file (`docs/specs/05-spec-diff-sources/05-tasks-diff-sources.md`, task
1.0 proof artifacts) — the literal check for "no scattered checks remain."

**Command:**

```
grep -rn "matches!(.*DiffTarget" src/ui/
```

**Result summary:**

```
src/ui/app_tests.rs:848:        if matches!(target, DiffTarget::Staged)
```

This is the one remaining hit, in `FakeGit` — a `StageOps` test double in
`app_tests.rs` — and it is **not** a capability decision:

```rust
impl StageOps for FakeGit {
    fn diff(&self, target: &DiffTarget) -> Result<Vec<RawFilePatch>, GitError> {
        if matches!(target, DiffTarget::Staged)
            && let Some(staged) = &self.staged_diff
        {
            return Ok(staged.clone());
        }
        match &self.diff_override {
            Some(h) => Ok(h.borrow().clone()),
            None => Ok(self.diff.clone()),
        }
    }
    ...
```

It selects *which canned diff fixture to return* for a given target inside a
fake git backend — data selection standing in for what `git diff --staged`
vs. `git diff` would each return, not a "can I stage / should I refresh / is
code-intel valid" decision. It is out of this refactor's scope and was left
untouched.

## Artifact: full-suite pass with test counts before/after the refactor (1.2)

**What it proves:** The 1.2 refactor commit (`40312c9`) is move-only per
`docs/rust-best-practices.md`'s invariant: "identical test counts and zero
assertion edits before/after." No test was added, removed, or had an
assertion changed by that commit — only the five call sites' source lines
changed (`matches!` → capability-method calls).

**Why it matters:** This is the literal proof artifact the task file
requires for 1.2, and it's the safety net that the refactor didn't
accidentally change behavior for `WorkingTree`/`Staged`/`Range` (only the
later, separate `fix:` commit changes observable behavior, on purpose, for
code-intel).

**Command:**

```
cargo test 2>&1 | grep -E "Running|test result"
```

**Result summary — immediately before commit `40312c9`** (i.e. right after
`611ee3b`, which only *adds* the new capability model + its 3 tests, with no
call sites touched yet):

```
Running unittests src/lib.rs (target/debug/deps/redquill-0c548af423561ef3)
test result: ok. 802 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
Running tests/git_integration.rs ...        test result: ok. 14 passed
Running tests/git_remote_integration.rs ... test result: ok. 4 passed
Running tests/git_stage_integration.rs ...  test result: ok. 10 passed
Running tests/git_worktree_integration.rs...test result: ok. 6 passed
Running tests/lsp_integration.rs ...        test result: ok. 4 passed
```

**Result summary — immediately after commit `40312c9`** (the call-site
refactor):

```
Running unittests src/lib.rs (target/debug/deps/redquill-0c548af423561ef3)
test result: ok. 802 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
Running tests/git_integration.rs ...        test result: ok. 14 passed
Running tests/git_remote_integration.rs ... test result: ok. 4 passed
Running tests/git_stage_integration.rs ...  test result: ok. 10 passed
Running tests/git_worktree_integration.rs...test result: ok. 6 passed
Running tests/lsp_integration.rs ...        test result: ok. 4 passed
```

Identical in every binary: **802 passed / 2 ignored** (lib unit tests) and
**38 passed** across the four integration binaries, both before and after.
`git diff --stat` for the refactor commit touches only
`src/git/mod.rs`, `src/ui/app.rs`, `src/ui/footer.rs`, `src/ui/mod.rs`,
`src/ui/refresh.rs`, `src/ui/stage_ops.rs`, `src/ui/staging.rs` — no test
file is in that list.

**Full suite after the code-intel gating commit (1.3/1.4, this proof's
commit)** — expected to *grow* by exactly the new gating tests (this commit
is the deliberate behavior change, so it is not held to the move-only
invariant):

```
Running unittests src/lib.rs (target/debug/deps/redquill-0c548af423561ef3)
test result: ok. 810 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
Running tests/git_integration.rs ...        test result: ok. 14 passed
Running tests/git_remote_integration.rs ... test result: ok. 4 passed
Running tests/git_stage_integration.rs ...  test result: ok. 10 passed
Running tests/git_worktree_integration.rs...test result: ok. 6 passed
Running tests/lsp_integration.rs ...        test result: ok. 4 passed
```

810 = 802 + 8 new tests added by the gating commit (3 in
`ui::code_intel::tests`, 3 in `ui::help::tests`, 2 in `ui::footer::tests`) —
zero existing assertions edited.

## Artifact: `code_intel.rs` module-doc excerpt (degradation contract)

**What it proves:** The silent-degradation choice for code-intel off the
live working tree is written down as a deliberate contract, per
`docs/rust-best-practices.md`'s "Decide deliberately... and document the
contract in the module doc" rule.

**Why it matters:** Without this, "code-intel silently does nothing on a
range/staged view" would read as unexplained error-swallowing to the next
reader (or agent) instead of an intentional design decision with a stated
reason (resolving against mismatched on-disk content would be actively
misleading, which is the bug this task fixes).

**Command:**

```
sed -n '1,31p' src/ui/code_intel.rs
```

**Result summary** (verbatim, `src/ui/code_intel.rs` lines 1–31):

```rust
//! Code-intelligence glue: correlating the diff cursor with `gd`/`gr`/`K`
//! LSP requests, routing the responses into the peek overlay, and the
//! peek-overlay navigation. Kept out of [`super::App`] so the coordinator
//! stays thin; these functions take the view's cursor position as input and
//! drive the LSP client (via [`super::lsp_ops::LspClient`]) and the peek
//! state, never blocking the render loop.
//!
//! ## Degradation contract: code-intel is silently absent off the live working tree
//!
//! An LSP server only ever sees the file *as it sits on disk right now* — it
//! has no notion of "the version this diff's new side shows," so a request
//! is only meaningful when that new side *is* the on-disk working tree.
//! [`request`] and [`refresh_peek_preview`] both gate on
//! [`crate::git::DiffTarget::supports_code_intel`] and, when it's `false`
//! (every target but [`crate::git::DiffTarget::WorkingTree`] today — a
//! staged diff shows the index's content, a range diff shows two historical
//! revisions, neither backed by the file at that path on disk), degrade
//! silently: `gd`/`gr`/`K` set the same `"no code intelligence here"` footer
//! message as any other request that can't start (no repo root, missing
//! file, unsupported language), rather than an error. The alternative —
//! resolving the request against on-disk content that doesn't match what's
//! displayed — would silently jump to the wrong line or definition, which is
//! strictly worse than the feature being unavailable (this was the actual
//! bug on range views before this gate existed).
//!
//! The same predicate drives which of `gd`/`gr`/`K` even *appear* in the `?`
//! help overlay and footer strip (see [`super::help::binding_hidden`],
//! consumed by [`super::footer`]) — mirroring how the staging keys already
//! hide on a read-only target — so the degradation is structurally invisible
//! rather than a key that's listed but silently does nothing.
//!
//! Per the repository's error-handling rules, this degrade-silently choice is
//! deliberate and scoped to *this* subsystem only: it does not license
//! swallowing errors elsewhere (a real LSP failure still surfaces via
//! `"lsp: failed"`, see [`handle_event`]).
```

## Artifact: unit-test coverage standing in for the interactive `redquill main..HEAD` CLI proof

**What it proves:** Every layer the interactive proof would exercise —
request dispatch, peek-preview population, help-overlay row filtering, and
footer/pending-strip row filtering — is independently covered by a unit test
that fails if the gate is ever removed or the predicate is wired backwards.

**Why it matters:** The task file's CLI proof
(`redquill main..HEAD` with an LSP server configured shows no code-intel
keys in the `?` overlay or footer, and pressing them does nothing) cannot
run in this sandbox (see the TTY-deferred section below); these tests
exercise the same code paths the interactive session would, at the
function level, and are the durable regression guard either way (a human
running the manual check only proves it once; these tests prove it on every
future change).

**Command:**

```
cargo test --lib code_intel::tests
cargo test --lib help::tests
cargo test --lib footer
```

**Result summary:**

```
test ui::code_intel::tests::gd_on_a_staged_target_sets_no_code_intelligence_message_without_dispatching ... ok
test ui::code_intel::tests::gr_and_k_are_also_gated_on_a_range_target ... ok
test ui::code_intel::tests::peek_preview_refresh_is_a_noop_on_a_non_worktree_target ... ok
test result: ok. 27 passed; 0 failed; 0 ignored (ui::code_intel::tests)

test ui::help::tests::code_intel_actions_hidden_only_when_code_intel_disallowed ... ok
test ui::help::tests::staging_actions_are_unaffected_by_code_intel_allowed ... ok
test ui::help::tests::unrelated_actions_are_never_hidden_by_either_flag ... ok
test ui::help::tests::help_overlay_covers_every_keymap_binding ... ok
test result: ok. 4 passed; 0 failed; 0 ignored (ui::help::tests)

test ui::footer::tests::pending_g_drops_gd_and_gr_when_code_intel_is_disallowed ... ok
test ui::footer::tests::build_hints_drops_gd_and_gr_from_the_pending_strip_when_code_intel_is_disallowed ... ok
test result: ok. 46 passed; 0 failed; 0 ignored (ui::footer::tests + incidental deps)
```

Specifically:

- `gd_on_a_staged_target_sets_no_code_intelligence_message_without_dispatching`
  / `gr_and_k_are_also_gated_on_a_range_target`: drive `Action::GotoDefinition`
  / `GotoReferences` / `Hover` through the real `App::apply` dispatch with
  `app.target` set to `Staged` / `Range`, and assert the fake LSP client's
  call log stays empty while the footer shows `"no code intelligence here"`
  — this is the "pressing them does nothing" half of the CLI proof.
- `peek_preview_refresh_is_a_noop_on_a_non_worktree_target`: defense-in-depth
  check that `refresh_peek_preview` itself won't populate the preview cache
  even if called directly against a non-worktree target.
- `code_intel_actions_hidden_only_when_code_intel_disallowed`: drives
  `help::binding_hidden` directly for `GotoDefinition`/`GotoReferences`/`Hover`
  against both `code_intel_allowed` values — this is the "no code-intel keys
  in the `?` overlay" half.
- `pending_g_drops_gd_and_gr_when_code_intel_is_disallowed` /
  `build_hints_drops_gd_and_gr_from_the_pending_strip_when_code_intel_is_disallowed`:
  covers the footer's pending two-key-prefix strip (`g` → `gd`/`gg`/`gr`),
  asserting `gd`/`gr` drop out while the unrelated `gg` (JumpToTop) survives
  — this is the "no code-intel keys in the footer" half.

### TTY-deferred proofs (operator)

The literal interactive CLI proof — launching `redquill main..HEAD` in a
real terminal with an LSP server configured, opening the `?` overlay, and
confirming no `gd`/`gr`/`K` rows appear (and that pressing them in the diff
view does nothing) — **cannot run in this sandbox**: `enable_raw_mode`
fails with `os error 6` (no controlling TTY), so the TUI can't start at all
here. The unit tests above exercise the same code paths, but a human should
still confirm the end-to-end interactive experience once. Exact repro
steps:

1. In a real terminal (not this sandbox), from the repo root:
   `git log --oneline -3` to confirm there are at least two commits, then
   run `redquill main..HEAD` (or any `A..B`/`A...B` range against a repo with
   commits) — or `redquill --staged` after `git add`-ing a file, for the
   `Staged` case.
2. Move the cursor onto an added/context line of any file section.
3. Press `gd`, then `gr`, then `K` — each should leave the view unchanged
   (no peek overlay opens) — the footer/status line should show nothing new
   (no `"lsp: resolving…"` message), confirming the request never dispatched.
4. Press `?` to open the help overlay; confirm the "Code intelligence"
   group section (`gd`/`gr`/`K`) is entirely absent — compare against
   `redquill` with no arguments (working tree, live) in the same repo, where
   the group *is* present.
5. Confirm the footer strip (bottom of the screen) never shows a `definition`
   / `references` hint while a `g` prefix is pending (press `g` alone and
   observe only `gg`'s "top" hint, if `gg`'s two-key completion strip is
   showing at all).

## Reviewer Conclusion

All four proof artifacts required by the task file are satisfied:
the capability-triple unit tests pass and are exhaustive per variant; the
grep verification confirms no capability-deciding `matches!(...DiffTarget...)`
remains in `src/ui/` (the one remaining hit is an unrelated test-fixture
data selector); the full suite passes with identical counts across the
move-only refactor commit and grows only by new, purpose-built tests in the
subsequent behavior-changing commit; and the code-intel degradation contract
is documented in the module doc it governs. The interactive CLI proof is
covered at the unit level and deferred to a human with TTY access per the
exact steps above. `cargo build`, `cargo test`, `cargo clippy --all-targets
-- -D warnings`, and `cargo fmt --check` all pass clean on every commit in
this task.
