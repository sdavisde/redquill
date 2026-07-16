# 07-spec-config-layer.md

## Introduction/Overview

redquill currently has no user-facing configuration: layout, editor integration, LSP servers, and every keybinding are compile-time defaults (the only runtime inputs are CLI flags and `$VISUAL`/`$EDITOR`). This spec introduces a configuration system backed by a single TOML file at `~/.config/redquill/config.toml`, read once at startup, that lets users customize layout, the external editor command, LSP server commands, and the complete keybinding surface — while the app continues to work identically for users with no config file. The system must be structured so future config sections (themes first) can be added without touching the loading machinery.

This spec grows the skeleton in `docs/config-layer.md` into the ratified design; that skeleton document is superseded by this spec once implementation lands.

## Goals

- A user can change sidebar layout, search defaults, external editor, LSP server commands, and any keybinding by editing one TOML file, with no rebuild.
- Zero behavior change for users with no config file: every default matches today's shipped behavior exactly.
- A malformed or partially invalid config never prevents the app from starting; the user always gets a visible explanation of what was ignored and why.
- The `?` help overlay and all key hints remain truthful after any remapping, enforced by the existing drift-test discipline.
- Adding a future config section (e.g. `[theme]`) requires only a new section struct and its wiring — no changes to discovery, parsing, or warning plumbing.

## User Stories

- **As a reviewer with vim muscle memory conflicts**, I want to rebind any redquill key (including panel and modal keys) so that the tool matches my hands instead of the other way around.
- **As a Zed/Sublime/Kakoune user**, I want to configure my editor command with a line-number template so that `g<Space>` opens my editor at the cursor line even though redquill's built-in defaults don't know my editor's syntax.
- **As a user with a nonstandard toolchain**, I want to override the LSP server command per language (or disable one) so that LSP peek works with my environment instead of silently degrading.
- **As a user with a wide monitor**, I want to set the sidebar's side and width so that the layout fits my screen and habits.
- **As a user who typo'd my config**, I want the app to start anyway and tell me what it couldn't apply so that a bad line never locks me out of reviewing code.
- **As a future contributor (human or agent)**, I want config sections to follow one obvious pattern so that adding theme support later is mechanical.

## Demoable Units of Work

### Unit 1: Config loading infrastructure + layout and search defaults

**Purpose:** Establish the whole config pipeline — file discovery, TOML parsing, defaults merging, and the warning contract — and prove it end-to-end with the simplest consumers: sidebar layout and search-option startup defaults.

**Functional Requirements:**

- The system shall look for a config file at `$XDG_CONFIG_HOME/redquill/config.toml`, falling back to `~/.config/redquill/config.toml`, on Linux and macOS (macOS deliberately uses `~/.config`, not `~/Library/Application Support`, matching helix/yazi user expectations); on Windows the platform config directory shall be used.
- The system shall read the config file exactly once, at startup, before the first render; there is no reload mechanism.
- When no config file exists, the system shall start silently with built-in defaults, and all behavior shall be byte-for-byte identical to current shipped behavior.
- When the file has a TOML syntax error, the system shall start with full defaults and display a visible, non-blocking in-app warning naming the file path and the parse error (including line number as reported by the parser).
- When the file parses but contains an unknown key or an invalid value for a known key, the system shall apply every valid setting, fall back to the default for each invalid one, and include each ignored key/value in the same in-app warning surface.
- The in-app warning shall be presented without blocking startup or input (e.g. a status-line notice), shall be dismissible, and shall not be written to stdout (stdout is reserved for the annotation format).
- The `Config` struct shall be composed of per-section structs (`[layout]`, `[search]`, `[editor]`, `[lsp]`, `[keys]`), each deserialized with serde defaults so users specify only the keys they want to change (partial override; never a full-file copy).
- The system shall support `[layout] sidebar_side = "left" | "right"` (default `right`) and `[layout] sidebar_width = <integer columns>`; when `sidebar_width` is unset, today's formula (30% of terminal width clamped to [40, 72]) shall remain the default. Configured widths shall be validated against a documented sane range and clamped at render time to available space; out-of-range values are treated as invalid values (warning + default).
- The system shall support `[search]` startup defaults for the existing Project Search toggles: `case = "smart" | "sensitive" | "insensitive"`, `whole_word = <bool>`, `literal = <bool>`; in-session toggles continue to work and are not written back.

**Proof Artifacts:**

