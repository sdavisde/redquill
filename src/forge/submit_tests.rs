use std::cell::RefCell;

use crate::annotate::{Annotation, Classification, Side, Source, Target};
use crate::forge::Verdict;

use super::super::github::build_review_payload;
use super::*;

/// One recorded call the fake executor saw, in the order it was invoked — the
/// sequencing proof reads this back to assert review-first-then-follow-ups.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    Review,
    FileComment { path: String },
    Reply { thread_id: u64 },
}

/// A recording [`ForgeSubmitExecutor`] fake: logs every call in order and
/// fails whichever specific call the test arms it to, so ordering, per-item
/// marking, and mid-sequence stop are all exercised without a network.
struct FakeExecutor {
    calls: RefCell<Vec<Call>>,
    /// Fail the review POST with this diagnostic.
    fail_review: Option<String>,
    /// Fail the file comment for this exact path.
    fail_file_path: Option<String>,
    /// Fail the reply to this exact thread id.
    fail_reply_thread: Option<u64>,
}

impl FakeExecutor {
    fn new() -> FakeExecutor {
        FakeExecutor {
            calls: RefCell::new(Vec::new()),
            fail_review: None,
            fail_file_path: None,
            fail_reply_thread: None,
        }
    }

    fn command_err(command: &str, stderr: &str) -> ForgeError {
        ForgeError::Command {
            cli: "gh",
            command: command.to_string(),
            code: "1".to_string(),
            stderr: stderr.to_string(),
        }
    }
}

impl ForgeSubmitExecutor for FakeExecutor {
    fn submit_review(&self, _payload: &ReviewPayload) -> Result<(), ForgeError> {
        self.calls.borrow_mut().push(Call::Review);
        match &self.fail_review {
            Some(stderr) => Err(FakeExecutor::command_err("pulls reviews", stderr)),
            None => Ok(()),
        }
    }

    fn post_file_comment(&self, path: &str, _body: &str) -> Result<(), ForgeError> {
        self.calls.borrow_mut().push(Call::FileComment {
            path: path.to_string(),
        });
        match &self.fail_file_path {
            Some(p) if p == path => Err(FakeExecutor::command_err("pulls comments", "file boom")),
            _ => Ok(()),
        }
    }

    fn post_reply(&self, thread_id: u64, _body: &str) -> Result<(), ForgeError> {
        self.calls.borrow_mut().push(Call::Reply { thread_id });
        match self.fail_reply_thread {
            Some(t) if t == thread_id => {
                Err(FakeExecutor::command_err("pulls replies", "reply boom"))
            }
            _ => Ok(()),
        }
    }
}

fn annotation(id: usize, target: Target, body: &str) -> Annotation {
    Annotation {
        id,
        target,
        classification: Classification::Issue,
        body: body.to_string(),
        source: Source::WorkingTree,
        published: false,
    }
}

/// A mixed batch: one line comment (id 0), one file comment (id 7), and two
/// replies — the shape every sequencing test reuses.
fn mixed_batch() -> SubmitBatch {
    let annotations = vec![
        annotation(0, Target::line("src/a.rs", 10, Side::New), "fix"),
        annotation(7, Target::file("src/d.rs"), "nice"),
    ];
    let plan = build_review_payload(&annotations, Verdict::Comment, Some("summary"));
    SubmitBatch {
        plan,
        replies: vec![
            SubmitReplyItem {
                reply_id: 3,
                thread_id: 100,
                body: "agreed".to_string(),
            },
            SubmitReplyItem {
                reply_id: 4,
                thread_id: 200,
                body: "thanks".to_string(),
            },
        ],
        include_review_post: true,
    }
}

