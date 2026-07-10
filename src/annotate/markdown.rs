//! Markdown serialization of an [`AnnotationStore`].
//!
//! [`render_markdown`] is the format emitted on quit to stdout by the
//! future UI. Treat it as a public API once shipped:
//!
//! ```text
//! ## src/auth/session.rs:44 (+)
//!
//! [question] where does keystore get rotated?
//! ```

use super::model::{Annotation, Classification, Side, Target};
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

/// Renders every annotation in `store`, in insertion order, as the
/// public-contract markdown format. An empty store renders to an empty
/// string; otherwise the output ends with a single trailing newline and
/// annotations are separated by exactly one blank line.
pub fn render_markdown(store: &AnnotationStore) -> String {
    let blocks: Vec<String> = store.iter().map(render_one).collect();
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
}
