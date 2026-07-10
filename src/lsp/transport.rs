//! Child process spawning and the reader/writer I/O threads that bridge a
//! language server's stdio to channels the [`crate::lsp::manager`] can poll
//! without ever blocking.
//!
//! Nothing here interprets JSON-RPC semantics beyond framing: this module's
//! job is turning bytes on a pipe into [`WireEvent`]s (and back), not
//! deciding what a message means.

use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::thread;

use crate::lsp::codec::FrameDecoder;
use crate::lsp::config::{LangServerCmd, ServerLang};

/// Size of the buffer used for each `read` call on a server's stdout.
const READ_BUF_SIZE: usize = 8 * 1024;

/// An event produced by a spawned server's I/O threads.
pub(crate) enum WireEvent {
    /// A parsed JSON-RPC message read from a server's stdout.
    Message {
        lang: ServerLang,
        msg: serde_json::Value,
    },
    /// The server's stdout reached EOF or errored: the process is
    /// considered dead.
    Exited { lang: ServerLang },
}

/// A running language server process plus the channel used to send it
/// pre-framed bytes.
pub(crate) struct ServerHandle {
    pub(crate) child: Child,
    /// Pre-framed bytes (already wrapped by [`crate::lsp::codec::encode_frame`])
    /// to be written to the server's stdin by the writer thread.
    pub(crate) writer_tx: mpsc::Sender<Vec<u8>>,
}

/// Spawns `cmd` as a child process rooted at `root`, wires its stdio to two
/// detached threads, and returns a handle for sending it framed messages.
///
/// The reader and writer threads are intentionally detached rather than
/// joined: they terminate on their own once the pipe closes (writer: all
/// senders dropped or a write fails; reader: EOF or a read error), so
/// nothing needs to join them. Joining from the render loop would risk
/// blocking indefinitely on a hung or misbehaving child process, which is
/// exactly what this module exists to avoid.
pub(crate) fn spawn_server(
    cmd: &LangServerCmd,
    root: &Path,
    lang: ServerLang,
    events_tx: mpsc::Sender<WireEvent>,
) -> io::Result<ServerHandle> {
    let mut child = Command::new(&cmd.command)
        .args(&cmd.args)
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("spawned server had no stdin"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("spawned server had no stdout"))?;

    let (writer_tx, writer_rx) = mpsc::channel::<Vec<u8>>();

    thread::spawn(move || writer_loop(stdin, writer_rx));
    thread::spawn(move || reader_loop(stdout, lang, events_tx));

    Ok(ServerHandle { child, writer_tx })
}

/// Writes pre-framed byte buffers to `stdin` as they arrive on `rx`. Exits
/// once every sender is dropped or a write to the pipe fails.
fn writer_loop(mut stdin: ChildStdin, rx: mpsc::Receiver<Vec<u8>>) {
    while let Ok(buf) = rx.recv() {
        if stdin.write_all(&buf).is_err() || stdin.flush().is_err() {
            break;
        }
    }
}

/// Reads raw bytes from `stdout`, feeds them through a [`FrameDecoder`], and
/// emits a [`WireEvent::Message`] for each successfully parsed frame.
/// Malformed JSON within an otherwise well-framed payload is skipped
/// silently; decoder errors are tolerated because the decoder always
/// discards at least the offending header, guaranteeing forward progress.
/// Sends a final [`WireEvent::Exited`] once the pipe closes.
fn reader_loop(mut stdout: ChildStdout, lang: ServerLang, events_tx: mpsc::Sender<WireEvent>) {
    let mut decoder = FrameDecoder::new();
    let mut buf = [0u8; READ_BUF_SIZE];

    loop {
        let n = match stdout.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        decoder.feed(&buf[..n]);

        loop {
            match decoder.next_frame() {
                Ok(Some(payload)) => {
                    if let Ok(msg) = serde_json::from_slice::<serde_json::Value>(&payload)
                        && events_tx.send(WireEvent::Message { lang, msg }).is_err()
                    {
                        // Manager is gone; nothing left to do.
                        return;
                    }
                    // Malformed JSON inside a well-formed frame: skip it and
                    // keep draining any further complete frames.
                }
                Ok(None) => break,
                // Decoder self-resyncs and always discards at least the bad
                // header, so retrying is guaranteed to make progress.
                Err(_) => continue,
            }
        }
    }

    let _ = events_tx.send(WireEvent::Exited { lang });
}
