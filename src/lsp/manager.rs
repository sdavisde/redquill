//! The app-facing LSP manager: owns server lifecycle, request/response
//! correlation, and the handshake/queueing dance, and exposes a
//! poll-based, non-blocking API to the rest of the app.
//!
//! [`LspManager`] is built for a synchronous, single-threaded render loop:
//! every public method returns immediately. `request_*` calls either hand
//! back a [`RequestId`] to correlate against a later [`LspEvent`] from
//! [`LspManager::poll`], or `None` when no server is or can be made
//! available for that file (unsupported extension, no configured command,
//! or a server that has already proven unreachable) — callers should treat
//! `None` as silent degradation, not an error to surface. `poll` never
//! blocks: it drains whatever has arrived on the background I/O threads
//! since the last call, sweeps timed-out requests, and returns.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::lsp::codec::encode_frame;
use crate::lsp::config::{LangServerCmd, ServerLang, default_commands, language_id};
use crate::lsp::event::{LspEvent, RequestId};
use crate::lsp::protocol::{
    build_definition, build_did_open, build_exit, build_hover, build_initialize, build_initialized,
    build_null_reply, build_references, build_shutdown, normalize_definition, normalize_hover,
    normalize_references, path_to_uri,
};
use crate::lsp::transport::{ServerHandle, WireEvent, spawn_server};

/// How long a request may sit unanswered before [`LspManager::poll`] gives
/// up on it and yields [`LspEvent::Failed`].
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// How long [`LspManager::shutdown`] waits for a server to exit after
/// `shutdown`/`exit` before killing it outright.
const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);

/// Which of the three supported requests a pending/queued entry represents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RequestKind {
    Definition,
    References,
    Hover,
}

/// Bookkeeping for a request that has been sent (or queued) and is awaiting
/// a response or a timeout.
struct Pending {
    kind: RequestKind,
    lang: ServerLang,
    sent: Instant,
}

/// A request that arrived while its server was still handshaking, held
/// until `initialize` completes.
struct QueuedRequest {
    wire_id: u64,
    kind: RequestKind,
    path: PathBuf,
    line: u32,
    character: u32,
}

/// Handshake state for a single server process.
enum ServerState {
    /// Waiting on the response to the `initialize` request with id
    /// `init_id`. Requests made in this state are held in `queued` rather
    /// than sent immediately.
    Initializing {
        init_id: u64,
        queued: Vec<QueuedRequest>,
    },
    /// Handshake complete; requests are dispatched immediately.
    Ready,
}

/// A live server process, its handshake state, and which files have already
/// been sent a `textDocument/didOpen`.
struct Server {
    handle: ServerHandle,
    state: ServerState,
    opened: HashSet<PathBuf>,
}

impl Drop for Server {
    fn drop(&mut self) {
        // Safety net: if the server wasn't already reaped via
        // `LspManager::shutdown`, make sure it doesn't outlive us. Killing
        // (or waiting on) an already-exited child is a harmless no-op error
        // we ignore.
        let _ = self.handle.child.kill();
        let _ = self.handle.child.wait();
    }
}

/// The state of a language's server slot: either a live process or a
/// language that has already proven unreachable and won't be retried.
enum Slot {
    Live(Server),
    Unavailable,
}

/// App-facing LSP client. See the module docs for the poll-based contract.
pub struct LspManager {
    root: PathBuf,
    commands: HashMap<ServerLang, LangServerCmd>,
    /// Absent key = a server for that language has never been tried yet.
    servers: HashMap<ServerLang, Slot>,
    pending: HashMap<u64, Pending>,
    /// Request ids that failed synchronously inside `request()` (e.g. a
    /// dispatch that couldn't be built) and need a `Failed` event on the
    /// next `poll`.
    immediate_failures: Vec<RequestId>,
    /// Monotonic id counter; also used for `initialize`/`shutdown` ids.
    next_id: u64,
    timeout: Duration,
    /// Kept around (never `try_recv`'d against being disconnected) so that
    /// cloning it per spawned server always succeeds.
    events_tx: mpsc::Sender<WireEvent>,
    events_rx: mpsc::Receiver<WireEvent>,
}

impl LspManager {
    /// Creates a manager rooted at `root` using the built-in default server
    /// commands (see [`crate::lsp::config::default_commands`]).
    pub fn new(root: PathBuf) -> LspManager {
        Self::with_commands(root, default_commands())
    }

