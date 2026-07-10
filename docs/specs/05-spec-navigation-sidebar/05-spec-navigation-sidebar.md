# Task 5 — Navigation and Sidebar Spec

## 1. Overview

Task 5 makes a large diff comfortable to traverse without a mouse: hunk/file
jumps wired to Task 3's navigation functions, a toggleable file sidebar per the
README layout sketch, a keymap-generated `?` help overlay, and a current-line
cursor distinct from scroll position (the groundwork annotations will need).
Definition of done: a 20-file diff is comfortable to traverse and the help
overlay stays correct if a binding changes.

## 2. Depends on

- `diff` navigation primitives from Task 3 (`next_hunk`/`prev_hunk`/`next_file`/
  `prev_file` over `DiffPosition`) — the jumps call these; `ui/` does not
  reimplement traversal.
- `diff::DiffFile` + `ChangeStatus` — the sidebar renders one row per file with a
  status letter derived from `ChangeStatus`.
- Task 4's `App`, `Keymap`, `Action`, and terminal guard — extended, not
  replaced. `]`/`[`/`Tab`/`Shift-Tab`/`s`/`?` bindings move from no-op to live.

## 3. Goals

- `]`/`[` jump to next/previous hunk and `Tab`/`Shift-Tab` to next/previous file,
  scrolling the target hunk header near the top with context visible.
- A left file sidebar: status letter + path, current-file highlight, kept in
  sync with position and directly selectable; visible by default and toggleable
  via a new non-conflicting binding added to the README map in this task
  (proposed: `f` — see Open Questions; `s` is already claimed by the staging
  panel).
- A `?` help overlay listing current bindings, generated from `Keymap` data, and
  dismissed by any key.
- A current-line cursor separate from scroll offset; `j`/`k` move the cursor and
  scrolling follows it.

## 4. Demoable Units of Work

### DUW 5.1 — Cursor concept

**Purpose:** Introduce a current-line cursor (annotation groundwork) that scroll
follows, replacing Task 4's pure scroll-offset movement.

- FR-nav-cursor-1: The system shall track a current-line cursor as a
  `DiffPosition` (or an equivalent flat line index) distinct from the scroll
  offset.
- FR-nav-cursor-2: `j`/`k` shall move the cursor down/up by one visible line,
  skipping non-navigable rows per the chosen policy (see Open Questions), clamped
  to bounds.
- FR-nav-cursor-3: When the cursor moves outside the viewport, the system shall
  scroll the minimum amount to keep it visible (with a small margin).
- FR-nav-cursor-4: The current line shall render with a visible cursor indicator.

**Proof Artifacts:**
- Unit test (pure helper): given cursor line, viewport height, and scroll offset,
  the follow-scroll function returns the expected new offset (cursor above → scroll
  up; below → scroll down; inside → unchanged).
- Observable: `j`/`k` moves a highlighted current line; the view follows at the
  edges.

### DUW 5.2 — Hunk and file jumps

**Purpose:** Fast structural traversal.

- FR-nav-jump-1: `]`/`[` shall move the cursor to the first line of the
  next/previous hunk using Task 3's navigation functions; at the ends it is a
  no-op (or wraps — see Open Questions).
- FR-nav-jump-2: `Tab`/`Shift-Tab` shall move the cursor to the first hunk of the
  next/previous file using Task 3's navigation functions.
- FR-nav-jump-3: After a jump, the target hunk header shall be scrolled to a sane
  position — near the top with a few lines of leading context/header visible, not
  jammed against the last row.

**Proof Artifacts:**
- Unit test (pure): a "reveal position" helper maps a target `DiffPosition` +
  viewport height to a scroll offset placing the hunk header near the top.
- Observable: on a 20-file diff, `]`/`Tab` walk hunks/files predictably; ends
  behave per the chosen end-of-list policy.

### DUW 5.3 — File sidebar

**Purpose:** The left panel from the README layout sketch.

- FR-nav-sidebar-1: The system shall render a left panel listing every changed
  file as `<status letter> <path>`, where the letter derives from
  `diff::ChangeStatus` (M/A/D/R/…), using the same letter vocabulary as the
  existing `git::StatusCode::letter()` so status letters mean the same thing
  everywhere in the product.
- FR-nav-sidebar-2: The sidebar shall highlight the file containing the cursor and
  update as the cursor crosses file boundaries.
