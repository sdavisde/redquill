# Task 01 Proofs - Side-by-side view retired

## Task Summary

This task removed the side-by-side rendering path entirely — `sbs_view.rs`, the `SbsRow` derivation, `ViewMode`/`toggle_view` state, the `t` keybinding, and all sbs parity tests — so the multibuffer work (tasks 2.0+) generalizes a single unified row model instead of two lockstep ones.

## What This Task Proves

- The codebase builds, tests, lints, and formats cleanly with the entire side-by-side path deleted (no dangling references).
- No orphaned side-by-side code remains anywhere in `src/` or the README.
- The `t` binding is retired from the public keymap contract (README table) and a regression test pins that `t` resolves to no action.

## Evidence Summary

- All four repository gates pass: build, 430 tests (405 unit + 25 integration), clippy `-D warnings`, fmt.
- `grep -riE "sbs|side.?by.?side|ToggleView" src/ README.md` exits 1 (zero hits).
- Net change: 12 files, +24/−1130 lines, including the 487-line `sbs_view.rs` deletion.

## Artifact: Four cargo gates green after the removal

**What it proves:** The deletion left no dangling references — the crate compiles, every remaining test passes, and lint/format gates hold.

**Why it matters:** This is the repo's blocking quality bar (CLAUDE.md + CI run the identical four commands).

**Command:**

~~~bash
cargo build && cargo test && cargo clippy -- -D warnings && cargo fmt --check
~~~

**Result summary:** All four gates green. Test output trimmed to the per-suite summary lines.

~~~text
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.06s
running 405 tests
test result: ok. 405 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.43s
running 11 tests
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.17s
running 10 tests
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.19s
running 4 tests
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.26s
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.31s
(cargo fmt --check: no output — clean)
~~~

## Artifact: No orphaned side-by-side code

**What it proves:** The rendering path is fully deleted, not merely unreachable.

**Why it matters:** Orphaned code would rot and confuse the multi-file builder work that follows in task 2.0.

**Command:**

~~~bash
grep -riE "sbs|side.?by.?side|ToggleView" src/ README.md; echo "grep exit: $?"
~~~

**Result summary:** Zero matches; grep exits 1.

~~~text
grep exit: 1
~~~

## Artifact: `t` retired from the public keymap

**What it proves:** The README keymap table (the public contract) no longer lists `t`, and the keymap test suite pins the retirement.

**Why it matters:** Every user-visible binding change must land in README.md's map per repo convention; the test prevents accidental resurrection.

**Command:**

~~~bash
git diff README.md   # excerpt
~~~

**Result summary:** The `t` row is removed from the keybinding table and feature/status prose now says unified-only. The regression test `t_resolves_to_no_action` (src/ui/keymap.rs) asserts `lookup(Char('t'))` is `None`.

~~~diff
-| `t` | Toggle side-by-side view |
-- Working-tree diff viewer: unified and side-by-side, syntax highlighting, word-level intra-line diff, file tree sidebar
+- Working-tree diff viewer: unified view, syntax highlighting, word-level intra-line diff, file tree sidebar
~~~

## Reviewer Conclusion

The side-by-side view is fully retired: one rendering path remains, all gates are green with 430 passing tests, the public keymap contract is updated, and a regression test pins `t` as unbound. The codebase is ready for the multi-file row model (task 2.0).
