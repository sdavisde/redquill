use super::*;

fn one(code: KeyCode, mods: KeyModifiers) -> KeySeqSpec {
    KeySeqSpec::One(ChordSpec { code, mods })
}

fn two(c1: KeyCode, m1: KeyModifiers, c2: KeyCode, m2: KeyModifiers) -> KeySeqSpec {
    KeySeqSpec::Two(
        ChordSpec { code: c1, mods: m1 },
        ChordSpec { code: c2, mods: m2 },
    )
}

// -- Single chords, per the spec's example grammar strings ------------------

#[test]
fn plain_letter_parses_as_a_bare_char() {
    assert_eq!(
        parse_key_string("a"),
        Ok(one(KeyCode::Char('a'), KeyModifiers::NONE))
    );
}

#[test]
fn uppercase_letter_parses_as_a_bare_char_with_no_modifier() {
    // Matches `KeyChord::matches`'s SHIFT-stripping convention: uppercase
    // chars carry no explicit SHIFT bit in the runtime representation.
    assert_eq!(
        parse_key_string("J"),
        Ok(one(KeyCode::Char('J'), KeyModifiers::NONE))
    );
}

#[test]
fn ctrl_prefix_parses() {
    assert_eq!(
        parse_key_string("ctrl-k"),
        Ok(one(KeyCode::Char('k'), KeyModifiers::CONTROL))
    );
}

#[test]
fn ctrl_prefix_is_case_insensitive() {
    assert_eq!(
        parse_key_string("Ctrl-K"),
        Ok(one(KeyCode::Char('K'), KeyModifiers::CONTROL))
    );
}

#[test]
fn alt_enter_parses() {
    assert_eq!(
        parse_key_string("alt-enter"),
        Ok(one(KeyCode::Enter, KeyModifiers::ALT))
    );
}

#[test]
fn shift_tab_collapses_to_backtab_with_no_modifier() {
    // Matches `KeyChord::label`'s rendering of the same physical key and
    // `default_map`'s own `BackTab` (no-modifier) representation.
    assert_eq!(
        parse_key_string("shift-tab"),
        Ok(one(KeyCode::BackTab, KeyModifiers::NONE))
    );
}

#[test]
fn f5_parses_as_a_function_key() {
    assert_eq!(
        parse_key_string("f5"),
        Ok(one(KeyCode::F(5), KeyModifiers::NONE))
    );
}

#[test]
fn f_key_range_is_bounded() {
    assert!(parse_key_string("f0").is_err());
    assert!(parse_key_string("f25").is_err());
    assert!(parse_key_string("f24").is_ok());
    assert!(parse_key_string("f1").is_ok());
}

#[test]
fn esc_parses() {
    assert_eq!(
        parse_key_string("esc"),
        Ok(one(KeyCode::Esc, KeyModifiers::NONE))
    );
}

#[test]
fn named_keys_parse() {
    assert_eq!(
        parse_key_string("space"),
        Ok(one(KeyCode::Char(' '), KeyModifiers::NONE))
    );
    assert_eq!(
        parse_key_string("enter"),
        Ok(one(KeyCode::Enter, KeyModifiers::NONE))
    );
    assert_eq!(
        parse_key_string("tab"),
        Ok(one(KeyCode::Tab, KeyModifiers::NONE))
    );
    assert_eq!(
        parse_key_string("backspace"),
        Ok(one(KeyCode::Backspace, KeyModifiers::NONE))
    );
    assert_eq!(
        parse_key_string("delete"),
        Ok(one(KeyCode::Delete, KeyModifiers::NONE))
    );
    assert_eq!(
        parse_key_string("home"),
        Ok(one(KeyCode::Home, KeyModifiers::NONE))
    );
    assert_eq!(
        parse_key_string("end"),
        Ok(one(KeyCode::End, KeyModifiers::NONE))
    );
    assert_eq!(
        parse_key_string("pageup"),
        Ok(one(KeyCode::PageUp, KeyModifiers::NONE))
    );
    assert_eq!(
        parse_key_string("pagedown"),
        Ok(one(KeyCode::PageDown, KeyModifiers::NONE))
    );
    assert_eq!(
        parse_key_string("up"),
        Ok(one(KeyCode::Up, KeyModifiers::NONE))
    );
    assert_eq!(
        parse_key_string("insert"),
        Ok(one(KeyCode::Insert, KeyModifiers::NONE))
    );
}

#[test]
fn punctuation_chars_parse_as_bare_chars() {
    for c in ['?', '@', '$', '#', '!', '/', '[', ']', '`'] {
        assert_eq!(
            parse_key_string(&c.to_string()),
            Ok(one(KeyCode::Char(c), KeyModifiers::NONE)),
            "punctuation char {c:?} must parse as a bare char"
        );
    }
}

#[test]
fn stacked_modifiers_apply_in_any_order() {
    assert_eq!(
        parse_key_string("ctrl-alt-delete"),
        Ok(one(
            KeyCode::Delete,
            KeyModifiers::CONTROL | KeyModifiers::ALT
        ))
    );
}

// -- Two-chord sequences -----------------------------------------------------

#[test]
fn space_separated_two_chords_parse_as_a_sequence() {
    assert_eq!(
        parse_key_string("g d"),
        Ok(two(
            KeyCode::Char('g'),
            KeyModifiers::NONE,
            KeyCode::Char('d'),
            KeyModifiers::NONE,
        ))
    );
}

