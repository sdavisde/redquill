//! The highlighting engine: owns per-language `HighlightConfiguration`s
//! (built lazily, cached forever) and turns a file's contents into
//! per-line token spans.

use std::collections::HashMap;
use std::ops::Range;

use tree_sitter_highlight::{Highlight, HighlightConfiguration, HighlightEvent};

use super::lang::Lang;
use super::token_kind::{TokenKind, kind_for_recognized_index, recognized_names};

/// Files larger than this are not highlighted at all: highlighting is a
/// progressive enhancement, and parsing a huge file once per selection
/// isn't worth the latency.
const MAX_CONTENT_BYTES: usize = 2 * 1024 * 1024;

/// Result of a lazy config build attempt, cached so we never retry (and
/// never rebuild a working config) more than once per language.
enum ConfigState {
    // Boxed: `HighlightConfiguration` is large (it embeds a compiled
    // `Query`), and `Failed` would otherwise force every `ConfigState`
    // (including the common case) to reserve that much space unused.
    Built(Box<HighlightConfiguration>),
    Failed,
}

impl ConfigState {
    fn config(&self) -> Option<&HighlightConfiguration> {
        match self {
            ConfigState::Built(config) => Some(config),
            ConfigState::Failed => None,
        }
    }
}

/// Owns the tree-sitter-highlight engine and the per-language
/// configuration cache. Cheap to construct; expensive work (building a
/// language's `HighlightConfiguration`) happens lazily on first use of
/// that language and is never repeated.
pub struct Highlighter {
    engine: tree_sitter_highlight::Highlighter,
    configs: HashMap<Lang, ConfigState>,
    markdown_inline: Option<ConfigState>,
}

impl Default for Highlighter {
    fn default() -> Self {
        Self::new()
    }
}

impl Highlighter {
    /// Create a new engine with no languages loaded yet.
    pub fn new() -> Self {
        Self {
            engine: tree_sitter_highlight::Highlighter::new(),
            configs: HashMap::new(),
            markdown_inline: None,
        }
    }

    /// Highlight `content` (the full text of one file) as `lang`, returning
    /// one entry per line (split on `'\n'`, matching `str::lines`-plus-a-
    /// trailing-empty-line-if-content-ends-in-`\n` semantics of a raw
    /// split). Each line's entry is a list of non-overlapping byte-range
    /// spans *relative to that line's start*, in order.
    ///
    /// This never panics and never returns an error: a language whose
    /// config fails to build, content over the size guard, or an internal
    /// tree-sitter error all degrade to empty spans (per line, or an empty
    /// outer `Vec` for empty content) rather than surfacing a failure to
    /// the caller.
    pub fn highlight_lines(
        &mut self,
        lang: Lang,
        content: &str,
    ) -> Vec<Vec<(Range<usize>, TokenKind)>> {
        if content.is_empty() {
            return Vec::new();
        }

        if content.len() > MAX_CONTENT_BYTES {
            return vec![Vec::new(); line_count(content)];
        }

        let line_ranges = line_ranges(content);
        let line_count = line_ranges.len();

        self.ensure_lang(lang);
        if lang == Lang::Markdown {
            self.ensure_markdown_inline();
        }

        let Some(config) = self.configs.get(&lang).and_then(ConfigState::config) else {
            return vec![Vec::new(); line_count];
        };
        let markdown_inline = self.markdown_inline.as_ref().and_then(ConfigState::config);

        let injection_callback = move |name: &str| -> Option<&HighlightConfiguration> {
            // Only the markdown block -> inline injection is wired up today;
            // other injected languages (fenced code blocks, YAML/TOML
            // frontmatter, embedded HTML) simply render unhighlighted
            // within their region, which is a safe, silent degradation.
            if name == "markdown_inline" {
                markdown_inline
            } else {
                None
            }
        };

        let events =
            match self
                .engine
                .highlight(config, content.as_bytes(), None, injection_callback)
            {
                Ok(events) => events,
                Err(_) => return vec![Vec::new(); line_count],
            };

        let mut lines_out: Vec<Vec<(Range<usize>, TokenKind)>> = vec![Vec::new(); line_count];
        let mut stack: Vec<TokenKind> = Vec::new();
        let mut cursor = 0usize;

        for event in events {
            match event {
                Ok(HighlightEvent::Source { start, end }) => {
                    if let Some(&kind) = stack.last() {
                        emit_span(&line_ranges, &mut lines_out, &mut cursor, start, end, kind);
                    }
                }
                Ok(HighlightEvent::HighlightStart(Highlight(index))) => {
                    stack.push(kind_for_recognized_index(index).unwrap_or(TokenKind::Other));
                }
                Ok(HighlightEvent::HighlightEnd) => {
                    stack.pop();
                }
                Err(_) => return vec![Vec::new(); line_count],
            }
        }

        lines_out
    }