- Test: unit tests for path discovery (env-var override, fallback, missing home) and for each warning-contract case (missing file silent; syntax error → defaults + warning; unknown key and invalid value → partial apply + warning) demonstrate the loading contract.
- CLI/screenshot (persisted under the spec's gitignored `proofs/` dir): app launched with a config setting `sidebar_side = "left"` and a fixed width, screenshot shows the moved/resized sidebar, demonstrating config reaches rendering.
- Screenshot: app launched with a deliberately malformed config shows the in-app warning naming the file and error, and the app is fully usable behind it, demonstrating the degradation contract.

### Unit 2: External editor templating and presets

**Purpose:** Let any editor work with `g<Space>` open-at-line by replacing the hardcoded two-family argument rule with a lazygit-style template plus a preset table.

**Functional Requirements:**

- The system shall support `[editor] edit_at_line = "<command template>"` where `{{filename}}` and `{{line}}` placeholders are substituted; `{{filename}}` is required in a template, `{{line}}` is optional.
- The system shall support `[editor] preset = "<name>"` with a built-in preset table covering at least: `vim`, `nvim`, `helix`, `vscode`, `vscodium`, `zed`, `emacs`, `nano`, `micro`, `sublime`, `kakoune`. Each preset expands to a known-correct `edit_at_line` template. An explicit `edit_at_line` overrides `preset` when both are set; an unknown preset name is an invalid value (warning + fall through to the next precedence tier).
- Editor resolution precedence shall be: `--editor` CLI flag > `[editor]` config (template or preset) > `$VISUAL` > `$EDITOR` > `nvim` default. Tiers below config keep today's family-based argument heuristic (VS Code family → `--goto file:line`, otherwise `+line`).
- Template substitution shall build the child process argv by splitting the template into tokens and substituting placeholders per-token — never via a shell (`sh -c` remains forbidden); filenames with spaces must survive substitution intact.
- The existing suspend/resume terminal handling around editor launch shall be unchanged.

**Proof Artifacts:**

- Test: unit tests over `build_editor_command` covering template substitution (spaces in filenames, `{{line}}` absent, placeholder ordering), each preset's expansion, unknown-preset fallthrough, and the full precedence chain demonstrate the resolution contract.
- CLI transcript (persisted under `proofs/`): with `preset = "zed"` in config, pressing `g<Space>` spawns `zed <file>:<line>` (captured via a logging fake or `ps` evidence), demonstrating an editor outside the old hardcoded families opens at the cursor line.

### Unit 3: LSP server overrides

**Purpose:** Make the hardcoded language-server table user-configurable so LSP peek works with nonstandard toolchains, and disableable per language.

**Functional Requirements:**

- The system shall support per-language tables for the four supported languages — `[lsp.rust]`, `[lsp.typescript]`, `[lsp.python]`, `[lsp.go]` — each accepting `command = "<string>"`, `args = ["<string>", ...]`, and `enabled = <bool>` (default `true`).
- A configured `command`/`args` shall replace that language's default server invocation; unspecified languages keep their defaults.
- `enabled = false` shall prevent the server from being spawned for that language; the UI shall degrade for that language exactly as it does today for a missing server (silent, per the documented LSP contract).
- A configured server binary that is missing or fails to start shall degrade exactly as today (silently); config errors in the `[lsp]` section itself (e.g. wrong type) follow the Unit 1 warning contract.

**Proof Artifacts:**

- Test: unit tests deserializing `[lsp]` overlays onto the default table (override one language, disable another, leave the rest) demonstrate merge semantics.
- CLI transcript (persisted under `proofs/`): with `[lsp.rust] command` pointed at a wrapper script that logs its invocation then execs `rust-analyzer`, a `gd` definition peek works and the log proves the configured command was used, demonstrating overrides drive the real LSP lifecycle.

### Unit 4: Full keybinding remapping (main keymap + modal tables)

**Purpose:** Make every binding in the app remappable from config — the main Normal/Visual/Panel keymap and all modal tables — while the `?` help overlay and per-mode hints stay truthful automatically.

**Functional Requirements:**

- The system shall accept keybinding overrides in per-scope TOML tables: `[keys.diff]` and `[keys.panel]` for the main keymap's two scopes, plus one table per modal mode currently defined in `modal_keys.rs` (e.g. `[keys.staging]`, `[keys.switcher]`, `[keys.finder]`, `[keys.help]`, `[keys.compose]`, `[keys.search]` — final table names enumerated during implementation to match the shipped mode set, and documented).
- Each entry shall map an action name to one key string or an array of key strings: `next-hunk = "]"` or `quit = ["q", "ctrl-c"]`. Action names shall be kebab-case, derived from the existing action enums by a single total mapping with a drift test guaranteeing every action variant has exactly one name and every name resolves back (no unmapped or duplicate names).
- Key strings shall be parsed by an established key-notation grammar (crokey-style: `"a"`, `"ctrl-k"`, `"alt-enter"`, `"shift-tab"`, `"f5"`, `"esc"`); two-key sequences shall be written space-separated (`"g d"`), capped at the existing two-chord maximum. Unparseable key strings and unknown action names are invalid values under the Unit 1 warning contract (that entry ignored, warning shown).
- Overrides shall merge per-action onto the defaults: an action named in config has exactly the listed keys (its default keys are replaced, not appended); actions not named keep their defaults; an empty array (`quit = []`) unbinds the action entirely.
- If a user binding collides with another binding in the same scope, the user binding shall win deterministically (the colliding default is dropped) and the collision shall be reported through the warning surface.
- The `const` modal tables in `modal_keys.rs` shall be refactored into runtime-built tables (defaults constructed from the current data, then config overrides applied) with their existing bidirectional drift tests preserved or equivalently adapted — every documented key must observably act, every undocumented key must observably not.
- Free-text input modes (compose, search, finder text entry) shall keep character insertion non-remappable; only their control keys (the entries already documented in their tables) are remappable.
- The `?` help overlay, footer hints, and modal hint lines shall render the *effective* (post-override) bindings with no additional wiring, because they already derive from the tables being overridden.
- Remapping shall not measurably regress input dispatch: the wall-clock tripwire tests in `src/ui/perf_tests.rs` shall pass unchanged (no loosened budgets).

**Proof Artifacts:**

- Test: drift tests for the action-name mapping (total, bijective), key-string parsing (valid grammar, sequences, rejects), merge semantics (replace/keep/unbind/collision), and the preserved modal drift suite demonstrate the remapping contract.
- Screenshot (persisted under `proofs/`): `?` help overlay after a config that remaps a main-keymap action, a modal action, and unbinds one action — showing all three reflected — demonstrates help/hints stay truthful automatically.
- CLI transcript: full gate run (`cargo build && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`) passing on the final tree demonstrates repo gates hold.

## Non-Goals (Out of Scope)

1. **Themes**: no `[theme]` section ships in this spec. The architecture must make adding it trivial (see Technical Considerations), and theming is the intended next spec.
2. **In-app settings UI**: explicitly withdrawn by the user for now; redquill displays no settings screen.
3. **Config writing**: redquill never writes, creates, or modifies the config file. No write-back, no `--init-config` generator (a documented example in the README/docs serves that purpose).
4. **Live reload**: no reload keybind, no file watcher; config is read once at startup. Changing config requires restart.
5. **Per-repository config overrides** (e.g. `<repo>/.git/redquill.toml`): global file only.
6. **Environment-variable overrides of config values** (beyond `$XDG_CONFIG_HOME` for the file's location and the existing `$VISUAL`/`$EDITOR` tiers).
7. **Git behavior tunables** (diff context lines, rename thresholds) and any change to the guarded git argv set.
8. **New LSP languages**: config overrides the four existing languages; adding languages remains a code change.
9. **Interactive key-capture remapping UI**: remapping is file-based only.
10. **Annotation output format changes**: the stdout markdown format is a frozen public API and takes no config.

## Design Considerations

- The warning surface must be non-blocking and quiet: a single status-line notice summarizing the problem (file path + first error + "N more" when applicable), dismissible, never covering the diff. It must not use stdout.
- With no config file present there is zero visible difference from today — no notices, no changed defaults.
- Help overlay, footer, and modal hints must show effective bindings; no "(default)" annotations needed.
- Documentation shipped with this spec: a complete annotated example `config.toml` (all sections, all action names, key-string grammar, preset list) added under `docs/`, and the README updated to mention the config file's existence and location. `docs/config-layer.md` is deleted or replaced by a pointer to this spec.

## Repository Standards

- All of `docs/rust-best-practices.md` applies; the gates (`cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`) must pass before every commit; conventional commits; refactors (the modal-table conversion) and behavior changes land in separate commits.
- TDD applies to the pure parts: config deserialization, defaults merging, key-string parsing, action-name mapping, editor template substitution — failing tests first.
- No `unwrap`/`expect`/panics in production code; config loading returns typed errors (`thiserror`) that the edge converts into the warning notice; `anyhow` stays at the `main.rs` edge.
- Layering: config types live in a new top-level module (e.g. `src/config/`); `git/` must not import it or any TUI types. The LSP section hands `lsp/` plain data (`LangServerCmd`-shaped), not config structs.
- Keymap/help/modal drift tests are preserved or strengthened — never deleted to make remapping fit. Performance tripwires in `src/ui/perf_tests.rs` keep their budgets.
- Integration tests use tempdirs (`tempfile`), canonicalized on macOS; tests must never read the developer's real `~/.config/redquill/config.toml` (path discovery must be injectable/env-overridable for tests).
- The `main.rs` CLI struct name collision with the new `Config` type (noted in `docs/config-layer.md`) is resolved by renaming one of them during implementation.

## Technical Considerations

- **Format & parsing**: TOML via the `toml` crate with `serde` derives; `#[serde(default)]` on every section and field so user files are pure partial overrides (starship's model). No layering crate (`config-rs`/`figment`) — single file, no env merging.
- **New dependencies** (each individually justified, `default-features = false` where practical): `toml` (parsing — unavoidable for the format); a path-discovery approach that yields `~/.config` on macOS — either the `etcetera` crate or a small hand-rolled resolver honoring `$XDG_CONFIG_HOME`; note the `directories` crate is NOT suitable as-is because it returns `~/Library/Application Support` on macOS, contradicting the Unit 1 requirement; `crokey` (key-string grammar + serde integration + display formatting for help rendering) unless implementation finds its sequence handling insufficient, in which case a hand-rolled parser mirroring its notation is acceptable. `toml_edit` and `notify` are explicitly not added (no write-back, no watcher).
- **Extensibility contract (themes next)**: `Config` is a struct of optional-by-default section structs; adding a section = define the struct with defaults + add one field + consume it where needed. This contract is documented in the `src/config/` module doc so future agents follow it. The theme research (gitui `struct_patch`, helix `inherits`/palette) is deliberately not pre-implemented — only not obstructed.
- **Keymap architecture**: defaults stay in code as the single source of truth (`default_map()` and the modal table data); config produces override entries applied at startup to build the effective tables. The existing `KeyChord`/`KeySeq`/`Action` types remain the runtime representation; config-side types (strings) convert at the boundary. `KeyChord::label()` and the config grammar must round-trip consistently so help output and config notation agree.
- **Degradation contract (documented in the module doc per rust-best-practices)**: missing file = silent; syntax error = whole file ignored + warning; invalid key/value/binding = that entry ignored + warning; everything else applies. LSP server spawn failures remain silent as today.
- **Current external guidance incorporated**: TOML + serde-defaults partial override (helix/yazi/starship/herdr convergence); human-readable key strings, never serialized crossterm events (universal across surveyed tools); lazygit's `editAtLine`/`{{filename}}`/`{{line}}` template + preset contract; per-action merge with explicit unbind rather than all-or-nothing keymap replacement (zellij's `clear-defaults` is the documented anti-pattern).

## Security Considerations

- The config file causes redquill to execute user-specified commands (editor template, LSP server commands). This is the same trust level as `$EDITOR` today — the file is user-owned local config executing as the user — but it must be stated in the docs, and templates must never pass through a shell (argv-only substitution), so config content cannot induce shell injection.
- Config never touches the guarded git operations: no config key may alter the fixed git argv set, the remote-ops behavior, or the annotation output contract.
- Proof artifacts must not capture personal information from the developer's real environment; tests and proofs use tempdir configs, never the real `~/.config`.

## Success Metrics

1. **User journey — editor**: a Zed user adds two lines (`[editor]`, `preset = "zed"`), restarts, presses `g<Space>`, and lands in Zed at the cursor line. Acceptance task executed against a real build with persisted evidence (transcript/screenshot under the spec's gitignored `proofs/` dir).
2. **User journey — remap**: a user remaps a main-keymap key and a staging-panel key and unbinds one action in `config.toml`; after restart all three behave as configured and `?` shows the new bindings. Acceptance task with persisted screenshot evidence.
3. **User journey — bad config**: a user with a typo'd config still gets a fully working review session and can read what was ignored from the on-screen warning. Acceptance task with persisted screenshot evidence.
4. **No-config invariant**: with no config file, the full test suite and a manual smoke session show behavior identical to pre-spec (zero notices, identical defaults); perf tripwires pass unchanged.
5. **Extensibility check**: a written walkthrough (in the module doc or PR description) shows the exact steps to add a hypothetical `[theme]` section, touching no discovery/parse/warning code.

## Open Questions

1. Assumption: `[search]` startup defaults are included (trivial) and git diff tunables are excluded, interpreting "everything except themes" as the surfaces discussed in round 1. Flag during review if you want either flipped.
2. The exact validated range for `sidebar_width` (proposed: reject outside [20, 200], clamp to terminal at render) can be tuned during implementation without changing the contract.
3. The initial editor preset list can grow during implementation (each preset is one data row + one test); the listed eleven are the floor.
4. Final `[keys.*]` table names for modal modes will be enumerated from the shipped mode set during implementation and documented in the example config; the contract (one table per modal mode, same override semantics) is fixed.
