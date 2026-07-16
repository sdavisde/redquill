# Task 5.0 Proofs — Config file remaps every modal panel (`[keys.staging]`, `[keys.switcher]`, ...)

## Task Summary

Implements Unit 4's modal-table half of spec 07
(`docs/specs/07-spec-config-layer/07-spec-config-layer.md`), completing what
task 4.0 deliberately left out of scope: every modal mode's key table
(`src/ui/modal_keys.rs`) is now remappable from `[keys.<mode>]` config, using
task 4's established grammar/merge-semantics machinery, in two separately
committed halves per the repo's refactor/behavior rule.

**Commit A (5.1, refactor, no behavior change)**: every `const` table in
`src/ui/modal_keys.rs` becomes a `static LazyLock<Vec<ModalBinding<A>>>`,
built once on first access instead of at compile time — the precondition for
layering config overrides onto them at all (a `const` can't be built from
runtime data). `ModalBinding::keys` moves from `&'static [ModalKey]` to an
owned `Vec<ModalKey>` (rvalue static promotion, what let the old arrays live
as `const` sub-expressions, only applies inside `const`/`static` item
initializers, not a `LazyLock::new` closure body). Move-only invariant
verified: identical test counts before/after (see below).

**Commit B (5.2–5.6, behavior)**:

- **5.2** Twelve canonical `[keys.<mode>]` table names — one per modal mode
  actually shipped (`list`, `staging`, `peek`, `switcher`, `help`,
  `help-search`, `compose`, `commit-message`, `search`, `finder`,
  `project-search-input`, `project-search-results`) — each with a bijective
  kebab-case action-name table and a `*_action_names_are_total_and_bijective`
  drift test (mirroring task 4.2's `action_names_are_total_and_bijective`).
- **5.3** `crate::ui::modal_keys_config::apply_modal_overrides` — a generic
  merge function reused across all twelve modes — applies task 4.4's
  replace/keep/unbind/collision semantics per mode. Free-text modes
  (Compose, the commit-message modal, Search, Finder, both Project Search
  focuses) were rewired from hand-written `match key.code` dispatch to
  resolve-first-then-insert: every documented control key is now a real,
  remappable per-mode action; an unresolved, unmodified `Char` still inserts
  literally (never an action, never bindable) — see `src/ui/modes.rs`.
- **5.4** `App` gained a `modal_keys: ModalKeymaps` field, built once in
  `ui::run` alongside the main effective keymap and threaded to every modal
  handler, the footer strip (`footer::build_hints`), and the `?` help
  overlay (`help::render`, via a new `HelpTables` bundle to stay under
  clippy's argument-count limit). `ModalBinding::key_label()` (new) computes
  a row's display text from its actual `keys` — mirroring
  `super::keymap::Binding::key_label` — so a remap's new key text reaches
  the footer/help overlay automatically, the same "derived, not stored"
  design the main keymap already used.
- **5.5** `docs/example-config.toml` gained a complete `[keys.<mode>]`
  section per mode (commented out, like `[keys.diff]`/`[keys.panel]`'s
  precedent — a copy-verbatim example shouldn't silently impose a
  nonstandard keymap), generated from the real tables (not hand-transcribed)
  and cross-checked by a new persisted test,
  `example_config_documents_every_modal_action_exactly_once`, which parses
  the doc's commented action lines back out and asserts every mode's
  complete action set is documented exactly once with parseable key
  strings — closing the gap the existing zero-warnings drift test (task 1.9)
  can't see (commented-out sections aren't live TOML).
- **5.6** User demo: **skipped by user decision** (2026-07-16, mid-task
  scope change) — see the note at the end of this document. The automated
  test suite stands as the proof in its place.

## A Necessary Correctness Fix Discovered Along the Way

Making free-text modes' control keys resolve through the shared `resolve()`
primitive exposed a latent bug in `ModalKey::matches`, which historically
compared key **code only** (ignoring modifiers) because no default modal
table depended on modifier discrimination for real dispatch (the
action-based tables never used modifier-chorded keys; the free-text tables'
`resolve()` was only ever exercised by drift-test cross-checks, never real
dispatch). Once free-text dispatch started resolving through the table for
real, code-only matching became unsafe: e.g. a user remapping `cancel` to
`ctrl-q` would have made a bare, unmodified `q` keystroke also resolve to
`Cancel` (wrongly stealing it from literal-text insertion). `ModalKey::matches`
now compares code **and** modifiers (with the same shift-strip-for-uppercase
special case `KeyChord::matches` already uses), which is what makes
Compose's own default table (`Enter` submits, `Shift-Enter` inserts a
newline, `Ctrl-j` also inserts a newline) resolve correctly through one
shared `resolve()` call instead of the old hand-written `if shift`/`if ctrl`
guards. This surfaced two pre-existing test gaps, both fixed as part of this
commit (not pushed to a follow-up, since they're direct fallout of the
matches-comparison fix within this same task):

