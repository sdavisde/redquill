use super::*;

fn table(s: &str) -> toml::Table {
    s.parse().expect("valid toml in test fixture")
}

#[test]
fn empty_file_is_all_defaults() {
    let (config, warnings) = Config::from_table(table(""));
    assert_eq!(config, Config::default());
    assert!(warnings.is_empty());
}

#[test]
fn partial_layout_overrides_only_the_named_key() {
    let (config, warnings) = Config::from_table(table(
        r#"
        [layout]
        sidebar_side = "left"
        "#,
    ));
    assert_eq!(config.layout.sidebar_side, SidebarSide::Left);
    // sidebar_width wasn't named, so it keeps its default (unset).
    assert_eq!(config.layout.sidebar_width, None);
    assert!(warnings.is_empty());
}

#[test]
fn invalid_value_for_a_known_key_falls_back_to_default_and_is_collected() {
    let (config, warnings) = Config::from_table(table(
        r#"
        [layout]
        sidebar_side = "up"
        "#,
    ));
    assert_eq!(config.layout.sidebar_side, SidebarSide::Right);
    assert_eq!(warnings.len(), 1);
    assert!(matches!(
        &warnings[0],
        ConfigWarning::InvalidValue { section, key, .. }
            if section == "layout" && key == "sidebar_side"
    ));
}

#[test]
fn sidebar_width_out_of_range_is_an_invalid_value() {
    let (config, warnings) = Config::from_table(table("[layout]\nsidebar_width = 5\n"));
    assert_eq!(config.layout.sidebar_width, None);
    assert_eq!(warnings.len(), 1);
    assert!(matches!(
        &warnings[0],
        ConfigWarning::InvalidValue { key, .. } if key == "sidebar_width"
    ));

    let (config, warnings) = Config::from_table(table("[layout]\nsidebar_width = 5000\n"));
    assert_eq!(config.layout.sidebar_width, None);
    assert_eq!(warnings.len(), 1);
}

#[test]
fn sidebar_width_in_range_applies() {
    let (config, warnings) = Config::from_table(table("[layout]\nsidebar_width = 50\n"));
    assert_eq!(config.layout.sidebar_width, Some(50));
    assert!(warnings.is_empty());

    // Boundary values are inclusive.
    let (config, warnings) = Config::from_table(table("[layout]\nsidebar_width = 20\n"));
    assert_eq!(config.layout.sidebar_width, Some(20));
    assert!(warnings.is_empty());
    let (config, warnings) = Config::from_table(table("[layout]\nsidebar_width = 200\n"));
    assert_eq!(config.layout.sidebar_width, Some(200));
    assert!(warnings.is_empty());
}

#[test]
fn scrolloff_defaults_to_the_shipped_margin() {
    let (config, warnings) = Config::from_table(table(""));
    assert_eq!(config.diff.scrolloff, SCROLLOFF_DEFAULT);
    assert!(warnings.is_empty());
}

#[test]
fn scrolloff_in_range_applies() {
    let (config, warnings) = Config::from_table(table("[diff]\nscrolloff = 4\n"));
    assert_eq!(config.diff.scrolloff, 4);
    assert!(warnings.is_empty());

    // Boundary values are inclusive; 0 means "cursor may reach the edge".
    let (config, warnings) = Config::from_table(table("[diff]\nscrolloff = 0\n"));
    assert_eq!(config.diff.scrolloff, 0);
    assert!(warnings.is_empty());
    let (config, warnings) =
        Config::from_table(table(&format!("[diff]\nscrolloff = {SCROLLOFF_MAX}\n")));
    assert_eq!(config.diff.scrolloff, SCROLLOFF_MAX);
    assert!(warnings.is_empty());
}

#[test]
fn scrolloff_out_of_range_or_wrong_type_is_an_invalid_value() {
    for raw in [
        "[diff]\nscrolloff = -1\n",
        "[diff]\nscrolloff = 100\n",
        "[diff]\nscrolloff = \"lots\"\n",
    ] {
        let (config, warnings) = Config::from_table(table(raw));
        assert_eq!(config.diff.scrolloff, SCROLLOFF_DEFAULT, "{raw}");
        assert_eq!(warnings.len(), 1, "{raw}");
        assert!(matches!(
            &warnings[0],
            ConfigWarning::InvalidValue { section, key, .. }
                if section == "diff" && key == "scrolloff"
        ));
    }
}

#[test]
fn search_section_partial_override() {
    let (config, warnings) = Config::from_table(table(
        r#"
        [search]
        case = "insensitive"
        "#,
    ));
    assert_eq!(config.search.case, CaseMode::Insensitive);
    assert!(!config.search.whole_word);
    assert!(!config.search.literal);
    assert!(warnings.is_empty());
}

