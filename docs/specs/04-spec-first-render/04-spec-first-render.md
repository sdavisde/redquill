# Task 4 — First Render Spec

## 1. Overview

Task 4 brings the diff on screen: it adds ratatui + crossterm, a robust terminal
enter/restore guard, an event loop, and a data-driven keymap, then renders the
full parsed diff as one scrollable, colored unified view. Scope is deliberately
minimal — scroll and quit only — so the rendering foundation and terminal safety
are solid before navigation, sidebar, and highlighting land in Tasks 5–6.

## 2. Depends on

- `diff::{DiffFile, Hunk, Line, LineKind}` and `Line.changed_spans` from Task 3
  — the model `ui/` renders. `ui/` receives this already-parsed; it never calls
  `git/`.
- `git::GitRunner` + `diff` parsing wired in `main.rs` — `main.rs` loads data and
  hands the model to `ui/`.
- `main.rs` CLI/`Config` (diff target resolution) — unchanged; now routes into
  the TUI instead of printing a summary.

## 3. Goals

- Add ratatui + crossterm as the only new dependencies (justified in the commit).
- Enter raw mode + alternate screen and always restore them — including on
  panic — before any rendering runs.
- Render all changed files as one scrollable unified diff with headers, gutters,
  old/new line numbers, +/- coloring, and word-diff emphasis.
- Route every key through a `Keymap` that maps `KeyEvent → Action` as data;
  widgets contain no key-matching.
- Bind `j`/`k` + `Ctrl-d`/`Ctrl-u` scrolling and `q` quit; every other README
  binding resolves to a no-op `Action` placeholder.

## 4. Demoable Units of Work

### DUW 4.1 — Terminal guard (do this first)

**Purpose:** Guarantee the terminal is never left in raw mode / alternate screen,
even if rendering panics.

- FR-render-term-1: The system shall enable raw mode and enter the alternate
  screen on startup, and disable/leave both on normal exit.
- FR-render-term-2: The system shall install a panic hook that restores the
  terminal (disable raw mode, leave alternate screen) before the default panic
  message prints.
- FR-render-term-3: Terminal setup/teardown shall be encapsulated in a guard
  type whose `Drop` restores the terminal, so early returns and `?` also restore.

**Proof Artifacts:**
- Observable: force a panic mid-render (temporary test hook); the shell prompt
  returns usable (no stuck raw mode, cursor visible, echo on).
- Observable: `q` exits and the terminal is clean.
- Unit test (where feasible): the guard's teardown path is exercised without a
  live TTY (e.g. teardown function is callable and idempotent).

### DUW 4.2 — Keymap as data

**Purpose:** Establish the remappable keymap layer CLAUDE.md mandates, seeded
with the README's default bindings.

- FR-render-keymap-1: The system shall define an `Action` enum covering the
  README's actions and resolve a `crossterm` `KeyEvent` to an `Action` through a
  `Keymap` lookup structure (data), not `match` arms inside widgets.
- FR-render-keymap-2: The default `Keymap` shall bind exactly the README draft
  map: `j`/`k` and `Ctrl-d`/`Ctrl-u` (move/scroll), `]`/`[`, `Tab`/`Shift-Tab`,
  `/`, `n`/`N`, `c`, `v`, `space`, `s`, `gd`/`gr`/`K`, `a`, `?`, `q`/`Q`.
- FR-render-keymap-3: In Task 4 only `j`/`k`, `Ctrl-d`/`Ctrl-u`, and `q` shall
  have live behavior; all other bound actions shall resolve to a defined `Action`
  that the loop treats as a no-op (no crash, no hidden feature).
- FR-render-keymap-4: Unbound keys shall resolve to `Action::Noop` (matching
  the sketched `resolve(&self, ev) -> Action` signature) and be ignored.
