# 10-spec-help-discoverability.md

## Introduction/Overview

The `?` help overlay currently renders every binding across every context in one long scroll — a reference pretending to be help. New users can't find the control they want and often don't yet know what they want to do. This spec redesigns `?` to be context-first (a short "what applies right now" view with an intent-phrased common-workflows header, the full reference demoted to a second tab) and adds which-key popups that reveal `g`/`z` prefix continuations after a brief pause, turning the prefix namespaces from memorization into browsing. **(Amendment 2026-07-18: the which-key popup half of this sentence was withdrawn after dogfooding — see the Unit 3 amendment.)**

Everything renders from the existing keymap/modal-key tables, preserving the repo's no-drift guarantee: help can never disagree with dispatch.

## Goals

- A user who presses `?` sees, on one screen without scrolling in a typical terminal, only what applies to their current context plus a short list of global keys.
- A new user can go from "I want to review a branch" to the right key using the workflows header alone, without reading the full reference.
- A user who types `g` or `z` and hesitates sees the available continuations within ~500 ms, without any popup appearing during fluent typing. **(Amendment 2026-07-18: the popup was withdrawn; the pre-existing footer pending-prefix strip is the surface for this goal — see the Unit 3 amendment below.)**
- All new surfaces (help tabs, workflows header, which-key popup) are generated from the same const tables that drive dispatch, extending the existing bidirectional drift tests rather than adding hand-maintained parallel content. **(Amendment 2026-07-18: which-key popup withdrawn — the other two surfaces stand.)**
- Zero perceptible rendering cost: perf tripwires stay green.

## User Stories

- **As a new user**, I want help to open with a handful of task-phrased workflows ("Review a branch or commit", "Comment on a line") so that I can find features by intent instead of by key name.
- **As a new user**, I want `?` to show only the keys that work right now so that I'm not scanning bindings for modes I'm not in.
- **As a learning user**, I want a popup listing what can follow `g` or `z` when I pause mid-chord so that I can explore the prefix namespaces without memorizing them first. **(Amendment 2026-07-18: served instead by the pre-existing footer pending-prefix strip — see the Unit 3 amendment below.)**
- **As an experienced user**, I want the full every-context reference still one `Tab` away, and no popups when I type chords fluently, so that the redesign costs me nothing. **(Amendment 2026-07-18: no popups at all now, fluent or otherwise — the withdrawal generalizes this story.)**
- **As a keymap customizer**, I want the workflows header and which-key popup to display my remapped keys so that help never shows a binding that doesn't work. **(Amendment 2026-07-18: which-key popup withdrawn; the footer pending-prefix strip already reflects remaps via `Keymap::completions_for`.)**

## Demoable Units of Work

### Unit 1: Context-first tabbed help overlay

**Purpose:** Restructure `?` into a two-tab overlay so the default view answers "what can I do right now." Serves new and current users alike.

**Functional Requirements:**

- FR-1: The help overlay shall have two tabs: **"This context"** (default on open) and **"All keys"**. `Tab`/`Shift-Tab` and `h`/`l` shall switch tabs, added to the help modal key table (which is already config-remappable) and shown in its footer hints.
- FR-2: The "This context" tab shall render, in order: the common-workflows header (Unit 2), the bindings applicable to the mode/scope the overlay was opened from (diff scope in `Normal`/`Visual`, panel scope in `Panel`, the corresponding modal table when opened from a mode whose table binds `?`), and a "Works everywhere" section listing `Scope::Global` bindings. Existing capability-gating (hiding inapplicable rows) continues to apply.
- FR-3: The "All keys" tab shall render today's full grouped reference (diff groups, panel section, modal sections) unchanged in content.
- FR-4: The existing `/` filter shall work on both tabs, filtering the visible tab's sections live; scroll position and filter shall reset when switching tabs.
- FR-5: The existing bidirectional help/dispatch drift tests shall be extended to the tabbed structure: every binding reachable in a context must appear on that context's "This context" view, and the "All keys" tab must remain complete.

**Proof Artifacts:**

- Test: drift tests demonstrate per-context completeness of "This context" and total completeness of "All keys".
- CLI: journey transcript — `?` from the diff view, from the git panel, and from a modal shows three different context views, each fitting one 80×24 screen without scrolling, demonstrating the context-first promise.
- Test: filter test demonstrates `/` narrows the active tab and resets on tab switch.

### Unit 2: Common-workflows header

**Purpose:** Bridge intent to keys for users who don't know what they're looking for. Serves new users in their first minutes.

