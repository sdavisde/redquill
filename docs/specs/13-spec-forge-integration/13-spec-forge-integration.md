# 13-spec-forge-integration.md

## Introduction/Overview

Branch review mode (spec 08) reviews *a local branch*; the Review launcher (spec 09) reserved a future Pull Requests tab. This spec ships that tab: a **Pull Requests** tab in the global `R` launcher that lists the repo's open PRs/MRs (GitHub and GitLab), turns a selected PR into a worktree-backed review session using the unchanged spec-08 machinery, renders teammates' existing review comments as read-only threads inside the diff, lets the reviewer draft replies and annotations locally, and publishes everything as one submitted review — verdict included — without leaving the terminal.

All forge communication shells out to the official CLIs (`gh`, `glab`) on PATH, exactly as redquill shells out to `git`: auth, SSO, and self-hosted instances are inherited from the CLIs rather than reimplemented. Provider detection is zero-config, including for hosted instances (e.g. a client's self-managed GitLab).

## Goals

- Open a teammate's PR for review in 3 keystrokes from anywhere (`R`, move, `Enter`), with the worktree, base resolution, tri-state file review, and annotations all behaving identically to a local branch review.
- Show the full conversation state of the PR (threads, replies, in order) inline during review, fetched live, so terminal reviewers are never blind to feedback already given.
- Publish a complete review (line comments from annotations, thread replies, verdict, summary) in one explicit, previewed, confirmable action — never implicitly, never on quit.
- Zero-config provider detection: `github.com`/`gitlab.com` by hostname; self-hosted instances by asking the CLIs which one holds credentials for the host; unresolvable states produce copy-pasteable setup prescriptions, not dead UI.
- Keep the write ceiling explicit and narrow: the only new forge write is "submit one review on the PR under review"; all new git refs live in a redquill-owned namespace; agents working in this repo may never invoke forge writes.

## User Stories

- **As a reviewer on a team**, I want to press `R` and see the repo's open PRs so that starting a code review doesn't require a browser, a `git fetch`, or knowing the branch name.
- **As a reviewer of a fork PR**, I want redquill to fetch the PR's commits regardless of where the author's branch lives so that fork contributions review identically to same-repo branches.
- **As a reviewer returning to a PR**, I want redquill to notice the author pushed new commits and tell me which files I'd accepted have changed so that I re-review only what moved.
- **As a participant in an ongoing discussion**, I want existing comment threads shown at their diff positions, in conversation order, with my drafted replies queued locally so that a 5-reply back-and-forth reads like a conversation, not a scavenger hunt.
- **As a careful reviewer**, I want one preview screen showing exactly what will be published — every comment, every reply, the verdict — before anything leaves my machine so that a half-formed thought never lands on a teammate's PR by accident.
- **As a consultant working on a client's self-managed GitLab**, I want the PRs tab to tell me the exact `glab auth login --hostname …` command for my host so that setup is one prescribed step, not a docs excavation.

## Demoable Units of Work

### Unit 1: `forge` module, provider detection, and the Pull Requests tab (GitHub)

**Purpose:** Establish the `src/forge/` layer and make open PRs visible and understandable in the launcher, including every degraded state. Serves discovery and setup.

**Functional Requirements:**

- FR-1: A new `src/forge/` module shall own all forge logic behind a `ForgeProvider` trait (list PRs, read PR detail, fetch comment threads, submit review, capability flags). No TUI types may appear in `forge/` signatures or imports (same rule as `git/`), and all CLI invocations shall build argv from closed/typed values (PR numbers as integers, hostnames validated against a strict charset) — never string-assembled command lines.
- FR-2: A pure remote-URL parser shall extract the hostname from `origin`'s URL (`https://`, `ssh://`, and `git@host:path` forms), developed TDD like other parsers. Provider resolution ladder: (a) `github.com` → GitHub, `gitlab.com` → GitLab; (b) unknown host → ask each CLI whether it holds credentials for that host (`gh auth token --hostname <h>` exit code, stdout discarded unread; glab via its cheapest credential lookup — see Technical Considerations); exactly one CLI knowing the host resolves it; (c) otherwise unresolved. Resolution runs off the render loop, is cached for the process lifetime, and never blocks a frame.
- FR-3: The launcher shall gain a third tab, **Pull Requests**, always visible. `LauncherTab::toggle`'s two-tab assumption shall be replaced by proper ordered cycling across all tabs; existing tab-switch keys and last-used-tab memory apply unchanged.
- FR-4: With a resolved provider and working CLI, the tab shall list open PRs — number, title, author, source branch, draft marker, last-updated — fetched via the CLI's JSON output (`gh pr list`-family / `glab mr list -F json`) on a background task with the launcher's existing single-flight + generation-guard discipline. The list shall integrate the shared `/` filter and motion layer (spec 12).
- FR-5: Every degraded state shall render a specific, actionable body in the tab, with the real hostname interpolated where relevant: provider unresolved (both CLI auth commands for the host, as copy-pasteable lines), CLI not on PATH (install pointer + auth command), CLI present but unauthenticated (exact `… auth login --hostname <h>` line), list call failed (first stderr line + retry hint). A successful list with zero open PRs is not a degraded state and shall render its own empty-state line naming the repo (e.g. "No open pull requests on <org/repo>"), visually distinct from diagnostics; the FR-22 finished-reviews footer still renders when cleanup candidates exist. Never a blank list, never a guess.
- FR-6: A new `docs/forge-setup.md` (linked from the README) shall document supported providers, how zero-config detection works, the hosted-instance walkthrough (install CLI → `auth login --hostname` → done), and troubleshooting for each FR-5 state.

**Proof Artifacts:**

- Test: URL-parser unit tests (TDD) demonstrate hostname extraction across https/ssh/scp-like forms and rejection of malformed input.
- Test: provider-resolution tests with a fake credential-checker demonstrate the ladder (known host, CLI-credential hit, ambiguous → unresolved) without spawning real CLIs.
- Test: PR-list JSON parsing tests on fixture `gh`/`glab` output demonstrate typed rows from machine-readable output only.
- Test: UI tests with a fake `ForgeProvider` demonstrate each FR-5 degraded body renders its prescription, hostname included.
- CLI: journey transcript — launcher opened in a scratch repo with no forge remote and again with an unauthenticated fake host, demonstrating the diagnosis bodies; a live `gh`-authenticated dogfood listing captured by the user demonstrates the happy path.

### Unit 2: PR checkout → review session

**Purpose:** Turn a selected PR into a spec-08 review session, fork-safe, with staleness handled. Serves the core review loop.

**Functional Requirements:**

- FR-7: `Enter` on a PR shall: fetch the PR head ref (`refs/pull/<n>/head` on GitHub, `refs/merge-requests/<iid>/head` on GitLab) into managed branch `redquill/pr/<n>` via fixed-shape refspec; plain-fetch the PR's base ref so `origin/<base>` resolves; then run the existing unchanged flow — `ensure_review_worktree`, review-state load + blob-SHA reconciliation, re-root onto `DiffTarget::Review` with the PR's base. In-session and single-in-flight guards behave as on the Branches tab.
- FR-8: Forced ref updates (`+` refspec) and branch deletion shall be permitted **only** for refs under `refs/heads/redquill/pr/`; the update path shall be structurally incapable of naming any other ref. All fetches run with `GIT_TERMINAL_PROMPT=0` on background tasks.
- FR-9: Every open of a PR review re-fetches the head ref first (fetch-on-open). If the head moved while a worktree exists, redquill shall recreate the session transparently: remove the managed worktree, force-update the managed branch, re-add the worktree — then reconcile, so previously `Accepted` files whose blobs changed surface as `ChangedSinceAccepted`. A one-line status reports "PR updated — N accepted file(s) changed". The existing manual refresh action re-runs the same check mid-session. No background polling exists.
- FR-10: A failed fetch (offline, ref not advertised, auth expired) shall leave existing local state untouched and surface a one-line diagnostic; if a prior worktree exists the user may continue reviewing the stale checkout, clearly labeled stale.
- FR-11: `review-state.json` shall move to schema v3: a `PersistedReview` may carry forge metadata (provider, host, PR number, last-fetched head SHA). v2 files load silently as v3 with absent forge fields; serialization stays deterministic (BTreeMap ordering) and byte-stable for non-forge reviews.

**Proof Artifacts:**

- Test: refspec-construction tests demonstrate only `redquill/pr/<n>` refs are ever writable/deletable and argv is fixed-shape.
- Test: integration tests on scratch tempdir repos (a bare "origin" advertising a `refs/pull/1/head`-style ref) demonstrate checkout → worktree → review, head-move → recreate → `ChangedSinceAccepted`, and fetch-failure → stale-labeled session. (Per guardrails, only agent-created scratch repos; never the host repo.)
- Test: store round-trip tests demonstrate v2 → v3 silent migration and byte-stable v3 output.
- CLI: journey transcript on scratch repos — `R`, PRs tab, `Enter` lands in a review session; author-side push, reopen shows the "PR updated" line and demoted files.

### Unit 3: Comment threads overlay and reply drafting (GitHub)

**Purpose:** Make the PR's existing conversation visible and answerable in-terminal. Serves discussion continuity.

**Functional Requirements:**

- FR-12: On entering (and refreshing) a PR review, redquill shall fetch the PR's review threads asynchronously — never blocking entry — and render them read-only at their diff anchors: a compact per-line marker in the gutter region plus an expandable thread view showing author, relative time, and bodies in conversation order (replies nested under roots via the forge's reply linkage). Resolved and outdated threads render collapsed with a state label; threads whose anchor no longer maps (outdated position) attach at file level.
- FR-13: Imported threads live in a store separate from the user's annotations: never editable, never serialized to stdout, never persisted to disk (live-fetch only). When the thread fetch fails, the review continues with a one-line "comments unavailable" notice.
- FR-14: The user shall be able to draft a reply to any thread (compose flow consistent with annotation compose). Draft replies are local until submit, target the thread root, are editable/deletable like annotations, appear in the annotation list panel with a reply marker, and persist in schema v3 (thread id + body) so a resumed session keeps them.
- FR-15: The user's own previously published comments arrive like any other thread content from the fetch; annotations marked published are not re-rendered as local annotations at the same anchor (the forge copy is authoritative on screen).
- FR-16: All new actions (expand/collapse thread, next/prev thread, reply) shall be driven from the shared keymap/modal-key tables with footer hints and `?` help coverage, passing the existing drift tests.

