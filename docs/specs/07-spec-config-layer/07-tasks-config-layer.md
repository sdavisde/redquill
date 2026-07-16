# 07-tasks-config-layer.md

Task list for `07-spec-config-layer.md`. Tasks are sequenced as vertical slices: every parent task ends with something a user can do in the running app that they couldn't do before, plus the documentation to do it. Proof artifacts are persisted under this spec directory's gitignored `proofs/` folder unless they are tests (which live with the code).

Each task's **User demo** is the acceptance ritual between stages: run it as written before checking the task off.

## Relevant Files

| File | Why It Is Relevant |
| --- | --- |
| `Cargo.toml` | New dependencies: `toml`, path-discovery crate (`etcetera` or none), `crokey`; each justified in its commit. |
| `src/config/mod.rs` (new) | `Config` root struct, per-section structs, module doc with degradation contract + extensibility walkthrough. |
| `src/config/load.rs` (new) | Path discovery, file read, TOML parse, warning collection (typed errors via `thiserror`). |
| `src/config/keys.rs` (new) | Key-string grammar, action-name mapping, per-action merge semantics for main + modal keymaps. |
| `src/config/*_tests.rs` (new) | Split test modules per repo convention (`#[cfg(test)] #[path]`) for load/sections/keys. |
| `src/main.rs` | Load config at startup before first render; resolve the CLI-struct/`Config` name collision; editor precedence wiring. |
| `src/ui/app.rs` | `App` holds the loaded config + warning notices; startup application of search defaults. |
| `src/ui/mod.rs` | `split_layout`/`sidebar_width` take configured side/width instead of computed defaults. |
| `src/ui/editor.rs` | Template substitution + preset table replace the hardcoded two-family arg-style rule. |
| `src/ui/keymap.rs` | Effective-keymap construction: `default_map()` + config overrides; action-name drift test. |
| `src/ui/modal_keys.rs` | `const` tables → runtime-built tables (refactor), then config overrides (behavior). |
| `src/ui/help.rs` | Verify-only: help renders effective bindings (derived, should need no logic change) + test with overrides. |
| `src/search/query.rs` | Search option types consumed by `[search]` startup defaults. |
| `src/lsp/config.rs` | `default_commands()`/`ServerLang`/`LangServerCmd`; stays free of TUI/config types (receives plain data only). |
| `src/ui/lsp_config.rs` (new) | `effective_lsp_commands`: the `[lsp]`-onto-`default_commands()` overlay, at the edge module (like `ui::editor`'s preset table) since it's the one place allowed to import both `crate::config` and `crate::lsp`. |
| `docs/example-config.toml` (new) | Annotated example grown one section per slice; drift-tested in 6.0. |
| `README.md` | Configuration section; remove stale "config layer planned" line. |
| `docs/config-layer.md` | Retired (deleted or replaced with a pointer to spec 07). |

### Notes

- Gates before every commit: `cargo build && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check` (pre-push hook enforces the same).
- TDD for the pure parts: write the failing test first for path discovery, deserialization, merge semantics, template substitution, key grammar. Tests commit with the code.
- Tests must never touch the developer's real `~/.config/redquill/config.toml`: path discovery is injectable (env override / parameter), integration tests use `tempfile` with canonicalized paths (macOS `/var` symlink).
- Conventional commits; the 5.1 refactor and 5.2+ behavior changes land in separate commits; move-only refactors state the identical-test-count invariant in the commit message.
- Sub-tasks within a parent are ordered; parents 2, 3, 4 are independent of each other but all depend on 1; 5 depends on 4; 6 depends on all.

## Tasks

### [x] 1.0 Config file moves the sidebar (loading infrastructure + layout + search defaults)

Establish the end-to-end config pipeline — path discovery (`$XDG_CONFIG_HOME`/`~/.config/redquill/config.toml`, `~/.config` on macOS too), one-shot startup load, serde-default partial-override `Config` struct, the documented degradation contract (missing = silent; syntax error = defaults + visible warning; invalid key/value = partial apply + warning), the dismissible non-blocking warning surface — proven through its first visible consumers: `[layout]` (sidebar side/width) and `[search]` (case/whole-word/literal startup defaults). Starts the annotated `docs/example-config.toml` with the `[layout]` and `[search]` sections.

**User demo:** create `~/.config/redquill/config.toml` containing `[layout]` `sidebar_side = "left"`, launch redquill, open the git panel — the sidebar is on the left. Break the file (delete a quote), relaunch — the app works on defaults and a status notice names the file and the parse error. Delete the file — behavior is exactly today's, no notice.

#### 1.0 Proof Artifact(s)

- Test: `cargo test config` — unit tests for path discovery (XDG override, fallback, missing home, test-injectable path), each degradation case, and `[layout]`/`[search]` deserialization demonstrate the Unit 1 FRs.
- Screenshot: `proofs/1-sidebar-left.png` — app running with `sidebar_side = "left"` + fixed width from the demo config demonstrates config reaches rendering.
- Screenshot: `proofs/1-malformed-warning.png` — app running behind a visible warning naming the file path and TOML parse error, fully usable, demonstrates the degradation contract.
- CLI: `proofs/1-no-config-identical.txt` — full test suite passing with no config file present demonstrates the no-config invariant.
- Diff: `docs/example-config.toml` created with annotated `[layout]`/`[search]` sections demonstrates docs ship with the slice.

#### 1.0 Tasks

- [x] 1.1 Add the `toml` dependency (`default-features = false` if workable) and decide the path-discovery approach: evaluate `etcetera` vs a small hand-rolled resolver; requirement either way is `$XDG_CONFIG_HOME/redquill/config.toml` else `~/.config/redquill/config.toml` on Linux **and macOS**, platform config dir on Windows (`directories` is ruled out — wrong macOS answer). Record the justification for the commit message.
- [x] 1.2 TDD: write failing tests, then implement config-path discovery as a pure function taking injected env/home values (cases: XDG set, XDG unset → `~/.config`, missing home → no path, explicit test-override hook used by integration tests). Location: `src/config/load.rs`.
- [x] 1.3 TDD: define `Config` with per-section structs (`LayoutConfig`, `SearchConfig` now; sections for editor/lsp/keys arrive in their slices) — every field `#[serde(default)]` defaulting to current shipped behavior. Tests: empty file → all defaults; partial file overrides only named keys; unknown keys are collected (not fatal); invalid value for a known key falls back to default and is collected. Location: `src/config/mod.rs`.
- [x] 1.4 TDD: implement `load() -> (Config, Vec<ConfigWarning>)` with typed errors (`thiserror`) internally: missing file → defaults, zero warnings; TOML syntax error → full defaults + warning carrying path and parser line info; parseable-but-invalid entries → partial apply + one warning each. Assert stdout is never written.
- [x] 1.5 Wire loading into `src/main.rs` before terminal/App setup, exactly once (no reload path exists); resolve the existing CLI-struct name collision (rename the clap struct to `Cli`-style or namespace the config type); hand `Config` + warnings to `App`.
- [x] 1.6 Implement the warning notice in the UI: dismissible, non-blocking status-line notice showing the first problem + "and N more"; renders over no content that blocks review; test render + dismiss; never printed to stdout.
- [x] 1.7 Apply `[layout]`: `sidebar_side = "left" | "right"` (default right) and `sidebar_width` columns override the 30%-clamp formula only when set; validate width against the documented range (out-of-range → warning + default) and clamp to available terminal width at render. Update `split_layout`/`sidebar_width` signatures; unit-test both paths including "unset preserves today's formula exactly".
- [x] 1.8 Apply `[search]`: `case = "smart" | "sensitive" | "insensitive"`, `whole_word`, `literal` set the Project Search startup state; in-session toggles unaffected. Unit-test mapping config → `CaseMode`/flags.
- [x] 1.9 Create `docs/example-config.toml` with fully annotated `[layout]` and `[search]` sections (every key, allowed values, defaults stated).
- [x] 1.10 Run the User demo; capture `proofs/1-sidebar-left.png`, `proofs/1-malformed-warning.png`, `proofs/1-no-config-identical.txt`; run all four gates; commit (deps commit may precede feature commit).

### [x] 2.0 Config file picks your editor (`[editor]` templating + presets)

Add `[editor]` config: `edit_at_line` template with `{{filename}}`/`{{line}}` placeholders (argv-token substitution, never a shell) and `preset` with the built-in table (vim, nvim, helix, vscode, vscodium, zed, emacs, nano, micro, sublime, kakoune). Wire into the resolution precedence `--editor` > config > `$VISUAL` > `$EDITOR` > `nvim`, keeping the family heuristic for non-config tiers; replaces the hardcoded two-family special case in `src/ui/editor.rs` with data. Adds the `[editor]` section (with the full preset list) to `docs/example-config.toml`.

**User demo:** add `[editor]` `preset = "zed"` (or your editor) to the config, put the cursor on any diff line, press `g<Space>` — your editor opens at that exact line. Set an unknown preset — the warning notice explains it and `g<Space>` falls back to `$VISUAL`/`$EDITOR` behavior.

#### 2.0 Proof Artifact(s)

- Test: `cargo test editor` — unit tests for template substitution (spaces in filenames, missing `{{line}}`, token ordering), every preset expansion, unknown-preset fallthrough warning, and the five-tier precedence chain demonstrate the Unit 2 FRs.
- CLI: `proofs/2-zed-preset.txt` — transcript showing `g<Space>` with `preset = "zed"` spawning `zed <file>:<line>` (via logging wrapper) demonstrates an editor outside the old hardcoded families opens at the cursor line.
- Diff: `docs/example-config.toml` gains the annotated `[editor]` section with the preset list demonstrates docs ship with the slice.

#### 2.0 Tasks

- [x] 2.1 TDD: template engine in `src/ui/editor.rs` — split template into whitespace tokens, substitute `{{filename}}`/`{{line}}` per-token (a token may mix literal + placeholder, e.g. `{{filename}}:{{line}}`); filenames with spaces survive because substitution happens after tokenization, never through a shell. Validation: template without `{{filename}}` is an invalid value (warning + tier fallthrough). Failing tests first: spaces, no-`{{line}}` template, mixed tokens, missing-`{{filename}}` rejection.
- [x] 2.2 TDD: preset table as `const` data mapping the eleven names → known-correct `edit_at_line` templates; one test per preset asserting the exact argv for a sample file/line; unknown preset name → invalid value (warning + fallthrough).
- [x] 2.3 Add `EditorConfig` (`preset`, `edit_at_line`; explicit `edit_at_line` wins when both set — tested) to `Config`; invalid entries flow through the 1.x warning contract.
- [x] 2.4 Wire the five-tier precedence in `src/main.rs`/`resolve_editor`: `--editor` flag > config template-or-preset > `$VISUAL` > `$EDITOR` > `nvim`; non-config tiers keep the existing family heuristic (VS Code family `--goto file:line`, else `+line`). Unit-test the full chain including empty/whitespace env vars (existing behavior preserved).
- [x] 2.5 Add the annotated `[editor]` section (template syntax, placeholder rules, full preset list) to `docs/example-config.toml`.
- [x] 2.6 Run the User demo with a logging wrapper named `zed` on PATH; capture `proofs/2-zed-preset.txt`; gates; commit.

### [x] 3.0 Config file controls code intelligence (`[lsp]` overrides)

Add `[lsp.rust|typescript|python|go]` tables (`command`, `args`, `enabled`) overlaid onto `default_commands()`; `enabled = false` prevents spawn with today's silent degradation; type errors in the section follow the warning contract. Config structs stay edge-side — `lsp/` receives plain `LangServerCmd`-shaped data. Adds the `[lsp]` section to `docs/example-config.toml`.

**User demo:** point `[lsp.rust] command` at a wrapper script (or alternate rust-analyzer path), press `gd` on a symbol — definition peek works through your configured server. Set `enabled = false`, relaunch — LSP keys degrade exactly like an unconfigured server, everything else works.

#### 3.0 Proof Artifact(s)

- Test: `cargo test lsp` — unit tests for overlay merge (override one language, disable another, defaults intact for the rest) demonstrate the Unit 3 FRs.
- CLI: `proofs/3-lsp-override.txt` — transcript of a `gd` peek working through a logging wrapper configured as `[lsp.rust] command`, wrapper log proving the configured command ran, demonstrates overrides drive the real LSP lifecycle.
- Diff: `docs/example-config.toml` gains the annotated `[lsp]` section demonstrates docs ship with the slice.

#### 3.0 Tasks

- [x] 3.1 TDD: `LspConfig` section — per-language tables (`rust`, `typescript`, `python`, `go`) each `{ command: Option<String>, args: Option<Vec<String>>, enabled: bool = true }`; a merge function overlaying it onto `default_commands()` producing the effective `HashMap<ServerLang, LangServerCmd>` minus disabled languages. Failing tests first: override one, disable one, others default; `args` without `command` overrides args only.
- [x] 3.2 Wire in `src/main.rs`/app setup: effective map (plain `LangServerCmd` data, no config types) handed to the LSP layer; `enabled = false` means the language is absent from the map → existing missing-server silent degradation. Verify `src/lsp/` gains no `config`/TUI imports (grep check noted in commit).
- [x] 3.3 Add the annotated `[lsp]` section to `docs/example-config.toml` (all four language tables, `enabled` semantics, degradation note).
- [x] 3.4 Run the User demo with a logging wrapper as `[lsp.rust] command`; capture `proofs/3-lsp-override.txt` including the wrapper log; gates; commit.

### [x] 4.0 Config file remaps the main keymap (`[keys.diff]`, `[keys.panel]`)

Make the main `Keymap` table remappable: kebab-case action-name mapping over the `Action` enum with a bijectivity drift test; key-string grammar (crokey-style chords, space-separated two-chord sequences); per-action merge semantics (config replaces that action's keys; unlisted actions keep defaults; `[]` unbinds; collisions resolve user-wins + warning); help overlay and footer render effective bindings. Adds `[keys.diff]`/`[keys.panel]` with the complete action-name list to `docs/example-config.toml`. Perf tripwires unchanged.

**User demo:** add `[keys.diff]` with `next-file = "J"`, `quit = ["q", "ctrl-c"]`, and unbind something (`toggle-collapse = []`); relaunch — `J` jumps files, both quit keys work, the unbound key does nothing, and `?` shows exactly these bindings. Add a nonsense key string — the warning notice names the bad entry and the default stays active.

#### 4.0 Proof Artifact(s)

- Test: `cargo test keymap` — action-name drift test (every `Action` variant exactly one name, every name resolves back), key-grammar parse/reject tests, and merge-semantics tests (replace/keep/unbind/collision) demonstrate the Unit 4 main-keymap FRs.
- Screenshot: `proofs/4-remap-help.png` — `?` overlay showing a remapped action and an unbound action reflected demonstrates help stays truthful automatically.
- CLI: `proofs/4-perf-tripwires.txt` — perf tripwire tests passing unchanged demonstrates no dispatch regression.
- Diff: `docs/example-config.toml` gains annotated `[keys.diff]`/`[keys.panel]` sections with the full action list demonstrates docs ship with the slice.

#### 4.0 Tasks

- [x] 4.1 Spike (timeboxed): verify `crokey` parses the needed chord notation and converts cleanly to `KeyChord { KeyCode, KeyModifiers }`, and confirm how two-chord sequences ("g d") layer on top (crokey parses single combinations; the sequence split is ours). Decide crokey vs hand-rolled parser mirroring its notation; record justification for the dependency commit. Acceptance: a written note in the PR/commit + the chosen path compiles with a round-trip test.
- [x] 4.2 TDD: action-name mapping in `src/config/keys.rs` — kebab-case name per `Action` variant via a single total table; drift test asserts bijectivity (every variant named exactly once, every name resolves back) so a new `Action` variant fails the build's tests until named.
- [x] 4.3 TDD: key-string grammar — single chords (`"a"`, `"ctrl-k"`, `"alt-enter"`, `"shift-tab"`, `"f5"`, `"esc"`), space-separated two-chord sequences (`"g d"`), max two chords (three+ rejected); parse-reject tests for garbage; a consistency test that grammar output formats agree with `KeyChord::label()` rendering so config notation and help display can't drift.
- [x] 4.4 TDD: merge semantics — `apply_overrides(default_map(), overrides) -> Vec<Binding>`: an action named in config gets exactly the listed keys (defaults for it dropped); unlisted actions untouched; `[]` unbinds; same-scope collision → user binding wins, colliding default dropped, collision recorded as warning. Failing tests for each case first.
- [x] 4.5 Add `[keys.diff]`/`[keys.panel]` deserialization (`action-name = "key"` or `= ["key", ...]`) to `Config`; unknown action names and unparseable key strings are invalid values (entry ignored + warning). Wire effective-keymap construction at startup in place of bare `default_map()`.
- [x] 4.6 Verify help/footer truthfulness: add a test that builds a keymap with an override + an unbind and asserts the help model reflects both (the overlay already derives from `Keymap::bindings()`; this pins it).
- [x] 4.7 Run the perf tripwire tests unchanged (`src/ui/perf_tests.rs`) and capture the run; add annotated `[keys.diff]`/`[keys.panel]` sections to `docs/example-config.toml` including the complete generated action-name list.
- [x] 4.8 Run the User demo; capture `proofs/4-remap-help.png` and `proofs/4-perf-tripwires.txt`; gates; commit (dependency commit separate if crokey chosen).

### [x] 5.0 Config file remaps every modal panel (`[keys.staging]`, `[keys.switcher]`, ...)

Two separately-committed halves per repo rules: (refactor) convert the `const` tables in `src/ui/modal_keys.rs` to runtime-built tables from the same default data — move-only invariant, identical test counts, zero assertion edits; (behavior) apply `[keys.<mode>]` overrides with task 4's merge semantics, preserving the bidirectional drift tests and keeping free-text character insertion non-remappable. Completes `docs/example-config.toml` with every modal `[keys.*]` table and its action names.

**User demo:** remap a staging-panel key (e.g. stage-line to `x`) and a switcher key in config; relaunch — the new keys act in their panels, the old ones don't, and each panel's hint line plus `?` show the remapped keys. Main-keymap remaps from stage 4 still work alongside.

#### 5.0 Proof Artifact(s)

- Test: `cargo test modal_keys` — preserved bidirectional drift suite passing post-refactor and post-override demonstrates the Unit 4 modal FRs; refactor commit message records identical test counts.
- Screenshot: `proofs/5-modal-remap.png` — a modal panel with its remapped key visible in the hint line plus the `?` overlay demonstrates modal remapping reaches the UI.
- Diff: `docs/example-config.toml` completed with all modal `[keys.*]` tables demonstrates docs ship with the slice.

#### 5.0 Tasks

- [x] 5.1 Refactor commit (no behavior change): convert each `const` table in `src/ui/modal_keys.rs` into a runtime-built default table (same rows, constructed once at startup or lazily); handlers and hint rendering consume the built tables. Invariant verified and stated in the commit message: identical test counts, zero assertion edits, all drift tests green before/after.
- [x] 5.2 Enumerate the shipped modal modes into their `[keys.<mode>]` table names (staging, switcher, finder, help, compose, search, plus any others present — e.g. peek, annotation list, project-search); extend action-naming (4.2's pattern, per-mode enums) with the same bijectivity drift tests; document the final name list.
- [x] 5.3 TDD: apply 4.4's merge semantics per modal table from `[keys.<mode>]` config; free-text modes expose only their documented control keys as actions — character insertion is not an action and cannot be bound; the reverse drift tests ("undocumented keys observably do nothing") still pass with overrides applied.
- [x] 5.4 Wire modal overrides into table construction; hint lines and `?` reflect effective keys (derived from the same tables — pin with one test per pattern, not per mode).
- [x] 5.5 Complete `docs/example-config.toml` with every modal `[keys.*]` table and its full action-name list.
- [x] 5.6 Run the User demo; capture `proofs/5-modal-remap.png`; gates; behavior commit (separate from 5.1's refactor commit). **Live tmux demo skipped by user decision** (2026-07-16) — automated test suite (modal drift/bijectivity tests, merge-semantics tests, hint/help pinning tests, example-config completeness drift test) stands as the proof instead; see `07-proofs/07-task-05-proofs.md`.

### [!] 6.0 A new user can adopt the whole system from the docs (README, retirement, acceptance journeys)

> **SKIPPED — user decision, 2026-07-16.** The user descoped this entire parent task during implementation ("i don't think we need to do #6 at all"). Consequences accepted with the descope: the README keeps its stale "config layer planned" line, `docs/config-layer.md` is not retired, the `src/config/` extensibility walkthrough (spec success metric 5) is unwritten, and the spec's three acceptance journeys have no persisted evidence. A partial example-config drift test shipped early (task 1.0) and was kept green through tasks 2–5, so 6.1's core guard exists in reduced form.

THIS TASK IS BLOCKED, DO NOT START WORK ON THIS TASK UNDER ANY CIRCUMSTANCES. I DO NOT KNOW WHETHER OR NOT WE ACTUALLY WANT TO DO THIS WORK. THERE IS ALSO CHANGES TO THE README IN MAIN THAT WE'D NEED TO WORK AGAINST, AND THESE CHANGES GO AGAINST MY INTENTIONS FOR THE README RIGHT NOW.

The final slice is the adoption experience itself: README gains a Configuration section (file location, quick example, pointer to `docs/example-config.toml`) and drops the stale "config layer planned" line; `docs/config-layer.md` is deleted or replaced with a pointer to spec 07; the `src/config/` module doc gets the extensibility walkthrough (exact steps to add a hypothetical `[theme]` section touching no loader code — the spec's success metric 5); a drift-style test asserts every action name and key string in `docs/example-config.toml` actually parses and resolves, so the example can't rot; and the spec's three user-journey acceptance tasks run against a real build with persisted evidence.

**User demo:** starting from only the README on a machine with no config file, follow the docs to: set your editor and open a file at a line, remap a key, and recover from a deliberate typo — without reading any source code. This is the spec's acceptance ritual, evidence persisted.

#### 6.0 Proof Artifact(s)

- Diff: README Configuration section + `docs/config-layer.md` retirement demonstrates docs track shipped behavior (docs-as-contract rule).
- Test: example-config drift test (every documented action name/key string parses and resolves) demonstrates the docs cannot silently rot.
- Screenshot/CLI: `proofs/6-journey-editor.txt`, `proofs/6-journey-remap.png`, `proofs/6-journey-badconfig.png` — the three acceptance journeys (Zed preset opens at line; remap+unbind reflected after restart; typo'd config still reviews with readable warning) demonstrate the spec's success metrics with persisted evidence.
- CLI: `proofs/6-gates.txt` — `cargo build && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check` all passing on the final tree demonstrates repo gates hold.

#### 6.0 Tasks

- [-] 6.1 Drift test: a unit test loads `docs/example-config.toml` through the real `load()` path (via `include_str!` or repo-relative path) and asserts zero warnings — every section, action name, key string, and preset in the example parses and resolves. This is the guard that keeps the example truthful forever.
- [-] 6.2 README: add a Configuration section (file location incl. macOS note, a 5-10 line quick example, pointer to `docs/example-config.toml`); update line ~50 to stop saying the config layer is "planned".
- [-] 6.3 Retire `docs/config-layer.md`: delete it and fix all references (README, `src/ui/mod.rs` doc comment points at it) to reference spec 07 / the example config instead.
- [-] 6.4 Write the extensibility walkthrough in the `src/config/` module doc: the exact steps to add a hypothetical `[theme]` section (define section struct with defaults → add one `Config` field → consume it), explicitly noting zero changes to discovery/parse/warning code (spec success metric 5).
- [-] 6.5 Verify `proofs/` is covered by the existing gitignore pattern (add if this spec's dir isn't); run the three acceptance journeys from the spec's Success Metrics against a release-ish build; persist `proofs/6-journey-editor.txt`, `proofs/6-journey-remap.png`, `proofs/6-journey-badconfig.png`.
- [-] 6.6 Full-gate transcript to `proofs/6-gates.txt`; final commit; confirm no stray references to the old config-layer skeleton remain (`grep -r "config-layer" --include="*.rs" --include="*.md"` excluding the spec dir).