- FR-nav-sidebar-3: The sidebar shall support direct selection of a file (moving
  the cursor to that file's first hunk).
- FR-nav-sidebar-4: The sidebar shall be toggleable and visible by default; it
  is distinct from the staging panel (hidden by default, not built in this
  task). The toggle binding is a new key added to README.md's map as part of
  this task (proposed `f`), per CLAUDE.md's "propose map changes in README.md
  itself" rule — toggleability is in this task's definition of done, so the
  binding cannot be deferred.
- FR-nav-sidebar-5: Toggling the sidebar shall reflow the diff pane width without
  layout corruption.

**Proof Artifacts:**
- Observable: sidebar shows all files with status letters; the current file is
  highlighted and tracks the cursor.
- Observable: selecting a file jumps the diff to it.
- Observable: toggle hides/shows the panel; diff reflows cleanly.
- Unit test (pure): status-letter mapping from `ChangeStatus` returns expected
  letters.

### DUW 5.4 — Keymap-driven help overlay

**Purpose:** Single-source-of-truth `?` help; no hidden features.

- FR-nav-help-1: `?` shall open an overlay listing bindings generated by
  iterating the `Keymap` data — not a hardcoded string.
- FR-nav-help-2: Any key shall dismiss the overlay.
- FR-nav-help-3: If a binding is changed in the `Keymap`, the help overlay shall
  reflect it without a separate edit.

**Proof Artifacts:**
- Unit test: a help-model builder over a `Keymap` produces entries whose key
  labels match the bindings; changing a fixture binding changes the output.
- Observable: `?` shows current bindings; any key closes it.

## 5. Data Model / Key Types

```rust
/// Extends Task 4's App with cursor, sidebar, and overlay state.
pub struct App {
    files: Vec<diff::DiffFile>,
    cursor: diff::DiffPosition,   // current line, distinct from scroll
    scroll: usize,                // line offset, matching Task 4's model
    sidebar_visible: bool,        // default true
    overlay: Option<Overlay>,
    should_quit: bool,
}

pub enum Overlay { Help }

/// One sidebar row.
pub struct FileEntry<'a> {
    pub letter: char,             // from ChangeStatus
    pub path: &'a str,
    pub is_current: bool,
}

/// Pure help model derived from the Keymap (single source of truth).
pub struct HelpEntry { pub keys: String, pub action: String }
pub fn help_entries(keymap: &Keymap) -> Vec<HelpEntry>;

/// Pure scroll helpers (unit-tested). Offsets are usize per Task 4's model;
/// only the viewport height is a terminal-sized u16.
pub fn follow_cursor(cursor_line: usize, viewport_h: u16, scroll: usize) -> usize;
pub fn reveal_position(target: &diff::DiffPosition, /*...*/ viewport_h: u16) -> usize;
```

## 6. Edge Cases

- Single-file diff — `Tab`/`Shift-Tab` no-op (or wrap); sidebar shows one entry.
- Single-hunk file — `]`/`[` cross into adjacent files correctly.
- Cursor on a file/hunk header row vs. a content line — define what `j`/`k` and
  "current line" mean on non-content rows (Open Questions).
- Very long paths in the sidebar — truncate (middle or left ellipsis) to panel
  width.
- Narrow terminal — sidebar min-width clamp; below a threshold, auto-hide rather
  than crush the diff pane.
- Empty diff — sidebar empty state; jumps are no-ops; `?` still works.
- Binary / zero-hunk file — still selectable in the sidebar; jumps skip or land on
  its header per policy.
- Help overlay open while a jump key is pressed — the key dismisses the overlay
  (does not also jump), per FR-nav-help-2.

## 7. Non-Goals

- No syntax highlighting — Task 6.
- No staging panel or staging actions (`space`/`s`-staging) — roadmap step 3.
  Note: `s` in the README maps to "toggle staging panel"; this task must not
  build staging (see Open Questions on which key toggles the sidebar).
- No annotations/comment entry — the cursor is groundwork only.
- No search execution, no side-by-side, no LSP.

## 8. Testing Strategy

- **Unit (pure, primary):** `follow_cursor`, `reveal_position`, status-letter
  mapping, and `help_entries` over a `Keymap`. These are the behaviors that must
  stay correct as bindings/layout change.
- **Not tested:** interactive rendering of the sidebar/overlay (per CLAUDE.md).
- **Manual/observable:** the 20-file traversal comfort check and reflow-on-toggle.

## 9. Open Questions

- **Sidebar toggle key** — the README's `s` is "toggle staging panel," so the
  sidebar needs its own key, and the task's definition of done requires
  toggleability (leaving it unbound would fail the task). CLAUDE.md forbids
  *conflicting* bindings and directs map changes through README.md.
  *Recommended default:* bind `f` (mnemonic: files; currently unclaimed in the
  map) and amend README.md's keybinding table in the same commit. Alternatives
  if `f` is wanted elsewhere later: `e` or `b`. Per the operator, all default
  bindings are provisional and will be tuned by feel — `f` just satisfies the
  toggleability requirement now; rebinding later is a one-line
  `default_map()` edit plus a README row (see Task 4's FR-render-keymap-5).
- **Sidebar width** — fixed columns or percentage? *Recommended default:* fixed
  24 cols with a min-clamp and auto-hide on very narrow terminals.
- **End-of-list jump behavior** — wrap or stop at ends for `]`/`Tab`?
  *Recommended default:* stop (no wrap); a jump past the end is a no-op.
- **Cursor granularity** — does the cursor stop on header rows or only content
  lines? *Recommended default:* content lines only for `j`/`k`; jumps land on the
  hunk's first content line, with the header scrolled into view.
- **Help dismissal swallows the key** — does the dismiss key also perform its
  action? *Recommended default:* no; the first key only closes the overlay.
