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
| GitHub (github.com, GitHub Enterprise) | `gh` | **Works today** — listing PRs, opening a review session |
| GitLab (gitlab.com, self-managed) | `glab` | **In progress** — hostname detection lands ahead of listing; the tab currently reports "GitLab isn't supported yet" once a GitLab host is detected |

If your `origin` points at a GitHub host, the Pull Requests tab is usable
right now. If it points at GitLab, redquill correctly recognizes the host
as GitLab but doesn't list MRs yet — that's a later unit of this feature,
not a bug.

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
     itself is never read). Exactly one CLI reporting credentials
     resolves the provider; zero or both leave it unresolved.
   - The `glab` half of that credential check is still a placeholder
     (finalized alongside the GitLab provider itself, later in this
     feature) — until then, a self-hosted host that only `glab` knows
     about will show up as unresolved even after `glab auth login`.
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

### GitLab detected, not supported yet

> `GitLab isn't supported yet (<host>)`

Shown once the host resolves to GitLab (`gitlab.com`, or a self-hosted
instance once the `glab` credential check is finalized) but GitLab
listing hasn't landed. GitHub works today; GitLab support is in progress.

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

## Security notes

redquill never reads, stores, logs, or displays a forge token. The one
command whose output could contain one (`gh auth token`) is run with its
stdout and stderr discarded entirely — only its exit status is used.
