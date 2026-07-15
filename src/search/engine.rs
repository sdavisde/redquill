//! In-process search engine (spec 06 Unit 2): the embedded ripgrep engine
//! behind Project Search. Pure — no TUI types; `crate::ui` (task 3.0) drains
//! [`spawn_scan`]'s channel once per tick, alongside the existing
//! `BackgroundTasks` polls, and renders [`SearchHit`]s.
//!
//! ## Shape of a scan
//!
//! [`spawn_scan`] compiles the [`SearchQuery`] into a `grep-regex`
//! [`grep_regex::RegexMatcher`] synchronously (so an invalid pattern is
//! reported to the caller immediately, before any thread runs or any prior
//! good results are disturbed), then spawns a background thread that:
//!
//! 1. Walks `root` in parallel with [`ignore::WalkBuilder`] — respects
//!    `.gitignore`/`.ignore`/global-gitignore, and (via the walker's
//!    default `hidden(true)`) skips hidden entries including `.git/`,
//!    matching `rg`'s own default behavior without `--hidden`. Untracked
//!    files that aren't gitignored are ordinary filesystem entries to this
//!    walker, so they're included — no separate "tracked vs untracked"
//!    distinction exists at this layer (unlike `crate::search::files`, which
//!    deliberately sources from `git ls-files`).
//! 2. For each regular file: skips it (counted) if its size exceeds
//!    `options.max_file_size`, or if its content contains a NUL byte
//!    (treated as binary) — checked once per file, on the fully-read
//!    content, rather than relying on `grep-searcher`'s own quit-on-NUL
//!    detection, so a binary file is skipped as a whole rather than
//!    partially searched up to its first NUL byte.
//! 3. Searches the file's content with `grep-searcher` using the compiled
//!    matcher, computing per-line match spans via `grep_matcher::Matcher`'s
//!    `find_iter` against the matched line.
//! 4. Streams hits over a bounded channel in small batches (so first results
//!    are visible while the scan continues), tagging every [`SearchHit`]
//!    with the caller-supplied `generation` so a consumer draining a shared
//!    channel across query changes can drop stragglers from a superseded
//!    scan.
//!
//! Cancellation: the returned `Arc<AtomicBool>` is checked in the sink (after
//! every matched line) and between files (in the walker callback), so setting
//! it stops the scan promptly rather than waiting for the whole tree walk to
//! finish. A cap (default [`DEFAULT_MAX_HITS`]) is enforced via a shared
//! atomic counter: once reached, further hits are dropped and
//! [`ScanSummary::capped`] is set — this is exact (never overshoots), not a
//! racy approximation, because the reservation is a compare-exchange loop.
//!
//! Per-file I/O errors (permission denied, and the rare non-UTF-8-without-NUL
//! content that defeats the binary sniff) degrade silently to the `errored`
//! counter rather than aborting the whole scan or panicking — a documented
//! silent-degradation contract, matching `crate::lsp`'s convention for
//! individually-failing units of background work.

use std::fs;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;

use grep_matcher::Matcher;
use grep_regex::RegexMatcher;
use grep_searcher::Searcher;
use grep_searcher::sinks::UTF8;
use ignore::{WalkBuilder, WalkState};

use super::query::{SearchError, SearchQuery, build_matcher};

/// Default cap on total hits collected in one scan (spec 06 Unit 2, open
/// question 2 — a proposed default, tunable without affecting scope): beyond
/// this, matching stops contributing new hits and [`ScanSummary::capped`] is
/// set, with the UI expected to show "capped — refine your query".
pub const DEFAULT_MAX_HITS: usize = 10_000;

/// Default per-file size ceiling (bytes) above which a file is skipped
/// (counted in [`ScanSummary::oversized_skipped`]) rather than read and
/// searched — keeps a stray large generated artifact from blowing the
/// instant-feel budget. 1 MiB comfortably covers real source files while
/// excluding e.g. bundled binaries or generated lockfiles that happen to be
/// text.
pub const DEFAULT_MAX_FILE_SIZE: u64 = 1_000_000;

/// Hits per [`ScanMessage::Batch`] sent down the channel — small enough that
/// the first batch (and therefore the first rendered results) is available
/// while the scan continues, per the streaming requirement.
const BATCH_SIZE: usize = 64;

