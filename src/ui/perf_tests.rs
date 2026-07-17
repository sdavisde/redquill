//! Performance tripwire for the "instant feel on a 5k-line diff" bar. These
//! are *not* frame-rate tests — they bound the wall-clock cost of
//! the terminal-free hot paths whose cost determines frame time:
//!
//! 1. Full row/multibuffer construction (`rebuild_rows` / `build_multibuffer`).
//! 2. `apply_snapshot` (the refresh fold-in).
//! 3. Highlight-cache population for the whole diff (tree-sitter over every
//!    expanded file).
//! 4. Cursor / hunk navigation over the built buffer (per-keypress cost).
//!
//! The budgets are deliberately loose — chosen with ~12-16x headroom over
//! measured debug-build times on a developer machine — so the assertions only fire on a
//! *complexity-class* regression (an accidental O(n²), or re-highlighting the
//! whole diff on every keypress), never on ordinary CI/machine variance. Where
//! a measurement is inherently noisy (a single fast operation), the operation is
//! run in a loop and the *total* is budgeted, which averages out timer jitter.
//!
//! Set `REDQUILL_PERF_PRINT=1` to print the measured timings to stderr.

use std::time::{Duration, Instant};

use super::*;
use crate::git::GitError;

// -- Synthetic diff generation --------------------------------------------

/// A ~5,000-line synthetic diff spread over a realistic file count, with two
/// large single files and the rest mid-sized, matching the audit's shape
/// ("20-50 files, mixed adds/removals/context, some large single files").
const FILE_COUNT: usize = 25;
/// Diff (and whole-file) line count for the two "large" files.
const LARGE_FILE_LINES: usize = 520;
/// Diff (and whole-file) line count for the remaining mid-sized files.
const SMALL_FILE_LINES: usize = 180;
/// Diff lines per hunk before splitting into the next hunk, so each file has
/// several hunks (exercising multi-hunk navigation and header scanning).
const HUNK_LINES: usize = 40;

/// The nominal size of file `i` in the synthetic set.
fn file_size(i: usize) -> usize {
    if i < 2 {
        LARGE_FILE_LINES
    } else {
        SMALL_FILE_LINES
    }
}

/// Generates `n` lines of source-code-like Rust for file `seed`, so tree-sitter
/// highlighting does real lexing/parsing work (keywords, identifiers, operators,
/// literals, comments, string literals) rather than degenerating to plain text.
fn rust_source_lines(seed: usize, n: usize) -> Vec<String> {
    let mut lines = Vec::with_capacity(n);
    lines.push("use std::collections::HashMap;".to_string());
    lines.push(format!("// generated module {seed}"));
    lines.push(String::new());
    let mut fi = 0usize;
    while lines.len() < n {
        // An 11-line function body with a mix of tokens tree-sitter must parse.
        lines.push(format!(
            "pub fn compute_{seed}_{fi}(alpha: i64, beta: i64) -> i64 {{"
        ));
        lines.push("    let mut total: i64 = 0;".to_string());
        lines.push(format!("    let label = \"item-{seed}-{fi}\";"));
        lines.push("    let sum = alpha + beta * 3 - 1;".to_string());
        lines.push("    for idx in 0..sum.max(0) {".to_string());
        lines.push("        total += idx * 2 + alpha % 7;".to_string());
        lines.push("    }".to_string());
        lines.push("    if total > beta && !label.is_empty() {".to_string());
        lines.push("        total -= beta;".to_string());
        lines.push("    }".to_string());
        lines.push("    total".to_string());
        lines.push("}".to_string());
        fi += 1;
    }
    lines.truncate(n);
    lines
}

