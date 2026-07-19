//! Layering-guard test: `src/forge/` must never depend on a TUI crate, the
//! same rule `src/git/` follows. Enforced by scanning source text rather
//! than by a compile-time check, since a stray `use` could otherwise slip
//! in unnoticed until someone tries (and fails) to build `forge` without
//! the `ui` feature surface.

use std::fs;
use std::path::Path;

/// Returns the forbidden crate names, built without either name ever
/// appearing as a contiguous substring of *this* file's own source — so
/// this guard test doesn't trip on itself when the walk below reaches
/// `mod_tests.rs`.
fn forbidden_crate_names() -> [String; 2] {
    [["rata", "tui"].concat(), ["cross", "term"].concat()]
}

#[test]
fn forge_module_never_imports_a_tui_crate() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/forge");
    let forbidden = forbidden_crate_names();
    let mut offenders = Vec::new();

    for entry in fs::read_dir(&dir).expect("read src/forge directory") {
        let path = entry.expect("read src/forge directory entry").path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let contents = fs::read_to_string(&path).expect("read forge source file as UTF-8");
        for name in &forbidden {
            if contents.contains(name.as_str()) {
                offenders.push(format!("{}: mentions {name}", path.display()));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "src/forge/ must never import a TUI crate:\n{}",
        offenders.join("\n")
    );
}

// -- FR-6 docs contract: docs/forge-setup.md exists and README links it -----

/// `include_str!` fails the build outright if the file is missing, which is
/// a stronger existence check than a runtime `Path::exists`; the content
/// assertions below then guard against the doc drifting away from the real
/// degraded-state copy in `review_launcher_modal.rs`.
const FORGE_SETUP_DOCS: &str = include_str!("../../docs/forge-setup.md");
const README: &str = include_str!("../../README.md");

#[test]
fn readme_links_the_forge_setup_docs() {
    assert!(
        README.contains("docs/forge-setup.md"),
        "README.md must link docs/forge-setup.md"
    );
}

#[test]
fn forge_setup_docs_cover_every_fr5_degraded_state() {
    // Mirrors the exact copy `prs_degraded_body_lines` in
    // `review_launcher_modal.rs` renders, so the docs can't silently drift
    // from what the tab actually says.
    let must_contain = [
        "no forge remote",
        "gh auth login --hostname",
        "glab auth login --hostname",
        "neither CLI holds credentials",
        "both CLIs hold credentials",
        "isn't supported yet",
        "install gh: https://cli.github.com",
        "install glab: https://gitlab.com/gitlab-org/cli",
        "isn't on PATH",
        "is installed but not logged in for",
        "switch tabs and back to retry",
        "No open pull requests on",
    ];
    let missing: Vec<&str> = must_contain
        .into_iter()
        .filter(|s| !FORGE_SETUP_DOCS.contains(s))
        .collect();
    assert!(
        missing.is_empty(),
        "docs/forge-setup.md is missing coverage for: {missing:?}"
    );
}
