//! Language-to-server mapping: which files get LSP support, and how to
//! launch the server for them.
//!
//! This intentionally does **not** reuse [`crate::highlight::Lang`]. The
//! two enums answer different questions and would make an awkward fit if
//! merged:
//!
//! - `highlight::Lang` is keyed by *grammar*: TSX and TypeScript are
//!   distinct tree-sitter grammars (and get distinct enum variants there)
//!   even though a single `typescript-language-server` process serves both;
//!   conversely JSX belongs to the JavaScript grammar but is also served by
//!   `typescript-language-server`. A grammar-shaped key doesn't line up
//!   1:1 with "which server process do I launch".
//! - Most of `highlight::Lang`'s variants (JSON, TOML, Markdown, Bash,
//!   YAML, ...) have no language server at all, so a server-oriented enum
//!   would need to grow `None`-server variants for every highlighting
//!   language, coupling this module's completeness to the highlighter's.
//!
//! So `lsp::config` keeps its own small extension-to-server table instead
//! of coupling to the highlighting module.

use std::collections::HashMap;
use std::path::Path;

/// A language for which a language server may be configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServerLang {
    Rust,
    TypeScript,
    Python,
    Go,
}

impl ServerLang {
    /// Detects the server language for a file from its extension,
    /// case-insensitively. Returns `None` for files with no extension or
    /// an extension with no configured server (e.g. `.md`, `.json`).
    pub fn from_path(path: &Path) -> Option<ServerLang> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "rs" => Some(ServerLang::Rust),
            "ts" | "mts" | "cts" | "tsx" | "js" | "mjs" | "cjs" | "jsx" => {
                Some(ServerLang::TypeScript)
            }
            "py" | "pyi" => Some(ServerLang::Python),
            "go" => Some(ServerLang::Go),
            _ => None,
        }
    }
}

/// How to launch a language server: the executable and its arguments.
///
/// v1 is a hardcoded default table; a later change may replace this with a
/// user-configurable table in the style of Helix's `languages.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LangServerCmd {
    pub command: String,
    pub args: Vec<String>,
}

impl LangServerCmd {
    fn new(command: &str, args: &[&str]) -> Self {
        Self {
            command: command.to_string(),
            args: args.iter().map(|a| a.to_string()).collect(),
        }
    }
}

/// Built-in default launch commands, one per [`ServerLang`]:
///
/// - Rust: `rust-analyzer`
/// - TypeScript: `typescript-language-server --stdio`
/// - Python: `pyright-langserver --stdio`
/// - Go: `gopls`
pub fn default_commands() -> HashMap<ServerLang, LangServerCmd> {
    let mut map = HashMap::new();
    map.insert(ServerLang::Rust, LangServerCmd::new("rust-analyzer", &[]));
    map.insert(
        ServerLang::TypeScript,
        LangServerCmd::new("typescript-language-server", &["--stdio"]),
    );
    map.insert(
        ServerLang::Python,
        LangServerCmd::new("pyright-langserver", &["--stdio"]),
    );
    map.insert(ServerLang::Go, LangServerCmd::new("gopls", &[]));
    map
}

/// The LSP `languageId` for a file, used in `textDocument/didOpen`.
///
/// Falls back to `"plaintext"` for extensions with no specific mapping;
/// callers should still gate on [`ServerLang::from_path`] before opening a
/// document with a server, since `"plaintext"` files have no server to send
/// it to.
pub(crate) fn language_id(path: &Path) -> &'static str {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return "plaintext";
    };
    match ext.to_ascii_lowercase().as_str() {
        "rs" => "rust",
        "py" | "pyi" => "python",
        "go" => "go",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "typescriptreact",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "javascriptreact",
        _ => "plaintext",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn path(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn from_path_maps_every_extension() {
        let cases = [
            ("main.rs", Some(ServerLang::Rust)),
            ("index.ts", Some(ServerLang::TypeScript)),
            ("index.mts", Some(ServerLang::TypeScript)),
            ("index.cts", Some(ServerLang::TypeScript)),
            ("component.tsx", Some(ServerLang::TypeScript)),
            ("script.js", Some(ServerLang::TypeScript)),
            ("script.mjs", Some(ServerLang::TypeScript)),
            ("script.cjs", Some(ServerLang::TypeScript)),
            ("component.jsx", Some(ServerLang::TypeScript)),
            ("app.py", Some(ServerLang::Python)),
            ("stub.pyi", Some(ServerLang::Python)),
            ("main.go", Some(ServerLang::Go)),
        ];
        for (file, expected) in cases {
            assert_eq!(ServerLang::from_path(&path(file)), expected, "{file}");
        }
    }

    #[test]
    fn from_path_is_case_insensitive() {
        assert_eq!(
            ServerLang::from_path(&path("main.RS")),
            Some(ServerLang::Rust)
        );
        assert_eq!(
            ServerLang::from_path(&path("Component.TSX")),
            Some(ServerLang::TypeScript)
        );
    }

    #[test]
    fn from_path_unknown_or_missing_extension_is_none() {
        assert_eq!(ServerLang::from_path(&path("README.md")), None);
        assert_eq!(ServerLang::from_path(&path("Makefile")), None);
        assert_eq!(ServerLang::from_path(&path("data.json")), None);
    }

    #[test]
    fn default_commands_has_exactly_four_languages() {
        let map = default_commands();
        assert_eq!(map.len(), 4);

        assert_eq!(
            map[&ServerLang::Rust],
            LangServerCmd::new("rust-analyzer", &[])
        );
        assert_eq!(
            map[&ServerLang::TypeScript],
            LangServerCmd::new("typescript-language-server", &["--stdio"])
        );
        assert_eq!(
            map[&ServerLang::Python],
            LangServerCmd::new("pyright-langserver", &["--stdio"])
        );
        assert_eq!(map[&ServerLang::Go], LangServerCmd::new("gopls", &[]));
    }

    #[test]
    fn language_id_spot_checks() {
        assert_eq!(language_id(&path("main.rs")), "rust");
        assert_eq!(language_id(&path("app.py")), "python");
        assert_eq!(language_id(&path("stub.pyi")), "python");
        assert_eq!(language_id(&path("main.go")), "go");
        assert_eq!(language_id(&path("index.ts")), "typescript");
        assert_eq!(language_id(&path("component.tsx")), "typescriptreact");
        assert_eq!(language_id(&path("script.js")), "javascript");
        assert_eq!(language_id(&path("component.jsx")), "javascriptreact");
        assert_eq!(language_id(&path("README.md")), "plaintext");
        assert_eq!(language_id(&path("Makefile")), "plaintext");
    }
}
