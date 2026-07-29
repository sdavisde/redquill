use std::cell::Cell;
use std::ffi::OsStr;

use super::*;
use crate::forge::remote_url::parse_origin_hostname;

/// A [`CredentialChecker`] whose answer and call count are both fixed by
/// the test, so the ladder's process-consultation behavior (which checkers
/// get asked, and how many times) is directly observable.
struct FakeChecker {
    hit: bool,
    calls: Cell<u32>,
}

impl FakeChecker {
    fn new(hit: bool) -> Self {
        FakeChecker {
            hit,
            calls: Cell::new(0),
        }
    }

    fn call_count(&self) -> u32 {
        self.calls.get()
    }
}

impl CredentialChecker for FakeChecker {
    fn has_credentials(&self, _hostname: &Hostname) -> bool {
        self.calls.set(self.calls.get() + 1);
        self.hit
    }
}

fn host(s: &str) -> Hostname {
    parse_origin_hostname(&format!("https://{s}/org/repo.git"))
        .expect("test fixture hostname must parse")
}

#[test]
fn github_dot_com_resolves_without_consulting_either_checker() {
    let gh = FakeChecker::new(true);
    let glab = FakeChecker::new(true);
    let resolution = resolve_provider(&host("github.com"), &gh, &glab);
    assert_eq!(
        resolution,
        ProviderResolution::Resolved(ProviderKind::GitHub)
    );
    assert_eq!(gh.call_count(), 0);
    assert_eq!(glab.call_count(), 0);
}

#[test]
fn gitlab_dot_com_resolves_without_consulting_either_checker() {
    let gh = FakeChecker::new(true);
    let glab = FakeChecker::new(true);
    let resolution = resolve_provider(&host("gitlab.com"), &gh, &glab);
    assert_eq!(
        resolution,
        ProviderResolution::Resolved(ProviderKind::GitLab)
    );
    assert_eq!(gh.call_count(), 0);
    assert_eq!(glab.call_count(), 0);
}

#[test]
fn unknown_host_with_only_a_gh_credential_hit_resolves_github() {
    let gh = FakeChecker::new(true);
    let glab = FakeChecker::new(false);
    let resolution = resolve_provider(&host("git.example.com"), &gh, &glab);
    assert_eq!(
        resolution,
        ProviderResolution::Resolved(ProviderKind::GitHub)
    );
}

#[test]
fn unknown_host_with_only_a_glab_credential_hit_resolves_gitlab() {
    let gh = FakeChecker::new(false);
    let glab = FakeChecker::new(true);
    let resolution = resolve_provider(&host("git.example.com"), &gh, &glab);
    assert_eq!(
        resolution,
        ProviderResolution::Resolved(ProviderKind::GitLab)
    );
}

#[test]
fn unknown_host_with_no_credential_hit_is_unresolved() {
    let gh = FakeChecker::new(false);
    let glab = FakeChecker::new(false);
    let resolution = resolve_provider(&host("git.example.com"), &gh, &glab);
    assert_eq!(
        resolution,
        ProviderResolution::Unresolved {
            hostname: "git.example.com".to_string(),
            reason: UnresolvedReason::NoCredentials,
        }
    );
}

#[test]
fn unknown_host_with_both_credential_hits_is_unresolved_ambiguous() {
    let gh = FakeChecker::new(true);
    let glab = FakeChecker::new(true);
    let resolution = resolve_provider(&host("git.example.com"), &gh, &glab);
    assert_eq!(
        resolution,
        ProviderResolution::Unresolved {
            hostname: "git.example.com".to_string(),
            reason: UnresolvedReason::Ambiguous,
        }
    );
}

#[test]
fn resolution_cache_runs_the_ladder_at_most_once() {
    let host = host("git.example.com");
    let gh = FakeChecker::new(true);
    let glab = FakeChecker::new(false);
    let cache = ResolutionCache::new();

    let first = cache.get_or_resolve(&host, &gh, &glab);
    let second = cache.get_or_resolve(&host, &gh, &glab);

    assert_eq!(first, ProviderResolution::Resolved(ProviderKind::GitHub));
    assert_eq!(second, first);
    assert_eq!(gh.call_count(), 1);
    assert_eq!(glab.call_count(), 1);
}

#[test]
fn gh_auth_token_command_has_the_fixed_argv_and_hardened_env() {
    let h = host("git.example.com");
    let cmd = gh_auth_token_command(&h);
    assert_eq!(cmd.get_program(), OsStr::new("gh"));
    let args: Vec<&OsStr> = cmd.get_args().collect();
    assert_eq!(
        args,
        vec![
            OsStr::new("auth"),
            OsStr::new("token"),
            OsStr::new("--hostname"),
            OsStr::new("git.example.com"),
        ]
    );
    let envs: Vec<_> = cmd.get_envs().collect();
    assert!(envs.contains(&(OsStr::new("GH_PROMPT_DISABLED"), Some(OsStr::new("1")))));
    assert!(envs.contains(&(OsStr::new("NO_COLOR"), Some(OsStr::new("1")))));
    assert!(envs.contains(&(OsStr::new("GIT_TERMINAL_PROMPT"), Some(OsStr::new("0")))));
}

#[test]
fn glab_config_get_token_command_has_the_fixed_argv_and_hardened_env() {
    let h = host("git.example.com");
    let cmd = glab_config_get_token_command(&h);
    assert_eq!(cmd.get_program(), OsStr::new("glab"));
    let args: Vec<&OsStr> = cmd.get_args().collect();
    assert_eq!(
        args,
        vec![
            OsStr::new("config"),
            OsStr::new("get"),
            OsStr::new("token"),
            OsStr::new("--host"),
            OsStr::new("git.example.com"),
        ]
    );
    let envs: Vec<_> = cmd.get_envs().collect();
    assert!(envs.contains(&(OsStr::new("NO_COLOR"), Some(OsStr::new("1")))));
    assert!(envs.contains(&(OsStr::new("GIT_TERMINAL_PROMPT"), Some(OsStr::new("0")))));
}

// -- stdout_indicates_credential (glab's combined status+emptiness contract) -

/// Builds a real `ExitStatus` without ever spawning `glab` — `true`/`false`
/// are universally present system binaries, the same trick `process.rs`'s
/// own tests use, so this stays deterministic across machines regardless of
/// whether `glab` is installed.
fn status(success: bool) -> std::process::ExitStatus {
    let bin = if success { "true" } else { "false" };
    std::process::Command::new(bin)
        .status()
        .expect("spawning a universally present system binary must succeed")
}

fn captured(success: bool, stdout: &[u8]) -> CapturedOutput {
    CapturedOutput {
        status: status(success),
        stdout: stdout.to_vec(),
        stderr: Vec::new(),
    }
}

#[test]
fn a_credential_requires_both_a_zero_exit_and_non_blank_stdout() {
    // A non-zero exit is authoritative "no credential" even when something
    // unexpected showed up on stdout — both signals must agree.
    let cases: &[(bool, &[u8], bool)] = &[
        (true, b"glpat-fake-token-value", true),
        (true, b"", false),
        (true, b"\n", false),
        (false, b"some output", false),
        (false, b"", false),
    ];
    for (success, stdout, expected) in cases {
        assert_eq!(
            stdout_indicates_credential(&captured(*success, stdout)),
            *expected,
            "exit success={success}, stdout={:?}",
            String::from_utf8_lossy(stdout)
        );
    }
}
