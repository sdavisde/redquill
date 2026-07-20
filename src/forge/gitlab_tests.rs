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

#[test]
fn imported_discussion_carries_its_string_id_for_reply_targeting() {
    // The discussion's string id (not the root note's u64 id) is what a reply
    // must target; import must carry it through so submit can reach it.
    let threads = parse_discussions_json(FIXTURE_DISCUSSIONS).unwrap();
    let t = threads.iter().find(|t| t.id == 7001).unwrap();
    assert_eq!(t.discussion_id.as_deref(), Some("ggg777"));
    assert_eq!(t.root.id, 7001, "the u64 id stays the root note id");
}

// -- Position-hash builder (build_note_position) ----------------------------

fn diff_refs() -> DiffRefs {
    DiffRefs {
        base_sha: "base00".to_string(),
        start_sha: "start0".to_string(),
        head_sha: "head00".to_string(),
    }
}

#[test]
fn added_new_side_line_builds_a_text_position_with_only_new_line() {
    let pos = build_note_position(
        &diff_refs(),
        &NoteTarget::Line {
            path: "src/a.rs".to_string(),
            side: Side::New,
            line: 42,
            other_line: None,
        },
    );
    assert_eq!(pos.position_type, "text");
    assert_eq!(pos.new_line, Some(42));
    assert_eq!(pos.old_line, None);
    assert_eq!(pos.new_path, "src/a.rs");
    assert_eq!(pos.old_path, "src/a.rs");
    // The MR's diff refs are pinned onto every position.
    assert_eq!(pos.base_sha, "base00");
    assert_eq!(pos.start_sha, "start0");
    assert_eq!(pos.head_sha, "head00");
}

#[test]
fn removed_old_side_line_builds_a_text_position_with_only_old_line() {
    let pos = build_note_position(
        &diff_refs(),
        &NoteTarget::Line {
            path: "src/b.rs".to_string(),
            side: Side::Old,
            line: 17,
            other_line: None,
        },
    );
    assert_eq!(pos.position_type, "text");
    assert_eq!(pos.new_line, None);
    assert_eq!(pos.old_line, Some(17));
}

#[test]
fn context_line_with_a_counterpart_builds_a_text_position_with_both_lines() {
    // GitLab 500s on a context-line position naming only one side; the
    // import fixture shows GitLab itself sending both for a context line.
    let pos = build_note_position(
        &diff_refs(),
        &NoteTarget::Line {
            path: "src/c.rs".to_string(),
            side: Side::New,
            line: 8,
            other_line: Some(6),
        },
    );
    assert_eq!(pos.position_type, "text");
    assert_eq!(pos.new_line, Some(8));
    assert_eq!(pos.old_line, Some(6));
}

#[test]
fn context_line_position_serializes_both_lines_byte_exactly() {
    let pos = build_note_position(
        &diff_refs(),
        &NoteTarget::Line {
            path: "src/c.rs".to_string(),
            side: Side::New,
            line: 8,
            other_line: Some(6),
        },
    );
    let json = serde_json::to_string(&pos).unwrap();
    assert_eq!(
        json,
        r#"{"base_sha":"base00","start_sha":"start0","head_sha":"head00","position_type":"text","new_path":"src/c.rs","old_path":"src/c.rs","new_line":8,"old_line":6}"#
    );
}

#[test]
fn old_side_context_line_with_a_counterpart_fills_new_line_from_it() {
    let pos = build_note_position(
        &diff_refs(),
        &NoteTarget::Line {
            path: "src/c.rs".to_string(),
            side: Side::Old,
            line: 6,
            other_line: Some(8),
        },
    );
    assert_eq!(pos.new_line, Some(8));
    assert_eq!(pos.old_line, Some(6));
}

#[test]
fn file_target_builds_a_file_position_with_no_lines() {
    let pos = build_note_position(
        &diff_refs(),
        &NoteTarget::File {
            path: "src/whole.rs".to_string(),
        },
    );
    assert_eq!(pos.position_type, "file");
    assert_eq!(pos.new_line, None);
    assert_eq!(pos.old_line, None);
    assert_eq!(pos.new_path, "src/whole.rs");
    assert_eq!(pos.old_path, "src/whole.rs");
}

