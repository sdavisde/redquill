//! Provider-resolution ladder: known hosts (`github.com`, `gitlab.com`)
//! resolve directly; any other host is resolved by asking each CLI's own
//! credential store whether it holds credentials for that host. Real CLI
//! invocations are hidden behind [`CredentialChecker`] so the ladder itself
//! ([`resolve_provider`]) is exercised entirely with fakes — nothing here
//! ever spawns a process in a test.
//!
//! **glab credential-lookup contract**: `gh auth token --hostname <h>`
//! (used by [`GhCredentialChecker`]) has a clean exit-status contract —
//! success means "has a credential", every other outcome (including a
//! nonzero exit) means "doesn't" — with stdout never even captured, so a
//! token can never reach this process's memory. `glab` has no confirmed
//! equivalent single-purpose command; the best local-only read is `glab
//! config get token --host <h>`, a plain config-store lookup (no network
//! call, unlike `glab auth status`, which can probe the host). This could
//! not be verified against a real `glab` (not installed on this machine),
//! so [`GlabCredentialChecker`] does not trust exit status alone: it treats
//! a host as credentialed only when the command *both* exits successfully
//! *and* prints non-empty stdout, on the theory that a config-store miss is
//! at least as likely to print empty output with a zero exit as it is to
//! fail outright. The token text itself is discarded the moment that
//! emptiness check is made ([`stdout_indicates_credential`]) — it is never
//! stored past that check, logged, or returned to any caller.

use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use super::process::{
    CapturedOutput, harden, harden_glab, run_captured_with_timeout, wait_status_with_timeout,
};
use super::remote_url::Hostname;

/// How long a real credential-lookup CLI invocation may run before it's
/// treated as "no credential" and killed. Deliberately short: this is a
/// local auth-store read, not a network round trip, and it must never be
/// allowed to stall the caller.
const CREDENTIAL_CHECK_TIMEOUT: Duration = Duration::from_secs(3);

/// One of the two forges a hostname can resolve to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    GitHub,
    GitLab,
}

/// Why a host failed to resolve to exactly one provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnresolvedReason {
    /// Neither CLI reported holding credentials for the host.
    NoCredentials,
    /// Both CLIs reported holding credentials for the host — ambiguous.
    Ambiguous,
}

/// The resolution ladder's outcome for one hostname.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderResolution {
    Resolved(ProviderKind),
    Unresolved {
        hostname: String,
        reason: UnresolvedReason,
    },
}

/// Injectable seam for "does this CLI hold credentials for this host?" so
/// resolution can be exercised without ever spawning a process. The real
/// `gh` implementation is [`GhCredentialChecker`]; `glab`'s is
/// [`GlabCredentialChecker`] — see the module doc for its lookup contract.
pub trait CredentialChecker {
    fn has_credentials(&self, hostname: &Hostname) -> bool;
}

/// Runs the resolution ladder for `hostname`: `github.com`/`gitlab.com`
/// short-circuit to their provider without consulting either checker;
/// otherwise exactly one checker reporting credentials resolves the
/// provider, and zero or both leave it [`ProviderResolution::Unresolved`].
pub fn resolve_provider(
    hostname: &Hostname,
    gh: &dyn CredentialChecker,
    glab: &dyn CredentialChecker,
) -> ProviderResolution {
    match hostname.as_str() {
        "github.com" => return ProviderResolution::Resolved(ProviderKind::GitHub),
        "gitlab.com" => return ProviderResolution::Resolved(ProviderKind::GitLab),
        _ => {}
    }

    let gh_hit = gh.has_credentials(hostname);
    let glab_hit = glab.has_credentials(hostname);
    match (gh_hit, glab_hit) {
        (true, false) => ProviderResolution::Resolved(ProviderKind::GitHub),
        (false, true) => ProviderResolution::Resolved(ProviderKind::GitLab),
        (true, true) => ProviderResolution::Unresolved {
            hostname: hostname.as_str().to_string(),
            reason: UnresolvedReason::Ambiguous,
        },
        (false, false) => ProviderResolution::Unresolved {
            hostname: hostname.as_str().to_string(),
            reason: UnresolvedReason::NoCredentials,
        },
    }
}

/// Caches one resolution for the lifetime of the value it's stored in. The
/// real application holds exactly one instance for the process's lifetime
/// (only one repo — and hence one origin hostname — is ever under review
/// per process); tests construct their own instance so runs never share
/// state with each other.
pub struct ResolutionCache(OnceLock<Mutex<Option<ProviderResolution>>>);

