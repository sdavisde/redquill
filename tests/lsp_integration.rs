//! Integration tests for the `lsp` module's process/threading layer.
//!
//! Each test drives `LspManager` against a small fake language server
//! written in Python (gated on `python3` being on `PATH`): the fake server
//! speaks real `Content-Length`-framed JSON-RPC over stdio, which exercises
//! the full stack — process spawn, the codec, the handshake, request/
//! response correlation, `didOpen` deduplication, and server-initiated
//! `workspace/configuration` requests — without depending on a real
//! language server being installed.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use redquill::lsp::{LangServerCmd, LspEvent, LspManager, ServerLang, SourceLocation};
use tempfile::TempDir;

/// The fake language server. Speaks framed JSON-RPC over stdio; see the
/// per-method behavior inline. `sys.argv[1]`, if given, is a path to write
/// this process's pid to (used by the shutdown test).
const FAKE_SERVER_PY: &str = r#"
import json, os, sys

def read_msg(stream):
    headers = {}
    while True:
        line = stream.readline()
        if not line:
            return None
        line = line.decode("utf-8").strip()
        if line == "":
            break
        key, _, value = line.partition(":")
        headers[key.strip().lower()] = value.strip()
    length = int(headers.get("content-length", "0"))
    body = stream.read(length)
    if body is None or len(body) < length:
        return None
    return json.loads(body)

def send(obj):
    data = json.dumps(obj).encode("utf-8")
    sys.stdout.buffer.write(b"Content-Length: " + str(len(data)).encode() + b"\r\n\r\n")
    sys.stdout.buffer.write(data)
    sys.stdout.buffer.flush()

if len(sys.argv) > 1:
    with open(sys.argv[1], "w") as fh:
        fh.write(str(os.getpid()))

opens = {}
cfg_replies = 0
stdin = sys.stdin.buffer
while True:
    msg = read_msg(stdin)
    if msg is None:
        break
    method = msg.get("method")
    mid = msg.get("id")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": mid, "result": {"capabilities": {}}})
    elif method == "initialized":
        send({"jsonrpc": "2.0", "id": 9001, "method": "workspace/configuration",
              "params": {"items": [{}]}})
    elif method == "textDocument/didOpen":
        uri = msg["params"]["textDocument"]["uri"]
        opens[uri] = opens.get(uri, 0) + 1
    elif method == "textDocument/definition":
        if msg["params"]["position"]["line"] == 999:
            continue  # exercised by the timeout test: never answer
        uri = msg["params"]["textDocument"]["uri"]
        send({"jsonrpc": "2.0", "id": mid,
              "result": {"uri": uri,
                         "range": {"start": {"line": 3, "character": 7},
                                   "end": {"line": 3, "character": 9}}}})
    elif method == "textDocument/references":
        uri = msg["params"]["textDocument"]["uri"]
        locs = [{"uri": uri,
                 "range": {"start": {"line": line, "character": 0},
                           "end": {"line": line, "character": 1}}}
                for line in (1, 2)]
        send({"jsonrpc": "2.0", "id": mid, "result": locs})
    elif method == "textDocument/hover":
        uri = msg["params"]["textDocument"]["uri"]
        value = "opens=%d;cfg=%d" % (opens.get(uri, 0), cfg_replies)
        send({"jsonrpc": "2.0", "id": mid,
              "result": {"contents": {"kind": "markdown", "value": value}}})
    elif method == "shutdown":
        send({"jsonrpc": "2.0", "id": mid, "result": None})
    elif method == "exit":
        sys.exit(0)
    elif method is None and mid == 9001:
        cfg_replies += 1
"#;

