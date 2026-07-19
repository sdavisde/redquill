# 13-tasks-forge-integration.md

Task list for `13-spec-forge-integration.md`. Parent tasks are vertical slices matching the spec's Demoable Units; each states a plain-language user verification. Live forge-write dogfood proofs (4.0, 6.0) are performed by the user — the designated GitHub target is PR #25 (`sdavisde/redquill#25`); agent-side proofs stop at fakes and scratch repos per the repo guardrails.

## Relevant Files

| File | Why It Is Relevant |
| --- | --- |
| `src/forge/mod.rs` | New module root: `ForgeProvider` trait, `PullRequest`/`Capabilities`/`Verdict` types, typed `ForgeError`. No TUI types. |
| `src/forge/remote_url.rs` | New pure parser: origin URL → hostname (https/ssh/scp-like forms). TDD. |
| `src/forge/detect.rs` | New provider-resolution ladder (known hosts → CLI credential lookup) behind an injectable credential-checker seam. |
| `src/forge/github.rs` | New GitHub provider: `gh` argv construction, PR list/detail/thread JSON parsing, review payload construction. |
| `src/forge/gitlab.rs` | New GitLab provider: `glab` argv construction, MR/discussions parsing, position hashes, draft-notes submit. |
| `src/forge/threads.rs` | New thread model: roots/replies ordering, resolved/outdated state, diff-anchor mapping with file-level fallback. |
| `src/forge/*_tests.rs` | Split test modules per repo convention (`#[cfg(test)] #[path]`) for the pure parsers/builders above. |
| `src/git/runner.rs` | Add PR-ref fetch (fixed-shape refspecs, forced update confined to `redquill/pr/*`), managed-branch list/delete. |
| `src/git/remote.rs` | Closed-type remote op additions for the PR fetch shapes. |
| `src/review/store.rs` | Schema v3: forge metadata, per-annotation published flag, draft replies; v2 silent migration. |
| `src/review/reconcile.rs` | Unchanged logic, exercised by PR head-move reconciliation tests. |
| `src/annotate/model.rs` | Published marker on annotations; reply-draft representation. |
| `src/annotate/markdown.rs` | Byte-format must not change — regression tests only. |
| `src/ui/review_launcher.rs` | Third tab: ordered tab cycling, PR list state, async load, degraded/empty bodies, finished-reviews footer. |
| `src/ui/review_launcher_modal.rs` | Rendering for the PRs tab bodies and footer. |
| `src/ui/review_session.rs` | PR-flavored session entry: fetch → managed branch → worktree → reconcile → reroot; head-move recreate. |
| `src/ui/forge_threads.rs` | New: thread overlay store + gutter markers + expandable thread view (UI side of the seam). |
| `src/ui/forge_submit.rs` | New: submit modal (batch preview, capability-driven verdicts, summary), submit-sequence driver. |
| `src/ui/modal_keys.rs` | Key tables for PRs tab, thread view, submit modal, cleanup confirm — drives dispatch, footer hints, `?` help. |
| `src/ui/keymap.rs` | New actions (`submit-forge-review`, thread nav/reply, cleanup) with non-shadowing defaults. |
| `src/ui/stage_ops.rs` | Async fetcher seams for PR list / thread fetch (fake-able, sync fallback), mirroring existing patterns. |
| `src/ui/app.rs` | Single-flight + generation-guard state for forge tasks; poll wiring. |
| `docs/forge-setup.md` | New user docs: providers, zero-config detection, hosted-instance walkthrough, troubleshooting. |
| `README.md` | Link to forge-setup docs. |
| `CLAUDE.md` | Guardrail amendment (FR-21), shipped with task 4.0. |

### Notes

- Gates before every commit: `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`. Conventional commits; refactors land separately from behavior.
- Integration tests only in canonicalized tempdir scratch repos; worktree/fetch flows never touch the host repo. Agents never invoke forge writes.
- TDD (failing test first) for all pure surfaces: URL parser, JSON parsers, payload/position-hash builders, finished-set detection, store round-trips.
- New keys ride the shared tables; help/footer drift tests must stay green; perf tripwires must stay green.

## Tasks

### [x] 1.0 Press `R` and see the repo's open pull requests — or exact setup instructions when the forge isn't reachable (GitHub)