/// Builds one file's `RawFilePatch` from its source lines: chunks them into
/// several hunks, and within each hunk assigns a repeating context/removed/
/// added/added pattern so the diff is a realistic mix of additions, removals,
/// and context (with adjacent removed/added pairs to exercise word-diff).
fn build_patch(path: &str, src: &[String]) -> RawFilePatch {
    let mut raw = format!(
        "diff --git a/{path} b/{path}\nindex 1111111..2222222 100644\n--- a/{path}\n+++ b/{path}\n"
    );
    let mut old_line = 1u32;
    let mut new_line = 1u32;
    for (chunk_idx, chunk) in src.chunks(HUNK_LINES).enumerate() {
        let old_start = old_line;
        let new_start = new_line;
        let mut body = String::new();
        let mut old_count = 0u32;
        let mut new_count = 0u32;
        for (j, text) in chunk.iter().enumerate() {
            let global = chunk_idx * HUNK_LINES + j;
            match global % 4 {
                1 => {
                    body.push('-');
                    old_count += 1;
                }
                2 | 3 => {
                    body.push('+');
                    new_count += 1;
                }
                _ => {
                    body.push(' ');
                    old_count += 1;
                    new_count += 1;
                }
            }
            body.push_str(text);
            body.push('\n');
        }
        raw.push_str(&format!(
            "@@ -{old_start},{old_count} +{new_start},{new_count} @@\n"
        ));
        raw.push_str(&body);
        old_line += old_count;
        new_line += new_count;
    }
    RawFilePatch {
        path: path.to_string(),
        old_path: None,
        raw,
        is_binary: false,
    }
}

/// A backend fake that serves each file's whole-file content (for both the
/// worktree/new side and the `show_file`/old side), so highlight-cache
/// population runs tree-sitter over real source. Every other op is a cheap
/// no-op — the tripwire never drives a real refresh through it.
struct ContentFake {
    /// Whole-file source keyed by repo-relative path.
    content: HashMap<String, String>,
}

impl ContentFake {
    /// Extracts the path from a `show_file` spec like `:0:src/foo.rs` or
    /// `HEAD:src/foo.rs` (everything after the last `:`).
    fn path_of_spec(spec: &str) -> &str {
        spec.rsplit(':').next().unwrap_or(spec)
    }
}

impl StageOps for ContentFake {
    fn diff(&self, _target: &DiffTarget) -> Result<Vec<RawFilePatch>, GitError> {
        // Empty for every target, including `DiffTarget::Staged`: the
        // synthetic scenario has no staged files, so `build_review`'s
        // extra staged-diff fetch (for fully-staged sections) measures its
        // real, realistically-empty cost rather than a fabricated one.
        Ok(Vec::new())
    }
    fn status(&self) -> Result<Vec<crate::git::FileStatus>, GitError> {
        Ok(Vec::new())
    }
    fn stage_file(&self, _path: &str) -> Result<(), GitError> {
        Ok(())
    }
    fn unstage_file(&self, _path: &str) -> Result<(), GitError> {
        Ok(())
    }
    fn apply_cached(&self, _patch: &str) -> Result<(), GitError> {
        Ok(())
    }
    fn unapply_cached(&self, _patch: &str) -> Result<(), GitError> {
        Ok(())
    }
    fn read_worktree_file(&self, path: &str) -> Option<Vec<u8>> {
        self.content.get(path).map(|s| s.clone().into_bytes())
    }
    fn show_file(&self, spec: &str) -> Option<String> {
        self.content.get(Self::path_of_spec(spec)).cloned()
    }
}

/// A synthetic ~5k-line review: its `ReviewSnapshot`, the whole-file content
/// map for the backend, and the total diff-line count (for reporting).
struct SyntheticReview {
    snapshot: ReviewSnapshot,
    content: HashMap<String, String>,
    diff_lines: usize,
}

