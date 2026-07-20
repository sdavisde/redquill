use super::*;

/// A captured-shape fixture matching one real `gh api
/// repos/{owner}/{repo}/pulls/{n}/comments` entry, trimmed to the fields
/// this model reads plus a representative sample of the ones it ignores.
/// Grouped into a struct (rather than a long parameter list) purely to
/// dodge `clippy::too_many_arguments` — every field is required, there's no
/// meaningful default.
struct RawCommentFixture<'a> {
    id: u64,
    in_reply_to: Option<u64>,
    path: &'a str,
    side: &'a str,
    line: Option<u32>,
    login: &'a str,
    created_at: &'a str,
    body: &'a str,
}

fn raw_comment_json(f: RawCommentFixture) -> String {
    let in_reply_to_field = match f.in_reply_to {
        Some(parent) => format!(r#""in_reply_to_id": {parent},"#),
        None => String::new(),
    };
    let line_field = match f.line {
        Some(n) => n.to_string(),
        None => "null".to_string(),
    };
    let RawCommentFixture {
        id,
        path,
        side,
        login,
        created_at,
        body,
        ..
    } = f;
    format!(
        r#"{{
            "id": {id},
            "pull_request_review_id": 999,
            "diff_hunk": "@@ -1,2 +1,2 @@",
            "path": "{path}",
            "commit_id": "abc123",
            "original_commit_id": "def456",
            {in_reply_to_field}
            "user": {{"login": "{login}", "id": 1}},
            "body": "{body}",
            "created_at": "{created_at}",
            "updated_at": "{created_at}",
            "html_url": "https://github.com/o/r/pull/1#discussion_r{id}",
            "author_association": "MEMBER",
            "start_line": null,
            "original_start_line": null,
            "start_side": null,
            "line": {line_field},
            "original_line": {line_field},
            "side": "{side}"
        }}"#
    )
}

fn wrap(entries: &[String]) -> String {
    format!("[{}]", entries.join(","))
}

// -- root/reply ordering ----------------------------------------------------

#[test]
fn parse_groups_root_and_replies_into_one_thread_in_conversation_order() {
    let json = wrap(&[
        raw_comment_json(RawCommentFixture {
            id: 1,
            in_reply_to: None,
            path: "src/a.rs",
            side: "RIGHT",
            line: Some(10),
            login: "author",
            created_at: "2026-07-01T10:00:00Z",
            body: "root comment",
        }),
        raw_comment_json(RawCommentFixture {
            id: 3,
            in_reply_to: Some(1),
            path: "src/a.rs",
            side: "RIGHT",
            line: Some(10),
            login: "bob",
            created_at: "2026-07-01T10:02:00Z",
            body: "second reply",
        }),
        raw_comment_json(RawCommentFixture {
            id: 2,
            in_reply_to: Some(1),
            path: "src/a.rs",
            side: "RIGHT",
            line: Some(10),
            login: "alice",
            created_at: "2026-07-01T10:01:00Z",
            body: "first reply",
        }),
    ]);

    let threads = parse_review_comments_json(&json).unwrap();
    assert_eq!(threads.len(), 1);
    let thread = &threads[0];
    assert_eq!(thread.id, 1);
    assert_eq!(thread.root.author, "author");
    assert_eq!(thread.root.body, "root comment");
    assert_eq!(thread.replies.len(), 2);
    // Out-of-array-order replies (id 3 appears before id 2 in the JSON) are
    // reordered into conversation (timestamp) order.
    assert_eq!(thread.replies[0].body, "first reply");
    assert_eq!(thread.replies[1].body, "second reply");
}

