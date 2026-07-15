//! Non-UI search domain (spec 06): the fuzzy file-finder core (Unit 1) —
//! candidate model plus `nucleo-matcher` ranking glue. No TUI types; `crate::ui`
//! composes this with the background-task poller and renders results (see
//! `ui::file_finder`).
//!
//! Unit 2 (Project Search) adds the in-process grep engine: [`query`] is the
//! pure query model and `grep-regex` matcher construction (task 2.2); a
//! sibling `engine` module (task 2.3+) will add the scan itself.

pub mod files;
pub mod fuzzy;
pub mod query;

pub use files::{FileCandidate, merge_candidates};
pub use fuzzy::{FuzzyMatch, rank};
pub use query::{CaseMode, SearchError, SearchQuery, build_matcher};