**User verification:** In a GitHub-backed repo with `gh` logged in, press `R`, switch to the new **Pull Requests** tab, and see the repo's open PRs (number, title, author, branch, updated time), filterable with `/`. When the repo simply has no open PRs, the tab says so plainly ("No open pull requests on org/repo") — clearly success, not an error. In a repo with no forge or no login, the same tab shows plain instructions naming the exact command to run — never a blank screen. A new `docs/forge-setup.md` page explains setup, including hosted GitLab.

**Covers:** FR-1 – FR-6 (Unit 1).

#### 1.0 Proof Artifact(s)

- Test: `src/forge/` URL-parser and provider-ladder unit tests pass (`cargo test forge`) demonstrates hostname parsing and zero-config detection (FR-2).
- Test: layering-guard test (greps `src/forge/` for `ratatui`/`crossterm` imports, fails on hit) passes demonstrates the module boundary (FR-1).
- Test: PR-list JSON fixture-parsing tests pass demonstrates typed rows from `gh` machine output (FR-4).
- Test: UI tests with a fake provider render each degraded-state prescription with the hostname interpolated, plus the zero-open-PRs empty-state line distinct from diagnostics (FR-5).
- Test: docs-drift check (forge-setup.md exists, README links it) passes demonstrates the docs contract (FR-6).
- CLI: journey transcript — launcher in a scratch repo with no forge remote, and with an unresolvable host, showing the prescription bodies (FR-3, FR-5).
- CLI: user dogfood capture — this repo's PRs tab listing PR #25 live (FR-4).

#### 1.0 Tasks

