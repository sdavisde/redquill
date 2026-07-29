use std::path::Path;

use super::*;
use crate::config::EditorConfig;

// --- `build_editor_command` (the original two-family heuristic) ---

#[test]
fn nvim_uses_plus_line_convention() {
    let (program, args) = build_editor_command("nvim", Path::new("path/to/file.rs"), 42);
    assert_eq!(program, "nvim");
    assert_eq!(args, vec!["+42".to_string(), "path/to/file.rs".to_string()]);
}

#[test]
fn code_with_leading_args_uses_goto_flag() {
    let (program, args) = build_editor_command("code --wait", Path::new("path/to/file.rs"), 42);
    assert_eq!(program, "code");
    assert_eq!(
        args,
        vec![
            "--wait".to_string(),
            "--goto".to_string(),
            "path/to/file.rs:42".to_string(),
        ]
    );
}

#[test]
fn codium_special_cases_like_code() {
    let (program, args) = build_editor_command("codium", Path::new("f.rs"), 7);
    assert_eq!(program, "codium");
    assert_eq!(args, vec!["--goto".to_string(), "f.rs:7".to_string()]);
}

#[test]
fn full_path_to_code_binary_still_special_cases_by_basename() {
    let (program, args) = build_editor_command("/usr/local/bin/code", Path::new("f.rs"), 3);
    assert_eq!(program, "/usr/local/bin/code");
    assert_eq!(args, vec!["--goto".to_string(), "f.rs:3".to_string()]);
}

#[test]
fn empty_editor_string_falls_back_to_nvim() {
    let (program, args) = build_editor_command("", Path::new("f.rs"), 1);
    assert_eq!(program, "nvim");
    assert_eq!(args, vec!["+1".to_string(), "f.rs".to_string()]);
}

// --- `build_from_template` (the config template engine) ---

#[test]
fn template_filename_with_spaces_survives_as_one_argv_element() {
    let (program, args) =
        build_from_template("zed {{filename}}:{{line}}", Path::new("my file.rs"), 5)
            .expect("template has {{filename}}");
    assert_eq!(program, "zed");
    // Splitting happens *before* substitution, so the space inside the
    // substituted filename doesn't get re-split into two args.
    assert_eq!(args, vec!["my file.rs:5".to_string()]);
}

#[test]
fn template_without_line_placeholder_ignores_line() {
    let (program, args) =
        build_from_template("subl {{filename}}", Path::new("f.rs"), 9).expect("valid template");
    assert_eq!(program, "subl");
    assert_eq!(args, vec!["f.rs".to_string()]);
}

#[test]
fn template_mixed_literal_and_placeholder_tokens() {
    let (program, args) = build_from_template(
        "editor --at={{filename}}:{{line}} --flag",
        Path::new("src/lib.rs"),
        12,
    )
    .expect("valid template");
    assert_eq!(program, "editor");
    assert_eq!(
        args,
        vec!["--at=src/lib.rs:12".to_string(), "--flag".to_string()]
    );
}

#[test]
fn template_placeholder_ordering_line_before_filename() {
    let (program, args) = build_from_template("kak +{{line}} {{filename}}", Path::new("f.rs"), 4)
        .expect("valid template");
    assert_eq!(program, "kak");
    assert_eq!(args, vec!["+4".to_string(), "f.rs".to_string()]);
}

#[test]
fn template_missing_filename_placeholder_is_rejected() {
    assert_eq!(
        build_from_template("vim {{line}}", Path::new("f.rs"), 1),
        None
    );
}

#[test]
fn template_empty_string_is_rejected() {
    assert_eq!(build_from_template("", Path::new("f.rs"), 1), None);
}

// --- preset table — every preset, exact argv ---

