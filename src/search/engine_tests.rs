use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;

use tempfile::TempDir;

use super::*;
use crate::search::query::CaseMode;

/// Creates a tempdir git repo (`git init -q`), canonicalized (macOS `/var`
/// symlink), and writes `files` (repo-relative path -> content) into it,
/// creating parent directories as needed. The `TempDir` guard must stay
/// alive for the duration of the test (dropping it deletes the directory).
fn repo(files: &[(&str, &str)]) -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().expect("canonicalize tempdir");
    let status = Command::new("git")
        .arg("init")
        .arg("-q")
        .arg(&root)
        .status()
        .expect("git init runs");
    assert!(status.success(), "git init must succeed");
    write_files(&root, files);
    (dir, root)
}

fn write_files(root: &Path, files: &[(&str, &str)]) {
    for (path, content) in files {
        let full = root.join(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).expect("create parent dirs");
        }
        fs::write(&full, content).expect("write fixture file");
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

/// Runs a scan to completion, collecting every hit (in receive order) and
/// the final summary.
fn run_to_completion(
    root: PathBuf,
    q: SearchQuery,
    generation: u64,
    options: ScanOptions,
) -> (Vec<SearchHit>, ScanSummary) {
    let (rx, _abort) = spawn_scan(root, q, generation, options).expect("valid query spawns");
    let mut hits = Vec::new();
    loop {
        match rx.recv().expect("channel stays open until Done") {
            ScanMessage::Batch(batch) => hits.extend(batch),
            ScanMessage::Done(summary) => return (hits, summary),
        }
    }
}

#[test]
fn regex_default_matches_a_pattern() {
    let (_dir, root) = repo(&[("src/main.rs", "fn main() {\n    println!(\"hi\");\n}\n")]);
    let (hits, summary) = run_to_completion(root, query(r"fn\s+main"), 1, ScanOptions::default());
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].path, "src/main.rs");
    assert_eq!(hits[0].line_number, 1);
    assert_eq!(hits[0].line_text, "fn main() {");
    assert_eq!(summary.total_hits, 1);
    assert_eq!(summary.files_scanned, 1);
    assert_eq!(summary.files_matched, 1);
    assert!(!summary.capped);
    assert!(!summary.aborted);
}

#[test]
fn match_spans_locate_the_match_within_the_line() {
    let (_dir, root) = repo(&[("a.txt", "xx needle yy\n")]);
    let (hits, _summary) = run_to_completion(root, query("needle"), 1, ScanOptions::default());
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].match_spans, vec![3..9]);
    assert_eq!(&hits[0].line_text[3..9], "needle");
}

#[test]
fn smart_case_lowercase_pattern_matches_any_case() {
    let (_dir, root) = repo(&[("a.txt", "Hello World\nhello world\n")]);
    let (hits, _) = run_to_completion(root, query("hello"), 1, ScanOptions::default());
    assert_eq!(hits.len(), 2);
}

#[test]
fn smart_case_uppercase_pattern_is_case_sensitive() {
    let (_dir, root) = repo(&[("a.txt", "Hello World\nhello world\n")]);
    let (hits, _) = run_to_completion(root, query("Hello"), 1, ScanOptions::default());
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].line_text, "Hello World");
}

#[test]
fn explicit_case_sensitive_overrides_pattern_casing() {
    let (_dir, root) = repo(&[("a.txt", "hello\nHELLO\n")]);
    let q = SearchQuery {
        pattern: "hello".to_string(),
        case: CaseMode::Sensitive,
        whole_word: false,
        literal: false,
    };
    let (hits, _) = run_to_completion(root, q, 1, ScanOptions::default());
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].line_text, "hello");
}

#[test]
fn explicit_case_insensitive_overrides_pattern_casing() {
    let (_dir, root) = repo(&[("a.txt", "hello\nHELLO\n")]);
    let q = SearchQuery {
        pattern: "Hello".to_string(),
        case: CaseMode::Insensitive,
        whole_word: false,
        literal: false,
    };
    let (hits, _) = run_to_completion(root, q, 1, ScanOptions::default());
    assert_eq!(hits.len(), 2);
}

#[test]
fn whole_word_excludes_substring_matches() {
    let (_dir, root) = repo(&[("a.txt", "the cat sat\nconcatenate\n")]);
    let q = SearchQuery {
        pattern: "cat".to_string(),
        case: CaseMode::Smart,
        whole_word: true,
        literal: false,
    };
    let (hits, _) = run_to_completion(root, q, 1, ScanOptions::default());
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].line_text, "the cat sat");
}

#[test]
fn literal_mode_treats_metacharacters_as_text() {
    let (_dir, root) = repo(&[("a.txt", "a.b\naxb\n")]);
    let q = SearchQuery {
        pattern: "a.b".to_string(),
        case: CaseMode::Smart,
        whole_word: false,
        literal: true,
    };
    let (hits, _) = run_to_completion(root, q, 1, ScanOptions::default());
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].line_text, "a.b");
}

#[test]
fn gitignored_files_are_excluded_and_untracked_unignored_are_included() {
    let (_dir, root) = repo(&[
        (".gitignore", "ignored.txt\n"),
        ("ignored.txt", "needle in the ignored file\n"),
        ("untracked.txt", "needle in an untracked file\n"),
    ]);
    // Neither file is `git add`ed — both are untracked. `untracked.txt` is
    // not covered by `.gitignore`, so it must still be found.
    let (hits, summary) = run_to_completion(root, query("needle"), 1, ScanOptions::default());
    let paths: Vec<&str> = hits.iter().map(|h| h.path.as_str()).collect();
    assert_eq!(paths, vec!["untracked.txt"]);
    assert_eq!(summary.total_hits, 1);
}

