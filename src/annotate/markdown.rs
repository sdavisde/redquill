//! Markdown serialization of an [`AnnotationStore`].
//!
//! [`render_markdown`] is the format emitted on quit to stdout by the UI.
//! Treat it as a public API once shipped:
//!
//! ```text
//! ## src/auth/session.rs:44 (+)
//!
//! [question] where does keystore get rotated?
//! ```
//!
//! ## The `Reviewing:` metadata line (non-working-tree sources)
//!
//! Annotations authored against the default working-tree source
//! ([`crate::annotate::model::Source::WorkingTree`]) are always emitted
//! first, in exactly the format above, with **no** metadata line — a
//! session that never leaves the working-tree view is byte-identical to the
//! format before this contract existed.
//!
//! Annotations authored against any other source are grouped by that source
//! (in order of first appearance, working-tree group excluded since it's
//! always first) and each group is preceded by exactly one metadata line of
//! the form `Reviewing: <spec>`, where `<spec>` is:
//!
//! - a commit: the short SHA (e.g. `Reviewing: abc1234`)
//! - a range: the range expression exactly as typed/selected (e.g.
//!   `Reviewing: main..feature`)
//! - the index: the literal word `staged` (e.g. `Reviewing: staged`)
//!
//! Example mixed session (one working-tree annotation, then one against a
//! historical commit):
//!
//! ```text
//! ## src/lib.rs:10-20 (+)
//!
//! [nit] extract this into a helper
//!
//! Reviewing: abc1234
//!
//! ## src/auth/session.rs:44 (+)
//!
//! [question] where does keystore get rotated?
//! ```
//!
//! ## The `(=)` marker (current file content, not a diff side)
//!
//! Annotations made in the read-only whole-file view (spec 06 Unit 3 — any
//! file opened via the project search or fuzzy file finder, not just files
//! with a diff) target [`crate::annotate::model::Target::WorktreeLine`] or
//! [`crate::annotate::model::Target::WorktreeRange`] instead of
//! [`crate::annotate::model::Target::Line`]/[`crate::annotate::model::Target::Range`],
//! and serialize with a third marker, `(=)`, meaning "this line/range is the
//! current worktree file content, not a diff side" — there is no `+`/`-` to
//! report because the file view shows no diff at all:
//!
//! ```text
//! ## docs/notes.md:44 (=)
//!
//! [question] should this doc mention the new flag?
//! ```
//!
//! The file view always reads live worktree content (never a historical
//! revision), so a `(=)` annotation always composes with the working-tree
//! group above: it is emitted in the same always-first, metadata-line-free
//! group as ordinary working-tree diff annotations, never its own
//! `Reviewing:` group. A `Target::File` (whole-file) comment made from the
//! file view is unaffected by this section — it already has no side marker
//! at all, diffed or not.

use super::model::{Annotation, Classification, Side, Source, Target};
use super::store::AnnotationStore;

fn side_marker(side: Side) -> &'static str {
    match side {
        Side::New => " (+)",
        Side::Old => " (-)",
    }
}

fn header(target: &Target) -> String {
    match target {
        Target::Line { path, line, side } => format!("## {path}:{line}{}", side_marker(*side)),
        Target::Range {
            path,
            start,
            end,
            side,
        } => format!("## {path}:{start}-{end}{}", side_marker(*side)),
        Target::Hunk { path, start, end } => {
            format!("## {path}:{start}-{end}{}", side_marker(Side::New))
        }
        Target::File { path } => format!("## {path}"),
        Target::WorktreeLine { path, line } => format!("## {path}:{line} (=)"),
        Target::WorktreeRange { path, start, end } => format!("## {path}:{start}-{end} (=)"),
    }
}

fn classification_tag(classification: Classification) -> String {
    format!("[{}] ", classification.label())
}

fn render_one(annotation: &Annotation) -> String {
    let mut out = header(&annotation.target);
    out.push_str("\n\n");
    let mut lines = annotation.body.lines();
    if let Some(first) = lines.next() {
        out.push_str(&classification_tag(annotation.classification));
        out.push_str(first);
    }
    for line in lines {
        out.push('\n');
        out.push_str(line);
    }
    out
}

