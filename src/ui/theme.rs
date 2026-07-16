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
    /// Gutter line-number foreground on the cursor row (rendered bold),
    /// brighter than [`Theme::gutter`] so the row's numbers read as the
    /// cursor position without any extra marker glyph.
    pub gutter_cursor_fg: Color,
    pub dot_marker: Color,
    pub selected_row_bg: Color,
    pub search_match_bg: Color,
    /// Foreground carried by query-match spans in the Project Search results
    /// list and the fuzzy file finder modal (spec 06 round-1 UX fix): a
    /// vivid blue + bold, so the matched substring's *text* itself reads as
    /// emphasized rather than relying solely on a background tint (the
    /// original `search_match_bg`-only treatment didn't read as high-contrast
    /// enough per user acceptance feedback). Not used by the in-diff `/`
    /// search highlight, which keeps its own `search_match_bg` treatment
    /// untouched — this is scoped to the two surfaces the feedback named.
    pub search_match_fg: Color,
    pub search_prompt: Color,
    pub annotation_text: Color,
    pub hunk_header: Color,
    pub binary_placeholder: Color,
    pub status_message: Color,
    /// The column cursor's cell highlight (diff pane) and the target
    /// line's highlight in the LSP peek overlay's preview pane.
    pub column_cursor_bg: Color,
    /// Standing background band behind every [`super::rows::Row::Annotation`]
    /// display row, so an annotation block reads as visually distinct from
    /// the surrounding diff buffer.
    pub annotation_bg: Color,
    /// The left accent bar drawn on annotation rows (first and continuation
    /// lines alike), so a multi-line annotation reads as one attached unit.
    pub annotation_accent: Color,
    /// Standing background band behind every file header row (expanded or
    /// collapsed), so file boundaries — and collapsed files, which are just
    /// their header row — stay visible against the diff buffer.
    pub file_header_bg: Color,

    /// The review-session banner's background (spec 08 Unit 2): a dark red,
    /// deliberately unlike any other chrome color in this palette, so the
    /// banner reads as an unmistakable "you are in a review, not your own
    /// working tree" signal. Paired with [`Theme::review_banner_fg`]; the
    /// pairing's contrast is guarded by a drift test in this module.
    pub review_banner_bg: Color,
    /// The review-session banner's foreground: a light, high-contrast color
    /// against [`Theme::review_banner_bg`].
    pub review_banner_fg: Color,

    /// The deferred-file review marker's color (`~`).
    pub review_deferred_marker: Color,
    /// The changed-since-accepted review marker's color (`!`) — spec 08 Unit
    /// 4 sets this status once persistence/reconciliation lands; the color
    /// exists now so the marker glyph has a home from the start.
    pub review_changed_marker: Color,

    // -- Change-kind letters (sidebar file list + staging panel) --
    pub kind_added: Color,
    pub kind_deleted: Color,
    pub kind_modified: Color,
    pub kind_renamed: Color,
    pub kind_untracked: Color,

    // -- Sidebar --
    /// The `●` staged-file marker's color. Also the accepted-file review
    /// marker's color (spec 08 Unit 3, deliberate exception, user decision
    /// 2026-07-16): a review session's [`Theme::staged_indicator`] never
    /// renders (review targets are read-only for staging), so reusing this
    /// exact green for `Accepted`'s `●` deliberately mirrors the staged-file
    /// affordance rather than inventing a new signal for the same "this file
    /// is done, collapsed, no action needed" meaning — see
    /// [`super::rows::ReviewMarker`].
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
            gutter_cursor_fg: Color::Rgb(220, 220, 230),
            dot_marker: Color::Yellow,
            selected_row_bg: Color::Rgb(45, 55, 90),
            search_match_bg: Color::Rgb(60, 40, 70),
            search_match_fg: Color::Rgb(100, 180, 255),
            search_prompt: Color::Cyan,
            annotation_text: Color::DarkGray,
            hunk_header: Color::Cyan,
            binary_placeholder: Color::DarkGray,
            status_message: Color::Yellow,
            column_cursor_bg: Color::Rgb(110, 120, 170),
            annotation_bg: Color::Rgb(24, 22, 32),
            annotation_accent: Color::Rgb(140, 120, 200),
            file_header_bg: Color::Rgb(20, 24, 28),

            review_banner_bg: Color::Rgb(110, 10, 10),
            review_banner_fg: Color::Rgb(255, 235, 235),

            review_deferred_marker: Color::Rgb(220, 170, 60),
            review_changed_marker: Color::Rgb(240, 100, 100),

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

