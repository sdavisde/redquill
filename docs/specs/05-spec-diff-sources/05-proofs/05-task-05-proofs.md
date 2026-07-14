# Task 05 Proofs - Empty-diff welcome state

## Task Summary

Parent task 5.0 (spec 05 Unit 5) replaces the diff pane's old bare
`"no changes"` placeholder with a situational welcome block whenever the
active target's review has zero files — the "agent already committed, clean
working tree" dead end the whole spec exists to fix.

1. **Empty-state detection + welcome block (5.1)** — the detection point is
   the existing single place the diff pane already asked "is there anything
   to show?": `app.view.files.is_empty()` in `diff_view::render`
   (`src/ui/diff_view.rs`). This is a genuine boundary, not a re-check bolted
   on: `view.files` is fully replaced by every route that changes what's
   being reviewed — the initial `build_review` (`App::with_git`), an
   auto-refresh delivering a fresh `ReviewSnapshot` (`apply_snapshot`,
   `src/ui/refresh.rs`), and opening a commit from the History tab
   (`open_commit_view`, `src/ui/git_panel.rs`) — so there is exactly one
   render-time check to change, not several. New module `src/ui/welcome.rs`
   owns the block: `situation()` (one line naming *why* the pane is empty,
   worded per `DiffTarget` variant — `"No uncommitted changes"` /
   `"Nothing staged"` / `"Empty diff for <range>"` /
   `"Empty commit diff for <short-sha>"`) and `render()` (centers the
   situation line plus hints inside the existing bordered block, reusing
   `help::centered`, promoted from `fn` to `pub(super) fn` rather than
   duplicating the two-axis `Flex::Center` helper).
2. **Keymap-sourced hints, no literals (5.2)** — `HINT_SPECS`, a `const`
   table of `(Scope, Action, label)` triples (open the git panel /
   `FocusGitPanel`, switch to the History tab / `TogglePanelTab`, open help /
   `ToggleHelp`), is the single source of truth `hints()` resolves against
   `Keymap::default_map()` at render time via `key_for` — no key string
   appears anywhere in the welcome text. `hints()` degrades silently
   (`filter_map`, never panics) if a spec's binding vanishes from the table —
   documented in the module doc as a deliberate, cosmetic-only degradation —
   while the drift test `welcome_hints_resolve_for_every_spec` asserts every
   spec *does* resolve, so CI (not the running app) is what catches a rename
   or removal.
3. **UI-state lifecycle tests + proof capture (5.3)** — `TestBackend` render
   tests in `mod_tests.rs` cover empty → welcome, target-appropriate wording
   per target, and welcome → gone once `apply_snapshot` delivers content; a
   real-git integration test in `history_integration_tests.rs` covers the
   commit-view case explicitly (an `--allow-empty` commit opened from
   History). An `#[ignore]`d test (mirroring the existing
   `capture_task_0N_smoke_transcript` convention) regenerates a
   rendered-buffer text capture standing in for the interactive screenshot
   this sandbox's missing TTY can't take.