#[test]
fn parse_reads_a_five_and_five_interleaved_back_and_forth_in_order() {
    // Two threads (roots 1 and 100), five replies each, interleaved in the
    // raw array so grouping can't rely on array order — only on
    // `in_reply_to_id` plus timestamp ordering within each group.
    let mut entries = vec![
        raw_comment_json(RawCommentFixture {
            id: 1,
            in_reply_to: None,
            path: "src/a.rs",
            side: "RIGHT",
            line: Some(5),
            login: "author",
            created_at: "2026-07-01T09:00:00Z",
            body: "thread A root",
        }),
        raw_comment_json(RawCommentFixture {
            id: 100,
            in_reply_to: None,
            path: "src/b.rs",
            side: "RIGHT",
            line: Some(20),
            login: "author",
            created_at: "2026-07-01T09:00:00Z",
            body: "thread B root",
        }),
    ];
    for i in 1..=5u64 {
        entries.push(raw_comment_json(RawCommentFixture {
            id: 1 + i,
            in_reply_to: Some(1),
            path: "src/a.rs",
            side: "RIGHT",
            line: Some(5),
            login: "reviewer",
            created_at: &format!("2026-07-01T09:0{i}:00Z"),
            body: &format!("A reply {i}"),
        }));
        entries.push(raw_comment_json(RawCommentFixture {
            id: 100 + i,
            in_reply_to: Some(100),
            path: "src/b.rs",
            side: "RIGHT",
            line: Some(20),
            login: "reviewer",
            created_at: &format!("2026-07-01T09:0{i}:00Z"),
            body: &format!("B reply {i}"),
        }));
    }

    let json = wrap(&entries);
    let threads = parse_review_comments_json(&json).unwrap();
    assert_eq!(threads.len(), 2);

    let thread_a = threads.iter().find(|t| t.id == 1).unwrap();
    assert_eq!(thread_a.root.body, "thread A root");
    assert_eq!(thread_a.replies.len(), 5);
    let a_bodies: Vec<&str> = thread_a.replies.iter().map(|c| c.body.as_str()).collect();
    assert_eq!(
        a_bodies,
        vec![
            "A reply 1",
            "A reply 2",
            "A reply 3",
            "A reply 4",
            "A reply 5"
        ]
    );

    let thread_b = threads.iter().find(|t| t.id == 100).unwrap();
    assert_eq!(thread_b.replies.len(), 5);
    let b_bodies: Vec<&str> = thread_b.replies.iter().map(|c| c.body.as_str()).collect();
    assert_eq!(
        b_bodies,
        vec![
            "B reply 1",
            "B reply 2",
            "B reply 3",
            "B reply 4",
            "B reply 5"
        ]
    );

    // Threads themselves come out in order of each root's first appearance.
    assert_eq!(threads[0].id, 1);
    assert_eq!(threads[1].id, 100);
}

// -- anchor mapping / outdated fallback --------------------------------------

#[test]
fn parse_maps_a_positioned_comment_to_a_position_anchor() {
    let json = wrap(&[raw_comment_json(RawCommentFixture {
        id: 1,
        in_reply_to: None,
        path: "src/a.rs",
        side: "RIGHT",
        line: Some(42),
        login: "author",
        created_at: "2026-07-01T10:00:00Z",
        body: "body",
    })]);
    let threads = parse_review_comments_json(&json).unwrap();
    assert_eq!(
        threads[0].anchor,
        ThreadAnchor::Position {
            path: "src/a.rs".to_string(),
            side: Side::New,
            line: 42,
        }
    );
    assert!(!threads[0].outdated);
}

#[test]
fn parse_maps_left_side_to_the_old_side() {
    let json = wrap(&[raw_comment_json(RawCommentFixture {
        id: 1,
        in_reply_to: None,
        path: "src/a.rs",
        side: "LEFT",
        line: Some(7),
        login: "author",
        created_at: "2026-07-01T10:00:00Z",
        body: "body",
    })]);
    let threads = parse_review_comments_json(&json).unwrap();
    assert_eq!(
        threads[0].anchor,
        ThreadAnchor::Position {
            path: "src/a.rs".to_string(),
            side: Side::Old,
            line: 7,
        }
    );
}