/// Bounded channel capacity, in batches. Backpressure here is deliberate:
/// once the consumer stops draining (e.g. the UI dropped the receiver
/// because the user cancelled), producer threads block on `send`, which
/// combined with the abort-flag checks below stops new work promptly rather
/// than racing ahead to buffer results nobody will read.
const CHANNEL_CAPACITY: usize = 8;

/// Tunable scan limits. [`ScanOptions::default`] matches the spec's proposed
/// defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanOptions {
    /// Global cap on total hits collected across the whole scan.
    pub max_hits: usize,
    /// Per-file size ceiling (bytes); files larger than this are skipped.
    pub max_file_size: u64,
}

impl Default for ScanOptions {
    fn default() -> ScanOptions {
        ScanOptions {
            max_hits: DEFAULT_MAX_HITS,
            max_file_size: DEFAULT_MAX_FILE_SIZE,
        }
    }
}

/// One matched line: a repo-relative path (forward-slash, matching
/// [`crate::search::files::FileCandidate`]'s convention), the 1-based line
/// number `grep-searcher` reports, the line's text (line terminator
/// stripped), and the byte ranges within `line_text` that matched — for the
/// UI to emphasize the match span. Tagged with the `generation` the caller
/// supplied to [`spawn_scan`], so a consumer draining a shared channel
/// across query changes can drop stragglers from a superseded scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    /// Repo-relative path, forward-slash separated.
    pub path: String,
    /// 1-based line number within the file.
    pub line_number: u64,
    /// The line's text, with its line terminator stripped.
    pub line_text: String,
    /// Byte ranges within `line_text` that matched the query.
    pub match_spans: Vec<Range<usize>>,
    /// The generation the caller supplied to [`spawn_scan`].
    pub generation: u64,
}

/// Per-scan counters and terminal flags, sent as the final [`ScanMessage`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanSummary {
    /// The generation this scan was spawned with.
    pub generation: u64,
    /// Files whose content was actually read and searched.
    pub files_scanned: usize,
    /// Of `files_scanned`, how many contributed at least one hit.
    pub files_matched: usize,
    /// Total hits collected (capped at `max_hits`; see `capped`).
    pub total_hits: usize,
    /// Files skipped because their content contained a NUL byte.
    pub binary_skipped: usize,
    /// Files skipped because they exceeded `max_file_size`.
    pub oversized_skipped: usize,
    /// Files that could not be read/searched (I/O error, invalid UTF-8 not
    /// already caught by the binary check) — skipped silently, counted here.
    pub errored: usize,
    /// Whether the scan hit `max_hits` and stopped contributing new hits.
    pub capped: bool,
    /// Whether the scan was stopped early via the abort flag.
    pub aborted: bool,
}

/// One message streamed from a scan: an incremental batch of hits, or the
/// terminal summary. Exactly one `Done` is sent, always last, whether the
/// scan ran to completion, hit the cap, or was aborted.
#[derive(Debug)]
pub enum ScanMessage {
    /// A batch of newly-found hits, in the order found (not globally
    /// sorted — files are scanned concurrently across threads).
    Batch(Vec<SearchHit>),
    /// The scan has finished (or been aborted); no further messages follow.
    Done(ScanSummary),
}

/// Spawns a background scan of `root` for `query`, tagged with `generation`.
/// Returns immediately after compiling the matcher: an invalid pattern is
/// reported as `Err(SearchError)` synchronously (no thread spawned, no
/// channel activity — the "never wipe prior results on an error" rule is a
/// caller-side/UI concern this makes possible by failing before touching
/// either). On success, returns the receiving end of a bounded channel that
/// streams [`ScanMessage`]s, and the scan's abort flag: setting it to `true`
/// stops the scan promptly (checked in the sink and between files).
pub fn spawn_scan(
    root: PathBuf,
    query: SearchQuery,
    generation: u64,
    options: ScanOptions,
) -> Result<(Receiver<ScanMessage>, Arc<AtomicBool>), SearchError> {
    let matcher = build_matcher(&query)?;
    let (tx, rx) = mpsc::sync_channel(CHANNEL_CAPACITY);
    let abort = Arc::new(AtomicBool::new(false));
    let abort_for_thread = Arc::clone(&abort);
    thread::spawn(move || {
        run_scan(&root, &matcher, generation, options, &abort_for_thread, &tx);
    });
    Ok((rx, abort))
}

/// Outcome of scanning one file, for the caller's counters.
enum FileOutcome {
    /// The file was read and searched; `matched` is whether it contributed
    /// at least one hit.
    Scanned { matched: bool },
    /// Skipped: content contained a NUL byte.
    Binary,
    /// Skipped: exceeded `max_file_size`.
    Oversized,
}

