//! Pure parser: `origin`'s remote URL to a validated hostname. No process is
//! ever spawned here — this only ever sees a string the caller already read
//! (e.g. via `crate::git::GitRunner::origin_url`).
//!
//! Three URL shapes are recognized: `https://host[:port]/path`,
//! `ssh://[user@]host[:port]/path`, and the scp-like `user@host:path` form
//! git itself accepts with no scheme at all. Anything else — including an
//! unrecognized scheme — is [`RemoteUrlError::Malformed`] rather than a
//! best-effort guess. Once a hostname is extracted it is validated against
//! a strict charset allowlist (alphanumerics, `-`, `.`); anything else is
//! [`RemoteUrlError::InvalidCharset`]. Never panics on malformed input.

use thiserror::Error;

/// A hostname that has passed [`parse_origin_hostname`]'s charset
/// validation. Carrying this type (rather than a bare `String`) downstream
/// is what makes "hostnames are validated before they ever reach argv"
/// structural rather than a matter of caller discipline.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Hostname(String);

impl Hostname {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Hostname {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<Hostname> for String {
    fn from(host: Hostname) -> String {
        host.0
    }
}

/// Errors from parsing a remote URL down to a hostname.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RemoteUrlError {
    /// The URL didn't match any of the recognized `https://`/`ssh://`/
    /// scp-like shapes closely enough to extract a hostname at all.
    #[error("could not parse a hostname from remote URL: {0}")]
    Malformed(String),
    /// A hostname was extracted, but it contains characters outside the
    /// allowlist (alphanumerics, `-`, `.`).
    #[error("hostname contains characters outside the allowed set: {0}")]
    InvalidCharset(String),
}

/// Extracts and validates the hostname from an `origin` remote URL.
pub fn parse_origin_hostname(url: &str) -> Result<Hostname, RemoteUrlError> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err(RemoteUrlError::Malformed(url.to_string()));
    }

    let host = if let Some(rest) = trimmed.strip_prefix("https://") {
        extract_authority_host(rest)
    } else if let Some(rest) = trimmed.strip_prefix("ssh://") {
        extract_authority_host(rest)
    } else if trimmed.contains("://") {
        // A scheme we don't recognize (e.g. `ftp://`, `git://`) — treated
        // as malformed rather than guessed at, per the recognized-shapes
        // list this parser commits to.
        None
    } else {
        extract_scp_like_host(trimmed)
    };

    let host = host.ok_or_else(|| RemoteUrlError::Malformed(url.to_string()))?;
    validate_hostname_charset(&host)?;
    Ok(Hostname(host))
}

/// Pulls the host out of what follows `scheme://`: strips an optional
/// `user@` prefix, then takes everything up to the first `/` or `:`
/// (port), whichever comes first.
fn extract_authority_host(rest: &str) -> Option<String> {
    let end = rest.find('/').unwrap_or(rest.len());
    let authority = &rest[..end];
    let host_part = match authority.rfind('@') {
        Some(idx) => &authority[idx + 1..],
        None => authority,
    };
    let host = host_part.split(':').next().unwrap_or("");
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

/// Pulls the host out of a scheme-less scp-like remote (`user@host:path`,
/// e.g. `git@github.com:org/repo.git`). Requires both the `@` and the `:`
/// after it, so an ordinary local filesystem path (no `@`) never matches.
fn extract_scp_like_host(s: &str) -> Option<String> {
    if s.starts_with('/') {
        return None;
    }
    let at_idx = s.find('@')?;
    let rest = &s[at_idx + 1..];
    let colon_idx = rest.find(':')?;
    let host = &rest[..colon_idx];
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

/// The strict hostname charset allowlist: alphanumerics, `-`, `.` — nothing
/// else is ever accepted, so a hostname can never smuggle a shell
/// metacharacter or extra argv element into a CLI invocation built from it.
fn validate_hostname_charset(host: &str) -> Result<(), RemoteUrlError> {
    let ok = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
    if ok {
        Ok(())
    } else {
        Err(RemoteUrlError::InvalidCharset(host.to_string()))
    }
}

#[cfg(test)]
#[path = "remote_url_tests.rs"]
mod tests;
