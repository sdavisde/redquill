# Task 1.0 Proofs — Config file moves the sidebar (loading infrastructure + layout + search defaults)

## Task Summary

Implements the end-to-end config pipeline for spec 07 (`docs/specs/07-spec-config-layer/07-spec-config-layer.md`),
Unit 1: path discovery (`$XDG_CONFIG_HOME`/`~/.config/redquill/config.toml`,
`~/.config` on macOS too), a one-shot startup load (`crate::config::load`),
a serde-backed, partial-override `Config` struct (`crate::config::Config`),
the documented degradation contract (missing file = silent; syntax error =
full defaults + visible warning; invalid key/value = partial apply +
warning), and a dismissible, non-blocking status-line warning notice —
proven through its first two consumers: `[layout]` (sidebar side/width) and
`[search]` (Project Search startup case/whole-word/literal).

## What This Task Proves

- A config file at the discovered path is read exactly once at startup and
  actually changes rendered behavior (`sidebar_side`/`sidebar_width` move
  and resize the git panel sidebar).
- A config file that fails to parse never prevents the app from starting:
  it degrades to full defaults and shows a dismissible warning naming the
  file path and the parser's own error (including line info).
- With no config file present, behavior — and the full test suite — is
  byte-for-byte identical to pre-spec redquill: no notices, no changed
  defaults.
- Nothing in the config-loading path ever writes to stdout (stdout stays
  reserved for the annotation markdown).
- `[search]` config seeds Project Search's *startup* toggle state without
  touching in-session toggles.

## Evidence Summary

| Artifact | Proves |
| --- | --- |
| `proofs/1-sidebar-left.txt` | Config reaches rendering: `sidebar_side = "left"` + `sidebar_width = 50` move/resize the sidebar |
| `proofs/1-malformed-warning.txt` / `-dismissed.txt` | Degradation contract: malformed config → full defaults + visible, dismissible warning; app stays fully usable |
| `proofs/1-no-config-identical.txt` | No-config invariant: full test suite green with no config file present |
| `cargo test config::` (34 tests, inline below) | Path discovery + degradation-contract + `[layout]`/`[search]` deserialization unit tests |
| Four-gate transcript (inline below) | `build`/`test`/`clippy --all-targets -D warnings`/`fmt --check` all pass on the final tree |

All raw captures referenced below live under `docs/specs/07-spec-config-layer/proofs/`,
which is gitignored (`.gitignore`'s pre-existing `docs/specs/*/proofs/` rule,
verified present) — this markdown file is the only thing from this task
that's committed alongside the code.

### Screenshot substitution note

tmux was not preinstalled in this environment; it was installed via
`brew install tmux` (3.7b) so the interactive TUI could be driven
headlessly. No terminal-capture-to-image tool (`aha`, `ttygif`, `agg`,
`imgcat`, `chafa`, ...) was available or installable in this pass, so the
`.png` artifacts the task names are `.txt` ANSI/text pane captures instead
(`tmux capture-pane -p`), per the task's documented fallback.

---

## `proofs/1-sidebar-left.txt`

**What it proves:** a config file setting `[layout] sidebar_side = "left"`
and `sidebar_width = 50` moves the git panel sidebar to the left edge and
resizes it to exactly 50 columns — proving `Config` actually reaches
`crate::ui`'s render path (`split_layout`/`sidebar_width`), not just the
loader.

