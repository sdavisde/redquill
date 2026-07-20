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

// -- discussions_command ------------------------------------------------------

#[test]
fn discussions_command_has_the_fixed_argv_and_hardened_env() {
    let cmd = discussions_command(42);
    assert_eq!(cmd.get_program(), OsStr::new("glab"));
    let args: Vec<&OsStr> = cmd.get_args().collect();
    assert_eq!(
        args,
        vec![
            OsStr::new("api"),
            OsStr::new("projects/:id/merge_requests/42/discussions"),
            OsStr::new("--paginate"),
        ]
    );
    let envs: Vec<_> = cmd.get_envs().collect();
    assert!(envs.contains(&(OsStr::new("NO_COLOR"), Some(OsStr::new("1")))));
}

#[test]
fn discussions_command_interpolates_only_the_typed_iid() {
    let one = discussions_command(1);
    let two = discussions_command(2);
    let args_one: Vec<&OsStr> = one.get_args().collect();
    let args_two: Vec<&OsStr> = two.get_args().collect();
    assert_ne!(args_one[1], args_two[1]);
    assert_eq!(
        args_one[1],
        OsStr::new("projects/:id/merge_requests/1/discussions")
    );
}

// -- parse_discussions_json (position -> ThreadAnchor mapping) --------------

/// One discussion covering every position shape the mapping table handles:
/// an added line (new_line only), a removed line (old_line only), a context
/// line (both present -> anchors new side), a file-type position, an
/// outdated/unmappable position (neither line present -> file fallback), a
/// general non-diff comment (`individual_note: true`, skipped), and a
/// 5-note back-and-forth to prove conversation ordering.
const FIXTURE_DISCUSSIONS: &str = r#"[
  {
    "id": "aaa111",
    "individual_note": false,
    "notes": [
      {
        "id": 1001,
        "body": "why this approach?",
        "author": { "username": "reviewer1" },
        "created_at": "2026-07-18T10:00:00.000Z",
        "resolvable": true,
        "resolved": false,
        "position": {
          "position_type": "text",
          "new_line": 42,
          "old_line": null,
          "new_path": "src/added.rs",
          "old_path": "src/added.rs"
        }
      }
    ]
  },
  {
    "id": "bbb222",
    "individual_note": false,
    "notes": [
      {
        "id": 2001,
        "body": "dead code here",
        "author": { "username": "reviewer2" },
        "created_at": "2026-07-18T10:05:00.000Z",
        "resolvable": true,
        "resolved": true,
        "position": {
          "position_type": "text",
          "new_line": null,
          "old_line": 17,
          "new_path": "src/removed.rs",
          "old_path": "src/removed.rs"
        }
      }
    ]
  },
  {
    "id": "ccc333",
    "individual_note": false,
    "notes": [
      {
        "id": 3001,
        "body": "context comment",
        "author": { "username": "reviewer3" },
        "created_at": "2026-07-18T10:10:00.000Z",
        "resolvable": true,
        "resolved": false,
        "position": {
          "position_type": "text",
          "new_line": 8,
          "old_line": 8,
          "new_path": "src/context.rs",
          "old_path": "src/context.rs"
        }
      }
    ]
  },
  {
    "id": "ddd444",
    "individual_note": false,
    "notes": [
      {
        "id": 4001,
        "body": "nice module overall",
        "author": { "username": "reviewer4" },
        "created_at": "2026-07-18T10:15:00.000Z",
        "resolvable": false,
        "resolved": false,
        "position": {
          "position_type": "file",
          "new_path": "src/whole_file.rs",
          "old_path": "src/whole_file.rs"
        }
      }
    ]
  },
  {
    "id": "eee555",
    "individual_note": false,
    "notes": [
      {
        "id": 5001,
        "body": "this line moved",
        "author": { "username": "reviewer5" },
        "created_at": "2026-07-18T10:20:00.000Z",
        "resolvable": true,
        "resolved": false,
        "position": {
          "position_type": "text",
          "new_line": null,
          "old_line": null,
          "new_path": "src/outdated.rs",
          "old_path": "src/outdated.rs"
        }
      }
    ]
  },
  {
    "id": "fff666",
    "individual_note": true,
    "notes": [
      {
        "id": 6001,
        "body": "general thanks for the PR",
        "author": { "username": "reviewer6" },
        "created_at": "2026-07-18T10:25:00.000Z",
        "resolvable": false,
        "resolved": false
      }
    ]
  },
  {
    "id": "ggg777",
    "individual_note": false,
    "notes": [
      {
        "id": 7001,
        "body": "root question",
        "author": { "username": "alice" },
        "created_at": "2026-07-18T09:00:00.000Z",
        "resolvable": true,
        "resolved": false,
        "position": {
          "position_type": "text",
          "new_line": 3,
          "old_line": null,
          "new_path": "src/thread.rs",
          "old_path": "src/thread.rs"
        }
      },
      {
        "id": 7002,
        "body": "reply one",
        "author": { "username": "bob" },
        "created_at": "2026-07-18T09:05:00.000Z",
        "resolvable": true,
        "resolved": false
      },
      {
        "id": 7003,
        "body": "reply two",
        "author": { "username": "alice" },
        "created_at": "2026-07-18T09:10:00.000Z",
        "resolvable": true,
        "resolved": false
      },
      {
        "id": 7004,
        "body": "reply three",
        "author": { "username": "bob" },
        "created_at": "2026-07-18T09:15:00.000Z",
        "resolvable": true,
        "resolved": false
      },
      {
        "id": 7005,
        "body": "reply four",
        "author": { "username": "alice" },
        "created_at": "2026-07-18T09:20:00.000Z",
        "resolvable": true,
        "resolved": true
      }
    ]
  }
]"#;

