# Rust Application Best Practices

Portable, project-agnostic discipline for Rust applications. Written as instructions for agents and humans working in the repo; a project's CLAUDE.md should import or reference this file and add only project-specific rules on top.

## Error handling

- No `unwrap()`, `expect()`, or panic macros (`panic!`, `unreachable!`, `todo!`) in production code. Tests may use them freely.
- `unreachable!` is not exempt even when provably unreachable today — the proof rots when someone edits the code above it. Prefer a defensive fallback arm or a typed error; it costs one line and removes a panic path.
- Typed errors (`thiserror`) in library/domain modules; `anyhow` only at the binary edge (`main`, CLI wiring). Never let `anyhow` leak downward into library APIs.
- Decide deliberately, per subsystem, what degrades silently vs. what surfaces to the user — and document the contract in the module doc. Silent degradation is a valid design (e.g. optional enhancements), but only when written down; otherwise it's error swallowing.
- Background/async failures are values, not panics: catch panics at thread/task boundaries (`catch_unwind`) and convert them into a result variant the owner polls, so one bad task can't poison the loop.

## Module boundaries and layering

- Draw the layer map explicitly (in CLAUDE.md or an architecture doc): which modules are pure domain, which do I/O, which are presentation. State the allowed dependency direction.
- Presentation types (TUI/GUI/serialization-framework types) must never appear in domain-layer signatures or imports. Verify with grep, not intention.
- Domain logic that happens to be written inside the presentation layer (e.g. "map cursor position to a domain target") should be factored into pure functions taking explicit data arguments — unit-testable without constructing the app. If its inputs are genuinely presentation types, it lives presentation-side in its own module; don't force it downward and leak types.
- Cross-layer type coupling is sometimes correct (a parser layer consuming a raw-output type from the layer below). When it's deliberate, record it so nobody "fixes" it casually.
- Wrap external processes/services behind a trait seam (e.g. an operations trait implemented by the real runner), so the presentation layer is testable with fakes and the boundary is explicit.

## State design (avoiding the god object)

- One aggregate-root struct is fine; a root with dozens of fields that sibling modules reach into is not. Watch two metrics: field count and `pub(super)` reach-ins. When either grows, split cohesive method clusters into sibling files as split `impl` blocks (`impl App` in `refresh.rs`, `render_glue.rs`, …) — Rust makes this cheap and it needs no redesign.
- State that only exists in one mode belongs in that mode's enum variant payload (`Mode::Panel { cursor }`), not in a parallel struct field that goes stale while inactive. The scattered-`.min(len-1)`-clamp pattern is the smell that says state outlived its scope.
- Exception: state that must *survive* mode exit (e.g. "reopen the panel where you left off") is legitimately a struct field. Behavior preservation outranks pattern completion — but document why.
- Predicates asked in more than one place ("is an overlay open?") get one named helper, so the question can't be answered inconsistently.
- Derived state is recomputed at a single rebuild point, never incrementally patched in multiple places.

## Data-driven invariants

- When behavior and documentation must agree (keybindings ↔ help screen, CLI flags ↔ usage text, config keys ↔ docs), drive both from one `const` table and add a test that fails when they drift. Hand-maintained parallel lists always diverge.
- Prefer dispatch through the table into a small per-mode action enum matched exhaustively — then the compiler enforces that a new table row is handled.
- For free-text input handlers where table dispatch would contort the code, keep the hand-written match but cover it with a bidirectional drift test: every documented key must observably act; every undocumented key must observably not.

## Testing

- TDD where the code is pure (parsers, data transforms, serializers): failing test first, tests committed with the code.
- Integration tests build throwaway state in tempdirs (`tempfile`); never touch the host repo/filesystem/config. Canonicalize tempdir paths (macOS `/var` symlink).
- When a file's test module dwarfs its production code, split it out via `#[cfg(test)] #[path = "foo_tests.rs"] mod tests;` — keeps private access, halves the file, zero logic change.
- Move-only refactors have an invariant: identical test counts and zero assertion edits before/after. Verify it, state it in the commit.
- Performance bars are tested contracts, not aspirations. For each hot path, add a wall-clock tripwire: measure debug-build timings first, budget 10–20x over measured, loop-amortize to defeat timer noise. The test exists to catch complexity-class regressions (accidental O(n²), cache-busting), not machine variance — flakiness is the enemy, so fat margins.

## Concurrency and background work

- Never block the latency-critical loop (render/event loop): no synchronous subprocess calls, file I/O, or lock waits on it. Anything slow runs on a background thread and re-enters via non-blocking polling (`try_recv` drain once per tick).
- Guard re-entry: single-flight flags so an operation can't run twice concurrently; a generation counter so a stale background result completed after a newer foreground action gets dropped, not applied.
- Drop results that would clobber in-progress user input (mid-edit, mid-selection) rather than applying them.
- Bounded buffers by default; an unbounded channel is acceptable only with a documented drain-cadence assumption.
- Child processes get kill+reap on drop as a safety net; disable interactive prompts in anything spawned headless (e.g. `GIT_TERMINAL_PROMPT=0`).

## Subprocess and external-command hygiene

- Build argv from closed types (enums/structs → fixed argument lists), never by interpolating strings into a command line. No `sh -c`.
- Parse machine-readable output formats (porcelain, NUL-separated, JSON) — never scrape human-readable output.
- Respect the user's environment and config: shell out to the tool on PATH rather than embedding a divergent library, when fidelity to user config matters.

## Dependencies

- Every dependency is justified in the PR/commit that adds it; the default answer is no. Prefer std (e.g. `std::time::Instant` over a benchmark framework for tripwires).
- `default-features = false` unless features are needed. Dev-only crates stay in `[dev-dependencies]`.

## Docs as contract

- Guardrails, README claims, and scope statements must track shipped code. Treat drift as a bug of the same severity as a failing test: a stale "never do X" misleads every future reader (and agent).
- Distinguish clearly between what the *product* may do at runtime and what an *agent working in the repo* may do during a task — they are different write ceilings.
- Anything emitted on stdout for other programs to parse is a public API: document the format, test it byte-exactly, reserve stdout for it (UI/logs to stderr).
- Don't promise unbuilt features in the present tense ("fully remappable"); say "designed for X (planned)".

## Commits and gates

- Gates before every commit, no exceptions: `cargo build`, `cargo test`, `cargo clippy -- -D warnings` (use `--all-targets` so test code is linted too), `cargo fmt --check`.
- Conventional commits (`feat:`, `fix:`, `refactor:`, `test:`, `docs:`, `perf:`). One self-contained change per commit; every commit leaves the tree green.
- Refactors and behavior changes never share a commit.