#[test]
fn parse_falls_back_to_file_level_when_the_position_is_unmappable() {
    // A `null` `line` is the documented outdated signal — the thread
    // attaches at file level rather than being dropped.
    let json = wrap(&[raw_comment_json(RawCommentFixture {
        id: 1,
        in_reply_to: None,
        path: "src/a.rs",
        side: "RIGHT",
        line: None,
        login: "author",
        created_at: "2026-07-01T10:00:00Z",
        body: "this line moved",
    })]);
    let threads = parse_review_comments_json(&json).unwrap();
    assert_eq!(threads.len(), 1);
    assert!(threads[0].outdated);
    assert_eq!(
        threads[0].anchor,
        ThreadAnchor::File {
            path: "src/a.rs".to_string()
        }
    );
}

#[test]
fn parse_outdated_fallback_applies_even_when_the_thread_has_replies() {
    let json = wrap(&[
        raw_comment_json(RawCommentFixture {
            id: 1,
            in_reply_to: None,
            path: "src/a.rs",
            side: "RIGHT",
            line: None,
            login: "author",
            created_at: "2026-07-01T10:00:00Z",
            body: "root",
        }),
        raw_comment_json(RawCommentFixture {
            id: 2,
            in_reply_to: Some(1),
            path: "src/a.rs",
            side: "RIGHT",
            line: None,
            login: "reviewer",
            created_at: "2026-07-01T10:01:00Z",
            body: "reply",
        }),
    ]);
    let threads = parse_review_comments_json(&json).unwrap();
    assert_eq!(threads.len(), 1);
    assert!(threads[0].outdated);
    assert_eq!(threads[0].replies.len(), 1);
}

// -- resolved state -----------------------------------------------------------

#[test]
fn rest_parse_alone_reports_unresolved_since_the_rest_endpoint_carries_no_such_field() {
    // The REST parser by itself never sees a resolution field, so it always
    // yields `resolved: false`; resolution is layered on afterward by
    // `apply_resolved_states` from a separate GraphQL read.
    let json = wrap(&[raw_comment_json(RawCommentFixture {
        id: 1,
        in_reply_to: None,
        path: "src/a.rs",
        side: "RIGHT",
        line: Some(1),
        login: "author",
        created_at: "2026-07-01T10:00:00Z",
        body: "body",
    })]);
    let threads = parse_review_comments_json(&json).unwrap();
    assert!(!threads[0].resolved);
}

// -- resolution overlay (GraphQL) --------------------------------------------

/// A captured-shape GraphQL `reviewThreads` payload: one resolved thread
/// (root databaseId 1) and one open thread (root databaseId 100).
const RESOLVED_GRAPHQL_FIXTURE: &str = r#"{
  "data": {
    "repository": {
      "pullRequest": {
        "reviewThreads": {
          "nodes": [
            {
              "isResolved": true,
              "isOutdated": false,
              "comments": { "nodes": [ { "databaseId": 1 } ] }
            },
            {
              "isResolved": false,
              "isOutdated": false,
              "comments": { "nodes": [ { "databaseId": 100 } ] }
            }
          ]
        }
      }
    }
  }
}"#;

#[test]
fn parse_resolved_thread_states_maps_root_id_to_is_resolved() {
    let states = parse_resolved_thread_states(RESOLVED_GRAPHQL_FIXTURE).unwrap();
    assert_eq!(states.get(&1), Some(&true));
    assert_eq!(states.get(&100), Some(&false));
    assert_eq!(states.len(), 2);
}

#[test]
fn parse_resolved_thread_states_is_total_over_an_errors_only_payload() {
    // An unauthorized/errors-only GraphQL response has no `data.repository`;
    // it must parse to an empty map (silent degradation), not an error.
    let json = r#"{"data":{"repository":null},"errors":[{"message":"Not Found"}]}"#;
    let states = parse_resolved_thread_states(json).unwrap();
    assert!(states.is_empty());
}