**Proof Artifacts:**

- Test: thread-model tests on fixture GitHub JSON demonstrate root/reply ordering, resolved/outdated labeling, and file-level fallback for unmappable anchors.
- Test: UI tests with a fake provider demonstrate marker rendering, expand/collapse, reply drafting, persistence of drafts across a simulated reopen, and the fetch-failure notice.
- CLI: journey transcript — scripted fake-provider session showing a 5-and-5 back-and-forth thread rendered in order, a reply drafted and visible in the annotation panel.

### Unit 4: Submit review — batch modal, verdicts, guardrail amendment (GitHub)

**Purpose:** Publish the review — the feature's payoff and its riskiest write. Serves review completion.

**Functional Requirements:**

- FR-17: A `submit-forge-review` action (keymap-table row, review sessions on a forge PR only) shall open the submit modal: every unpublished annotation (rendered with its anchor and classification), every draft reply, a verdict selector limited to the provider's capabilities (GitHub: comment / approve / request changes), and an optional summary body. Worktree-anchored annotations (`WorktreeLine`/`WorktreeRange`) are listed as "local-only — will not publish". File-target annotations are labeled "posts as a separate file comment" (see FR-19). Nothing is sent until the user confirms from this modal.
- FR-18: On confirm (GitHub), redquill shall submit one review via the reviews REST endpoint (`gh api` POST with a `comments` array + `event` + summary body): `Line`/`Range` annotations map to `side`/`line`/`start_line`/`start_side` from their existing `Side` + line data; `Hunk` maps as a new-side range; classification is preserved as the existing `[issue]`/`[question]`/`[nit]`/`[praise]` body prefix.
- FR-19: After a successful review submission, follow-up writes post sequentially, each marked published individually on success: file-target annotations via the single-comment endpoint (`subject_type: file`), and draft replies via the reply endpoint. A failure mid-sequence stops, reports which items published, and leaves the rest unpublished — re-submitting sends only what remains.
- FR-20: Published annotations and replies are retained locally with a published marker, excluded from every future submit, and persist their published state in schema v3. The stdout markdown emitted on quit remains byte-identical in format and includes annotations regardless of published state (the stdout contract is a public API and is not modified by this spec).
- FR-21: The guardrails documentation (CLAUDE.md) shall be amended in the same change that ships this unit: the product's write ceiling gains exactly (a) forge review submission on the PR under review (comments, replies, verdict) behind the confirm modal, and (b) forced ref update + branch deletion confined to `refs/heads/redquill/pr/*`. Forbidden explicitly: PR merge/close, editing or deleting any forge comment, thread resolution changes, and any forge write outside the submit flow. The agent ceiling is restated: agents never invoke forge writes; forge testing uses fakes and scratch repos only.