#[test]
fn text_position_serializes_only_the_present_line_and_never_a_null() {
    let pos = build_note_position(
        &diff_refs(),
        &NoteTarget::Line {
            path: "src/a.rs".to_string(),
            side: Side::New,
            line: 5,
            other_line: None,
        },
    );
    let json = serde_json::to_string(&pos).unwrap();
    assert!(json.contains("\"new_line\":5"));
    assert!(
        !json.contains("old_line"),
        "absent side must be omitted, not null: {json}"
    );
    assert!(json.contains("\"position_type\":\"text\""));
}

// -- Submit sequence (fake executor) ----------------------------------------

/// One recorded call the fake executor saw, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    Draft {
        body: String,
        positioned: bool,
        reply_to: Option<String>,
    },
    BulkPublish,
    Discussion {
        body: String,
        positioned: bool,
    },
    DiscussionReply {
        discussion_id: String,
        body: String,
    },
    Approve,
}

/// A recording [`GitlabSubmitExecutor`] that fails whichever calls a test
/// programs it to (`fail_draft_at`: the 0-based draft index to 404 on;
/// `fail_bulk`, `fail_nth_discussion`, `fail_approve`). Never touches a network.
#[derive(Default)]
struct FakeExec {
    calls: std::cell::RefCell<Vec<Call>>,
    draft_count: std::cell::Cell<usize>,
    discussion_count: std::cell::Cell<usize>,
    fail_draft_unavailable_at: Option<usize>,
    fail_draft_error_at: Option<usize>,
    fail_bulk: bool,
    fail_discussion_at: Option<usize>,
    fail_approve: bool,
}

fn command_err(code: &str, stderr: &str) -> ForgeError {
    ForgeError::Command {
        cli: "glab",
        command: "x".to_string(),
        code: code.to_string(),
        stderr: stderr.to_string(),
    }
}

impl GitlabSubmitExecutor for FakeExec {
    fn create_draft_note(
        &self,
        body: &str,
        position: Option<&NotePosition>,
        in_reply_to_discussion_id: Option<&str>,
    ) -> Result<(), ForgeError> {
        let idx = self.draft_count.get();
        self.draft_count.set(idx + 1);
        self.calls.borrow_mut().push(Call::Draft {
            body: body.to_string(),
            positioned: position.is_some(),
            reply_to: in_reply_to_discussion_id.map(str::to_string),
        });
        if self.fail_draft_unavailable_at == Some(idx) {
            return Err(command_err("404", "404 Not Found"));
        }
        if self.fail_draft_error_at == Some(idx) {
            return Err(command_err("1", "boom"));
        }
        Ok(())
    }

    fn bulk_publish_drafts(&self) -> Result<(), ForgeError> {
        self.calls.borrow_mut().push(Call::BulkPublish);
        if self.fail_bulk {
            return Err(command_err("1", "publish failed"));
        }
        Ok(())
    }

    fn create_discussion(
        &self,
        body: &str,
        position: Option<&NotePosition>,
    ) -> Result<(), ForgeError> {
        let idx = self.discussion_count.get();
        self.discussion_count.set(idx + 1);
        self.calls.borrow_mut().push(Call::Discussion {
            body: body.to_string(),
            positioned: position.is_some(),
        });
        if self.fail_discussion_at == Some(idx) {
            return Err(command_err("1", "discussion failed"));
        }
        Ok(())
    }

    fn create_discussion_reply(&self, discussion_id: &str, body: &str) -> Result<(), ForgeError> {
        self.calls.borrow_mut().push(Call::DiscussionReply {
            discussion_id: discussion_id.to_string(),
            body: body.to_string(),
        });
        Ok(())
    }

    fn approve(&self) -> Result<(), ForgeError> {
        self.calls.borrow_mut().push(Call::Approve);
        if self.fail_approve {
            return Err(command_err("1", "approve failed"));
        }
        Ok(())
    }
}

