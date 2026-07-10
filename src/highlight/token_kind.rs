//! Semantic token palette the UI maps to colors, and the table that maps
//! `tree-sitter-highlight` capture names (e.g. `"function.method"`,
//! `"type.builtin"`) onto that palette.

/// A small semantic palette that the diff renderer can map onto colors.
///
/// This intentionally collapses the much larger set of tree-sitter capture
/// names (see the `highlights.scm` query files shipped by each grammar)
/// into a handful of buckets a theme can reasonably color distinctly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    /// Language keywords (`fn`, `if`, `return`, ...).
    Keyword,
    /// Function and method names, at definition or call sites.
    Function,
    /// Type names, including builtin types.
    Type,
    /// String literals, including their escape sequences.
    String,
    /// Numeric literals.
    Number,
    /// Comments, including doc comments.
    Comment,
    /// Constants, including builtin constants and booleans.
    Constant,
    /// Object/struct properties and fields.
    Property,
    /// Operators (`+`, `->`, `=>`, ...).
    Operator,
    /// Punctuation: brackets, delimiters, and other structural marks.
    Punctuation,
    /// Variable names, including parameters and builtins.
    Variable,
    /// Attributes/decorators (`#[derive(...)]`, `@decorator`, ...).
    Attribute,
    /// Content embedded in a larger construct (e.g. `${ ... }` interpolation).
    Embedded,
    /// Anything recognized by a grammar's query but with no better home in
    /// this palette. Never a parse failure — just an unopinionated token.
    Other,
}

/// Maps a raw capture name (as it appears in a `highlights.scm` query, dot
/// separated, without the leading `@`) to a [`TokenKind`] by longest
/// dot-segment-prefix match.
///
/// For example `"function.method.builtin"` matches `"function.method"`
/// before falling back to `"function"` if a more specific entry isn't
/// present in the table. Names with no match at all (including the
/// `tree-sitter-highlight`-internal `"none"` capture some queries use to
/// explicitly opt out of highlighting) resolve to [`TokenKind::Other`].
pub fn capture_name_to_kind(name: &str) -> TokenKind {
    let mut candidate = name;
    loop {
        if let Some((_, kind)) = CAPTURE_KIND_TABLE.iter().find(|(key, _)| *key == candidate) {
            return *kind;
        }
        match candidate.rfind('.') {
            Some(idx) => candidate = &candidate[..idx],
            None => return TokenKind::Other,
        }
    }
}

/// The full list of capture names this crate recognizes, in the order
/// passed to `tree-sitter-highlight`'s `HighlightConfiguration::configure`.
///
/// Order matters only as a tie-breaker between equally-specific matches;
/// entries are grouped by [`TokenKind`] with the most specific dotted names
/// first within each capture family.
pub(crate) fn recognized_names() -> Vec<&'static str> {
    CAPTURE_KIND_TABLE.iter().map(|(name, _)| *name).collect()
}

/// Resolve a `tree-sitter-highlight` `Highlight` index (an index into the
/// slice returned by [`recognized_names`]) directly to its [`TokenKind`].
pub(crate) fn kind_for_recognized_index(index: usize) -> Option<TokenKind> {
    CAPTURE_KIND_TABLE.get(index).map(|(_, kind)| *kind)
}

