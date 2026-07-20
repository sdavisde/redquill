# Forge setup (Pull Requests tab)

The Review launcher's **Pull Requests** tab (`R`, then switch tabs) lists
your repo's open PRs/MRs and turns one into a full worktree-backed review
session. redquill never talks to GitHub/GitLab's API directly — it shells
out to the official CLIs, `gh` and `glab`, exactly like it shells out to
`git` for everything else. Auth, SSO, and self-hosted instances all come
from whatever the CLI already has configured; redquill has no `[forge]`
config section of its own.

## Supported providers

| Provider | Transport | Status |
| --- | --- | --- |
| GitHub (github.com, GitHub Enterprise) | `gh` | **Works today** — listing PRs, opening a review session, submitting a review |
| GitLab (gitlab.com, self-managed) | `glab` | **Works today** — listing MRs, opening a review session, submitting a review (comment / approve) |

Both forges are end-to-end: whichever one your `origin` points at, the
Pull Requests tab lists it, opens a review session, shows existing comment
threads, and publishes your review from the confirm modal. The only visible
difference is the verdict set — GitLab has no "request changes" verdict, so
the picker offers comment / approve only (see [Submitting a
review](#submitting-a-review)).

## How zero-config detection works

No repo configuration, no `[forge]` section, nothing to type in beyond
logging into a CLI. Every time the tab loads, redquill:

1. Reads `origin`'s remote URL (from your normal git config) and extracts
   its hostname — `https://`, `ssh://`, and the scp-like `git@host:path`
   form are all understood.
2. Resolves that hostname to a provider:
   - `github.com` → GitHub, `gitlab.com` → GitLab, always, with no CLI
     call needed.
   - Any other hostname (a self-hosted/enterprise instance) is resolved by
     asking each CLI whether it holds credentials for that host — `gh`
     via `gh auth token --hostname <host>` (exit status only; the token
     itself is never read), and `glab` via a local
     `glab config get token --host <host>` lookup (a config-store read, not
     a network probe; its output is inspected only for emptiness and
     dropped immediately). A
     `glab`-authenticated self-managed host therefore resolves to GitLab
     with zero redquill configuration. Exactly one CLI reporting
     credentials resolves the provider; zero or both leave it unresolved.
3. Caches the result for the process's lifetime — one redquill session
   only ever reviews one repo, so the ladder never re-runs.

All of the above runs off the render loop, so opening the launcher never
blocks on it.

## Hosted-instance walkthrough

For a client's self-managed GitHub Enterprise or GitLab instance, setup is
one prescribed step:

1. Install the CLI if you don't have it:
   - GitHub: `gh` — <https://cli.github.com>
   - GitLab: `glab` — <https://gitlab.com/gitlab-org/cli>
2. Authenticate against the exact host:
   - `gh auth login --hostname <your-host>`
   - `glab auth login --hostname <your-host>`
3. Done. Open the Pull Requests tab — no redquill-side configuration, no
   host list to maintain.

## Troubleshooting: what the tab shows and what to do

The Pull Requests tab never renders a blank screen. Every state below is
copy-pasteable — command lines first, explanation second.

### No forge remote

> no forge remote — add a GitHub/GitLab `origin` remote to use this tab

Shown when the repo has no `origin` remote at all, or its URL doesn't
parse to a hostname. Add an `origin` remote pointing at your GitHub or
GitLab repo.

### Provider unresolved

```
gh auth login --hostname <host>
glab auth login --hostname <host>
```

followed by one of:

- `neither CLI holds credentials for <host> — run one of the above`
- `both CLIs hold credentials for <host> — redquill can't tell which forge this is`

Shown when `origin`'s hostname isn't `github.com`/`gitlab.com` and neither
(or both) CLIs report holding credentials for it. Run the login command
for whichever forge actually hosts the repo.

### CLI not on PATH

```
install gh: https://cli.github.com
gh auth login --hostname <host>
```

or, on a GitLab host:

```
install glab: https://gitlab.com/gitlab-org/cli
glab auth login --hostname <host>
```

followed by:

> `<cli> isn't on PATH`

Install the CLI, then log in.

### CLI installed but not authenticated

```
gh auth login --hostname <host>
```

followed by:

> `<cli> is installed but not logged in for <host>`

The CLI is on `PATH` but has no credentials for this host. Run the login
command shown.

### List call failed

The tab shows the first line of whatever the CLI printed to stderr,
followed by:

> `switch tabs and back to retry`

This covers everything that isn't a straightforward auth problem — rate
limiting, a network blip, an unexpected API response. Switching tabs away
and back re-runs the fetch.

### No open pull requests

> `No open pull requests on <org/repo>`

This is **success**, not a diagnostic — rendered in the same color the
diff view uses for additions, distinct from every state above. It means
detection, auth, and the listing call all worked; there's simply nothing
open right now.

## Submitting a review

Everything you draft — line/range comments, whole-file comments, thread
replies, a verdict, an optional summary — is local until you confirm it
from the submit modal. Nothing is ever sent on quit, and nothing is sent
until that confirm.

- **GitHub** posts one review (the reviews endpoint carries every
  positioned comment plus the verdict and summary at once), then any
  file-level comments and thread replies follow one at a time. Verdicts:
  comment / approve / request changes.
- **GitLab** stages every comment and reply as a *private draft note*,
  then publishes the whole batch together with one `bulk_publish` — so a
  failure partway through leaves nothing visible on the MR — and approves
  only when the verdict is approve. On an older GitLab instance without the
  draft-notes API, redquill falls back to posting each comment as a visible
  discussion one at a time (marking each published as it lands, so a retry
  re-sends only what didn't). The submit modal discloses which path applies.
  Verdicts: comment / approve (GitLab has no "request changes" verdict).

A re-submit only ever sends what hasn't already published, so a partial
failure never double-posts.

## Security notes

redquill never reads, stores, logs, or displays a forge token. The two
commands whose output could contain one (`gh auth token`, `glab config get
token`) are used only for presence — `gh auth token`'s output is discarded
entirely and only its exit status read; `glab config get token`'s output is
inspected only for emptiness and dropped immediately, never stored or
logged.