/// Runs the parallel walk and search, sending [`ScanMessage`]s to `tx` as it
/// goes, then a final `Done` with the accumulated [`ScanSummary`]. Blocks the
/// calling thread until the walk finishes, is aborted, or the receiver is
/// dropped — callers run this on a background thread via [`spawn_scan`].
fn run_scan(
    root: &Path,
    matcher: &RegexMatcher,
    generation: u64,
    options: ScanOptions,
    abort: &AtomicBool,
    tx: &SyncSender<ScanMessage>,
) {
    let files_scanned = AtomicUsize::new(0);
    let files_matched = AtomicUsize::new(0);
    let total_hits = AtomicUsize::new(0);
    let binary_skipped = AtomicUsize::new(0);
    let oversized_skipped = AtomicUsize::new(0);
    let errored = AtomicUsize::new(0);
    let capped = AtomicBool::new(false);
    let disconnected = AtomicBool::new(false);

    let walker = WalkBuilder::new(root).build_parallel();
    walker.run(|| {
        let matcher = matcher.clone();
        let tx = tx.clone();
        let files_scanned = &files_scanned;
        let files_matched = &files_matched;
        let total_hits = &total_hits;
        let binary_skipped = &binary_skipped;
        let oversized_skipped = &oversized_skipped;
        let errored = &errored;
        let capped = &capped;
        let disconnected = &disconnected;
        Box::new(move |result: Result<ignore::DirEntry, ignore::Error>| {
            if abort.load(Ordering::Relaxed) || disconnected.load(Ordering::Relaxed) {
                return WalkState::Quit;
            }
            let entry = match result {
                Ok(entry) => entry,
                Err(_) => {
                    errored.fetch_add(1, Ordering::Relaxed);
                    return WalkState::Continue;
                }
            };
            let is_file = entry.file_type().map(|ft| ft.is_file()).unwrap_or(false);
            if !is_file {
                return WalkState::Continue;
            }
            if capped.load(Ordering::Relaxed) {
                // Cap already reached: skip further search work entirely.
                // `reserve_hit` inside `scan_one_file` would refuse every
                // hit anyway; this just avoids the wasted read+search.
                return WalkState::Continue;
            }
            let rel_path = relative_path(root, entry.path());
            match scan_one_file(
                entry.path(),
                &rel_path,
                &matcher,
                &options,
                generation,
                abort,
                disconnected,
                total_hits,
                capped,
                &tx,
            ) {
                Ok(FileOutcome::Scanned { matched }) => {
                    files_scanned.fetch_add(1, Ordering::Relaxed);
                    if matched {
                        files_matched.fetch_add(1, Ordering::Relaxed);
                    }
                }
                Ok(FileOutcome::Binary) => {
                    binary_skipped.fetch_add(1, Ordering::Relaxed);
                }
                Ok(FileOutcome::Oversized) => {
                    oversized_skipped.fetch_add(1, Ordering::Relaxed);
                }
                Err(_) => {
                    errored.fetch_add(1, Ordering::Relaxed);
                }
            }
            if abort.load(Ordering::Relaxed) || disconnected.load(Ordering::Relaxed) {
                WalkState::Quit
            } else {
                WalkState::Continue
            }
        })
    });

    let summary = ScanSummary {
        generation,
        files_scanned: files_scanned.load(Ordering::Relaxed),
        files_matched: files_matched.load(Ordering::Relaxed),
        total_hits: total_hits.load(Ordering::Relaxed),
        binary_skipped: binary_skipped.load(Ordering::Relaxed),
        oversized_skipped: oversized_skipped.load(Ordering::Relaxed),
        errored: errored.load(Ordering::Relaxed),
        capped: capped.load(Ordering::Relaxed),
        aborted: abort.load(Ordering::Relaxed),
    };
    // If the receiver is gone, there's nowhere for the summary to go — that's
    // fine, the caller has already stopped listening.
    let _ = tx.send(ScanMessage::Done(summary));
}

