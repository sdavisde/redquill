//! Shared subprocess plumbing for spawning forge CLIs (`gh`, `glab`):
//! consistent environment hygiene, and a hard wall-clock timeout so a hung,
//! prompting, or slow child can never stall a caller — including the
//! background thread these calls are always meant to run from, never the
//! render loop itself.

use std::io;
use std::process::{Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};

/// How often the timeout loop polls a spawned child for completion.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

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
}
