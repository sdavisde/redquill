use super::*;

// -- error_headline ---------------------------------------------------------

#[test]
fn command_error_headline_is_the_first_stderr_line() {
    let e = ForgeError::Command {
        cli: "gh",
        command: "pr list".to_string(),
        code: "1".to_string(),
        stderr: "not logged in\nrun `gh auth login`".to_string(),
    };
    assert_eq!(error_headline(&e), "not logged in");
}

#[test]
fn command_error_with_empty_stderr_falls_back_to_display() {
    let e = ForgeError::Command {
        cli: "gh",
        command: "pr list".to_string(),
        code: "1".to_string(),
        stderr: String::new(),
    };
    assert_eq!(error_headline(&e), e.to_string());
}

#[test]
fn non_command_error_headline_is_the_display_string() {
    let e = ForgeError::CliNotFound { cli: "gh" };
    assert_eq!(error_headline(&e), e.to_string());
}

// -- submit_error_headline: 401/403 hints -----------------------------------

fn command_err(cli: &'static str, code: &str, stderr: &str) -> ForgeError {
    ForgeError::Command {
        cli,
        command: "api".to_string(),
        code: code.to_string(),
        stderr: stderr.to_string(),
    }
}

#[test]
fn glab_403_gets_the_scope_hint() {
    let e = command_err("glab", "403", "glab: HTTP 403");
    assert_eq!(
        submit_error_headline(&e),
        "glab: HTTP 403 (write blocked — token may lack the 'api' scope \
(read_api is read-only); re-auth with 'glab auth login' or check your \
project role)"
    );
}

#[test]
fn gh_403_gets_the_token_scope_hint() {
    let e = command_err("gh", "403", "gh: HTTP 403");
    assert_eq!(
        submit_error_headline(&e),
        "gh: HTTP 403 (write blocked — check token scopes with 'gh auth status' \
and your repo permission)"
    );
}

#[test]
fn glab_401_gets_the_not_authenticated_hint() {
    let e = command_err("glab", "401", "glab: HTTP 401");
    assert_eq!(
        submit_error_headline(&e),
        "glab: HTTP 401 (not authenticated — run 'glab auth login')"
    );
}

#[test]
fn gh_401_gets_the_not_authenticated_hint() {
    let e = command_err("gh", "401", "gh: HTTP 401");
    assert_eq!(
        submit_error_headline(&e),
        "gh: HTTP 401 (not authenticated — run 'gh auth login')"
    );
}

#[test]
fn forbidden_in_stderr_is_recognized_without_an_http_line() {
    let e = command_err("glab", "1", "POST failed: Forbidden");
    assert_eq!(
        submit_error_headline(&e),
        "POST failed: Forbidden (write blocked — token may lack the 'api' scope \
(read_api is read-only); re-auth with 'glab auth login' or check your \
project role)"
    );
}

#[test]
fn unauthorized_in_stderr_is_recognized_without_an_http_line() {
    let e = command_err("gh", "1", "request failed: unauthorized");
    assert_eq!(
        submit_error_headline(&e),
        "request failed: unauthorized (not authenticated — run 'gh auth login')"
    );
}

#[test]
fn a_bare_403_digit_run_in_stderr_is_not_treated_as_http_403() {
    // "403" appears but not as "HTTP 403" or "forbidden" — must not match.
    let e = command_err("glab", "1", "422 error: field 403 is invalid");
    assert_eq!(submit_error_headline(&e), "422 error: field 403 is invalid");
}

#[test]
fn plain_command_error_gets_no_hint() {
    let e = command_err("gh", "1", "network error: connection reset");
    assert_eq!(submit_error_headline(&e), "network error: connection reset");
}

#[test]
fn a_404_gets_no_hint() {
    // 404 drives GitLab's draft-notes-unavailable fallback and must stay a
    // plain headline.
    let e = command_err("glab", "404", "glab: HTTP 404");
    assert_eq!(submit_error_headline(&e), "glab: HTTP 404");
}

#[test]
fn non_command_error_gets_no_hint() {
    let e = ForgeError::CliNotFound { cli: "glab" };
    assert_eq!(submit_error_headline(&e), e.to_string());
}
