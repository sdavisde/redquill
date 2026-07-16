# 07 Questions Round 1 - Config Layer

Please answer each question below (select one or more options, or add your own notes). Feel free to add additional context under any question.

Context: this spec grows the skeleton in `docs/config-layer.md` into a real user-facing configuration system. Research findings (this session) inventoried the codebase's config-ready surfaces and surveyed gitui, helix, lazygit, yazi, zellij, delta, atuin, starship, and herdr for proven patterns.

## 1. Scope of this first spec

The full config system spans five surfaces of very different effort: sidebar layout (low), external editor (low), LSP server table (low), theme (medium: ~55-field color deserialization + tripwire-test reconciliation), keymap (high: key-string grammar, Action name map, modal-table refactor). Which slice should THIS spec cover?

- [ ] (A) Skeleton scope only: config loading infrastructure + sidebar side/width (exactly what `docs/config-layer.md` sketches)
- [ ] (B) Infrastructure + all three low-effort surfaces: sidebar layout, external editor (config tier + line-number templating), LSP server overrides
- [ ] (C) Option B plus theme customization
- [ ] (D) Everything in one spec, including keymap remapping
- [x] (E) Other (describe) Everything except for themes should be in this spec - but we should build this system in such a way that adding themes is trivial for future agents.

**Current best-practice context:** Mature TUIs shipped config incrementally; gitui's theme override mechanism went through a breaking redesign (pre-0.23 full-copy → patch-based) because it shipped too much surface too early. Theme and keymap each carry design decisions (color grammar, key-string grammar) big enough to warrant their own spec and review cycle.

**Recommended answer(s):** [(B)]

**Why these are recommended:**

- `(B)` delivers a complete, demoable config system (file discovery, parsing, precedence, error contract) plus three user-visible wins, while every included surface is already serde-shaped or precedence-ordered in the code — no refactors of tested invariants.
- `(A)` builds the plumbing but demos only a sidebar tweak; the marginal cost of editor + LSP on top of working infra is small and they're the surfaces users ask for first.
- `(C)` and `(D)` pull in the color parser / key grammar decisions, tripwire-test scoping, and the modal-`const`-table refactor — each material enough that folding them in risks an oversized task list. Better as specs 08 (theme) and 09 (keymap) reusing this spec's infrastructure.

## 2. Config file format

Which serialization format should the config file use?

- [x] (A) TOML
- [ ] (B) RON (Rusty Object Notation, as gitui uses)
- [ ] (C) YAML (as lazygit uses)
- [ ] (D) Other (describe)

**Current best-practice context:** TOML is where the Rust TUI ecosystem converged (helix, yazi, atuin, starship, herdr, and the official ratatui config template). RON works well for gitui but is niche. zellij's KDL migration is the community's cautionary tale about off-ecosystem formats; lazygit's YAML predates the Rust convergence.

**Recommended answer(s):** [(A)]

**Why these are recommended:**

- `(A)` TOML is what your users already edit for helix/yazi/starship/herdr, round-trips through plain `serde`, supports dotted-key tables (useful later for keymap sequences like `[keys.g]`), and the repo already ships a TOML tree-sitter grammar for highlighting it.
- `(B)` RON's advantage (native Rust enum syntax) matters most for raw key-event serialization, which the ecosystem avoids anyway in favor of human-readable strings.
- `(C)` YAML brings whitespace pitfalls and a heavier parser dependency for no familiarity gain in Rust-tool userbases.

## 3. File layout and location

Where should config live, and one file or several?

- [x] (A) Single `~/.config/redquill/config.toml` for now; split into `theme.toml`/`keymap.toml` only when specs 08/09 land, if size warrants
- [ ] (B) Create the three-file split (`config.toml`, `theme.toml`, `keymap.toml`) now, even though two files start empty
- [ ] (C) Single file, committed to staying single forever (don't update readme, that's internal implementation details)
- [ ] (D) Other (describe)

**Current best-practice context:** yazi and helix split by concern (general/keymap/theme); gitui uses two files. All use the XDG config dir (`~/.config/<tool>/`), typically via the `directories` crate, with an env-var override for the config path.

**Recommended answer(s):** [(A)]

**Why these are recommended:**

- `(A)` keeps this spec's surface minimal (one discovery path, one parse) while not foreclosing the ecosystem-standard split — TOML tables (`[layout]`, `[editor]`, `[lsp]`) already namespace the content so a later split is mechanical.
- `(B)` adds file-precedence questions this spec doesn't need to answer yet; `(C)` commits prematurely — a 55-color theme plus a full keymap in one file gets unwieldy, which is exactly why yazi/helix split.

## 4. Error-handling contract

What happens when the config file is missing, and when it is malformed (syntax error, unknown key, invalid value)?

- [x] (A) Missing file: silent fallback to defaults. Malformed file: app still starts with defaults, but shows a visible non-blocking warning (e.g. status-line message) naming the file and error
- [ ] (B) Fully silent in all cases (mirror the LSP degradation contract exactly, as `docs/config-layer.md` currently sketches)
- [ ] (C) Malformed file is a hard startup error: print the parse error to stderr and exit non-zero
- [ ] (D) Other (describe)