#[test]
fn added_line_position_anchors_new_side() {
    let threads = parse_discussions_json(FIXTURE_DISCUSSIONS).unwrap();
    let t = threads.iter().find(|t| t.id == 1001).unwrap();
    assert_eq!(
        t.anchor,
        ThreadAnchor::Position {
            path: "src/added.rs".to_string(),
            side: Side::New,
            line: 42,
        }
    );
    assert!(!t.outdated);
    assert!(!t.resolved);
}

#[test]
fn removed_line_position_anchors_old_side() {
    let threads = parse_discussions_json(FIXTURE_DISCUSSIONS).unwrap();
    let t = threads.iter().find(|t| t.id == 2001).unwrap();
    assert_eq!(
        t.anchor,
        ThreadAnchor::Position {
            path: "src/removed.rs".to_string(),
            side: Side::Old,
            line: 17,
        }
    );
    assert!(
        t.resolved,
        "resolved must come straight from the note field"
    );
}

#[test]
fn context_line_with_both_sides_present_anchors_new_side() {
    let threads = parse_discussions_json(FIXTURE_DISCUSSIONS).unwrap();
    let t = threads.iter().find(|t| t.id == 3001).unwrap();
    assert_eq!(
        t.anchor,
        ThreadAnchor::Position {
            path: "src/context.rs".to_string(),
            side: Side::New,
            line: 8,
        }
    );
}

#[test]
fn file_type_position_anchors_at_file_level() {
    let threads = parse_discussions_json(FIXTURE_DISCUSSIONS).unwrap();
    let t = threads.iter().find(|t| t.id == 4001).unwrap();
    assert_eq!(
        t.anchor,
        ThreadAnchor::File {
            path: "src/whole_file.rs".to_string(),
        }
    );
    // A deliberately file-scoped comment isn't "outdated" in the diff-drift
    // sense, but this model derives `outdated` from the anchor variant, same
    // as GitHub's — mirrors `github.rs`'s convention exactly.
    assert!(t.outdated);
}

#[test]
fn unmappable_position_with_neither_line_falls_back_to_file_level() {
    let threads = parse_discussions_json(FIXTURE_DISCUSSIONS).unwrap();
    let t = threads.iter().find(|t| t.id == 5001).unwrap();
    assert_eq!(
        t.anchor,
        ThreadAnchor::File {
            path: "src/outdated.rs".to_string(),
        }
    );
    assert!(t.outdated);
}

#[test]
fn general_non_diff_discussion_is_skipped() {
    let threads = parse_discussions_json(FIXTURE_DISCUSSIONS).unwrap();
    assert!(threads.iter().all(|t| t.id != 6001));
}

#[test]
fn discussion_notes_form_root_plus_ordered_replies_in_array_order() {
    let threads = parse_discussions_json(FIXTURE_DISCUSSIONS).unwrap();
    let t = threads.iter().find(|t| t.id == 7001).unwrap();
    assert_eq!(t.root.id, 7001);
    assert_eq!(t.root.author, "alice");
    assert_eq!(t.root.body, "root question");
    let reply_ids: Vec<u64> = t.replies.iter().map(|r| r.id).collect();
    assert_eq!(reply_ids, vec![7002, 7003, 7004, 7005]);
    assert_eq!(t.replies[0].author, "bob");
    assert_eq!(t.replies[3].author, "alice");
    // The discussion's resolved state comes from the root note, not any
    // reply — the last reply is resolved but the discussion (keyed off the
    // root) is not.
    assert!(!t.resolved);
}

#[test]
fn threads_are_returned_in_discussion_order() {
    let threads = parse_discussions_json(FIXTURE_DISCUSSIONS).unwrap();
    // 6 discussions total, one (fff666, general) skipped -> 5 threads, in
    // the same order the discussions array listed them.
    let ids: Vec<u64> = threads.iter().map(|t| t.id).collect();
    assert_eq!(ids, vec![1001, 2001, 3001, 4001, 5001, 7001]);
}

#[test]
fn parse_discussions_json_handles_an_empty_array() {
    let threads = parse_discussions_json("[]").unwrap();
    assert!(threads.is_empty());
}

#[test]
fn parse_discussions_json_rejects_malformed_json() {
    let err = parse_discussions_json("not json").unwrap_err();
    assert!(matches!(err, ForgeError::Parse { cli: "glab", .. }));
}

#[test]
fn a_discussion_with_no_position_at_all_is_skipped_rather_than_invented() {
    let json = r#"[{
      "id": "zzz",
      "individual_note": false,
      "notes": [{
        "id": 9001,
        "body": "no position field present",
        "author": { "username": "x" },
        "created_at": "2026-07-18T00:00:00.000Z",
        "resolvable": false,
        "resolved": false
      }]
    }]"#;
    let threads = parse_discussions_json(json).unwrap();
    assert!(threads.is_empty());
}

#[test]
fn a_discussion_with_no_notes_at_all_is_skipped_rather_than_panicking() {
    let json = r#"[{"id": "empty", "individual_note": false, "notes": []}]"#;
    let threads = parse_discussions_json(json).unwrap();
    assert!(threads.is_empty());
}
