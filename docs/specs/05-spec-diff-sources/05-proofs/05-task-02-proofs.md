# Task 02 Proofs - Single-commit diff target and commit-log read model (git layer)

## Task Summary

Parent task 2.0 ("Single-commit diff target and commit-log read model", spec
05 Unit 2) extends the git layer (no UI types) with two new capabilities:

1. **`DiffTarget::Commit(String)`** — a single commit's own changes,
   computed as `git diff <rev>^ <rev>` (first-parent semantics, so a merge
   commit shows only what the merge itself introduced relative to
   mainline), falling back to git's well-known empty-tree object as the
   base when `<rev>^` doesn't resolve (a root commit has no parent, so its
   diff is "everything, added"). The new variant inherits the task 1.0
   capability model by construction: `is_live() == false`,
   `staging_mode() == ReadOnly`, `supports_code_intel() == false` — the
   compiler forced this decision at every one of the three exhaustive
   matches in `src/git/diff.rs` the moment the variant was added.
2. **`src/git/log.rs`** — a commit-log read model (`CommitLogEntry`: full
   SHA, short SHA, subject, author name, Unix timestamp) parsed from a
   NUL-delimited `git log --format=<COMMIT_LOG_FORMAT>` payload, with
   count/skip pagination exposed as `GitRunner::commit_log`.

`src/ui/syntax.rs`'s `content_source` seam also gained `Commit` arms so
historical file content for syntax highlighting always comes from git
objects (`<rev>^:<path>` / `<rev>:<path>`) and never the on-disk working
tree.

