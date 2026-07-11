# Task 01 Proofs - Side-by-side view retired

## Task Summary

Parent task 1.0 "Retire the side-by-side view" removed the entire side-by-side rendering path ahead of the multibuffer work: `src/ui/sbs_view.rs` (487 lines) is deleted; `SbsRow`, `build_sbs_rows`, and `SbsRow::source_rows` are gone from `src/ui/rows.rs`; `ViewMode`, `layout`, `toggle_view`, `sbs_rows`, `sbs_visual_of`, `sbs_scroll`, and the sbs branch of `ensure_visible` are gone from `src/ui/diff_view_state.rs`; the lockstep sbs rebuild is gone from `App::rebuild_rows`; `Action::ToggleView` and the `t` binding are retired from the keymap, help overlay, and README. Net change: 12 files, +24/−1130 lines.

## What This Task Proves

- The side-by-side code path is fully deleted, not orphaned: no source or README text matches `sbs`, `side-by-side`, or `ToggleView`.
- Nothing dangles: all four repository quality gates (build, test, clippy `-D warnings`, fmt) are green after the removal.
- The `t` binding is retired from the public keymap contract: the README keymap table no longer lists it, and a keymap test pins `t` to no action.

## Evidence Summary

| Check | Result |
| --- | --- |
| `cargo build` | pass |
| `cargo test` | pass — 430 tests (405 unit + 25 integration), 0 failed |
| `cargo clippy -- -D warnings` | pass |
| `cargo fmt --check` | pass |
| `grep -riE "sbs\|side.?by.?side\|ToggleView" src/ README.md` | zero hits, exit code 1 |
| README `t` row | removed |
| New regression test | `ui::keymap::tests::t_resolves_to_no_action` |

Test count moved from 455 (430 unit + 25 integration) before the removal to 430 (405 unit + 25 integration) after: 25 sbs-specific unit tests were deleted with the code they covered, and the retired `t_resolves_to_toggle_view` keymap test was replaced by `t_resolves_to_no_action`.

## Artifact: Four cargo gates green

**What it proves:** The removal left no dangling references — the whole crate compiles, every remaining test passes, clippy is warning-free under `-D warnings`, and formatting is canonical.

**Why it matters:** The sbs code was interwoven with the row model, view state, app wiring, staging tests, and code-intel tests; a green build+test run demonstrates the seams were cut cleanly rather than stubbed. These four commands are the repo's blocking quality bar (CLAUDE.md).

**Command:** `cargo build && cargo test && cargo clippy -- -D warnings && cargo fmt --check`

**Result summary:** All four gates exit 0. Test output trimmed to the `running`/`test result` summary lines (full per-test listing omitted; every test `ok`).

```
$ cargo build
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.09s
BUILD_EXIT=0

$ cargo test          # trimmed to "running"/"test result" lines
running 405 tests
test result: ok. 405 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.42s
running 0 tests
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
running 11 tests
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.21s
running 10 tests
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.22s
running 4 tests
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.29s
running 0 tests
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

$ cargo clippy -- -D warnings
    Checking redquill v0.2.0 (…/sdd-03-multibuffer-review)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.57s
CLIPPY_EXIT=0

$ cargo fmt --check
FMT_EXIT=0
```

## Artifact: No orphaned sbs/side-by-side/ToggleView references

**What it proves:** The code path is fully deleted — no identifier, comment, doc-string, or README sentence still refers to the side-by-side view or its toggle action anywhere in `src/` or `README.md`.

**Why it matters:** Orphaned references (a stale doc comment pointing at `super::sbs_view`, a leftover `ViewMode` re-export) would mislead the multi-file builder work in task 2.0 and rot immediately. Exit code 1 from grep is the machine-checkable "nothing left" signal.

**Command:** `grep -riE "sbs|side.?by.?side|ToggleView" src/ README.md; echo "GREP_EXIT=$?"`

**Result summary:** Zero matches; grep exits 1 (no lines selected).

```
$ grep -riE "sbs|side.?by.?side|ToggleView" src/ README.md
GREP_EXIT=1
```

## Artifact: README keymap `t` row removed and `t` asserted unbound

**What it proves:** The `t` binding is retired from the public keymap contract: the README table drops the row, the two "unified and side-by-side" prose mentions become unified-only, and a keymap unit test pins `t` to `None`.

**Why it matters:** README.md owns the keybinding map (repo convention: every user-visible action is in the map and the `?` overlay, and nothing else is). The test makes the retirement regression-proof — rebinding `t` by accident now fails `cargo test`.

**Command:** `git diff README.md`

**Result summary:** Three hunks: feature-list prose, the keymap table row, and the Status prose.

```diff
diff --git a/README.md b/README.md
index 3fa7626..d6ed8f3 100644
--- a/README.md
+++ b/README.md
@@ -18,7 +18,7 @@
 ## Core features
 
 **v1 — the review loop**
-- Working-tree diff viewer: unified and side-by-side, syntax highlighting, word-level intra-line diff, file tree sidebar
+- Working-tree diff viewer: unified view, syntax highlighting, word-level intra-line diff, file tree sidebar
@@ -45,7 +45,6 @@
 | `h` / `l`, `w` / `b` | Move / word-jump the column cursor (needed for `gd`/`gr`/`K`) |
 | `]` / `[` | Next / previous hunk |
 | `Tab` / `Shift-Tab` | Next / previous file |
-| `t` | Toggle side-by-side view |
 | `/` then `n` / `N` | Search |
@@ -120,4 +119,4 @@
 ## Status
 
-Pre-alpha, but the v1 review loop is implemented and usable via `cargo run --`: diff viewer (unified and side-by-side), annotations with markdown-on-quit, and file/hunk/line staging. …
+Pre-alpha, but the v1 review loop is implemented and usable via `cargo run --`: diff viewer (unified view), annotations with markdown-on-quit, and file/hunk/line staging. …
```

The new regression test, in `src/ui/keymap.rs`:

```rust
#[test]
fn t_resolves_to_no_action() {
    let km = Keymap::default_map();
    assert_eq!(km.lookup(key(KeyCode::Char('t'), KeyModifiers::NONE)), None);
}
```

## Reviewer Conclusion

The side-by-side view is fully retired: 1130 lines deleted across 11 source files plus README, all four cargo gates green, a machine-checked zero-reference grep, and a pinned test guaranteeing `t` is unbound. The row model, view state, and app wiring are now unified-only — exactly the halved surface the multi-file row builder (task 2.0) generalizes next. No behavior other than the `t` toggle was removed; annotation, staging, search, and LSP-peek coverage all still pass.
