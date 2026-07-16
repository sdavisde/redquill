# Task 4.0 Proofs — Config file remaps the main keymap (`[keys.diff]`, `[keys.panel]`)

## Task Summary

Implements Unit 4's main-keymap half of spec 07
(`docs/specs/07-spec-config-layer/07-spec-config-layer.md`): a kebab-case
action-name mapping over every `Action` variant with a bijectivity
guarantee, a hand-rolled key-string grammar (crokey-style chord notation:
single chords and space-separated two-chord sequences), per-action merge
semantics onto `Keymap::default_map()` (replace/keep/unbind/collision), and
`[keys.diff]`/`[keys.panel]` deserialization wired into `Config`. The
effective keymap is built exactly once at startup and threaded by
reference through the whole session — no per-keystroke parsing. The
`?` help overlay and footer strip render the effective bindings with no
additional wiring (they already derive from `Keymap::bindings()`); a task
4.6 audit found three *other* places in `src/ui/` that hardcoded a
main-keymap key literal in prose (outside the render path) and fixed all
three to resolve dynamically instead, closing the same class of drift the
task exists to prevent. The modal tables (`src/ui/modal_keys.rs`) are
explicitly out of scope — that's task 5.0.

## Crokey vs. Hand-Rolled: Decision (task 4.1)

**Decision: hand-rolled parser. No new dependency added.**

`cargo add crokey --dry-run` and a scratch `cargo tree` (see Method below)
showed `crokey` pulls in `crokey-proc_macros` (a `proc-macro` crate), which
transitively pulls a **second, independent copy of `crossterm 0.29.0`**
(this crate already depends on `crossterm 0.29.0` directly) plus the full
`syn`/`quote`/`proc-macro2` proc-macro toolchain, `parking_lot`, `mio`,
`signal-hook`, `derive_more`, and more — a large addition for a single
static binary whose CLAUDE.md explicitly says "don't add dependencies
casually" and requires justification for anything beyond the existing
stack.

