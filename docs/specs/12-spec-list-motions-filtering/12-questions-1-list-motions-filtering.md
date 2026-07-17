# 12 Questions Round 1 - List Motions & Filtering

This round was asked and answered interactively in-session on 2026-07-17 and codified here for the record. Selected answers are checked. Prior context: the parity audit (see spec 11 questions round 1) found that panel and modal lists lack fast navigation and filtering; the user scoped gaps 3, 7, and 8 into this spec.

## 1. Filter Trigger in Lists With Letter-Key Verbs

The annotation list binds `e`/`d`, the staging panel binds `Space`, the switcher binds `h`/`l` — printable keys can't silently become query input. How is filtering triggered?

- [x] (A) `/` enters filter mode (help-overlay idiom): press `/` to type a fuzzy query, `Esc` clears/exits, `Enter` locks the filter; letter verbs keep working outside filter mode.
- [ ] (B) Always-on typing (file-finder idiom): every printable char extends the query; letter verbs move to arrows/Ctrl chords. Zero-friction but changes existing muscle memory in those lists.

**Recommended answer(s):** [(A)] — accepted.

**Why this was recommended:**

- `/`-to-filter is already shipped in the help overlay, so it extends an existing idiom instead of introducing a second one.
- `(B)` would force re-binding every letter verb in every list it touches — a migration cost with no offsetting benefit for lists this size.
- Matching itself is fuzzy (reusing the file finder's matcher), per the user's direction that branch filtering should feel like the finder.

## 2. Scope of Fast Navigation (Motion Layer Reframe)

Original question: should panel fast-nav be paging + `gg`/`G` only (recommended, with count prefixes deferred), or include count prefixes now?

- [ ] (A) Paging + `gg`/`G` only; counts deferred.
- [ ] (B) Include count prefixes via per-context generalization.
- [x] (E) Other — user reframed the question: "the git panel is a type of buffer that should support vim motions in the same way our diff view does. I'd expect that in the future if we enabled better movement commands, they would apply everywhere."

**Resolution adopted:** motions become a **shared layer** — the motion set (cursor step, half/full page, top/bottom, count prefixes) is defined once and consumed by every buffer-like context (diff view, git panel, modal lists). Count prefixes are therefore in scope naturally: the existing count machinery moves into the shared layer rather than being duplicated per context. A drift test asserts every consuming context handles the full motion set, so future motion additions propagate everywhere or fail the build. This supersedes the original A/B framing (both options were per-context thinking).

## 3. Do Spec 09's Review-Launcher Tabs Get the Filter?

- [x] (A) Yes, as spec-12 targets: spec 09 stays frozen and ships arrow-only; spec 12 lists the launcher's Branches/Commits tabs among its filter/motion targets and lands after 09.
- [ ] (B) No — filter only the pre-existing lists; revisit the launcher after dogfooding.

**Recommended answer(s):** [(A)] — accepted.

**Why this was recommended:**

- Keeps spec 09's passed planning audit intact while ensuring the launcher doesn't become a new parity gap the moment it ships.
- Establishes the ordering contract explicitly: 09 → 12 for the launcher-adoption unit.