/// Every built-in preset expands to the exact argv its editor's CLI expects.
/// The row set is checked against `PRESETS` itself, so a new preset without a
/// row here fails rather than going unverified.
#[test]
fn every_preset_expands_to_its_editors_exact_argv() {
    // (preset name, expected program, expected args)
    let expected: &[(&str, &str, &[&str])] = &[
        ("vim", "vim", &["+42", "src/lib.rs"]),
        ("nvim", "nvim", &["+42", "src/lib.rs"]),
        ("helix", "hx", &["src/lib.rs:42"]),
        ("vscode", "code", &["--goto", "src/lib.rs:42"]),
        ("vscodium", "codium", &["--goto", "src/lib.rs:42"]),
        ("zed", "zed", &["src/lib.rs:42"]),
        ("emacs", "emacs", &["+42", "src/lib.rs"]),
        ("nano", "nano", &["+42", "src/lib.rs"]),
        ("micro", "micro", &["src/lib.rs:42"]),
        ("sublime", "subl", &["src/lib.rs:42"]),
        ("kakoune", "kak", &["+42", "src/lib.rs"]),
    ];

    let covered: Vec<&str> = expected.iter().map(|(name, ..)| *name).collect();
    let declared: Vec<&str> = PRESETS.iter().map(|(name, _)| *name).collect();
    assert_eq!(
        covered, declared,
        "every preset in PRESETS needs a row here, in table order"
    );

    for (name, program, args) in expected {
        let template = preset_template(name).expect("known preset");
        assert!(
            template.contains("{{filename}}"),
            "preset {name} must place the file"
        );
        let (got_program, got_args) =
            build_from_template(template, Path::new("src/lib.rs"), 42).expect("valid template");
        assert_eq!(got_program, *program, "preset {name} program");
        assert_eq!(
            got_args,
            args.iter().map(|a| a.to_string()).collect::<Vec<_>>(),
            "preset {name} args"
        );
    }
}

#[test]
fn unknown_preset_name_is_rejected() {
    assert_eq!(preset_template("bogus"), None);
    assert_eq!(preset_template(""), None);
    // Names aren't case-folded — an exact match is required, matching the
    // rest of the config grammar's plain-string enum parsing.
    assert_eq!(preset_template("Vim"), None);
}

// --- `resolve_editor_config_tier` (tasks 2.3/2.4: the config tier itself) ---

#[test]
fn absent_when_neither_field_is_set() {
    let cfg = EditorConfig::default();
    assert_eq!(resolve_editor_config_tier(&cfg), EditorConfigTier::Absent);
}

#[test]
fn preset_alone_resolves_to_its_template() {
    let cfg = EditorConfig {
        preset: Some("zed".to_string()),
        edit_at_line: None,
    };
    assert_eq!(
        resolve_editor_config_tier(&cfg),
        EditorConfigTier::Template("zed {{filename}}:{{line}}".to_string())
    );
}

#[test]
fn edit_at_line_alone_resolves_to_itself() {
    let cfg = EditorConfig {
        preset: None,
        edit_at_line: Some("myeditor {{filename}} {{line}}".to_string()),
    };
    assert_eq!(
        resolve_editor_config_tier(&cfg),
        EditorConfigTier::Template("myeditor {{filename}} {{line}}".to_string())
    );
}

#[test]
fn explicit_edit_at_line_wins_over_preset_when_both_set() {
    let cfg = EditorConfig {
        preset: Some("vim".to_string()),
        edit_at_line: Some("myeditor {{filename}} {{line}}".to_string()),
    };
    assert_eq!(
        resolve_editor_config_tier(&cfg),
        EditorConfigTier::Template("myeditor {{filename}} {{line}}".to_string())
    );
}

#[test]
fn unknown_preset_name_is_reported_not_silently_dropped() {
    let cfg = EditorConfig {
        preset: Some("emacs-but-misspelled".to_string()),
        edit_at_line: None,
    };
    assert_eq!(
        resolve_editor_config_tier(&cfg),
        EditorConfigTier::UnknownPreset("emacs-but-misspelled".to_string())
    );
}