#[test]
fn search_invalid_case_falls_back_to_default_and_is_collected() {
    let (config, warnings) = Config::from_table(table(
        r#"
        [search]
        case = "loud"
        "#,
    ));
    assert_eq!(config.search.case, CaseMode::Smart);
    assert_eq!(warnings.len(), 1);
    assert!(matches!(
        &warnings[0],
        ConfigWarning::InvalidValue { section, key, .. }
            if section == "search" && key == "case"
    ));
}

#[test]
fn search_whole_word_and_literal_apply() {
    let (config, warnings) = Config::from_table(table(
        r#"
        [search]
        whole_word = true
        literal = true
        "#,
    ));
    assert!(config.search.whole_word);
    assert!(config.search.literal);
    assert!(warnings.is_empty());
}

#[test]
fn both_sections_together_with_a_mix_of_valid_and_invalid_keys() {
    let (config, warnings) = Config::from_table(table(
        r#"
        [layout]
        sidebar_side = "left"
        sidebar_width = 55

        [search]
        case = "sensitive"
        whole_word = true
        "#,
    ));
    assert_eq!(config.layout.sidebar_side, SidebarSide::Left);
    assert_eq!(config.layout.sidebar_width, Some(55));
    assert_eq!(config.search.case, CaseMode::Sensitive);
    assert!(config.search.whole_word);
    assert!(!config.search.literal);
    assert!(warnings.is_empty());
}

#[test]
fn editor_section_partial_override_preset_only() {
    let (config, warnings) = Config::from_table(table(
        r#"
        [editor]
        preset = "zed"
        "#,
    ));
    assert_eq!(config.editor.preset.as_deref(), Some("zed"));
    assert_eq!(config.editor.edit_at_line, None);
    assert!(warnings.is_empty());
}

#[test]
fn editor_section_edit_at_line_with_filename_placeholder_applies() {
    let (config, warnings) = Config::from_table(table(
        r#"
        [editor]
        edit_at_line = "zed {{filename}}:{{line}}"
        "#,
    ));
    assert_eq!(
        config.editor.edit_at_line.as_deref(),
        Some("zed {{filename}}:{{line}}")
    );
    assert!(warnings.is_empty());
}

#[test]
fn editor_edit_at_line_missing_filename_placeholder_is_an_invalid_value() {
    let (config, warnings) = Config::from_table(table(
        r#"
        [editor]
        edit_at_line = "zed {{line}}"
        "#,
    ));
    assert_eq!(config.editor.edit_at_line, None);
    assert_eq!(warnings.len(), 1);
    assert!(matches!(
        &warnings[0],
        ConfigWarning::InvalidValue { section, key, .. }
            if section == "editor" && key == "edit_at_line"
    ));
}

#[test]
fn lsp_section_empty_is_all_defaults() {
    let (config, warnings) = Config::from_table(table("[lsp]\n"));
    assert_eq!(config.lsp, LspConfig::default());
    assert!(warnings.is_empty());
}

#[test]
fn lsp_override_one_language_command_and_args_leaves_others_default() {
    let (config, warnings) = Config::from_table(table(
        r#"
        [lsp.rust]
        command = "my-rust-analyzer"
        args = ["--wrapped"]
        "#,
    ));
    assert_eq!(config.lsp.rust.command.as_deref(), Some("my-rust-analyzer"));
    assert_eq!(config.lsp.rust.args, Some(vec!["--wrapped".to_string()]));
    assert!(config.lsp.rust.enabled);
    // Other languages are untouched.
    assert_eq!(config.lsp.typescript, LspServerOverride::default());
    assert_eq!(config.lsp.python, LspServerOverride::default());
    assert_eq!(config.lsp.go, LspServerOverride::default());
    assert!(warnings.is_empty());
}

#[test]
fn lsp_disable_one_language() {
    let (config, warnings) = Config::from_table(table(
        r#"
        [lsp.go]
        enabled = false
        "#,
    ));
    assert!(!config.lsp.go.enabled);
    assert_eq!(config.lsp.go.command, None);
    assert_eq!(config.lsp.go.args, None);
    assert!(warnings.is_empty());
}

#[test]
fn lsp_args_without_command_overrides_args_only() {
    let (config, warnings) = Config::from_table(table(
        r#"
        [lsp.typescript]
        args = ["--stdio", "--verbose"]
        "#,
    ));
    assert_eq!(config.lsp.typescript.command, None);
    assert_eq!(
        config.lsp.typescript.args,
        Some(vec!["--stdio".to_string(), "--verbose".to_string()])
    );
    assert!(warnings.is_empty());
}

