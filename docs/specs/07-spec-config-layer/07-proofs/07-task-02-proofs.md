# Task 2.0 Proofs — Config file picks your editor (`[editor]` templating + presets)

## Task Summary

Implements Unit 2 of spec 07 (`docs/specs/07-spec-config-layer/07-spec-config-layer.md`):
a lazygit-style `[editor] edit_at_line` template grammar (`{{filename}}`/
`{{line}}` placeholders, substituted per whitespace token, never through a
shell), a built-in `preset` table covering the eleven names the spec
requires as a floor set (`vim`, `nvim`, `helix`, `vscode`, `vscodium`,
`zed`, `emacs`, `nano`, `micro`, `sublime`, `kakoune`), an `EditorConfig`
section added to `Config`, and the five-tier editor-resolution precedence
(`--editor` flag > `[editor]` config > `$VISUAL` > `$EDITOR` > `"nvim"`)
wired into `main.rs`. This replaces the old two-family hardcoded heuristic
(VS Code family `--goto file:line`, else `+line`) as the *only* rule for
config-supplied editors; that heuristic still applies, unchanged, to every
non-config tier.

## What This Task Proves

- A template or preset from `[editor]` produces the exact argv the spec
  requires, with substitution happening strictly per-token so a filename
  containing spaces survives intact.
- All eleven built-in presets expand to known-correct, individually
  tested argv for a sample file/line.
- An unknown preset name is an invalid value: it's reported through the
  same `ConfigWarning` notice Unit 1 built, and `g<Space>` falls through
  to `$VISUAL`/`$EDITOR`/`"nvim"` exactly as if `[editor]` were absent.
- The five-tier precedence resolves correctly end to end, including the
  "explicit `edit_at_line` wins over `preset`" rule and every empty/
  whitespace env-var edge case the pre-existing three-tier chain already
  covered.
- A real editor outside the old two hardcoded families (`zed`) opens at
  the cursor's exact line, driven through the live TUI via a logging fake
  `zed` binary on `PATH`.
- `docs/example-config.toml` grew an annotated `[editor]` section without
  breaking its own zero-warnings drift test.

## Evidence Summary

| Artifact | Proves |
| --- | --- |
| `proofs/2-zed-preset.txt` | `preset = "zed"` + `g<Space>` spawns `zed <file>:<line>` — a non-hardcoded-family editor opens at the cursor line |
| `proofs/2-unknown-preset-fallthrough.txt` | Unknown preset name → visible warning naming `[editor] preset` + the bad value, app stays usable |
| `cargo test editor::`/`config::tests::editor` (38 tests, inline below) | Template substitution, preset table, config-tier resolution, and `EditorConfig` deserialization unit tests |
| `proofs/2-gates.txt` (four-gate transcript, inline below) | `build`/`test`/`clippy --all-targets -D warnings`/`fmt --check` all pass on the final tree |

All raw captures referenced below live under
`docs/specs/07-spec-config-layer/proofs/`, which is gitignored (the
pre-existing `docs/specs/*/proofs/` rule) — this markdown file is the only
thing from this task that's committed alongside the code.

### Screenshot substitution note

Per the same precedent set in task 1.0's proofs: no terminal-capture-to-
image tool was available in this environment, so the task's `.txt`
transcript fallback is used throughout (`tmux capture-pane -p`) instead of
`.png` screenshots.

---

## `proofs/2-zed-preset.txt`

**What it proves:** with `[editor] preset = "zed"` in a demo config (loaded
via `XDG_CONFIG_HOME` pointed at a scratch dir — never the developer's real
`~/.config`), pressing `g<Space>` on a diff line spawns a fake `zed`
binary (placed first on `PATH`, logging its own argv rather than actually
launching an editor) with exactly the argv the `zed` preset's template
(`"zed {{filename}}:{{line}}"`) predicts for the line under the cursor —
proving the config → resolution → spawn path works end to end for an
editor outside the old hardcoded `code`/`codium` and `+line` families.