/// Builds the synthetic review. `variant` perturbs the generated source so two
/// different variants produce byte-different `FileDiff`s (used to force
/// `apply_snapshot` to invalidate and re-highlight every file).
fn synthetic_review(variant: usize) -> SyntheticReview {
    let mut files = Vec::with_capacity(FILE_COUNT);
    let mut patches = Vec::with_capacity(FILE_COUNT);
    let mut content = HashMap::new();
    let mut diff_lines = 0usize;
    for i in 0..FILE_COUNT {
        let path = format!("src/module_{i:02}.rs");
        let src = rust_source_lines(i * 100 + variant, file_size(i));
        diff_lines += src.len();
        let whole = src.join("\n") + "\n";
        content.insert(path.clone(), whole);
        let patch = build_patch(&path, &src);
        let file = FileDiff::from_patch(&patch).expect("synthetic patch parses");
        files.push(file);
        patches.push(Some(patch));
    }
    let snapshot = ReviewSnapshot {
        files,
        patches,
        staged: Vec::new(),
        staged_states: HashMap::new(),
    };
    SyntheticReview {
        snapshot,
        content,
        diff_lines,
    }
}

/// Builds an `App` over the synthetic review with the content-serving backend
/// attached, so every expanded file gets tree-sitter highlighting. Nothing is
/// staged, so no section starts collapsed — the whole diff is live.
fn build_app() -> (App, usize) {
    let review = synthetic_review(0);
    let diff_lines = review.diff_lines;
    let fake = ContentFake {
        content: review.content,
    };
    let app = App::with_git(review.snapshot, DiffTarget::WorkingTree, Box::new(fake));
    (app, diff_lines)
}

// -- Timing helpers --------------------------------------------------------

fn perf_print_enabled() -> bool {
    std::env::var_os("REDQUILL_PERF_PRINT").is_some()
}

fn report(label: &str, elapsed: Duration, budget: Duration) {
    if perf_print_enabled() {
        eprintln!(
            "[perf] {label:<28} {:>8.2?}  (budget {:?})",
            elapsed, budget
        );
    }
}

// -- Tripwires -------------------------------------------------------------

// Budgets are total wall-clock ceilings with ~15x headroom over measured
// debug-build times; see the module docs. They guard complexity-class
// regressions, not machine variance.

/// Hot path 1: full row/multibuffer construction with a warm highlight cache
/// (pure `build_multibuffer` cost — the rebuild every collapse/annotation/stage
/// gesture funnels through). Looped to amortize timer noise.
#[test]
fn rebuild_rows_warm_is_bounded() {
    const ITERS: u32 = 20;
    // Measured ~0.42s debug on a dev machine; ~14x headroom.
    const BUDGET: Duration = Duration::from_millis(6_000);
    let (mut app, diff_lines) = build_app();
    // Cache is already warm from `with_git`'s initial rebuild.
    let start = Instant::now();
    for _ in 0..ITERS {
        app.rebuild_rows();
    }
    let elapsed = start.elapsed();
    report(
        &format!("rebuild_rows x{ITERS} ({diff_lines} ln)"),
        elapsed,
        BUDGET,
    );
    assert!(
        elapsed < BUDGET,
        "warm rebuild_rows x{ITERS} took {elapsed:?}, over budget {BUDGET:?} \
         (possible O(n^2) in multibuffer assembly)"
    );
}

/// Hot path 3: highlight-cache population — tree-sitter over every expanded
/// file's whole content, both sides. Measured cold (cache cleared) each pass.
#[test]
fn highlight_population_is_bounded() {
    const ITERS: u32 = 5;
    // Measured ~1.4s debug on a dev machine (tree-sitter is the noisiest hot
    // path, so the margin is widest); ~13x headroom.
    const BUDGET: Duration = Duration::from_millis(18_000);
    let (mut app, diff_lines) = build_app();
    let start = Instant::now();
    for _ in 0..ITERS {
        app.highlight_cache.clear();
        app.rebuild_rows();
    }
    let elapsed = start.elapsed();
    report(
        &format!("highlight cold x{ITERS} ({diff_lines} ln)"),
        elapsed,
        BUDGET,
    );
    assert!(
        elapsed < BUDGET,
        "cold highlight+rebuild x{ITERS} took {elapsed:?}, over budget {BUDGET:?} \
         (possible re-highlight blowup)"
    );
}