/// Returns whether `python3` is available on `PATH`.
fn python3_available() -> bool {
    Command::new("python3")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// Writes the fake server script and a small Rust source fixture into
/// `tmp`, returning the script's path.
fn write_fixtures(tmp: &TempDir) -> PathBuf {
    let script = tmp.path().join("fake_server.py");
    fs::write(&script, FAKE_SERVER_PY).expect("write fake server script");
    fs::write(
        tmp.path().join("main.rs"),
        "fn main() {\n    println!(\"hi\");\n}\n",
    )
    .expect("write fixture source");
    script
}

/// Builds an `LspManager` rooted at `tmp` whose `ServerLang::Rust` command
/// launches the fake server via `python3`. If `pidfile` is given, it's
/// passed to the script as `sys.argv[1]`.
fn manager_for(tmp: &TempDir, pidfile: Option<&Path>) -> LspManager {
    let script = write_fixtures(tmp);
    let mut args = vec![script.to_string_lossy().into_owned()];
    if let Some(pidfile) = pidfile {
        args.push(pidfile.to_string_lossy().into_owned());
    }
    let mut commands = HashMap::new();
    commands.insert(
        ServerLang::Rust,
        LangServerCmd {
            command: "python3".to_string(),
            args,
        },
    );
    LspManager::with_commands(tmp.path().to_path_buf(), commands)
}

/// Polls `mgr` every 10ms, accumulating events, until `pred` is satisfied
/// or `deadline` elapses. Panics (including the events collected so far)
/// on timeout.
fn poll_until(
    mgr: &mut LspManager,
    deadline: Duration,
    mut pred: impl FnMut(&[LspEvent]) -> bool,
) -> Vec<LspEvent> {
    let start = Instant::now();
    let mut events = Vec::new();
    loop {
        events.extend(mgr.poll());
        if pred(&events) {
            return events;
        }
        if start.elapsed() > deadline {
            panic!("poll_until timed out; events so far: {events:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn definition_references_hover_flow_and_did_open_once() {
    if !python3_available() {
        eprintln!("skipping: python3 not found");
        return;
    }
    let tmp = TempDir::new().expect("tempdir");
    let mut mgr = manager_for(&tmp, None);
    let file = tmp.path().join("main.rs");
    let long_wait = Duration::from_secs(10);

    let id1 = mgr.request_definition(&file, 1, 2).expect("id1");
    let events = poll_until(&mut mgr, long_wait, |evs| {
        evs.iter().any(|e| matches!(e, LspEvent::Definition { .. }))
    });
    let def = events
        .iter()
        .find(|e| matches!(e, LspEvent::Definition { .. }))
        .expect("definition event");
    assert_eq!(
        def,
        &LspEvent::Definition {
            id: id1,
            locations: vec![SourceLocation {
                path: file.clone(),
                line: 3,
                character: 7,
            }],
        }
    );

    let id2 = mgr.request_references(&file, 0, 0).expect("id2");
    let events = poll_until(&mut mgr, long_wait, |evs| {
        evs.iter().any(|e| matches!(e, LspEvent::References { .. }))
    });
    let refs = events
        .iter()
        .find(|e| matches!(e, LspEvent::References { .. }))
        .expect("references event");
    assert_eq!(
        refs,
        &LspEvent::References {
            id: id2,
            locations: vec![
                SourceLocation {
                    path: file.clone(),
                    line: 1,
                    character: 0,
                },
                SourceLocation {
                    path: file.clone(),
                    line: 2,
                    character: 0,
                },
            ],
        }
    );

    let id3 = mgr.request_definition(&file, 0, 0).expect("id3");
    poll_until(&mut mgr, long_wait, |evs| {
        evs.iter()
            .any(|e| matches!(e, LspEvent::Definition { id, .. } if *id == id3))
    });

    let id4 = mgr.request_hover(&file, 0, 0).expect("id4");
    let events = poll_until(&mut mgr, long_wait, |evs| {
        evs.iter().any(|e| matches!(e, LspEvent::Hover { .. }))
    });
    let hover = events
        .iter()
        .find(|e| matches!(e, LspEvent::Hover { .. }))
        .expect("hover event");
    // opens=1: didOpen was sent exactly once across all four requests.
    // cfg=1: we answered the server-initiated workspace/configuration
    // request that followed our `initialized` notification.
    assert_eq!(
        hover,
        &LspEvent::Hover {
            id: id4,
            contents: "opens=1;cfg=1".to_string(),
        }
    );
}

#[test]
fn second_file_gets_its_own_did_open() {
    if !python3_available() {
        eprintln!("skipping: python3 not found");
        return;
    }
    let tmp = TempDir::new().expect("tempdir");
    let mut mgr = manager_for(&tmp, None);
    let a = tmp.path().join("main.rs");
    let b = tmp.path().join("b.rs");
    fs::write(&b, "fn other() {}\n").expect("write b.rs");
    let long_wait = Duration::from_secs(10);

    // This first hover is dispatched through the `Initializing` ->
    // `finish_handshake` immediate-dispatch path: `didOpen`+`hover` are
    // written to the server's stdin in the very same `poll()` call that
    // processes the `initialize` response, before the client can possibly
    // have replied to the server's `workspace/configuration` request (that
    // reply requires an extra scheduling round trip the fake server never
    // waits for). So `cfg=0` here is deterministic, not a race -- unlike
    // the warmed-up flow in `definition_references_hover_flow_and_did_open_once`,
    // where hover is the *last* of several round trips and so is dispatched
    // well after the configuration reply has already gone out.
    let id_a1 = mgr.request_hover(&a, 0, 0).expect("id_a1");
    let events = poll_until(&mut mgr, long_wait, |evs| {
        evs.iter()
            .any(|e| matches!(e, LspEvent::Hover { id, .. } if *id == id_a1))
    });
    let hover_a1 = events
        .iter()
        .find(|e| matches!(e, LspEvent::Hover { id, .. } if *id == id_a1))
        .expect("hover a1");
    assert_eq!(
        hover_a1,
        &LspEvent::Hover {
            id: id_a1,
            contents: "opens=1;cfg=0".to_string(),
        }
    );

    let id_b = mgr.request_hover(&b, 0, 0).expect("id_b");
    let events = poll_until(&mut mgr, long_wait, |evs| {
        evs.iter()
            .any(|e| matches!(e, LspEvent::Hover { id, .. } if *id == id_b))
    });
    let hover_b = events
        .iter()
        .find(|e| matches!(e, LspEvent::Hover { id, .. } if *id == id_b))
        .expect("hover b");
    assert_eq!(
        hover_b,
        &LspEvent::Hover {
            id: id_b,
            contents: "opens=1;cfg=1".to_string(),
        }
    );

    let id_a2 = mgr.request_hover(&a, 0, 0).expect("id_a2");
    let events = poll_until(&mut mgr, long_wait, |evs| {
        evs.iter()
            .any(|e| matches!(e, LspEvent::Hover { id, .. } if *id == id_a2))
    });
    let hover_a2 = events
        .iter()
        .find(|e| matches!(e, LspEvent::Hover { id, .. } if *id == id_a2))
        .expect("hover a2");
    assert_eq!(
        hover_a2,
        &LspEvent::Hover {
            id: id_a2,
            contents: "opens=1;cfg=1".to_string(),
        }
    );
}

#[test]
fn unanswered_request_times_out_to_failed() {
    if !python3_available() {
        eprintln!("skipping: python3 not found");
        return;
    }
    let tmp = TempDir::new().expect("tempdir");
    let mut mgr = manager_for(&tmp, None);
    let file = tmp.path().join("main.rs");
    let long_wait = Duration::from_secs(10);

    // Do one normal round-trip first so the handshake is done before we
    // start timing the actual request under test.
    let warm_id = mgr.request_hover(&file, 0, 0).expect("warm id");
    poll_until(&mut mgr, long_wait, |evs| {
        evs.iter()
            .any(|e| matches!(e, LspEvent::Hover { id, .. } if *id == warm_id))
    });

    mgr.set_request_timeout(Duration::from_millis(200));
    let id = mgr.request_definition(&file, 999, 0).expect("id");
    let events = poll_until(&mut mgr, Duration::from_secs(5), |evs| {
        evs.iter()
            .any(|e| matches!(e, LspEvent::Failed { id: fid } if *fid == id))
    });
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, LspEvent::Definition { id: did, .. } if *did == id)),
        "the unanswered request should never produce a Definition event"
    );
}

#[cfg(unix)]
#[test]
fn shutdown_terminates_the_server() {
    if !python3_available() {
        eprintln!("skipping: python3 not found");
        return;
    }
    let tmp = TempDir::new().expect("tempdir");
    let pidfile = tmp.path().join("server.pid");
    let mut mgr = manager_for(&tmp, Some(&pidfile));
    let file = tmp.path().join("main.rs");
    let long_wait = Duration::from_secs(10);

    let id = mgr.request_hover(&file, 0, 0).expect("id");
    poll_until(&mut mgr, long_wait, |evs| {
        evs.iter()
            .any(|e| matches!(e, LspEvent::Hover { id: hid, .. } if *hid == id))
    });

    let pid_deadline = Instant::now() + Duration::from_secs(5);
    let pid = loop {
        if let Ok(contents) = fs::read_to_string(&pidfile)
            && let Ok(pid) = contents.trim().parse::<u32>()
        {
            break pid;
        }
        assert!(Instant::now() < pid_deadline, "pidfile never appeared");
        std::thread::sleep(Duration::from_millis(10));
    };

    mgr.shutdown();

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .expect("run kill -0");
        if !status.status.success() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "server process {pid} is still alive"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