/// Partitions `store`'s annotations into groups by [`Source`], in order of
/// first appearance, with annotations keeping their relative insertion order
/// within a group. The working-tree group (if present) is then moved to the
/// front regardless of when it first appeared, since the format contract
/// requires it to be emitted first.
fn group_by_source(store: &AnnotationStore) -> Vec<(&Source, Vec<&Annotation>)> {
    let mut groups: Vec<(&Source, Vec<&Annotation>)> = Vec::new();
    for annotation in store.iter() {
        match groups
            .iter_mut()
            .find(|(source, _)| **source == annotation.source)
        {
            Some((_, members)) => members.push(annotation),
            None => groups.push((&annotation.source, vec![annotation])),
        }
    }
    if let Some(pos) = groups
        .iter()
        .position(|(source, _)| **source == Source::WorkingTree)
    {
        let working_tree_group = groups.remove(pos);
        groups.insert(0, working_tree_group);
    }
    groups
}

/// Renders one [`Source`] group: the `Reviewing:` metadata line — omitted
/// entirely for [`Source::WorkingTree`], per the byte-identical-by-default
/// contract — followed by that group's annotations in the existing
/// per-annotation format, separated by one blank line each.
fn render_group(source: &Source, annotations: &[&Annotation]) -> String {
    let bodies: Vec<String> = annotations.iter().map(|a| render_one(a)).collect();
    let body = bodies.join("\n\n");
    match source {
        Source::WorkingTree => body,
        _ => format!("Reviewing: {}\n\n{body}", source.label()),
    }
}