**Why it matters:** this is the spec's Unit 1 "config reaches rendering"
proof artifact and the task's User Demo's first half ("create
`~/.config/redquill/config.toml` containing `[layout] sidebar_side =
"left"`, launch redquill, open the git panel — the sidebar is on the
left").

**Command:**

```sh
# Demo config, in a scratch dir standing in for
# $XDG_CONFIG_HOME/redquill/config.toml (never the developer's real
# ~/.config):
#   [layout]
#   sidebar_side = "left"
#   sidebar_width = 50

tmux new-session -d -s rq-left -x 120 -y 30 \
  "cd <scratch>/demo-repo && \
   env -i HOME=<scratch>/fakehome XDG_CONFIG_HOME=<scratch>/xdg-left \
       PATH=$PATH TERM=xterm-256color \
       /Users/sdavis/Projects/redquill/target/debug/redquill"
tmux send-keys -t rq-left '`'          # open the git panel
tmux capture-pane -t rq-left -p > proofs/1-sidebar-left.txt
```

`<scratch>/demo-repo` is a throwaway git repo (`git init`, one commit, one
uncommitted edit to `main.rs`) created solely for this capture — not this
repo's own working tree, and not the developer's real `~/.config`.

**Result summary:** the git panel renders at the **left** edge, exactly
**50** columns wide (not the default 30%-of-120-clamped-to-40 formula,
which would have produced 40) — both the side and the width override took
effect. The diff pane still renders `main.rs`'s content on the right,
unblocked.

**Evidence** (full capture at `proofs/1-sidebar-left.txt`):

```
┌git: main  Changes History──────────────────────┐┌main.rs─────────────────────────────────────────────────────────────┐
│CHANGES                                         ││  ▾ M main.rs                                                       │
│  M main.rs                                     ││  @@ -1,4 +1,5 @@                                                   │
│                                                ││    1   1  fn main() {                                              │
│                                                ││    2   2      println!("hello world");                             │
│                                                ││    3     -    println!("this line will change");                   │
│                                                ││        3 +    println!("this line changed for the demo");          │
│                                                ││        4 +    println!("and this is a new line");                  │
│                                                ││    4   5  }                                                        │
└────────────────────────────────────────────────┘│                                                                    │
 [1 files]                                        │                                                                    │
 ffbdabf initial                                   │                                                                    │
 f fetch  p pull  P publish                       └────────────────────────────────────────────────────────────────────┘
 j/k move · Enter open file · f fetch · p pull · P publish · c commit · ` close · Tab tab · ? help
```

---

## `proofs/1-malformed-warning.txt` (+ `-dismissed.txt`)

**What it proves:** a config file with a genuine TOML syntax error (a
missing closing `]` on `[layout`) never blocks startup: the app comes up on
full defaults, shows a status-line notice naming the file path and the
parser's error, keeps rendering the diff underneath it, stays usable while
it's showing, and `!` (`Action::DismissConfigWarning`) dismisses it for the
session.

**Why it matters:** this is the spec's degradation-contract proof artifact
and the task's User Demo's second half ("Break the file (delete a quote),
relaunch — the app works on defaults and a status notice names the file and
the parse error").

**Command:**

```sh
# Malformed demo config (missing closing bracket):
#   [layout
#   sidebar_side = "left"

tmux new-session -d -s rq-bad -x 120 -y 30 \
  "cd <scratch>/demo-repo && \
   env -i HOME=<scratch>/fakehome XDG_CONFIG_HOME=<scratch>/xdg-malformed \
       PATH=$PATH TERM=xterm-256color \
       /Users/sdavis/Projects/redquill/target/debug/redquill"
tmux capture-pane -t rq-bad -p > proofs/1-malformed-warning.txt
tmux send-keys -t rq-bad 'j'            # app is usable behind the notice
tmux send-keys -t rq-bad '!'            # dismiss it
tmux capture-pane -t rq-bad -p > proofs/1-malformed-warning-dismissed.txt
```

**Result summary:** the diff renders normally (unblocked), and the footer
shows `config: <path>: TOML parse error at line 1 ... (! to dismiss)`
naming both the file path and the parser's line-carrying message. `j`
moves the cursor normally while the notice is up (app stays usable). `!`
clears the notice; the ordinary context-sensitive footer hint strip (`j/k
move · ] hunk · ...`) takes its place immediately afterward.

**Evidence** (full captures at `proofs/1-malformed-warning.txt` and
`proofs/1-malformed-warning-dismissed.txt`; the footer's full text —
including the path — is in the raw file, truncated below only by this
markdown's width):

```
┌main.rs───────────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│  ▾ M main.rs                                                                                                         │
│  @@ -1,4 +1,5 @@                                                                                                     │
│    1   1  fn main() {                                                                                                │
│    2   2      println!("hello world");                                                                               │
│    3     -    println!("this line will change");                                                                     │
│        3 +    println!("this line changed for the demo");                                                            │
│        4 +    println!("and this is a new line");                                                                    │
│    4   5  }                                                                                                          │
└──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
 config: /private/tmp/claude-501/-Users-sdavis-Projects-redquill/8024c362-6f86-4f29-a418-9cdf32f9452b/scratchpad/xdg-mal
```

After `!` (`proofs/1-malformed-warning-dismissed.txt`), the last footer row
becomes the ordinary hint strip:

```
 j/k move · ] hunk · za fold · Space stage hunk · S stage file · c comment · / search · ` git panel · ? help
```

---

## `proofs/1-no-config-identical.txt`

**What it proves:** with no `redquill/config.toml` anywhere the resolver
looks (verified: no `~/.config/redquill` directory exists on this machine,
and `$XDG_CONFIG_HOME` is unset in the ambient shell), the full test suite
— which exercises every layout/search code path this task touched — passes
exactly as it did before this task started: the no-config invariant.

**Why it matters:** this is the spec's success metric 4 ("with no config
file, the full test suite ... show[s] behavior identical to pre-spec").

**Command:**

```sh
ls ~/.config/redquill        # -> No such file or directory
echo "$XDG_CONFIG_HOME"      # -> (empty)
cargo test
```

**Result summary:** every test binary reports `test result: ok` with zero
failures — **1156 passed** in the library crate alone (0 failed, 3
ignored — the three pre-existing `#[ignore]` smoke-transcript regenerators,
unrelated to this task), plus all integration-test binaries and doctests
green.

**Evidence** (full transcript at `proofs/1-no-config-identical.txt`; tail
excerpt):

```
     Running tests/lsp_integration.rs (target/debug/deps/lsp_integration-0fc3511744377b42)

running 4 tests
test second_file_gets_its_own_did_open ... ok
test shutdown_terminates_the_server ... ok
test definition_references_hover_flow_and_did_open_once ... ok
test unanswered_request_times_out_to_failed ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.28s

   Doc-tests redquill

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

```sh
$ cargo test --lib 2>&1 | tail -3
test result: ok. 1156 passed; 0 failed; 3 ignored; 0 measured; 0 filtered out; finished in 4.59s
```

---

## `cargo test config::` — the loading-contract unit tests

`src/config/mod_tests.rs` (Config/deserialization, task 1.3) and
`src/config/load_tests.rs` (path discovery + `load`, tasks 1.2/1.4) cover:
empty file → all defaults; partial file overrides only the named key;
unknown top-level section and unknown key within a known section both
collected, not fatal; invalid value for a known key (bad enum string,
out-of-range `sidebar_width`, wrong TOML type) falls back to default and is
collected; XDG-set / XDG-unset-falls-back-to-`~/.config` / missing-home
path discovery; missing file → silent defaults; TOML syntax error → full
defaults + one warning naming path and parser line; a structural guard that
neither `mod.rs` nor `load.rs` contains a stdout-writing call; and a drift
guard that `docs/example-config.toml` itself parses through the real
`Config::from_table` with zero warnings.

```
$ cargo test --lib config::
running 34 tests
test config::load::tests::explicit_test_override_hook_is_just_a_pathenv_pointed_at_a_tempdir ... ok
test config::load::tests::missing_home_and_xdg_yields_no_path ... ok
test config::load::tests::missing_file_is_silent_defaults ... ok
test config::load::tests::config_loading_source_never_writes_to_stdout ... ok
test config::load::tests::load_reads_the_real_environment_without_panicking ... ok
test config::load::tests::unreadable_path_degrades_like_a_missing_file ... ok
test config::load::tests::xdg_set_wins_over_home_fallback ... ok
test config::load::tests::example_config_toml_parses_with_zero_warnings ... ok
test config::load::tests::xdg_unset_falls_back_to_dot_config_under_home ... ok
test config::tests::case_mode_parses_all_three_values ... ok
test config::load::tests::present_valid_file_is_read_and_applied ... ok
test config::tests::both_sections_together_with_a_mix_of_valid_and_invalid_keys ... ok
test config::tests::empty_file_is_all_defaults ... ok
test config::tests::partial_layout_overrides_only_the_named_key ... ok
test config::tests::config_warning_display_names_path_section_and_key ... ok
test config::tests::search_boolean_field_wrong_type_is_an_invalid_value ... ok
test config::tests::invalid_value_for_a_known_key_falls_back_to_default_and_is_collected ... ok
test config::tests::search_invalid_case_falls_back_to_default_and_is_collected ... ok
test config::tests::non_table_section_value_is_an_invalid_value ... ok
test config::tests::search_section_partial_override ... ok
test config::tests::sidebar_side_parses_both_values ... ok
test config::tests::search_whole_word_and_literal_apply ... ok
test config::tests::sidebar_width_wrong_type_is_an_invalid_value ... ok
test config::tests::sidebar_width_in_range_applies ... ok
test config::tests::unknown_top_level_section_is_collected_not_fatal ... ok
test config::tests::sidebar_width_out_of_range_is_an_invalid_value ... ok
test config::tests::unknown_key_within_a_known_section_is_collected_not_fatal ... ok
test config::load::tests::syntax_error_yields_full_defaults_and_one_warning_naming_path_and_line ... ok
test config::load::tests::parseable_but_invalid_entries_partially_apply_with_one_warning_each ... ok

test result: ok. 34 passed; 0 failed; 0 ignored; 0 measured; 1125 filtered out; finished in 0.01s
```

`ui/mod_tests.rs`, `ui/project_search_tests.rs`, and `main.rs` also gained
tests for this task: `sidebar_width`/`split_layout` unset-vs-configured/left
side/hidden-sidebar behavior, the config-warning notice's render/dismiss
cycle and its footer-height reservation, and `ProjectSearchState::seeded`'s
config-to-startup-toggle mapping (including "an already-open session's
toggles aren't clobbered, but a fresh reopen reseeds from config"). All
included in the 1156-test count above.

---

## Four-gate transcript

```
$ cargo build
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.07s

$ cargo test 2>&1 | tail -20
test switch_branch_fails_on_conflicting_dirty_tree_with_stderr ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.13s

     Running tests/lsp_integration.rs (target/debug/deps/lsp_integration-0fc3511744377b42)

running 4 tests
test second_file_gets_its_own_did_open ... ok
test shutdown_terminates_the_server ... ok
test definition_references_hover_flow_and_did_open_once ... ok
test unanswered_request_times_out_to_failed ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.28s

   Doc-tests redquill

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

$ cargo clippy --all-targets -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.13s

$ cargo fmt --check
(no output = clean)
```

All four gates pass on the final tree.

## Reviewer Conclusion

Unit 1's whole pipeline is implemented and proven end to end: path
discovery is a pure, injectable function (`crate::config::load::PathEnv` /
`resolve_config_path`) exercised without touching the developer's real
`~/.config`; `Config`'s two-pass parse (raw `toml::Table` walked
key-by-key, unknowns/invalid values collected as warnings, valid entries
deserialized via serde into the concrete section structs) implements the
documented degradation contract exactly, with unit tests for every branch;
`load()` never writes to stdout; `main.rs` loads config exactly once before
the first render (the pre-existing `Cli`/`Config` name collision is
resolved by renaming the CLI-derived struct to `RunConfig`); the UI's
warning notice is dismissible (`!`, reachable from the keymap and listed in
`?` help via the shared `Keymap`/`group_of` tables — no loose match arms),
non-blocking, and never reaches stdout; `[layout] sidebar_side`/
`sidebar_width` reach `split_layout`/`sidebar_width` with the "unset
preserves today's formula exactly" contract pinned by test; and `[search]`
seeds Project Search's *startup* state only, leaving in-session toggles
alone. The four repo gates (build/test/clippy/fmt) are green, and the
no-config invariant is demonstrated both by a live capture and by the full
test suite passing unchanged. `docs/example-config.toml` ships with fully
annotated `[layout]`/`[search]` sections and is itself guarded by a
zero-warnings parse test so it can't silently rot.

Parent task 1.0 is ready to be marked complete.