fn note(id: usize, body: &str) -> GitlabNote {
    GitlabNote {
        annotation_id: id,
        draft_created: false,
        body: body.to_string(),
        position: build_note_position(
            &diff_refs(),
            &NoteTarget::Line {
                path: "src/a.rs".to_string(),
                side: Side::New,
                line: id as u32,
                other_line: None,
            },
        ),
    }
}

fn reply(id: usize, discussion: &str, body: &str) -> GitlabReply {
    GitlabReply {
        reply_id: id,
        discussion_id: discussion.to_string(),
        body: body.to_string(),
        draft_created: false,
    }
}

#[test]
fn draft_path_creates_all_drafts_then_bulk_publishes_then_approves_in_order() {
    let batch = GitlabSubmitBatch {
        summary: Some("looks good overall".to_string()),
        summary_draft_created: false,
        notes: vec![note(1, "fix this"), note(2, "and this")],
        replies: vec![reply(5, "ddd", "agreed")],
        approve: true,
    };
    let exec = FakeExec::default();
    let report = run_gitlab_submit_sequence(&batch, &exec);

    let calls = exec.calls.borrow().clone();
    // Summary draft, two positioned drafts, one reply draft, THEN bulk publish,
    // THEN approve — nothing published before the single bulk_publish.
    assert_eq!(
        calls,
        vec![
            Call::Draft {
                body: "looks good overall".to_string(),
                positioned: false,
                reply_to: None
            },
            Call::Draft {
                body: "fix this".to_string(),
                positioned: true,
                reply_to: None
            },
            Call::Draft {
                body: "and this".to_string(),
                positioned: true,
                reply_to: None
            },
            Call::Draft {
                body: "agreed".to_string(),
                positioned: false,
                reply_to: Some("ddd".to_string())
            },
            Call::BulkPublish,
            Call::Approve,
        ]
    );
    // On bulk-publish success every item is marked published at once.
    assert_eq!(report.published_annotation_ids, vec![1, 2]);
    assert_eq!(report.published_reply_ids, vec![5]);
    assert!(report.review_submitted);
    assert!(report.failure.is_none());
}

#[test]
fn draft_create_failure_before_publish_publishes_nothing() {
    let batch = GitlabSubmitBatch {
        summary: None,
        summary_draft_created: false,
        notes: vec![note(1, "a"), note(2, "b")],
        replies: vec![],
        approve: true,
    };
    // Second draft (index 1) fails with a non-404 error.
    let exec = FakeExec {
        fail_draft_error_at: Some(1),
        ..FakeExec::default()
    };
    let report = run_gitlab_submit_sequence(&batch, &exec);

    let calls = exec.calls.borrow().clone();
    // No bulk publish, no approve — drafts are invisible, so nothing landed.
    assert!(
        !calls
            .iter()
            .any(|c| matches!(c, Call::BulkPublish | Call::Approve))
    );
    assert!(report.published_annotation_ids.is_empty());
    assert!(report.published_reply_ids.is_empty());
    assert!(!report.review_submitted);
    assert_eq!(report.failure.as_deref(), Some("boom"));
    // The first draft was created before the stop: it exists server-side as
    // a private draft, and the report says so — a resubmit must not
    // re-create it.
    assert_eq!(report.draft_annotation_ids, vec![1]);
    assert!(report.draft_reply_ids.is_empty());
    assert!(!report.summary_draft_created);
}

#[test]
fn bulk_publish_failure_publishes_nothing() {
    let batch = GitlabSubmitBatch {
        summary: None,
        summary_draft_created: false,
        notes: vec![note(1, "a")],
        replies: vec![],
        approve: false,
    };
    let exec = FakeExec {
        fail_bulk: true,
        ..FakeExec::default()
    };
    let report = run_gitlab_submit_sequence(&batch, &exec);
    assert!(report.published_annotation_ids.is_empty());
    assert!(!report.review_submitted);
    assert_eq!(report.failure.as_deref(), Some("publish failed"));
    // Every draft was created; only the publish failed — all reported as
    // pending drafts so a retry only bulk-publishes.
    assert_eq!(report.draft_annotation_ids, vec![1]);
}

