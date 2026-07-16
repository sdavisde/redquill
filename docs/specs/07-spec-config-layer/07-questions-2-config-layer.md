# 07 Questions Round 2 - Config Layer

> **Resolution (2026-07-16, answered in conversation — supersedes checkboxes below):** the user withdrew the in-app settings UI and live reload entirely. Final decisions: config.toml only, read once at startup; redquill never writes the file; no reload keybind, no watcher (Q1/Q2/Q4 moot). Q3: **(B) everything remappable, including the modal tables** (confirmed interactively). Q5: moot — settings UI dropped, so this stays one spec covering the file-based system.

Follow-ups from your round-1 answers. Two of them (everything-but-themes scope; herdr-style settings UI with live reload on save) pull in sub-decisions that materially change the spec, so this round pins those down. Please answer in this file as before.

Settled from round 1 and carried forward as spec inputs: TOML; single `~/.config/redquill/config.toml`; missing file silent, malformed file → visible non-blocking warning; lazygit-style `edit_at_line` template + preset table as a precedence tier between `--editor` and `$VISUAL`; per-repo overrides and env-var overrides are non-goals; dependencies are acceptable when individually justified (planned set: `toml`, `directories`, `crokey` for key-string parsing, and `toml_edit` if question 2 lands on comment-preserving writes); architecture must make a future theme section trivial to add (serde-default partial-override sections, so specs 08+ add a `[theme]` table without touching the loader).

## 1. Settings UI: what can it edit in v1?

An in-app settings screen (herdr-style) is now in scope. Interactive key-remapping UI (press-a-key capture, conflict detection, sequence entry) is a large feature on its own — distinct from *supporting* remapping via the config file.

- [x] (A) Settings UI edits simple values only: sidebar side/width, editor preset/template, LSP server commands on/off, search defaults. Keybindings are remappable via config.toml only; the settings UI displays them read-only with a pointer to the file
- [ ] (B) Settings UI edits everything including interactive key remapping (key-capture widget, conflict warnings)
- [ ] (C) Other (describe)

**Current best-practice context:** herdr's settings screen covers general options while keybinding changes go through config.toml + reload; no surveyed TUI ships in-app key-capture remapping in its first config release.

**Recommended answer(s):** [(A)]

**Why these are recommended:**

- `(A)` keeps the settings UI a form over typed values — buildable with existing modal/panel machinery — while file-based remapping still ships full keymap power in this spec.
- `(B)` adds a key-capture interaction model, conflict resolution UX, and sequence-entry design that would dominate the task list; it layers cleanly on later as its own small spec once the settings UI exists.

## 2. Config file write-back

A settings UI that saves implies redquill now writes `config.toml` (round 1 declined to make "never writes config" a non-goal). How should writes work?

- [ ] (A) Comment-preserving edits via `toml_edit`: only the keys the user changed in the UI are modified; their hand-written comments and formatting survive (starship's approach for config-writing commands)
- [ ] (B) Serialize the full Config struct back to disk (simpler, but clobbers user comments/layout)
- [ ] (C) Settings UI changes apply to the running session only; persisting them to the file stays manual
- [ ] (D) Other (describe)

**Current best-practice context:** starship uses `toml` for reads and `toml_edit` for writes precisely to preserve user comments; tools that round-trip through plain serialization get bug reports the first time a commented config gets flattened.

**Recommended answer(s):** [(A)]

**Why these are recommended:**

- `(A)` is what makes "edit by hand" and "edit in-app" coexist — users who annotate their config don't lose work because they touched the settings screen. `toml_edit` is the established, single-purpose crate for this.
- `(B)` silently destroys user-authored content on first save; `(C)` contradicts the apply-on-save experience you described.

## 3. Keymap remapping: which bindings are remappable in this spec?

Two tiers of bindings exist today: the main `Keymap` table in `src/ui/keymap.rs` (a remap-ready `Vec<Binding>` — all Normal/Visual/Panel actions, including two-key sequences like `gd`, `za`) and the `const` modal tables in `src/ui/modal_keys.rs` (staging panel, switcher, finder, help, compose, search — currently fixed tables with drift tests; making them overridable is a separate refactor).

- [ ] (A) Main keymap fully remappable (string keys via crokey, multiple keys per action, explicit unbind, help overlay reflects remaps automatically); modal tables stay fixed this spec and follow in a later spec
- [ ] (B) Everything remappable now, including the modal-table refactor
- [ ] (C) Other (describe)

**Current best-practice context:** helix scopes remapping to three modes and leaves picker/prompt internals fixed; lazygit exposes named contexts but grew them over releases. Shipping the high-traffic surface first is the pattern.

**Recommended answer(s):** [(A)]

**Why these are recommended:**

- `(A)` covers every action a reviewer touches during normal use, and the help overlay stays truthful for free since it renders from the same table. The drift-test suite carries over unchanged for modal tables.
- `(B)` adds a refactor of eleven `const` tables plus their bidirectional drift tests to an already-large spec; nothing about `(A)`'s design blocks doing it later.

## 4. Reload semantics

You want live reload on save from the settings menu. What about the config file being edited outside the app?

- [ ] (A) Apply-on-save from the settings UI only; external file edits require restart (no watcher, no reload keybind)
- [ ] (B) Apply-on-save from the settings UI, plus a manual reload action (keybind, listed in `?`) that re-reads the file — covers hand-editing without a file watcher
- [ ] (C) Full file watcher (`notify` crate) — any external change applies automatically
- [ ] (D) Other (describe)

**Current best-practice context:** helix (`:config-reload`) and herdr (`prefix+shift+r`) both chose manual reload over file watching; a watcher adds a background thread and render-loop integration for marginal benefit.

**Recommended answer(s):** [(B)]

**Why these are recommended:**

- `(B)` reuses the exact same load-and-apply path the settings UI needs anyway, so the marginal cost is one keybinding — and hand-editing is the primary way keybindings get remapped under question 3(A), so it deserves a no-restart path.
- `(A)` makes file-based remapping feel second-class; `(C)` buys automation at the cost of a dependency, a background thread, and mid-edit-clobber concerns your concurrency guardrails would make you engineer around.

## 5. One spec or a split, now that scope grew

Round 1 chose "everything except themes." With the settings UI added, this spec now covers: config infrastructure, sidebar layout, editor templating/presets, LSP overrides, full main-keymap remapping, settings UI, write-back, and reload — roughly 5-6 demoable units (the workflow's guidance tops out around 4). Confirm how to package it.

- [ ] (A) Keep one spec as you chose, with demoable units ordered so each lands independently (infra+layout → editor+LSP → keymap → settings UI+write-back+reload) and the task list generated per-unit to stay manageable
- [ ] (B) Two specs: 07 = config file system including keymap (everything file-based); 08 = settings UI + write-back + reload, building on 07
- [ ] (C) Other (describe)

**Current best-practice context:** Every surveyed tool shipped the file-based system first and the in-app settings surface in a later release; herdr's settings screen postdates its config file.

**Recommended answer(s):** [(B)]

**Why these are recommended:**

- `(B)` honors your "everything except themes" intent across two consecutive specs while keeping each at a size the task-planning and audit phases handle well; 07 is fully demoable on its own (edit file → reload → behavior changes), and 08's settings UI then has a stable config API to build on.
- `(A)` is workable if you prefer one document of record, but the audit gate and validation phase get long, and a stumble in the settings UI would hold hostage the already-done file-based work.
