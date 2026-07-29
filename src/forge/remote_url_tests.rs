use super::*;

/// Expected outcome for one `parse_origin_hostname` case.
enum HostExpect {
    Host(&'static str),
    Malformed,
    InvalidCharset,
}

#[test]
fn parse_origin_hostname_covers_every_recognized_and_rejected_url_shape() {
    use HostExpect::*;
    let cases: &[(&str, HostExpect)] = &[
        // https, with and without the .git suffix, and with a port to drop.
        ("https://github.com/org/repo.git", Host("github.com")),
        ("https://github.com/org/repo", Host("github.com")),
        ("https://example.com:8443/org/repo.git", Host("example.com")),
        // ssh, with and without a user, with and without a port.
        ("ssh://git@github.com/org/repo.git", Host("github.com")),
        ("ssh://git@example.com:22/org/repo.git", Host("example.com")),
        ("ssh://example.com/org/repo.git", Host("example.com")),
        // scp-like, flat and multi-label host with a nested path.
        ("git@github.com:org/repo.git", Host("github.com")),
        (
            "git@gitlab.example.com:group/sub/repo.git",
            Host("gitlab.example.com"),
        ),
        // The full accepted charset: alphanumerics, dashes, dots.
        (
            "https://git-hub.example-01.co/org/repo.git",
            Host("git-hub.example-01.co"),
        ),
        // Nothing to parse a hostname out of at all.
        ("", Malformed),
        ("not a url", Malformed),
        ("https://", Malformed),
        ("ftp://host.example.com/path", Malformed),
        // A local filesystem path is malformed, never read as scp-like.
        ("/home/user/repos/origin", Malformed),
        // A hostname was found but carries a disallowed character.
        ("https://ho$t.example.com/org/repo", InvalidCharset),
        ("git@host_name.example.com:org/repo.git", InvalidCharset),
    ];

    for (url, expected) in cases {
        let got = parse_origin_hostname(url);
        match expected {
            Host(want) => match got {
                Ok(host) => assert_eq!(host.as_str(), *want, "{url}"),
                Err(err) => panic!("{url}: expected host {want}, got error {err}"),
            },
            Malformed => assert!(
                matches!(got, Err(RemoteUrlError::Malformed(_))),
                "{url}: expected Malformed, got {got:?}"
            ),
            InvalidCharset => assert!(
                matches!(got, Err(RemoteUrlError::InvalidCharset(_))),
                "{url}: expected InvalidCharset, got {got:?}"
            ),
        }
    }
}

#[test]
fn parse_origin_repo_slug_covers_every_recognized_and_rejected_url_shape() {
    let cases: &[(&str, Option<&str>)] = &[
        ("https://github.com/org/repo.git", Some("org/repo")),
        ("https://github.com/org/repo", Some("org/repo")),
        (
            "ssh://git@example.com:22/group/sub/repo.git",
            Some("group/sub/repo"),
        ),
        ("git@github.com:org/repo.git", Some("org/repo")),
        (
            "git@gitlab.example.com:group/sub/repo.git",
            Some("group/sub/repo"),
        ),
        // No path at all, and the malformed shapes, yield no slug.
        ("https://github.com", None),
        ("not a url", None),
        ("", None),
        ("ftp://host.example.com/org/repo", None),
    ];

    for (url, expected) in cases {
        assert_eq!(parse_origin_repo_slug(url).as_deref(), *expected, "{url}");
    }
}
