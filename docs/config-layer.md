# Config layer (skeleton spec)

Not implemented yet. This is a thin starting point for whoever picks up a
`Config.yaml` feature later — not a committed design.

## Motivation

`Theme` (`src/ui/theme.rs`) and `Keymap` (`src/ui/keymap.rs`) are both
already built as swappable-default structs with doc comments anticipating
"a future config layer would construct a different instance." Sidebar
position/width (`split_layout` in `src/ui/mod.rs`) is a third candidate.
Right now all three are hardcoded at their single construction/call site.

## Scope (first cut)

- Sidebar side (left/right) and width
- Nothing else, initially — Theme/Keymap deserialization is a bigger lift
  (colors, key sequences) and should be a separate follow-up once the
  loading/discovery plumbing below exists and is proven out.

## Non-goals

- No hot-reload — read once at startup.
- No per-project config, only a single user-level file.
- Not replacing CLI flags (`--staged`, `-o`) — config is for persistent
  preferences, flags stay for one-off invocation behavior.

## New dependency

No YAML-parsing crate exists in `Cargo.toml` today (`serde`/`serde_json`
are present; tree-sitter-yaml is for syntax highlighting only, not
deserialization). Adding one (e.g. `serde_yaml` or `serde_yml`) is the one
net-new dependency this needs — call that out explicitly in the PR per
CLAUDE.md's "don't add dependencies casually."

## Shape (sketch, not final)

```rust
// src/config.rs (new)
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Config {
    pub sidebar: SidebarConfig,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct SidebarConfig {
    pub side: SidebarSide,   // Left | Right, default Right
    pub width: u16,          // default 32
}
```

Loaded once in `main.rs` (near where the CLI `Config` struct is already
built at `main.rs:44-72` — name collision to resolve, e.g. rename one),
from `$XDG_CONFIG_HOME/redquill/config.yaml` (fall back to
`~/.config/redquill/config.yaml`). Missing file is not an error — fall
back to `Config::default()` silently, consistent with LSP's "missing
server degrades silently" precedent.

`App` would hold the resolved `SidebarConfig` (or the values it needs)
the same way it holds `Theme` today, and `split_layout` would take a
width/side param instead of the hardcoded `Constraint::Length(32)` and
fixed left/right order.

## Open questions for whoever implements this

- Single `Config.yaml` for sidebar+theme+keymap eventually, or separate
  files? (Keymap remap syntax in particular needs its own design pass —
  key sequences aren't trivially `Deserialize`.)
- Validate `width` bounds (e.g. reject 0 or > terminal width) at load
  time or clamp silently at render time?
