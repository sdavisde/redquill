# Task 3.0 Proofs — Config file controls code intelligence (`[lsp]` overrides)

## Task Summary

Implements Unit 3 of spec 07 (`docs/specs/07-spec-config-layer/07-spec-config-layer.md`):
per-language `[lsp.rust]`/`[lsp.typescript]`/`[lsp.python]`/`[lsp.go]`
override tables (`command`, `args`, `enabled`, default `enabled = true`)
overlaid onto `crate::lsp::config::default_commands()`, producing the
effective `HashMap<ServerLang, LangServerCmd>` that `LspManager` is now
constructed with in `src/ui/code_intel.rs`. `enabled = false` removes a
language from the effective map entirely, so it degrades exactly like an
unconfigured/missing server (silent — no server spawned, `gd`/`gr`/`K` fall
back to the "no code intelligence here" footer message). Config errors in
the section itself (wrong type, non-table value, unknown key/language)
follow the Unit 1 warning contract via `Config::from_table`'s two-pass
known-keys walk.

## What This Task Proves

- `LspServerOverride`/`LspConfig` deserialize partial overlays correctly:
  overriding one language's `command`+`args` leaves the other three at
  their defaults; `enabled = false` disables a language; `args` set alone
  overrides args only (default command kept); `command` set alone
  overrides command only (default args kept) — the exact case called out
  in task 3.1's failing-tests-first list.
- The merge function (`crate::ui::lsp_config::effective_lsp_commands`)
  overlays `LspConfig` onto `default_commands()` with the same semantics,
  independently unit-tested against the plain `HashMap<ServerLang,
  LangServerCmd>` shape `LspManager` actually consumes.
- The layering boundary holds: `src/lsp/*.rs` has zero `use crate::config`
  (or `crate::ui`/`ratatui`) imports, and `src/config/*.rs` has zero `use
  crate::lsp` imports — verified by grep, not just intention. The overlay
  function that necessarily depends on both types lives at the edge, in
  `crate::ui::lsp_config`, the same edge module pattern Unit 2 already
  established for `[editor]` (`resolve_editor_config_tier`/`PRESETS` in
  `ui::editor`).
- A real, live `gd` definition peek — driven through the actual TUI via
  tmux — resolves through a logging wrapper script configured as
  `[lsp.rust] command`, with the wrapper's own log proving the *configured*
  command (not the hardcoded `rust-analyzer` default) is what got spawned,
  and the peek overlay showing a correct definition location and preview.
- `enabled = false` degrades a configured-or-not language exactly like an
  unconfigured server: `gd` shows `"no code intelligence here"` and the
  wrapper log gains no new entry (server never spawned for that language).
- `docs/example-config.toml` grew an annotated `[lsp]` section covering all
  four language tables and the `enabled` degradation contract, without
  breaking its own zero-warnings drift test
  (`config::load::tests::example_config_toml_parses_with_zero_warnings`).

## Evidence Summary

| Artifact | Proves |
| --- | --- |
| `proofs/3-lsp-override.txt` | `[lsp.rust] command` pointed at a logging wrapper → `gd` resolves a real definition through it; wrapper log shows the invocation |
| `proofs/3-lsp-disabled-degradation.txt` | `[lsp.rust] enabled = false` → `gd` degrades silently exactly like a missing server; wrapper log unchanged (no spawn) |
| `cargo test --lib config::tests::lsp` / `cargo test --lib lsp_config::` (21 tests, inline below) | `LspConfig`/`LspServerOverride` deserialization and the `effective_lsp_commands` merge semantics |
| `proofs/3-gates.txt` (four-gate transcript, inline below) | `build`/`test`/`clippy --all-targets -D warnings`/`fmt --check` all pass on the final tree |

All raw captures referenced below live under
`docs/specs/07-spec-config-layer/proofs/`, which is gitignored (the
pre-existing `docs/specs/*/proofs/` rule) — this markdown file is the only
thing from this task that's committed alongside the code.

### Screenshot substitution note

Per the same precedent set in tasks 1.0/2.0: no terminal-capture-to-image
tool was available in this environment, so `.txt` transcripts
(`tmux capture-pane -p`) are used throughout instead of `.png` screenshots.

