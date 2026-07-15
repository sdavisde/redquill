//! Non-UI search domain (spec 06): the fuzzy file-finder core (Unit 1) —
//! candidate model plus `nucleo-matcher` ranking glue. No TUI types; `crate::ui`
//! composes this with the background-task poller and renders results (see
//! `ui::file_finder`).
//!
//! Unit 2 (Project Search) adds the in-process grep engine: [`query`] is the
//! pure query model and `grep-regex` matcher construction; [`engine`] is the
//! scan itself (parallel `.gitignore`-respecting walk, streaming sink,
//! cancellation, caps). `crate::ui` (task 3.0) wires the engine's channel
//! into the background-task poller and renders results.

pub mod engine;
pub mod files;
pub mod fuzzy;
pub mod query;

pub use engine::{ScanMessage, ScanOptions, ScanSummary, SearchHit, spawn_scan};
pub use files::{FileCandidate, merge_candidates};
pub use fuzzy::{FuzzyMatch, rank};
pub use query::{CaseMode, SearchError, SearchQuery, build_matcher};