#[test]
fn resubmit_skips_already_created_drafts_and_bulk_publishes_the_whole_set() {
    // Note 1's draft survived a prior stopped run; note 2's create failed
    // then. The retry creates only note 2's draft, then one bulk publish
    // flips both — and both are reported published.
    let mut precreated = note(1, "a");
    precreated.draft_created = true;
    let batch = GitlabSubmitBatch {
        summary: None,
        summary_draft_created: false,
        notes: vec![precreated, note(2, "b")],
        replies: vec![],
        approve: false,
    };
    let exec = FakeExec::default();
    let report = run_gitlab_submit_sequence(&batch, &exec);

    let calls = exec.calls.borrow().clone();
    assert_eq!(
        calls,
        vec![
            Call::Draft {
                body: "b".to_string(),
                positioned: true,
                reply_to: None
            },
            Call::BulkPublish,
        ]
    );
    assert_eq!(report.published_annotation_ids, vec![1, 2]);
    assert!(report.draft_annotation_ids.is_empty());
    assert!(report.review_submitted);
    assert!(report.failure.is_none());
}

#[test]
fn resubmit_with_only_precreated_drafts_still_bulk_publishes() {
    let mut precreated = note(1, "a");
    precreated.draft_created = true;
    let batch = GitlabSubmitBatch {
        summary: None,
        summary_draft_created: false,
        notes: vec![precreated],
        replies: vec![],
        approve: false,
    };
    let exec = FakeExec::default();
    let report = run_gitlab_submit_sequence(&batch, &exec);
    assert_eq!(exec.calls.borrow().clone(), vec![Call::BulkPublish]);
    assert_eq!(report.published_annotation_ids, vec![1]);
    assert!(report.review_submitted);
}

#[test]
fn resubmit_skips_an_already_drafted_summary_and_reply() {
    let mut predrafted_reply = reply(5, "ddd", "agreed");
    predrafted_reply.draft_created = true;
    let batch = GitlabSubmitBatch {
        summary: Some("overall".to_string()),
        summary_draft_created: true,
        notes: vec![],
        replies: vec![predrafted_reply],
        approve: false,
    };
    let exec = FakeExec::default();
    let report = run_gitlab_submit_sequence(&batch, &exec);
    // No creates at all — both drafts exist — just the publish.
    assert_eq!(exec.calls.borrow().clone(), vec![Call::BulkPublish]);
    assert_eq!(report.published_reply_ids, vec![5]);
    assert!(report.review_submitted);
}

#[test]
fn a_stop_reports_precreated_drafts_alongside_fresh_ones() {
    // Note 1 pre-drafted, note 2 freshly drafted, note 3's create fails:
    // the report carries both existing drafts so the caller's records stay
    // complete, and nothing is published.
    let mut precreated = note(1, "a");
    precreated.draft_created = true;
    let batch = GitlabSubmitBatch {
        summary: None,
        summary_draft_created: false,
        notes: vec![precreated, note(2, "b"), note(3, "c")],
        replies: vec![],
        approve: false,
    };
    // Draft index 1 is note 3's create (note 2's is index 0).
    let exec = FakeExec {
        fail_draft_error_at: Some(1),
        ..FakeExec::default()
    };
    let report = run_gitlab_submit_sequence(&batch, &exec);
    assert!(report.published_annotation_ids.is_empty());
    assert_eq!(report.draft_annotation_ids, vec![1, 2]);
    assert_eq!(report.failure.as_deref(), Some("boom"));
}

