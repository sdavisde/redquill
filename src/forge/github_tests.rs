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

/// The commit page is reached by the *positional* sha, not `--commit=<sha>`:
/// `gh browse --commit=<sha>` resolves to `/tree/<sha>` (the repository tree
/// as of that commit) while the bare positional resolves to `/commit/<sha>`
/// (the commit's own diff). Verified against `gh` 2.x with `gh browse -n`;
/// pinned here because the two spellings differ by one character and fail
/// silently — landing the reviewer on a plausible-looking wrong page.
#[test]
fn browse_argv_is_read_only_and_puts_the_sha_positionally() {
    let branch = branch_web_command("feature/thing");
    assert_eq!(branch.get_program(), OsStr::new("gh"));
    let args: Vec<&OsStr> = branch.get_args().collect();
    assert_eq!(
        args,
        vec![
            OsStr::new("browse"),
            OsStr::new("--branch"),
            OsStr::new("feature/thing"),
        ]
    );

    let commit = commit_web_command("abc123");
    let args: Vec<&OsStr> = commit.get_args().collect();
    assert_eq!(args, vec![OsStr::new("browse"), OsStr::new("abc123")]);
    assert!(
        !args
            .iter()
            .any(|a| a.to_string_lossy().starts_with("--commit")),
        "--commit=<sha> would open /tree/<sha>, not the commit's own diff"
    );
}

#[test]
fn pr_web_command_is_a_read_only_view_argv_carrying_only_the_typed_number() {
    let cmd = pr_web_command(42);
    assert_eq!(cmd.get_program(), OsStr::new("gh"));
    let args: Vec<&OsStr> = cmd.get_args().collect();
    assert_eq!(
        args,
        vec![
            OsStr::new("pr"),
            OsStr::new("view"),
            OsStr::new("42"),
            OsStr::new("--web"),
        ]
    );
    let envs: Vec<_> = cmd.get_envs().collect();
    assert!(envs.contains(&(OsStr::new("GH_PROMPT_DISABLED"), Some(OsStr::new("1")))));
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
            OsStr::new("--paginate"),
        ]
    );
    let envs: Vec<_> = cmd.get_envs().collect();
    assert!(envs.contains(&(OsStr::new("GH_PROMPT_DISABLED"), Some(OsStr::new("1")))));
    assert!(envs.contains(&(OsStr::new("NO_COLOR"), Some(OsStr::new("1")))));
}

// -- review_threads_resolved_command (GraphQL resolution overlay) ------------

#[test]
fn review_threads_resolved_command_has_the_fixed_argv_and_hardened_env() {
    let cmd = review_threads_resolved_command("octocat", "redquill", 42);
    assert_eq!(cmd.get_program(), OsStr::new("gh"));
    let args: Vec<String> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert_eq!(args[0], "api");
    assert_eq!(args[1], "graphql");
    // owner/name/number ride as typed GraphQL variable fields, never spliced
    // into the query text.
    assert!(args.contains(&"owner=octocat".to_string()));
    assert!(args.contains(&"name=redquill".to_string()));
    assert!(args.contains(&"number=42".to_string()));
    // The query itself is a fixed constant carrying only `$owner`/`$name`/
    // `$number` placeholders — nothing from the caller is interpolated in.
    assert!(args.iter().any(|a| a.starts_with("query=")));
    assert!(
        args.iter()
            .all(|a| !a.contains("octocat") || a == "owner=octocat")
    );
    let envs: Vec<_> = cmd.get_envs().collect();
    assert!(envs.contains(&(OsStr::new("GH_PROMPT_DISABLED"), Some(OsStr::new("1")))));
    assert!(envs.contains(&(OsStr::new("NO_COLOR"), Some(OsStr::new("1")))));
}

// -- build_review_payload (submit-flow payload construction) ----------------

use crate::annotate::Source;

fn annotation(id: usize, target: Target, classification: Classification, body: &str) -> Annotation {
    Annotation {
        id,
        target,
        classification,
        body: body.to_string(),
        source: Source::WorkingTree,
        published: false,
        draft_created: false,
    }
}

#[test]
fn line_target_maps_to_a_single_line_comment_with_no_start_fields() {
    let annotations = vec![
        annotation(
            0,
            Target::line("src/a.rs", 10, Side::New),
            Classification::Issue,
            "fix this",
        ),
        annotation(
            1,
            Target::line("src/a.rs", 9, Side::Old),
            Classification::Nit,
            "dead code",
        ),
    ];
    let plan = build_review_payload(&annotations, Verdict::Comment, None);
    assert_eq!(
        plan.payload.comments[0],
        ReviewCommentPayload {
            path: "src/a.rs".to_string(),
            body: "[issue] fix this".to_string(),
            line: 10,
            side: "RIGHT",
            start_line: None,
            start_side: None,
        }
    );
    assert_eq!(plan.payload.comments[1].side, "LEFT");
    assert_eq!(plan.payload.comments[1].start_line, None);
    assert!(plan.file_comment_follow_ups.is_empty());
}

