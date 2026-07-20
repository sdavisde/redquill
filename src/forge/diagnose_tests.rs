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
