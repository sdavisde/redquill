//! The LSP seam between the TUI and the language-server manager:
//! [`LspClient`] is the small trait [`super::App`] drives `gd`/`gr`/`K`
//! requests through, implemented by [`LspManager`] for real sessions and by
//! a recording fake in unit tests. Mirrors [`super::stage_ops::StageOps`]'s
//! role for git.

use std::path::Path;

use crate::lsp::{LspEvent, LspManager, RequestId};

/// The LSP operations the TUI needs, kept behind a trait so `App`'s request
/// routing and event handling are unit-testable without spawning real
/// language servers. [`LspManager`] is the production implementation.
///
/// `Send` (spec 03 Unit 3): a worktree re-root shuts down the old client
/// off-thread (`take_lsp_client` + a spawned `shutdown` call) so the render
/// loop never blocks on server teardown, which requires `Box<dyn LspClient>`
/// to cross a thread boundary.
pub trait LspClient: Send {
    /// Requests `textDocument/definition`. See
    /// [`LspManager::request_definition`] for position conventions.
    fn request_definition(&mut self, path: &Path, line: u32, character: u32) -> Option<RequestId>;
    /// Requests `textDocument/references`.
    fn request_references(&mut self, path: &Path, line: u32, character: u32) -> Option<RequestId>;
    /// Requests `textDocument/hover`.
    fn request_hover(&mut self, path: &Path, line: u32, character: u32) -> Option<RequestId>;
    /// Drains events produced since the last call. Never blocks.
    fn poll(&mut self) -> Vec<LspEvent>;
    /// Gracefully stops every live server backing this client.
    fn shutdown(self: Box<Self>);
}

impl LspClient for LspManager {
    fn request_definition(&mut self, path: &Path, line: u32, character: u32) -> Option<RequestId> {
        LspManager::request_definition(self, path, line, character)
    }

    fn request_references(&mut self, path: &Path, line: u32, character: u32) -> Option<RequestId> {
        LspManager::request_references(self, path, line, character)
    }

    fn request_hover(&mut self, path: &Path, line: u32, character: u32) -> Option<RequestId> {
        LspManager::request_hover(self, path, line, character)
    }

    fn poll(&mut self) -> Vec<LspEvent> {
        LspManager::poll(self)
    }

    fn shutdown(self: Box<Self>) {
        LspManager::shutdown(*self)
    }
}
