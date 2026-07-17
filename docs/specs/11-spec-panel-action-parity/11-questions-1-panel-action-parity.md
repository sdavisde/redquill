# 11 Questions Round 1 - Panel Action Parity

This round was asked and answered interactively in-session on 2026-07-17 and codified here for the record. Selected answers are checked. Prior context: an Opus parity audit of the codebase found nine cross-context gaps (same intent expressible in one context but not a sibling); the user triaged them, and a packaging question split the accepted work into spec 11 (this spec — mechanical action parity) and spec 12 (shared list motions & filtering).

## 1. Gap Triage (user-directed scope)

Which of the audited parity gaps are in scope for fixing?

- [x] Gap 1 — stage/unstage the highlighted file from the git panel (`Space`/`S`), the user's original report. **In (spec 11).**
- [x] Gap 2 — accept/defer the highlighted file from the panel in review mode (`Space`/`S`/`d`); panel already renders tri-state markers it can't act on. **In (spec 11).**
- [x] Gap 3 — fast navigation in panel lists (paging, `gg`/`G`). **In — reframed and moved to spec 12** (see spec 12 questions round: motions become a shared layer, not panel-scope rows).
- [x] Gap 4 — `Esc` doesn't leave the git panel. **In (spec 11).**
- [x] Gap 5 — `s` (staging panel) and `/` (search) unbound in panel scope. **In (spec 11).**
- [x] Gap 6 — annotations render inline in the diff but can only be edited/deleted from the annotation list; `c` on an annotated line creates a duplicate. **In (spec 11).**
- [x] Gap 7 — annotation list / staging panel / accepted panel lack filtering and paging. **In (spec 12)** — user: "anywhere we're rendering a list like those places I do think it makes sense to support filtering at least, if not also paging."
- [x] Gap 8 — switcher/branch lists lack type-to-filter. **In (spec 12)** — user: "similar UX to the file finder in terms of fuzzy searching."
- [ ] Gap 9 — comment on a file from the git panel (`c` means commit there). **Out** — user: "I think its okay that you can't annotate from the git panel directly."

**User verdict:** "1–6 feel like clear wins to me."

## 2. Packaging

How should the accepted work be packaged?

- [x] (A) Two specs: 11 "panel action parity" (mechanical binding/routing fixes, lands right after spec 09) + 12 "list motions & filtering" (shared mechanisms, more design surface).
- [ ] (B) One spec 11 covering everything — fewer artifacts but mixes trivial binding rows with cross-cutting components, bigger audit/validation blast radius.

**Recommended answer(s):** [(A)] — accepted. Mirrors the 09/10 two-spec pattern the user chose earlier the same day.
