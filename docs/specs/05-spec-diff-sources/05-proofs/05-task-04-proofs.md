# Task 04 Proofs - Source-aware annotation output

## Task Summary

Parent task 4.0 (spec 05 Unit 4) makes the annotation stdout format
self-describing when a session leaves the working-tree view: every
annotation now records the [`Source`](../../../../src/annotate/model.rs) it
was authored against, and `render_markdown` groups the output by source —
working-tree annotations first with no metadata line (byte-identical to the
format before this task), then each non-worktree group preceded by exactly
one `Reviewing: <spec>` line.

1. **Model (4.1)** — `annotate::model::Source` is a new, `annotate`-owned
   plain enum (`WorkingTree` / `Staged` / `Range(String)` / `Commit(String)`),
   deliberately **not** a re-export of `git::DiffTarget`. The module doc on
   `Source` records this as a deliberate cross-layer decision (per
   `docs/rust-best-practices.md`'s "cross-layer type coupling... when it's
   deliberate, record it"): `DiffTarget` also carries UI-facing capability
   methods (`is_live`/`staging_mode`/`supports_code_intel`) `annotate` has no
   business depending on, and its `Commit`/`Range` payloads are raw git
   rev-specs the caller (the `ui` layer, which already depends on both `git`
   and `annotate`) is responsible for resolving into the small display value
   `Source` actually needs. `Annotation` gained a `source: Source` field,
   defaulted via `Source::default() == Source::WorkingTree`.
   `AnnotationStore::add` (used by every existing call site: `rows.rs`,
   `modal_keys.rs`, and every existing test fixture) is unchanged in
   signature and now delegates to a new `add_with_source`, which is the only
   place `Annotation` is constructed. `App::annotation_source()`
   (`src/ui/app.rs`) is the one production call site deriving a `Source`
   from the live `DiffTarget` at `submit_compose` time — using
   `active_commit`'s already-`core.abbrev`-aware `short_sha` for a commit
   target rather than having `annotate` (or this method) recompute an
   abbreviation of its own, with a defensive (non-panicking) fallback to the
   full rev string if a commit is somehow open with no matching
   `active_commit` entry.
2. **Grouped emission (4.2)** — byte-exact tests were written first (red),
   then `annotate::markdown::group_by_source` (partition by `Source`, first
   appearance order, working-tree group forced to the front) and
   `render_group` (the `Reviewing:` line, omitted only for `WorkingTree`)
   implement the grouping. `render_markdown`'s public signature is
   unchanged.
3. **Format contract (4.3)** — the exact `Reviewing:` syntax
   (`abc1234` / `main..feature` / `staged`) is documented in
   `annotate::markdown`'s module doc and a new "Annotation output format"
   README section (present tense only — the format described is exactly
   what ships).
4. **Scripted proof + gates (4.4)** — see the real-git integration test
   below (this sandbox has no controlling TTY, so the interactive rehearsal
   is recorded as a TTY-deferred proof with exact steps); all four gates
   pass.

Landed as one commit on `worktree-diff-sources` (this proof file's commit):
`feat(annotate): group annotation output by diff source with a Reviewing: line`.

## What This Task Proves

- A working-tree-only session's `render_markdown` output is byte-identical
  to the pre-task format — proven both by the fact that every pre-existing
  test in `annotate/markdown.rs` passes with its expected string
  byte-for-byte unedited (see the `git diff --stat` proof below) and by a
  new explicit test naming that guarantee.
- A mixed session (working-tree + non-worktree annotations) groups the
  working-tree annotations first with no metadata line, then each other
  group preceded by exactly one `Reviewing:` line — proven byte-exactly for
  all three non-worktree kinds (commit/range/staged), for multiple
  annotations sharing one group, for multiple groups in first-appearance
  order, and for the "commit-first, working-tree-second insertion order"
  case (working-tree must still render first).
- A real, disposable git repository, driven through the actual key-dispatch
  pipeline (open History tab -> open a commit -> annotate a line -> render
  the store), produces a `Reviewing: <short-sha>` line naming the exact
  commit that was opened.
- The `annotate` module doc and README both document the `Reviewing:` syntax
  in present tense, matching exactly what `render_group` emits.

## Evidence Summary

| Proof | Result |
| --- | --- |
| `annotate::model` new tests (`Source` default/label contract) | 2/2 pass |
| `annotate::store` new tests (`add` defaults to `WorkingTree`; `add_with_source` records the given source) | 2/2 pass |
| `annotate::markdown` pre-existing byte-exact fixtures (unedited — see `git diff --stat`) | 13/13 pass, unchanged |
| `annotate::markdown` new grouping tests (working-tree-only, mixed, per-kind `Reviewing:` line, same-group sharing, multi-group ordering, working-tree-always-first, no-leading-blank-group) | 9/9 pass |
| Real-git scripted CLI proof: annotate a commit-view line, render, assert `Reviewing: <short-sha>` | 1/1 pass |
| Full lib unit suite before/after this task | 858 passed → 872 passed (+14 new tests, 0 removed/edited existing assertions) |
| Full integration suite (`tests/*.rs`, untouched by this task) | 45 passed across 6 binaries, unchanged |
| Four gates (build/test/clippy `--all-targets`/fmt) | All pass |

## Artifact: byte-exact backward compatibility — existing fixtures unedited (task 4.2a)

**What it proves:** Every pre-existing test in `src/annotate/markdown.rs`
still asserts the exact same expected string it did before this task — the
only changes to the file are the module doc, the `Source` import, the new
`group_by_source`/`render_group` helpers, and new test functions appended
after the existing ones. No existing `assert_eq!` expected string was
touched.

**Why it matters:** This is the task's explicit 4.0/4.2 acceptance
requirement (FR: "byte-identical default") and the top-level spec
requirement that working-tree-only sessions remain byte-identical on
stdout.

**Command:**

```
git diff --stat -- src/annotate/markdown.rs
git diff -- src/annotate/markdown.rs | grep '^-' | grep -v '^---'
```

**Result:**

```
 src/annotate/markdown.rs | 278 +++++++++++++++++++++++++++++++++++++++++++++--
 1 file changed, 270 insertions(+), 8 deletions(-)
```

The 8 deleted lines, verbatim (module doc rewording, the `Source` import
addition, and the `render_markdown` body swapped for the new grouped
implementation — no test assertion is among them):

```
-//! [`render_markdown`] is the format emitted on quit to stdout by the
-//! future UI. Treat it as a public API once shipped:
-use super::model::{Annotation, Classification, Side, Target};
-/// Renders every annotation in `store`, in insertion order, as the
-/// public-contract markdown format. An empty store renders to an empty
-/// string; otherwise the output ends with a single trailing newline and
-/// annotations are separated by exactly one blank line.
-    let blocks: Vec<String> = store.iter().map(render_one).collect();
```

The explicit backward-compatibility test added alongside the pre-existing
fixtures (`src/annotate/markdown.rs`), verbatim:

```rust
#[test]
fn working_tree_only_session_has_no_reviewing_line() {
    // Every existing test above already builds a working-tree-only
    // session via `add` (which defaults to `Source::WorkingTree`) and
    // asserts an exact expected string with no `Reviewing:` line — this
    // is the explicit backward-compatibility assertion the task calls
    // for, phrased as its own test.
    let mut store = AnnotationStore::new();
    store
        .add(
            Target::range("src/lib.rs", 10, 20, Side::New).unwrap(),
            Classification::Nit,
            "extract this into a helper",
        )
        .unwrap();
    let expected = "## src/lib.rs:10-20 (+)\n\n[nit] extract this into a helper\n";
    assert_eq!(render_markdown(&store), expected);
    assert!(!render_markdown(&store).contains("Reviewing:"));
}
```

**Test run:**

```
cargo test --lib annotate::markdown
```

```
test result: ok. 22 passed; 0 failed; 0 ignored; 0 measured; 850 filtered out; finished in 0.00s
```

## Artifact: mixed-session byte-exact grouping (task 4.2b)

**What it proves:** A store with one working-tree annotation and one
commit annotation renders the working-tree group first with no metadata
line, then exactly one `Reviewing: <sha>` line, then the commit group — the
task's literal mixed-fixture requirement.

**Test (`src/annotate/markdown.rs`), verbatim:**

```rust
#[test]
fn mixed_session_groups_working_tree_first_then_one_reviewing_line_per_group() {
    let mut store = AnnotationStore::new();
    store
        .add(
            Target::range("src/lib.rs", 10, 20, Side::New).unwrap(),
            Classification::Nit,
            "extract this into a helper",
        )
        .unwrap();
    store
        .add_with_source(
            Target::line("src/auth/session.rs", 44, Side::New),
            Classification::Question,
            "where does keystore get rotated?",
            Source::Commit("abc1234".to_string()),
        )
        .unwrap();
    let expected = "## src/lib.rs:10-20 (+)\n\n\
         [nit] extract this into a helper\n\n\
         Reviewing: abc1234\n\n\
         ## src/auth/session.rs:44 (+)\n\n\
         [question] where does keystore get rotated?\n";
    assert_eq!(render_markdown(&store), expected);
    assert_eq!(render_markdown(&store).matches("Reviewing:").count(), 1);
}
```

**Rendered output (exactly the `expected` string above, this is the actual
byte-for-byte document `render_markdown` produces):**

```text
## src/lib.rs:10-20 (+)

[nit] extract this into a helper

Reviewing: abc1234

## src/auth/session.rs:44 (+)

[question] where does keystore get rotated?
```

Companion tests cover the remaining per-kind syntax and edge cases:
`commit_group_reviewing_line_uses_short_sha`,
`range_group_reviewing_line_uses_range_as_typed`,
`staged_group_reviewing_line_is_literally_staged`,
`multiple_annotations_in_the_same_non_worktree_group_share_one_reviewing_line`,
`different_non_worktree_sources_get_separate_reviewing_lines_in_first_appearance_order`,
`working_tree_group_always_emitted_first_regardless_of_insertion_order` (commit
annotation added *before* the working-tree one — proves the reorder, not
just the happy-path ordering), and
`non_worktree_only_session_has_no_leading_blank_group`.

**Command:**

```
cargo test --lib annotate::markdown::tests
```

**Result:**

```
test result: ok. 22 passed; 0 failed; 0 ignored; 0 measured; 850 filtered out; finished in 0.00s
```

## Artifact: scripted CLI proof — real-git commit annotation shows `Reviewing: <short-sha>` (task 4.4)

**What it proves:** Against a real, throwaway 3-commit repository, driven
through the actual key-dispatch pipeline — focus the git panel, switch to
History, open the newest commit, land on a line row, compose an annotation,
submit it, then render the store — the emitted document contains
`Reviewing: <short-sha>` naming exactly the commit that was opened
(`app.active_commit`'s SHA), with exactly one such line.

**Why it matters:** This is the task's explicit CLI proof artifact
("scripted session annotating a commit shows `Reviewing: <short-sha>` in
stdout") and Success Metric 2 from the spec (an agent must be able to
resolve annotated sites against the right revision). `repo_with_history()`
(the shared fixture from task 3.0's integration tests) intentionally ends
with a **clean working tree** — "the agent already committed" scenario this
whole spec targets — so there is no working-tree diff to additionally
annotate in the same real repo; the working-tree-first / mixed-grouping
*ordering* itself needs no real git process and is proven byte-exactly by
the mixed-session test above instead.

**Test (`src/ui/history_integration_tests.rs`), verbatim:**

```rust
#[test]
fn annotating_a_commit_view_records_a_reviewing_line_with_the_short_sha() {
    let tmp = repo_with_history();
    let mut app = app_for(tmp.path());
    let keymap = Keymap::default_map();
    let mut pending: Option<KeyEvent> = None;

    open_history_tab(&mut app, &keymap, &mut pending);
    press(&mut app, &keymap, &mut pending, KeyCode::Enter);
    let opened_short_sha = app
        .active_commit
        .clone()
        .expect("opening a commit sets active_commit")
        .short_sha;

    advance_to_line_row(&mut app, &keymap, &mut pending);
    assert!(
        matches!(app.view.rows.get(app.view.cursor), Some(Row::Line(_))),
        "commit view must have a line row to annotate"
    );
    press(&mut app, &keymap, &mut pending, KeyCode::Char('c'));
    for ch in "reviewed against the commit".chars() {
        press(&mut app, &keymap, &mut pending, KeyCode::Char(ch));
    }
    press(&mut app, &keymap, &mut pending, KeyCode::Enter);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.annotations.len(), 1);

    let rendered = crate::annotate::render_markdown(&app.annotations);
    let expected_reviewing_line = format!("Reviewing: {opened_short_sha}");
    assert!(
        rendered.contains(&expected_reviewing_line),
        "expected {expected_reviewing_line:?} in:\n{rendered}"
    );
    assert_eq!(
        rendered.matches("Reviewing:").count(),
        1,
        "exactly one Reviewing: line for the single non-worktree group:\n{rendered}"
    );
    assert!(
        rendered.starts_with(&expected_reviewing_line),
        "no working-tree group precedes the only (commit) group here:\n{rendered}"
    );
}
```

**The full emitted document, captured verbatim from a real run (a temporary
`eprintln!` was added to print `rendered`, the run below was captured, and
the print was then removed — the test's own assertions are the permanent
proof):**

```text
Reviewing: dc5be02

## a.txt:1 (+)

[issue] reviewed against the commit
```

`dc5be02` is `repo_with_history()`'s newest commit's real, `git`-resolved
short SHA (`active_commit.short_sha`, sourced from `CommitLogEntry`'s `%h`
format, which itself respects the user's `core.abbrev`) — not a fixture
string, proving the value flows end-to-end from a real `git log` invocation
through `open_commit_view` -> `App::annotation_source()` ->
`AnnotationStore::add_with_source` -> `render_markdown`.

**Command:**

```
cargo test --lib history_integration_tests::annotating_a_commit_view_records_a_reviewing_line_with_the_short_sha
```

**Result:**

```
running 1 test
test ui::history_integration_tests::annotating_a_commit_view_records_a_reviewing_line_with_the_short_sha ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 872 filtered out; finished in 0.20s
```

### A bug this proof caught before it shipped

The first version of this test also composed a working-tree annotation
*before* opening the commit, to additionally exercise the mixed-ordering
path against real git. It hung: `repo_with_history()`'s working tree is
clean, so the initial working-tree view has zero `Row::Line` rows, and the
loop advancing the cursor to a line row (`while
!matches!(...Row::Line(_)) { press(... 'j') }`) never terminated — pressing
`j` on an empty view never moves the cursor, so the condition never became
false. Killed the runaway `cargo test` processes, then fixed it two ways:
extracted a bounded `advance_to_line_row` helper (mirrors the
already-bounded inline pattern in
`commit_view_annotations_are_fully_functional`, capped at `rows.len() + 1`
iterations and returning early if the cursor stops advancing) for future
callers, and dropped the working-tree portion of *this* test since the
fixture genuinely has no working-tree diff to annotate — the ordering
guarantee it would have proven is already covered byte-exactly by the
mixed-session test above, which needs no real repository at all.

## Artifact: format contract documented in the module doc and README (task 4.3)

**What it proves:** The exact `Reviewing:` syntax is written down in two
places — the `annotate::markdown` module doc (the format's home) and the
README's public-facing format section — in present tense, matching exactly
what `render_group` emits (verified by the byte-exact tests above).

**`src/annotate/markdown.rs` module doc (added section), verbatim:**

```rust
//! ## The `Reviewing:` metadata line (non-working-tree sources)
//!
//! Annotations authored against the default working-tree source
//! ([`crate::annotate::model::Source::WorkingTree`]) are always emitted
//! first, in exactly the format above, with **no** metadata line — a
//! session that never leaves the working-tree view is byte-identical to the
//! format before this contract existed.
//!
//! Annotations authored against any other source are grouped by that source
//! (in order of first appearance, working-tree group excluded since it's
//! always first) and each group is preceded by exactly one metadata line of
//! the form `Reviewing: <spec>`, where `<spec>` is:
//!
//! - a commit: the short SHA (e.g. `Reviewing: abc1234`)
//! - a range: the range expression exactly as typed/selected (e.g.
//!   `Reviewing: main..feature`)
//! - the index: the literal word `staged` (e.g. `Reviewing: staged`)
```

**README.md diff excerpt (new "Annotation output format" section):**

```diff
+### Annotation output format (public API)
+
+Each annotation renders as a header naming its target, a blank line, then a
+`[classification] body` line (subsequent body lines follow unindented):
+
+```text
+## src/auth/session.rs:44 (+)
+
+[question] where does keystore get rotated?
+```
+
+Headers vary by target granularity — `path:line`, `path:start-end`, or just
+`path` for a whole-file comment — with a trailing `(+)`/`(-)` marking which
+side of the diff a line/range/hunk refers to (a hunk comment always shows
+`(+)`, since a hunk is anchored to its new-side span; a whole-file comment
+has no side marker at all). Multiple annotations are separated by
+exactly one blank line; the whole document ends with a single trailing
+newline.
+
+A session that never leaves the working-tree view renders exactly as above,
+byte-for-byte, with no extra lines — this is unchanged and will stay
+unchanged. Annotations made against any other diff target (a commit, an
+explicit range, or the staged index) are grouped after the working-tree
+annotations, each group preceded by exactly one metadata line:
+
+- a commit: `Reviewing: <short-sha>`
+- a range: `Reviewing: <range-as-typed>` (e.g. `Reviewing: main..feature`)
+- the staged index: `Reviewing: staged`
+
+so a script or agent consuming the output always knows which revision a
+group's line numbers resolve against.
+
+### Diff targets
+
+Any diff shown in the multibuffer — working tree (default), staged, an
+explicit range/ref, or a single commit opened from the git panel's History
+tab — can be annotated. Staging and code-intelligence keys are only ever
+shown and active for the working tree/staged targets they apply to; a
+read-only or historical target simply omits those keys from the footer and
+the `?` overlay rather than showing an inert one.
```

## Artifact: full-suite pass with test counts before/after this task

**What it proves:** Every new test is new (`annotate::model::tests`,
`annotate::store::tests`, `annotate::markdown::tests`,
`history_integration_tests::annotating_a_commit_view_records_a_reviewing_line_with_the_short_sha`);
no existing test's assertions were edited. Every `tests/*.rs` integration
binary is untouched.

**Command:**

```
cargo test 2>&1 | grep -E "Running|test result"
```

**Result summary — after this task's commit:**

```
Running unittests src/lib.rs (target/debug/deps/redquill-...)
test result: ok. 872 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
Running tests/git_integration.rs ...          test result: ok. 17 passed
Running tests/git_log_integration.rs ...      test result: ok. 4 passed
Running tests/git_remote_integration.rs ...   test result: ok. 4 passed
Running tests/git_stage_integration.rs ...    test result: ok. 10 passed
Running tests/git_worktree_integration.rs...  test result: ok. 6 passed
Running tests/lsp_integration.rs ...          test result: ok. 4 passed
```

872 lib unit tests = 858 (end of task 3.0, per `05-task-03-proofs.md`) + 14
new: 2 `annotate::model::tests::source_*`, 2
`annotate::store::tests::add_*_source*`, 9 new `annotate::markdown::tests`
(one explicit backward-compat test plus eight grouping tests), 1
`history_integration_tests::annotating_a_commit_view_records_a_reviewing_line_with_the_short_sha`.
All six `tests/*.rs` integration binaries are unchanged at 17, 4, 4, 10, 6,
and 4 respectively. `src/ui/perf_tests.rs` is untouched by this task
(`git diff --stat -- src/ui/perf_tests.rs` produces no output).

## Four gates

```
cargo build                              # Finished, no warnings
cargo test                               # 872 lib + 45 integration, all pass
cargo clippy --all-targets -- -D warnings  # Finished, no warnings
cargo fmt --check                        # no diff
```

## TTY-deferred proofs (operator)

This sandbox has no controlling TTY (`enable_raw_mode` fails with os error
6), so the task's CLI proof cannot be captured as a literal interactive
`redquill` session here. The real-git integration test above (driven
through the actual key-dispatch pipeline against a disposable repository)
is the equivalent evidence: every step it takes — focus panel, switch tab,
open commit, move cursor, compose, submit, render — is the same state
transition an interactive keypress would trigger. An operator with a real
terminal can reproduce the literal CLI proof:

1. In a git repo with at least one commit, run `cargo run --` from the repo
   root.
2. Press `` ` `` to focus the git panel, `Tab` to switch to the History
   tab.
3. Press `Enter` on a commit row to open it in the multibuffer.
4. Move to any changed line with `j`/`k`, press `c` to open Compose, type a
   comment (e.g. `reviewed against the commit`), press `Enter` to submit.
5. Press `q` to quit. The emitted stdout begins with `Reviewing:
   <short-sha>` (the short SHA of the commit opened in step 3), followed by
   a blank line, then the annotation in the existing per-annotation format
   — exactly the shape the automated proof above captured.
6. Pipe it to confirm the format is consumable: `cargo run -- | cat` (or
   pipe to an agent per the README's example) shows the same document on
   stdout.

## Reviewer Conclusion

All four proof artifacts required by the task file are satisfied: the
working-tree-only format is provably byte-identical (every pre-existing
`annotate::markdown` fixture passes unedited, plus an explicit test naming
the guarantee); the mixed-session grouping is byte-exact (working-tree
group first, no metadata line; each non-worktree group preceded by exactly
one correctly-spelled `Reviewing:` line) across all three non-worktree
source kinds and multiple ordering scenarios; the scripted CLI proof is
demonstrated end-to-end against a real disposable git repository (with the
literal interactive rehearsal recorded above as TTY-deferred, exact steps
included); and the format contract is documented in present tense in both
the `annotate::markdown` module doc and a new README section. `cargo
build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, and
`cargo fmt --check` all pass clean, with test counts growing from 858 to
872 lib tests (all six integration binaries unchanged) and zero existing
assertions edited.
