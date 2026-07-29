use super::*;
use crate::config::LspServerOverride;

#[test]
fn no_overrides_yields_defaults_unchanged() {
    let cfg = LspConfig::default();
    let map = effective_lsp_commands(&cfg);
    assert_eq!(map, default_commands());
}

#[test]
fn override_one_language_leaves_others_default() {
    let cfg = LspConfig {
        rust: LspServerOverride {
            command: Some("my-rust-analyzer".to_string()),
            args: Some(vec!["--wrapped".to_string()]),
            enabled: true,
        },
        ..LspConfig::default()
    };
    let map = effective_lsp_commands(&cfg);
    assert_eq!(
        map[&ServerLang::Rust],
        LangServerCmd {
            command: "my-rust-analyzer".to_string(),
            args: vec!["--wrapped".to_string()],
        }
    );
    let defaults = default_commands();
    assert_eq!(
        map[&ServerLang::TypeScript],
        defaults[&ServerLang::TypeScript]
    );
    assert_eq!(map[&ServerLang::Python], defaults[&ServerLang::Python]);
    assert_eq!(map[&ServerLang::Go], defaults[&ServerLang::Go]);
    assert_eq!(map.len(), 4);
}

#[test]
fn disable_one_language_removes_it_others_stay_default() {
    let cfg = LspConfig {
        go: LspServerOverride {
            command: None,
            args: None,
            enabled: false,
        },
        ..LspConfig::default()
    };
    let map = effective_lsp_commands(&cfg);
    assert!(!map.contains_key(&ServerLang::Go));
    assert_eq!(map.len(), 3);
    let defaults = default_commands();
    assert_eq!(map[&ServerLang::Rust], defaults[&ServerLang::Rust]);
    assert_eq!(
        map[&ServerLang::TypeScript],
        defaults[&ServerLang::TypeScript]
    );
    assert_eq!(map[&ServerLang::Python], defaults[&ServerLang::Python]);
}

#[test]
fn args_without_command_overrides_args_only() {
    let cfg = LspConfig {
        typescript: LspServerOverride {
            command: None,
            args: Some(vec!["--stdio".to_string(), "--verbose".to_string()]),
            enabled: true,
        },
        ..LspConfig::default()
    };
    let map = effective_lsp_commands(&cfg);
    let entry = &map[&ServerLang::TypeScript];
    assert_eq!(entry.command, "typescript-language-server");
    assert_eq!(
        entry.args,
        vec!["--stdio".to_string(), "--verbose".to_string()]
    );
}

#[test]
fn command_without_args_keeps_default_args() {
    let cfg = LspConfig {
        typescript: LspServerOverride {
            command: Some("my-typescript-server".to_string()),
            args: None,
            enabled: true,
        },
        ..LspConfig::default()
    };
    let map = effective_lsp_commands(&cfg);
    let entry = &map[&ServerLang::TypeScript];
    assert_eq!(entry.command, "my-typescript-server");
    // Default args are kept since this language's args weren't overridden.
    assert_eq!(entry.args, vec!["--stdio".to_string()]);
}

#[test]
fn disabling_a_language_wins_even_if_command_or_args_are_also_set() {
    let cfg = LspConfig {
        python: LspServerOverride {
            command: Some("my-pyright".to_string()),
            args: Some(vec!["--stdio".to_string()]),
            enabled: false,
        },
        ..LspConfig::default()
    };
    let map = effective_lsp_commands(&cfg);
    assert!(!map.contains_key(&ServerLang::Python));
}