#[test]
fn range_target_on_old_side_carries_matching_start_and_end_side() {
    let annotations = vec![annotation(
        0,
        Target::range("src/b.rs", 5, 8, Side::Old).unwrap(),
        Classification::Question,
        "why?",
    )];
    let plan = build_review_payload(&annotations, Verdict::Comment, None);
    assert_eq!(
        plan.payload.comments,
        vec![ReviewCommentPayload {
            path: "src/b.rs".to_string(),
            body: "[question] why?".to_string(),
            line: 8,
            side: "LEFT",
            start_line: Some(5),
            start_side: Some("LEFT"),
        }]
    );
}

#[test]
fn hunk_target_always_maps_as_a_new_side_range_regardless_of_diff_content() {
    let annotations = vec![annotation(
        0,
        Target::hunk("src/c.rs", 1, 3).unwrap(),
        Classification::Nit,
        "tidy",
    )];
    let plan = build_review_payload(&annotations, Verdict::Comment, None);
    assert_eq!(
        plan.payload.comments,
        vec![ReviewCommentPayload {
            path: "src/c.rs".to_string(),
            body: "[nit] tidy".to_string(),
            line: 3,
            side: "RIGHT",
            start_line: Some(1),
            start_side: Some("RIGHT"),
        }]
    );
}

#[test]
fn one_line_range_collapses_to_a_single_line_comment_without_start_fields() {
    // A Range spanning one line (start == end) must not carry start_line ==
    // line: GitHub 422s a multi-line comment whose start_line is not strictly
    // below line, so a one-line span emits the plain single-line shape.
    let annotations = vec![annotation(
        0,
        Target::range("src/b.rs", 8, 8, Side::New).unwrap(),
        Classification::Question,
        "why?",
    )];
    let plan = build_review_payload(&annotations, Verdict::Comment, None);
    assert_eq!(
        plan.payload.comments,
        vec![ReviewCommentPayload {
            path: "src/b.rs".to_string(),
            body: "[question] why?".to_string(),
            line: 8,
            side: "RIGHT",
            start_line: None,
            start_side: None,
        }]
    );
}

#[test]
fn carries_content_is_false_only_for_an_empty_comment_review() {
    // Empty-body COMMENT with no comments: nothing to publish, would 422.
    let empty = build_review_payload(&[], Verdict::Comment, None);
    assert!(!empty.payload.carries_content());

    // A summary body makes a COMMENT review worth posting.
    let with_body = build_review_payload(&[], Verdict::Comment, Some("looks good"));
    assert!(with_body.payload.carries_content());

    // A comment makes a COMMENT review worth posting.
    let with_comment = build_review_payload(
        &[annotation(
            0,
            Target::line("src/a.rs", 3, Side::New),
            Classification::Issue,
            "fix",
        )],
        Verdict::Comment,
        None,
    );
    assert!(with_comment.payload.carries_content());

    // Approve and request-changes are themselves the payload.
    assert!(
        build_review_payload(&[], Verdict::Approve, None)
            .payload
            .carries_content()
    );
    assert!(
        build_review_payload(&[], Verdict::RequestChanges, Some("please fix"))
            .payload
            .carries_content()
    );
}

#[test]
fn file_target_is_excluded_from_comments_and_routed_to_follow_ups() {
    let annotations = vec![annotation(
        7,
        Target::file("src/d.rs"),
        Classification::Praise,
        "nice module",
    )];
    let plan = build_review_payload(&annotations, Verdict::Comment, None);
    assert!(plan.payload.comments.is_empty());
    assert_eq!(
        plan.file_comment_follow_ups,
        vec![FileCommentFollowUp {
            annotation_id: 7,
            path: "src/d.rs".to_string(),
            body: "[praise] nice module".to_string(),
        }]
    );
}

#[test]
fn worktree_targets_are_excluded_entirely_as_local_only() {
    let annotations = vec![
        annotation(
            0,
            Target::worktree_line("docs/notes.md", 2),
            Classification::Question,
            "stale?",
        ),
        annotation(
            1,
            Target::worktree_range("docs/notes.md", 5, 6).unwrap(),
            Classification::Nit,
            "stale range",
        ),
    ];
    let plan = build_review_payload(&annotations, Verdict::Comment, None);
    assert!(
        plan.payload.comments.is_empty(),
        "worktree targets must never enter the comments array"
    );
    assert!(
        plan.file_comment_follow_ups.is_empty(),
        "worktree targets must never enter the follow-up set either"
    );
}