Landed as one commit on `worktree-diff-sources` (this proof file's commit):
`feat(git): add DiffTarget::Commit and the commit-log read model`.

## What This Task Proves

- The `Commit` variant's capability triple is exhaustive and matches the
  spec's contract (read-only, not live, no code-intel) — proven at the unit
  level in `src/git/diff.rs`, the same table-per-variant pattern task 1.0
  established.
- `GitRunner::diff` builds the commit-diff argv from discrete, owned argv
  elements (no string interpolation into a shell) and correctly falls back
  to the empty tree for a root commit, verified end-to-end in a throwaway
  tempdir repo for all three commit shapes the spec calls out: a normal
  commit, a merge commit (first-parent diff), and a root commit
  (all-added).
- The NUL-delimited `git log` parser (`src/git/log.rs`) is robust against
  hostile subjects (colons, internal whitespace, embedded quote
  characters) and degrades to an empty list — not an error — for an empty
  repository.
- `GitRunner::commit_log`'s count/skip pagination produces stable,
  non-overlapping pages in a real tempdir repo with real history.
- `content_source`'s new `Commit` arms resolve to `<rev>:<path>` (new side)
  and `<rev>^:<path>` (old side), including the rename case (old side
  prefers `old_path`), and the root-commit "old side doesn't resolve" case
  degrades gracefully through the existing `show_file`-returns-`None`
  contract — no special-casing needed in the pure `content_source`
  function.

## Evidence Summary

| Proof | Result |
| --- | --- |
| `Commit` capability-triple unit test | 1/1 pass (`git::diff::tests::commit_capability_triple`) |
| NUL-delimited log parser unit tests (incl. hostile subjects, empty repo) | 8/8 pass (`git::log::tests::*`) |
| `content_source` `Commit`-arm unit tests | 5/5 pass (`ui::syntax::tests::*commit*`, incl. root-commit fallback) |
| Tempdir integration: normal/merge/root commit-shape cases | 3/3 pass (`tests/git_integration.rs::commit_target_*`) |
| Tempdir integration: log pagination (two pages, stable order, empty-repo, past-end) | 4/4 pass (`tests/git_log_integration.rs`) |
| Full lib unit suite before/after this task | 810 passed → 824 passed (+14 new tests, 0 removed/edited existing assertions) |
| Full integration suite before/after this task | 38 passed across 5 binaries → 45 passed across 6 binaries (+3 in `git_integration`, +4 new `git_log_integration` binary) |
| Four gates (build/test/clippy --all-targets/fmt) | All pass |

## Artifact: `Commit` capability-triple unit test

**What it proves:** The new `DiffTarget::Commit` variant reports the exact
capability triple the spec requires (`is_live() == false`,
`staging_mode() == ReadOnly`, `supports_code_intel() == false`), and — because
`is_live`, `staging_mode`, and `supports_code_intel` are each an exhaustive
match with no wildcard arm — the compiler refused to build until this
decision was made explicitly for the new variant at all three call sites.

**Why it matters:** This is what makes the capability model in task 1.0
actually pay off for a new source: every future `DiffTarget` variant is
forced through the same three-question gate rather than silently inheriting
a default (e.g. a forgotten wildcard `_ => false` that happens to be right
today and wrong tomorrow).

**Command:**

```
cargo test --lib commit_capability_triple
```

**Result summary:**

```
running 1 test
test git::diff::tests::commit_capability_triple ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 825 filtered out; finished in 0.00s
```

The test (`src/git/diff.rs`), verbatim:

```rust
#[test]
fn commit_capability_triple() {
    let target = DiffTarget::Commit("HEAD".to_string());
    assert!(!target.is_live());
    assert_eq!(target.staging_mode(), StagingMode::ReadOnly);
    assert!(!target.supports_code_intel());
}
```

## Artifact: NUL-delimited commit-log parser tests, including hostile subjects and an empty repo

**What it proves:** `parse_commit_log` (`src/git/log.rs`) correctly splits a
`git log --format=<COMMIT_LOG_FORMAT>` record on `\0` regardless of what a
hostile subject line contains — a colon (a plausible "field separator"
red herring for a human-format parser), internal multi-space runs, and
embedded double/single quote characters — because the parser only ever
looks for the literal NUL byte, which git text output never contains. It also
proves an empty repository's `git log` output (empty string) yields an empty
list rather than a parse error, and that malformed records (missing fields,
non-numeric timestamp) do error, so a truncated/corrupted payload is never
silently misread.

**Why it matters:** This is the task's explicit acceptance requirement
(2.0 proof artifacts: "parser unit tests ... including subjects containing
`:`/whitespace/quote characters" and the log read model's TDD-first
requirement in 2.3) — the exact class of format-parsing bug the
`rust-best-practices.md` "parse machine-readable output ... never scrape
human-readable output" rule exists to prevent.

**Command:**

```
cargo test --lib git::log::tests
```

**Result summary:**

```
running 8 tests
test git::log::tests::empty_repo_output_yields_no_entries ... ok
test git::log::tests::missing_fields_errors ... ok
test git::log::tests::invalid_timestamp_errors ... ok
test git::log::tests::parses_multiple_records_preserving_order ... ok
test git::log::tests::subject_containing_quote_characters_is_preserved ... ok
test git::log::tests::subject_containing_internal_whitespace_is_preserved ... ok
test git::log::tests::parses_a_single_record ... ok
test git::log::tests::subject_containing_a_colon_is_preserved ... ok

test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 818 filtered out; finished in 0.00s
```

The hostile-subject tests (`src/git/log.rs`), verbatim:

```rust
#[test]
fn subject_containing_a_colon_is_preserved() {
    let input = "sha\0sha\0feat: add async: remote ops\0A\x001700000000\n";
    let entries = parse_commit_log(input).unwrap();
    assert_eq!(entries[0].subject, "feat: add async: remote ops");
}

#[test]
fn subject_containing_internal_whitespace_is_preserved() {
    let input = "sha\0sha\0fix   weird   spacing\0A\x001700000000\n";
    let entries = parse_commit_log(input).unwrap();
    assert_eq!(entries[0].subject, "fix   weird   spacing");
}

#[test]
fn subject_containing_quote_characters_is_preserved() {
    let input = "sha\0sha\0fix: handle \"quoted\" and 'single' text\0A\x001700000000\n";
    let entries = parse_commit_log(input).unwrap();
    assert_eq!(
        entries[0].subject,
        "fix: handle \"quoted\" and 'single' text"
    );
}

#[test]
fn empty_repo_output_yields_no_entries() {
    assert!(parse_commit_log("").unwrap().is_empty());
}
```

## Artifact: tempdir integration tests for the three commit-shape cases (normal, merge, root)

**What it proves:** `GitRunner::diff(&DiffTarget::Commit(rev))` against a
real, throwaway git repository (never the host repo) produces exactly the
diff semantics the spec requires for each shape:

- **Normal commit**: only that commit's own changes appear — a later,
  unrelated third commit's edit is absent, and the *first* commit's
  original content is absent too (the diff is bounded to `<rev>^..<rev>`,
  not `<root>..<rev>`).
- **Merge commit**: the diff is against the *first* parent (the mainline
  commit), not the pre-branch-point ancestor — mainline's own prior change
  is correctly excluded (it's already reflected in the first parent and
  cancels out), and only the feature branch's incoming file shows up.
- **Root commit**: `git rev-parse --verify <sha>^` is independently
  confirmed to fail (no parent exists), and the resulting diff shows every
  file in the commit as `new file mode` — the empty-tree fallback exercised
  end-to-end, not just asserted in isolation.

**Why it matters:** This is the task's explicit 2.0/2.5 acceptance
requirement — diff semantics only really prove out against a real git
process on a real, disposable repository; a unit test on argv construction
alone couldn't catch a wrong revision spec or a git flag that changes
first-parent behavior.

**Command:**

```
cargo test --test git_integration commit_target
```

**Result summary:**

```
running 3 tests
test commit_target_root_commit_is_an_all_added_diff_against_the_empty_tree ... ok
test commit_target_normal_commit_shows_only_that_commits_own_changes ... ok
test commit_target_merge_commit_diffs_against_first_parent ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 14 filtered out; finished in 0.15s
```

The merge-commit test (`tests/git_integration.rs`), verbatim (the case most
prone to getting first-parent semantics backwards):

```rust
#[test]
fn commit_target_merge_commit_diffs_against_first_parent() {
    let tmp = init_repo();
    let dir = tmp.path();
    git(dir, &["branch", "-M", "main"]);

    // A commit on main after the branch point, so the merge has real
    // first-parent history to exclude.
    write(dir, "base.txt", b"line one\nmainline change\n");
    git(dir, &["commit", "-aqm", "mainline commit"]);

    // A feature branch off "initial" with its own, non-conflicting change.
    git(dir, &["branch", "feature", "HEAD~1"]);
    git(dir, &["checkout", "-q", "feature"]);
    write(dir, "feature.txt", b"feature content\n");
    git(dir, &["add", "feature.txt"]);
    git(dir, &["commit", "-qm", "feature commit"]);

    // Merge feature into main with an explicit merge commit (no
    // fast-forward), so HEAD has two parents.
    git(dir, &["checkout", "-q", "main"]);
    git(
        dir,
        &["merge", "-q", "--no-ff", "-m", "merge feature", "feature"],
    );
    let merge_sha = git_out(dir, &["rev-parse", "HEAD"]);

    let runner = runner_for(&tmp);
    let patches = runner.diff(&DiffTarget::Commit(merge_sha)).unwrap();

    // First-parent diff (<merge>^ == mainline commit, not the pre-branch
    // "initial"): only feature.txt's arrival shows up, since mainline's own
    // change is already reflected in the first parent and cancels out.
    assert_eq!(patches.len(), 1);
    assert_eq!(patches[0].path, "feature.txt");
    assert!(patches[0].raw.contains("+feature content"));
}
```

## Artifact: tempdir integration tests for commit-log pagination

**What it proves:** `GitRunner::commit_log(count, skip)` against a real
5-commit repo produces newest-first ordering, two pages that don't overlap,
identical results on a repeated call for the same page (stable — no
accidental non-determinism from process-level ordering), and an empty page
past the end of history — plus an empty list (not an error) for a
repository with zero commits.

**Why it matters:** This is the task's explicit 2.0/2.5 pagination
acceptance requirement; the History tab (task 3.0, not in this task's
scope) depends on paging never re-fetching or dropping a commit as the user
scrolls, which only a real multi-commit repo can verify.

**Command:**

```
cargo test --test git_log_integration
```

**Result summary:**

```
running 4 tests
test commit_log_on_an_empty_repo_yields_no_entries ... ok
test commit_log_skip_past_the_end_yields_an_empty_final_page ... ok
test commit_log_first_page_is_newest_first ... ok
test commit_log_pagination_yields_two_non_overlapping_pages_in_stable_order ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.15s
```

The pagination test (`tests/git_log_integration.rs`), verbatim:

```rust
#[test]
fn commit_log_pagination_yields_two_non_overlapping_pages_in_stable_order() {
    let tmp = init_bare_repo();
    commit_n_times(tmp.path(), 5);

    let runner = runner_for(&tmp);
    let page1 = runner.commit_log(2, 0).unwrap();
    let page2 = runner.commit_log(2, 2).unwrap();

    assert_eq!(page1.len(), 2);
    assert_eq!(page2.len(), 2);
    assert_eq!(page1[0].subject, "commit 4");
    assert_eq!(page1[1].subject, "commit 3");
    assert_eq!(page2[0].subject, "commit 2");
    assert_eq!(page2[1].subject, "commit 1");

    // No overlap between pages.
    let page1_shas: Vec<&str> = page1.iter().map(|c| c.sha.as_str()).collect();
    assert!(page2.iter().all(|c| !page1_shas.contains(&c.sha.as_str())));

    // Requesting the same pages again is stable (no history changed).
    let page1_again = runner.commit_log(2, 0).unwrap();
    assert_eq!(page1, page1_again);
}
```

## Artifact: `content_source` `Commit`-arm tests — `<rev>^:<path>` / `<rev>:<path>` resolution

**What it proves:** The pure `content_source` function (`src/ui/syntax.rs`)
maps a `Commit(rev)` target's new side to `<rev>:<path>` and its old side to
`<rev>^:<path>` (preferring `old_path` for a rename, matching every other
target's old-side rename behavior), and that the root-commit case — where
`<rev>^:<path>` never resolves because there is no parent — degrades to
`None` through the existing `show_file`-returns-`None`/"unresolvable spec"
contract rather than requiring a root-commit special case inside this pure,
I/O-free function.

**Why it matters:** This is the task's explicit 2.0/2.2 acceptance
requirement ("historical content never reads the working tree"); it also
demonstrates the layering discipline the repo's conventions require — the
git-object resolution logic doesn't need to know whether a commit is a root
commit, because "an unresolvable git spec" is already a first-class,
gracefully-handled outcome one layer up.

**Command:**

```
cargo test --lib ui::syntax::tests -- commit
```

**Result summary:**

```
test ui::syntax::tests::new_side_commit_ignores_old_path_even_for_renames ... ok
test ui::syntax::tests::new_side_commit_is_rev_colon_path ... ok
test ui::syntax::tests::old_side_commit_is_rev_caret_colon_path ... ok
test ui::syntax::tests::old_side_commit_prefers_old_path_for_renames ... ok
test ui::syntax::tests::root_commit_old_side_degrades_to_no_content_not_a_panic ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; ... filtered out
```

The formatting tests plus the root-commit degradation test
(`src/ui/syntax.rs`), verbatim:

```rust
#[test]
fn new_side_commit_is_rev_colon_path() {
    assert_eq!(
        content_source(
            &DiffTarget::Commit("abc123".to_string()),
            Side::New,
            "a.rs",
            None
        ),
        ContentSource::Show("abc123:a.rs".to_string())
    );
}

#[test]
fn old_side_commit_is_rev_caret_colon_path() {
    assert_eq!(
        content_source(
            &DiffTarget::Commit("abc123".to_string()),
            Side::Old,
            "a.rs",
            None
        ),
        ContentSource::Show("abc123^:a.rs".to_string())
    );
}

#[test]
fn root_commit_old_side_degrades_to_no_content_not_a_panic() {
    // A root commit has no parent, so `<rev>^:<path>` never resolves.
    // fetch_content must degrade to `None` (fall through to
    // `show_file`'s own "unresolvable spec" contract) rather than the
    // git layer needing any root-commit special case.
    struct RootCommitOps;
    impl StageOps for RootCommitOps {
        // ... (canned StageOps impl; see src/ui/syntax.rs)
        fn show_file(&self, spec: &str) -> Option<String> {
            // `<rev>^:<path>` never resolves for a root commit; every
            // other spec would.
            if spec.contains('^') {
                None
            } else {
                Some("fn main() {}\n".to_string())
            }
        }
    }

    let ops = RootCommitOps;
    let target = DiffTarget::Commit("root".to_string());
    assert_eq!(fetch_content(&ops, &target, "a.rs", None, Side::Old), None);
    assert_eq!(
        fetch_content(&ops, &target, "a.rs", None, Side::New),
        Some("fn main() {}\n".to_string())
    );
}
```

## Artifact: full-suite pass with test counts before/after this task

**What it proves:** This task is purely additive at the test level — every
new test is new (a `Commit` variant, a new `log.rs` module, new
`content_source` arms, two new integration test groups), and nothing in the
existing suite was touched, edited, or removed.

**Command:**

```
cargo test 2>&1 | grep -E "Running|test result"
```

**Result summary — after this task's commit:**

```
Running unittests src/lib.rs (target/debug/deps/redquill-0c548af423561ef3)
test result: ok. 824 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
Running tests/git_integration.rs ...          test result: ok. 17 passed
Running tests/git_log_integration.rs ...      test result: ok. 4 passed
Running tests/git_remote_integration.rs ...   test result: ok. 4 passed
Running tests/git_stage_integration.rs ...    test result: ok. 10 passed
Running tests/git_worktree_integration.rs...  test result: ok. 6 passed
Running tests/lsp_integration.rs ...          test result: ok. 4 passed
```

824 lib unit tests = 810 (end of task 1.0, per `05-task-01-proofs.md`) + 14
new (1 `commit_capability_triple` + 8 `git::log::tests::*` + 5
`ui::syntax::tests::*commit*`/root-commit fallback). `git_integration.rs`
grew from 14 to 17 (+3 commit-shape tests); `git_log_integration.rs` is a
new binary (4 tests). `git_remote_integration.rs`, `git_stage_integration.rs`,
`git_worktree_integration.rs`, and `lsp_integration.rs` are unchanged at 4,
10, 6, and 4 respectively.

## Reviewer Conclusion

All four proof artifacts required by the task file are satisfied: the
NUL-delimited log parser handles hostile subjects (colons, internal
whitespace, quotes) and an empty repo correctly; tempdir integration tests
cover all three commit-shape cases (normal, merge-vs-first-parent, root-vs
-empty-tree) and log pagination (two stable, non-overlapping pages) against
real disposable repositories; `content_source`'s `Commit` arms resolve to
`<rev>^:<path>` / `<rev>:<path>` and degrade gracefully for a root commit's
unresolvable parent spec; and the `Commit` variant's capability triple
matches the spec's read-only/non-live/no-code-intel contract, enforced by
the compiler via the task 1.0 exhaustive-match model. `cargo build`,
`cargo test`, `cargo clippy --all-targets -- -D warnings`, and `cargo fmt
--check` all pass clean.
