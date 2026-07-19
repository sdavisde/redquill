use super::*;

#[test]
fn https_url_with_git_suffix_yields_the_hostname() {
    let host = parse_origin_hostname("https://github.com/org/repo.git").unwrap();
    assert_eq!(host.as_str(), "github.com");
}

#[test]
fn https_url_without_git_suffix_yields_the_hostname() {
    let host = parse_origin_hostname("https://github.com/org/repo").unwrap();
    assert_eq!(host.as_str(), "github.com");
}

#[test]
fn https_url_with_explicit_port_drops_the_port() {
    let host = parse_origin_hostname("https://example.com:8443/org/repo.git").unwrap();
    assert_eq!(host.as_str(), "example.com");
}

#[test]
fn ssh_url_with_user_yields_the_hostname() {
    let host = parse_origin_hostname("ssh://git@github.com/org/repo.git").unwrap();
    assert_eq!(host.as_str(), "github.com");
}

#[test]
fn ssh_url_with_user_and_port_yields_the_hostname() {
    let host = parse_origin_hostname("ssh://git@example.com:22/org/repo.git").unwrap();
    assert_eq!(host.as_str(), "example.com");
}

#[test]
fn ssh_url_without_user_yields_the_hostname() {
    let host = parse_origin_hostname("ssh://example.com/org/repo.git").unwrap();
    assert_eq!(host.as_str(), "example.com");
}

#[test]
fn scp_like_form_yields_the_hostname() {
    let host = parse_origin_hostname("git@github.com:org/repo.git").unwrap();
    assert_eq!(host.as_str(), "github.com");
}

#[test]
fn scp_like_form_with_multi_label_hostname_and_nested_path() {
    let host = parse_origin_hostname("git@gitlab.example.com:group/sub/repo.git").unwrap();
    assert_eq!(host.as_str(), "gitlab.example.com");
}

#[test]
fn empty_string_is_malformed() {
    let err = parse_origin_hostname("").unwrap_err();
    assert!(matches!(err, RemoteUrlError::Malformed(_)));
}

#[test]
fn plain_text_with_no_recognizable_shape_is_malformed() {
    let err = parse_origin_hostname("not a url").unwrap_err();
    assert!(matches!(err, RemoteUrlError::Malformed(_)));
}

#[test]
fn https_scheme_with_empty_authority_is_malformed() {
    let err = parse_origin_hostname("https://").unwrap_err();
    assert!(matches!(err, RemoteUrlError::Malformed(_)));
}

#[test]
fn unsupported_scheme_is_malformed() {
    let err = parse_origin_hostname("ftp://host.example.com/path").unwrap_err();
    assert!(matches!(err, RemoteUrlError::Malformed(_)));
}

#[test]
fn local_filesystem_path_is_malformed_not_scp_like() {
    let err = parse_origin_hostname("/home/user/repos/origin").unwrap_err();
    assert!(matches!(err, RemoteUrlError::Malformed(_)));
}

#[test]
fn hostname_with_disallowed_character_is_rejected() {
    let err = parse_origin_hostname("https://ho$t.example.com/org/repo").unwrap_err();
    assert!(matches!(err, RemoteUrlError::InvalidCharset(_)));
}

#[test]
fn scp_like_hostname_with_underscore_is_rejected() {
    let err = parse_origin_hostname("git@host_name.example.com:org/repo.git").unwrap_err();
    assert!(matches!(err, RemoteUrlError::InvalidCharset(_)));
}

#[test]
fn valid_hostname_charset_accepts_alphanumerics_dashes_and_dots() {
    let host = parse_origin_hostname("https://git-hub.example-01.co/org/repo.git").unwrap();
    assert_eq!(host.as_str(), "git-hub.example-01.co");
}