    /// Build (if not already attempted) and cache the config for `lang`.
    fn ensure_lang(&mut self, lang: Lang) {
        self.configs
            .entry(lang)
            .or_insert_with(|| match build_config(lang) {
                Some(config) => ConfigState::Built(Box::new(config)),
                None => ConfigState::Failed,
            });
    }

    /// Build (if not already attempted) and cache the markdown inline
    /// grammar's config, used only via injection from markdown blocks.
    fn ensure_markdown_inline(&mut self) {
        if self.markdown_inline.is_none() {
            self.markdown_inline = Some(match build_markdown_inline_config() {
                Some(config) => ConfigState::Built(Box::new(config)),
                None => ConfigState::Failed,
            });
        }
    }
}

/// Split `line_ranges`-relative work: clip `[start, end)` (byte offsets
/// into the whole file) to each line it overlaps, excluding the newline
/// byte itself, and push the clipped, line-relative span onto that line's
/// output vec. `cursor` tracks the last line touched and only ever moves
/// forward, since callers invoke this with non-decreasing `start` values.
fn emit_span(
    line_ranges: &[Range<usize>],
    lines_out: &mut [Vec<(Range<usize>, TokenKind)>],
    cursor: &mut usize,
    mut start: usize,
    end: usize,
    kind: TokenKind,
) {
    if start >= end {
        return;
    }
    while *cursor < line_ranges.len() {
        let line_range = line_ranges[*cursor].clone();
        if end <= line_range.start {
            break;
        }
        if start >= line_range.end {
            *cursor += 1;
            continue;
        }

        let clip_start = start.max(line_range.start);
        let clip_end = end.min(line_range.end);
        if clip_start < clip_end {
            lines_out[*cursor].push((
                clip_start - line_range.start..clip_end - line_range.start,
                kind,
            ));
        }

        if end <= line_range.end {
            break;
        }
        start = line_range.end;
        *cursor += 1;
    }
}

/// Byte ranges of each line in `content`, split on `'\n'` and excluding
/// the newline byte itself (mirrors `str::split('\n')` boundaries).
fn line_ranges(content: &str) -> Vec<Range<usize>> {
    let bytes = content.as_bytes();
    let mut ranges = Vec::with_capacity(bytes.iter().filter(|&&b| b == b'\n').count() + 1);
    let mut start = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            ranges.push(start..i);
            start = i + 1;
        }
    }
    ranges.push(start..bytes.len());
    ranges
}

/// Cheap line count (no allocation) for the size-guard rejection path.
fn line_count(content: &str) -> usize {
    content.bytes().filter(|&b| b == b'\n').count() + 1
}

