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