- `footer::tests::switcher_mode_hints` — the switcher's `ToggleTab` label
  changed from the old hand-curated `"Tab / h / l"` to the fully-computed
  `"Tab / Shift-Tab / h / l / Left / Right"` (every bound key now genuinely
  shown, per the FR that hint text must reflect the real bindings).
- `handle_project_search_key`'s Results-focus `j`/`k`/`/` arms gained
  explicit `&& !alt` guards — the table always claimed only the bare keys,
  but the hand-written dispatch didn't check `alt` for those three, so
  `Alt-j` used to silently move the selection (an actual latent bug the
  now-honest table exposed via the reverse-drift test).

The help overlay's box-width cap also grew from 92 to 130 columns
(`src/ui/help.rs`): computed key labels can legitimately be longer than the
old hand-curated shorthand (a row with several alternate key encodings for
one action, like `delete-word-back`'s four), and 92 was already tight
against the widest existing description text; 130 comfortably fits the
current worst case with margin, still clamped to the terminal's actual width
on narrower screens.

## 5.1 Refactor — Identical Test Count Invariant

Verified by stashing the refactor diff and re-running the full suite against
unchanged `HEAD`, then popping the stash and re-running:

| | lib | main | git_integration | git_log | git_ls_files | git_remote | git_stage | git_worktree | lsp | doctests | **total** |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Before (HEAD) | 1249 (3 ignored) | 9 | 17 | 4 | 3 | 4 | 10 | 6 | 4 | 0 | **1306** |
| After (5.1 diff) | 1249 (3 ignored) | 9 | 17 | 4 | 3 | 4 | 10 | 6 | 4 | 0 | **1306** |

Byte-identical counts, zero assertion edits — a pure move (const arrays to
lazily-built statics) with the runtime behavior unchanged. Every modal drift
test (the bidirectional "every documented key drives its action" /
"every undocumented key does nothing" pairs already established by earlier
specs) passed unchanged before and after.

## What Commit B Proves

- **Bijectivity (5.2)**: `cargo test bijective` — 13 tests (the pre-existing
  main-keymap one plus 12 new modal ones, one per mode) all green: every
  action that appears in a mode's default table gets exactly one kebab-case
  name, no two actions share a name, and every name resolves back to the
  same action.
- **Merge semantics (5.3)**: `src/ui/modal_keys_config_tests.rs` — TDD
  coverage for replace-not-append, keep-unlisted, empty-array-unbinds,
  same-table collision (override wins, one warning), multi-key overrides
  staying one row (not duplicated per key — matching how `ToggleTab`'s six
  default keys already render as one row), unknown action name (invalid
  value, table untouched), a two-chord key string rejected (modal tables
  never supported `gd`-style sequences), mode isolation (an override in one
  mode never leaks into another), and a free-text mode
  (`compose`) proving a control-action override coexists with every other
  action untouched. Plus a cross-check
  (`modal_mode_names_match_config_keys_hardcoded_list`) that the ui-side
  `MODAL_MODE_NAMES` list and `crate::config::keys`'s independently
  hardcoded list (required by the layering rule: config must never import
  ui) name exactly the same twelve modes.
- **Reverse drift still holds with overrides applied (5.3)**: the
  pre-existing bidirectional drift suite (`every_*_hint_key_is_consumed_by_the_handler`
  / `*_ignores_control_keys_absent_from_its_table`, one pair per free-text
  mode) passed unchanged through the entire rewrite, since every mode's
  *default* dispatch behavior is provably identical to before (see the
  correctness-fix section above for the two real behavior changes the
  fix required, both now pinned by tests).
- **Hint/help truthfulness (5.4)**: one pattern-level test each —
  `footer::tests::switcher_mode_hints`/`compose_mode_hints_are_just_save_and_discard`
  (etc.) pin that `footer::modal_hints` derives its key text from the
  effective table's `key_label()`, and the pre-existing
  `help_overlay_lists_remote_and_command_log_bindings` /
  `help_overlay_scrolls_to_reveal_lower_sections` continue to pass against
  the wider box. No additional per-mode test was needed for "hint lines
  reflect effective keys" beyond what already exercises `key_label()`,
  because both the footer and the help overlay share the exact same
  `ModalBinding::key_label()` computation — one function, one place it could
  drift, already covered.
- **Docs cannot silently rot (5.5)**: `example_config_documents_every_modal_action_exactly_once`
  — parses `docs/example-config.toml`'s commented `[keys.<mode>]` blocks and
  asserts, per mode: every documented action name is real, every documented
  key string parses under the grammar, and the mode's complete action set is
  present exactly once (no omissions, no invented names). Verified to
  actually catch drift (not just vacuously pass) by temporarily corrupting
  one action name during development and confirming the test failed with a
  precise message, then reverting.
- **Perf tripwires unchanged**: all 5 tests in `ui::app::perf_tests` pass
  with unchanged budgets — the effective-modal-keys build happens once per
  session in `ui::run`, not on any measured hot path.

## Evidence Summary

| Artifact | Proves |
| --- | --- |
| `proofs/5-gates-commit-a-refactor.txt` | Commit A's full four-gate transcript (build/test/clippy/fmt), all green, plus the identical-test-count numbers reproduced above |
| `proofs/5-gates-commit-b-behavior.txt` | Commit B's full four-gate transcript (1330 total tests passed across all binaries: 1273 lib + 9 main + 17/4/3/4/10/6/4 integration + 0 doctests, 3 ignored, 0 failed), clippy clean, fmt clean |
| `cargo test bijective` (13 passed) | Every mode's action-name table is total and bijective (5.2) |
| `cargo test --lib modal_keys_config` (11 passed) | Merge-semantics coverage: replace/keep/unbind/collision/multi-key-one-row/unknown-name/two-chord-rejected/mode-isolation/free-text-coexistence, plus the mode-name cross-check (5.3) |
| `cargo test --lib example_config_documents_every_modal_action_exactly_once` | The shipped example config documents every modal action exactly once with parseable keys (5.5) |
| `cargo test --lib ui::app::perf_tests::` (5 passed) | Perf tripwires unaffected |

Both commits' gate transcripts and this markdown are committed; raw
`.txt` captures live under `docs/specs/07-spec-config-layer/proofs/`, which
is gitignored (the pre-existing `docs/specs/*/proofs/` rule).

## User Demo (5.6) — Skipped By User Decision

The task file's User demo for 5.0 calls for a live tmux session remapping a
staging-panel key and a switcher key, confirming the new keys act, the old
ones don't, and the hint line/`?` overlay show the remap, alongside a
still-working task-4 main-keymap remap. **This capture was explicitly
skipped mid-task per an operator scope change** ("SKIP the live tmux
User-demo capture for sub-task 5.6 ... Rely on the automated test suite as
the proof of behavior"), received before any tmux session was started for
this task — no `proofs/5-modal-remap.png`/`.txt` exists for this slice.

The automated coverage above exercises the identical contract the demo
would have shown interactively:
`overriding_a_staging_action_replaces_its_default_keys_rather_than_appending`
and `colliding_override_wins_and_drops_the_default_with_a_warning` (driving
`effective_modal_keys` directly, the same function `ui::run` calls at
startup) prove a staging remap takes effect and the old key stops working;
the switcher's bijectivity/merge tests cover the equivalent for
`[keys.switcher]`; and `footer::tests::switcher_mode_hints` plus the help
overlay's rendering tests prove the hint line and `?` overlay derive from
the same effective table a remap would change. What the automated suite
cannot show is the *visual* end-to-end proof (a real terminal frame with the
remap visibly active) — that gap is accepted per the operator's explicit
instruction, not an oversight.

## Deviations From the Task File (flagged for reviewer awareness)

- **5.6's screenshot proof is missing**, per the operator scope change
  described above. Flagged, not silently dropped.
- **Scope growth, required by the resolve-first rewrite (not optional)**:
  making free-text modes' control keys genuinely remappable required (a)
  splitting historically-merged multi-behavior hint rows into one row per
  actual behavior (e.g. Compose's single "Move cursor" row covering four
  keys is now four rows — `MoveLeft`/`MoveRight`/`MoveUp`/`MoveDown` — since
  a single action can't be independently remapped per direction), (b) fixing
  `ModalKey::matches` to compare modifiers (see the correctness-fix section
  above, with its two consequent test fixes), and (c) widening the help
  overlay's box-width cap from 92 to 130. All three are necessary
  consequences of the FR itself ("every documented control key is
  independently remappable," "hint lines reflect effective keys"), not
  speculative extras — flagged here per the instruction to call out scope
  growth rather than let it pass unremarked.
- **`KeysConfig::from_value`'s visibility widened** from `pub(super)` to
  `pub(crate)` so `crate::ui::modal_keys_config`'s tests can drive it
  directly for the mode-name cross-check — the same visibility
  `crate::ui::keymap_config::effective_keymap` already uses for the
  analogous main-keymap purpose, so this follows existing precedent rather
  than introducing a new one.

## Reviewer Conclusion

Unit 4's modal-table contract is implemented and proven: twelve modal modes
each have a bijective action-name table, a config-driven merge onto their
compiled-in defaults (replace/keep/unbind/collision, identical semantics to
the main keymap), and effective tables built once at startup and threaded
through every handler, the footer strip, and the help overlay — with hint
text derived from the actual bound keys rather than stored strings, so a
remap is automatically truthful everywhere it's shown. Free-text modes keep
character insertion permanently non-remappable while making every other
documented key a real action. The refactor/behavior split is clean (two
commits, identical test counts proven for the refactor), the shipped example
config is drift-tested for completeness and correctness, and all four repo
gates are green on the final tree. The one gap — 5.6's live visual demo — is
an explicit, operator-directed scope reduction, not an implementation
shortfall.

Parent task 5.0 (modal-table remapping) is ready to be marked complete.
