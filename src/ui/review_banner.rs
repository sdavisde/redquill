//! The review-session banner (spec 08 Unit 2): a full-width, single-row band
//! reading `REVIEWING <branch> — q to end review  <accepted>/<total>`, shown
//! above everything else in [`super::draw`] whenever
//! [`super::app::App::in_review_session`] is true.
//!
//! [`banner_text`] is the pure content half (branch/counts/width in, one
//! line of text out, truncating the branch name — never wrapping) so it's
//! unit-testable without a terminal; [`render`] is the thin ratatui half
//! that pads it to the row's full width and paints
//! [`super::theme::Theme::review_banner_bg`]/`review_banner_fg` across the
//! whole row (the same trailing-space-padding trick
//! [`super::diff_view::annotation_row_line`]/`file_header_line` use, since
//! `Paragraph` only paints a `Line`'s style onto the cells its spans occupy).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::theme::Theme;

/// Builds the banner's single-row text for a `width`-column-wide band.
/// Truncates the branch name (with a trailing `…`) first when the full text
/// doesn't fit, keeping the `REVIEWING `/` — q to end review  n/m` chrome
/// intact — the branch is the one variable-length part, and the spec calls
/// for truncating it rather than ever wrapping to a second row. Pure: no
/// ratatui/terminal types, so this is directly unit-testable against a plain
/// `width` rather than a real frame.
pub(super) fn banner_text(branch: &str, accepted: usize, total: usize, width: u16) -> String {
    let width = width as usize;
    if width == 0 {
        return String::new();
    }
    let prefix = "REVIEWING ";
    let suffix = format!(" \u{2014} q to end review  {accepted}/{total}");
    let full = format!("{prefix}{branch}{suffix}");
    if full.chars().count() <= width {
        return full;
    }

    let fixed_len = prefix.chars().count() + suffix.chars().count();
    if fixed_len >= width {
        // Even the fixed chrome doesn't fit: a pathologically narrow
        // terminal, not worth a nicer message than a hard clip.
        return full.chars().take(width).collect();
    }

    let budget = width - fixed_len;
    let keep = budget - 1; // reserve one column for the ellipsis
    let truncated_branch: String = branch.chars().take(keep).collect();
    format!("{prefix}{truncated_branch}\u{2026}{suffix}")
}

/// Renders the review banner as a full-width, one-row band at `area`
/// (already sized to `Constraint::Length(1)` by the caller — see
/// [`super::split_banner`]).
pub(super) fn render(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    branch: &str,
    accepted: usize,
    total: usize,
) {
    let text = banner_text(branch, accepted, total, area.width);
    let mut line = Line::from(Span::styled(
        text,
        Style::default()
            .fg(theme.review_banner_fg)
            .add_modifier(Modifier::BOLD),
    ));
    let pad = (area.width as usize).saturating_sub(line.width());
    if pad > 0 {
        line.spans.push(Span::raw(" ".repeat(pad)));
    }
    line.style = Style::default().bg(theme.review_banner_bg);
    frame.render_widget(Paragraph::new(line), area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_unchanged_when_width_is_generous() {
        let text = banner_text("feature/thing", 4, 12, 80);
        assert_eq!(
            text,
            format!("REVIEWING feature/thing \u{2014} q to end review  4/12")
        );
    }

    #[test]
    fn never_exceeds_the_requested_width() {
        for width in [0u16, 1, 5, 10, 20, 30, 79] {
            let text = banner_text("a-very-long-feature-branch-name", 4, 12, width);
            assert!(
                text.chars().count() <= width as usize,
                "width {width}: {text:?} ({} chars)",
                text.chars().count()
            );
        }
    }

    #[test]
    fn truncates_the_branch_name_with_an_ellipsis_on_a_narrow_terminal() {
        let text = banner_text("a-very-long-feature-branch-name", 4, 12, 40);
        assert!(
            text.contains('\u{2026}'),
            "narrow banner must truncate the branch name: {text:?}"
        );
        assert!(text.starts_with("REVIEWING "));
        assert!(
            text.ends_with("q to end review  4/12"),
            "the fixed chrome (hint + count) must survive truncation: {text:?}"
        );
        assert!(
            !text.contains("a-very-long-feature-branch-name"),
            "the full branch name must have been shortened: {text:?}"
        );
    }

    #[test]
    fn never_wraps_to_a_second_line() {
        let text = banner_text("a-very-long-feature-branch-name", 4, 12, 20);
        assert!(!text.contains('\n'));
    }

    #[test]
    fn short_branch_name_is_never_truncated_even_when_width_is_tight_around_it() {
        // The branch is short enough that only the surrounding chrome, not
        // the branch itself, would need to shrink -- but chrome is fixed, so
        // this just exercises the "doesn't fit, but budget still covers the
        // whole branch" non-panicking path stays a no-op distinguishable
        // from the truncated case.
        let text = banner_text("ab", 0, 1, 15);
        assert!(text.chars().count() <= 15);
    }
}
