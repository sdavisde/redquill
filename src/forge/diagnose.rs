//! Turns a raw [`ForgeError`] into the one line a status message or tab body
//! shows a reviewer. [`error_headline`] is the shared "first non-empty
//! stderr line, else the error's own `Display`" extraction every call site
//! that surfaces a forge failure used to duplicate locally.

use super::ForgeError;

/// The one-line diagnostic a [`ForgeError`] contributes to a stopped run: a
/// `Command` error's first non-empty stderr line (CLI stderr is often
/// multi-line; one actionable line is what the status line shows), or the
/// error's own `Display` otherwise.
pub(crate) fn error_headline(e: &ForgeError) -> String {
    match e {
        ForgeError::Command { stderr, .. } if !stderr.trim().is_empty() => stderr
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or(stderr)
            .trim()
            .to_string(),
        other => other.to_string(),
    }
}

/// [`error_headline`], with an actionable next step appended in parens when
/// the failure is HTTP-401/403-shaped — a raw "glab: HTTP 403" tells a
/// reviewer something broke but not what to do about it, and a scoped
/// review token (`read_api` only) failing to publish is the case that
/// prompted this. Every other status, including 404 (which drives GitLab's
/// draft-notes-unavailable fallback in `gitlab.rs`), is left untouched.
pub(crate) fn submit_error_headline(e: &ForgeError) -> String {
    let headline = error_headline(e);
    match e {
        ForgeError::Command {
            cli, code, stderr, ..
        } => match auth_hint(cli, code, stderr) {
            Some(hint) => format!("{headline} ({hint})"),
            None => headline,
        },
        _ => headline,
    }
}

/// The next-step hint for an auth-shaped `Command` failure, or `None` for
/// every other status. Detection is deliberately conservative: an exact
/// status code, or an unambiguous "HTTP <code>"/"forbidden"/"unauthorized"
/// marker in stderr — never a bare digit run, which risks matching an
/// unrelated "403" appearing inside some other message.
fn auth_hint(cli: &'static str, code: &str, stderr: &str) -> Option<String> {
    let lower = stderr.to_ascii_lowercase();
    let is_403 = code == "403" || stderr.contains("HTTP 403") || lower.contains("forbidden");
    let is_401 = code == "401" || stderr.contains("HTTP 401") || lower.contains("unauthorized");

    if is_403 {
        Some(match cli {
            "glab" => "write blocked — token may lack the 'api' scope (read_api is \
read-only); re-auth with 'glab auth login' or check your project role"
                .to_string(),
            _ => "write blocked — check token scopes with 'gh auth status' and your \
repo permission"
                .to_string(),
        })
    } else if is_401 {
        Some(format!("not authenticated — run '{cli} auth login'"))
    } else {
        None
    }
}

#[cfg(test)]
#[path = "diagnose_tests.rs"]
mod tests;
