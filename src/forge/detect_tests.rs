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
fn peek_is_none_until_resolved_then_returns_the_cached_value_without_re_running() {
    let host = host("git.example.com");
    let gh = FakeChecker::new(true);
    let glab = FakeChecker::new(false);
    let cache = ResolutionCache::new();

    // Before any resolve, peek must never run the ladder.
    assert_eq!(cache.peek(), None);
    assert_eq!(gh.call_count(), 0);

    let resolved = cache.get_or_resolve(&host, &gh, &glab);
    // Peek now returns the cached value and still hasn't re-run the checkers.
    assert_eq!(cache.peek(), Some(resolved));
    assert_eq!(gh.call_count(), 1);
    assert_eq!(glab.call_count(), 1);
}

#[test]
fn a_fresh_cache_instance_starts_uncached() {
    let host = host("github.com");
    let gh = FakeChecker::new(false);
    let glab = FakeChecker::new(false);
    let cache = ResolutionCache::new();
    // A known host never even touches the checkers, but this pins that a
    // brand new cache has no stale state to answer from either.
    let resolution = cache.get_or_resolve(&host, &gh, &glab);
    assert_eq!(
        resolution,
        ProviderResolution::Resolved(ProviderKind::GitHub)
    );
}

#[test]
fn glab_authenticated_custom_host_resolves_gitlab_via_the_ladder() {
    let gh = FakeChecker::new(false);
    let glab = FakeChecker::new(true);
    let resolution = resolve_provider(&host("git.client-host.example"), &gh, &glab);
    assert_eq!(
        resolution,
        ProviderResolution::Resolved(ProviderKind::GitLab)
    );
}

#[test]
fn both_clis_authenticated_for_a_custom_host_is_ambiguous_unresolved() {
    let gh = FakeChecker::new(true);
    let glab = FakeChecker::new(true);
    let resolution = resolve_provider(&host("git.client-host.example"), &gh, &glab);
    assert_eq!(
        resolution,
        ProviderResolution::Unresolved {
            hostname: "git.client-host.example".to_string(),
            reason: UnresolvedReason::Ambiguous,
        }
    );
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

#[test]
fn glab_config_get_token_command_interpolates_only_the_typed_hostname() {
    let one = glab_config_get_token_command(&host("one.example.com"));
    let two = glab_config_get_token_command(&host("two.example.com"));
    let args_one: Vec<&OsStr> = one.get_args().collect();
    let args_two: Vec<&OsStr> = two.get_args().collect();
    assert_eq!(args_one[..4], args_two[..4]);
    assert_ne!(args_one[4], args_two[4]);
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
fn success_with_non_empty_stdout_indicates_a_credential() {
    assert!(stdout_indicates_credential(&captured(
        true,
        b"glpat-fake-token-value"
    )));
}

#[test]
fn success_with_empty_stdout_indicates_no_credential() {
    assert!(!stdout_indicates_credential(&captured(true, b"")));
}

#[test]
fn success_with_only_whitespace_stdout_indicates_no_credential() {
    assert!(!stdout_indicates_credential(&captured(true, b"\n")));
}

#[test]
fn failure_with_non_empty_stdout_indicates_no_credential() {
    // A non-zero exit is treated as authoritative "no credential" even if
    // something unexpected showed up on stdout — the combined check
    // requires both signals to agree.
    assert!(!stdout_indicates_credential(&captured(
        false,
        b"some output"
    )));
}

#[test]
fn failure_with_empty_stdout_indicates_no_credential() {
    assert!(!stdout_indicates_credential(&captured(false, b"")));
}
