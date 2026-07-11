//! The color palette every renderer routes through: syntax-token colors,
//! diff line-background tints, word-diff/search-match highlight
//! backgrounds, and general UI chrome. One [`Theme::default`] instance
//! exists today; a future config layer would just construct a different
//! `Theme` and hand it to the same rendering code.

use ratatui::style::Color;

use crate::diff::{FileChangeKind, LineOrigin};
use crate::highlight::TokenKind;

/// The full color palette the sidebar, diff pane, panels, and help overlay
/// render through. Every color used by those widgets should live here
/// rather than as an inline `Color::X` literal, so a future theme just
/// swaps the instance.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    // -- Syntax, by TokenKind --
    pub keyword: Color,
    pub function: Color,
    pub type_: Color,
    pub string: Color,
    pub number: Color,
    pub comment: Color,
    pub constant: Color,
    pub property: Color,
    pub operator: Color,
    pub punctuation: Color,
    pub variable: Color,
    pub attribute: Color,
    pub embedded: Color,
    pub other: Color,

    // -- Diff line tints (backgrounds) and fallback foregrounds --
    pub added_bg: Color,
    pub removed_bg: Color,
    pub word_diff_bg: Color,
    pub added_fg: Color,
    pub removed_fg: Color,
    pub context_fg: Color,

    // -- Chrome shared across the diff pane, sidebar, and panels --
    pub gutter: Color,
    pub dot_marker: Color,
    pub selected_row_bg: Color,
    pub search_match_bg: Color,
    pub search_prompt: Color,
    pub annotation_text: Color,
    pub hunk_header: Color,
    pub binary_placeholder: Color,
    pub status_message: Color,
    /// The column cursor's cell highlight (diff pane) and the target
    /// line's highlight in the LSP peek overlay's preview pane.
    pub column_cursor_bg: Color,

    // -- Change-kind letters (sidebar file list + staging panel) --
    pub kind_added: Color,
    pub kind_deleted: Color,
    pub kind_modified: Color,
    pub kind_renamed: Color,
    pub kind_untracked: Color,

    // -- Sidebar --
    pub staged_indicator: Color,
    pub dir_prefix: Color,
    pub footer_text: Color,
    /// The border color of whichever pane currently holds focus (diff pane
    /// by default, the git panel while it is focused).
    pub focused_border: Color,

    // -- Help overlay --
    pub help_section_header: Color,
    pub help_key: Color,

    // -- Annotation list panel --
    pub classification_tag: Color,
}

impl Default for Theme {
    fn default() -> Theme {
        Theme {
            keyword: Color::Magenta,
            function: Color::Blue,
            type_: Color::Yellow,
            string: Color::Green,
            number: Color::Cyan,
            comment: Color::DarkGray,
            constant: Color::Cyan,
            property: Color::Blue,
            operator: Color::White,
            punctuation: Color::White,
            variable: Color::White,
            attribute: Color::Magenta,
            embedded: Color::White,
            other: Color::Reset,

            added_bg: Color::Rgb(20, 40, 20),
            removed_bg: Color::Rgb(45, 20, 20),
            word_diff_bg: Color::Rgb(60, 60, 20),
            added_fg: Color::Green,
            removed_fg: Color::Red,
            context_fg: Color::Reset,

            gutter: Color::DarkGray,
            dot_marker: Color::Yellow,
            selected_row_bg: Color::Rgb(30, 30, 40),
            search_match_bg: Color::Rgb(60, 40, 70),
            search_prompt: Color::Cyan,
            annotation_text: Color::DarkGray,
            hunk_header: Color::Cyan,
            binary_placeholder: Color::DarkGray,
            status_message: Color::Yellow,
            column_cursor_bg: Color::Rgb(70, 70, 100),

            kind_added: Color::Green,
            kind_deleted: Color::Red,
            kind_modified: Color::Yellow,
            kind_renamed: Color::Blue,
            kind_untracked: Color::DarkGray,

            staged_indicator: Color::Green,
            dir_prefix: Color::DarkGray,
            footer_text: Color::DarkGray,
            focused_border: Color::Cyan,

            help_section_header: Color::Yellow,
            help_key: Color::Cyan,

            classification_tag: Color::Cyan,
        }
    }
}

impl Theme {
    /// Maps a highlight [`TokenKind`] to its foreground color.
    pub fn token_color(&self, kind: TokenKind) -> Color {
        match kind {
            TokenKind::Keyword => self.keyword,
            TokenKind::Function => self.function,
            TokenKind::Type => self.type_,
            TokenKind::String => self.string,
            TokenKind::Number => self.number,
            TokenKind::Comment => self.comment,
            TokenKind::Constant => self.constant,
            TokenKind::Property => self.property,
            TokenKind::Operator => self.operator,
            TokenKind::Punctuation => self.punctuation,
            TokenKind::Variable => self.variable,
            TokenKind::Attribute => self.attribute,
            TokenKind::Embedded => self.embedded,
            TokenKind::Other => self.other,
        }
    }

    /// A file's change-kind letter color (sidebar + Compose modal title).
    pub fn kind_color(&self, kind: FileChangeKind) -> Color {
        match kind {
            FileChangeKind::Added => self.kind_added,
            FileChangeKind::Deleted => self.kind_deleted,
            FileChangeKind::Modified => self.kind_modified,
            FileChangeKind::Renamed | FileChangeKind::Copied => self.kind_renamed,
        }
    }

    /// A porcelain status letter's color (sidebar staged marker, staging
    /// panel), matching git's own `--name-status` convention plus `?` for
    /// untracked.
    pub fn letter_color(&self, letter: char) -> Color {
        match letter {
            'A' => self.kind_added,
            'M' => self.kind_modified,
            'D' => self.kind_deleted,
            'R' | 'C' => self.kind_renamed,
            '?' => self.kind_untracked,
            _ => Color::White,
        }
    }

    /// A diff line's background tint by origin (`None` for context — no
    /// tint applied).
    pub fn origin_bg(&self, origin: LineOrigin) -> Option<Color> {
        match origin {
            LineOrigin::Added => Some(self.added_bg),
            LineOrigin::Removed => Some(self.removed_bg),
            LineOrigin::Context => None,
        }
    }

    /// A diff line's marker/fallback foreground color by origin (used for
    /// the `+`/`-` gutter marker, and for content with no syntax-highlight
    /// span covering it).
    pub fn origin_fg(&self, origin: LineOrigin) -> Color {
        match origin {
            LineOrigin::Added => self.added_fg,
            LineOrigin::Removed => self.removed_fg,
            LineOrigin::Context => self.context_fg,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_color_covers_every_kind() {
        let theme = Theme::default();
        // Just exercise every variant so a newly added TokenKind fails to
        // compile here (non-exhaustive match) rather than silently falling
        // through at render time.
        for kind in [
            TokenKind::Keyword,
            TokenKind::Function,
            TokenKind::Type,
            TokenKind::String,
            TokenKind::Number,
            TokenKind::Comment,
            TokenKind::Constant,
            TokenKind::Property,
            TokenKind::Operator,
            TokenKind::Punctuation,
            TokenKind::Variable,
            TokenKind::Attribute,
            TokenKind::Embedded,
            TokenKind::Other,
        ] {
            let _ = theme.token_color(kind);
        }
    }

    #[test]
    fn origin_bg_has_no_tint_for_context() {
        let theme = Theme::default();
        assert_eq!(theme.origin_bg(LineOrigin::Context), None);
        assert!(theme.origin_bg(LineOrigin::Added).is_some());
        assert!(theme.origin_bg(LineOrigin::Removed).is_some());
    }
}
