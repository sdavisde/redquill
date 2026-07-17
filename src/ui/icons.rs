//! Nerd Font glyphs for the git panel's file tree: a folder glyph (open or
//! closed) for directory rows and a file-type glyph keyed off the extension
//! for file rows. These codepoints live in the Nerd Font private-use range,
//! so they render as icons only in a terminal using a patched Nerd Font;
//! elsewhere they fall back to the font's tofu box. That trade-off is
//! deliberate (see the panel's chosen icon style) — the glyph is decoration,
//! never the sole carrier of meaning (change kind is also color-coded and
//! spelled out as a status letter).
//!
//! Pure lookup tables, unit-tested on their own; no TUI or theme types here.

/// The folder glyph for a directory row: open when expanded, closed when
/// collapsed (nf-fa-folder / nf-fa-folder_open).
pub(super) fn dir_icon(collapsed: bool) -> &'static str {
    if collapsed { "\u{f07b}" } else { "\u{f07c}" }
}

/// The collapse chevron shown before a directory's folder glyph, so the
/// fold state reads even where the folder glyphs themselves don't render.
pub(super) fn chevron(collapsed: bool) -> &'static str {
    if collapsed { "\u{25b8}" } else { "\u{25be}" }
}

/// The file-type glyph for `name`, keyed off its extension with a generic
/// file glyph as the fallback.
pub(super) fn file_icon(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match ext.to_ascii_lowercase().as_str() {
        "rs" => "\u{e7a8}",                                             // nf-dev-rust
        "md" | "markdown" => "\u{f48a}",                                // nf-oct-markdown
        "json" => "\u{e60b}",                                           // nf-seti-json
        "toml" | "yaml" | "yml" | "ini" | "cfg" | "conf" => "\u{e615}", // nf-seti-config
        "js" | "mjs" | "cjs" => "\u{e74e}",                             // nf-dev-javascript
        "ts" | "tsx" | "jsx" => "\u{e628}",                             // nf-seti-typescript
        "py" => "\u{e73c}",                                             // nf-dev-python
        "go" => "\u{e627}",                                             // nf-seti-go
        "sh" | "bash" | "zsh" => "\u{f489}",                            // nf-oct-terminal
        "html" | "htm" => "\u{e736}",                                   // nf-dev-html5
        "css" | "scss" => "\u{e749}",                                   // nf-dev-css3
        "lock" => "\u{f023}",                                           // nf-fa-lock
        _ => "\u{f15b}",                                                // nf-fa-file
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_icon_reflects_collapse_state() {
        assert_ne!(dir_icon(true), dir_icon(false));
    }

    #[test]
    fn chevron_points_right_when_collapsed_down_when_open() {
        assert_eq!(chevron(true), "\u{25b8}");
        assert_eq!(chevron(false), "\u{25be}");
    }

    #[test]
    fn known_extensions_map_to_distinct_glyphs() {
        assert_eq!(file_icon("main.rs"), "\u{e7a8}");
        assert_eq!(file_icon("README.md"), "\u{f48a}");
        assert_eq!(file_icon("Cargo.toml"), "\u{e615}");
    }

    #[test]
    fn extension_lookup_is_case_insensitive() {
        assert_eq!(file_icon("MAIN.RS"), file_icon("main.rs"));
    }

    #[test]
    fn unknown_or_missing_extension_falls_back_to_the_generic_file_glyph() {
        assert_eq!(file_icon("Makefile"), "\u{f15b}");
        assert_eq!(file_icon("noext"), "\u{f15b}");
    }
}
