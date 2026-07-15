//! Non-UI search domain (spec 06): the fuzzy file-finder core (Unit 1) —
//! candidate model plus `nucleo-matcher` ranking glue. No TUI types; `crate::ui`
//! composes this with the background-task poller and renders results (see
//! `ui::file_finder`).
//!
//! Later tasks (Unit 2, Project Search) add further submodules for the
//! in-process grep engine; only what Unit 1 needs exists today.

pub mod files;
pub mod fuzzy;

pub use files::{FileCandidate, merge_candidates};
pub use fuzzy::{FuzzyMatch, rank};
