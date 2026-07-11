//! A small, transport-agnostic background-task poller: spawn work on a
//! background thread, get a task id back immediately, and drain completed
//! results once per render tick — modelled on [`crate::lsp::LspManager`]'s
//! thread + `mpsc` channel + non-blocking `poll()` design, but generalized
//! to run arbitrary closures (or commands) rather than LSP requests.
//!
//! This is a seam for the git-panel workstream (spec 02), which needs
//! non-blocking fetch/pull/push: those run as closures whose result type the
//! caller chooses, so no git-specific (or LSP-specific) types leak in here.
//! It ships with no production callers yet — hence the module-scoped
//! `dead_code` allowance below.
#![allow(dead_code)]

use std::panic::AssertUnwindSafe;
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

/// An opaque handle correlating a spawned task with its later result. Handed
/// back by [`BackgroundTasks::spawn`] immediately, before the work runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TaskId(pub u64);

/// A background task that unwound instead of returning a value. Captured and
/// delivered as a value so a panicking task never poisons the poller or the
/// render loop (mirrors the "failures are values, not panics" contract).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskPanic {
    /// The panic payload's message, best-effort.
    pub message: String,
}

/// The result of one finished command: its exit status plus captured
/// stdout/stderr. A nonzero exit — or even a failure to spawn the process —
/// is represented here as a value (`success == false`), never a panic, so
/// command tasks fit the same drain-per-tick model as any other closure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandOutcome {
    /// Whether the process exited with a success status.
    pub success: bool,
    /// The exit code, if the process exited via one (`None` if it was
    /// signalled, or never started).
    pub code: Option<i32>,
    /// Captured standard output (lossy UTF-8).
    pub stdout: String,
    /// Captured standard error (lossy UTF-8); also carries the spawn error
    /// message when the process could not be started at all.
    pub stderr: String,
}

/// Runs `command` to completion, capturing its output. A nonzero exit, or a
/// failure to even spawn the process, is returned as a [`CommandOutcome`]
/// value rather than surfaced as an error or a panic — so callers can run it
/// inside a [`BackgroundTasks::spawn`] closure and inspect the outcome when
/// it drains.
pub fn run_command(command: &mut Command) -> CommandOutcome {
    match command.output() {
        Ok(output) => CommandOutcome {
            success: output.status.success(),
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        },
        Err(e) => CommandOutcome {
            success: false,
            code: None,
            stdout: String::new(),
            stderr: e.to_string(),
        },
    }
}

/// A poller over background tasks that each produce a `T`. Spawning returns
/// immediately; [`BackgroundTasks::poll`] never blocks and yields only the
/// tasks that have finished since the last call.
pub struct BackgroundTasks<T> {
    next_id: u64,
    /// Kept alive (never dropped while tasks may still be running) so every
    /// per-task clone always has a live receiver to send to.
    tx: Sender<(TaskId, Result<T, TaskPanic>)>,
    rx: Receiver<(TaskId, Result<T, TaskPanic>)>,
}

impl<T: Send + 'static> BackgroundTasks<T> {
    /// Creates an empty poller.
    pub fn new() -> BackgroundTasks<T> {
        let (tx, rx) = mpsc::channel();
        BackgroundTasks { next_id: 0, tx, rx }
    }

    /// Spawns `f` on a background thread and returns its [`TaskId`]
    /// immediately. The closure's return value (or a captured
    /// [`TaskPanic`] if it unwinds) is delivered through a later
    /// [`BackgroundTasks::poll`]. Never blocks the caller.
    pub fn spawn<F>(&mut self, f: F) -> TaskId
    where
        F: FnOnce() -> T + Send + 'static,
    {
        let id = TaskId(self.next_id);
        self.next_id += 1;
        let tx = self.tx.clone();
        thread::spawn(move || {
            let outcome =
                std::panic::catch_unwind(AssertUnwindSafe(f)).map_err(|payload| TaskPanic {
                    message: panic_message(&payload),
                });
            // The receiver only disappears once the poller is dropped; a
            // send failure then just means nobody is listening anymore.
            let _ = tx.send((id, outcome));
        });
        id
    }

    /// Drains and returns every task that has completed since the last call,
    /// as `(id, Ok(value))` or `(id, Err(panic))`. Never blocks: tasks still
    /// running are simply not present.
    pub fn poll(&mut self) -> Vec<(TaskId, Result<T, TaskPanic>)> {
        let mut done = Vec::new();
        while let Ok(item) = self.rx.try_recv() {
            done.push(item);
        }
        done
    }
}