#[test]
fn dot_git_directory_is_never_scanned() {
    let (_dir, root) = repo(&[("a.txt", "unrelated\n")]);
    // Plant a file inside `.git/` with content that would match if `.git/`
    // were walked — proves the hidden-directory default keeps it out,
    // independent of whether real git internals happen to contain the text.
    fs::write(root.join(".git/fake-marker.txt"), "findme token\n")
        .expect("write into .git for the test");
    let (hits, summary) = run_to_completion(root, query("findme"), 1, ScanOptions::default());
    assert!(hits.is_empty(), "a `.git/` file must never be searched");
    assert_eq!(summary.total_hits, 0);
}

#[test]
fn binary_files_are_skipped_and_counted() {
    let (dir, root) = repo(&[]);
    let path = root.join("binary.dat");
    fs::write(&path, b"prefix findme \x00 trailing binary junk").expect("write binary fixture");
    let (hits, summary) = run_to_completion(root, query("findme"), 1, ScanOptions::default());
    assert!(hits.is_empty(), "a binary file must not contribute hits");
    assert_eq!(summary.binary_skipped, 1);
    assert_eq!(summary.files_scanned, 0);
    drop(dir);
    drop(path);
}

#[test]
fn oversized_files_are_skipped_and_counted() {
    let (_dir, root) = repo(&[(
        "big.txt",
        "findme plus enough bytes to exceed the tiny limit\n",
    )]);
    let options = ScanOptions {
        max_hits: DEFAULT_MAX_HITS,
        max_file_size: 5,
    };
    let (hits, summary) = run_to_completion(root, query("findme"), 1, options);
    assert!(
        hits.is_empty(),
        "an oversized file must not contribute hits"
    );
    assert_eq!(summary.oversized_skipped, 1);
    assert_eq!(summary.files_scanned, 0);
}

#[test]
fn cap_stops_contributing_new_hits_and_reports_capped() {
    let mut content = String::new();
    for i in 0..10 {
        content.push_str(&format!("needle line {i}\n"));
    }
    let (_dir, root) = repo(&[
        ("a.txt", content.as_str()),
        ("b.txt", content.as_str()),
        ("c.txt", content.as_str()),
    ]);
    let options = ScanOptions {
        max_hits: 7,
        max_file_size: DEFAULT_MAX_FILE_SIZE,
    };
    let (hits, summary) = run_to_completion(root, query("needle"), 1, options);
    assert_eq!(hits.len(), 7);
    assert_eq!(summary.total_hits, 7);
    assert!(summary.capped);
}

#[test]
fn generation_is_stamped_on_every_hit_and_the_summary() {
    let (_dir, root) = repo(&[("a.txt", "needle\nneedle\n")]);
    let (hits, summary) = run_to_completion(root, query("needle"), 42, ScanOptions::default());
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().all(|h| h.generation == 42));
    assert_eq!(summary.generation, 42);
}

#[test]
fn hits_stream_in_more_than_one_batch_when_they_exceed_batch_size() {
    let mut content = String::new();
    for i in 0..200 {
        content.push_str(&format!("needle line {i}\n"));
    }
    let (_dir, root) = repo(&[("a.txt", content.as_str())]);
    let (rx, _abort) =
        spawn_scan(root, query("needle"), 1, ScanOptions::default()).expect("valid query spawns");
    let mut batch_count = 0usize;
    let mut total = 0usize;
    loop {
        match rx.recv().expect("channel stays open until Done") {
            ScanMessage::Batch(batch) => {
                assert!(!batch.is_empty());
                assert!(batch.len() <= 64, "batches must stay small");
                batch_count += 1;
                total += batch.len();
            }
            ScanMessage::Done(summary) => {
                assert_eq!(total, 200);
                assert_eq!(summary.total_hits, 200);
                break;
            }
        }
    }
    assert!(
        batch_count >= 3,
        "200 hits at a 64-hit batch size must arrive in several batches, got {batch_count}"
    );
}

#[test]
fn invalid_regex_returns_a_typed_error_without_spawning_a_scan() {
    let (_dir, root) = repo(&[("a.txt", "hello\n")]);
    let result = spawn_scan(root, query("(unclosed"), 1, ScanOptions::default());
    assert!(matches!(result, Err(SearchError::InvalidPattern(_))));
}

#[test]
fn abort_flag_stops_a_mid_scan_promptly() {
    // A corpus large enough that a full scan measurably outlasts the time it
    // takes this thread to receive one message and flip the abort flag —
    // avoids relying on a sleep or a race that could occasionally pass by
    // coincidence (the scan happening to finish before abort is even set).
    const FILE_COUNT: usize = 5_000;
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
        fs::write(root.join(format!("file_{i:05}.txt")), "target line\n")
            .expect("write corpus file");
    }

    let (rx, abort) =
        spawn_scan(root, query("target"), 1, ScanOptions::default()).expect("valid query spawns");
    // Read exactly one message, then abort immediately — no delay, so any
    // slack in the scan's progress only helps prove promptness, not fake it.
    let first = rx.recv().expect("at least one message arrives");
    assert!(
        matches!(first, ScanMessage::Batch(_)),
        "a 5,000-file corpus must not finish in a single message before abort lands"
    );
    abort.store(true, Ordering::Relaxed);

    let summary = loop {
        match rx.recv().expect("channel stays open until Done") {
            ScanMessage::Batch(_) => continue,
            ScanMessage::Done(summary) => break summary,
        }
    };
    assert!(summary.aborted, "summary must report the scan was aborted");
    assert!(
        summary.files_scanned < FILE_COUNT,
        "abort must stop the scan before every file is scanned (scanned {} of {})",
        summary.files_scanned,
        FILE_COUNT
    );
}