- [x] 1.1 Create `src/forge/mod.rs`: `ForgeError` (thiserror), `PullRequest`, `Verdict`, `Capabilities`, and the `ForgeProvider` trait (list PRs, PR detail, fetch threads, submit review, capabilities). Add the layering-guard test (no TUI crate imports under `src/forge/`).
- [x] 1.2 TDD `src/forge/remote_url.rs`: hostname extraction for `https://`, `ssh://`, and `git@host:path` origin URLs; strict hostname charset validation; malformed input → typed error. Add `git remote get-url origin` read to the git layer (machine output, background-capable).
- [x] 1.3 TDD `src/forge/detect.rs`: resolution ladder behind an injectable `CredentialChecker` trait — known hosts first, then per-CLI credential lookup (gh: `gh auth token --hostname <h>` exit-status only, stdout discarded unread; glab: placeholder seam, finalized in 6.2), ambiguous/none → `Unresolved` with the reason. Process-lifetime cache; all lookups off the render loop with a timeout.
- [x] 1.4 TDD `src/forge/github.rs` listing: fixed argv for `gh pr list` with JSON fields (number, title, author, headRefName, isDraft, updatedAt), fixture-based parse tests into typed rows; prompts disabled (`GH_PROMPT_DISABLED=1`, `NO_COLOR=1`), kill-on-drop.
- [x] 1.5 Refactor-only commit: replace `LauncherTab::toggle`'s two-tab assumption with ordered cycling over all variants (existing keys unchanged, last-used-tab memory intact); identical test counts before adding the third variant.
- [x] 1.6 Add the **Pull Requests** tab: always-visible third variant; async list load through a new `stage_ops` fetcher seam + `BackgroundTasks` with single-flight and generation guard (mirror the Commits tab); shared `/` filter and motion layer; `Enter` stubbed to a status line until 2.0.
- [x] 1.7 Render the tab bodies: loading line, PR rows, zero-open-PRs empty state naming `org/repo`, and the four degraded-state prescriptions with real hostname interpolation; UI tests with a fake provider for every state.
- [x] 1.8 Write `docs/forge-setup.md` (providers, detection ladder, hosted-instance walkthrough, per-state troubleshooting), link from README, add the docs-drift existence/link test.
- [x] 1.9 Capture the scratch-repo journey transcripts; run all four gates; hand the live-listing dogfood step to the user (PRs tab showing PR #25).

### [ ] 2.0 Select a PR and land in a full review session, with author pushes detected on reopen

**User verification:** Press `Enter` on a PR and arrive in the familiar review screen (worktree-backed, per-file accept/defer, annotations) without touching git yourself — including PRs from forks. If the author pushes new commits, reopening the PR shows "PR updated — N accepted file(s) changed" and those files drop back to needing re-review. Going offline doesn't destroy anything: the tab says why, and an existing checkout reopens clearly labeled stale.

**Covers:** FR-7 – FR-11 (Unit 2).

#### 2.0 Proof Artifact(s)

- Test: refspec-construction tests pass demonstrates only `redquill/pr/<n>` refs are writable/deletable (FR-8).
- Test: tempdir integration tests (bare origin advertising a PR-style ref) pass demonstrates checkout → worktree → review, head-move → recreate → `ChangedSinceAccepted`, fetch-failure → stale-labeled session (FR-7, FR-9, FR-10).
- Test: store round-trip tests pass demonstrates v2 → v3 silent migration and byte-stable output (FR-11).
- CLI: journey transcript on scratch repos — `R` → PRs → `Enter` into a session; simulated author push; reopen shows the update line and demoted files (FR-9).

#### 2.0 Tasks

- [ ] 2.1 TDD git-layer PR fetch: a closed `PrRef` type (provider ref pattern + integer PR number) producing fixed-shape argv for `git fetch origin <special-ref>:refs/heads/redquill/pr/<n>` (forced form only for that namespace — structurally unable to name any other ref), plus plain base-ref fetch; `GIT_TERMINAL_PROMPT=0`; managed-branch list/delete helpers restricted to the `redquill/pr/` prefix.
- [ ] 2.2 TDD `src/review/store.rs` schema v3: optional forge block on `PersistedReview` (provider, host, number, last head SHA); v2 files load silently with absent fields; deterministic serialization; byte-stable round-trip for non-forge reviews; bump `SCHEMA_VERSION`, keep corrupt-file salvage behavior.
- [ ] 2.3 Wire `Enter` on a PR row: guards (in-session, single-in-flight) → head fetch → base fetch → `ensure_review_worktree(redquill/pr/<n>)` → reconciled state load → reroot onto `DiffTarget::Review` with the PR's base; store forge metadata in v3.
- [ ] 2.4 Head-move handling: compare fetched head SHA to stored `last head SHA`; on move, remove managed worktree → forced namespace ref update → re-add worktree → reconcile; emit "PR updated — N accepted file(s) changed"; wire the existing manual refresh action to re-run the same check mid-session.
- [ ] 2.5 Fetch-failure path: existing local state untouched, one-line diagnostic, stale-labeled session entry when a prior worktree exists.
- [ ] 2.6 Tempdir integration tests: scratch bare origin advertising `refs/pull/1/head`-style refs — happy path, fork-style head (no matching origin branch), head-move demotion, fetch failure; canonicalized paths.
- [ ] 2.7 Capture the journey transcript (checkout → author push → reopen shows demotions); run all four gates.

### [ ] 3.0 See the PR's existing conversations inside the diff and draft replies to them

**User verification:** Reviewing a PR that already has comments, you see markers at the commented lines; opening one shows the whole conversation in order (a 5-reply back-and-forth reads top-to-bottom, replies under the comment they answer), with resolved/outdated threads collapsed and labeled. You can draft a reply in the terminal; it appears in your annotation panel marked as a reply and survives quitting and reopening. If comments can't be fetched, review continues with a one-line notice.

**Covers:** FR-12 – FR-16 (Unit 3).

#### 3.0 Proof Artifact(s)

- Test: thread-model fixture tests pass demonstrates root/reply ordering, resolved/outdated labels, file-level fallback for unmappable anchors (FR-12).
- Test: UI fake-provider tests pass demonstrates markers, expand/collapse, reply drafting, draft persistence across reopen, fetch-failure notice (FR-13 – FR-15).
- Test: keymap/help drift tests pass with the new thread/reply actions demonstrates table-driven keys and `?` coverage (FR-16).
- CLI: journey transcript — scripted fake-provider session showing an ordered 5-and-5 thread and a drafted reply in the annotation panel (FR-12, FR-14).

#### 3.0 Tasks

- [ ] 3.1 TDD `src/forge/threads.rs`: thread model (root + ordered replies via `in_reply_to` linkage, author, timestamp, resolved/outdated state) and anchor mapping (path + side + line → diff anchor; unmappable → file-level), built from GitHub review-comment fixture JSON.
- [ ] 3.2 GitHub thread fetch: fixed argv `gh api` call for PR review comments, async through a fetcher seam; overlay store separate from annotations — never persisted, never serialized to stdout (regression-guard test on `markdown.rs` output).
- [ ] 3.3 UI overlay: single-cell gutter markers on annotated lines, expandable thread view (conversation order, nested replies, collapsed resolved/outdated with labels), next/prev-thread navigation; keys chosen against the shared tables (no shadowing), footer hints + `?` sections, drift tests green.
- [ ] 3.4 Reply drafting: compose flow consistent with annotation compose; drafts target the thread root, are editable/deletable, appear in the annotation list with a reply marker, persist in schema v3 (thread id + body) and restore on reopen.
- [ ] 3.5 Published-copy dedupe (FR-15): annotations marked published are not rendered as local annotations at their anchor once the forge copy is present in fetched threads.
- [ ] 3.6 Fetch-failure notice ("comments unavailable") without blocking review entry; scripted fake-provider journey transcript; all four gates.

### [ ] 4.0 Publish the whole review — comments, replies, verdict — from one previewed confirm screen (GitHub)

**User verification:** A "submit review" key opens a preview listing every unpublished comment and reply, a verdict choice (comment / approve / request changes), and an optional summary. Nothing is sent until you confirm; after confirming, the review appears on GitHub exactly as previewed (dogfood target: PR #25), and the published items are marked locally so re-submitting sends nothing twice. Quitting still prints your annotations to stdout exactly as today. CLAUDE.md's guardrails visibly document the new, narrow write ceiling.

**Covers:** FR-17 – FR-21 (Unit 4).

#### 4.0 Proof Artifact(s)

- Test: payload-construction tests pass demonstrates exact review JSON for a mixed batch (line/range/old-side/hunk, classification prefixes, verdict, summary) (FR-18).
- Test: submit-sequence fake-provider tests pass demonstrates review-then-follow-ups ordering, per-item published marking, mid-failure stop + duplicate-free resume (FR-19, FR-20).
- Test: store tests pass demonstrates published flags round-trip and exclusion from re-submit; stdout serialization tests unchanged (FR-20).
- Diff: CLAUDE.md guardrail amendment ships in the same change; docs-drift test asserts the amended ceiling text exists (FR-21).
- CLI: user dogfood — live submit to PR #25; transcript + forge screenshot match the preview (FR-17 – FR-19). Agent-side proofs stop at recorded-argv fakes per the agent ceiling.

#### 4.0 Tasks

- [ ] 4.1 TDD GitHub review payload builder in `src/forge/github.rs`: `Line`/`Range` → `side`/`line`/`start_line`/`start_side`, `Hunk` → new-side range, classification body prefixes, `event` from verdict, summary body; file-target annotations excluded from the array and routed to the follow-up set; worktree-anchored targets excluded as local-only.
- [ ] 4.2 Annotation published state: marker on `Annotation`/`PersistedAnnotation` (v3, absent = unpublished), excluded from future submits, rendered with a published indicator in the annotation list; stdout `markdown.rs` byte-format untouched (regression tests).
- [ ] 4.3 Submit modal (`src/ui/forge_submit.rs`): grouped-by-file batch preview (annotations with anchors + classifications, draft replies, local-only and posts-as-file-comment labels), capability-driven verdict picker, summary input, target line (`#N on host/org/repo`); `submit-forge-review` keymap action (non-shadowing default) live only in forge-PR review sessions; drift tests.
- [ ] 4.4 Submit sequence driver: one reviews-endpoint POST (comments array + event + body) → sequential follow-ups (file comments via `subject_type: file`, replies via the replies endpoint), each marked published on success; mid-sequence failure stops, reports published/unpublished split, resume sends only remainder; fake-provider tests for ordering, marking, resume.
- [ ] 4.5 Amend CLAUDE.md guardrails in the same change: product ceiling gains exactly the confirmed submit flow + `redquill/pr/*` namespace writes; forbidden list extended (merge/close, comment edit/delete, resolve); agent ceiling restated (no forge writes, fakes + scratch repos only). Add a docs-drift test asserting the guardrail section names the forge-submit ceiling and the `redquill/pr/` namespace (same pattern as the FR-6 docs check).
- [ ] 4.6 Run all four gates; hand live dogfood to the user: submit a mixed-batch review to PR #25, capture transcript + screenshot, then a second submit proving nothing re-sends.

### [ ] 5.0 Clean up finished PR reviews safely from the launcher

**User verification:** After a reviewed PR merges or closes, the Pull Requests tab shows "N finished review(s)"; pressing the cleanup key lists them (with a warning for any unpublished comments) and, only after you confirm, removes each one's worktree, branch, and saved state — verifiable by the tab count reaching zero and `git branch` no longer showing `redquill/pr/…`. Declining leaves everything untouched.

**Covers:** FR-22 – FR-24 (Unit 5).

#### 5.0 Proof Artifact(s)

- Test: finished-set detection tests pass demonstrates managed-branch vs open-PR set difference, never-reviewed PRs excluded (FR-22).
- Test: tempdir integration test passes demonstrates confirm-gated deletion of worktree + branch + state entry, unpublished-annotation warning, decline path intact (FR-23), per-entry failure continuation (FR-24).
- CLI: journey transcript — before/after `git worktree list`, `git branch`, and `review-state.json` around a confirmed cleanup (FR-23).

#### 5.0 Tasks

- [ ] 5.1 TDD finished-set detection: managed `redquill/pr/*` branches with a state entry, minus the open-PR set from the existing list call (no extra network); pure function + tests incl. never-reviewed and still-open cases.
- [ ] 5.2 Finished-reviews footer on the PRs tab (count line, renders alongside list or empty state) and a cleanup keymap action opening the confirm modal: per-entry PR number/title, worktree path, unpublished-annotation count when nonzero.
- [ ] 5.3 Confirmed deletion sequence per entry: `git worktree remove` → `worktree prune` → managed-branch delete → state-entry removal (v3 save); per-entry failure → one-line diagnostic, continue, end-of-run outcome summary; decline path mutates nothing.
- [ ] 5.4 Tempdir integration tests for confirm, decline, unpublished-warning, and locked-worktree continuation; journey transcript with before/after `git worktree list` / `git branch` / `review-state.json`; all four gates.

### [ ] 6.0 The same experience end-to-end on GitLab, including self-managed hosts with zero config

**User verification:** In a GitLab-backed repo (gitlab.com or a self-managed host where `glab` is logged in), the same tab lists open MRs with no redquill configuration; reviewing, threads, replies, and publishing all work, with the verdict choices honestly limited to what GitLab supports (comment / approve). The setup docs and in-tab prescriptions name `glab` commands for the exact host.

**Covers:** FR-25 – FR-28 (Unit 6).

#### 6.0 Proof Artifact(s)

- Test: position-hash construction tests pass demonstrates added/removed/context line mapping against fixture `diff_refs` (FR-25, FR-26).
- Test: fake-provider submit tests pass demonstrates draft-create → bulk-publish → approve ordering, nothing-published-on-early-failure, sequential fallback (FR-26).
- Test: `glab` MR JSON fixture-parsing tests pass demonstrates typed listing/detail rows (FR-25).
- Test: capability-rendering tests pass demonstrates the submit modal offers comment/approve only on GitLab (FR-27).
- CLI: user dogfood — live list → review → submit on a scratch GitLab project; hosted-instance detection capture (read-only) for the client host (FR-25, FR-26, FR-28).

#### 6.0 Tasks

- [ ] 6.1 TDD `src/forge/gitlab.rs` reads: fixed argv for `glab mr list -F json` / MR detail (incl. `diff_refs`), fixture-based parse tests into the same typed rows; prompts disabled, kill-on-drop.
- [ ] 6.2 Resolve the glab credential-lookup open question against the pinned glab version (prefer a local-only command; document the choice in `detect.rs`); fill the 1.3 seam; add ladder tests for a glab-authenticated custom host.
- [ ] 6.3 Wire GitLab through Units 1–2: `refs/merge-requests/<iid>/head` in the `PrRef` type, listing/checkout/head-move/cleanup flows passing existing tests against the GitLab provider fake.
- [ ] 6.4 TDD discussions import: position-hash → thread-model anchor mapping (added → `new_line`, removed → `old_line`, context → both; file-type positions → file-level), fixture JSON from the discussions API.
- [ ] 6.5 TDD GitLab submit: position-hash builder from annotation side/line + `diff_refs`; draft-notes create per item → `bulk_publish` → `glab mr approve` on approve verdict; sequential-discussions fallback with per-item published marking when draft notes are unavailable; capability flags (comment/approve) driving the 4.3 modal, with fallback disclosure; fake-provider tests for ordering, early-failure, fallback.
- [ ] 6.6 Extend `docs/forge-setup.md` + FR-5 prescriptions for glab/hosted GitLab; run all four gates; hand live dogfood to the user (scratch GitLab project submit; read-only detection capture on the client host).