/// Table of every capture name observed across the bundled grammars'
/// `highlights.scm` queries (rust, python, javascript, typescript, go,
/// json, toml, bash, yaml, markdown block + inline), plus a few generic
/// dotted roots for forward compatibility with grammars/captures not yet
/// seen. Ordered most-specific-first within each family.
const CAPTURE_KIND_TABLE: &[(&str, TokenKind)] = &[
    // Functions
    ("function.method", TokenKind::Function),
    ("function.builtin", TokenKind::Function),
    ("function.macro", TokenKind::Function),
    ("function", TokenKind::Function),
    // Types
    ("type.builtin", TokenKind::Type),
    ("type", TokenKind::Type),
    ("constructor", TokenKind::Type),
    // Strings
    ("string.special", TokenKind::String),
    ("string.escape", TokenKind::String),
    ("string", TokenKind::String),
    ("text.literal", TokenKind::String),
    ("text.uri", TokenKind::String),
    ("escape", TokenKind::String),
    // Numbers
    ("number", TokenKind::Number),
    // Comments
    ("comment.documentation", TokenKind::Comment),
    ("comment", TokenKind::Comment),
    // Constants
    ("constant.builtin", TokenKind::Constant),
    ("constant", TokenKind::Constant),
    ("boolean", TokenKind::Constant),
    // Properties
    ("string.special.key", TokenKind::Property),
    ("property", TokenKind::Property),
    ("text.reference", TokenKind::Property),
    // Operators
    ("operator", TokenKind::Operator),
    // Punctuation
    ("punctuation.bracket", TokenKind::Punctuation),
    ("punctuation.delimiter", TokenKind::Punctuation),
    ("punctuation.special", TokenKind::Punctuation),
    ("punctuation", TokenKind::Punctuation),
    // Variables
    ("variable.builtin", TokenKind::Variable),
    ("variable.parameter", TokenKind::Variable),
    ("variable", TokenKind::Variable),
    // Attributes / markdown emphasis (closest available bucket)
    ("attribute", TokenKind::Attribute),
    ("text.strong", TokenKind::Attribute),
    ("text.emphasis", TokenKind::Attribute),
    // Embedded content
    ("embedded", TokenKind::Embedded),
    // Keywords / markdown headings (closest available bucket)
    ("keyword", TokenKind::Keyword),
    ("text.title", TokenKind::Keyword),
    // Explicitly-uncategorized captures
    ("label", TokenKind::Other),
    ("none", TokenKind::Other),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_matches_resolve_directly() {
        assert_eq!(capture_name_to_kind("keyword"), TokenKind::Keyword);
        assert_eq!(capture_name_to_kind("string"), TokenKind::String);
        assert_eq!(capture_name_to_kind("comment"), TokenKind::Comment);
        assert_eq!(capture_name_to_kind("embedded"), TokenKind::Embedded);
    }

    #[test]
    fn longest_prefix_wins_over_shorter_family_match() {
        assert_eq!(capture_name_to_kind("function.method"), TokenKind::Function);
        assert_eq!(capture_name_to_kind("type.builtin"), TokenKind::Type);
        // "string.special.key" is deliberately bucketed as Property (an
        // object/map key), taking priority over the shorter "string" match.
        assert_eq!(
            capture_name_to_kind("string.special.key"),
            TokenKind::Property
        );
    }

    #[test]
    fn unknown_deeper_suffix_falls_back_to_known_prefix() {
        // Not a literal table entry, but should walk up to "function".
        assert_eq!(
            capture_name_to_kind("function.method.builtin.extra"),
            TokenKind::Function
        );
        // Not a literal table entry either; walks up to "type.builtin".
        assert_eq!(capture_name_to_kind("type.builtin.enum"), TokenKind::Type);
    }

    #[test]
    fn completely_unknown_name_is_other() {
        assert_eq!(
            capture_name_to_kind("totally.unknown.capture"),
            TokenKind::Other
        );
        assert_eq!(capture_name_to_kind("none"), TokenKind::Other);
        assert_eq!(capture_name_to_kind("label"), TokenKind::Other);
    }

    #[test]
    fn recognized_names_and_table_stay_in_sync() {
        let names = recognized_names();
        assert_eq!(names.len(), CAPTURE_KIND_TABLE.len());
        for (i, (name, kind)) in CAPTURE_KIND_TABLE.iter().enumerate() {
            assert_eq!(names[i], *name);
            assert_eq!(kind_for_recognized_index(i), Some(*kind));
        }
    }
}
