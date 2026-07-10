//! Supported languages and extension-based detection.

/// A language the highlighting engine knows how to parse.
///
/// `Tsx` is kept distinct from `TypeScript` because they're genuinely
/// different tree-sitter grammars (`tree-sitter-typescript` ships both);
/// `JavaScript` covers `.jsx` too since the JS grammar already parses JSX
/// syntax and there's no separate grammar for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Tsx,
    Go,
    Json,
    Toml,
    Markdown,
    Bash,
    Yaml,
}

impl Lang {
    /// Every supported language, for iteration (e.g. warming caches, tests).
    pub const ALL: &'static [Lang] = &[
        Lang::Rust,
        Lang::Python,
        Lang::JavaScript,
        Lang::TypeScript,
        Lang::Tsx,
        Lang::Go,
        Lang::Json,
        Lang::Toml,
        Lang::Markdown,
        Lang::Bash,
        Lang::Yaml,
    ];

    /// Detect a language from a file path by its extension.
    ///
    /// Matching is case-insensitive and looks only at the extension, not
    /// the full filename, so `main.rs`, `Foo.RS`, and `/a/b/c.rs` all
    /// resolve to [`Lang::Rust`]. Returns `None` for unrecognized or
    /// missing extensions.
    pub fn from_path(path: &str) -> Option<Lang> {
        let ext = std::path::Path::new(path).extension()?.to_str()?;
        let ext = ext.to_ascii_lowercase();
        match ext.as_str() {
            "rs" => Some(Lang::Rust),
            "py" => Some(Lang::Python),
            "js" | "mjs" | "cjs" | "jsx" => Some(Lang::JavaScript),
            "ts" | "mts" | "cts" => Some(Lang::TypeScript),
            "tsx" => Some(Lang::Tsx),
            "go" => Some(Lang::Go),
            "json" => Some(Lang::Json),
            "toml" => Some(Lang::Toml),
            "md" => Some(Lang::Markdown),
            "sh" | "bash" | "zsh" => Some(Lang::Bash),
            "yml" | "yaml" => Some(Lang::Yaml),
            _ => None,
        }
    }

    /// A short, stable name for this language (used as the
    /// `tree-sitter-highlight` config name, and matched against
    /// `#[set! injection.language ...]` query hints for cross-language
    /// injection).
    pub(crate) fn name(self) -> &'static str {
        match self {
            Lang::Rust => "rust",
            Lang::Python => "python",
            Lang::JavaScript => "javascript",
            Lang::TypeScript => "typescript",
            Lang::Tsx => "tsx",
            Lang::Go => "go",
            Lang::Json => "json",
            Lang::Toml => "toml",
            Lang::Markdown => "markdown",
            Lang::Bash => "bash",
            Lang::Yaml => "yaml",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_every_supported_extension() {
        let cases: &[(&str, Lang)] = &[
            ("main.rs", Lang::Rust),
            ("script.py", Lang::Python),
            ("index.js", Lang::JavaScript),
            ("mod.mjs", Lang::JavaScript),
            ("mod.cjs", Lang::JavaScript),
            ("component.jsx", Lang::JavaScript),
            ("index.ts", Lang::TypeScript),
            ("mod.mts", Lang::TypeScript),
            ("mod.cts", Lang::TypeScript),
            ("component.tsx", Lang::Tsx),
            ("main.go", Lang::Go),
            ("data.json", Lang::Json),
            ("Cargo.toml", Lang::Toml),
            ("README.md", Lang::Markdown),
            ("run.sh", Lang::Bash),
            ("run.bash", Lang::Bash),
            ("run.zsh", Lang::Bash),
            ("config.yml", Lang::Yaml),
            ("config.yaml", Lang::Yaml),
        ];
        for (path, expected) in cases {
            assert_eq!(Lang::from_path(path), Some(*expected), "path: {path}");
        }
    }

    #[test]
    fn extension_matching_is_case_insensitive() {
        assert_eq!(Lang::from_path("MAIN.RS"), Some(Lang::Rust));
        assert_eq!(Lang::from_path("Component.TSX"), Some(Lang::Tsx));
    }

    #[test]
    fn unknown_or_missing_extension_is_none() {
        assert_eq!(Lang::from_path("README"), None);
        assert_eq!(Lang::from_path("binary.exe"), None);
        assert_eq!(Lang::from_path("archive.tar.gz"), None);
        assert_eq!(Lang::from_path(""), None);
        assert_eq!(Lang::from_path("no_extension_at_all"), None);
    }

    #[test]
    fn all_contains_every_variant_exactly_once() {
        // Guards against a variant being added without updating ALL.
        assert_eq!(Lang::ALL.len(), 11);
        let mut seen = std::collections::HashSet::new();
        for lang in Lang::ALL {
            assert!(seen.insert(format!("{lang:?}")), "duplicate: {lang:?}");
        }
    }
}