#[test]
fn parse_resolved_thread_states_rejects_malformed_json() {
    let err = parse_resolved_thread_states("not json").unwrap_err();
    assert!(matches!(err, ForgeError::Parse { cli: "gh", .. }));
}

#[test]
fn apply_resolved_states_flips_only_matching_threads() {
    let comments = wrap(&[
        raw_comment_json(RawCommentFixture {
            id: 1,
            in_reply_to: None,
            path: "src/a.rs",
            side: "RIGHT",
            line: Some(10),
            login: "author",
            created_at: "2026-07-01T10:00:00Z",
            body: "resolved thread",
        }),
        raw_comment_json(RawCommentFixture {
            id: 100,
            in_reply_to: None,
            path: "src/b.rs",
            side: "RIGHT",
            line: Some(20),
            login: "author",
            created_at: "2026-07-01T10:00:00Z",
            body: "open thread",
        }),
    ]);
    let mut threads = parse_review_comments_json(&comments).unwrap();
    let states = parse_resolved_thread_states(RESOLVED_GRAPHQL_FIXTURE).unwrap();
    apply_resolved_states(&mut threads, &states);

    let resolved = threads.iter().find(|t| t.id == 1).unwrap();
    let open = threads.iter().find(|t| t.id == 100).unwrap();
    assert!(resolved.resolved);
    assert!(!open.resolved);
}

#[test]
fn apply_resolved_states_leaves_a_thread_untouched_when_absent_from_the_map() {
    let comments = wrap(&[raw_comment_json(RawCommentFixture {
        id: 7,
        in_reply_to: None,
        path: "src/a.rs",
        side: "RIGHT",
        line: Some(1),
        login: "author",
        created_at: "2026-07-01T10:00:00Z",
        body: "body",
    })]);
    let mut threads = parse_review_comments_json(&comments).unwrap();
    apply_resolved_states(&mut threads, &HashMap::new());
    assert!(!threads[0].resolved);
}

// -- malformed input ----------------------------------------------------------

#[test]
fn parse_rejects_malformed_json() {
    let err = parse_review_comments_json("not json").unwrap_err();
    assert!(matches!(err, ForgeError::Parse { cli: "gh", .. }));
}

#[test]
fn parse_rejects_a_comment_missing_a_required_field() {
    let missing_body =
        r#"[{"id":1,"path":"a.rs","user":{"login":"o"},"created_at":"t","line":1,"side":"RIGHT"}]"#;
    let err = parse_review_comments_json(missing_body).unwrap_err();
    assert!(matches!(err, ForgeError::Parse { cli: "gh", .. }));
}

#[test]
fn parse_handles_an_empty_list() {
    let threads = parse_review_comments_json("[]").unwrap();
    assert!(threads.is_empty());
}

// -- reply chains that don't point straight at the root (defensive) ---------

#[test]
fn parse_resolves_a_reply_that_points_at_another_reply_up_to_its_root() {
    // Not a shape GitHub actually produces (every reply points at the
    // root), but the resolver walks the chain defensively rather than
    // treating a mid-chain reply as its own thread.
    let json = wrap(&[
        raw_comment_json(RawCommentFixture {
            id: 1,
            in_reply_to: None,
            path: "src/a.rs",
            side: "RIGHT",
            line: Some(1),
            login: "author",
            created_at: "2026-07-01T10:00:00Z",
            body: "root",
        }),
        raw_comment_json(RawCommentFixture {
            id: 2,
            in_reply_to: Some(1),
            path: "src/a.rs",
            side: "RIGHT",
            line: Some(1),
            login: "bob",
            created_at: "2026-07-01T10:01:00Z",
            body: "reply to root",
        }),
        raw_comment_json(RawCommentFixture {
            id: 3,
            in_reply_to: Some(2),
            path: "src/a.rs",
            side: "RIGHT",
            line: Some(1),
            login: "alice",
            created_at: "2026-07-01T10:02:00Z",
            body: "reply to reply",
        }),
    ]);
    let threads = parse_review_comments_json(&json).unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].id, 1);
    assert_eq!(threads[0].replies.len(), 2);
}