    /// Creates a manager rooted at `root` using an explicit language ->
    /// command table. Public so tests (and future user configuration) can
    /// swap in different server commands.
    pub fn with_commands(
        root: PathBuf,
        commands: HashMap<ServerLang, LangServerCmd>,
    ) -> LspManager {
        let (events_tx, events_rx) = mpsc::channel();
        LspManager {
            root,
            commands,
            servers: HashMap::new(),
            pending: HashMap::new(),
            immediate_failures: Vec::new(),
            next_id: 1,
            timeout: DEFAULT_REQUEST_TIMEOUT,
            events_tx,
            events_rx,
        }
    }

    /// Overrides the default 5-second request timeout.
    pub fn set_request_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    /// Requests `textDocument/definition` for a 0-based `line`/`character`
    /// position in `path` (callers convert from 1-based UI positions).
    /// Returns `None` when no server is or can be made available for this
    /// file.
    pub fn request_definition(
        &mut self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Option<RequestId> {
        self.request(RequestKind::Definition, path, line, character)
    }

    /// Requests `textDocument/references`. See [`Self::request_definition`]
    /// for position conventions and the meaning of `None`.
    pub fn request_references(
        &mut self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Option<RequestId> {
        self.request(RequestKind::References, path, line, character)
    }

    /// Requests `textDocument/hover`. See [`Self::request_definition`] for
    /// position conventions and the meaning of `None`.
    pub fn request_hover(&mut self, path: &Path, line: u32, character: u32) -> Option<RequestId> {
        self.request(RequestKind::Hover, path, line, character)
    }

    /// Drains and returns every [`LspEvent`] produced since the last call.
    /// Never blocks.
    pub fn poll(&mut self) -> Vec<LspEvent> {
        let mut events = Vec::new();

        for id in self.immediate_failures.drain(..) {
            events.push(LspEvent::Failed { id });
        }

        while let Ok(ev) = self.events_rx.try_recv() {
            match ev {
                WireEvent::Message { lang, msg } => self.handle_message(lang, msg, &mut events),
                WireEvent::Exited { lang } => self.handle_exit(lang, &mut events),
            }
        }

        let timeout = self.timeout;
        let timed_out: Vec<u64> = self
            .pending
            .iter()
            .filter(|(_, p)| p.sent.elapsed() > timeout)
            .map(|(id, _)| *id)
            .collect();
        // A request that times out here and is answered later is simply
        // ignored by `handle_message`, since its id is no longer in
        // `pending`.
        for id in timed_out {
            self.pending.remove(&id);
            events.push(LspEvent::Failed { id: RequestId(id) });
        }

        events
    }

    /// Gracefully stops every live server: sends `shutdown`/`exit`, gives
    /// each a short grace period to exit on its own, and kills any survivor
    /// via `Server`'s `Drop` impl.
    pub fn shutdown(mut self) {
        let langs: Vec<ServerLang> = self
            .servers
            .iter()
            .filter(|(_, slot)| matches!(slot, Slot::Live(_)))
            .map(|(lang, _)| *lang)
            .collect();

        let mut held = Vec::new();
        for lang in langs {
            let Some(Slot::Live(server)) = self.servers.remove(&lang) else {
                continue;
            };
            let id = self.alloc_id();
            // Sent back-to-back without waiting for the `shutdown`
            // response: a v1 simplification. Well-behaved servers tolerate
            // this, and the grace period below (plus the kill-on-drop
            // safety net) covers the rest.
            if let Some(payload) = build_shutdown(id) {
                let _ = server.handle.writer_tx.send(encode_frame(&payload));
            }
            if let Some(payload) = build_exit() {
                let _ = server.handle.writer_tx.send(encode_frame(&payload));
            }
            held.push(server);
        }

        let deadline = Instant::now() + SHUTDOWN_GRACE;
        for server in &mut held {
            loop {
                match server.handle.child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(25));
                    }
                    _ => break,
                }
            }
        }
        // `held` drops here; any process that didn't exit in time is killed
        // by `Server`'s `Drop` impl. Killing/waiting on an already-reaped
        // child is a harmless no-op error we ignore there.
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Shared implementation for the three `request_*` methods.
    fn request(
        &mut self,
        kind: RequestKind,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Option<RequestId> {
        let lang = ServerLang::from_path(path)?;

        match self.servers.get(&lang) {
            Some(Slot::Unavailable) => return None,
            Some(Slot::Live(_)) => {}
            None => {
                let Some(cmd) = self.commands.get(&lang) else {
                    self.servers.insert(lang, Slot::Unavailable);
                    return None;
                };
                match spawn_server(cmd, &self.root, lang, self.events_tx.clone()) {
                    Err(_) => {
                        self.servers.insert(lang, Slot::Unavailable);
                        return None;
                    }
                    Ok(handle) => {
                        let init_id = self.alloc_id();
                        let Some(payload) = build_initialize(init_id, &self.root) else {
                            self.servers.insert(lang, Slot::Unavailable);
                            return None;
                        };
                        let _ = handle.writer_tx.send(encode_frame(&payload));
                        self.servers.insert(
                            lang,
                            Slot::Live(Server {
                                handle,
                                state: ServerState::Initializing {
                                    init_id,
                                    queued: Vec::new(),
                                },
                                opened: HashSet::new(),
                            }),
                        );
                    }
                }
            }
        }

        let wire_id = self.alloc_id();
        let abs_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };

        let Some(Slot::Live(server)) = self.servers.get_mut(&lang) else {
            // We just ensured a Live slot above (or returned already).
            return None;
        };

        match &mut server.state {
            ServerState::Initializing { queued, .. } => {
                queued.push(QueuedRequest {
                    wire_id,
                    kind,
                    path: abs_path,
                    line,
                    character,
                });
                // Queued requests sit in `pending` from creation, so the
                // timeout also covers slow handshakes, not just slow
                // responses.
                self.pending.insert(
                    wire_id,
                    Pending {
                        kind,
                        lang,
                        sent: Instant::now(),
                    },
                );
                Some(RequestId(wire_id))
            }
            ServerState::Ready => {
                match Self::dispatch_now(server, wire_id, kind, &abs_path, line, character) {
                    Ok(()) => {
                        self.pending.insert(
                            wire_id,
                            Pending {
                                kind,
                                lang,
                                sent: Instant::now(),
                            },
                        );
                        Some(RequestId(wire_id))
                    }
                    Err(()) => {
                        self.immediate_failures.push(RequestId(wire_id));
                        Some(RequestId(wire_id))
                    }
                }
            }
        }
    }

