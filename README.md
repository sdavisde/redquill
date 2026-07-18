<p align="center">
  <img src="redquill-logo.png" alt="redquill logo" width="300" height="300">
</p>

<p align="center">
  A portable, efficeint tool for reviewing code
</p>

## Vision

redquill is the human checkpoint between agent output and commit. Every hunk in the working tree gets one of two verdicts:

- **Keep it** → stage it (file, hunk, or line granularity)
- **Fix it** → annotate it, and batch the annotations back to the agent as its next prompt

What makes this tool unique is **code intelligence during review**: a limited language-server client so that go-to-definition, find-references, and hover docs are one keystroke away from any symbol in the diff — because the most common problem when reviewing code: not understanding how the changes impact other areas in the codebase.

## Getting Started

1. Install the redquill application

```bash
brew install sdavisde/tap/redquill
```

2. Run `redquill` in the git repo you want to review
3. Press \` to open the git panel, and `?` for help — it opens on a "This context" view scoped to wherever you pressed it from (plus a curated list of common workflows), with the full reference one `Tab` away. Pause after `g` or `z` to see a popup of what keys can follow.
4. When viewing the diff, press `c` to leave a comment. When the session ends, your comments are copied to the clipboard so you can paste them straight into an agent.