**Functional Requirements:**

- FR-6: A curated const table (target 5 entries, hard cap 7) shall pair an intent phrase with an `Action` (e.g., "Review a branch or commit" → open-review-launcher; "Comment on a line" → compose; "Stage this hunk" → stage; "Search the diff" → search; "Quit and print annotations" → quit).
- FR-7: The header shall render at the top of the "This context" tab, resolving each entry's displayed key(s) live from the effective (post-config-merge) keymap, so user remaps display correctly.
- FR-8: An entry whose action is unbound in the effective keymap shall be omitted from display, and a drift test shall fail if any curated entry's action is unbound in the *default* keymap — curation errors break the build, user unbinds degrade gracefully.
- FR-9: Entries whose action is capability-gated off in the current context (e.g., staging in a read-only view) shall be hidden by the same gating as regular help rows.

**Proof Artifacts:**

- Test: drift test demonstrates every curated entry resolves to a bound default-keymap action.
- Test: remap test demonstrates a `[keys.global]`/`[keys.diff]` override changes the key shown in the header.
- CLI: transcript of the header rendering with default bindings demonstrates the intent-phrased five-liner.

### Unit 3: Which-key popup for pending prefixes (withdrawn)

### Amendment (2026-07-18)

**Withdrawn.** Unit 3 (FR-10..FR-13) shipped (task 4.0, commits `e82355f`/`4912ca1`) and was then withdrawn by operator decision after dogfooding. The footer already renders every continuation for a pending `g`/`z` prefix instantly via `Keymap::completions_for` (`pending_hints`, `src/ui/footer.rs`) with capability gating. The popup showed the same data ~500 ms later, without gating — duplicative. The pre-existing footer pending-prefix strip remains the single surface for prefix continuations; no replacement surface is planned. The rest of this Unit's text is left below unaltered as the historical record of what was ratified and built.

**Purpose:** Make the `g`/`z` namespaces explorable in place. Serves learning users; invisible to fluent ones.

**Functional Requirements:**

