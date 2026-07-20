use std::ffi::OsStr;

use super::*;

// -- mr_list_command / parse_mr_list_json ------------------------------------

/// A captured-shape fixture matching `glab mr list -F json`'s documented
/// output: GitLab's full `MergeRequest` resource per row, trimmed here to a
/// realistic subset of fields (extras present, ignored by serde) — one MR
/// using the current `draft` field, one using only the older
/// `work_in_progress` alias.
const FIXTURE_TWO_MRS: &str = r#"[
  {
    "id": 501,
    "iid": 42,
    "project_id": 7,
    "title": "Add widget support",
    "author": {
      "id": 3,
      "username": "octocat",
      "name": "The Octocat"
    },
    "source_branch": "feature/widget",
    "target_branch": "main",
    "draft": false,
    "work_in_progress": false,
    "state": "opened",
    "updated_at": "2026-07-18T12:34:56.000Z"
  },
  {
    "id": 502,
    "iid": 43,
    "project_id": 7,
    "title": "WIP: refactor gizmo",
    "author": {
      "id": 4,
      "username": "hubot",
      "name": "Hu Bot"
    },
    "source_branch": "wip/gizmo",
    "target_branch": "develop",
    "work_in_progress": true,
    "state": "opened",
    "updated_at": "2026-07-19T08:00:00.000Z"
  }
]"#;

#[test]
fn mr_list_command_has_the_fixed_argv_and_hardened_env() {
    let cmd = mr_list_command();
    assert_eq!(cmd.get_program(), OsStr::new("glab"));
    let args: Vec<&OsStr> = cmd.get_args().collect();
    assert_eq!(
        args,
        vec![
            OsStr::new("mr"),
            OsStr::new("list"),
            OsStr::new("-F"),
            OsStr::new("json"),
            OsStr::new("--per-page"),
            OsStr::new("100"),
        ]
    );
    let envs: Vec<_> = cmd.get_envs().collect();
    assert!(envs.contains(&(OsStr::new("NO_COLOR"), Some(OsStr::new("1")))));
    assert!(envs.contains(&(OsStr::new("GIT_TERMINAL_PROMPT"), Some(OsStr::new("0")))));
}

#[test]
fn parse_mr_list_json_maps_a_fixture_into_the_same_typed_rows_github_uses() {
    let rows = parse_mr_list_json(FIXTURE_TWO_MRS).unwrap();
    assert_eq!(rows.len(), 2);

    assert_eq!(rows[0].number, 42);
    assert_eq!(rows[0].title, "Add widget support");
    assert_eq!(rows[0].author, "octocat");
    assert_eq!(rows[0].head_ref, "feature/widget");
    assert_eq!(rows[0].base_ref, "main");
    assert!(!rows[0].is_draft);
    assert_eq!(rows[0].updated_at, "2026-07-18T12:34:56.000Z");

    assert_eq!(rows[1].number, 43);
    assert_eq!(rows[1].author, "hubot");
    assert_eq!(rows[1].head_ref, "wip/gizmo");
    assert_eq!(rows[1].base_ref, "develop");
    assert!(rows[1].is_draft, "work_in_progress alone must set is_draft");
}

#[test]
fn parse_mr_list_json_handles_an_empty_list() {
    let rows = parse_mr_list_json("[]").unwrap();
    assert!(rows.is_empty());
}

#[test]
fn parse_mr_list_json_rejects_malformed_json() {
    let err = parse_mr_list_json("not json").unwrap_err();
    assert!(matches!(err, ForgeError::Parse { cli: "glab", .. }));
}

#[test]
fn parse_mr_list_json_rejects_a_row_missing_a_required_field() {
    let missing_iid = r#"[{"title":"x","author":{"username":"o"},"source_branch":"h","target_branch":"b","updated_at":"t"}]"#;
    let err = parse_mr_list_json(missing_iid).unwrap_err();
    assert!(matches!(err, ForgeError::Parse { cli: "glab", .. }));
}

#[test]
fn parse_mr_list_json_defaults_is_draft_false_when_neither_field_is_present() {
    let no_draft_fields = r#"[{"iid":1,"title":"x","author":{"username":"o"},"source_branch":"h","target_branch":"b","updated_at":"t"}]"#;
    let rows = parse_mr_list_json(no_draft_fields).unwrap();
    assert!(!rows[0].is_draft);
}

// -- mr_detail_command / parse_mr_detail_json --------------------------------

const FIXTURE_MR_DETAIL: &str = r#"{
  "id": 501,
  "iid": 42,
  "project_id": 7,
  "title": "Add widget support",
  "diff_refs": {
    "base_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "head_sha": "cccccccccccccccccccccccccccccccccccccccc",
    "start_sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
  }
}"#;

#[test]
fn mr_detail_command_has_the_fixed_argv_and_hardened_env() {
    let cmd = mr_detail_command(42);
    assert_eq!(cmd.get_program(), OsStr::new("glab"));
    let args: Vec<&OsStr> = cmd.get_args().collect();
    assert_eq!(
        args,
        vec![
            OsStr::new("api"),
            OsStr::new("projects/:id/merge_requests/42"),
        ]
    );
    let envs: Vec<_> = cmd.get_envs().collect();
    assert!(envs.contains(&(OsStr::new("NO_COLOR"), Some(OsStr::new("1")))));
}

#[test]
fn mr_detail_command_interpolates_only_the_typed_iid() {
    let one = mr_detail_command(1);
    let two = mr_detail_command(2);
    let args_one: Vec<&OsStr> = one.get_args().collect();
    let args_two: Vec<&OsStr> = two.get_args().collect();
    assert_eq!(args_one[0], args_two[0]);
    assert_ne!(args_one[1], args_two[1]);
    assert_eq!(args_one[1], OsStr::new("projects/:id/merge_requests/1"));
}

#[test]
fn parse_mr_detail_json_maps_the_fixture_including_diff_refs() {
    let detail = parse_mr_detail_json(FIXTURE_MR_DETAIL).unwrap();
    assert_eq!(detail.number, 42);
    assert_eq!(
        detail.diff_refs,
        DiffRefs {
            base_sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            head_sha: "cccccccccccccccccccccccccccccccccccccccc".to_string(),
            start_sha: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
        }
    );
}

#[test]
fn parse_mr_detail_json_rejects_a_payload_missing_diff_refs() {
    let err = parse_mr_detail_json(r#"{"iid": 42}"#).unwrap_err();
    assert!(matches!(err, ForgeError::Parse { cli: "glab", .. }));
}

#[test]
fn parse_mr_detail_json_rejects_malformed_json() {
    let err = parse_mr_detail_json("not json").unwrap_err();
    assert!(matches!(err, ForgeError::Parse { cli: "glab", .. }));
}