**Proof Artifacts:**

- Test: payload-construction tests demonstrate the exact JSON for a mixed batch (line, range on old side, hunk, classification prefixes, verdict, summary) with no network involved.
- Test: submit-sequence tests with a fake provider demonstrate atomic-review-then-follow-ups ordering, per-item published marking, mid-sequence failure stop + resume-without-duplicates.
- Test: store tests demonstrate published flags round-trip and published items are excluded from a second submit.
- Docs: CLAUDE.md guardrail diff demonstrates the amended ceiling shipped with the code.
- CLI: user-performed live dogfood against a personal scratch GitHub repo (agent ceiling forbids agent-run forge writes): transcript + resulting PR review screenshot demonstrate the end-to-end publish; agent-side proofs stop at recorded-argv fakes.

### Unit 5: Finished-review lifecycle and cleanup

**Purpose:** Keep managed branches, worktrees, and state from accumulating after PRs merge or close. Serves long-term hygiene.

**Functional Requirements:**

- FR-22: While listing PRs, redquill shall cross-reference managed `redquill/pr/*` branches against the open-PR set (no extra network call). Managed reviews whose PR is no longer open are surfaced in the PRs tab as a "finished" section footer with a count.
- FR-23: A cleanup action (keymap-table row on the PRs tab) shall open a confirm modal enumerating each finished review — PR number/title, worktree path, and an explicit unpublished-annotation count when nonzero — and on confirm delete, per review: the managed worktree (`git worktree remove`, then `prune`), the `redquill/pr/<n>` branch, and its review-state entry. Nothing is ever deleted without this confirm; declining leaves everything intact.
- FR-24: Cleanup failures (locked worktree, dirty checkout) stop that entry with a one-line diagnostic and continue to the next, reporting a per-entry outcome summary.

