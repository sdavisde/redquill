//! Public event types produced by the LSP layer.
//!
//! These types are the boundary between the async transport/manager
//! machinery and the rest of the application: they carry no process
//! handles, sockets, or `lsp_types` wire types, so they can be constructed
//! and compared in pure unit tests.

use std::path::PathBuf;

/// Identifier correlating an outgoing LSP request with the [`LspEvent`] that
/// eventually answers it.
///
/// Callers mint these (typically from a monotonically increasing counter)
/// when issuing a request, then match them against the `id` field on the
/// event that comes back out of `LspManager::poll`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RequestId(pub u64);

/// A resolved source location: an absolute file path plus a 0-based
/// line/character position, matching LSP's own indexing convention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocation {
    /// Absolute path to the file the location points into.
    pub path: PathBuf,
    /// 0-based line number.
    pub line: u32,
    /// 0-based UTF-16 code unit offset within the line.
    pub character: u32,
}

/// Events drained from `LspManager::poll`.
///
/// Every request-shaped event carries the [`RequestId`] of the request it
/// answers so the caller can correlate it back to the UI action that
/// triggered it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LspEvent {
    /// Response to a `textDocument/definition` request.
    Definition {
        id: RequestId,
        locations: Vec<SourceLocation>,
    },
    /// Response to a `textDocument/references` request.
    References {
        id: RequestId,
        locations: Vec<SourceLocation>,
    },
    /// Response to a `textDocument/hover` request, already flattened to a
    /// single displayable string.
    Hover { id: RequestId, contents: String },
    /// The request failed: an error response, a dead/unreachable server, a
    /// timeout, or (for hover) a response with no usable content.
    Failed { id: RequestId },
}
