//! Shared subprocess plumbing for spawning forge CLIs (`gh`, `glab`):
//! consistent environment hygiene, and a hard wall-clock timeout so a hung,
//! prompting, or slow child can never stall a caller — including the
//! background thread these calls are always meant to run from, never the
//! render loop itself.

use std::io::{self, Read};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// How often the timeout loop polls a spawned child for completion.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// The captured result of a process run to completion: exit status plus
/// whatever it wrote to stdout/stderr.
pub(crate) struct CapturedOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Environment hygiene applied to every spawned `gh`/`glab` invocation:
/// disables each CLI's interactive prompts and ANSI color, and (in case a
/// CLI itself shells out to git) disables git's own terminal prompt, matching
/// every other subprocess this crate spawns.
pub(crate) fn harden(cmd: &mut Command) {
    cmd.env("GH_PROMPT_DISABLED", "1");
    cmd.env("NO_COLOR", "1");
    cmd.env("GIT_TERMINAL_PROMPT", "0");
}

/// Runs `cmd` to completion, returning its exit status — killing (and
/// reaping) it if it hasn't exited within `timeout`. Never reads
/// stdout/stderr itself: a caller that must guarantee output never enters
/// this process's memory (e.g. a token-bearing command) sets
/// `Stdio::null()` on `cmd` before calling this.
pub(crate) fn wait_status_with_timeout(
    cmd: &mut Command,
    timeout: Duration,
) -> io::Result<ExitStatus> {
    let mut child = cmd.spawn()?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(io::ErrorKind::TimedOut, "process timed out"));
        }
        thread::sleep(POLL_INTERVAL);
    }
}

/// Runs `cmd` to completion, capturing stdout and stderr, with a hard
/// wall-clock timeout. Stdout/stderr are drained on their own threads (as
/// `crate::lsp::transport` does for a language server's pipes) so a chatty
/// child can never fill the OS pipe buffer and deadlock against this
/// thread's timeout poll; a child that outlives `timeout` is killed and its
/// output discarded.
pub(crate) fn run_captured_with_timeout(
    cmd: &mut Command,
    timeout: Duration,
) -> io::Result<CapturedOutput> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn()?;

    let stdout_handle = child.stdout.take().map(|mut pipe| {
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf);
            buf
        })
    });
    let stderr_handle = child.stderr.take().map(|mut pipe| {
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf);
            buf
        })
    });

    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(io::ErrorKind::TimedOut, "process timed out"));
        }
        thread::sleep(POLL_INTERVAL);
    };

    let stdout = stdout_handle
        .map(|h| h.join().unwrap_or_default())
        .unwrap_or_default();
    let stderr = stderr_handle
        .map(|h| h.join().unwrap_or_default())
        .unwrap_or_default();
    Ok(CapturedOutput {
        status,
        stdout,
        stderr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn harden_sets_prompt_color_and_terminal_prompt_env() {
        let mut cmd = Command::new("true");
        harden(&mut cmd);
        let envs: Vec<_> = cmd.get_envs().collect();
        assert!(envs.contains(&(OsStr::new("GH_PROMPT_DISABLED"), Some(OsStr::new("1")))));
        assert!(envs.contains(&(OsStr::new("NO_COLOR"), Some(OsStr::new("1")))));
        assert!(envs.contains(&(OsStr::new("GIT_TERMINAL_PROMPT"), Some(OsStr::new("0")))));
    }

    #[test]
    fn wait_status_with_timeout_reports_success_for_a_fast_command() {
        let mut cmd = Command::new("true");
        let status = wait_status_with_timeout(&mut cmd, Duration::from_secs(2)).unwrap();
        assert!(status.success());
    }

    #[test]
    fn wait_status_with_timeout_reports_failure_status_for_a_failing_command() {
        let mut cmd = Command::new("false");
        let status = wait_status_with_timeout(&mut cmd, Duration::from_secs(2)).unwrap();
        assert!(!status.success());
    }

    #[test]
    fn wait_status_with_timeout_kills_a_slow_command_and_returns_promptly() {
        let mut cmd = Command::new("sleep");
        cmd.arg("5");
        let start = Instant::now();
        let result = wait_status_with_timeout(&mut cmd, Duration::from_millis(100));
        assert!(result.is_err());
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "should return shortly after the timeout, not wait out the full sleep"
        );
    }

    #[test]
    fn run_captured_with_timeout_collects_stdout_and_status() {
        let mut cmd = Command::new("echo");
        cmd.arg("hello");
        let output = run_captured_with_timeout(&mut cmd, Duration::from_secs(2)).unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"hello\n");
    }

    #[test]
    fn run_captured_with_timeout_kills_a_slow_command_and_returns_promptly() {
        let mut cmd = Command::new("sleep");
        cmd.arg("5");
        let start = Instant::now();
        let result = run_captured_with_timeout(&mut cmd, Duration::from_millis(100));
        assert!(result.is_err());
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "should return shortly after the timeout, not wait out the full sleep"
        );
    }
}
