//! Provider-resolution ladder: known hosts (`github.com`, `gitlab.com`)
//! resolve directly; any other host is resolved by asking each CLI's own
//! credential store whether it holds credentials for that host. Real CLI
//! invocations are hidden behind [`CredentialChecker`] so the ladder itself
//! ([`resolve_provider`]) is exercised entirely with fakes — nothing here
//! ever spawns a process in a test.

use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use super::process::{harden, wait_status_with_timeout};
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
/// `gh` implementation is [`GhCredentialChecker`]; `glab`'s
/// ([`GlabCredentialChecker`]) is a placeholder until its lookup command is
/// finalized against the pinned glab version.
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

/// Placeholder `glab` credential checker: always reports no credentials
/// until the real lookup command is finalized against the pinned glab
/// version. An unknown host with only glab credentials therefore resolves
/// `Unresolved` rather than `GitLab` until then — acceptable for this
/// GitHub-only slice.
pub struct GlabCredentialChecker;

impl CredentialChecker for GlabCredentialChecker {
    fn has_credentials(&self, _hostname: &Hostname) -> bool {
        false
    }
}

#[cfg(test)]
#[path = "detect_tests.rs"]
mod tests;
