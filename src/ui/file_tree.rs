//! The git panel's file tree: a pure transform from a flat list of changed
//! files into the nested, directory-grouped rows the panel renders, mirroring
//! an editor's file explorer. Directories are collapsible; single-child
//! directory chains are compressed into one row (`docs/specs` rather than
//! `docs` > `specs`), the same way Zed's panel folds them.
//!
//! No TUI types leak in here — this is data + transforms, unit-tested on its
//! own. The renderer in [`super::git_panel`] maps [`TreeRow`]s to styled
//! lines; the panel cursor model treats the flattened row list as its
//! navigable set.

use std::collections::BTreeMap;
use std::collections::HashSet;

use crate::diff::FileChangeKind;

/// One changed file fed into the tree: its index into `app.view.files`, full
/// path, change kind, and whether git considers it untracked.
#[derive(Debug, Clone)]
pub(super) struct TreeFile {
    pub file_index: usize,
    pub path: String,
    pub kind: FileChangeKind,
    pub untracked: bool,
}

/// A flattened tree row: either a directory (collapsible) or a file leaf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TreeNode {
    /// A directory row. `key` is its full path (`docs/specs`), the stable
    /// identity used for collapse state; `name` is the display text, which
    /// for a compressed chain spans several path components (`docs/specs`).
    Dir { key: String, name: String },
    /// A file leaf carrying everything the renderer needs plus the diff-view
    /// index the panel cursor follows to.
    File {
        file_index: usize,
        kind: FileChangeKind,
        untracked: bool,
        name: String,
    },
}

/// A visible row plus its indentation depth (0 = tree root).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TreeRow {
    pub depth: usize,
    pub node: TreeNode,
}

/// An in-progress directory during the build: sub-directories and files,
/// both keyed by name in a `BTreeMap` so iteration is alphabetical and
/// deterministic.
#[derive(Default)]
struct DirNode {
    dirs: BTreeMap<String, DirNode>,
    files: BTreeMap<String, TreeFile>,
}

/// Builds the nested tree from the flat file list. A file's directory
/// components become nested [`DirNode`]s; the basename lands in `files`.
fn build(files: &[TreeFile]) -> DirNode {
    let mut root = DirNode::default();
    for f in files {
        let comps: Vec<&str> = f.path.split('/').collect();
        let Some((base, dirs)) = comps.split_last() else {
            continue;
        };
        let mut node = &mut root;
        for comp in dirs {
            node = node.dirs.entry((*comp).to_string()).or_default();
        }
        node.files.insert((*base).to_string(), f.clone());
    }
    root
}

/// Emits `node`'s rows into `out`: directories first (alphabetical), then
/// files (alphabetical) — the conventional file-explorer ordering. A
/// directory whose only child is another directory (and which holds no files)
/// is compressed with that child into a single row, and so on down the chain.
/// A collapsed directory's subtree is skipped entirely.
fn emit(
    node: &DirNode,
    prefix: &str,
    depth: usize,
    collapsed: &HashSet<String>,
    out: &mut Vec<TreeRow>,
) {
    for (name, child) in &node.dirs {
        let mut key = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let mut display = name.clone();
        // Compress a single-subdirectory / no-file chain into one row.
        let mut cur = child;
        while cur.files.is_empty() && cur.dirs.len() == 1 {
            let Some((child_name, child_node)) = cur.dirs.iter().next() else {
                break;
            };
            display = format!("{display}/{child_name}");
            key = format!("{key}/{child_name}");
            cur = child_node;
        }
        out.push(TreeRow {
            depth,
            node: TreeNode::Dir {
                key: key.clone(),
                name: display,
            },
        });
        if !collapsed.contains(&key) {
            emit(cur, &key, depth + 1, collapsed, out);
        }
    }
    for (name, f) in &node.files {
        out.push(TreeRow {
            depth,
            node: TreeNode::File {
                file_index: f.file_index,
                kind: f.kind,
                untracked: f.untracked,
                name: name.clone(),
            },
        });
    }
}