#[test]
fn happy_path_posts_review_first_then_file_comments_then_replies_in_order() {
    let batch = mixed_batch();
    let exec = FakeExecutor::new();
    let report = run_submit_sequence(&batch, &exec);

    assert_eq!(
        exec.calls.into_inner(),
        vec![
            Call::Review,
            Call::FileComment {
                path: "src/d.rs".to_string()
            },
            Call::Reply { thread_id: 100 },
            Call::Reply { thread_id: 200 },
        ],
        "review must post before follow-ups, replies last"
    );
    assert_eq!(report.failure, None);
    assert!(report.review_submitted);
    assert_eq!(report.published_annotation_ids, vec![0, 7]);
    assert_eq!(report.published_reply_ids, vec![3, 4]);
}

#[test]
fn review_post_failure_publishes_nothing_and_stops() {
    let batch = mixed_batch();
    let exec = FakeExecutor {
        fail_review: Some("review boom".to_string()),
        ..FakeExecutor::new()
    };
    let report = run_submit_sequence(&batch, &exec);

    assert_eq!(
        exec.calls.into_inner(),
        vec![Call::Review],
        "a failed review POST must not attempt any follow-up"
    );
    assert_eq!(report.failure.as_deref(), Some("review boom"));
    assert!(!report.review_submitted);
    assert!(report.published_annotation_ids.is_empty());
    assert!(report.published_reply_ids.is_empty());
}

#[test]
fn mid_sequence_file_comment_failure_stops_and_reports_the_split() {
    let batch = mixed_batch();
    let exec = FakeExecutor {
        fail_file_path: Some("src/d.rs".to_string()),
        ..FakeExecutor::new()
    };
    let report = run_submit_sequence(&batch, &exec);

    // Review landed (its line comment published); the file comment failed, so
    // neither it nor the later replies were sent.
    assert_eq!(
        exec.calls.into_inner(),
        vec![
            Call::Review,
            Call::FileComment {
                path: "src/d.rs".to_string()
            },
        ]
    );
    assert!(report.review_submitted);
    assert_eq!(report.published_annotation_ids, vec![0]);
    assert!(report.published_reply_ids.is_empty());
    assert_eq!(report.failure.as_deref(), Some("file boom"));
}

#[test]
fn mid_sequence_reply_failure_stops_after_earlier_replies() {
    let batch = mixed_batch();
    let exec = FakeExecutor {
        fail_reply_thread: Some(200),
        ..FakeExecutor::new()
    };
    let report = run_submit_sequence(&batch, &exec);

    assert!(report.review_submitted);
    assert_eq!(report.published_annotation_ids, vec![0, 7]);
    assert_eq!(
        report.published_reply_ids,
        vec![3],
        "the earlier reply published; the failing one and anything after did not"
    );
    assert_eq!(report.failure.as_deref(), Some("reply boom"));
}

#[test]
fn resume_skips_the_review_post_and_sends_only_the_remainder() {
    // Simulate a resume after a first run whose review + line comment landed
    // but whose file comment failed: the remaining batch carries no line
    // comments (already published, so build_review_payload sees none), the
    // still-unpublished file comment, and the replies, with the review POST
    // suppressed.
    let annotations = vec![annotation(7, Target::file("src/d.rs"), "nice")];
    let plan = build_review_payload(&annotations, Verdict::Comment, Some("summary"));
    let batch = SubmitBatch {
        plan,
        replies: vec![SubmitReplyItem {
            reply_id: 4,
            thread_id: 200,
            body: "thanks".to_string(),
        }],
        include_review_post: false,
    };
    let exec = FakeExecutor::new();
    let report = run_submit_sequence(&batch, &exec);

    assert_eq!(
        exec.calls.into_inner(),
        vec![
            Call::FileComment {
                path: "src/d.rs".to_string()
            },
            Call::Reply { thread_id: 200 },
        ],
        "a resume must not re-post the review (no duplicate verdict)"
    );
    assert!(
        report.review_submitted,
        "review_submitted stays true on a resume so it can't flip back"
    );
    assert_eq!(report.published_annotation_ids, vec![7]);
    assert_eq!(report.published_reply_ids, vec![4]);
    assert_eq!(report.failure, None);
}