- FR-render-keymap-5: Changing a default binding shall require editing exactly
  one data entry in `Keymap::default_map()` (plus the README table row and at
  most one default-map test assertion). No widget, prompt string, or doc
  comment may hardcode a key name; anything that displays a key (the future
  help overlay, messages like "press q to quit") derives it from the `Keymap`.
  Rationale: all default bindings are provisional — the operator intends to
  tune them by feel after using the tool, so a rebind must stay a one-line
  change.

**Proof Artifacts:**
- Unit test: `keymap.resolve(KeyEvent(char 'j'))` → `Action::ScrollDown`;
  `Ctrl-d` → `Action::HalfPageDown`; `q` → `Action::Quit`.
- Unit test: `]` resolves to `Action::NextHunk` even though it's a no-op this
  task (binding present, behavior deferred).
- Unit test: an unbound key resolves to no action.

### DUW 4.3 — Unified diff render + scrolling

**Purpose:** The actual on-screen diff.

- FR-render-view-1: The system shall render every `DiffFile` in sequence as one
  continuous scrollable buffer: a file header per file, a hunk header per hunk,
  and one row per `Line`.
- FR-render-view-2: Each line row shall show a +/- (or space) gutter marker and
  the old and new line numbers from the model, then the content.
- FR-render-view-3: Added lines shall render in an added color and removed lines
  in a removed color (plain colors, no syntax highlighting); context is default.
- FR-render-view-4: Within paired lines, the `changed_spans` regions shall render
  with added emphasis (e.g. brighter/inverted) distinct from the rest of the line.
- FR-render-view-5: `j`/`k` shall scroll by one line and `Ctrl-d`/`Ctrl-u` by a
  half page, clamped to content bounds; rendering shall occur on event (render at
  need), not in a busy loop.

**Proof Artifacts:**
- Observable: `cargo run` in a dirty repo shows a colored, scrollable unified
  diff spanning all changed files.
- Unit test: pure layout helpers (e.g. gutter formatting, line-number column
  width, clamped scroll offset) return expected values for representative inputs.
- Observable: scrolling past the end/top clamps rather than panicking.

### DUW 4.4 — main.rs → ui wiring

**Purpose:** Keep the boundary clean: data in `main.rs`, presentation in `ui/`.

- FR-render-wire-1: `main.rs` shall load `RawFilePatch`es via `git/`, parse them
  via `diff/`, and pass the owned model into a `ui/` entry point; `ui/` shall not
  reference `git/`.
- FR-render-wire-2: An empty diff shall enter the TUI showing an explicit empty
  state and quit cleanly on `q` (no special-case bypass).

**Proof Artifacts:**
- Observable / grep: `ui/` has no `use ...git` imports.
- Observable: running in a clean repo shows an empty-state message and `q` exits.

## 5. Data Model / Key Types

```rust
/// Every reviewer action; bound by the Keymap. Most are no-ops in Task 4.
pub enum Action {
    ScrollDown, ScrollUp, HalfPageDown, HalfPageUp,
    NextHunk, PrevHunk, NextFile, PrevFile,        // no-op this task
    Search, SearchNext, SearchPrev,                // no-op
    Comment, VisualSelect, StageToggle, ToggleStagePanel, // no-op
    GotoDefinition, FindReferences, Hover,         // no-op
    AnnotationList, Help,                          // no-op
    Quit, QuitDiscard,
    Noop,
}

/// KeyEvent -> Action as data. Seeded from README defaults; remappable later.
pub struct Keymap {
    bindings: std::collections::HashMap<KeyChord, Action>,
}
impl Keymap {
    pub fn default_map() -> Self { /* README draft bindings */ }
    pub fn resolve(&self, ev: crossterm::event::KeyEvent) -> Action { /* ... */ }
}

/// Normalized key incl. modifiers; also the seed for future chords (gd/gr).
pub struct KeyChord { /* code + modifiers */ }

/// Owns scroll offset and the parsed model; the render/event driver.
pub struct App {
    files: Vec<diff::DiffFile>,
    /// Line offset into the flattened diff. `usize`, not `u16`: the render
    /// path slices the visible range itself (see Edge Cases: very large diff),
    /// so nothing forces ratatui's `u16` scroll bound on the model.
    scroll: usize,
    should_quit: bool,
}
```

