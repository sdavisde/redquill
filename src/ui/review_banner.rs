//! The review-session banner (spec 08 Unit 2): a full-width, single-row band
//! reading ` REVIEWING <branch> — q to end review` with the
//! `<accepted>/<total>` progress count right-aligned at the row's far edge,
//! shown above everything else in [`super::draw`] whenever
//! [`super::app::App::in_review_session`] is true.
//!
//! [`layout`] is the pure content half (branch/counts/width in, the banner's
//! five text pieces out, truncating the branch name — never wrapping) so
//! it's unit-testable without a terminal; [`banner_text`] concatenates those
//! pieces into the one-line string the existing byte-exact tests assert
//! against; [`render`] is the thin ratatui half that turns the same pieces
//! into styled spans (bold branch, dim hint) and pads the row to its full
//! width, painting [`super::theme::Theme::review_banner_bg`]/
//! `review_banner_fg` across the whole row (the same trailing-space-padding
//! trick [`super::diff_view::annotation_row_line`]/`file_header_line` use,
//! since `Paragraph` only paints a `Line`'s style onto the cells its spans
//! occupy).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::theme::Theme;

/// One space of left padding, then the `REVIEWING` label and a trailing
/// space before the branch name.
const PREFIX: &str = " REVIEWING ";
/// The de-emphasized hint between the branch name and the right-aligned
/// progress count.
const HINT: &str = " \u{2014} q to end review";

/// The banner's text, broken into the pieces [`render`] styles individually
/// (bold branch, dim hint) and [`banner_text`] concatenates verbatim.
///
/// `Full` covers every terminal wide enough for the fixed chrome (`PREFIX`
/// plus `HINT` plus the progress count plus one trailing-space column) to
/// fit, truncating only the branch name (with a trailing ellipsis) when it
/// doesn't fit alongside that chrome. The branch is the one variable-length
/// part, and the spec calls for truncating it rather than ever wrapping to a
/// second row.
///
/// `Clipped` covers the pathologically narrow remainder, where even the
/// fixed chrome doesn't fit: a hard clip of the unpadded text, not worth a
/// nicer message.
enum BannerLayout {
    Full {
        /// The branch name, truncated with a trailing `…` if it didn't fit.
        branch: String,
        /// Columns of padding between the hint and the right-aligned count.
        pad: usize,
        /// `accepted/total`.
        count: String,
    },
    Clipped(String),
}

/// Builds the banner's text pieces for a `width`-column-wide band. Pure: no
/// ratatui/terminal types, so this is directly unit-testable (via
/// [`banner_text`]) against a plain `width` rather than a real frame.
fn layout(branch: &str, accepted: usize, total: usize, width: usize) -> BannerLayout {
    let count = format!("{accepted}/{total}");
    // PREFIX + HINT + count + one trailing-space column; the fixed-width
    // parts every layout reserves regardless of the branch name or padding.
    let fixed_len = PREFIX.chars().count() + HINT.chars().count() + count.chars().count() + 1;

    if fixed_len >= width {
        let full = format!("{PREFIX}{branch}{HINT} {count}");
        return BannerLayout::Clipped(full.chars().take(width).collect());
    }

    let branch_len = branch.chars().count();
    let branch_display = if fixed_len + branch_len <= width {
        branch.to_string()
    } else {
        // Truncate the branch name only; the chrome (PREFIX/HINT/count)
        // never shrinks. Reserves one column for the ellipsis and, budget
        // permitting, one more so the padding between the hint and the
        // count never collapses to zero on top of the truncation.
        let budget = width - fixed_len;
        let keep = budget.saturating_sub(2);
        let truncated: String = branch.chars().take(keep).collect();
        format!("{truncated}\u{2026}")
    };

    let used = PREFIX.chars().count()
        + branch_display.chars().count()
        + HINT.chars().count()
        + count.chars().count()
        + 1;
    let pad = width.saturating_sub(used);
    BannerLayout::Full {
        branch: branch_display,
        pad,
        count,
    }
}

/// Builds the banner's single-row text for a `width`-column-wide band, for
/// the byte-exact tests below (`render` builds the same pieces from
/// [`layout`] directly, as separately styled spans, rather than going
/// through this concatenated string — hence `#[cfg(test)]`: this exists
/// purely so the truncation/padding contract stays unit-testable against a
/// plain `width`, without a real frame). See [`layout`]'s doc for the
/// truncation contract this preserves unchanged from before the
/// padding/right-alignment polish pass.
#[cfg(test)]
pub(super) fn banner_text(branch: &str, accepted: usize, total: usize, width: u16) -> String {
    let width = width as usize;
    if width == 0 {
        return String::new();
    }
    match layout(branch, accepted, total, width) {
        BannerLayout::Clipped(s) => s,
        BannerLayout::Full { branch, pad, count } => {
            format!("{PREFIX}{branch}{HINT}{}{count} ", " ".repeat(pad))
        }
    }
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
    let fg = Style::default().fg(theme.review_banner_fg);
    let spans = match layout(branch, accepted, total, area.width as usize) {
        BannerLayout::Clipped(s) => vec![Span::styled(s, fg.add_modifier(Modifier::BOLD))],
        BannerLayout::Full { branch, pad, count } => vec![
            Span::styled(PREFIX, fg),
            Span::styled(branch, fg.add_modifier(Modifier::BOLD)),
            Span::styled(HINT, fg.add_modifier(Modifier::DIM)),
            Span::raw(" ".repeat(pad)),
            Span::styled(count, fg.add_modifier(Modifier::BOLD)),
            Span::raw(" "),
        ],
    };
    let mut line = Line::from(spans);
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
        let expected = format!(
            " REVIEWING feature/thing \u{2014} q to end review{}4/12 ",
            " ".repeat(33)
        );
        assert_eq!(text, expected);
    }

    #[test]
    fn leading_padding_and_right_aligned_count() {
        let text = banner_text("feature/thing", 4, 12, 80);
        assert!(
            text.starts_with(" REVIEWING "),
            "one space of left padding before REVIEWING: {text:?}"
        );
        assert!(
            text.ends_with("4/12 "),
            "progress count right-aligned with one trailing space: {text:?}"
        );
        assert_eq!(text.chars().count(), 80);
    }

    #[test]
    fn never_exceeds_the_requested_width() {
        for width in [0u16, 1, 5, 10, 20, 30, 34, 35, 40, 79] {
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
        let text = banner_text("a-very-long-feature-branch-name", 4, 12, 45);
        assert!(
            text.contains('\u{2026}'),
            "narrow banner must truncate the branch name: {text:?}"
        );
        assert!(text.starts_with(" REVIEWING "));
        assert!(
            text.contains("\u{2014} q to end review"),
            "the hint must survive truncation: {text:?}"
        );
        assert!(
            text.ends_with("4/12 "),
            "the progress count must survive truncation: {text:?}"
        );
        assert!(
            !text.contains("a-very-long-feature-branch-name"),
            "the full branch name must have been shortened: {text:?}"
        );
        assert_eq!(text.chars().count(), 45);
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
        let text = banner_text("ab", 0, 1, 40);
        assert!(text.contains("ab"));
        assert!(!text.contains('\u{2026}'));
        assert_eq!(text.chars().count(), 40);
    }
}