#[test]
fn a_verdict_only_batch_still_posts_the_review_once() {
    let plan = build_review_payload(&[], Verdict::Approve, Some("LGTM"));
    let batch = SubmitBatch {
        plan,
        replies: Vec::new(),
        include_review_post: true,
    };
    let exec = FakeExecutor::new();
    let report = run_submit_sequence(&batch, &exec);
    assert_eq!(exec.calls.into_inner(), vec![Call::Review]);
    assert!(report.review_submitted);
    assert!(report.failure.is_none());
}

#[test]
fn reply_only_batch_skips_the_empty_comment_review_post() {
    // No annotations, verdict Comment, no summary: the reviews POST would be
    // an empty-body COMMENT review that GitHub 422s. The driver must skip it
    // and go straight to the reply.
    let plan = build_review_payload(&[], Verdict::Comment, None);
    let batch = SubmitBatch {
        plan,
        replies: vec![SubmitReplyItem {
            reply_id: 9,
            thread_id: 555,
            body: "resolved, thanks".to_string(),
        }],
        include_review_post: true,
    };
    let exec = FakeExecutor::new();
    let report = run_submit_sequence(&batch, &exec);

    assert_eq!(
        exec.calls.into_inner(),
        vec![Call::Reply { thread_id: 555 }],
        "an empty COMMENT review must not be POSTed; only the reply is sent"
    );
    assert!(
        !report.review_submitted,
        "no review was posted, so a later verdict-carrying batch can still post one"
    );
    assert_eq!(report.published_reply_ids, vec![9]);
    assert!(report.published_annotation_ids.is_empty());
    assert_eq!(report.failure, None);
}

#[test]
fn verdict_only_comment_batch_with_summary_still_posts_the_review() {
    // Verdict Comment with a summary body carries content, so the review POST
    // must still happen even with no comments.
    let plan = build_review_payload(&[], Verdict::Comment, Some("overall looks good"));
    let batch = SubmitBatch {
        plan,
        replies: Vec::new(),
        include_review_post: true,
    };
    let exec = FakeExecutor::new();
    let report = run_submit_sequence(&batch, &exec);
    assert_eq!(exec.calls.into_inner(), vec![Call::Review]);
    assert!(report.review_submitted);
    assert!(report.failure.is_none());
}

#[test]
fn approve_with_no_comments_or_summary_still_posts_the_review() {
    // An empty-bodied Approve is the verdict itself — it must post.
    let plan = build_review_payload(&[], Verdict::Approve, None);
    let batch = SubmitBatch {
        plan,
        replies: Vec::new(),
        include_review_post: true,
    };
    let exec = FakeExecutor::new();
    let report = run_submit_sequence(&batch, &exec);
    assert_eq!(exec.calls.into_inner(), vec![Call::Review]);
    assert!(report.review_submitted);
    assert!(report.failure.is_none());
}

#[test]
fn reply_only_resume_after_a_failed_reply_re_sends_only_the_remainder() {
    // A reply-only batch whose first reply landed but second failed: the resume
    // rebuilds from the still-unpublished reply, skips the empty review POST
    // again, and sends only the remaining reply — no duplicate, no 422.
    let plan = build_review_payload(&[], Verdict::Comment, None);
    let batch = SubmitBatch {
        plan,
        replies: vec![SubmitReplyItem {
            reply_id: 4,
            thread_id: 200,
            body: "thanks".to_string(),
        }],
        include_review_post: true,
    };
    let exec = FakeExecutor::new();
    let report = run_submit_sequence(&batch, &exec);

    assert_eq!(
        exec.calls.into_inner(),
        vec![Call::Reply { thread_id: 200 }],
        "the resume must not post an empty review; it sends only the remaining reply"
    );
    assert_eq!(report.published_reply_ids, vec![4]);
    assert!(!report.review_submitted);
    assert_eq!(report.failure, None);
}