### Environment note: a real `rust-analyzer`, not a stub

The task brief anticipated the demo machine might have no working
`rust-analyzer` and allowed substituting a stub that just proves the
configured command was spawned. This environment turned out to have a
broken `rustup` proxy at `~/.cargo/bin/rust-analyzer` (no `rust-analyzer`
component installed for the active toolchain — it exits immediately with
"rustup could not choose a version of rust-analyzer to run"), but running
`rustup component add rust-analyzer` installed a real, working binary at
`~/.rustup/toolchains/stable-aarch64-apple-darwin/bin/rust-analyzer`. The
logging wrapper execs that real binary, so this proof demonstrates the
full FR — a genuine `gd` definition resolution through the configured
command — rather than the stub fallback.

Two environment quirks worth recording since they cost real debugging time
and would trip up a future re-run:

- The wrapper's `PATH` must include a directory with real `cargo`/`rustc`
  binaries (`~/.cargo/bin`, alongside the wrapper directory placed first so
  `rust-analyzer` itself still resolves to the wrapper) — rust-analyzer
  shells out to `cargo metadata`/`rustc --print cfg` to load the workspace,
  and without them on `PATH` it fails to load the project (visible only in
  `stderr`, which the app's own `spawn_server` intentionally nulls per
  `src/lsp/transport.rs`, so this only surfaced via manual `stderr`
  capture during debugging, not through redquill's own footer).
- The very first request in a fresh session can return a fast
  `LspEvent::Failed` (a real JSON-RPC error response, not a timeout) while
  rust-analyzer is still finishing its initial index; the server process
  stays alive (`Slot::Live`), and a second `gd`/`K` a few seconds later
  succeeds normally. This is real-world `rust-analyzer` behavior, not a
  redquill bug — `docs/specs/07-spec-config-layer/07-spec-config-layer.md`
  Unit 3's FR only requires that a *missing or failing-to-start* command
  degrade silently, which is separately proven by the disabled-language
  capture; a slow-to-index live server answering its first query with an
  error is outside that FR's scope.

---

## `proofs/3-lsp-override.txt`

**What it proves:** with `[lsp.rust] command` in a demo config (loaded via
`XDG_CONFIG_HOME` pointed at a scratch dir — never the developer's real
`~/.config`) pointed at a logging wrapper script that logs its own
invocation then `exec`s the real `rust-analyzer`, positioning the cursor on
the `greet` call in `src/main.rs` and pressing `g` then `d` opens the
definition peek at `src/main.rs:1` with the correct preview
(`fn greet(name: &str) -> String { format!("Hello, {name}!") }`) — and the
wrapper's log file shows it was invoked, proving the *configured* command
is what actually ran the whole LSP lifecycle (spawn, handshake, `didOpen`,
`textDocument/definition`), not the hardcoded default.

**Why it matters:** this is the spec's Unit 3 CLI-transcript proof artifact
and the task's User Demo ("point `[lsp.rust] command` at a wrapper script
... press `gd` on a symbol — definition peek works through your configured
server").

**Method:**

```sh
# Demo repo: a tiny real cargo crate (needed so rust-analyzer can actually
# load a workspace and resolve symbols), one commit, one uncommitted edit
# adding a farewell() function and its call site so the diff has Added/
# Context lines to put the cursor on.
#   src/main.rs (working tree, uncommitted diff against the initial commit):
#     fn greet(name: &str) -> String {
#         format!("Hello, {name}!")
#     }
#
#     fn main() {
#         let message = greet("world");
#         println!("{message}");
#         let bye = farewell("world");       # added
#         println!("{bye}");                 # added
#     }
#
#     fn farewell(name: &str) -> String {    # added
#         format!("Goodbye, {name}!")        # added
#     }                                      # added

# Demo config ($XDG_CONFIG_HOME/redquill/config.toml equivalent):
#   [lsp.rust]
#   command = "<scratch>/bin-wrapper3/rust-analyzer"

# Logging wrapper, execs the real toolchain binary (not the rustup shim —
# see the environment note above):
cat > <scratch>/bin-wrapper3/rust-analyzer <<'SH'
#!/bin/sh
LOG=<scratch>/rust-analyzer-wrapper.log
{
    printf '[%s] rust-analyzer wrapper invoked with argv:' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    for arg in "$@"; do printf ' [%s]' "$arg"; done
    printf '\n'
} >> "$LOG"
exec /Users/sdavis/.rustup/toolchains/stable-aarch64-apple-darwin/bin/rust-analyzer "$@"
SH
chmod +x <scratch>/bin-wrapper3/rust-analyzer

tmux new-session -d -s rq-lsp -x 160 -y 40 \
  "cd <scratch>/demo3-repo && \
   env -i HOME=/Users/sdavis XDG_CONFIG_HOME=<scratch>/xdg-lsp \
       PATH=<scratch>/bin-wrapper3:/Users/sdavis/.cargo/bin:/usr/bin:/bin \
       TERM=xterm-256color \
       /Users/sdavis/Projects/redquill/target/debug/redquill"

# Navigate to the `greet(` call site (line 6, column 18 — the 'g' of
# `greet`) and open the definition peek:
tmux send-keys -t rq-lsp -l "jjjjjjjjjjjj"   # cursor onto the call-site line
tmux send-keys -t rq-lsp -l "0"              # column 0
tmux send-keys -t rq-lsp -l "llllllllllllllllll"  # 18 x 'l' -> column 18 ('g' of greet)
tmux send-keys -t rq-lsp -l "g"
tmux send-keys -t rq-lsp -l "d"              # g<then>d: GotoDefinition
# (first request in a fresh session can return a fast error while
# rust-analyzer is still indexing — see the environment note; retrying a
# few seconds later succeeds, which is what this transcript captures)
tmux capture-pane -t rq-lsp -p
cat <scratch>/rust-analyzer-wrapper.log
```

**Result summary:** the config loads with **zero warnings** (a valid
absolute-path `command` string produces no notice — confirmed by the
default footer hint bar showing on first launch, no `config:` prefix). The
`gd` press opens a two-pane peek overlay: the left pane lists
`src/main.rs:1` (the one definition location `greet` actually has), and the
right pane previews its real source —
`fn greet(name: &str) -> String {` /
`    format!("Hello, {name}!")` /
`}` — exactly matching `src/main.rs`'s committed content at that line. The
wrapper log contains exactly one invocation line, timestamped to the same
session, confirming the *configured* wrapper command (not the default
`rust-analyzer` binary name) is what `LspManager::with_commands` spawned —
proving `effective_lsp_commands` correctly wired the `[lsp.rust] command`
override into the real spawn path end to end.

**Evidence** (full capture at `proofs/3-lsp-override.txt`):

```
┌src/main.rs───────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│  ▾ A Cargo.lock                                                                                                                                              │
│  @@ -0,0 +1,7 @@                                                                                                                                             │
...
│  ▾ M src/main.rs                                                                                                                                             │
│  @@ -5,4 +5,10 @@ fn greet(name: &str) -> String {                                                                                                           │
│    5   5  fn main() {                                                                                                                                        │
│    6   6      let message = greet("world");                                                                                                                  │
...
┌definition───────────────────────────┐┌preview────────────────────────────────────────────────────────────────┐
│src/main.rs:1                        ││fn greet(name: &str) -> String {                                       │
│                                     ││    format!("Hello, {name}!")                                          │
│                                     ││}                                                                      │
└ j/k move  Enter jump  Esc/q close ──┘└───────────────────────────────────────────────────────────────────────┘

=== rust-analyzer-wrapper.log ===
[2026-07-16T15:32:33Z] rust-analyzer wrapper invoked with argv:
```

(The wrapper's argv list is empty because Rust's default `LangServerCmd`
has `args: []` — `[lsp.rust] command` overrode the command only, per
task 3.1's "command without args overrides command only" contract, so the
default empty args were kept and passed through.)

---

## `proofs/3-lsp-disabled-degradation.txt`

**What it proves:** `[lsp.rust] enabled = false` removes Rust from the
effective server map entirely; pressing `gd` at the exact same cursor
position as above degrades silently to the same `"no code intelligence
here"` footer message the app shows for any unsupported/missing-server
case — never a spawn attempt, never a warning. The wrapper log file (still
containing only the one entry from the prior capture) confirms no new
invocation happened.

**Why it matters:** this is the task's User Demo's second half ("Set
`enabled = false`, relaunch — LSP keys degrade exactly like an
unconfigured server, everything else works") and the spec's `enabled =
false` FR.

**Method:** same `tmux`/`env -i`/`XDG_CONFIG_HOME` approach, config
containing `[lsp.rust]\nenabled = false\n`, same cursor navigation, `gd`
pressed immediately (no retry needed, since there's no server to wait on).

**Result summary:** the footer shows `no code intelligence here`
immediately, and the wrapper log's line count is unchanged from the prior
capture — the disabled language was never spawned.

**Evidence** (full capture at `proofs/3-lsp-disabled-degradation.txt`):

```
 no code intelligence here
```

(preceded by the same diff render as the enabled case — omitted here since
it's identical to the render already shown above).

---

## `cargo test --lib config::tests::lsp` / `cargo test --lib lsp_config::` — the Unit 3 unit tests

`src/config/mod.rs` + `src/config/mod_tests.rs` (`LspConfig`/
`LspServerOverride` deserialization, task 3.1) and `src/ui/lsp_config.rs` +
`src/ui/lsp_config_tests.rs` (the `effective_lsp_commands` merge, task 3.1)
cover: partial override of one language leaving the other three at their
defaults; `enabled = false` disabling a language; `args` overriding args
only (default command kept); `command` overriding command only (default
args kept); wrong-type/non-table/unknown-key/unknown-language rejection
with one warning each; and the merge function's own independent test suite
against the plain `HashMap<ServerLang, LangServerCmd>` shape, including all
four languages overridden independently in one config and `enabled = false`
winning even when `command`/`args` are also set on the same table.

```
$ cargo test --lib config::tests::lsp
running 14 tests
test config::tests::lsp_disable_one_language ... ok
test config::tests::lsp_args_without_command_overrides_args_only ... ok
test config::tests::lsp_args_wrong_type_is_an_invalid_value ... ok
test config::tests::lsp_args_element_wrong_type_is_an_invalid_value ... ok
test config::tests::lsp_command_wrong_type_is_an_invalid_value ... ok
test config::tests::lsp_language_non_table_value_is_an_invalid_value ... ok
test config::tests::lsp_enabled_wrong_type_is_an_invalid_value ... ok
test config::tests::lsp_command_without_args_overrides_command_only ... ok
test config::tests::lsp_non_table_section_value_is_an_invalid_value ... ok
test config::tests::lsp_section_empty_is_all_defaults ... ok
test config::tests::lsp_override_one_language_command_and_args_leaves_others_default ... ok
test config::tests::lsp_server_override_default_is_enabled_with_no_overrides ... ok
test config::tests::lsp_unknown_key_within_a_language_table_is_collected_not_fatal ... ok
test config::tests::lsp_unknown_language_table_is_collected_not_fatal ... ok

test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 1198 filtered out; finished in 0.00s

$ cargo test --lib lsp_config::
running 7 tests
test ui::lsp_config::tests::disabling_a_language_wins_even_if_command_or_args_are_also_set ... ok
test ui::lsp_config::tests::args_without_command_overrides_args_only ... ok
test ui::lsp_config::tests::disable_one_language_removes_it_others_stay_default ... ok
test ui::lsp_config::tests::command_without_args_keeps_default_args ... ok
test ui::lsp_config::tests::all_four_languages_can_be_overridden_independently ... ok
test ui::lsp_config::tests::no_overrides_yields_defaults_unchanged ... ok
test ui::lsp_config::tests::override_one_language_leaves_others_default ... ok

test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 1205 filtered out; finished in 0.00s
```

The full library test suite remains green throughout, including the
pre-existing `config::load::tests::example_config_toml_parses_with_zero_warnings`
drift guard, which still passes with the new `[lsp]` section added to
`docs/example-config.toml`:

```
$ cargo test --lib
test result: ok. 1216 passed; 0 failed; 3 ignored; 0 measured; 0 filtered out; finished in ~5s
```

---

## Layering verification (grep, not intention)

```
$ grep -rn "use crate::config\|use crate::ui\|ratatui" src/lsp/*.rs
(no output)

$ grep -rn "use crate::lsp\|use crate::ui\|ratatui" src/config/*.rs
src/config/mod.rs:33://! `crate::ui` or any TUI/ratatui type; `Config` crosses into
(doc-comment prose only, not an import)
```

`src/lsp/` imports neither `crate::config` nor `crate::ui`; `src/config/`
imports neither `crate::lsp` nor `crate::ui` (the one hit is a module-doc
sentence, not a `use` statement). The merge function that necessarily
depends on both types (`effective_lsp_commands`) lives at the edge, in
`crate::ui::lsp_config`, per the task's own guidance and the precedent
`ui::editor` already set for `[editor]`.

---

## Four-gate transcript

Full transcript at `proofs/3-gates.txt`; excerpted:

```
$ cargo build
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.23s

$ cargo test 2>&1 | tail -10
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.29s

   Doc-tests redquill

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

$ cargo clippy --all-targets -- -D warnings
    Checking redquill v0.8.0 (/Users/sdavis/Projects/redquill)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.18s

$ cargo fmt --check
(no output = clean)
```

All four gates pass on the final tree.

## Deviations From the Task File (flagged for reviewer awareness)

- **Where `LspManager` is constructed.** The task brief says "wire in
  `src/main.rs`/app setup." `main.rs` loads config and hands the whole
  `Config` to `App::set_config` unchanged (no LSP-specific code needed
  there, unlike `[editor]`, which needs CLI-flag/env-var inputs only
  `main.rs` has). The actual `LspManager::with_commands` construction
  happens where it always has — lazily, on first `gd`/`gr`/`K`, in
  `src/ui/code_intel.rs::request` — now computing `effective_lsp_commands(
  &app.config.lsp)` at that point instead of calling `LspManager::new`.
  Since `app.config` is only ever set once, before the first render, this
  is equivalent to eager construction at startup, just deferred to the
  existing lazy-init call site rather than duplicated in `main.rs`. Flagged
  here so a reviewer looking for new `main.rs` lines for this task
  understands why there are none.
- **`LspManager::new` is now unused within the crate** (every call site
  switched to `LspManager::with_commands`) but stays as `pub fn` — it's
  part of the module's public API (re-exported from `crate::lsp`) and
  documents the "no overrides" case explicitly; removing it wasn't in this
  task's scope and clippy raises no dead-code warning for a `pub` item.
- **Environment substitution**: the task anticipated needing a stub in
  place of a real `rust-analyzer`; this environment had a broken `rustup`
  shim that was fixed with `rustup component add rust-analyzer`, so the
  proof uses a genuinely functional server rather than the stub fallback —
  see the "Environment note" above for the two quirks that made this
  non-obvious (stderr nulled by design, first-request timing).

## Reviewer Conclusion

Unit 3 is implemented and proven end to end: `LspConfig`/
`LspServerOverride` are added to `Config` as plain partial-override data
(no `ServerLang`/`LangServerCmd` knowledge), validated key-by-key through
the same two-pass `from_table` warning contract every other section uses;
`effective_lsp_commands` (in the new `src/ui/lsp_config.rs` edge module)
overlays that plain data onto `default_commands()` with the exact merge
semantics required (`args`-without-`command` overrides args only,
`command`-without-`args` overrides command only, `enabled = false` removes
the language), independently unit-tested against the real
`HashMap<ServerLang, LangServerCmd>` shape `LspManager` consumes; the
layering boundary (`crate::lsp` never imports `crate::config` and vice
versa) is verified by grep; `docs/example-config.toml` gained a fully
annotated `[lsp]` section without breaking its own zero-warnings drift
test; and a live TUI transcript (via a logging wrapper around a real
`rust-analyzer` and tmux) proves a `gd` definition peek resolves correctly
through the *configured* command, with the disabled-language case shown to
degrade silently exactly like an unconfigured server. All four repo gates
are green.

Parent task 3.0 is ready to be marked complete.
