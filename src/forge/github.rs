//! GitHub provider: `gh` argv construction and JSON parsing for the PR
//! listing. `gh pr list` infers the repository from the current working
//! directory exactly as `git` itself does elsewhere in this codebase, so no
//! repo argument is ever built from user input — the argv is entirely
//! fixed.

use std::process::Command;
use std::time::Duration;

use serde::Deserialize;

use super::process::{harden, run_captured_with_timeout};
use super::{ForgeError, PullRequest};

/// The exact `--json` field list `gh pr list` is asked for — fixed at the
/// listing's field set, never composed from caller input.
pub const PR_LIST_JSON_FIELDS: &str = "number,title,author,headRefName,isDraft,updatedAt";

/// How long a `gh pr list` invocation may run before it's treated as
/// failed and killed. Generous relative to the credential-check timeout
/// since this is a real network round trip, not a local auth-store read.
const LIST_TIMEOUT: Duration = Duration::from_secs(15);

/// Builds the fixed argv for `gh pr list --json <fields>`, with prompts
/// disabled and color stripped from the (JSON, machine-read) output.
pub fn pr_list_command() -> Command {
    let mut cmd = Command::new("gh");
    cmd.args(["pr", "list", "--json", PR_LIST_JSON_FIELDS]);
    harden(&mut cmd);
    cmd
}

/// Runs `gh pr list` and returns the typed rows. The only function here
/// that actually spawns a process — kept thin and deliberately untested;
/// [`parse_pr_list_json`] carries the fixture coverage, since exercising
/// this would require a real `gh` on PATH.
pub fn list_open_prs() -> Result<Vec<PullRequest>, ForgeError> {
    let mut cmd = pr_list_command();
    let output = run_captured_with_timeout(&mut cmd, LIST_TIMEOUT).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            ForgeError::CliNotFound { cli: "gh" }
        } else {
            ForgeError::Spawn { cli: "gh", source }
        }
    })?;

    if !output.status.success() {
        return Err(ForgeError::Command {
            cli: "gh",
            command: "pr list".to_string(),
            code: output
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".to_string()),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }

    let json = String::from_utf8_lossy(&output.stdout);
    parse_pr_list_json(&json)
}

/// The raw shape of one entry in `gh pr list --json ...`'s JSON array.
#[derive(Debug, Deserialize)]
struct RawPr {
    number: u64,
    title: String,
    author: RawAuthor,
    #[serde(rename = "headRefName")]
    head_ref_name: String,
    #[serde(rename = "isDraft")]
    is_draft: bool,
    #[serde(rename = "updatedAt")]
    updated_at: String,
}

#[derive(Debug, Deserialize)]
struct RawAuthor {
    login: String,
}

/// Parses `gh pr list --json ...`'s stdout into typed rows. Pure — no
/// process involved — so it's exercised entirely by fixture tests built
/// from captured-shape `gh` output.
pub fn parse_pr_list_json(json: &str) -> Result<Vec<PullRequest>, ForgeError> {
    let raw: Vec<RawPr> = serde_json::from_str(json).map_err(|e| ForgeError::Parse {
        cli: "gh",
        message: e.to_string(),
    })?;
    Ok(raw
        .into_iter()
        .map(|r| PullRequest {
            number: r.number,
            title: r.title,
            author: r.author.login,
            head_ref: r.head_ref_name,
            is_draft: r.is_draft,
            updated_at: r.updated_at,
        })
        .collect())
}

#[cfg(test)]
#[path = "github_tests.rs"]
mod tests;