**Proof Artifacts:**

- Test: finished-detection tests demonstrate correct set difference between managed branches and open PRs, including PRs never reviewed locally (no state entry → not listed).
- Test: integration test on scratch repos demonstrates confirm-gated deletion of worktree + branch + state entry, the unpublished-annotation warning, and the decline path leaving all artifacts.
- CLI: journey transcript — a finished review cleaned up via the modal; `git worktree list`, `git branch`, and `review-state.json` before/after captured.

### Unit 6: GitLab provider

**Purpose:** Prove the trait against the second forge and deliver GitLab/self-managed parity. Serves GitLab users (including hosted-instance client work).

**Functional Requirements:**

- FR-25: A GitLab `ForgeProvider` implementation shall supply: MR listing (`glab mr list -F json`), MR detail incl. `diff_refs` SHAs, head-ref checkout via `refs/merge-requests/<iid>/head` (Units 1–2 flows unchanged), and thread import from the discussions API with position-hash anchors mapped onto the same thread model (added line → `new_line`, removed → `old_line`, context → both).
- FR-26: GitLab submit shall use draft notes for near-atomicity: create one draft note per annotation/reply (position hash built from the annotation's side + line data and the MR's `diff_refs`; drafts are invisible to others, so mid-sequence failure publishes nothing), then `bulk_publish` once, then `glab mr approve` when the verdict is approve. Where the instance lacks the draft-notes API, submission falls back to sequential visible discussions with per-item published marking (the Unit 4 resume discipline). File-target annotations use a file-type position; worktree-anchored annotations remain local-only.
- FR-27: GitLab capabilities shall declare verdicts comment / approve — no request-changes in v1 — and the submit modal shall adapt automatically per FR-17's capability-driven rendering, including honest disclosure when the sequential fallback is in use.
- FR-28: `docs/forge-setup.md` and the FR-5 prescriptions shall cover glab and hosted GitLab end-to-end; the FR-2 credential lookup shall resolve a `glab`-authenticated custom host with zero redquill configuration.

**Proof Artifacts:**

