//! redquill library crate: the reusable, TUI-free core.
//!
//! The binary (`main.rs`) is a thin CLI wrapper over these modules, and the
//! integration tests exercise them directly. Only `git` carries real logic so
//! far; the rest are module stubs filled in by later roadmap tasks.

pub mod annotate;
pub mod diff;
pub mod git;
pub mod lsp;
pub mod ui;
