//! Review-session domain model (spec 08 Unit 3): the per-file review-status
//! tri-state ([`ReviewStatus`]) and its pure transition functions
//! ([`toggle_accept`]/[`accept`]/[`toggle_defer`]). Pure domain code — no TUI
//! types, no I/O — driven from `src/ui/review_ops.rs`, the presentation-side
//! seam that maps `Space`/`S`/`d` onto these functions against an
//! `App`-owned per-path status map (mirroring how `src/ui/staging.rs` drives
//! `staged_states`).
//!
//! Persistence (`review-state.json`, blob-SHA reconciliation, GC) is spec 08
//! Unit 4's `store`/`reconcile` submodules.

mod model;
mod reconcile;
pub mod store;

pub use model::{ReviewStatus, accept, toggle_accept, toggle_defer};
pub use reconcile::reconcile;