- Test: position-hash construction tests demonstrate correct old/new line mapping for added, removed, and context lines against fixture `diff_refs`.
- Test: fake-provider submit tests demonstrate draft-create → bulk-publish → approve ordering, nothing-published-on-early-failure, and the sequential fallback path.
- Test: MR JSON parsing tests on fixture `glab` output demonstrate typed listing and detail rows.
- CLI: user-performed live dogfood against a scratch GitLab project (and, when available, the client's hosted instance for detection only — no writes) demonstrating list → review → submit; agent-side proofs stop at fakes.

## Non-Goals (Out of Scope)

1. **Any forge write beyond the submit flow**: no PR/MR creation, merge, close, reopen, label, assignee, or CI actions; no editing/deleting any comment (including your own already-published ones); no thread resolve/unresolve.
2. **`--pr <N>` CLI flag**: explicitly rejected — the launcher is the only entry point.
3. **Forge config surface**: no `[forge]` config section, host maps, or per-repo provider keys; detection is the CLI-credential ladder only. Revisit only if a real ambiguous-host case appears.
4. **Thread caching / offline threads**: teammates' threads are never persisted; offline reopening shows local work plus "comments unavailable".
5. **Background polling or notifications**: freshness comes from fetch-on-open and manual refresh only.
6. **GitLab request-changes verdict**: exists on GitLab 17.3+ but requires version detection; the capability flags leave room, a follow-up can light it up.
7. **GitHub GraphQL pending reviews**: draft-review parking on the forge is not offered; local state is the draft.
8. **Multi-remote support**: only `origin` is consulted, matching `default_base()`.
9. **PR statuses beyond listing** (CI checks, mergeability, review-requested filters): listing fields are fixed at FR-4's set for v1.

## Design Considerations

- The PRs tab mirrors the Branches tab's weight and footer language ("start PR review"), with the draft marker and updated-time kept visually secondary to number + title.
- Thread markers must not fight the diff: a single-cell gutter indicator per annotated line, with the thread body on demand — never inline expansion that reflows the diff by default.
- The submit modal is the safety boundary: it must fit the batch on one screen where possible, group by file, always show the verdict and target PR (`#123 on github.com/org/repo`), and make "nothing happens without confirm" visually unmistakable.
- Degraded-state bodies (FR-5) are prescriptions, not errors: imperative copy-pasteable lines first, prose second.
- All new keys ride the shared tables in `src/ui/modal_keys.rs`/`keymap.rs` with footer hints and `?` coverage; no bespoke bindings (per the UX-simplification direction).

## Repository Standards

- Rust best practices per `docs/rust-best-practices.md` in full: no `unwrap`/`expect` in production code, typed `thiserror` errors in `forge/` and `git/`, subprocess argv from closed types, machine-readable output only, background work off the render loop with single-flight + generation guards, kill+reap and prompt-disabling for spawned CLIs.
- TDD for the pure surfaces: remote-URL parser, forge JSON parsers, payload/position-hash construction, finished-set detection, store v3 round-trip.
- Integration tests only in canonicalized tempdir scratch repos (2026-07-16 tempdir-leak incident); worktree flows never run against the host repo.
- Layering: `forge/` depends on `git/` types where deliberate and documented; `ui/` consumes `forge/` through the trait seam with fakes for tests; no TUI types below `ui/`.
- All four gates before every commit (`cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`); conventional commits; refactors (e.g. `LauncherTab` cycling) land separately from behavior changes.
- Perf tripwires stay green: PR lists and thread overlays add no per-frame work proportional to PR count or thread count beyond the visible window.

## Technical Considerations

- **CLI-only transport, `api` subcommands for precision**: listing and simple actions use the CLIs' porcelain JSON (`gh pr list`, `glab mr list -F json`, `glab mr approve`); positioned operations use `gh api` / `glab api` against the documented REST endpoints, since `gh pr review` has no line-comment support and glab has no native positioned-discussion command (verified against current official docs, 2026-07). No HTTP/TLS crates, no async runtime, no new dependencies anticipated; any exception must be justified per repo policy.
- **GitHub submit shape**: one POST to the reviews endpoint with `comments[]` (+`line`/`side`/`start_line`/`start_side`), `event`, `body`. File-level comments are *not* accepted in that array (confirmed current limitation) — hence FR-19's follow-up single-comment posts. Replies likewise post-submit via the replies endpoint (top-level-comment targets only, hence FR-14's thread-root targeting).
- **GitLab submit shape**: draft notes (`draft_notes` + `bulk_publish`, ~15.10+) as primary — drafts are author-only-visible, making partial failure harmless; sequential discussions as fallback. Position hashes derive from the MR's `diff_refs` (`base_sha`/`start_sha`/`head_sha`) plus the annotation's side-aware line data, which `DiffLine` already carries.
- **Credential lookup asymmetry**: `gh auth token --hostname <h>` is a local read (exit code 4 = no credential) — use exit status only and discard stdout unread. glab's `auth status` may attempt a network validation per host; the implementation shall prefer a local-only glab lookup (candidate: `glab config get token --host <h>`; verify at task time) and in all cases run lookups on background tasks with a timeout so detection can never stall the launcher.
- **Hidden-ref caveats**: `refs/pull/*/head` and `refs/merge-requests/*/head` are fetchable from standard clones today, but GitHub is introducing settings that can hide magic refs — a fetch failure is a first-class degraded state (FR-10), not a panic path. The `/merge` preview refs are unreliable and are not used.
- **Reuse over rebuild**: the PRs tab reuses the launcher's tab/filter/motion shell, `BackgroundTasks` + generation guards, `ensure_review_worktree`, reconciliation, `DiffTarget::Review`, and the annotation compose/list surfaces. Genuinely new: `forge/` (trait, parsers, payloads), the thread overlay store/rendering, the submit modal, schema v3 fields, cleanup flow.
- **Non-interactive CLI hygiene**: spawned `gh`/`glab` processes run with prompts disabled (e.g. `GH_PROMPT_DISABLED=1`, `NO_COLOR=1`, and glab equivalents) plus the existing kill-on-drop discipline.