Landed as one commit on `worktree-diff-sources` (this proof file's commit):
`feat(ui): show a keyed welcome state instead of a blank empty diff`.

## What This Task Proves

- Every diff target's empty case shows situational wording, not a generic
  placeholder or a blank buffer: working tree, staged, an explicit range, and
  a commit each get their own phrasing, including the explicit "what should
  an empty commit diff say" question the task raised — `"Empty commit diff
  for <short-sha>"`, falling back to the raw rev string if `active_commit`
  metadata is somehow absent.
- Every hinted key is read from the shared keymap table at render time, never
  hardcoded — proven both by construction (`HINT_SPECS` names actions, not
  strings) and by a drift test that fails loudly (demonstrated live, not just
  argued) when a spec's `(scope, action)` pair no longer resolves.
- The welcome state's lifecycle is correct: it appears the instant a target
  has zero files and disappears the instant content arrives via the same
  `apply_snapshot` path auto-refresh uses — no separate "clear welcome" flag
  to go stale.
- A rendered-buffer text capture (`05-task-05-welcome-buffer.txt`, generated
  by an explicit, re-runnable test) stands in for the interactive screenshot
  proof, with exact manual steps recorded for an operator with a real
  terminal.
- The existing wall-clock perf tripwires (`src/ui/perf_tests.rs`, file
  byte-for-byte unmodified) still pass — the welcome state adds no
  render-loop complexity-class regression (it only replaces an
  already-short-circuited empty-files branch).

## Evidence Summary

| Proof | Result |
| --- | --- |
| `welcome::situation` per-target wording (working tree / staged / range / commit with and without header metadata) | 5/5 pass |
| `welcome::hints` resolves one hint per spec, in order, with real (non-empty) keys | 1/1 pass |
| Drift test: every `HINT_SPECS` entry resolves against `Keymap::default_map()` | 1/1 pass |
| Drift test failure demonstrated live (temporary mutation, captured, reverted) | 1/1 fails as expected, then reverts clean |
| UI-state: empty working-tree target renders the welcome block + all three hints, old `"no changes"` text gone | 1/1 pass |
| UI-state: Staged/Range targets get their own wording, not the working-tree phrase | 1/1 pass |
| UI-state: welcome clears once `apply_snapshot` delivers a file | 1/1 pass |
| Real-git: History-opened commit with an empty diff (`--allow-empty`) shows `"Empty commit diff for <short-sha>"`, not the working-tree wording | 1/1 pass |
| Rendered-buffer text capture (stand-in screenshot) regenerated and inspected | 1/1 pass, output below |
| Perf tripwires, unmodified budgets (`src/ui/perf_tests.rs` byte-identical) | 5/5 pass |
| Full lib unit suite before/after this task | 872 passed → 883 passed (+12 new tests [1 `#[ignore]`d], 1 removed [superseded by 5.3's lifecycle test], 0 other existing assertions edited) |
| Full integration suite (`tests/*.rs`, untouched by this task) | 45 passed across 6 binaries, unchanged |
| Four gates (build/test/clippy `--all-targets`/fmt) | All pass |

## Artifact: lifecycle tests (task 5.3) — empty → welcome, target wording, welcome → gone

**What it proves:** the three UI-state assertions the task's proof artifact
calls for by name.

**Tests (`src/ui/mod_tests.rs`), verbatim:**

```rust
#[test]
fn empty_working_tree_target_shows_welcome_state() {
    let app = App::new(vec![]);
    let keymap = Keymap::default_map();
    let content = rendered_content(&app, &keymap);

    assert!(content.contains("No uncommitted changes"));
    // Hints come from the table: FocusGitPanel is bound to `` ` `` and
    // ToggleHelp to `?` in Scope::Diff by default (see `keymap.rs`).
    assert!(content.contains("open the git panel"));
    assert!(content.contains("switch to the History tab"));
    assert!(content.contains("open help"));
    assert!(
        !content.contains("no changes"),
        "old placeholder must be gone"
    );
}

#[test]
fn welcome_state_uses_target_appropriate_wording_per_target() {
    let keymap = Keymap::default_map();

    let mut staged_app = App::new(vec![]);
    staged_app.target = DiffTarget::Staged;
    assert!(rendered_content(&staged_app, &keymap).contains("Nothing staged"));

    let mut range_app = App::new(vec![]);
    range_app.target = DiffTarget::Range("main..HEAD".to_string());
    assert!(
        rendered_content(&range_app, &keymap).contains("Empty diff for main..HEAD"),
        "range wording must name the range as typed"
    );
}

#[test]
fn welcome_state_clears_once_a_snapshot_delivers_content() {
    let mut app = App::new(vec![]);
    let keymap = Keymap::default_map();
    assert!(rendered_content(&app, &keymap).contains("No uncommitted changes"));

    app.apply_snapshot(ReviewSnapshot {
        files: vec![sample_file()],
        patches: vec![None],
        staged: Vec::new(),
        staged_states: std::collections::HashMap::new(),
    });

    let content = rendered_content(&app, &keymap);
    assert!(
        !content.contains("No uncommitted changes"),
        "welcome text must clear once the target has content"
    );
    assert!(
        content.contains("src/main.rs"),
        "the delivered file must render"
    );
}
```

`rendered_content` is a small shared helper added alongside these tests
(renders `app` to an 80x20 `TestBackend` via the real `draw()` the blocking
event loop calls, and flattens the buffer to a string) — the same pattern
every other render test in this file already used, just factored out once
three tests needed it instead of one.

**Command:**

```
cargo test --lib ui::tests::empty_working_tree_target_shows_welcome_state \
           ui::tests::welcome_state_uses_target_appropriate_wording_per_target \
           ui::tests::welcome_state_clears_once_a_snapshot_delivers_content
```

**Result:**

```
running 3 tests
test ui::tests::welcome_state_uses_target_appropriate_wording_per_target ... ok
test ui::tests::empty_working_tree_target_shows_welcome_state ... ok
test ui::tests::welcome_state_clears_once_a_snapshot_delivers_content ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 882 filtered out; finished in 0.00s
```

## Artifact: History-opened commit with an empty diff → commit-appropriate wording (task 5.3)

**What it proves:** the fourth lifecycle case named explicitly in the task's
proof artifact — a real, disposable repository where the newest commit is an
`--allow-empty` commit (introduces no changes relative to its parent). Opening
it from the History tab yields `DiffTarget::Commit(sha)` whose own diff has
zero files, and the welcome block shows `"Empty commit diff for
<short-sha>"` — not the working-tree phrase, not a blank pane.

**Fixture + test (`src/ui/history_integration_tests.rs`), verbatim:**

```rust
fn repo_with_history_and_a_trailing_empty_commit() -> TempDir {
    let tmp = repo_with_history();
    git(
        tmp.path(),
        &["commit", "-qm", "empty commit", "--allow-empty"],
    );
    tmp
}

#[test]
fn opening_a_commit_with_an_empty_diff_shows_commit_appropriate_welcome_wording() {
    let tmp = repo_with_history_and_a_trailing_empty_commit();
    let mut app = app_for(tmp.path());
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    open_history_tab(&mut app, &keymap, &mut pending);
    assert_eq!(app.history[0].subject, "empty commit", "newest first");
    press(&mut app, &keymap, &mut pending, KeyCode::Enter);

    assert!(
        app.view.files.is_empty(),
        "the opened commit must have introduced no changes"
    );
    let short_sha = app
        .active_commit
        .clone()
        .expect("opening a commit sets active_commit")
        .short_sha;

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

    let expected = format!("Empty commit diff for {short_sha}");
    assert!(
        content.contains(&expected),
        "expected {expected:?} in:\n{content}"
    );
    assert!(
        !content.contains("No uncommitted changes"),
        "must not reuse the working-tree wording for a commit target"
    );
}
```

**Command:**

```
cargo test --lib history_integration_tests::opening_a_commit_with_an_empty_diff_shows_commit_appropriate_welcome_wording
```

**Result:**

```
running 1 test
test ui::history_integration_tests::opening_a_commit_with_an_empty_diff_shows_commit_appropriate_welcome_wording ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 885 filtered out; finished in 0.19s
```

## Artifact: hint keys are table-sourced, and the drift test fails loudly when they aren't (task 5.2)

**What it proves:** `HINT_SPECS` names actions, never key literals, and
`welcome_hints_resolve_for_every_spec` genuinely catches a broken mapping —
demonstrated live by temporarily mutating one spec, capturing the failure,
then reverting (matching the "whichever the existing drift tests' style
uses" instruction; this repo's precedent — e.g. `05-task-03-proofs.md`'s
keymap/help drift section — argues from the exhaustive-match/table structure
rather than mutating live code, but a live demonstration is strictly stronger
evidence and the mutation is trivial to revert cleanly here).

**The table and the drift test (`src/ui/welcome.rs`), verbatim:**

```rust
const HINT_SPECS: [(Scope, Action, &str); 3] = [
    (Scope::Diff, Action::FocusGitPanel, "open the git panel"),
    (
        Scope::Panel,
        Action::TogglePanelTab,
        "switch to the History tab to review recent commits",
    ),
    (Scope::Diff, Action::ToggleHelp, "open help"),
];

#[test]
fn welcome_hints_resolve_for_every_spec() {
    let km = Keymap::default_map();
    for &(scope, action, label) in &HINT_SPECS {
        assert!(
            key_for(&km, scope, action).is_some(),
            "no {scope:?} binding for {action:?} (hint {label:?}) — \
             the shared keymap table no longer has this action"
        );
    }
}
```

**Demonstration — mutated `TogglePanelTab`'s spec from `Scope::Panel` (where
it's actually bound) to `Scope::Diff` (where it is not), ran the drift test,
captured the failure, then reverted the file byte-for-byte (`diff` against
the pre-mutation copy showed no difference afterward):**

```
$ cargo test --lib welcome_hints_resolve_for_every_spec

running 1 test
test ui::welcome::tests::welcome_hints_resolve_for_every_spec ... FAILED

failures:

---- ui::welcome::tests::welcome_hints_resolve_for_every_spec stdout ----

thread 'ui::welcome::tests::welcome_hints_resolve_for_every_spec' panicked at src/ui/welcome.rs:217:13:
no Diff binding for TogglePanelTab (hint "switch to the History tab to review recent commits") — the shared keymap table no longer has this action

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 885 filtered out
```

**After reverting** (`cp` from a pre-mutation backup, verified with `diff`
that the file matched exactly):

```
$ cargo test --lib welcome_hints_resolve_for_every_spec

running 1 test
test ui::welcome::tests::welcome_hints_resolve_for_every_spec ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 885 filtered out; finished in 0.00s
```

This is the concrete evidence a renamed or removed action fails CI loudly,
not a hint silently going missing from the screen (which is exactly what
`hints()`'s own `filter_map` would do at runtime — a deliberate, documented,
cosmetic-only degradation; the drift test is what makes that degradation
visible to a reviewer instead of shipping unnoticed).

## Artifact: rendered-buffer text capture (stand-in for the screenshot proof)

**What it proves:** the task's screenshot proof artifact — "`redquill`
launched in a clean tempdir repo shows the welcome state (situation text +
keyed hints: open panel, History tab, `?` help) instead of a blank buffer" —
captured as an exact terminal-buffer text dump instead of a pixel screenshot,
since this sandbox has no controlling TTY (see the TTY-deferred section
below for the literal steps an operator can run instead).

**Capture test (`src/ui/mod_tests.rs`), re-runnable via
`cargo test capture_task_05_welcome_buffer -- --ignored`:**

```rust
#[test]
#[ignore = "writes the task-05 welcome-buffer proof artifact; run explicitly"]
fn capture_task_05_welcome_buffer() {
    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let app = App::new(vec![]);
    let keymap = Keymap::default_map();
    terminal
        .draw(|frame| draw(frame, &app, &keymap, None))
        .unwrap();
    // ... flattens buffer rows to text, writes to
    // docs/specs/05-spec-diff-sources/05-proofs/05-task-05-welcome-buffer.txt
}
```

**Captured output (`05-task-05-welcome-buffer.txt`), verbatim:**

```text
Task 5.0 proof — empty working-tree welcome state, TestBackend(80x20)
Rendered via the real `draw()` the blocking event loop calls; the
only difference from a live terminal is the backend (no controlling
TTY in this sandbox — see 05-task-05-proofs.md's TTY-deferred section).

┌diff──────────────────────────────────────────────────────────────────────────┐
│                                                                              │
│                                                                              │
│                                                                              │
│                                                                              │
│                                                                              │
│                                                                              │
│                            No uncommitted changes                            │
│                                                                              │
│                             ` open the git panel                             │
│            Tab switch to the History tab to review recent commits            │
│                                  ? open help                                 │
│                                                                              │
│                                                                              │
│                                                                              │
│                                                                              │
│                                                                              │
└──────────────────────────────────────────────────────────────────────────────┘
 j/k move · ] hunk · za fold · Space stage hunk · S stage file · c comment
 / search · ` git panel · ? help
```

The situation line and all three hints are centered both horizontally and
vertically inside the bordered diff pane, each hint's key (`` ` ``, `Tab`,
`?`) matching exactly what the footer strip below independently derives from
the same keymap table (`` ` `` git panel, `?` help) — two independent
consumers of `Keymap::default_map()` agreeing is itself a small proof the
table is the single source of truth.

## Artifact: full-suite pass with test counts before/after this task

**What it proves:** every new test is new; the one removed test
(`empty_diff_shows_no_changes_message`) is superseded by
`empty_working_tree_target_shows_welcome_state`, which asserts strictly more
(the new wording, all three hints, and the absence of the old placeholder
text) — not a weakened replacement. No other existing test's assertions were
edited. Every `tests/*.rs` integration binary is untouched.

**Command:**

```
cargo test 2>&1 | grep -E "Running|test result"
```

**Result summary — after this task's commit:**

```
Running unittests src/lib.rs (target/debug/deps/redquill-...)
test result: ok. 883 passed; 0 failed; 3 ignored; 0 measured; 0 filtered out
Running tests/git_integration.rs ...          test result: ok. 17 passed
Running tests/git_log_integration.rs ...      test result: ok. 4 passed
Running tests/git_remote_integration.rs ...   test result: ok. 4 passed
Running tests/git_stage_integration.rs ...    test result: ok. 10 passed
Running tests/git_worktree_integration.rs...  test result: ok. 6 passed
Running tests/lsp_integration.rs ...          test result: ok. 4 passed
```

883 lib unit tests = 872 (end of task 4.0, per `05-task-04-proofs.md`) − 1
(`empty_diff_shows_no_changes_message`, superseded) + 12 new: 8
`ui::welcome::tests::*` (situation wording × 5, hint resolution × 2, drift
test × 1), 3 `ui::tests::*welcome*` lifecycle tests, 1
`ui::history_integration_tests::opening_a_commit_with_an_empty_diff_shows_commit_appropriate_welcome_wording`
— plus 1 more new test, `ui::tests::capture_task_05_welcome_buffer`, which is
`#[ignore]`d (proof-artifact generator, not part of the default suite),
accounting for the ignored count rising from 2 to 3. All six `tests/*.rs`
integration binaries are unchanged at 17, 4, 4, 10, 6, and 4 respectively.
`src/ui/perf_tests.rs` is untouched by this task (`git diff --stat --
src/ui/perf_tests.rs` produces no output).

## Four gates

```
cargo build                                # Finished, no warnings
cargo test                                 # 883 lib + 45 integration, all pass
cargo clippy --all-targets -- -D warnings  # Finished, no warnings
cargo fmt --check                          # no diff
```

## TTY-deferred proofs (operator)

This sandbox has no controlling TTY (`enable_raw_mode` fails with os error
6), so the task's literal screenshot cannot be captured here. The rendered-
buffer text capture above (from a real `TestBackend` rendering the actual
`draw()` function the event loop calls) is the equivalent evidence — same
render code path, same layout math, same keymap lookups, only the backend
differs. An operator with a real terminal can reproduce the literal
screenshot:

1. In an empty tempdir, run `git init && git config user.email
   test@example.com && git config user.name test && git commit --allow-empty
   -qm "seed"` to get a repo with a commit but a clean working tree — the
   "agent already committed" dead end.
2. Run `cargo run --` from that repo's root.
3. Observe the diff pane: instead of a blank buffer, it shows "No
   uncommitted changes" centered, followed by three hints — `` ` `` open the
   git panel, `Tab` switch to the History tab to review recent commits, `?`
   open help — each key exactly matching what `` ` `` / `Tab` (once the panel
   is focused) / `?` actually do.
4. Press `` ` `` — the git panel opens. Press `Tab` — the panel switches to
   the History tab, listing the seed commit. Press `Enter` on it — the
   commit's own diff (empty, since it was created with `--allow-empty`) opens
   in the multibuffer, and the welcome block now reads "Empty commit diff for
   `<short-sha>`" instead of the working-tree phrase.
5. Press `Esc` to return, then `?` to confirm the help overlay opens and
   lists `` ` ``/`Tab`/`?` among its bindings — same keys the welcome hints
   promised.
6. Take the screenshot at step 3 (and optionally step 4) and attach it
   alongside this proof file.

## Reviewer Conclusion

All three proof artifacts required by the task file are satisfied: the
welcome block renders situational wording for every target (working tree,
staged, range, and the explicitly-asked-about empty commit diff) with 3–4
action hints; every hint's key is sourced from the shared keymap table with
no literal in the welcome text, and the drift test is proven to fail loudly
(demonstrated live, then cleanly reverted) rather than let a renamed or
missing action degrade silently past CI; and the UI-state lifecycle — empty
→ welcome, content arrives → welcome gone, a History-opened empty commit →
commit-appropriate wording — is covered by both `TestBackend` render tests
and a real-git integration test, with a rendered-buffer text capture standing
in for the screenshot this sandbox's missing TTY can't take (exact manual
steps recorded above for an operator to capture the real one). `cargo
build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, and `cargo
fmt --check` all pass clean, with test counts growing from 872 to 883
passing lib tests (net +12 new, 1 removed as superseded, all six integration
binaries unchanged) and zero other existing assertions edited.
