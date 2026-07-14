# The tool never lies (Success Metric 3)

> **The tool never lies**: in any non-working-tree view, every key shown in
> `?`/footer does what it says, and no absent capability (staging, code-intel)
> has any effect. No wrong jumps, no misleading affordances during review.

**Verdict: MET.** In a commit view the `?` overlay lists exactly the keys that
work; the file/hunk staging gestures and the `gd`/`gr`/`K` code-intel keys are
absent from the overlay and the footer, and pressing them is inert.

## How this was produced

Backing test:
`src/ui/history_integration_tests.rs::commit_view_help_overlay_shows_only_truthful_keys`
opens a commit view (via `` ` `` → `Tab` → `Enter`) and cross-checks the
rendered overlay against behavior three ways:

1. the unfiltered overlay screenshot (visible evidence);
2. an **exhaustive, viewport-independent** keymap cross-check — for every
   diff-scope binding, `help::binding_hidden(action, /*staging*/false,
   /*code_intel*/false)` is `true` **iff** the action is one of
   `ToggleStage`/`StageFile`/`GotoDefinition`/`GotoReferences`/`Hover`. Since
   `help.rs` builds the overlay's rows by dropping exactly the
   `binding_hidden` bindings *before* rendering, this proves the inert keys
   can appear at **no** scroll position, and every other diff-scope key stays
   listed because it genuinely works;
3. presence of the working keys via the overlay's own `/` filter (using
   strings unique to each working row).

Behavior (keys are not just hidden but truly inert) is covered by the task-3.6
tests in the same file, confirmed below.

Reproduce verbatim:

```sh
cargo test --lib -- --nocapture --test-threads=1 \
  history_integration_tests::commit_view_help_overlay_shows_only
```

## The commit-view `?` overlay (unfiltered top)

Note the **Stage** section is reduced to the one affordance that genuinely
still works — `s Toggle staging panel` (it shows the index regardless of the
diff target) — with the inert `Space`/`S` diff-line stage gestures gone, and
there is **no** Code-intelligence section at all:

```text
 e6e56ce  redquill test  2026-07-14 22:50 UTC  third commit
┌a.txt─────────────────────────────────────────────────────────────────────────────────────────────┐
│  ▾ M a.txt                                                                                       │
│  @@ -1,2 +1,3 @@                                                                                 │
│    1   ╭ keybinds ──────────────────────────────────────────────────────────── esc close ╮       │
│    2   │ available commands and configured shortcuts                                     │       │
│        │ Navigation                                                                    █ │       │
│        │ j                    Move cursor down                                         █ │       │
│        │ ...                                                                             │       │
│        │ Annotate                                                                      ║ │       │
│        │ v                    Enter visual selection / cancel                          ║ │       │
│        │ c                    Comment on line/hunk/file (or visual selection)          ║ │       │
│        │                                                                               ║ │       │
│        │ Stage                                                                         ║ │       │
│        │ s                    Toggle staging panel                                     ║ │       │
│        │                                                                               ║ │       │
│        │ Search                                                                        ║ │       │
│        │ /                    Search                                                   ║ │       │
│        ╰───── j/k scroll  ·  pgup/pgdn page  ·  g/G ends  ·  / filter  ·  esc close ─────╯       │
└──────────────────────────────────────────────────────────────────────────────────────────────────┘
```

(No `Space  Stage/unstage hunk`, no `S  Stage/unstage file`, no
`Go to definition`/`Find references`/`Hover docs`.)

## Working keys are listed (filtered captures)

The overlay's `/` filter surfaces each working key on one screen:

```text
│ /emit annotations                        │        │ /return from a commit view                    │
│ Quit                                     │        │ Panels                                        │
│ q      Quit and emit annotations         │        │ Esc    Close help / cancel selection /        │
│ Git panel (focused)                      │        │        return from a commit view              │
│ q      Quit and emit annotations         │        │                                               │

│ /Changes / History                       │        │ /Comment on                                   │
│ Git panel (focused)                      │        │ Annotate                                      │
│ Tab    Switch Changes / History tab      │        │ c      Comment on line/hunk/file (...)        │
```

## Footer strip agrees (from the dead-end capture)

The commit-view footer offers only truthful hints — note `Esc return` and the
**absence** of stage/code-intel hints that the working-tree footer shows:

```text
commit view : j/k move · ] hunk · za fold · c comment · Esc return · / search · ` git panel · ? help
working tree: j/k move · ] hunk · za fold · Space stage hunk · S stage file · c comment · / search · ` git panel
```

## Behavior is inert, not just hidden (task 3.6 coverage — confirmed)

The following existing tests in `src/ui/history_integration_tests.rs` already
prove the absent capabilities do nothing, so no extension was needed:

- `commit_view_hides_and_disarms_staging_keys` — `staging_mode() == ReadOnly`;
  the `stage` hint is absent from the footer; `help::binding_hidden` hides
  `ToggleStage`/`StageFile`; pressing `Space` sets `"read-only diff target"`
  and makes **no** git call.
- `commit_view_hides_and_disarms_code_intel_keys` — `supports_code_intel() ==
  false`; `help::binding_hidden` hides `GotoDefinition`/`GotoReferences`/
  `Hover`; pressing `K` does **not** open the peek overlay.
- `commit_view_never_auto_refreshes` — `is_live() == false`; `maybe_auto_refresh`
  spawns no background refresh.
- `commit_view_annotations_are_fully_functional` — the keys that *are* listed
  (comment) genuinely act.

Together: every key the commit-view `?`/footer shows does what it says, and
every absent capability is both invisible and inert. No lies.

## TTY-deferred proof (operator)

`cargo run --`, open a commit (`` ` `` → `Tab` → `Enter`), press `?`: the
overlay has no `Space/S` stage rows and no Code-intelligence section; the
footer shows `Esc return` and no stage/code-intel hints. Press `Space` or `K`
in the commit view — nothing happens (a `read-only diff target` / no-op),
confirming absent ≡ inert.