#[test]
fn a_404_with_precreated_drafts_is_an_error_not_the_visible_fallback() {
    // Drafts already exist server-side, so the endpoint demonstrably works:
    // a 404 on the next create must not switch to visible discussions
    // (which would duplicate the pending drafts once published).
    let mut precreated = note(1, "a");
    precreated.draft_created = true;
    let batch = GitlabSubmitBatch {
        summary: None,
        summary_draft_created: false,
        notes: vec![precreated, note(2, "b")],
        replies: vec![],
        approve: false,
    };
    let exec = FakeExec {
        fail_draft_unavailable_at: Some(0),
        ..FakeExec::default()
    };
    let report = run_gitlab_submit_sequence(&batch, &exec);
    assert!(
        !exec
            .calls
            .borrow()
            .iter()
            .any(|c| matches!(c, Call::Discussion { .. })),
        "must not fall back to visible discussions"
    );
    assert!(report.failure.is_some());
    assert_eq!(report.draft_annotation_ids, vec![1]);
}

#[test]
fn missing_draft_notes_api_falls_back_to_visible_discussions_with_per_item_marking() {
    let batch = GitlabSubmitBatch {
        summary: Some("summary".to_string()),
        summary_draft_created: false,
        notes: vec![note(1, "a"), note(2, "b")],
        replies: vec![reply(9, "ddd", "reply body")],
        approve: false,
    };
    // The very first draft create 404s → fall back.
    let exec = FakeExec {
        fail_draft_unavailable_at: Some(0),
        ..FakeExec::default()
    };
    let report = run_gitlab_submit_sequence(&batch, &exec);

    let calls = exec.calls.borrow().clone();
    // One failed draft attempt, then visible discussions (summary + 2 notes),
    // then the reply — no bulk publish anywhere.
    assert!(!calls.iter().any(|c| matches!(c, Call::BulkPublish)));
    assert_eq!(
        calls,
        vec![
            Call::Draft {
                body: "summary".to_string(),
                positioned: false,
                reply_to: None
            },
            Call::Discussion {
                body: "summary".to_string(),
                positioned: false
            },
            Call::Discussion {
                body: "a".to_string(),
                positioned: true
            },
            Call::Discussion {
                body: "b".to_string(),
                positioned: true
            },
            Call::DiscussionReply {
                discussion_id: "ddd".to_string(),
                body: "reply body".to_string()
            },
        ]
    );
    // Per-item published marking (Unit-4 discipline), summary carries no id.
    assert_eq!(report.published_annotation_ids, vec![1, 2]);
    assert_eq!(report.published_reply_ids, vec![9]);
    assert!(report.review_submitted);
    assert!(report.failure.is_none());
}

#[test]
fn fallback_stops_mid_sequence_and_reports_only_what_published() {
    let batch = GitlabSubmitBatch {
        summary: None,
        summary_draft_created: false,
        notes: vec![note(1, "a"), note(2, "b"), note(3, "c")],
        replies: vec![],
        approve: false,
    };
    // Force fallback (first draft 404), then fail the 2nd visible discussion
    // (index 1 of the discussion calls).
    let exec = FakeExec {
        fail_draft_unavailable_at: Some(0),
        fail_discussion_at: Some(1),
        ..FakeExec::default()
    };
    let report = run_gitlab_submit_sequence(&batch, &exec);
    // First note published, the rest not — a resume rebuilt from the
    // unpublished set re-sends only notes 2 and 3.
    assert_eq!(report.published_annotation_ids, vec![1]);
    assert_eq!(report.failure.as_deref(), Some("discussion failed"));
}

#[test]
fn approve_is_only_sent_when_the_verdict_is_approve() {
    let batch = GitlabSubmitBatch {
        summary: None,
        summary_draft_created: false,
        notes: vec![note(1, "a")],
        replies: vec![],
        approve: false,
    };
    let exec = FakeExec::default();
    let report = run_gitlab_submit_sequence(&batch, &exec);
    assert!(
        !exec
            .calls
            .borrow()
            .iter()
            .any(|c| matches!(c, Call::Approve))
    );
    assert!(report.review_submitted);
    assert!(report.failure.is_none());
}

