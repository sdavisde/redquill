# 10 Questions Round 1 - Help & Discoverability Redesign

This round was asked and answered interactively in-session on 2026-07-17 and codified here for the record. Selected answers are checked. Prior context ratified in the same session: `?` becomes context-first (common-workflows header + current-context keys + global keys, full reference demoted), and which-key popups for the `g`/`z` prefixes are in scope. The "works everywhere" global-keys section depends on `Scope::Global` from spec 09.

## 1. Reaching the Full Reference

How should the full every-binding reference be reachable from the new context-first `?` view?

- [x] (A) Tabs inside the overlay: `?` opens on a "This context" tab (workflows header + current-mode keys + global keys); `Tab`/`h`/`l` switches to an "All keys" tab holding today's full grouped reference. Matches the Switcher and Review-launcher tab pattern.
- [ ] (B) Toggle key expands the full reference in place below the short view. No tab chrome, but a longer scroll and less clear state.
- [ ] (C) Two separate overlays (`?` short, `g?` full). Cleanest separation but two entry points to learn.

**Recommended answer(s):** [(A)] — accepted.

**Why this was recommended:**

- Tabs are now an established in-app idiom (git panel, Switcher, spec-09 launcher); reusing it costs new users nothing.
- `(B)` blurs the "short by default" promise; `(C)` doubles the entry points the redesign is trying to reduce.

## 2. Source of the "Common Workflows" Header

How should the workflows header lines be defined?

- [x] (A) A hand-curated const table (~5 entries: intent phrase + `Action`). Keys render live from the effective keymap so user remaps display correctly; a drift test fails if a referenced action becomes unbound.
- [ ] (B) Auto-derived from the existing help groups / footer-rank data. Zero curation burden, but intent phrasing ("Review a branch or commit") cannot be derived mechanically.

**Recommended answer(s):** [(A)] — accepted.

**Why this was recommended:**

- Matches the repo's data-driven-invariants convention exactly: one const table drives display, a bidirectional test prevents drift.
- Editorial intent-phrasing is the entire value of the header; `(B)` can only produce key-centric lines, which is what the redesign is escaping.

## 3. Which-Key Popup Trigger

When should the which-key popup for pending prefixes (`g`, `z`) appear?

- [x] (A) After a short delay (~500 ms) once a prefix is pressed with no continuation. Fast typists never see it; hesitation summons help. Classic vim/helix behavior.
- [ ] (B) Immediately on every prefix press. Maximum discoverability but flashes constantly for users with muscle memory.
- [ ] (C) Delay with a TOML config key to adjust or set 0. Small config-layer addition on top of `(A)`.

**Recommended answer(s):** [(A)] — accepted.

**Why this was recommended:**

- `(A)` is the smallest slice that delivers the explorability win; `(B)` punishes experts.
- `(C)` remains an easy follow-up once dogfooding says the default delay is wrong — deferring it avoids touching the config layer (spec 07, still unsettled) now.

## 4. New-User Nudge Toward `?`

Should there be an explicit nudge toward `?` beyond the existing footer hints?

- [ ] (A) Startup status message ("press ? for help") until the first keypress.
- [x] (B) No extra nudge — the footer hint strip already surfaces keys contextually; rely on it plus the redesigned `?`.
- [ ] (C) First-run-only nudge via a persisted flag.

**Recommended answer(s):** [(A)] — user chose (B) instead.

**Why (A) was recommended and (B) was chosen:**

- `(A)` was recommended as a one-line, zero-state hint; the user preferred zero new surface, trusting the footer strip (which already ranks and displays context hints) to do this job.
- `(C)` was rejected by both: persistence machinery for a single hint is not worth it.