**Current best-practice context:** helix and lazygit surface config parse errors to the user rather than silently ignoring them. Silent degradation is your documented contract for *optional enhancements* (LSP), but a user who wrote a config expects it to apply — silently ignoring a typo'd file is a classic support trap.

**Recommended answer(s):** [(A)]

**Why these are recommended:**

- `(A)` preserves "the tool always starts" (important for a review TUI sitting in an agent pipeline) while making user error visible — the failure mode of `(B)` is "I changed my editor and nothing happened," with no signal at all.
- `(C)` is defensible (fail fast) but hostile in redquill's primary use: you'd block a code review because of a theme typo. A visible warning gets the same information across without breaking the run.
- Whichever you choose becomes the documented degradation contract per `docs/rust-best-practices.md` ("silent degradation is a valid design, but only when written down").

## 5. New dependencies

The lean-dependency guardrail makes this a user decision. Which dependencies may this spec add?

- [ ] (A) `toml` (parse) + `directories` (XDG/platform config paths), both with `default-features = false` where possible
- [ ] (B) `toml` only; hand-roll config-path discovery with `std::env` (`$XDG_CONFIG_HOME`, `$HOME/.config`)
- [ ] (C) A layered-config crate (`config-rs` or `figment`) for file+env+defaults merging
- [x] (D) Other (describe) I've always felt tyhe "zero dependencies" guard rail was too strict. Dependencies are helpful! as long as they're evaluated and there's a good reason to use them.

**Current best-practice context:** starship deliberately uses plain `serde`+`toml` with `#[serde(default)]` and no layering crate; atuin uses `config-rs` only because it wants env-var overrides. The official ratatui template uses `directories` for paths. `crokey` (key-string parsing) is deferred to the keymap spec under the recommended scope.

**Recommended answer(s):** [(A)]

**Why these are recommended:**

- `(A)` is the justified minimum: `toml` is unavoidable for the format, and `directories` is small, ubiquitous, and gets macOS/Linux/Windows path conventions right — hand-rolling `(B)` saves one small dep but re-derives platform quirks the crate already encodes (worth it only if you want zero new deps beyond the parser).
- `(C)` is over-machinery for a single-file config with no env-override requirement, against the lean-binary guardrail; starship is the direct precedent for skipping it.

## 6. External editor: templating and presets

Today the editor command is `--editor` flag > `$VISUAL` > `$EDITOR` > `nvim`, with a hardcoded two-family arg-style rule (VS Code/VSCodium get `--goto file:line`, everyone else `+line`). If editor config is in scope, what shape should it take?

- [x] (A) lazygit's model: an `edit_at_line` command template with `{{filename}}`/`{{line}}` placeholders, plus a named `preset` table (nvim, vscode, helix, zed, emacs, ...) so most users never write a template; config slots in as a precedence tier below the `--editor` flag, above `$VISUAL`
- [ ] (B) Config supplies only the editor command string (a 4th precedence tier); keep the hardcoded arg-style special-casing as-is
- [ ] (C) Presets only, no free-form template
- [ ] (D) Other (describe)

**Current best-practice context:** lazygit's `os.editAtLine: 'editor --line={{line}} {{filename}}'` + `editPreset: 'vscode'` is the established contract for "open at line" across editors. Placeholder substitution into a split argv list (never `sh -c`) satisfies the repo's subprocess guardrail.

**Recommended answer(s):** [(A)]

**Why these are recommended:**

- `(A)` fixes the real limitation (any editor with a novel line syntax works, not just two hardcoded families) and converts the special-case in `src/ui/editor.rs` into data; presets keep the common path zero-effort.
- `(B)` exposes config but preserves the limitation, so a Sublime or Kakoune user still can't jump to a line; `(C)` blocks editors we didn't anticipate.

## 7. Explicit non-goals for this spec

Which should be declared out of scope for this spec (multi-select — recommended: all four)?

- [x] (A) Per-repository config override (lazygit-style `<repo>/.git/redquill.toml` layered over global)
- [ ] (B) Live reload (reload keybind and/or file watcher) — config is read once at startup
- [x] (C) Environment-variable overrides of config values
- [ ] (D) In-app settings UI / config-writing commands (redquill never writes the config file)
- [x] (E) Other (describe) - I think the in-app settings UI is actually a great feature that we should steal from the way the `herdr` multiplexer works. WE can do the live reload on save in that settings menu too?

**Current best-practice context:** Per-repo config (helix, lazygit) and reload keybinds (helix `:config-reload`, herdr `prefix+shift+r`) are valued polish, but every surveyed tool shipped them after the core file-based system. A manual reload keybind fits the no-file-watcher/no-render-loop-blocking constraints and is a natural fast-follow.

**Recommended answer(s):** [(A), (B), (C), (D)]

**Why these are recommended:**

- Each is additive later without reshaping this spec's design: per-repo override is one more layer in an already-defined precedence chain; a reload keybind reuses the same load function; env overrides would only ever motivate `config-rs`, which question 5 recommends against.
- Declaring them non-goals now prevents scope creep during task planning and matches how helix/lazygit/herdr sequenced their own config systems.