// -- ThreadOverlayStore -------------------------------------------------------

#[test]
fn overlay_store_starts_empty() {
    let store = ThreadOverlayStore::new();
    assert!(store.is_empty());
    assert_eq!(store.len(), 0);
}

#[test]
fn overlay_store_replace_swaps_contents_wholesale() {
    let json = wrap(&[raw_comment_json(RawCommentFixture {
        id: 1,
        in_reply_to: None,
        path: "src/a.rs",
        side: "RIGHT",
        line: Some(1),
        login: "author",
        created_at: "2026-07-01T10:00:00Z",
        body: "body",
    })]);
    let mut store = ThreadOverlayStore::new();
    store.replace(parse_review_comments_json(&json).unwrap());
    assert_eq!(store.len(), 1);

    store.replace(Vec::new());
    assert!(store.is_empty());
}

#[test]
fn overlay_store_for_path_filters_by_anchor_path() {
    let json = wrap(&[
        raw_comment_json(RawCommentFixture {
            id: 1,
            in_reply_to: None,
            path: "src/a.rs",
            side: "RIGHT",
            line: Some(1),
            login: "author",
            created_at: "2026-07-01T10:00:00Z",
            body: "a",
        }),
        raw_comment_json(RawCommentFixture {
            id: 2,
            in_reply_to: None,
            path: "src/b.rs",
            side: "RIGHT",
            line: Some(1),
            login: "author",
            created_at: "2026-07-01T10:00:00Z",
            body: "b",
        }),
    ]);
    let mut store = ThreadOverlayStore::new();
    store.replace(parse_review_comments_json(&json).unwrap());
    let a_threads: Vec<&Thread> = store.for_path("src/a.rs").collect();
    assert_eq!(a_threads.len(), 1);
    assert_eq!(a_threads[0].id, 1);
}

#[test]
fn overlay_store_find_looks_up_by_root_id() {
    let json = wrap(&[raw_comment_json(RawCommentFixture {
        id: 7,
        in_reply_to: None,
        path: "src/a.rs",
        side: "RIGHT",
        line: Some(1),
        login: "author",
        created_at: "2026-07-01T10:00:00Z",
        body: "body",
    })]);
    let mut store = ThreadOverlayStore::new();
    store.replace(parse_review_comments_json(&json).unwrap());
    assert!(store.find(7).is_some());
    assert!(store.find(999).is_none());
}

// -- markdown regression guard: fetched threads never affect stdout ---------

#[test]
fn fetched_threads_never_change_annotation_markdown_output() {
    use crate::annotate::{AnnotationStore, Classification, Target, render_markdown};

    let mut annotations = AnnotationStore::new();
    annotations
        .add(
            Target::line("src/a.rs", 10, Side::New),
            Classification::Issue,
            "please fix this",
        )
        .unwrap();
    annotations
        .add(Target::file("src/b.rs"), Classification::Praise, "nice")
        .unwrap();

    let without_threads = render_markdown(&annotations);

    // Populate an overlay store with fetched threads alongside the
    // unchanged annotation store — `render_markdown` takes only the
    // annotation store, so this has no path by which to reach the output.
    let json = wrap(&[raw_comment_json(RawCommentFixture {
        id: 1,
        in_reply_to: None,
        path: "src/a.rs",
        side: "RIGHT",
        line: Some(10),
        login: "teammate",
        created_at: "2026-07-01T10:00:00Z",
        body: "existing forge comment",
    })]);
    let mut overlay = ThreadOverlayStore::new();
    overlay.replace(parse_review_comments_json(&json).unwrap());
    assert!(!overlay.is_empty());

    let with_threads = render_markdown(&annotations);
    assert_eq!(without_threads, with_threads);
}