#[test]
fn approve_failure_after_publish_keeps_items_published_and_surfaces_the_diagnostic() {
    let batch = GitlabSubmitBatch {
        summary: None,
        summary_draft_created: false,
        notes: vec![note(1, "a")],
        replies: vec![],
        approve: true,
    };
    let exec = FakeExec {
        fail_approve: true,
        ..FakeExec::default()
    };
    let report = run_gitlab_submit_sequence(&batch, &exec);
    // The comments already published on bulk_publish; only the approve failed.
    assert_eq!(report.published_annotation_ids, vec![1]);
    assert!(report.review_submitted);
    assert_eq!(report.failure.as_deref(), Some("approve failed"));
}

// -- Submit argv builders ---------------------------------------------------

#[test]
fn draft_note_command_is_a_fixed_post_with_stdin_body() {
    let cmd = draft_note_command(42);
    assert_eq!(cmd.get_program(), OsStr::new("glab"));
    let args: Vec<&OsStr> = cmd.get_args().collect();
    assert_eq!(
        args,
        vec![
            OsStr::new("api"),
            OsStr::new("--method"),
            OsStr::new("POST"),
            OsStr::new("-H"),
            OsStr::new("Content-Type: application/json"),
            OsStr::new("projects/:id/merge_requests/42/draft_notes"),
            OsStr::new("--input"),
            OsStr::new("-"),
        ]
    );
    let envs: Vec<_> = cmd.get_envs().collect();
    assert!(envs.contains(&(OsStr::new("NO_COLOR"), Some(OsStr::new("1")))));
}

#[test]
fn bulk_publish_command_is_a_fixed_post_with_no_body() {
    let cmd = bulk_publish_command(7);
    let args: Vec<&OsStr> = cmd.get_args().collect();
    assert_eq!(
        args,
        vec![
            OsStr::new("api"),
            OsStr::new("--method"),
            OsStr::new("POST"),
            OsStr::new("-H"),
            OsStr::new("Content-Type: application/json"),
            OsStr::new("projects/:id/merge_requests/7/draft_notes/bulk_publish"),
        ]
    );
}

#[test]
fn discussion_and_reply_and_approve_commands_have_fixed_shapes() {
    let disc = discussion_create_command(3);
    let disc_args: Vec<String> = disc
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert_eq!(disc_args[5], "projects/:id/merge_requests/3/discussions");

    let reply = discussion_reply_command(3, "abc123");
    let reply_args: Vec<String> = reply
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        reply_args[5],
        "projects/:id/merge_requests/3/discussions/abc123/notes"
    );

    let approve = approve_command(3);
    let approve_args: Vec<&OsStr> = approve.get_args().collect();
    assert_eq!(
        approve_args,
        vec![OsStr::new("mr"), OsStr::new("approve"), OsStr::new("3")]
    );
}

#[test]
fn draft_and_reply_argv_interpolate_only_the_typed_iid_and_discussion_id() {
    let one = draft_note_command(1);
    let two = draft_note_command(2);
    let a: Vec<&OsStr> = one.get_args().collect();
    let b: Vec<&OsStr> = two.get_args().collect();
    assert_ne!(a[5], b[5]);
    assert_eq!(
        a[5],
        OsStr::new("projects/:id/merge_requests/1/draft_notes")
    );
}

/// Regression guard for the GitLab 415 (`glab api --input -` sends no
/// Content-Type on its own): every POST argv builder used by the submit flow
/// must carry an explicit `-H "Content-Type: application/json"`, immediately
/// before the endpoint path, so `glab` always sends a header GitLab accepts —
/// including `bulk_publish`, which has no body but still 415s without it.
#[test]
fn all_post_command_builders_send_an_explicit_json_content_type_header() {
    let commands: Vec<Command> = vec![
        draft_note_command(1),
        bulk_publish_command(1),
        discussion_create_command(1),
        discussion_reply_command(1, "abc123"),
    ];
    for cmd in commands {
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let h_pos = args
            .iter()
            .position(|a| a == "-H")
            .unwrap_or_else(|| panic!("missing -H flag in argv: {args:?}"));
        assert_eq!(
            args.get(h_pos + 1).map(String::as_str),
            Some("Content-Type: application/json"),
            "unexpected header value in argv: {args:?}"
        );
    }
}