Note: `gd`/`gr` are two-key chords in the README. Task 4 may register them as
placeholder single-entry no-ops and defer real chord sequencing; call this out
in a doc comment (Open Questions).

Remap-readiness note: keep `KeyChord` and `Action` plain, serializable-shaped
data (simple enums/structs, no closures or trait objects in the map). The
README promises "fully remappable", and comparable tools do this via a user
config file (lazygit's `config.yml` keybinding section, gitui's
`key_bindings.ron`); a future task must be able to build a `Keymap` from such
a file without refactoring this layer. Config-file loading itself is out of
scope for Milestone 1 — the requirement here is only that the data shape
doesn't foreclose it.

## 6. Edge Cases

- Panic during render — terminal must restore (DUW 4.1).
- Empty diff — empty state, clean quit.
- Very long lines — must not corrupt layout; render truncated-to-width or
  horizontally clipped (no wrap-induced misalignment of gutters).
- Very large diff (thousands of lines) — scrolling must stay responsive; render
  only the visible slice, not the whole buffer each frame.
- Terminal resize — re-render to new size without panic.
- Binary / zero-hunk files — render the header and a "binary / no textual diff"
  placeholder row, not an empty gap.
- Content is guaranteed valid UTF-8 — `git/` decodes strictly and fails with
  `GitError::Utf8` upstream rather than normalizing, so `ui/` renders as-is.
  (A repo with non-UTF-8 file content will currently error before the TUI ever
  starts; lossy decoding in `git/` is a known future hardening item, not Task 4
  scope.)
- Narrow terminal (fewer columns than the gutter needs) — clamp gracefully.

## 7. Non-Goals

- No file sidebar, no hunk/file jumping, no cursor concept — Task 5.
- No syntax highlighting — Task 6.
- No side-by-side view, no search execution, no staging, no annotations — later
  roadmap steps; their keys are bound but no-op.
- No real two-key chord handling for `gd`/`gr` beyond a placeholder — Task 5+.

## 8. Testing Strategy

- **Unit:** the `Keymap` resolution table and pure layout helpers (scroll
  clamping, gutter/line-number formatting, visible-range computation).
- **Not tested (per CLAUDE.md):** interactive rendering itself — no snapshot
  harness required this task.
- **Manual/observable:** the panic-restore and clean-quit behaviors, verified by
  running in a dirty repo and via a temporary panic trigger.

## 9. Open Questions

- **Line wrapping vs. horizontal clip for long lines** — wrap, truncate, or
  horizontal scroll? *Recommended default:* truncate to width with a trailing
  marker; horizontal scroll is deferred. Keeps gutters aligned and layout math
  simple.
- **`gd`/`gr` chord handling now or later** — model two-key chords in the Keymap
  in Task 4, or stub single keys? *Recommended default:* stub as no-op single
  entries now; build real chord sequencing in Task 5 when `?`/help needs to
  display them accurately.
- **Half-page size source** — fixed constant or viewport height? *Recommended
  default:* half the current viewport height, recomputed per event.
- **ratatui backend** — crossterm backend only? *Recommended default:* yes,
  crossterm only, matching the stack decision; no termion/other backends.
- **Arrow-key / page-key aliases** — should `Down`/`Up`, `PgDn`/`PgUp` alias
  `j`/`k`, `Ctrl-d`/`Ctrl-u`? Comparable TUIs (lazygit, gitui) accept both
  vim keys and arrows. *Recommended default:* yes — add the aliases as extra
  `default_map()` entries (zero extra code since the keymap is data). They are
  aliases of existing actions, not new features, so this doesn't violate the
  README-map rule; list them in the README row alongside `j`/`k` when wired.