impl ResolutionCache {
    pub const fn new() -> Self {
        ResolutionCache(OnceLock::new())
    }

    /// Returns the cached resolution if one exists; otherwise runs the
    /// ladder once, caches the result, and returns it. Never runs the
    /// ladder twice for the same cache instance.
    pub fn get_or_resolve(
        &self,
        hostname: &Hostname,
        gh: &dyn CredentialChecker,
        glab: &dyn CredentialChecker,
    ) -> ProviderResolution {
        let lock = self.0.get_or_init(|| Mutex::new(None));
        let mut slot = match lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(resolution) = slot.as_ref() {
            return resolution.clone();
        }
        let resolution = resolve_provider(hostname, gh, glab);
        *slot = Some(resolution.clone());
        resolution
    }

    /// Returns the already-cached resolution without ever running the ladder,
    /// so a render-thread caller (the checkout dispatch) can read the provider
    /// a prior background list already resolved without risking a
    /// credential-check subprocess. `None` until a `get_or_resolve` has
    /// populated the cache.
    pub fn peek(&self) -> Option<ProviderResolution> {
        let lock = self.0.get()?;
        let slot = match lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        slot.clone()
    }
}

impl Default for ResolutionCache {
    fn default() -> Self {
        Self::new()
    }
}

/// The real `gh` credential checker: `gh auth token --hostname <h>`, exit
/// status only. Stdout/stderr are never captured (`Stdio::null()` in
/// [`gh_auth_token_command`]), so a token can never reach this process's
/// memory, let alone a log — a CLI that times out, is missing, or exits
/// non-zero all read the same as "no credential", since resolution only
/// needs the presence signal, not the failure mode.
pub struct GhCredentialChecker;

impl CredentialChecker for GhCredentialChecker {
    fn has_credentials(&self, hostname: &Hostname) -> bool {
        wait_status_with_timeout(
            &mut gh_auth_token_command(hostname),
            CREDENTIAL_CHECK_TIMEOUT,
        )
        .map(|status| status.success())
        .unwrap_or(false)
    }
}

/// Builds the fixed argv for `gh auth token --hostname <h>` with output
/// discarded and prompts disabled. Split out from
/// [`GhCredentialChecker::has_credentials`] so the exact command shape is
/// unit-testable without ever spawning it.
fn gh_auth_token_command(hostname: &Hostname) -> Command {
    let mut cmd = Command::new("gh");
    cmd.args(["auth", "token", "--hostname", hostname.as_str()]);
    harden(&mut cmd);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::null());
    cmd
}

/// The real `glab` credential checker: `glab config get token --host <h>`,
/// exit status *and* stdout emptiness combined — see the module doc for why
/// exit status alone isn't trusted here the way it is for `gh`. The
/// captured stdout bytes are inspected only for emptiness
/// ([`stdout_indicates_credential`]) and dropped immediately after; no
/// token content is ever stored, logged, or returned.
pub struct GlabCredentialChecker;

impl CredentialChecker for GlabCredentialChecker {
    fn has_credentials(&self, hostname: &Hostname) -> bool {
        run_captured_with_timeout(
            &mut glab_config_get_token_command(hostname),
            CREDENTIAL_CHECK_TIMEOUT,
        )
        .map(|output| stdout_indicates_credential(&output))
        .unwrap_or(false)
    }
}

/// Builds the fixed argv for `glab config get token --host <h>`, with
/// prompts disabled. Split out from
/// [`GlabCredentialChecker::has_credentials`] so the exact command shape is
/// unit-testable without ever spawning it, mirroring
/// [`gh_auth_token_command`].
fn glab_config_get_token_command(hostname: &Hostname) -> Command {
    let mut cmd = Command::new("glab");
    cmd.args(["config", "get", "token", "--host", hostname.as_str()]);
    harden_glab(&mut cmd);
    cmd
}

/// The credential-presence decision described in the module doc: success
/// *and* non-empty stdout. Checking emptiness over raw bytes (never
/// materialized as a `String` beyond this) is the last place the captured
/// output is touched — the caller drops `output` immediately after this
/// call returns, so no token content outlives this function.
fn stdout_indicates_credential(output: &CapturedOutput) -> bool {
    output.status.success() && !output.stdout.iter().all(u8::is_ascii_whitespace)
}

#[cfg(test)]
#[path = "detect_tests.rs"]
mod tests;