## Security Considerations

- redquill never reads, stores, logs, or displays tokens. The one command whose stdout would contain a token (`gh auth token`) is used strictly for its exit status with output discarded; diagnostics render stderr first-lines only, and CLI stderr shall never be logged verbatim into persisted proof artifacts without review.
- All argv is constructed from typed values; hostnames come from the user's own git config and are validated (charset allowlist) before appearing in argv; PR numbers are integers end-to-end. No `sh -c` anywhere.
- The forge write surface is exactly the FR-21 ceiling, enforced structurally (closed provider-method set — no generic "run arbitrary api call" escape hatch reachable from UI actions).
- Agent ceiling: agents implementing this spec never run forge writes, never touch the host repo's worktrees/branches, and test only against fakes and scratch repos; live-write dogfood proofs are performed by the user.
- Published-comment content is user-authored and previewed in the confirm modal before any network write; there is no path that publishes without that modal.

## Success Metrics

Per this repo's UX-outcome verification convention, metrics are user journeys with persisted evidence:

1. **Journey A (PR review in 3 keystrokes)**: from anywhere, `R` → PRs tab → `Enter` lands in a full review session of a real PR (worktree, tri-state, annotations) — transcript persisted.
2. **Journey B (conversation fidelity)**: a PR with a ≥4-message thread renders the thread in order at its anchor, a reply drafted in-terminal appears on the forge after submit, threaded under the right root — user dogfood evidence (screenshot + transcript).
3. **Journey C (safe publish)**: a mixed batch (line, range, old-side, hunk, file, classification prefixes, verdict) previews completely in the submit modal and lands on the forge exactly as previewed; a forced mid-sequence failure then a re-submit produces no duplicates — fake-provider transcript + user dogfood.
4. **Journey D (zero-config hosted GitLab)**: on a machine with `glab` authenticated to a self-managed host and no redquill config, the PRs tab lists MRs; with credentials removed, the tab prescribes the exact auth command — captured transcript.
5. **No regressions**: all existing gates and perf tripwires green; stdout annotation markdown byte-format unchanged (existing serialization tests).

## Open Questions

1. Default key choices for `submit-forge-review`, thread navigation/expand, reply, and cleanup — to be picked against the current shared tables at task-planning time so no existing binding is shadowed (actions and table placement are specified above; only the default keycaps are open).
2. Exact glab local credential-lookup command (`glab config get token --host <h>` vs alternatives) — verify against the pinned glab version at implementation; the ladder design is unaffected, only which command fills slot (b).
3. Whether the finished-reviews footer (FR-22) should also appear as a startup status line outside the launcher — deferred as a dogfood observation; no behavior specified.
4. GitLab draft-notes minimum version messaging: whether the fallback disclosure names the version threshold — copy decision at implementation.