#[test]
fn a_wrong_typed_value_falls_back_to_default_and_is_collected() {
    // (toml, warning section, warning key). Every fixture names exactly one
    // key, so a correct fallback leaves the whole config at its defaults.
    let cases: &[(&str, &str, &str)] = &[
        (
            "[layout]\nsidebar_width = \"wide\"\n",
            "layout",
            "sidebar_width",
        ),
        ("[search]\nwhole_word = \"yes\"\n", "search", "whole_word"),
        ("[editor]\npreset = 5\n", "editor", "preset"),
        ("[editor]\nedit_at_line = true\n", "editor", "edit_at_line"),
        ("[lsp.rust]\ncommand = 5\n", "lsp.rust", "command"),
        ("[lsp.rust]\nargs = \"nope\"\n", "lsp.rust", "args"),
        ("[lsp.rust]\nargs = [\"ok\", 5]\n", "lsp.rust", "args"),
        ("[lsp.rust]\nenabled = \"nope\"\n", "lsp.rust", "enabled"),
        // A section whose value isn't a table at all reports itself as both
        // the section and the key.
        ("layout = 5\n", "layout", "layout"),
        ("search = \"nope\"\n", "search", "search"),
        ("editor = 5\n", "editor", "editor"),
        ("[lsp]\nrust = 5\n", "lsp.rust", "lsp.rust"),
        ("lsp = 5\n", "lsp", "lsp"),
    ];
    for (raw, expected_section, expected_key) in cases {
        let (config, warnings) = Config::from_table(table(raw));
        assert_eq!(config, Config::default(), "{raw}");
        assert_eq!(warnings.len(), 1, "{raw}");
        assert!(
            matches!(
                &warnings[0],
                ConfigWarning::InvalidValue { section, key, .. }
                    if section == expected_section && key == expected_key
            ),
            "{raw} produced {:?}",
            warnings[0]
        );
    }
}

#[test]
fn an_unknown_key_is_collected_not_fatal_in_every_section() {
    // The fourth field asserts what must still have applied from the same
    // fixture — proving the unknown key didn't abort the parse.
    type UnknownKeyCase = (&'static str, &'static str, &'static str, fn(&Config));
    let cases: &[UnknownKeyCase] = &[
        ("[bogus]\nx = 1\n", "top-level", "bogus", |c| {
            assert_eq!(*c, Config::default())
        }),
        (
            "[layout]\nsidebar_side = \"left\"\nbogus = true\n",
            "layout",
            "bogus",
            |c| assert_eq!(c.layout.sidebar_side, SidebarSide::Left),
        ),
        ("[diff]\nscrolloff = 6\nbogus = 1\n", "diff", "bogus", |c| {
            assert_eq!(c.diff.scrolloff, 6)
        }),
        (
            "[editor]\npreset = \"vim\"\nbogus = true\n",
            "editor",
            "bogus",
            |c| assert_eq!(c.editor.preset.as_deref(), Some("vim")),
        ),
        ("[lsp.java]\ncommand = \"jdtls\"\n", "lsp", "java", |c| {
            assert_eq!(c.lsp, LspConfig::default())
        }),
        (
            "[lsp.rust]\ncommand = \"my-rust-analyzer\"\nbogus = true\n",
            "lsp.rust",
            "bogus",
            |c| assert_eq!(c.lsp.rust.command.as_deref(), Some("my-rust-analyzer")),
        ),
    ];
    for (raw, expected_section, expected_key, still_applies) in cases {
        let (config, warnings) = Config::from_table(table(raw));
        still_applies(&config);
        assert_eq!(warnings.len(), 1, "{raw}");
        assert!(
            matches!(
                &warnings[0],
                ConfigWarning::UnknownKey { section, key }
                    if section == expected_section && key == expected_key
            ),
            "{raw} produced {:?}",
            warnings[0]
        );
    }
}

#[test]
fn config_warning_display_names_path_section_and_key() {
    let syntax = ConfigWarning::SyntaxError {
        path: "/tmp/config.toml".to_string(),
        message: "TOML parse error at line 1".to_string(),
    };
    assert_eq!(
        syntax.to_string(),
        "/tmp/config.toml: TOML parse error at line 1"
    );

    let invalid = ConfigWarning::invalid("layout", "sidebar_side", "expected left or right");
    assert_eq!(
        invalid.to_string(),
        "[layout] sidebar_side: expected left or right"
    );

    let unknown = ConfigWarning::unknown("layout", "bogus");
    assert_eq!(unknown.to_string(), "[layout] unknown key `bogus`");
}
