//! redquill library crate: the reusable, TUI-free core.
//!
//! The binary (`main.rs`) is a thin CLI wrapper over these modules, and the
//! integration tests exercise them directly.

pub mod annotate;
pub mod config;
pub mod diff;
pub mod git;
pub mod highlight;
pub mod lsp;
pub mod review;
pub mod search;
pub mod ui;