/// Build the `HighlightConfiguration` for a top-level supported language.
/// Returns `None` if the grammar/query combination fails to construct
/// (malformed query, ABI mismatch, etc.) so the caller can silently
/// degrade that one language rather than surfacing an error.
fn build_config(lang: Lang) -> Option<HighlightConfiguration> {
    let mut config = match lang {
        Lang::Rust => HighlightConfiguration::new(
            tree_sitter_rust::LANGUAGE.into(),
            lang.name(),
            tree_sitter_rust::HIGHLIGHTS_QUERY,
            tree_sitter_rust::INJECTIONS_QUERY,
            "",
        ),
        Lang::Python => HighlightConfiguration::new(
            tree_sitter_python::LANGUAGE.into(),
            lang.name(),
            tree_sitter_python::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        Lang::JavaScript => {
            let highlights = format!(
                "{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY
            );
            HighlightConfiguration::new(
                tree_sitter_javascript::LANGUAGE.into(),
                lang.name(),
                &highlights,
                tree_sitter_javascript::INJECTIONS_QUERY,
                tree_sitter_javascript::LOCALS_QUERY,
            )
        }
        // The `tree-sitter-typescript` grammar's own `highlights.scm` only
        // covers TS-specific additions (interfaces, `keyof`, etc.); the
        // node types it shares with JS (functions, `const`, control flow,
        // ...) are only highlighted by layering the JS query underneath,
        // same as nvim-treesitter/helix do.
        Lang::TypeScript => {
            let highlights = format!(
                "{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_typescript::HIGHLIGHTS_QUERY
            );
            HighlightConfiguration::new(
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                lang.name(),
                &highlights,
                "",
                tree_sitter_typescript::LOCALS_QUERY,
            )
        }
        Lang::Tsx => {
            let highlights = format!(
                "{}\n{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
                tree_sitter_typescript::HIGHLIGHTS_QUERY
            );
            HighlightConfiguration::new(
                tree_sitter_typescript::LANGUAGE_TSX.into(),
                lang.name(),
                &highlights,
                "",
                tree_sitter_typescript::LOCALS_QUERY,
            )
        }
        Lang::Go => HighlightConfiguration::new(
            tree_sitter_go::LANGUAGE.into(),
            lang.name(),
            tree_sitter_go::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        Lang::Json => HighlightConfiguration::new(
            tree_sitter_json::LANGUAGE.into(),
            lang.name(),
            JSON_HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        Lang::Toml => HighlightConfiguration::new(
            tree_sitter_toml_ng::LANGUAGE.into(),
            lang.name(),
            tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        Lang::Markdown => HighlightConfiguration::new(
            tree_sitter_md::LANGUAGE.into(),
            lang.name(),
            tree_sitter_md::HIGHLIGHT_QUERY_BLOCK,
            MARKDOWN_BLOCK_INJECTIONS,
            "",
        ),
        Lang::Bash => HighlightConfiguration::new(
            tree_sitter_bash::LANGUAGE.into(),
            lang.name(),
            tree_sitter_bash::HIGHLIGHT_QUERY,
            "",
            "",
        ),
        Lang::Yaml => HighlightConfiguration::new(
            tree_sitter_yaml::LANGUAGE.into(),
            lang.name(),
            tree_sitter_yaml::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
    }
    .ok()?;
    config.configure(&recognized_names());
    Some(config)
}

/// `tree-sitter-json`'s own shipped `highlights.scm` captures object keys
/// with the more specific `@string.special.key` pattern *before* the
/// catch-all `(string) @string` pattern that also matches key nodes (every
/// key is structurally a string node too). `tree-sitter-highlight` resolves
/// multiple captures on the same node by taking the *last* matching
/// pattern, so upstream's ordering means keys always render as plain
/// strings, never as `string.special.key`. Same query, reordered so the
/// specific pattern comes last and wins.
const JSON_HIGHLIGHTS_QUERY: &str = r#"
(string) @string

(number) @number

[
  (null)
  (true)
  (false)
] @constant.builtin

(escape_sequence) @escape

(comment) @comment

(pair
  key: (_) @string.special.key)
"#;

/// `tree-sitter-md`'s own shipped `injections.scm` marks `(inline)` nodes
/// for injection into the `markdown_inline` grammar, but doesn't set
/// `injection.include-children`. The block grammar's `(inline)` node
/// *does* have child tokens (the literal `*`/`_`/backtick delimiter
/// bytes), and `tree-sitter-highlight` excludes a content node's children
/// from the injected byte ranges by default — so without this override,
/// exactly the delimiter bytes the inline grammar needs to recognize
/// `**bold**`/`` `code` ``/etc. get cut out of the reparse, and inline
/// formatting silently fails to highlight. Same query as upstream, plus
/// the one predicate needed to include those children.
const MARKDOWN_BLOCK_INJECTIONS: &str = r#"
(fenced_code_block
  (info_string
    (language) @injection.language)
  (code_fence_content) @injection.content)

((html_block) @injection.content
  (#set! injection.language "html"))

((minus_metadata) @injection.content
  (#set! injection.language "yaml"))

((plus_metadata) @injection.content
  (#set! injection.language "toml"))

((inline) @injection.content
  (#set! injection.language "markdown_inline")
  (#set! injection.include-children))
"#;

/// Build the markdown *inline* grammar's config (headings/emphasis/code
/// spans/links live here, injected into `(inline)` nodes from the block
/// grammar). Not a top-level [`Lang`] variant since it's never selected
/// directly by file extension.
fn build_markdown_inline_config() -> Option<HighlightConfiguration> {
    let mut config = HighlightConfiguration::new(
        tree_sitter_md::INLINE_LANGUAGE.into(),
        "markdown_inline",
        tree_sitter_md::HIGHLIGHT_QUERY_INLINE,
        tree_sitter_md::INJECTION_QUERY_INLINE,
        "",
    )
    .ok()?;
    config.configure(&recognized_names());
    Some(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_text(content: &str, line_idx: usize) -> &str {
        content.split('\n').nth(line_idx).expect("line exists")
    }

    /// Assert that some span on `line_idx` covers exactly `expected_text`
    /// (by byte content, resolved against the un-highlighted source line)
    /// and is tagged with `kind`.
    fn assert_span(
        lines: &[Vec<(Range<usize>, TokenKind)>],
        content: &str,
        line_idx: usize,
        expected_text: &str,
        kind: TokenKind,
    ) {
        let text = line_text(content, line_idx);
        let found = lines[line_idx]
            .iter()
            .any(|(range, k)| *k == kind && text.get(range.clone()) == Some(expected_text));
        assert!(
            found,
            "line {line_idx} ({text:?}) missing span {expected_text:?} as {kind:?}; spans: {:?}",
            lines[line_idx]
                .iter()
                .map(|(r, k)| (text.get(r.clone()), k))
                .collect::<Vec<_>>()
        );
    }

    fn assert_line_has_kind(
        lines: &[Vec<(Range<usize>, TokenKind)>],
        line_idx: usize,
        kind: TokenKind,
    ) {
        assert!(
            lines[line_idx].iter().any(|(_, k)| *k == kind),
            "line {line_idx} missing any {kind:?} span; spans: {:?}",
            lines[line_idx]
        );
    }

    #[test]
    fn every_supported_lang_builds_a_config() {
        for &lang in Lang::ALL {
            let config = build_config(lang);
            assert!(config.is_some(), "{lang:?} failed to build a config");
        }
        assert!(
            build_markdown_inline_config().is_some(),
            "markdown inline grammar failed to build a config"
        );
    }

    #[test]
    fn empty_content_returns_empty_vec() {
        let mut hl = Highlighter::new();
        assert_eq!(hl.highlight_lines(Lang::Rust, ""), Vec::<Vec<_>>::new());
    }

    #[test]
    fn oversized_content_is_guarded() {
        let mut hl = Highlighter::new();
        let content = "a".repeat(MAX_CONTENT_BYTES + 1);
        let result = hl.highlight_lines(Lang::Rust, &content);
        assert_eq!(result.len(), 1);
        assert!(result[0].is_empty());
    }

    #[test]
    fn rust_keyword_string_comment() {
        let content = "fn main() {\n    let s = \"hi\";\n    // a comment\n}\n";
        let mut hl = Highlighter::new();
        let lines = hl.highlight_lines(Lang::Rust, content);
        assert_eq!(lines.len(), content.split('\n').count());
        assert_span(&lines, content, 0, "fn", TokenKind::Keyword);
        assert_span(&lines, content, 1, "\"hi\"", TokenKind::String);
        assert_span(&lines, content, 2, "// a comment", TokenKind::Comment);
    }

    #[test]
    fn rust_raw_string_spans_every_line_it_covers() {
        let content = "let s = r#\"line one\nline two\nline three\"#;\n";
        let mut hl = Highlighter::new();
        let lines = hl.highlight_lines(Lang::Rust, content);
        assert_line_has_kind(&lines, 0, TokenKind::String);
        assert_line_has_kind(&lines, 1, TokenKind::String);
        assert_line_has_kind(&lines, 2, TokenKind::String);
    }

    #[test]
    fn rust_block_comment_spans_every_line_it_covers() {
        let content = "/* first\nsecond\nthird */\nfn main() {}\n";
        let mut hl = Highlighter::new();
        let lines = hl.highlight_lines(Lang::Rust, content);
        assert_line_has_kind(&lines, 0, TokenKind::Comment);
        assert_line_has_kind(&lines, 1, TokenKind::Comment);
        assert_line_has_kind(&lines, 2, TokenKind::Comment);
    }

    #[test]
    fn python_keyword_string_comment() {
        let content = "def f():\n    s = \"hi\"\n    # a comment\n";
        let mut hl = Highlighter::new();
        let lines = hl.highlight_lines(Lang::Python, content);
        assert_span(&lines, content, 0, "def", TokenKind::Keyword);
        assert_span(&lines, content, 1, "\"hi\"", TokenKind::String);
        assert_span(&lines, content, 2, "# a comment", TokenKind::Comment);
    }

    #[test]
    fn javascript_keyword_string_comment() {
        let content = "function f() {\n  const s = \"hi\";\n  // a comment\n}\n";
        let mut hl = Highlighter::new();
        let lines = hl.highlight_lines(Lang::JavaScript, content);
        assert_span(&lines, content, 0, "function", TokenKind::Keyword);
        assert_span(&lines, content, 1, "\"hi\"", TokenKind::String);
        assert_span(&lines, content, 2, "// a comment", TokenKind::Comment);
    }

    #[test]
    fn typescript_keyword_and_type() {
        let content = "function f(x: number): number {\n  return x;\n}\n";
        let mut hl = Highlighter::new();
        let lines = hl.highlight_lines(Lang::TypeScript, content);
        assert_span(&lines, content, 0, "function", TokenKind::Keyword);
        assert_span(&lines, content, 0, "number", TokenKind::Type);
    }

    #[test]
    fn tsx_builds_and_highlights() {
        let content = "const el = <div>hi</div>;\n";
        let mut hl = Highlighter::new();
        let lines = hl.highlight_lines(Lang::Tsx, content);
        assert_span(&lines, content, 0, "const", TokenKind::Keyword);
    }

    #[test]
    fn go_keyword_and_string() {
        let content = "package main\n\nfunc main() {\n\ts := \"hi\"\n\t_ = s\n}\n";
        let mut hl = Highlighter::new();
        let lines = hl.highlight_lines(Lang::Go, content);
        assert_span(&lines, content, 0, "package", TokenKind::Keyword);
        assert_span(&lines, content, 3, "\"hi\"", TokenKind::String);
    }

    #[test]
    fn json_string_and_number() {
        let content = "{\n  \"a\": 1\n}\n";
        let mut hl = Highlighter::new();
        let lines = hl.highlight_lines(Lang::Json, content);
        assert_span(&lines, content, 1, "\"a\"", TokenKind::Property);
        assert_span(&lines, content, 1, "1", TokenKind::Number);
    }

    #[test]
    fn toml_string_value() {
        let content = "name = \"redquill\"\n";
        let mut hl = Highlighter::new();
        let lines = hl.highlight_lines(Lang::Toml, content);
        assert_span(&lines, content, 0, "\"redquill\"", TokenKind::String);
    }

    #[test]
    fn bash_keyword_and_string() {
        let content = "if true; then\n  echo \"hi\"\nfi\n";
        let mut hl = Highlighter::new();
        let lines = hl.highlight_lines(Lang::Bash, content);
        assert_span(&lines, content, 0, "if", TokenKind::Keyword);
        assert_line_has_kind(&lines, 1, TokenKind::String);
    }

    #[test]
    fn yaml_string_value() {
        let content = "name: \"redquill\"\n";
        let mut hl = Highlighter::new();
        let lines = hl.highlight_lines(Lang::Yaml, content);
        assert_line_has_kind(&lines, 0, TokenKind::String);
    }

    #[test]
    fn markdown_heading_and_inline_emphasis() {
        let content = "# Title\n\nSome **bold** text.\n";
        let mut hl = Highlighter::new();
        let lines = hl.highlight_lines(Lang::Markdown, content);
        assert_line_has_kind(&lines, 0, TokenKind::Keyword);
        assert_line_has_kind(&lines, 2, TokenKind::Attribute);
    }

    #[test]
    fn spans_within_a_line_are_ordered_and_non_overlapping() {
        let content = "let x = 1 + 2; // sum\n";
        let mut hl = Highlighter::new();
        let lines = hl.highlight_lines(Lang::Rust, content);
        let spans = &lines[0];
        for pair in spans.windows(2) {
            assert!(
                pair[0].0.end <= pair[1].0.start,
                "spans overlap or are out of order: {:?}",
                spans
            );
        }
    }

    #[test]
    fn unrecognized_capture_never_panics_and_lang_is_independent_of_cache() {
        // Building the same language twice must reuse the cached config,
        // not rebuild (and must keep working).
        let mut hl = Highlighter::new();
        let content = "fn main() {}\n";
        let first = hl.highlight_lines(Lang::Rust, content);
        let second = hl.highlight_lines(Lang::Rust, content);
        assert_eq!(first, second);
    }
}