/// Flattens `files` into the ordered, visible tree rows, hiding the subtree of
/// every directory whose key is in `collapsed`. The single source of truth
/// shared by the panel's renderer and its cursor motion helpers.
pub(super) fn flatten(files: &[TreeFile], collapsed: &HashSet<String>) -> Vec<TreeRow> {
    let root = build(files);
    let mut out = Vec::new();
    emit(&root, "", 0, collapsed, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(index: usize, path: &str, kind: FileChangeKind, untracked: bool) -> TreeFile {
        TreeFile {
            file_index: index,
            path: path.to_string(),
            kind,
            untracked,
        }
    }

    fn dir_keys(rows: &[TreeRow]) -> Vec<String> {
        rows.iter()
            .filter_map(|r| match &r.node {
                TreeNode::Dir { key, .. } => Some(key.clone()),
                _ => None,
            })
            .collect()
    }

    fn file_names(rows: &[TreeRow]) -> Vec<String> {
        rows.iter()
            .filter_map(|r| match &r.node {
                TreeNode::File { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn nests_files_under_their_directories() {
        let files = vec![
            file(0, "src/session.rs", FileChangeKind::Modified, false),
            file(1, "src/parser.rs", FileChangeKind::Added, false),
            file(2, "README.md", FileChangeKind::Modified, false),
        ];
        let rows = flatten(&files, &HashSet::new());
        // The `src` directory row, then its two files (alphabetical), then the
        // root-level README.
        assert_eq!(
            rows,
            vec![
                TreeRow {
                    depth: 0,
                    node: TreeNode::Dir {
                        key: "src".to_string(),
                        name: "src".to_string()
                    }
                },
                TreeRow {
                    depth: 1,
                    node: TreeNode::File {
                        file_index: 1,
                        kind: FileChangeKind::Added,
                        untracked: false,
                        name: "parser.rs".to_string()
                    }
                },
                TreeRow {
                    depth: 1,
                    node: TreeNode::File {
                        file_index: 0,
                        kind: FileChangeKind::Modified,
                        untracked: false,
                        name: "session.rs".to_string()
                    }
                },
                TreeRow {
                    depth: 0,
                    node: TreeNode::File {
                        file_index: 2,
                        kind: FileChangeKind::Modified,
                        untracked: false,
                        name: "README.md".to_string()
                    }
                },
            ]
        );
    }

    #[test]
    fn directories_sort_before_files_at_each_level() {
        let files = vec![
            file(0, "zzz.rs", FileChangeKind::Modified, false),
            file(1, "aaa/nested.rs", FileChangeKind::Modified, false),
        ];
        let rows = flatten(&files, &HashSet::new());
        assert!(matches!(rows[0].node, TreeNode::Dir { .. }));
        assert_eq!(dir_keys(&rows), vec!["aaa"]);
        assert_eq!(file_names(&rows), vec!["nested.rs", "zzz.rs"]);
    }

    #[test]
    fn compresses_single_child_directory_chains() {
        let files = vec![file(
            0,
            "docs/specs/09-spec.md",
            FileChangeKind::Added,
            true,
        )];
        let rows = flatten(&files, &HashSet::new());
        // `docs` > `specs` folds into one `docs/specs` row at depth 0.
        assert_eq!(
            dir_keys(&rows),
            vec!["docs/specs".to_string()],
            "the chain must compress to one row"
        );
        assert_eq!(rows[0].depth, 0);
        assert_eq!(rows[1].depth, 1); // the file sits one level in
    }

    #[test]
    fn does_not_compress_a_directory_that_also_holds_files() {
        let files = vec![
            file(0, "docs/top.md", FileChangeKind::Added, false),
            file(1, "docs/specs/deep.md", FileChangeKind::Added, false),
        ];
        let rows = flatten(&files, &HashSet::new());
        // `docs` holds a file, so it can't fold into `specs`.
        assert_eq!(dir_keys(&rows), vec!["docs", "docs/specs"]);
    }

    #[test]
    fn collapsing_a_directory_hides_its_subtree() {
        let files = vec![
            file(0, "src/session.rs", FileChangeKind::Modified, false),
            file(1, "README.md", FileChangeKind::Modified, false),
        ];
        let collapsed = HashSet::from(["src".to_string()]);
        let rows = flatten(&files, &collapsed);
        // The `src` row survives; its file is hidden. README still shows.
        assert_eq!(dir_keys(&rows), vec!["src"]);
        assert_eq!(file_names(&rows), vec!["README.md"]);
    }

    #[test]
    fn collapsing_a_compressed_chain_uses_the_full_key() {
        let files = vec![file(
            0,
            "docs/specs/09-spec.md",
            FileChangeKind::Added,
            true,
        )];
        // The compressed row's key is the full `docs/specs`, so that's what
        // collapse toggles against.
        let collapsed = HashSet::from(["docs/specs".to_string()]);
        let rows = flatten(&files, &collapsed);
        assert_eq!(dir_keys(&rows), vec!["docs/specs"]);
        assert!(file_names(&rows).is_empty());
    }

    #[test]
    fn empty_input_yields_no_rows() {
        assert!(flatten(&[], &HashSet::new()).is_empty());
    }
}