impl<T: Send + 'static> Default for BackgroundTasks<T> {
    fn default() -> BackgroundTasks<T> {
        BackgroundTasks::new()
    }
}

/// Best-effort extraction of a panic payload's message.
fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "background task panicked".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Polls `tasks` until it yields at least one result or the deadline
    /// passes, returning whatever drained (possibly empty on timeout).
    fn drain_one<T: Send + 'static>(
        tasks: &mut BackgroundTasks<T>,
    ) -> Vec<(TaskId, Result<T, TaskPanic>)> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let done = tasks.poll();
            if !done.is_empty() || Instant::now() >= deadline {
                return done;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn spawn_returns_distinct_ids_immediately() {
        let mut tasks: BackgroundTasks<i32> = BackgroundTasks::new();
        let a = tasks.spawn(|| 1);
        let b = tasks.spawn(|| 2);
        assert_ne!(a, b);
    }

    #[test]
    fn successful_task_drains_its_value() {
        let mut tasks: BackgroundTasks<i32> = BackgroundTasks::new();
        let id = tasks.spawn(|| 6 * 7);
        let done = drain_one(&mut tasks);
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].0, id);
        assert_eq!(done[0].1, Ok(42));
    }

    #[test]
    fn task_error_is_delivered_as_a_value_not_a_panic() {
        // A task whose *result* is an error is just an `Ok(Err(..))` value:
        // the poller reports it, it does not panic.
        let mut tasks: BackgroundTasks<Result<(), String>> = BackgroundTasks::new();
        tasks.spawn(|| Err("boom".to_string()));
        let done = drain_one(&mut tasks);
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].1, Ok(Err("boom".to_string())));
    }

    #[test]
    fn panicking_task_is_reported_as_a_task_panic_not_a_process_abort() {
        let mut tasks: BackgroundTasks<i32> = BackgroundTasks::new();
        tasks.spawn(|| panic!("kaboom"));
        let done = drain_one(&mut tasks);
        assert_eq!(done.len(), 1);
        let Err(panic) = &done[0].1 else {
            panic!("expected a TaskPanic");
        };
        assert!(panic.message.contains("kaboom"), "got {:?}", panic.message);
    }

    #[test]
    fn poll_returns_empty_while_a_task_is_still_pending() {
        // The task blocks on a gate the test controls, so it cannot complete
        // until we release it — poll must report nothing until then.
        let (gate_tx, gate_rx) = mpsc::channel::<()>();
        let mut tasks: BackgroundTasks<u8> = BackgroundTasks::new();
        tasks.spawn(move || {
            let _ = gate_rx.recv();
            9
        });

        assert!(tasks.poll().is_empty(), "task should still be pending");

        gate_tx.send(()).expect("release the gate");
        let done = drain_one(&mut tasks);
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].1, Ok(9));
    }

    #[test]
    fn run_command_reports_spawn_failure_as_a_value() {
        let outcome = run_command(&mut Command::new("redquill-no-such-binary-for-tests-xyz"));
        assert!(!outcome.success);
        assert_eq!(outcome.code, None);
        assert!(!outcome.stderr.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn run_command_reports_a_nonzero_exit_as_a_value() {
        // `false` exits nonzero without any I/O — a synthetic failing
        // command, not git or the network.
        let outcome = run_command(&mut Command::new("false"));
        assert!(!outcome.success);
        assert_eq!(outcome.code, Some(1));
    }

    #[cfg(unix)]
    #[test]
    fn run_command_inside_spawn_drains_its_outcome() {
        let mut tasks: BackgroundTasks<CommandOutcome> = BackgroundTasks::new();
        tasks.spawn(|| run_command(&mut Command::new("true")));
        let done = drain_one(&mut tasks);
        assert_eq!(done.len(), 1);
        let Ok(outcome) = &done[0].1 else {
            panic!("expected an outcome");
        };
        assert!(outcome.success);
    }
}