#[test]
fn every_classification_gets_its_bracketed_prefix_on_the_first_line_only() {
    for (classification, tag) in [
        (Classification::Issue, "issue"),
        (Classification::Question, "question"),
        (Classification::Nit, "nit"),
        (Classification::Praise, "praise"),
    ] {
        let annotations = vec![annotation(
            0,
            Target::file("a.rs"),
            classification,
            "first line\nsecond line",
        )];
        let plan = build_review_payload(&annotations, Verdict::Comment, None);
        assert_eq!(
            plan.file_comment_follow_ups[0].body,
            format!("[{tag}] first line\nsecond line")
        );
    }
}

#[test]
fn every_verdict_maps_to_its_github_event_string() {
    for (verdict, event) in [
        (Verdict::Comment, "COMMENT"),
        (Verdict::Approve, "APPROVE"),
        (Verdict::RequestChanges, "REQUEST_CHANGES"),
    ] {
        let plan = build_review_payload(&[], verdict, None);
        assert_eq!(plan.payload.event, event);
    }
}

/// The exhaustive mixed batch: line, range on the old side, hunk, file, and
/// both worktree variants, one of each classification, an approve verdict
/// and a summary — asserting the exact serialized JSON body the reviews
/// endpoint receives, byte for byte.
#[test]
fn mixed_batch_serializes_to_the_exact_reviews_endpoint_json() {
    let annotations = vec![
        annotation(
            0,
            Target::line("src/a.rs", 10, Side::New),
            Classification::Issue,
            "fix this",
        ),
        annotation(
            1,
            Target::range("src/b.rs", 5, 8, Side::Old).unwrap(),
            Classification::Question,
            "why?",
        ),
        annotation(
            2,
            Target::hunk("src/c.rs", 1, 3).unwrap(),
            Classification::Nit,
            "tidy",
        ),
        annotation(
            3,
            Target::file("src/d.rs"),
            Classification::Praise,
            "nice module",
        ),
        annotation(
            4,
            Target::worktree_line("docs/notes.md", 2),
            Classification::Question,
            "stale?",
        ),
        annotation(
            5,
            Target::worktree_range("docs/notes.md", 5, 6).unwrap(),
            Classification::Nit,
            "stale range",
        ),
    ];

    let plan = build_review_payload(&annotations, Verdict::Approve, Some("Looks solid overall"));

    let json = serde_json::to_string_pretty(&plan.payload).unwrap();
    let expected = r#"{
  "body": "Looks solid overall",
  "event": "APPROVE",
  "comments": [
    {
      "path": "src/a.rs",
      "body": "[issue] fix this",
      "line": 10,
      "side": "RIGHT"
    },
    {
      "path": "src/b.rs",
      "body": "[question] why?",
      "line": 8,
      "side": "LEFT",
      "start_line": 5,
      "start_side": "LEFT"
    },
    {
      "path": "src/c.rs",
      "body": "[nit] tidy",
      "line": 3,
      "side": "RIGHT",
      "start_line": 1,
      "start_side": "RIGHT"
    }
  ]
}"#;
    assert_eq!(json, expected);

    assert_eq!(
        plan.file_comment_follow_ups,
        vec![FileCommentFollowUp {
            annotation_id: 3,
            path: "src/d.rs".to_string(),
            body: "[praise] nice module".to_string(),
        }],
        "the file-target annotation must route to the follow-up set, not the comments array"
    );

    // The parallel id list attributes each comments-array entry back to its
    // annotation, in input order, so a successful review POST marks exactly
    // those published; the file target's id (3) is not in it.
    assert_eq!(plan.comment_annotation_ids, vec![0, 1, 2]);
}

// -- Submit-sequence argv builders (the three write endpoints) --------------

#[test]
fn every_write_endpoint_posts_a_fixed_argv_with_stdin_input_and_hardened_env() {
    let cases = [
        (
            submit_review_command(42),
            "repos/{owner}/{repo}/pulls/42/reviews",
        ),
        (
            file_comment_command(7),
            "repos/{owner}/{repo}/pulls/7/comments",
        ),
        (
            reply_command(42, 9988),
            "repos/{owner}/{repo}/pulls/42/comments/9988/replies",
        ),
    ];
    for (cmd, endpoint) in cases {
        assert_eq!(cmd.get_program(), OsStr::new("gh"), "{endpoint}");
        let args: Vec<&OsStr> = cmd.get_args().collect();
        assert_eq!(
            args,
            vec![
                OsStr::new("api"),
                OsStr::new("--method"),
                OsStr::new("POST"),
                OsStr::new(endpoint),
                OsStr::new("--input"),
                OsStr::new("-"),
            ],
            "{endpoint}"
        );
        let envs: Vec<_> = cmd.get_envs().collect();
        assert!(
            envs.contains(&(OsStr::new("GH_PROMPT_DISABLED"), Some(OsStr::new("1")))),
            "{endpoint}"
        );
        assert!(
            envs.contains(&(OsStr::new("NO_COLOR"), Some(OsStr::new("1")))),
            "{endpoint}"
        );
    }
}