    /// Sends a `textDocument/didOpen` (if `path` hasn't been opened on this
    /// server yet) followed by the actual request. An associated function
    /// (rather than a method) so it only needs a `&mut Server`, avoiding a
    /// double borrow of `self.servers`.
    fn dispatch_now(
        server: &mut Server,
        wire_id: u64,
        kind: RequestKind,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<(), ()> {
        if !server.opened.contains(path) {
            let text = fs::read_to_string(path).map_err(|_| ())?;
            let uri = path_to_uri(path).ok_or(())?;
            let payload = build_did_open(&uri, language_id(path), &text).ok_or(())?;
            let _ = server.handle.writer_tx.send(encode_frame(&payload));
            server.opened.insert(path.to_path_buf());
        }

        let uri = path_to_uri(path).ok_or(())?;
        let payload = match kind {
            RequestKind::Definition => build_definition(wire_id, &uri, line, character),
            RequestKind::References => build_references(wire_id, &uri, line, character),
            RequestKind::Hover => build_hover(wire_id, &uri, line, character),
        }
        .ok_or(())?;
        let _ = server.handle.writer_tx.send(encode_frame(&payload));
        Ok(())
    }

    /// Handles one parsed JSON-RPC message read from `lang`'s server.
    fn handle_message(&mut self, lang: ServerLang, msg: Value, events: &mut Vec<LspEvent>) {
        if let Some(method) = msg.get("method").and_then(Value::as_str) {
            if let Some(id) = msg.get("id") {
                // Server-initiated request. We don't act on any of these
                // (e.g. `workspace/configuration`,
                // `window/workDoneProgress/create`); reply with a
                // reasonable-shaped null result so well-behaved servers
                // like rust-analyzer don't stall waiting for an answer.
                let result = if method == "workspace/configuration" {
                    let n = msg
                        .get("params")
                        .and_then(|p| p.get("items"))
                        .and_then(Value::as_array)
                        .map(|items| items.len())
                        .unwrap_or(1);
                    Value::Array(vec![Value::Null; n])
                } else {
                    Value::Null
                };
                if let Some(Slot::Live(server)) = self.servers.get(&lang)
                    && let Some(payload) = build_null_reply(id, result)
                {
                    let _ = server.handle.writer_tx.send(encode_frame(&payload));
                }
            }
            // Notifications from the server carry no `id` and we ignore them.
            return;
        }

        let Some(id) = msg.get("id").and_then(Value::as_u64) else {
            // Neither a request nor a response we can correlate; ignore.
            return;
        };

        let init_match = match self.servers.get(&lang) {
            Some(Slot::Live(server)) => {
                matches!(&server.state, ServerState::Initializing { init_id, .. } if *init_id == id)
            }
            _ => false,
        };

        if init_match {
            if msg.get("error").is_some() {
                self.handle_exit(lang, events);
            } else {
                self.finish_handshake(lang, events);
            }
            return;
        }

        let Some(pending) = self.pending.remove(&id) else {
            // Unknown or stale (already timed out) id; ignore.
            return;
        };

        if msg.get("error").is_some() {
            events.push(LspEvent::Failed { id: RequestId(id) });
            return;
        }

        let result = msg.get("result").cloned().unwrap_or(Value::Null);
        let event = match pending.kind {
            RequestKind::Definition => LspEvent::Definition {
                id: RequestId(id),
                locations: normalize_definition(result),
            },
            RequestKind::References => LspEvent::References {
                id: RequestId(id),
                locations: normalize_references(result),
            },
            RequestKind::Hover => match normalize_hover(result) {
                Some(contents) => LspEvent::Hover {
                    id: RequestId(id),
                    contents,
                },
                // An empty hover has nothing to show; treat it the same as
                // a failure rather than inventing an empty-string event.
                None => LspEvent::Failed { id: RequestId(id) },
            },
        };
        events.push(event);
    }

    /// Completes the `initialize` handshake for `lang`: flips its state to
    /// `Ready`, sends `initialized`, and dispatches everything that was
    /// queued while handshaking.
    fn finish_handshake(&mut self, lang: ServerLang, events: &mut Vec<LspEvent>) {
        let queued = {
            let Some(Slot::Live(server)) = self.servers.get_mut(&lang) else {
                return;
            };
            let queued = match std::mem::replace(&mut server.state, ServerState::Ready) {
                ServerState::Initializing { queued, .. } => queued,
                ServerState::Ready => Vec::new(),
            };
            if let Some(msg) = build_initialized() {
                let _ = server.handle.writer_tx.send(encode_frame(&msg));
            }
            queued
        };

        // Drop entries that already timed out while we were handshaking.
        let queued: Vec<_> = queued
            .into_iter()
            .filter(|q| self.pending.contains_key(&q.wire_id))
            .collect();

        let mut failed = Vec::new();
        if let Some(Slot::Live(server)) = self.servers.get_mut(&lang) {
            for q in queued {
                if Self::dispatch_now(server, q.wire_id, q.kind, &q.path, q.line, q.character)
                    .is_err()
                {
                    failed.push(q.wire_id);
                }
            }
        }
        for w in failed {
            self.pending.remove(&w);
            events.push(LspEvent::Failed { id: RequestId(w) });
        }
    }

    /// Handles a server process dying (stdout EOF/error): marks it
    /// `Unavailable` (dropping the old `Server`, which kills/reaps the
    /// child as a safety net) and fails every request still pending for it.
    fn handle_exit(&mut self, lang: ServerLang, events: &mut Vec<LspEvent>) {
        self.servers.insert(lang, Slot::Unavailable);

        let stale: Vec<u64> = self
            .pending
            .iter()
            .filter(|(_, p)| p.lang == lang)
            .map(|(id, _)| *id)
            .collect();
        for id in stale {
            self.pending.remove(&id);
            events.push(LspEvent::Failed { id: RequestId(id) });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> tempfile::TempDir {
        tempfile::TempDir::new().expect("tempdir")
    }

    #[test]
    fn unknown_extension_returns_none() {
        let tmp = temp_root();
        let mut mgr = LspManager::with_commands(tmp.path().to_path_buf(), default_commands());
        let file = tmp.path().join("foo.txt");
        assert_eq!(mgr.request_definition(&file, 0, 0), None);
        assert!(mgr.poll().is_empty());
    }

    #[test]
    fn lang_not_in_commands_table_returns_none() {
        let tmp = temp_root();
        let mut mgr = LspManager::with_commands(tmp.path().to_path_buf(), HashMap::new());
        let file = tmp.path().join("foo.rs");
        assert_eq!(mgr.request_definition(&file, 0, 0), None);
    }

    #[test]
    fn nonexistent_binary_short_circuits_without_retry() {
        let tmp = temp_root();
        let mut commands = HashMap::new();
        commands.insert(
            ServerLang::Rust,
            LangServerCmd {
                command: "redquill-no-such-server-binary-xyz".to_string(),
                args: vec![],
            },
        );
        let mut mgr = LspManager::with_commands(tmp.path().to_path_buf(), commands);
        let file = tmp.path().join("foo.rs");
        assert_eq!(mgr.request_definition(&file, 0, 0), None);
        assert_eq!(mgr.request_definition(&file, 0, 0), None);
    }

    #[cfg(unix)]
    #[test]
    fn unanswered_requests_time_out_to_failed_with_distinct_ids() {
        let tmp = temp_root();
        let mut commands = HashMap::new();
        commands.insert(
            ServerLang::Rust,
            LangServerCmd {
                command: "/bin/sleep".to_string(),
                args: vec!["30".to_string()],
            },
        );
        let mut mgr = LspManager::with_commands(tmp.path().to_path_buf(), commands);
        mgr.set_request_timeout(Duration::from_millis(100));

        let file = tmp.path().join("foo.rs");
        fs::write(&file, "fn main() {}\n").expect("write fixture");

        let id1 = mgr.request_definition(&file, 0, 0).expect("id1");
        let id2 = mgr.request_references(&file, 0, 0).expect("id2");
        let id3 = mgr.request_hover(&file, 0, 0).expect("id3");
        assert!(id1.0 < id2.0);
        assert!(id2.0 < id3.0);

        let mut seen: HashSet<RequestId> = HashSet::new();
        let deadline = Instant::now() + Duration::from_secs(3);
        while seen.len() < 3 && Instant::now() < deadline {
            for ev in mgr.poll() {
                match ev {
                    LspEvent::Failed { id } => {
                        assert!(id == id1 || id == id2 || id == id3, "unexpected id {id:?}");
                        seen.insert(id);
                    }
                    other => panic!("unexpected event: {other:?}"),
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            seen,
            HashSet::from([id1, id2, id3]),
            "expected all three requests to time out independently"
        );
    }

    #[cfg(unix)]
    #[test]
    fn dead_server_marks_language_unavailable() {
        let tmp = temp_root();
        let mut commands = HashMap::new();
        commands.insert(
            ServerLang::Rust,
            LangServerCmd {
                command: "true".to_string(),
                args: vec![],
            },
        );
        let mut mgr = LspManager::with_commands(tmp.path().to_path_buf(), commands);
        let file = tmp.path().join("foo.rs");

        let id = mgr
            .request_definition(&file, 0, 0)
            .expect("spawn should succeed even though the process exits immediately");

        let deadline = Instant::now() + Duration::from_secs(3);
        let mut failed = false;
        while Instant::now() < deadline {
            for ev in mgr.poll() {
                if let LspEvent::Failed { id: failed_id } = ev {
                    assert_eq!(failed_id, id);
                    failed = true;
                }
            }
            if failed {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            failed,
            "expected a Failed event once the dead server's exit is observed"
        );

        // The language is now marked Unavailable: no respawn attempt.
        assert_eq!(mgr.request_definition(&file, 0, 0), None);
    }

    #[cfg(unix)]
    #[test]
    fn dropping_the_manager_with_a_live_server_does_not_panic_or_hang() {
        let tmp = temp_root();
        let mut commands = HashMap::new();
        commands.insert(
            ServerLang::Rust,
            LangServerCmd {
                command: "/bin/sleep".to_string(),
                args: vec!["30".to_string()],
            },
        );
        let mut mgr = LspManager::with_commands(tmp.path().to_path_buf(), commands);
        let file = tmp.path().join("foo.rs");
        let _ = mgr.request_definition(&file, 0, 0);
        drop(mgr);
    }
}
