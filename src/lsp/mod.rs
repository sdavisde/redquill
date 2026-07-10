//! Language server lifecycle management and the three supported requests:
//! definition, references, and hover. Must be fully async and never block
//! the render loop; missing or slow servers degrade silently.
//!
//! Module layout:
//!
//! - [`codec`] — pure `Content-Length` frame encode/decode over raw bytes.
//! - [`config`] — language detection and default server launch commands.
//! - [`protocol`] — JSON-RPC message construction, response
//!   normalization, and `file://` URI conversion, all pure/no I/O.
//! - [`event`] — the public event types the rest of the app consumes.
//! - [`transport`] — child process spawning and the reader/writer I/O
//!   threads that bridge a server's stdio to plain channels.
//! - [`manager`] — server lifecycle (spawn, handshake, shutdown), request/
//!   response correlation, and the poll-based public API
//!   ([`LspManager`]) that turns raw server output into [`LspEvent`]s.

mod codec;
mod config;
mod event;
mod manager;
mod protocol;
mod transport;

pub use config::{LangServerCmd, ServerLang, default_commands};
pub use event::{LspEvent, RequestId, SourceLocation};
pub use manager::LspManager;