**Why it matters:** this is the spec's Unit 2 CLI-transcript proof
artifact and the task's User Demo ("add `[editor] preset = "zed"` ... press
`g<Space>` — your editor opens at that exact line") and success metric 1
("a Zed user adds two lines ... presses `g<Space>`, and lands in Zed at the
cursor line").

**Method:**

```sh
# Demo config ($XDG_CONFIG_HOME/redquill/config.toml equivalent, in a
# scratch dir):
#   [editor]
#   preset = "zed"

# Logging fake `zed`, placed first on $PATH:
cat > <scratch>/bin-wrapper/zed <<'SH'
#!/bin/sh
LOG=<scratch>/zed-wrapper.log
{
    printf 'zed invoked with argv:'
    for arg in "$@"; do printf ' [%s]' "$arg"; done
    printf '\n'
} >> "$LOG"
exit 0
SH
chmod +x <scratch>/bin-wrapper/zed

tmux new-session -d -s rq-zed -x 140 -y 30 \
  "cd <scratch>/demo2-repo && \
   env -i HOME=<scratch>/fakehome2 XDG_CONFIG_HOME=<scratch>/xdg-zed \
       PATH=<scratch>/bin-wrapper:$PATH TERM=xterm-256color \
       /Users/sdavis/Projects/redquill/target/debug/redquill"
tmux capture-pane -t rq-zed -p                 # confirm no warning notice
tmux send-keys -t rq-zed -l "g"
tmux send-keys -t rq-zed -l " "                # g<Space> at cursor line 1
tmux send-keys -t rq-zed -l "jjjjj"             # move cursor to line 5
tmux send-keys -t rq-zed -l "g"
tmux send-keys -t rq-zed -l " "                # g<Space> again at line 5
cat <scratch>/zed-wrapper.log
tmux capture-pane -t rq-zed -p                 # confirm the TUI resumed cleanly
```

`<scratch>/demo2-repo` is a throwaway git repo (`git init`, one commit, one
uncommitted edit to `main.rs`) created solely for this capture.

**Result summary:** the config loads with **zero warnings** (a valid
preset produces no notice). The wrapper log shows two invocations:

```
zed invoked with argv: [main.rs:1]
zed invoked with argv: [main.rs:5]
```

— `main.rs:1` for the first `g<Space>` (cursor on the file header, line 1)
and `main.rs:5` after moving the cursor down to line 5, both exactly one
argv token (`{{filename}}:{{line}}` substituted into a single mixed token,
per the `zed` preset's template), and the program name is `zed` in both
cases — matching `preset_template("zed") == "zed {{filename}}:{{line}}"`
and its dedicated unit test (`preset_zed_expands_correctly`) exactly. Both
times the wrapper returned immediately (`exit 0`) and the TUI resumed
cleanly (`restore_terminal`/`init_terminal`/`refresh` all completed without
error), rendering the same diff afterward with no crash or hang — proving
the suspend/resume contract around editor launch is unchanged.

**Evidence** (full capture at `proofs/2-zed-preset.txt`):

```
┌main.rs───────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│  ▾ M main.rs                                                                                                                             │
│  @@ -1,4 +1,5 @@                                                                                                                         │
│    1   1  fn main() {                                                                                                                    │
│    2   2      println!("hello world");                                                                                                   │
│    3     -    println!("this line will change");                                                                                         │
│        3 +    println!("this line changed for the demo");                                                                                │
│        4 +    println!("and this is a new line");                                                                                        │
│    4   5  }                                                                                                                              │
└──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
 j/k move · ] hunk · za fold · Space stage hunk · S stage file · c comment · / search · ` git panel · ? help

zed invoked with argv: [main.rs:1]
zed invoked with argv: [main.rs:5]
```

---

## `proofs/2-unknown-preset-fallthrough.txt`

**What it proves:** `[editor] preset = "notareallpreset"` (a name outside
the built-in table) is an invalid value: the app starts normally, the diff
renders unblocked, and the status-line notice names the exact section/key/
value at fault (`[editor] preset: unknown preset "notareallpreset"`) —
reusing the same warning surface Unit 1 built, not a separate mechanism.

**Why it matters:** this is the task's User Demo's second half ("Set an
unknown preset — the warning notice explains it and `g<Space>` falls back
to `$VISUAL`/`$EDITOR` behavior") and the spec's "unknown preset name is an
invalid value (warning + fall through)" FR.

**Method:** same `tmux`/`env -i`/`XDG_CONFIG_HOME` approach as above, config
containing `[editor]\npreset = "notareallpreset"\n`.

**Result summary:** the footer shows
`config: [editor] preset: unknown preset "notareallpreset" (! to dismiss)`,
naming both the offending key and value; the diff renders normally
underneath, unblocked. The fallthrough half of this demo (`g<Space>`
actually invoking `$VISUAL`/`$EDITOR`/`"nvim"`) is intentionally **not**
re-driven live here, to avoid spawning a real editor process against this
sandboxed `HOME`/`PATH` combination inside the capture — it's covered
instead by `main.rs`'s unit tests over `resolve_editor` (e.g.
`nvim_is_the_final_fallback`, and the whole chain with
`EditorConfigTier::Absent` standing in for exactly what
`run_tui` passes when `resolve_editor_config_tier` reports
`UnknownPreset`).

**Evidence** (full capture at `proofs/2-unknown-preset-fallthrough.txt`):

```
┌main.rs───────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│  ▾ M main.rs                                                                                                                             │
│  @@ -1,4 +1,5 @@                                                                                                                         │
│    1   1  fn main() {                                                                                                                    │
│    2   2      println!("hello world");                                                                                                   │
│    3     -    println!("this line will change");                                                                                         │
│        3 +    println!("this line changed for the demo");                                                                                │
│        4 +    println!("and this is a new line");                                                                                        │
│    4   5  }                                                                                                                              │
└──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
 config: [editor] preset: unknown preset "notareallpreset" (! to dismiss)
```

---

## `cargo test editor::` / `cargo test config::tests::editor` — the Unit 2 unit tests

`src/ui/editor.rs` + `src/ui/editor_tests.rs` (template engine, task 2.1;
preset table, task 2.2; config-tier resolution, tasks 2.3/2.4) and
`src/config/mod_tests.rs` (`EditorConfig` deserialization, task 2.3) cover:
template substitution (spaces in filenames surviving as one argv element,
a template with no `{{line}}`, mixed literal+placeholder tokens,
placeholder ordering, missing-`{{filename}}` rejection, empty-string
rejection); every one of the eleven presets' exact argv for a sample
file/line; unknown preset name rejection; the config-tier resolver
(`resolve_editor_config_tier`) for absent/preset-alone/`edit_at_line`-
alone/both-set-explicit-wins/unknown-preset; `EditorLaunch`'s default; and
`EditorConfig`'s own parse-time validation (partial override, both fields
independently, `edit_at_line` missing `{{filename}}`, wrong TOML types,
unknown key within the section, non-table section value).

```
$ cargo test --lib editor::
running 30 tests
test ui::editor::tests::absent_when_neither_field_is_set ... ok
test ui::editor::tests::edit_at_line_alone_resolves_to_itself ... ok
test ui::editor::tests::editor_launch_default_matches_todays_nvim_fallback ... ok
test ui::editor::tests::codium_special_cases_like_code ... ok
test ui::editor::tests::code_with_leading_args_uses_goto_flag ... ok
test ui::editor::tests::empty_editor_string_falls_back_to_nvim ... ok
test ui::editor::tests::explicit_edit_at_line_wins_over_preset_when_both_set ... ok
test ui::editor::tests::full_path_to_code_binary_still_special_cases_by_basename ... ok
test ui::editor::tests::nvim_uses_plus_line_convention ... ok
test ui::editor::tests::preset_alone_resolves_to_its_template ... ok
test ui::editor::tests::preset_helix_expands_correctly ... ok
test ui::editor::tests::preset_emacs_expands_correctly ... ok
test ui::editor::tests::preset_kakoune_expands_correctly ... ok
test ui::editor::tests::preset_micro_expands_correctly ... ok
test ui::editor::tests::preset_nano_expands_correctly ... ok
test ui::editor::tests::preset_nvim_expands_correctly ... ok
test ui::editor::tests::preset_sublime_expands_correctly ... ok
test ui::editor::tests::preset_vim_expands_correctly ... ok
test ui::editor::tests::preset_vscode_expands_correctly ... ok
test ui::editor::tests::preset_vscodium_expands_correctly ... ok
test ui::editor::tests::preset_zed_expands_correctly ... ok
test ui::editor::tests::template_empty_string_is_rejected ... ok
test ui::editor::tests::template_filename_with_spaces_survives_as_one_argv_element ... ok
test ui::editor::tests::template_missing_filename_placeholder_is_rejected ... ok
test ui::editor::tests::template_mixed_literal_and_placeholder_tokens ... ok
test ui::editor::tests::template_placeholder_ordering_line_before_filename ... ok
test ui::editor::tests::template_without_line_placeholder_ignores_line ... ok
test ui::editor::tests::unknown_preset_name_is_rejected ... ok
test ui::editor::tests::unknown_preset_name_is_reported_not_silently_dropped ... ok
test ui::editor::tests::whitespace_only_editor_string_falls_back_to_nvim ... ok

test result: ok. 30 passed; 0 failed; 0 ignored; 0 measured; 1161 filtered out; finished in 0.00s

$ cargo test --lib config::tests::editor
running 8 tests
test config::tests::editor_section_both_fields_set_deserialize_independently ... ok
test config::tests::editor_edit_at_line_wrong_type_is_an_invalid_value ... ok
test config::tests::editor_non_table_section_value_is_an_invalid_value ... ok
test config::tests::editor_edit_at_line_missing_filename_placeholder_is_an_invalid_value ... ok
test config::tests::editor_preset_wrong_type_is_an_invalid_value ... ok
test config::tests::editor_section_edit_at_line_with_filename_placeholder_applies ... ok
test config::tests::editor_unknown_key_within_the_section_is_collected_not_fatal ... ok
test config::tests::editor_section_partial_override_preset_only ... ok

test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 1183 filtered out; finished in 0.00s
```

`src/main.rs`'s `resolve_editor` test module also grew from 7 to 9 tests
for this task, covering the full five-tier chain (config-tier wins over
`$VISUAL`/`$EDITOR`, flag still wins over config-tier, and every existing
empty/whitespace-env-var edge case preserved unchanged):

```
$ cargo test --bin redquill
running 9 tests
test tests::config_template_wins_when_no_flag ... ok
test tests::editor_env_wins_when_no_flag_config_tier_or_visual ... ok
test tests::empty_editor_env_falls_through_to_nvim ... ok
test tests::empty_flag_falls_through_to_config_tier ... ok
test tests::empty_visual_falls_through_to_editor_env ... ok
test tests::empty_flag_and_absent_config_tier_falls_through_to_visual ... ok
test tests::flag_wins_over_everything ... ok
test tests::nvim_is_the_final_fallback ... ok
test tests::visual_wins_when_no_flag_or_config_tier ... ok

test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

The full library test suite remains green throughout, including the
pre-existing `config::load::tests::example_config_toml_parses_with_zero_warnings`
drift guard, which still passes with the new `[editor]` section added to
`docs/example-config.toml`:

```
$ cargo test --lib
test result: ok. 1188 passed; 0 failed; 3 ignored; 0 measured; 0 filtered out; finished in 4.86s
```

---

## Four-gate transcript

Full transcript at `proofs/2-gates.txt`; excerpted:

```
$ cargo build
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.14s

$ cargo test 2>&1 | tail -20
test switch_branch_fails_on_conflicting_dirty_tree_with_stderr ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.13s

     Running tests/lsp_integration.rs (target/debug/deps/lsp_integration-0fc3511744377b42)

running 4 tests
test second_file_gets_its_own_did_open ... ok
test definition_references_hover_flow_and_did_open_once ... ok
test shutdown_terminates_the_server ... ok
test unanswered_request_times_out_to_failed ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.27s

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

- **Where "unknown preset" is validated.** Task 2.3 says invalid entries
  flow through the warning contract "(two-pass parse: extend the
  known-keys walk in `from_table` for the new section)". `EditorConfig::
  from_value` (`src/config/mod.rs`) does exactly that for everything it
  *can* validate without new dependencies: `preset`/`edit_at_line` must be
  strings, and `edit_at_line` must contain `{{filename}}`. It does **not**
  validate that a `preset` *name* is one of the eleven built-ins there,
  because that requires the preset table, which — per this same task's
  instruction and the module's own documented layering rule
  ("`crate::config` must never import `crate::ui`") — lives in
  `src/ui/editor.rs` alongside the rest of the template/spawn machinery.
  Instead, `ui::editor::resolve_editor_config_tier` (which *may* depend on
  `crate::config`, the allowed direction) resolves `preset` against the
  table and returns `EditorConfigTier::UnknownPreset(name)` for a miss;
  `main::run_tui` folds that into the *same* `Vec<ConfigWarning>`
  `config::load` produced, before handing it to `App::set_config` — so the
  warning still reaches the identical UI surface, just assembled in two
  places instead of one. This is a deliberate layering trade-off, not a
  scope cut; it's called out here so a reviewer checking "did 2.3 really
  extend `from_table`" can see why the preset-name check isn't there too.
- **`App.editor`'s type changed** from `String` to a new `EditorLaunch`
  enum (`Template(String)` / `Command(String)`), since a config template
  must bypass `build_editor_command`'s family heuristic entirely (it's a
  full argv rule already) while `--editor`/`$VISUAL`/`$EDITOR`/`"nvim"`
  still need that heuristic applied. This was necessary to satisfy the
  FR ("non-config tiers keep the existing family heuristic") correctly;
  flagged here since it's a public-field type change on `App`, not just
  additive.
- **`Config` is no longer `Copy`** (only `Clone`), since `EditorConfig`
  owns `String`s. Verified no call site relied on `Config`-by-value copy
  semantics (every existing read site accesses a `Copy` sub-field, e.g.
  `app.config.search`, `app.config.layout.sidebar_side`) — all pass
  unchanged.

## Reviewer Conclusion

Unit 2 is implemented and proven end to end: the template engine
(`build_from_template`) substitutes `{{filename}}`/`{{line}}` strictly
per-token, after splitting, so multi-word filenames survive intact and
never touch a shell; the eleven-preset table each has an individually
asserted argv test; `EditorConfig` is added to `Config` as plain
partial-override data with `edit_at_line`-wins-over-`preset` resolved (and
tested) at the point that already owns the preset table for layering
reasons; the five-tier precedence is wired into `main.rs`'s `resolve_editor`
with the pre-existing empty/whitespace-env-var behavior preserved
unchanged and unit-tested; `docs/example-config.toml` gained a fully
annotated `[editor]` section without breaking its own zero-warnings drift
test; and a live TUI transcript (via a logging fake `zed` binary and tmux)
proves an editor entirely outside the old two hardcoded families opens at
the exact cursor line, with the unknown-preset case shown to degrade to a
visible, dismissible warning rather than blocking startup. All four repo
gates are green.

Parent task 2.0 is ready to be marked complete.