/// Composites a selection highlight over a standing background tint by
/// per-channel saturating addition of the two `Color::Rgb` values, so the
/// result keeps the tint's hue (an added line under the cursor still reads
/// green) while gaining the selection's brightness. If either side is not
/// `Color::Rgb` (named/indexed colors have no portable channel values),
/// falls back to `selection` unchanged — the cursor row must stay
/// highlighted even under a non-Rgb theme.
pub fn blend(selection: Color, tint: Color) -> Color {
    match (selection, tint) {
        (Color::Rgb(sr, sg, sb), Color::Rgb(tr, tg, tb)) => Color::Rgb(
            sr.saturating_add(tr),
            sg.saturating_add(tg),
            sb.saturating_add(tb),
        ),
        _ => selection,
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

    #[test]
    fn blend_composites_rgb_channels_with_saturation() {
        assert_eq!(
            blend(Color::Rgb(45, 55, 90), Color::Rgb(20, 40, 20)),
            Color::Rgb(65, 95, 110)
        );
        // Channels saturate at 255 instead of wrapping.
        assert_eq!(
            blend(Color::Rgb(200, 200, 200), Color::Rgb(100, 100, 100)),
            Color::Rgb(255, 255, 255)
        );
    }

    #[test]
    fn blend_falls_back_to_the_selection_color_when_either_side_is_not_rgb() {
        assert_eq!(
            blend(Color::Rgb(45, 55, 90), Color::Green),
            Color::Rgb(45, 55, 90)
        );
        assert_eq!(blend(Color::Blue, Color::Rgb(1, 2, 3)), Color::Blue);
    }

    /// Perceived brightness proxy for the contrast drift-guards below:
    /// plain channel sum is enough to order these hand-picked constants.
    fn channel_sum(c: Color) -> u32 {
        match c {
            Color::Rgb(r, g, b) => r as u32 + g as u32 + b as u32,
            _ => 0,
        }
    }

    #[test]
    fn column_cursor_bg_stays_brighter_than_every_selected_row_blend() {
        let theme = Theme::default();
        let blends = [
            theme.selected_row_bg,
            blend(theme.selected_row_bg, theme.added_bg),
            blend(theme.selected_row_bg, theme.removed_bg),
            blend(theme.selected_row_bg, theme.search_match_bg),
        ];
        for bg in blends {
            assert!(
                channel_sum(theme.column_cursor_bg) > channel_sum(bg),
                "column cursor {:?} must outshine row bg {bg:?}",
                theme.column_cursor_bg
            );
        }
    }

    #[test]
    fn selected_row_blends_are_brighter_than_their_unselected_tints() {
        let theme = Theme::default();
        for tint in [theme.added_bg, theme.removed_bg] {
            assert!(channel_sum(blend(theme.selected_row_bg, tint)) > channel_sum(tint));
        }
    }

    // -- Review banner contrast (spec 08 Unit 2) -----------------------------
    //
    // Written before `review_banner_bg`/`review_banner_fg` existed (TDD): the
    // banner must read as an unmistakable, high-contrast "you're in a review"
    // signal, so these guard both halves of that claim — the background
    // reads as dark, and the foreground reads as far brighter than it.

    #[test]
    fn review_banner_bg_reads_as_dark() {
        let theme = Theme::default();
        assert!(
            channel_sum(theme.review_banner_bg) < 300,
            "banner background {:?} must read as dark",
            theme.review_banner_bg
        );
    }

    #[test]
    fn review_banner_fg_is_far_brighter_than_its_background() {
        let theme = Theme::default();
        assert!(
            channel_sum(theme.review_banner_fg) > 600,
            "banner foreground {:?} must read as bright",
            theme.review_banner_fg
        );
        assert!(
            channel_sum(theme.review_banner_fg) > channel_sum(theme.review_banner_bg) + 400,
            "banner foreground must stay high-contrast against its dark-red background"
        );
    }
}
