use std::ffi::OsStr;

use super::*;

/// A captured-shape fixture matching `gh pr list --json
/// number,title,author,headRefName,isDraft,updatedAt`'s real output: two
/// open PRs, one of them a draft.
const FIXTURE_TWO_PRS: &str = r#"[
  {
    "number": 42,
    "title": "Add widget support",
    "author": {
      "id": "MDQ6VXNlcjE=",
      "is_bot": false,
      "login": "octocat",
      "name": "The Octocat"
    },
    "headRefName": "feature/widget",
    "baseRefName": "main",
    "isDraft": false,
    "updatedAt": "2026-07-18T12:34:56Z"
  },
  {
    "number": 43,
    "title": "WIP: refactor gizmo",
    "author": {
      "id": "MDQ6VXNlcjI=",
      "is_bot": false,
      "login": "hubot",
      "name": "Hu Bot"
    },
    "headRefName": "wip/gizmo",
    "baseRefName": "develop",
    "isDraft": true,
    "updatedAt": "2026-07-19T08:00:00Z"
  }
]"#;

#[test]
fn pr_list_command_has_the_fixed_argv_and_hardened_env() {
    let cmd = pr_list_command();
    assert_eq!(cmd.get_program(), OsStr::new("gh"));
    let args: Vec<&OsStr> = cmd.get_args().collect();
    assert_eq!(
        args,
        vec![
            OsStr::new("pr"),
            OsStr::new("list"),
            OsStr::new("--json"),
            OsStr::new(PR_LIST_JSON_FIELDS),
        ]
    );
    let envs: Vec<_> = cmd.get_envs().collect();
    assert!(envs.contains(&(OsStr::new("GH_PROMPT_DISABLED"), Some(OsStr::new("1")))));
    assert!(envs.contains(&(OsStr::new("NO_COLOR"), Some(OsStr::new("1")))));
}

#[test]
fn pr_list_json_fields_are_exactly_fr4s_set() {
    assert_eq!(
        PR_LIST_JSON_FIELDS,
        "number,title,author,headRefName,baseRefName,isDraft,updatedAt"
    );
}

#[test]
fn parse_pr_list_json_maps_a_fixture_into_typed_rows() {
    let rows = parse_pr_list_json(FIXTURE_TWO_PRS).unwrap();
    assert_eq!(rows.len(), 2);

    assert_eq!(rows[0].number, 42);
    assert_eq!(rows[0].title, "Add widget support");
    assert_eq!(rows[0].author, "octocat");
    assert_eq!(rows[0].head_ref, "feature/widget");
    assert_eq!(rows[0].base_ref, "main");
    assert!(!rows[0].is_draft);
    assert_eq!(rows[0].updated_at, "2026-07-18T12:34:56Z");

    assert_eq!(rows[1].number, 43);
    assert_eq!(rows[1].author, "hubot");
    assert_eq!(rows[1].head_ref, "wip/gizmo");
    assert_eq!(rows[1].base_ref, "develop");
    assert!(rows[1].is_draft);
}

#[test]
fn parse_pr_list_json_handles_an_empty_list() {
    let rows = parse_pr_list_json("[]").unwrap();
    assert!(rows.is_empty());
}

#[test]
fn parse_pr_list_json_rejects_malformed_json() {
    let err = parse_pr_list_json("not json").unwrap_err();
    assert!(matches!(err, ForgeError::Parse { cli: "gh", .. }));
}

#[test]
fn parse_pr_list_json_rejects_a_row_missing_a_required_field() {
    let missing_number = r#"[{"title":"x","author":{"login":"o"},"headRefName":"h","baseRefName":"b","isDraft":false,"updatedAt":"t"}]"#;
    let err = parse_pr_list_json(missing_number).unwrap_err();
    assert!(matches!(err, ForgeError::Parse { cli: "gh", .. }));
}

// -- review_comments_command -------------------------------------------------

#[test]
fn review_comments_command_has_the_fixed_argv_and_hardened_env() {
    let cmd = review_comments_command(42);
    assert_eq!(cmd.get_program(), OsStr::new("gh"));
    let args: Vec<&OsStr> = cmd.get_args().collect();
    assert_eq!(
        args,
        vec![
            OsStr::new("api"),
            OsStr::new("repos/{owner}/{repo}/pulls/42/comments"),
        ]
    );
    let envs: Vec<_> = cmd.get_envs().collect();
    assert!(envs.contains(&(OsStr::new("GH_PROMPT_DISABLED"), Some(OsStr::new("1")))));
    assert!(envs.contains(&(OsStr::new("NO_COLOR"), Some(OsStr::new("1")))));
}

#[test]
fn review_comments_command_interpolates_only_the_typed_pr_number() {
    // The `{owner}`/`{repo}` placeholders are literal text `gh` itself
    // substitutes — never assembled from any caller-provided string — so
    // the only thing that varies with input is the number.
    let cmd_one = review_comments_command(1);
    let cmd_two = review_comments_command(2);
    let args_one: Vec<&OsStr> = cmd_one.get_args().collect();
    let args_two: Vec<&OsStr> = cmd_two.get_args().collect();
    assert_eq!(args_one[0], args_two[0]);
    assert_ne!(args_one[1], args_two[1]);
    assert!(
        args_one[1]
            .to_str()
            .unwrap()
            .starts_with("repos/{owner}/{repo}/pulls/1/comments")
    );
}
