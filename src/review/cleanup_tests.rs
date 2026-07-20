//! Tests for [`super::finished_reviews`]: the pure managed-branch-vs-open-PR
//! set difference driving the Pull Requests tab's finished-reviews footer and
//! cleanup flow.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use super::*;
use crate::annotate::{Classification, PersistedAnnotation, Side, Source, Target};
use crate::review::store::{ForgeMetadata, PersistedReply, PersistedReview};

fn forge_review(number: u64, title: &str) -> PersistedReview {
    PersistedReview {
        base: "main".to_string(),
        worktree_path: PathBuf::from(format!("/tmp/wt/redquill/pr/{number}")),
        files: BTreeMap::new(),
        annotations: Vec::new(),
        replies: Vec::new(),
        forge: Some(ForgeMetadata {
            provider: ForgeProviderKind::GitHub,
            host: "github.com".to_string(),
            number,
            title: title.to_string(),
            last_head_sha: "abc".to_string(),
            diff_refs: None,
        }),
    }
}

fn open(numbers: &[u64]) -> HashSet<u64> {
    numbers.iter().copied().collect()
}

#[test]
fn a_managed_review_whose_pr_is_closed_is_finished() {
    let mut reviews = BTreeMap::new();
    reviews.insert("redquill/pr/1".to_string(), forge_review(1, "old work"));
    let managed = vec!["redquill/pr/1".to_string()];

    let finished = finished_reviews(&managed, &reviews, &open(&[]));

    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0].branch, "redquill/pr/1");
    assert_eq!(finished[0].number, 1);
    assert_eq!(finished[0].title, "old work");
    assert_eq!(
        finished[0].worktree_path,
        PathBuf::from("/tmp/wt/redquill/pr/1")
    );
    assert_eq!(finished[0].unpublished_count, 0);
}

#[test]
fn a_managed_review_whose_pr_is_still_open_is_excluded() {
    let mut reviews = BTreeMap::new();
    reviews.insert("redquill/pr/2".to_string(), forge_review(2, "live"));
    let managed = vec!["redquill/pr/2".to_string()];

    let finished = finished_reviews(&managed, &reviews, &open(&[2]));

    assert!(finished.is_empty(), "an open PR's review is never finished");
}

#[test]
fn a_managed_branch_with_no_state_entry_is_excluded() {
    // A never-reviewed PR: the branch exists but nothing persisted it, so
    // there is no review to clean up.
    let reviews = BTreeMap::new();
    let managed = vec!["redquill/pr/3".to_string()];

    let finished = finished_reviews(&managed, &reviews, &open(&[]));

    assert!(finished.is_empty(), "a never-reviewed branch is excluded");
}

#[test]
fn a_review_entry_with_no_forge_block_is_excluded() {
    // A plain local-branch review that happens to be keyed under the managed
    // prefix (shouldn't occur, but the forge block is what identifies a PR
    // review) is not a finished PR review.
    let mut reviews = BTreeMap::new();
    let mut plain = forge_review(4, "x");
    plain.forge = None;
    reviews.insert("redquill/pr/4".to_string(), plain);
    let managed = vec!["redquill/pr/4".to_string()];

    let finished = finished_reviews(&managed, &reviews, &open(&[]));

    assert!(finished.is_empty());
}

#[test]
fn empty_inputs_yield_no_finished_reviews() {
    assert!(finished_reviews(&[], &BTreeMap::new(), &open(&[])).is_empty());
}

#[test]
fn open_and_closed_reviews_partition_correctly() {
    let mut reviews = BTreeMap::new();
    reviews.insert("redquill/pr/1".to_string(), forge_review(1, "closed"));
    reviews.insert("redquill/pr/2".to_string(), forge_review(2, "open"));
    reviews.insert("redquill/pr/3".to_string(), forge_review(3, "closed too"));
    let managed = vec![
        "redquill/pr/1".to_string(),
        "redquill/pr/2".to_string(),
        "redquill/pr/3".to_string(),
    ];

    let finished = finished_reviews(&managed, &reviews, &open(&[2]));

    let numbers: Vec<u64> = finished.iter().map(|f| f.number).collect();
    assert_eq!(
        numbers,
        vec![1, 3],
        "only the closed PRs' reviews are finished"
    );
}

#[test]
fn unpublished_annotations_and_replies_are_counted() {
    let mut review = forge_review(1, "work");
    review.annotations = vec![
        PersistedAnnotation {
            target: Target::line("a.rs", 1, Side::New),
            classification: Classification::Issue,
            body: "unpublished".to_string(),
            source: Source::WorkingTree,
            published: false,
            draft_created: false,
        },
        PersistedAnnotation {
            target: Target::line("a.rs", 2, Side::New),
            classification: Classification::Nit,
            body: "already sent".to_string(),
            source: Source::WorkingTree,
            published: true,
            draft_created: false,
        },
    ];
    review.replies = vec![
        PersistedReply {
            thread_id: 10,
            body: "queued".to_string(),
            published: false,
            draft_created: false,
        },
        PersistedReply {
            thread_id: 11,
            body: "sent".to_string(),
            published: true,
            draft_created: false,
        },
    ];
    let mut reviews = BTreeMap::new();
    reviews.insert("redquill/pr/1".to_string(), review);
    let managed = vec!["redquill/pr/1".to_string()];

    let finished = finished_reviews(&managed, &reviews, &open(&[]));

    assert_eq!(
        finished[0].unpublished_count, 2,
        "one unpublished annotation + one unpublished reply"
    );
}

#[test]
fn a_managed_branch_absent_from_the_managed_list_is_not_listed() {
    // The managed-branch list is the driver; a stale state entry with no
    // corresponding branch is never a cleanup candidate.
    let mut reviews = BTreeMap::new();
    reviews.insert("redquill/pr/9".to_string(), forge_review(9, "orphan"));
    let managed: Vec<String> = Vec::new();

    let finished = finished_reviews(&managed, &reviews, &open(&[]));

    assert!(finished.is_empty());
}