/// Reads, binary/size-checks, and searches one file, sending hit batches to
/// `tx` as they fill. Returns the file-level [`FileOutcome`] for the
/// caller's counters, or an `io::Error` for a file that couldn't be read or
/// searched (the caller counts this as `errored` and continues — see the
/// module doc's silent-degradation contract).
#[allow(clippy::too_many_arguments)]
fn scan_one_file(
    path: &Path,
    rel_path: &str,
    matcher: &RegexMatcher,
    options: &ScanOptions,
    generation: u64,
    abort: &AtomicBool,
    disconnected: &AtomicBool,
    total_hits: &AtomicUsize,
    capped: &AtomicBool,
    tx: &SyncSender<ScanMessage>,
) -> io::Result<FileOutcome> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > options.max_file_size {
        return Ok(FileOutcome::Oversized);
    }
    let content = fs::read(path)?;
    if content.contains(&0u8) {
        return Ok(FileOutcome::Binary);
    }

    let mut batch: Vec<SearchHit> = Vec::with_capacity(BATCH_SIZE);
    let mut matched = false;

    let search_result = Searcher::new().search_slice(
        matcher,
        &content,
        UTF8(|line_number, line| {
            if abort.load(Ordering::Relaxed) {
                return Ok(false);
            }
            if !reserve_hit(total_hits, options.max_hits) {
                capped.store(true, Ordering::Relaxed);
                return Ok(false);
            }
            matched = true;
            let trimmed = line.trim_end_matches(['\n', '\r']);
            batch.push(SearchHit {
                path: rel_path.to_string(),
                line_number,
                line_text: trimmed.to_string(),
                match_spans: match_spans(matcher, trimmed.as_bytes()),
                generation,
            });
            if batch.len() >= BATCH_SIZE {
                let ready = std::mem::take(&mut batch);
                if tx.send(ScanMessage::Batch(ready)).is_err() {
                    disconnected.store(true, Ordering::Relaxed);
                    return Ok(false);
                }
            }
            Ok(true)
        }),
    );

    // Flush any partial batch regardless of how the search ended, so hits
    // found before a later abort/cap/error aren't silently dropped.
    if !batch.is_empty() && tx.send(ScanMessage::Batch(batch)).is_err() {
        disconnected.store(true, Ordering::Relaxed);
    }

    // A `search_slice` error can only come from the sink's own error type
    // (`io::Error` here, per `UTF8`'s bound); our closure never returns one,
    // so this only guards a hypothetical future change and the rare case of
    // non-UTF-8 content that the NUL-byte binary check above didn't catch
    // (e.g. a UTF-16-encoded text file with no embedded NUL bytes in the
    // low-surrogate positions this check inspects... in practice this is
    // vanishingly rare, but documented rather than assumed impossible).
    search_result.map_err(|err| io::Error::other(err.to_string()))?;

    Ok(FileOutcome::Scanned { matched })
}

/// Reserves one slot in the global hit cap: returns `true` (and increments
/// the shared counter) if `total_hits` is still below `max_hits`, `false`
/// otherwise. A compare-exchange loop rather than a plain `fetch_add`, so
/// concurrent callers across worker threads can never push the counter past
/// `max_hits` — the cap is exact, not a racy approximation.
fn reserve_hit(total_hits: &AtomicUsize, max_hits: usize) -> bool {
    let mut current = total_hits.load(Ordering::Relaxed);
    loop {
        if current >= max_hits {
            return false;
        }
        match total_hits.compare_exchange_weak(
            current,
            current + 1,
            Ordering::SeqCst,
            Ordering::Relaxed,
        ) {
            Ok(_) => return true,
            Err(actual) => current = actual,
        }
    }
}

/// The byte ranges within `line` (already trimmed of its line terminator)
/// that `matcher` matches, for match-span highlighting. Degrades silently to
/// an empty vec on a matcher error — `grep-searcher` already matched this
/// exact line successfully via the same matcher, so a `find_iter` error here
/// would indicate an internal inconsistency rather than a real user-facing
/// failure; losing highlight emphasis on that theoretical case is an
/// acceptable, documented trade-off against propagating a fourth error path
/// out of a hot per-line loop.
fn match_spans(matcher: &RegexMatcher, line: &[u8]) -> Vec<Range<usize>> {
    let mut spans = Vec::new();
    let _ = matcher.find_iter(line, |m| {
        spans.push(m.start()..m.end());
        true
    });
    spans
}

/// `path` relative to `root`, forward-slash separated, matching
/// [`crate::search::files::FileCandidate`]'s path convention. Falls back to
/// `path` itself (defensive, not expected to trigger: `path` always comes
/// from walking `root`) if stripping the prefix fails, rather than panicking.
fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
