# Dead-end journey (Success Metric 1)

> **The dead-end disappears**: in a repo where an agent has just committed
> (clean working tree — today an empty, useless viewer), a user discovering
> controls only from the empty-state hints and `?` gets from launch to reading
> that commit's diff in a handful of obvious keystrokes (target: panel →
> History → select → open, ≤5 keys).

**Verdict: MET.** Launch → newest commit's diff in **3 keystrokes**
(`` ` `` → `Tab` → `Enter`), each step named on-screen before it is pressed.

## How this was produced

The live TUI cannot launch in this sandbox (no controlling TTY:
`enable_raw_mode` → os error 6). The journey is therefore driven through the
real key-dispatch pipeline (`dispatch_key`, the same handler the blocking
event loop calls) against a throwaway tempdir repo, and each screen is
captured by rendering the real `draw` into a `TestBackend` — the screenshot
stand-in. See the operator TTY-deferred steps at the bottom for the literal
interactive reproduction.

Backing test (asserts every claim below):
`src/ui/history_integration_tests.rs::dead_end_journey_reaches_the_newest_commit_in_a_handful_of_keys`.
Reproduce the verbatim screenshots with:

```sh
cargo test --lib -- --nocapture --test-threads=1 \
  history_integration_tests::dead_end_journey
```

Fixture: `repo_with_history()` — three commits touching `a.txt`, then a
**clean working tree** (the "agent already committed" dead end this spec
targets).

## Keystroke path (exact)

| # | Key | Discoverable from | Result |
|---|-----|-------------------|--------|
| 1 | `` ` `` | welcome hint `` ` open the git panel`` | git panel focused (Changes tab) |
| 2 | `Tab` | welcome hint `Tab switch to the History tab to review recent commits` | History tab, commits load |
| 3 | `Enter` | panel footer `Enter open file` **and** `?` overlay row `Enter  Focus diff on this file (History tab: open the commit)` | newest commit's diff opens |

Three keystrokes to content. Well within the ≤5 budget, and each step is
literally named on the preceding screen — no hidden knowledge required. (An
optional `? … Esc` detour to *read* the control is not part of the path.)

## Step 0 — launch: the welcome state (not a blank screen)

The situation is named, and three keyed next steps are shown; the keys come
from the shared keymap table (not hardcoded), so a remap would display
correctly.

```text
┌diff──────────────────────────────────────────────────────────────────────────────────────────────┐
│                                                                                                  │
│                                      No uncommitted changes                                      │
│                                                                                                  │
│                                       ` open the git panel                                       │
│                      Tab switch to the History tab to review recent commits                      │
│                                            ? open help                                           │
│                                                                                                  │
└──────────────────────────────────────────────────────────────────────────────────────────────────┘
 j/k move · ] hunk · za fold · Space stage hunk · S stage file · c comment · / search · ` git panel
 ? help
```

Discoverability asserted: the screen literally contains `No uncommitted
changes`, `open the git panel`, `switch to the History tab to review recent
commits`, and `open help`.

## Step 2 — after `` ` `` then `Tab`: the History tab

Real `git log` commits load newest-first via the background poller (Zed-style
two-line rows: subject; then dimmed `author · relative-time · short-sha`). The
always-visible footer already promises `Enter open file`.

```text
┌diff──────────────────────────────────────────────────────────────┐┌git: main  Changes History────┐
│                                                                  ││third commit                  │
│                                                                  ││  redquill test · just now · c│
│                                                                  ││second commit                 │
│                                                                  ││  redquill test · just now · 9│
│                                                                  ││first commit                  │
│                                                                  ││  redquill test · just now · 8│
│                      No uncommitted changes                      ││                              │
│                       ` open the git panel                       ││                              │
│      Tab switch to the History tab to review recent commits      ││                              │
│                            ? open help                           ││                              │
│                                                                  │└──────────────────────────────┘
│                                                                  │ [0 files]
│                                                                  │ c8cda7e third commit
└──────────────────────────────────────────────────────────────────┘ f fetch  p pull  P publish
 j/k move · Enter open file · f fetch · p pull · P publish · c commit · ` close · Tab tab · ? help
```

## Step 2b — `?` teaches that `Enter` opens the commit (optional detour)

The `?` overlay is long and scrollable; filtering it with its own `/` search
(what a user would do) brings the Git-panel `Enter` row into one viewport:

```text
         ╭ keybinds ───────────────────────────────────────────────────────── esc close ╮
         │ /open the commit                                                             │
         │                                                                              │
         │ Git panel (focused)                                                          │
         │ Enter                Focus diff on this file (History tab: open the commit)  │
         ╰─── j/k scroll  ·  pgup/pgdn page  ·  g/G ends  ·  / filter  ·  esc close ────╯
```

## Step 3 — after `Enter`: the newest commit's diff

`Enter` on the highlighted row (cursor rests on row 0 = the newest commit)
opens that commit read-only in the same multibuffer, with the header block
(short SHA `c8cda7e`, author, absolute date, subject) above the diff. The
footer now shows `Esc return` and drops the staging/code-intel hints (see the
no-lies proof). The welcome dead-end is gone; the user is reading the diff.

```text
 c8cda7e  redquill test  2026-07-14 22:50 UTC  third commit
┌a.txt─────────────────────────────────────────────────────────────────────────────────────────────┐
│  ▾ M a.txt                                                                                       │
│  @@ -1,2 +1,3 @@                                                                                 │
│    1   1  one                                                                                    │
│    2   2  two                                                                                    │
│        3 +three                                                                                  │
└──────────────────────────────────────────────────────────────────────────────────────────────────┘
 j/k move · ] hunk · za fold · c comment · Esc return · / search · ` git panel · ? help
```

Asserted: `app.target` is `DiffTarget::Commit(_)`, `app.viewing_commit()` is
true, `app.active_commit.subject == "third commit"` (the **newest** commit),
the diff has files, and the header shows the opened commit's short SHA.

## TTY-deferred proof (operator)

With a real terminal, reproduce the literal journey:

1. In a repo whose working tree is clean but which has commits, run
   `cargo run --` from the repo root. The diff area shows the welcome state
   above ("No uncommitted changes" + the three keyed hints).
2. Press `` ` `` — the git panel opens (Changes tab).
3. Press `Tab` — the History tab lists the branch's commits, newest first.
4. (Optional) Press `?`, type `/open the commit` to see that `Enter` opens the
   highlighted commit; press `Esc` twice to clear the filter and close help.
5. Press `Enter` — the newest commit opens read-only in the multibuffer with
   its header block. You are now reading the commit's diff, 3 keystrokes from
   launch.