- FR-10: When a prefix key (`g` or `z` — derived from the keymap table's two-key sequences, not hardcoded) is pending and no continuation has been typed for ~500 ms, a small popup shall render listing each bound continuation with its description, generated from the effective keymap.
- FR-11: The popup shall never block or delay input: the pending-prefix state machine's behavior is unchanged; typing a continuation, `Esc`, or an invalid key dismisses the popup exactly as it already resolves the pending prefix. The delay check shall piggyback on the existing render tick (no new threads or timers).
- FR-12: The popup shall reflect the effective keymap, including count prefixes composed with chords (e.g., pending `3g` shows the same continuations as `g`).
- FR-13: A drift test shall verify the popup's contents equal the set of two-key bindings for that prefix in the effective keymap — no hand-maintained popup list.

**Proof Artifacts:**

- Test: drift test demonstrates popup contents are derived from the keymap table.
- Test: state-machine tests demonstrate pending-prefix resolution is byte-identical with the popup feature present (fluent chords never render it).
- CLI: journey transcript — press `g`, wait, see the continuation popup, press `d` to jump to definition, demonstrating explore-then-execute.

## Non-Goals (Out of Scope)

1. **Configurable which-key delay**: the ~500 ms delay is a compile-time constant this round; a TOML knob is an explicit deferred follow-up (questions round 1, Q3) pending the config layer (spec 07) settling.
2. **New-user startup nudge**: explicitly rejected in questions round 1 (Q4) — the footer hint strip plus the redesigned `?` carry discoverability; no startup message, no first-run persistence.
3. **Keybinding changes**: this spec adds no new bindings outside the help modal's tab keys and changes no existing dispatch behavior.
4. **Which-key for modal contexts or single keys**: only the table-driven two-key prefixes (`g`, `z`) get popups; modal list navigation already has footer hints. **(Amendment 2026-07-18: moot — the popup itself was withdrawn; moved here for record only.)**
5. **Command palette**: still deferred; revisit only if launcher (spec 09) + this spec prove insufficient.
6. **Interactive/executable help** (pressing a key inside the overlay to run it): help remains display-only.

## Design Considerations

- Help overlay keeps its current chrome (centered, scrollable, `/` filter line); tab headers render like the Switcher/launcher tabs for idiom consistency.
- The workflows header is visually distinct from key sections (e.g., title "Common workflows", intent phrase left, key right) and never shows more than 7 lines.
- The which-key popup is small and anchored near the footer (not centered) so it reads as an input hint, not a modal interruption; it lists `key — description` rows in table order. **(Amendment 2026-07-18: popup withdrawn; the pre-existing footer pending-prefix strip already satisfies this idiom without a separate surface.)**
- "This context" opened from a modal shows that modal's table — the overlay must capture the origin mode at open (the same origin idea spec 09 generalizes).

## Repository Standards

- Data-driven invariants are the heart of this spec: every new surface renders from the existing const tables in `src/ui/keymap.rs` / `src/ui/modal_keys.rs`, with bidirectional drift tests extended accordingly — no parallel hand-maintained lists.
- Rust best practices per `docs/rust-best-practices.md`: no panics in production code, no blocking work on the render loop, presentation logic factored into pure functions unit-testable without constructing the app.
- The help modal key table changes stay config-remappable; new tab keys appear in its hints.
- All four gates before any commit (`cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`); conventional commits; the Unit 1 restructure (behavior) and any incidental refactors land separately.
- Perf tripwires in `src/ui/perf_tests.rs` stay green; the which-key delay check is O(1) per tick and the popup renders only while a prefix is pending. **(Amendment 2026-07-18: the which-key delay check/popup no longer exist post-withdrawal; the perf tripwires that mattered here now simply cover the pre-existing footer strip.)**

## Technical Considerations

- **Dependency on spec 09**: the "Works everywhere" section (FR-2) renders `Scope::Global` bindings, which spec 09 introduces. Planned order is 09 then 10. If built before 09 lands, the section falls back to rendering the currently-duplicated cross-scope rows under the same heading — the visual contract holds either way; state which path was taken in the proof artifacts.
- **Timing without timers**: the render loop already ticks; the which-key check compares a stored `Instant` (set when the prefix went pending) against now, on the existing tick. No threads, channels, or timer wheels. Tests exercise the state machine with injected elapsed values rather than real sleeps to stay flake-free. **(Amendment 2026-07-18: this timing mechanism was removed along with the popup; historical record only.)**
- **Prefix discovery, not hardcoding** (FR-10): the set of which-key prefixes is derived from the keymap's two-key sequences so a future prefix (or a user remap creating one) gets the popup for free. **(Amendment 2026-07-18: `Keymap::which_key_prefixes`/`continuations_for` were removed with the popup — no other caller depended on them.)**
- **Origin capture for context help**: reuses the origin concept spec 09 generalizes from `EndReviewOrigin`; the overlay records which mode/scope it opened over and renders that context's view.
- No new dependencies. No latest-standards research needed beyond the in-session survey of which-key prior art (vim/helix/magit) that shaped the questions.

## Security Considerations

No specific security considerations identified — display-only presentation changes over existing in-memory tables; no new I/O, subprocess, or persistence surface.

## Success Metrics

Per this repo's UX-outcome verification convention, metrics are user journeys with persisted evidence:

1. **Journey A (context help)**: `?` from diff view, git panel, and one modal each shows a distinct one-screen (80×24) context view with the workflows header and a "Works everywhere" section — transcripts persisted.
2. **Journey B (intent to action)**: starting from the workflows header alone, a scripted journey executes "Review a branch or commit" end-to-end (header names the key → key opens the launcher) — transcript persisted.
3. **Journey C (explore a prefix) — withdrawn (Amendment 2026-07-18)**: originally, `g` + pause shows the continuation popup; choosing `d` performs goto-definition; a fluent `gd` in the same session never renders the popup — transcript persisted. Withdrawn along with Unit 3; the footer pending-prefix strip already demonstrates this journey without a popup.
4. **Zero drift, zero regression**: extended drift tests, all pre-existing help tests, perf tripwires, and all four cargo gates pass. **(Amendment 2026-07-18: scope no longer includes the withdrawn which-key popup tests.)**

## Open Questions

1. **Workflows header final wording**: the five intent phrases are editorial; the const table in FR-6 ships with the examples given and can be re-worded freely without structural change. Non-blocking.
2. **Which-key delay value**: shipping at 500 ms as a compile-time constant; if dogfooding says it's wrong, adjusting is a one-line change (and the TOML knob remains the deferred escalation). Non-blocking. **(Amendment 2026-07-18: moot — dogfooding concluded the popup itself was duplicative of the footer strip and withdrew it rather than retuning the delay.)**
3. **"This context" for rare modes**: modals that don't bind `?` (free-text inputs like compose/search) simply never open help from within; their tables remain reachable on the "All keys" tab. Recorded as an accepted asymmetry. Non-blocking.