The grammar this spec actually needs is small: single chords
(`"a"`, `"ctrl-k"`, `"alt-enter"`, `"shift-tab"`, `"f5"`, `"esc"`) plus our
own two-chord sequence splitting on top (`crokey` only parses one
combination at a time regardless — the spec's own Technical Considerations
section says as much: "two-key sequences are our layer on top either
way"). The task brief's own guidance — "if a hand-rolled parser is small
(~100-200 lines) and covers the grammar, prefer it" — applies directly:
the implemented parser (`src/config/keys.rs`'s `parse_chord`/
`parse_key_string`) is ~90 lines including doc comments, covers every
example in the spec, and needs zero glue beyond one bridging function
(`KeySeq::from_spec`) to reach the runtime `KeyChord`/`KeySeq`
representation — glue that would be needed either way, since `crokey`'s
own `KeyCombination` type isn't `crate::ui::keymap::KeyChord` and would
still need converting at the boundary.

**Acceptance**: the chosen path (hand-rolled) compiles with a round-trip
test — `default_binding_key_labels_round_trip_through_the_config_grammar`
in `src/ui/keymap.rs`'s test module feeds every default binding's
`KeyChord::label()` output back through `crate::config::keys::parse_key_string`
and asserts it resolves to the identical chord, for every single-key and
decomposed two-key binding in the table. This is the "chosen path compiles
with a round-trip test" acceptance criterion from the task file, and it
doubles as task 4.3's grammar/label consistency test.

No dependency commit was needed since no dependency was added — `Cargo.toml`
is unchanged by this task.

## What This Task Proves

- **Action-name mapping (4.2)**: `action_name`/`action_from_name` in
  `src/ui/keymap.rs` are a bijective kebab-case mapping over every `Action`
  variant. `action_name` is an *exhaustive match* — the compiler refuses to
  build the crate if a new `Action` variant is added without a name,
  stronger than a test. The runtime half
  (`action_names_are_total_and_bijective`) proves no two actions share a
  name and every name resolves back to the same action, iterating
  `Keymap::default_map().bindings()` — the same "every user-visible action
  is reachable from the keymap" convention `help.rs`'s own `group_of` drift
  test already relies on (this repo carries no enum-reflection derive
  crate, so that convention is how totality is checked at runtime; the
  compiler's exhaustiveness check is the totality guarantee for the code
  itself).
- **Location decision**: `action_name`/`action_from_name` live in
  `src/ui/keymap.rs` (beside `Action` itself), not `src/config/keys.rs`.
  `crate::config` must never import `crate::ui` (the module doc's layering
  rule), so a table resolving config strings to `Action` cannot live
  config-side. `src/config/keys.rs` instead holds the *grammar* (string ->
  `ChordSpec`/`KeySeqSpec`, plain crossterm-typed data with zero `Action`/
  `Keymap` knowledge) and the raw `[keys.diff]`/`[keys.panel]` section data
  (`KeysConfig`, action names as un-resolved `String` keys). The actual
  merge — where both action-name resolution and grammar output meet — lives
  in the new edge module `src/ui/keymap_config.rs`, the same
  both-sides-import pattern `src/ui/lsp_config.rs` (Unit 3) and
  `src/ui/editor.rs`'s preset table (Unit 2) already established.
- **Key-string grammar (4.3)**: single chords, modifier stacking
  (`ctrl-`/`alt-`/`shift-`, case-insensitive, any combination), the fixed
  named-key vocabulary (`esc`, `enter`, `tab`, `space`, `backspace`,
  `delete`, `home`, `end`, `pageup`, `pagedown`, arrows, `insert`,
  `f1`..`f24`), `shift-tab` collapsing to the same `(BackTab, NONE)`
  representation `KeyChord::label` renders it as, space-separated two-chord
  sequences, three-or-more chords rejected, and a battery of garbage-input
  reject tests (empty string, dangling modifier prefix, unknown name,
  multi-char garbage) — all in `src/config/keys_tests.rs`. The label/grammar
  round-trip consistency test is described above.
- **Merge semantics (4.4)**: `src/ui/keymap_config.rs`'s `apply_overrides`
  (via `effective_keymap`) proves, with failing-tests-first-then-green
  coverage in `src/ui/keymap_config_tests.rs`: an overridden action gets
  *exactly* the listed keys (old defaults dropped, not appended);
  unlisted actions are untouched (binding count unchanged); `= []` unbinds
  an action entirely (its physical key resolves to nothing); a same-scope
  collision (a new binding landing on a key another binding — default or
  an earlier override — already claims) drops the loser and records one
  warning, with the override always winning; an unknown action name is an
  invalid value (warning, entry ignored, keymap untouched); and
  `[keys.panel]` overrides never leak into diff scope or vice versa.
- **Config wiring (4.5)**: `KeysConfig` (raw `BTreeMap<String,
  Vec<KeySeqSpec>>` per scope) is added to `Config` and deserialized inside
  `Config::from_table`'s existing two-pass walk, following the same
  warning-collection contract every other section uses. `src/ui/mod.rs`'s
  `pub fn run` now builds the effective keymap once
  (`keymap_config::effective_keymap(&app.config.keys)`) in place of the
  bare `Keymap::default_map()` call, and appends any merge warnings to
  `app.config_warnings` — the same dismissible status-line notice
  `crate::config::load`'s warnings already render through.
- **Help/footer truthfulness (4.6)**: `src/ui/mod_tests.rs` and
  `src/ui/keymap.rs` gained tests that build a keymap with an override
  *and* an unbind together and assert the help model reflects both — see
  the live tmux transcript below for the same proof end to end. A grep
  audit (delegated to an Explore agent, scoped to every `src/ui/*.rs` file
  outside the already-known shared-table files) found three additional
  hardcoded main-keymap key literals that the audit itself wouldn't have
  caught otherwise:
  - `src/ui/git_panel.rs`'s `remote_keys_line` hardcoded `"f"`/`"p"`/`"P"`
    for `RemoteFetch`/`RemotePull`/`RemotePush`.
  - `src/ui/staging_panel.rs`'s empty-panel hint hardcoded `"space"` for
    `ToggleStage`.
  - `src/ui/list_panel.rs`'s empty-panel hint hardcoded `"c"` for
    `Compose`.

  All three were small, mechanical, in-scope fixes (the task's own
  guidance: "fix only if small and in-scope") — each now resolves its key
  via a new shared `Keymap::label_for(scope, action) -> Option<String>`
  helper (which also replaced an equivalent private duplicate in
  `src/ui/welcome.rs`, itself already doing this correctly and serving as
  the reference pattern), gracefully omitting/degrading the segment if the
  action is ever unbound. New tests
  (`remote_keys_line_reflects_a_remapped_and_an_unbound_action` in
  `git_panel.rs`, `empty_staging_panel_hint_reflects_a_remapped_toggle_stage_key`/
  `empty_list_panel_hint_reflects_a_remapped_compose_key` in
  `mod_tests.rs`) pin the fix so it can't regress silently.
- **Perf tripwires (4.7)**: `src/ui/perf_tests.rs` (via
  `ui::app::perf_tests`) runs unchanged, budgets untouched, all five
  passing comfortably inside their existing margins (see
  `proofs/4-perf-tripwires.txt`) — the effective-keymap build happens once
  per session in `ui::run`, not on the hot paths these tests measure.
- **Docs**: `docs/example-config.toml` gained annotated `[keys.diff]`/
  `[keys.panel]` sections with the grammar explained, the merge semantics
  explained, a live example matching the spec's own demo
  (`next-file = "J"`, `quit = ["q", "ctrl-c"]`, `toggle-collapse = []`),
  and the **complete** 55-action-name list (46 diff-scope, 15 panel-scope,
  6 shared between both) each annotated with its default key — cross-
  checked programmatically against the real `action_name` table (every
  name present in both, nothing extra on either side; see Method) so the
  docs can't silently drift from the code. The pre-existing
  `config::load::tests::example_config_toml_parses_with_zero_warnings`
  drift test (task 1.9) stays green.

## Evidence Summary

| Artifact | Proves |
| --- | --- |
| `proofs/4-remap-help.txt` | Live tmux session: `[keys.diff]` with `next-file = "J"`, `quit = ["q", "ctrl-c"]`, `toggle-collapse = []` — `J` jumps files (proven via the Compose modal's title), `za` is a no-op (unbound), `?` shows exactly these three effects, and a final Ctrl-c actually quits *emitting* an added annotation (proving Ctrl-c now runs `Quit`, not the original `QuitDiscard` it collided with) |
| `proofs/4-nonsense-key-warning.txt` | A garbage key string (`"not a real key notation!!"`, too many chords) and an unknown action name in the same config both surface through the warning notice, the app stays fully usable, and the help overlay shows the defaults were preserved (both bad entries dropped) |
| `proofs/4-perf-tripwires.txt` | `cargo test --lib ui::app::perf_tests::` — all 5 tripwires pass, unchanged budgets |
| `cargo test --lib` (1249 passed, inline in `proofs/4-gates.txt`) | Every new test (grammar, bijectivity, merge semantics, help/footer truthfulness) plus the full pre-existing suite |
| `proofs/4-gates.txt` | Full four-gate transcript (`build`/`test`/`clippy --all-targets -D warnings`/`fmt --check`), all green |

All raw captures referenced below live under
`docs/specs/07-spec-config-layer/proofs/`, which is gitignored (the
pre-existing `docs/specs/*/proofs/` rule) — this markdown file is the only
thing from this task that's committed alongside the code.

### Screenshot substitution note

Per the precedent set in tasks 1.0/2.0/3.0: no terminal-capture-to-image
tool was available in this environment, so `.txt` transcripts
(`tmux capture-pane -p`) are used throughout instead of `.png` screenshots.

## Method

### crokey dependency-weight check (task 4.1)

```sh
$ mkdir -p /tmp/crokey-check && cd /tmp/crokey-check && cargo init -q
$ cargo add crokey -q
$ cargo tree
crokey_check v0.1.0
└── crokey v1.4.0
    ├── crokey-proc_macros v1.4.0 (proc-macro)
    │   ├── crossterm v0.29.0   # a SECOND, independent crossterm — this
    │   │                       # crate already depends on crossterm 0.29.0
    │   │   ├── ... parking_lot, mio, signal-hook, rustix, derive_more ...
    │   ├── proc-macro2, quote, syn   # full proc-macro toolchain
    │   └── strict
    ├── crossterm v0.29.0 (*)
    ├── once_cell
    └── serde (*)
```

(scratch directory removed after the check; no dependency was added to
this repo's `Cargo.toml`)

### Action-name/docs cross-check (task 4.7)

```sh
$ python3 - <<'EOF'
# Extracted every `# <name> = ` line under the [keys.diff]/[keys.panel]
# comment blocks in docs/example-config.toml (55 names) and every string
# literal in src/ui/keymap.rs's action_name() match body (55 names), and
# diff'd the two sets.
EOF
doc count: 55
real count: 55
in doc but not real: set()
in real but not doc: set()
```

### User demo (task 4.8), via tmux

```sh
# Demo repo: tempdir git repo, one commit, two uncommitted edits
#   src/a.rs: fn two() {} -> fn two_changed() {}
#   src/b.rs: + fn five() {}

# Demo config ($XDG_CONFIG_HOME/redquill/config.toml):
#   [keys.diff]
#   next-file = "J"
#   quit = ["q", "ctrl-c"]
#   toggle-collapse = []

tmux new-session -d -s rq_demo -x 200 -y 50 \
  "cd <demo-repo> && XDG_CONFIG_HOME=<demo-repo>/.xdgconfig HOME=<demo-repo> \
   /Users/sdavis/Projects/redquill/target/debug/redquill"

# 1. Startup: config-warning notice names the expected collision (quit's
#    ctrl-c collides with the default quit-discard binding on ctrl-c; the
#    user override wins) — this collision is an inherent, expected part of
#    reproducing the spec's own literal demo config, not a bug.
tmux capture-pane -t rq_demo -p
#  config: [keys.diff] quit: key "Ctrl-c" collides with "quit-discard"; "quit" wins (! to dismiss)

# 2. Press J (remapped next-file), then c (Compose) — the modal title
#    names src/b.rs, proving the cursor jumped from src/a.rs.
tmux send-keys -t rq_demo "J"; tmux send-keys -t rq_demo "c"
tmux capture-pane -t rq_demo -p   # modal titled "src/b.rs — issue"

# 3. Back at top (gg), press za (toggle-collapse, unbound): no-op — the
#    src/a.rs section stays expanded.
tmux send-keys -t rq_demo Escape; tmux send-keys -t rq_demo "gg"
tmux send-keys -t rq_demo "za"
tmux capture-pane -t rq_demo -p   # section still shows ▾ and the hunk body

# 4. Open help (?): Navigation shows "J   Next file section" (no more
#    "Tab"), and no "za  Collapse/expand file section" row at all.
tmux send-keys -t rq_demo "?"
tmux capture-pane -t rq_demo -p

# 5. Filter to "quit": diff-scope Quit shows Q / q / Ctrl-c (all three);
#    Panel scope (untouched — only [keys.diff] was configured) still shows
#    its original defaults (q / Q / Ctrl-c on Quit/QuitDiscard/QuitDiscard
#    respectively) — scope isolation, live.
tmux send-keys -t rq_demo "/quit"
tmux capture-pane -t rq_demo -p

# 6. Functional proof of the collision resolution: add one annotation via
#    c, then press Ctrl-c (remapped from quit-discard onto quit). The app
#    quit AND emitted the annotation markdown to stdout — proving Ctrl-c
#    now runs Quit (emit), not the original QuitDiscard (which would print
#    nothing and drop the annotation).
tmux send-keys -t rq_demo Escape; tmux send-keys -t rq_demo Escape
tmux send-keys -t rq_demo "c"; tmux send-keys -t rq_demo "demo annotation for quit proof"; tmux send-keys -t rq_demo Enter
tmux send-keys -t rq_demo C-c
tmux capture-pane -t rq_demo -p
#  ## src/a.rs
#
#  [issue] demo annotation for quit proof
```

### Nonsense key-string warning case (task 4.8)

```sh
# Demo config:
#   [keys.diff]
#   next-file = "not a real key notation!!"     # 5 space-separated tokens, max 2
#   frobnicate-nonexistent-action = "J"          # not a real action name

tmux new-session -d -s rq_demo2 ... redquill
tmux capture-pane -t rq_demo2 -p
#  config: [keys.diff] next-file: too many chords in "not a real key
#  notation!!" (max 2, space-separated) (and 1 more) (! to dismiss)

# App fully usable behind the warning; help overlay confirms both bad
# entries were dropped and next-file kept its default (Tab).
tmux send-keys -t rq_demo2 "?"; tmux send-keys -t rq_demo2 "/file"
tmux capture-pane -t rq_demo2 -p
#  Tab    Next file section
```

---

## Four-gate transcript

Full transcript at `proofs/4-gates.txt`; excerpted:

```
$ cargo build
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.10s

$ cargo test
     Running unittests src/lib.rs
test result: ok. 1249 passed; 0 failed; 3 ignored; 0 measured; 0 filtered out; finished in 5.23s
     Running unittests src/main.rs
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
     Running tests/git_integration.rs
test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.51s
     Running tests/git_log_integration.rs
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.16s
     Running tests/git_ls_files_integration.rs
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s
     Running tests/git_remote_integration.rs
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.36s
     Running tests/git_stage_integration.rs
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.20s
     Running tests/git_worktree_integration.rs
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.14s
     Running tests/lsp_integration.rs
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.26s
   Doc-tests redquill
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

$ cargo clippy --all-targets -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.10s

$ cargo fmt --check
(no output = clean)
```

All four gates pass on the final tree.

## Deviations From the Task File (flagged for reviewer awareness)

- **4.1's dependency question resolved to "no dependency."** The task file
  and spec both leave the door open to a hand-rolled parser; this
  implementation exercised that door because of `crokey`'s proc-macro/
  duplicate-crossterm weight (see the Decision section above). No
  `Cargo.toml` change, so no separate dependency commit exists — everything
  lands in the one feature commit.
- **4.2's file location**: `src/config/keys.rs` holds the grammar
  (`ChordSpec`/`KeySeqSpec`/`parse_key_string`) and the raw `KeysConfig`
  section data, exactly as the task file names it — but the *action-name*
  bijection table (`action_name`/`action_from_name`) lives in
  `src/ui/keymap.rs`, not `src/config/keys.rs`, per the CAUTION note in the
  task brief itself (config must never import ui types). This is a
  deliberate, documented placement choice, not a deviation from intent.
- **Scope growth (small, flagged as required by the task brief's own
  4.6 instruction)**: three additional hardcoded key literals were found
  and fixed outside the originally-enumerated file set
  (`git_panel.rs`/`staging_panel.rs`/`list_panel.rs`) — see the 4.6 bullet
  above. Each fix is a small, mechanical, behavior-preserving change
  (swap a literal for a `Keymap::label_for` lookup) with new tests pinning
  it; none touches the merge/grammar/bijectivity core of this task. Flagged
  here rather than silently expanding the diff without comment.

## Reviewer Conclusion

Unit 4's main-keymap contract is implemented and proven end to end: a
bijective, exhaustive-match action-name table over every `Action` variant;
a small hand-rolled key-string grammar (justified over `crokey` by
dependency weight, with a round-trip test against `KeyChord::label`
pinning the config-notation/help-display consistency the spec requires);
per-action merge semantics (replace/keep/unbind/collision) fully unit-
tested; `[keys.diff]`/`[keys.panel]` wired into `Config` through the
existing two-pass warning-collection contract; the effective keymap built
exactly once at startup with no per-keystroke parsing; the perf tripwires
unchanged and passing; and a live tmux session proving the spec's own demo
end to end, including a real quit-dispatch proof (an emitted annotation)
that the collision-resolved key genuinely changed behavior, not just the
help text. A task-4.6 audit closed three additional stale-key-literal
sites the spec's stated file list didn't originally cover. All four repo
gates are green.

Parent task 4.0 (main-keymap half of Unit 4) is ready to be marked
complete; task 5.0 (modal-table remapping) remains as the follow-up slice.