/// Renders every annotation in `store` as the public-contract markdown
/// format. An empty store renders to an empty string; otherwise the output
/// ends with a single trailing newline.
///
/// Annotations authored against the default working-tree source are
/// rendered first, in the existing per-annotation format with no metadata
/// line — byte-identical to the format before source-grouping existed.
/// Annotations authored against any other source are grouped by that source
/// (see [`group_by_source`]) and each group is preceded by exactly one
/// `Reviewing: <spec>` line; see the module doc for the exact `<spec>` per
/// source kind.
pub fn render_markdown(store: &AnnotationStore) -> String {
    let groups = group_by_source(store);
    let blocks: Vec<String> = groups
        .iter()
        .map(|(source, annotations)| render_group(source, annotations))
        .collect();
    if blocks.is_empty() {
        return String::new();
    }
    let mut out = blocks.join("\n\n");
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotate::model::{Classification, Side, Target};

    #[test]
    fn empty_store_renders_empty_string() {
        let store = AnnotationStore::new();
        assert_eq!(render_markdown(&store), "");
    }

    #[test]
    fn line_target_header_uses_new_side_marker() {
        let mut store = AnnotationStore::new();
        store
            .add(
                Target::line("src/auth/session.rs", 44, Side::New),
                Classification::Question,
                "where does keystore get rotated?",
            )
            .unwrap();
        let expected =
            "## src/auth/session.rs:44 (+)\n\n[question] where does keystore get rotated?\n";
        assert_eq!(render_markdown(&store), expected);
    }

    #[test]
    fn line_target_header_uses_old_side_marker() {
        let mut store = AnnotationStore::new();
        store
            .add(
                Target::line("src/auth/session.rs", 43, Side::Old),
                Classification::Issue,
                "this branch was dead code",
            )
            .unwrap();
        let expected = "## src/auth/session.rs:43 (-)\n\n[issue] this branch was dead code\n";
        assert_eq!(render_markdown(&store), expected);
    }

    #[test]
    fn range_target_header_uses_start_dash_end() {
        let mut store = AnnotationStore::new();
        store
            .add(
                Target::range("src/lib.rs", 10, 20, Side::New).unwrap(),
                Classification::Nit,
                "extract this into a helper",
            )
            .unwrap();
        let expected = "## src/lib.rs:10-20 (+)\n\n[nit] extract this into a helper\n";
        assert_eq!(render_markdown(&store), expected);
    }

    #[test]
    fn range_target_old_side() {
        let mut store = AnnotationStore::new();
        store
            .add(
                Target::range("src/lib.rs", 10, 20, Side::Old).unwrap(),
                Classification::Nit,
                "this whole block used to do X",
            )
            .unwrap();
        let expected = "## src/lib.rs:10-20 (-)\n\n[nit] this whole block used to do X\n";
        assert_eq!(render_markdown(&store), expected);
    }

    #[test]
    fn hunk_target_header_always_uses_plus_marker() {
        let mut store = AnnotationStore::new();
        store
            .add(
                Target::hunk("src/lib.rs", 1, 15).unwrap(),
                Classification::Praise,
                "clean refactor",
            )
            .unwrap();
        let expected = "## src/lib.rs:1-15 (+)\n\n[praise] clean refactor\n";
        assert_eq!(render_markdown(&store), expected);
    }

    #[test]
    fn file_target_header_has_no_side_marker() {
        let mut store = AnnotationStore::new();
        store
            .add(
                Target::file("src/lib.rs"),
                Classification::Praise,
                "great module boundary",
            )
            .unwrap();
        let expected = "## src/lib.rs\n\n[praise] great module boundary\n";
        assert_eq!(render_markdown(&store), expected);
    }

    #[test]
    fn every_classification_gets_correct_tag() {
        let mut store = AnnotationStore::new();
        store
            .add(Target::file("a.rs"), Classification::Issue, "body")
            .unwrap();
        store
            .add(Target::file("b.rs"), Classification::Question, "body")
            .unwrap();
        store
            .add(Target::file("c.rs"), Classification::Nit, "body")
            .unwrap();
        store
            .add(Target::file("d.rs"), Classification::Praise, "body")
            .unwrap();
        let rendered = render_markdown(&store);
        assert!(rendered.contains("[issue] body"));
        assert!(rendered.contains("[question] body"));
        assert!(rendered.contains("[nit] body"));
        assert!(rendered.contains("[praise] body"));
    }

    #[test]
    fn multiline_body_only_tags_first_line() {
        let mut store = AnnotationStore::new();
        store
            .add(
                Target::file("a.rs"),
                Classification::Issue,
                "first line\nsecond line\nthird line",
            )
            .unwrap();
        let expected = "## a.rs\n\n[issue] first line\nsecond line\nthird line\n";
        assert_eq!(render_markdown(&store), expected);
    }

    #[test]
    fn multiple_annotations_are_separated_by_one_blank_line() {
        let mut store = AnnotationStore::new();
        store
            .add(Target::file("a.rs"), Classification::Nit, "first")
            .unwrap();
        store
            .add(Target::file("b.rs"), Classification::Issue, "second")
            .unwrap();
        let expected = "## a.rs\n\n[nit] first\n\n## b.rs\n\n[issue] second\n";
        assert_eq!(render_markdown(&store), expected);
    }

    #[test]
    fn output_ends_with_single_trailing_newline() {
        let mut store = AnnotationStore::new();
        store
            .add(Target::file("a.rs"), Classification::Nit, "first")
            .unwrap();
        store
            .add(Target::file("b.rs"), Classification::Issue, "second")
            .unwrap();
        let rendered = render_markdown(&store);
        assert!(rendered.ends_with('\n'));
        assert!(!rendered.ends_with("\n\n"));
    }

    #[test]
    fn insertion_order_is_preserved_in_output() {
        let mut store = AnnotationStore::new();
        store
            .add(Target::file("z.rs"), Classification::Nit, "z-note")
            .unwrap();
        store
            .add(Target::file("a.rs"), Classification::Nit, "a-note")
            .unwrap();
        let rendered = render_markdown(&store);
        let z_pos = rendered.find("z.rs").unwrap();
        let a_pos = rendered.find("a.rs").unwrap();
        assert!(z_pos < a_pos);
    }

    // -- Source grouping (task 4.0) -----------------------------------------

    #[test]
    fn working_tree_only_session_has_no_reviewing_line() {
        // Every existing test above already builds a working-tree-only
        // session via `add` (which defaults to `Source::WorkingTree`) and
        // asserts an exact expected string with no `Reviewing:` line — this
        // is the explicit backward-compatibility assertion the task calls
        // for, phrased as its own test.
        let mut store = AnnotationStore::new();
        store
            .add(
                Target::range("src/lib.rs", 10, 20, Side::New).unwrap(),
                Classification::Nit,
                "extract this into a helper",
            )
            .unwrap();
        let expected = "## src/lib.rs:10-20 (+)\n\n[nit] extract this into a helper\n";
        assert_eq!(render_markdown(&store), expected);
        assert!(!render_markdown(&store).contains("Reviewing:"));
    }

    #[test]
    fn mixed_session_groups_working_tree_first_then_one_reviewing_line_per_group() {
        let mut store = AnnotationStore::new();
        store
            .add(
                Target::range("src/lib.rs", 10, 20, Side::New).unwrap(),
                Classification::Nit,
                "extract this into a helper",
            )
            .unwrap();
        store
            .add_with_source(
                Target::line("src/auth/session.rs", 44, Side::New),
                Classification::Question,
                "where does keystore get rotated?",
                Source::Commit("abc1234".to_string()),
            )
            .unwrap();
        let expected = "## src/lib.rs:10-20 (+)\n\n\
             [nit] extract this into a helper\n\n\
             Reviewing: abc1234\n\n\
             ## src/auth/session.rs:44 (+)\n\n\
             [question] where does keystore get rotated?\n";
        assert_eq!(render_markdown(&store), expected);
        assert_eq!(render_markdown(&store).matches("Reviewing:").count(), 1);
    }

    #[test]
    fn commit_group_reviewing_line_uses_short_sha() {
        let mut store = AnnotationStore::new();
        store
            .add_with_source(
                Target::file("a.rs"),
                Classification::Issue,
                "note",
                Source::Commit("abc1234".to_string()),
            )
            .unwrap();
        let expected = "Reviewing: abc1234\n\n## a.rs\n\n[issue] note\n";
        assert_eq!(render_markdown(&store), expected);
    }

    #[test]
    fn range_group_reviewing_line_uses_range_as_typed() {
        let mut store = AnnotationStore::new();
        store
            .add_with_source(
                Target::file("a.rs"),
                Classification::Issue,
                "note",
                Source::Range("main..feature".to_string()),
            )
            .unwrap();
        let expected = "Reviewing: main..feature\n\n## a.rs\n\n[issue] note\n";
        assert_eq!(render_markdown(&store), expected);
    }

    #[test]
    fn staged_group_reviewing_line_is_literally_staged() {
        let mut store = AnnotationStore::new();
        store
            .add_with_source(
                Target::file("a.rs"),
                Classification::Issue,
                "note",
                Source::Staged,
            )
            .unwrap();
        let expected = "Reviewing: staged\n\n## a.rs\n\n[issue] note\n";
        assert_eq!(render_markdown(&store), expected);
    }

    #[test]
    fn multiple_annotations_in_the_same_non_worktree_group_share_one_reviewing_line() {
        let mut store = AnnotationStore::new();
        store
            .add_with_source(
                Target::file("a.rs"),
                Classification::Issue,
                "first",
                Source::Commit("abc1234".to_string()),
            )
            .unwrap();
        store
            .add_with_source(
                Target::file("b.rs"),
                Classification::Nit,
                "second",
                Source::Commit("abc1234".to_string()),
            )
            .unwrap();
        let expected =
            "Reviewing: abc1234\n\n## a.rs\n\n[issue] first\n\n## b.rs\n\n[nit] second\n";
        assert_eq!(render_markdown(&store), expected);
        assert_eq!(render_markdown(&store).matches("Reviewing:").count(), 1);
    }

    #[test]
    fn different_non_worktree_sources_get_separate_reviewing_lines_in_first_appearance_order() {
        let mut store = AnnotationStore::new();
        store
            .add_with_source(
                Target::file("a.rs"),
                Classification::Issue,
                "first",
                Source::Commit("abc1234".to_string()),
            )
            .unwrap();
        store
            .add_with_source(
                Target::file("b.rs"),
                Classification::Nit,
                "second",
                Source::Staged,
            )
            .unwrap();
        let expected = "Reviewing: abc1234\n\n## a.rs\n\n[issue] first\n\n\
             Reviewing: staged\n\n## b.rs\n\n[nit] second\n";
        assert_eq!(render_markdown(&store), expected);
        assert_eq!(render_markdown(&store).matches("Reviewing:").count(), 2);
    }

    #[test]
    fn working_tree_group_always_emitted_first_regardless_of_insertion_order() {
        let mut store = AnnotationStore::new();
        store
            .add_with_source(
                Target::file("a.rs"),
                Classification::Issue,
                "commit note",
                Source::Commit("abc1234".to_string()),
            )
            .unwrap();
        store
            .add(Target::file("b.rs"), Classification::Nit, "worktree note")
            .unwrap();
        let expected = "## b.rs\n\n[nit] worktree note\n\n\
             Reviewing: abc1234\n\n## a.rs\n\n[issue] commit note\n";
        assert_eq!(render_markdown(&store), expected);
    }

    // -- The `(=)` marker (task 4.2) -----------------------------------------

    #[test]
    fn worktree_line_target_header_uses_equals_marker() {
        let mut store = AnnotationStore::new();
        store
            .add(
                Target::worktree_line("docs/notes.md", 44),
                Classification::Question,
                "should this doc mention the new flag?",
            )
            .unwrap();
        let expected =
            "## docs/notes.md:44 (=)\n\n[question] should this doc mention the new flag?\n";
        assert_eq!(render_markdown(&store), expected);
    }

    #[test]
    fn worktree_range_target_header_uses_start_dash_end_and_equals_marker() {
        let mut store = AnnotationStore::new();
        store
            .add(
                Target::worktree_range("docs/notes.md", 10, 20).unwrap(),
                Classification::Nit,
                "this whole section is stale",
            )
            .unwrap();
        let expected = "## docs/notes.md:10-20 (=)\n\n[nit] this whole section is stale\n";
        assert_eq!(render_markdown(&store), expected);
    }

    #[test]
    fn worktree_target_annotation_groups_with_working_tree_group_no_reviewing_line() {
        // A `(=)` annotation always reads live worktree content, so it must
        // land in the same metadata-line-free working-tree group as an
        // ordinary working-tree diff annotation -- never its own
        // `Reviewing:` group -- even though its own `add` call is
        // indistinguishable in insertion order from the diff one.
        let mut store = AnnotationStore::new();
        store
            .add(
                Target::line("src/lib.rs", 10, Side::New),
                Classification::Nit,
                "diff-side note",
            )
            .unwrap();
        store
            .add(
                Target::worktree_line("docs/notes.md", 3),
                Classification::Question,
                "worktree-side note",
            )
            .unwrap();
        let expected = "## src/lib.rs:10 (+)\n\n[nit] diff-side note\n\n\
             ## docs/notes.md:3 (=)\n\n[question] worktree-side note\n";
        assert_eq!(render_markdown(&store), expected);
        assert!(!render_markdown(&store).contains("Reviewing:"));
    }

    #[test]
    fn worktree_target_still_groups_with_working_tree_when_source_explicit() {
        // Same composition guarantee, but going through `add_with_source`
        // explicitly with `Source::WorkingTree` (what
        // `App::annotation_source` actually records for a file-view
        // annotation) rather than relying on `add`'s default.
        let mut store = AnnotationStore::new();
        store
            .add_with_source(
                Target::worktree_range("docs/notes.md", 1, 2).unwrap(),
                Classification::Issue,
                "note",
                Source::WorkingTree,
            )
            .unwrap();
        let expected = "## docs/notes.md:1-2 (=)\n\n[issue] note\n";
        assert_eq!(render_markdown(&store), expected);
    }

    #[test]
    fn non_worktree_only_session_has_no_leading_blank_group() {
        let mut store = AnnotationStore::new();
        store
            .add_with_source(
                Target::file("a.rs"),
                Classification::Issue,
                "note",
                Source::Commit("abc1234".to_string()),
            )
            .unwrap();
        let rendered = render_markdown(&store);
        assert!(rendered.starts_with("Reviewing: abc1234\n\n"));
        assert!(!rendered.starts_with('\n'));
    }
}
