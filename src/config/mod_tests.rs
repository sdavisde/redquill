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
fn unknown_top_level_section_is_collected_not_fatal() {
    let (config, warnings) = Config::from_table(table(
        r#"
        [bogus]
        x = 1
        "#,
    ));
    assert_eq!(config, Config::default());
    assert_eq!(warnings.len(), 1);
    assert!(matches!(
        &warnings[0],
        ConfigWarning::UnknownKey { section, key }
            if section == "top-level" && key == "bogus"
    ));
}

#[test]
fn unknown_key_within_a_known_section_is_collected_not_fatal() {
    let (config, warnings) = Config::from_table(table(
        r#"
        [layout]
        sidebar_side = "left"
        bogus = true
        "#,
    ));
    // The valid key still applies...
    assert_eq!(config.layout.sidebar_side, SidebarSide::Left);
    // ...and the unknown one is collected, not fatal to the rest of the file.
    assert_eq!(warnings.len(), 1);
    assert!(matches!(
        &warnings[0],
        ConfigWarning::UnknownKey { section, key }
            if section == "layout" && key == "bogus"
    ));
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
fn sidebar_width_wrong_type_is_an_invalid_value() {
    let (config, warnings) = Config::from_table(table("[layout]\nsidebar_width = \"wide\"\n"));
    assert_eq!(config.layout.sidebar_width, None);
    assert_eq!(warnings.len(), 1);
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
fn search_boolean_field_wrong_type_is_an_invalid_value() {
    let (config, warnings) = Config::from_table(table("[search]\nwhole_word = \"yes\"\n"));
    assert!(!config.search.whole_word);
    assert_eq!(warnings.len(), 1);
}

#[test]
fn non_table_section_value_is_an_invalid_value() {
    let (config, warnings) = Config::from_table(table("layout = 5\n"));
    assert_eq!(config.layout, LayoutConfig::default());
    assert_eq!(warnings.len(), 1);

    let (config, warnings) = Config::from_table(table("search = \"nope\"\n"));
    assert_eq!(config.search, SearchConfig::default());
    assert_eq!(warnings.len(), 1);
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
fn sidebar_side_parses_both_values() {
    assert_eq!(SidebarSide::parse("left"), Some(SidebarSide::Left));
    assert_eq!(SidebarSide::parse("right"), Some(SidebarSide::Right));
    assert_eq!(SidebarSide::parse("Left"), None);
    assert_eq!(SidebarSide::parse(""), None);
}

#[test]
fn case_mode_parses_all_three_values() {
    assert_eq!(parse_case_mode("smart"), Some(CaseMode::Smart));
    assert_eq!(parse_case_mode("sensitive"), Some(CaseMode::Sensitive));
    assert_eq!(parse_case_mode("insensitive"), Some(CaseMode::Insensitive));
    assert_eq!(parse_case_mode("SMART"), None);
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
fn editor_section_both_fields_set_deserialize_independently() {
    // `EditorConfig` itself never picks a winner between `preset` and
    // `edit_at_line` — both fields simply hold whatever was configured; the
    // "explicit template wins" precedence is resolved (and tested) in
    // `crate::ui::editor::resolve_editor_config_tier`.
    let (config, warnings) = Config::from_table(table(
        r#"
        [editor]
        preset = "vim"
        edit_at_line = "zed {{filename}}:{{line}}"
        "#,
    ));
    assert_eq!(config.editor.preset.as_deref(), Some("vim"));
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
fn editor_preset_wrong_type_is_an_invalid_value() {
    let (config, warnings) = Config::from_table(table("[editor]\npreset = 5\n"));
    assert_eq!(config.editor.preset, None);
    assert_eq!(warnings.len(), 1);
    assert!(matches!(
        &warnings[0],
        ConfigWarning::InvalidValue { section, key, .. }
            if section == "editor" && key == "preset"
    ));
}

#[test]
fn editor_edit_at_line_wrong_type_is_an_invalid_value() {
    let (config, warnings) = Config::from_table(table("[editor]\nedit_at_line = true\n"));
    assert_eq!(config.editor.edit_at_line, None);
    assert_eq!(warnings.len(), 1);
}

#[test]
fn editor_unknown_key_within_the_section_is_collected_not_fatal() {
    let (config, warnings) = Config::from_table(table(
        r#"
        [editor]
        preset = "vim"
        bogus = true
        "#,
    ));
    assert_eq!(config.editor.preset.as_deref(), Some("vim"));
    assert_eq!(warnings.len(), 1);
    assert!(matches!(
        &warnings[0],
        ConfigWarning::UnknownKey { section, key }
            if section == "editor" && key == "bogus"
    ));
}

#[test]
fn editor_non_table_section_value_is_an_invalid_value() {
    let (config, warnings) = Config::from_table(table("editor = 5\n"));
    assert_eq!(config.editor, EditorConfig::default());
    assert_eq!(warnings.len(), 1);
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