/// Hot path 2: `apply_snapshot` — the refresh fold-in. Alternating variants
/// force every file's content to differ across applies, so each pass
/// invalidates and re-highlights the whole diff (the worst case). Looped to
/// amortize noise; snapshots are pre-built outside the timer.
#[test]
fn apply_snapshot_is_bounded() {
    const ITERS: usize = 6;
    // Measured ~1.65s debug on a dev machine (each pass re-highlights the whole
    // diff); ~12x headroom.
    const BUDGET: Duration = Duration::from_millis(20_000);
    let (mut app, diff_lines) = build_app();
    // Pre-build alternating snapshots so their construction/parse cost is not
    // counted against the apply budget.
    let snapshots: Vec<ReviewSnapshot> = (0..ITERS)
        .map(|k| synthetic_review(1 + k).snapshot)
        .collect();
    let start = Instant::now();
    for snapshot in snapshots {
        app.apply_snapshot(snapshot);
    }
    let elapsed = start.elapsed();
    report(
        &format!("apply_snapshot x{ITERS} ({diff_lines} ln)"),
        elapsed,
        BUDGET,
    );
    assert!(
        elapsed < BUDGET,
        "apply_snapshot x{ITERS} took {elapsed:?}, over budget {BUDGET:?} \
         (possible full-diff re-work regression)"
    );
}

/// Hot path 4: per-keypress cursor navigation across the whole buffer. These
/// fire on every `j`/`k`, so they must stay near-instant even over a 5k-line
/// buffer; the total for a full top-to-bottom sweep is budgeted.
#[test]
fn cursor_navigation_is_bounded() {
    // Measured <1ms debug for a full down+up sweep; the budget guards against a
    // regression that makes a keypress do O(rows) work (e.g. rebuilding rows per
    // move), which would balloon a full sweep into many seconds.
    const BUDGET: Duration = Duration::from_millis(1_500);
    let (mut app, _) = build_app();
    let rows = app.view.rows.len();
    app.view.cursor = 0;
    let start = Instant::now();
    for _ in 0..rows {
        app.apply(Action::CursorDown);
    }
    for _ in 0..rows {
        app.apply(Action::CursorUp);
    }
    let elapsed = start.elapsed();
    report(&format!("cursor sweep ({rows} rows)"), elapsed, BUDGET);
    assert!(
        elapsed < BUDGET,
        "cursor sweep over {rows} rows took {elapsed:?}, over budget {BUDGET:?}"
    );
}

/// Hot path 4 (cont.): hunk-to-hunk navigation. `next_hunk` scans the buffer's
/// header rows on every press, so repeated full sweeps over a many-hunk buffer
/// are the stress case; the total is budgeted.
#[test]
fn hunk_navigation_is_bounded() {
    const SWEEPS: u32 = 40;
    // Comfortably above the synthetic set's total hunk count (~141), so each
    // sweep walks every hunk to the end and then no-ops (still scanning).
    const PRESSES: u32 = 220;
    // Measured ~0.5s debug for 40 sweeps; ~16x headroom.
    const BUDGET: Duration = Duration::from_millis(8_000);
    let (mut app, _) = build_app();
    let rows = app.view.rows.len();
    let start = Instant::now();
    for _ in 0..SWEEPS {
        app.view.cursor = 0;
        for _ in 0..PRESSES {
            app.apply(Action::NextHunk);
        }
    }
    let elapsed = start.elapsed();
    report(
        &format!("hunk nav x{SWEEPS} ({rows} rows)"),
        elapsed,
        BUDGET,
    );
    assert!(
        elapsed < BUDGET,
        "hunk navigation x{SWEEPS} sweeps took {elapsed:?}, over budget {BUDGET:?} \
         (possible per-keypress scan blowup)"
    );
}
