use std::path::{Path, PathBuf};

use super::*;
use crate::config::SidebarSide;

fn env(xdg: Option<&Path>, home: Option<&Path>) -> PathEnv {
    PathEnv {
        xdg_config_home: xdg.map(PathBuf::from),
        home: home.map(PathBuf::from),
        appdata: None,
    }
}

// -- Path discovery -----------------------------------------------------------

#[test]
fn xdg_set_wins_over_home_fallback() {
    let path = resolve_config_path(&env(Some(Path::new("/xdg")), Some(Path::new("/home/user"))));
    assert_eq!(path, Some(PathBuf::from("/xdg/redquill/config.toml")));
}

#[test]
fn xdg_unset_falls_back_to_dot_config_under_home() {
    let path = resolve_config_path(&env(None, Some(Path::new("/home/user"))));
    assert_eq!(
        path,
        Some(PathBuf::from("/home/user/.config/redquill/config.toml"))
    );
}

#[test]
fn missing_home_and_xdg_yields_no_path() {
    let path = resolve_config_path(&env(None, None));
    assert_eq!(path, None);
}

#[test]
fn explicit_test_override_hook_is_just_a_pathenv_pointed_at_a_tempdir() {
    // Any test can point discovery at a tempdir directly via `PathEnv`,
    // bypassing the real environment entirely (see the `load_from` tests
    // below, which do exactly this with a real file on disk).
    let dir = tempfile::tempdir().expect("tempdir");
    let dir = dir
        .path()
        .canonicalize()
        .expect("canonicalize tempdir (macOS /var symlink)");
    let path = resolve_config_path(&env(Some(&dir), None));
    assert_eq!(path, Some(dir.join("redquill").join("config.toml")));
}

// -- load_from (the degradation contract) ------------------------------------

#[test]
fn missing_file_is_silent_defaults() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dir = dir
        .path()
        .canonicalize()
        .expect("canonicalize tempdir (macOS /var symlink)");
    let (config, warnings) = load_from(&env(Some(&dir), None));
    assert_eq!(config, Config::default());
    assert!(warnings.is_empty());
}

#[test]
fn present_valid_file_is_read_and_applied() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dir = dir
        .path()
        .canonicalize()
        .expect("canonicalize tempdir (macOS /var symlink)");
    std::fs::create_dir_all(dir.join("redquill")).expect("mkdir");
    std::fs::write(
        dir.join("redquill").join("config.toml"),
        "[layout]\nsidebar_side = \"left\"\n",
    )
    .expect("write config");

    let (config, warnings) = load_from(&env(Some(&dir), None));
    assert_eq!(config.layout.sidebar_side, SidebarSide::Left);
    assert!(warnings.is_empty());
}

#[test]
fn syntax_error_yields_full_defaults_and_one_warning_naming_path_and_line() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dir = dir
        .path()
        .canonicalize()
        .expect("canonicalize tempdir (macOS /var symlink)");
    std::fs::create_dir_all(dir.join("redquill")).expect("mkdir");
    let path = dir.join("redquill").join("config.toml");
    // A missing closing bracket: a genuine TOML syntax error, not just an
    // invalid value for a known key.
    std::fs::write(&path, "[layout\nsidebar_side = \"left\"\n").expect("write config");

    let (config, warnings) = load_from(&env(Some(&dir), None));
    assert_eq!(config, Config::default());
    assert_eq!(warnings.len(), 1);
    match &warnings[0] {
        ConfigWarning::SyntaxError { path: p, message } => {
            assert_eq!(p, &path.display().to_string());
            assert!(
                message.to_lowercase().contains("line"),
                "expected the parser's line info in {message:?}"
            );
        }
        other => panic!("expected SyntaxError, got {other:?}"),
    }
}

#[test]
fn parseable_but_invalid_entries_partially_apply_with_one_warning_each() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dir = dir
        .path()
        .canonicalize()
        .expect("canonicalize tempdir (macOS /var symlink)");
    std::fs::create_dir_all(dir.join("redquill")).expect("mkdir");
    std::fs::write(
        dir.join("redquill").join("config.toml"),
        "[layout]\nsidebar_side = \"left\"\nsidebar_width = 999999\n",
    )
    .expect("write config");

    let (config, warnings) = load_from(&env(Some(&dir), None));
    // The valid key still applied...
    assert_eq!(config.layout.sidebar_side, SidebarSide::Left);
    // ...the invalid one fell back to its default...
    assert_eq!(config.layout.sidebar_width, None);
    // ...and was collected as exactly one warning.
    assert_eq!(warnings.len(), 1);
}

#[test]
fn unreadable_path_degrades_like_a_missing_file() {
    // A path whose parent directory doesn't exist can never be read; this
    // exercises the same silent-defaults branch as "no file present" (the
    // I/O error is deliberately not distinguished from "missing").
    let dir = tempfile::tempdir().expect("tempdir");
    let dir = dir
        .path()
        .canonicalize()
        .expect("canonicalize tempdir (macOS /var symlink)")
        .join("does-not-exist");
    let (config, warnings) = load_from(&env(Some(&dir), None));
    assert_eq!(config, Config::default());
    assert!(warnings.is_empty());
}

// -- stdout contract ----------------------------------------------------------

#[test]
fn config_loading_source_never_writes_to_stdout() {
    // A real OS-level fd capture would need unsafe FFI outside std's stable
    // API for a dependency-free unit test; this is the structural guard for
    // the same regression class (someone adding a stray debugging
    // `println!`): stdout is reserved for the annotation markdown
    // (`crate::annotate`), never for config loading.
    for src in [include_str!("mod.rs"), include_str!("load.rs")] {
        assert!(
            !src.contains("println!") && !src.contains("print!(") && !src.contains("io::stdout"),
            "config loading must never write to stdout"
        );
    }
}

#[test]
fn load_reads_the_real_environment_without_panicking() {
    // Smoke test for the real, non-injected entry point `main` calls.
    let (_config, _warnings) = load();
}

// -- Example config drift guard ----------------------------------------------

#[test]
fn example_config_toml_parses_with_zero_warnings() {
    // A lightweight drift test, scoped to the
    // sections this slice ships (`[layout]`/`[search]`): every key and
    // value in `docs/example-config.toml` must actually be recognized, so
    // the shipped example can't silently rot as the code evolves.
    let text = include_str!("../../docs/example-config.toml");
    let raw: toml::Table = text.parse().expect("example config must be valid TOML");
    let (_config, warnings) = Config::from_table(raw);
    assert!(
        warnings.is_empty(),
        "docs/example-config.toml produced warnings: {warnings:?}"
    );
}
