//! Performance tripwire for Project Search's "instant feel" bar (spec 06:
//! first results <100ms on a ~5,000-file repository). Style matches
//! `src/ui/perf_tests.rs`: budgets are deliberately loose (~10-20x measured
//! debug-build time) so the assertions catch a complexity-class regression
//! (e.g. an accidental full second read-back, or losing the streaming
//! property so nothing renders until the whole scan finishes), never
//! ordinary machine variance. The corpus is generated once (unmeasured
//! setup); the scan itself is looped to amortize timer noise.
//!
//! Set `REDQUILL_PERF_PRINT=1` to print the measured timings to stderr.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use super::*;
use crate::search::query::CaseMode;

/// File count matching the spec's benchmarked scale ("~5,000-file
/// repository"; headroom target ~30ms full-scan).
const FILE_COUNT: usize = 5_000;
/// Source-like lines per file, so the corpus's total line count is
/// realistic (tens of thousands) rather than one line per file.
const LINES_PER_FILE: usize = 8;

/// Builds a `FILE_COUNT`-file, `LINES_PER_FILE`-line-each tempdir git repo.
/// Every file's line 3 contains the needle (`"needle_token"`); the rest are
/// non-matching filler, so the scan does real per-line regex work rather
/// than short-circuiting on an empty search space.
fn build_corpus() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().expect("canonicalize tempdir");
    let status = Command::new("git")
        .arg("init")
        .arg("-q")
        .arg(&root)
        .status()
        .expect("git init runs");
    assert!(status.success());
    for i in 0..FILE_COUNT {
        let mut content = String::new();
        for line in 0..LINES_PER_FILE {
            if line == 3 {
                content.push_str(&format!("let needle_token = {i}; // match\n"));
            } else {
                content.push_str(&format!(
                    "pub fn generated_{i}_{line}(x: i64) -> i64 {{ x + {line} }}\n"
                ));
            }
        }
        fs::write(root.join(format!("mod_{i:05}.rs")), content).expect("write corpus file");
    }
    (dir, root)
}

fn perf_print_enabled() -> bool {
    std::env::var_os("REDQUILL_PERF_PRINT").is_some()
}

fn report(label: &str, elapsed: Duration, budget: Duration) {
    if perf_print_enabled() {
        eprintln!(
            "[perf] {label:<32} {:>8.2?}  (budget {:?})",
            elapsed, budget
        );
    }
}

fn query(pattern: &str) -> SearchQuery {
    SearchQuery {
        pattern: pattern.to_string(),
        case: CaseMode::Smart,
        whole_word: false,
        literal: false,
    }
}

/// Query→first-batch and full-scan latency over the generated corpus,
/// looped to amortize timer noise. This is the complexity-class guard for
/// the spec's "<100ms first results" bar: it fails on an accidental
/// linear-in-cap-size regression or a lost-streaming regression, not on
/// ordinary machine variance.
#[test]
fn query_to_first_and_full_results_are_bounded() {
    const ITERS: u32 = 5;
    // Measured on a dev machine (debug build), 5,000-file corpus, x5 loop
    // totals: first-batch ~35ms (~7ms/query, comfortably under the spec's
    // <100ms bar), full-scan ~306ms (~61ms/scan). Budgets carry ~14x and
    // ~13x headroom respectively — see
    // `docs/specs/06-spec-project-search/proofs/task-2-engine.md` for the
    // full recorded run.
    const FIRST_BATCH_BUDGET: Duration = Duration::from_millis(500);
    const FULL_SCAN_BUDGET: Duration = Duration::from_millis(4_000);

    let (_dir, root) = build_corpus();

    let mut first_batch_total = Duration::ZERO;
    let mut full_scan_total = Duration::ZERO;

    for iter in 0..ITERS {
        let start = Instant::now();
        let (rx, _abort) = spawn_scan(
            root.clone(),
            query("needle_token"),
            iter as u64,
            ScanOptions::default(),
        )
        .expect("valid query spawns");
        let first = rx.recv().expect("at least one message arrives");
        first_batch_total += start.elapsed();
        let mut total_hits = 0usize;
        if let ScanMessage::Batch(batch) = &first {
            total_hits += batch.len();
        }
        loop {
            match rx.recv().expect("channel stays open until Done") {
                ScanMessage::Batch(batch) => total_hits += batch.len(),
                ScanMessage::Done(summary) => {
                    assert_eq!(
                        total_hits, FILE_COUNT,
                        "every file's needle line must be found"
                    );
                    assert!(!summary.capped);
                    break;
                }
            }
        }
        full_scan_total += start.elapsed();
    }

    report(
        &format!("search first-batch x{ITERS} ({FILE_COUNT} files)"),
        first_batch_total,
        FIRST_BATCH_BUDGET,
    );
    report(
        &format!("search full-scan x{ITERS} ({FILE_COUNT} files)"),
        full_scan_total,
        FULL_SCAN_BUDGET,
    );
    assert!(
        first_batch_total < FIRST_BATCH_BUDGET,
        "first-batch latency x{ITERS} took {first_batch_total:?}, over budget {FIRST_BATCH_BUDGET:?}"
    );
    assert!(
        full_scan_total < FULL_SCAN_BUDGET,
        "full-scan latency x{ITERS} took {full_scan_total:?}, over budget {FULL_SCAN_BUDGET:?}"
    );
}