#[test]
fn two_chord_sequence_with_modifiers_parses() {
    assert_eq!(
        parse_key_string("g ctrl-d"),
        Ok(two(
            KeyCode::Char('g'),
            KeyModifiers::NONE,
            KeyCode::Char('d'),
            KeyModifiers::CONTROL,
        ))
    );
}

#[test]
fn three_or_more_chords_is_rejected() {
    assert_eq!(
        parse_key_string("g d d"),
        Err(KeyGrammarError::TooManyChords("g d d".to_string()))
    );
}

// -- Parse-reject cases for garbage ------------------------------------------

#[test]
fn empty_string_is_rejected() {
    assert_eq!(parse_key_string(""), Err(KeyGrammarError::Empty));
    assert_eq!(parse_key_string("   "), Err(KeyGrammarError::Empty));
}

#[test]
fn unknown_key_name_is_rejected() {
    assert!(matches!(
        parse_key_string("frobnicate"),
        Err(KeyGrammarError::UnknownKeyName(_))
    ));
}

#[test]
fn dangling_modifier_prefix_is_rejected() {
    assert!(matches!(
        parse_key_string("ctrl-"),
        Err(KeyGrammarError::UnknownKeyName(_))
    ));
}

#[test]
fn multi_char_garbage_is_rejected() {
    assert!(matches!(
        parse_key_string("xyz"),
        Err(KeyGrammarError::UnknownKeyName(_))
    ));
}

// -- `[keys.diff]`/`[keys.panel]` section parsing ----------------------------

#[test]
fn keys_section_parses_string_and_array_values() {
    let toml = r#"
        [diff]
        next-file = "J"
        quit = ["q", "ctrl-c"]
        toggle-collapse = []

        [panel]
        stage-line = "x"
    "#;
    let raw: toml::Table = toml.parse().unwrap();
    let mut warnings = Vec::new();
    let cfg = KeysConfig::from_value(toml::Value::Table(raw), &mut warnings);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    assert_eq!(
        cfg.diff.get("next-file"),
        Some(&vec![one_spec(KeyCode::Char('J'), KeyModifiers::NONE)])
    );
    assert_eq!(
        cfg.diff.get("quit"),
        Some(&vec![
            one_spec(KeyCode::Char('q'), KeyModifiers::NONE),
            one_spec(KeyCode::Char('c'), KeyModifiers::CONTROL),
        ])
    );
    assert_eq!(cfg.diff.get("toggle-collapse"), Some(&Vec::new()));
    assert_eq!(
        cfg.panel.get("stage-line"),
        Some(&vec![one_spec(KeyCode::Char('x'), KeyModifiers::NONE)])
    );
}

#[test]
fn keys_section_parses_the_global_table() {
    let toml = r#"
        [global]
        toggle-command-log = "ctrl-l"
        quit = []
    "#;
    let raw: toml::Table = toml.parse().unwrap();
    let mut warnings = Vec::new();
    let cfg = KeysConfig::from_value(toml::Value::Table(raw), &mut warnings);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    assert_eq!(
        cfg.global.get("toggle-command-log"),
        Some(&vec![one_spec(KeyCode::Char('l'), KeyModifiers::CONTROL)])
    );
    assert_eq!(cfg.global.get("quit"), Some(&Vec::new()));
}

fn one_spec(code: KeyCode, mods: KeyModifiers) -> KeySeqSpec {
    KeySeqSpec::One(ChordSpec { code, mods })
}

#[test]
fn keys_section_collects_a_warning_for_an_unparseable_key_string_and_drops_the_entry() {
    let toml = r#"
        [diff]
        next-file = "not a real key notation!!"
    "#;
    let raw: toml::Table = toml.parse().unwrap();
    let mut warnings = Vec::new();
    let cfg = KeysConfig::from_value(toml::Value::Table(raw), &mut warnings);
    assert_eq!(warnings.len(), 1);
    assert!(!cfg.diff.contains_key("next-file"));
}

#[test]
fn keys_section_collects_a_warning_for_an_unknown_subsection() {
    let toml = r#"
        [bogus]
        foo = "a"
    "#;
    let raw: toml::Table = toml.parse().unwrap();
    let mut warnings = Vec::new();
    let cfg = KeysConfig::from_value(toml::Value::Table(raw), &mut warnings);
    assert_eq!(warnings.len(), 1);
    assert!(cfg.diff.is_empty());
    assert!(cfg.panel.is_empty());
}

#[test]
fn keys_section_rejects_a_non_table_value_at_top_level() {
    let mut warnings = Vec::new();
    let cfg = KeysConfig::from_value(toml::Value::String("nope".to_string()), &mut warnings);
    assert_eq!(warnings.len(), 1);
    assert_eq!(cfg, KeysConfig::default());
}

#[test]
fn keys_subsection_rejects_a_non_table_value() {
    let toml = r#"diff = "nope""#;
    let raw: toml::Table = toml.parse().unwrap();
    let mut warnings = Vec::new();
    let cfg = KeysConfig::from_value(toml::Value::Table(raw), &mut warnings);
    assert_eq!(warnings.len(), 1);
    assert!(cfg.diff.is_empty());
}

#[test]
fn keys_entry_rejects_a_non_string_array_item() {
    let toml = r#"
        [diff]
        quit = ["q", 5]
    "#;
    let raw: toml::Table = toml.parse().unwrap();
    let mut warnings = Vec::new();
    let cfg = KeysConfig::from_value(toml::Value::Table(raw), &mut warnings);
    assert_eq!(warnings.len(), 1);
    assert!(!cfg.diff.contains_key("quit"));
}
