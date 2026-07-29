//! The review-session banner: a full-width, single-row band
//! reading ` REVIEWING <label> +A -R` — where `<label>` is
//! `#<number> <title>` for a forge PR/MR review and the branch name for a
//! local-branch one (see [`super::app::App::review_banner_label`]) — with the
//! `<accepted>/<total>` progress count right-aligned at the row's far edge,
//! shown above everything else in [`super::draw`] whenever
//! [`super::app::App::in_review_session`] is true. The keys a review session
//! offers (`q` to end it, `gx` to open the PR) live in the footer strip, not
//! here — the banner names what is being reviewed and how far it has got.
//!
//! [`layout`] is the pure content half (label/counts/stat/width in, the
//! banner's text pieces out, truncating only the label — never
//! wrapping) so it's unit-testable without a terminal; [`banner_text`]
//! concatenates those pieces into the one-line string the existing
//! byte-exact tests assert against; [`render`] is the thin ratatui half that
//! turns the same pieces into styled spans (bold label,
//! kind_added/kind_deleted stat halves) and pads the row to its full width,
//! painting [`super::theme::Theme::review_banner_bg`]/`review_banner_fg`
//! across the whole row (the same trailing-space-padding trick
//! [`super::diff_view::annotation_row_line`]/`file_header_line` use, since
//! `Paragraph` only paints a `Line`'s style onto the cells its spans
//! occupy).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::diff::DiffStat;

use super::theme::Theme;

/// One space of left padding, then the `REVIEWING` word and a trailing
/// space before the session label.
const PREFIX: &str = " REVIEWING ";
/// The banner's text, broken into the pieces [`render`] styles individually
/// (bold label, colored stat halves) and [`banner_text`] concatenates
/// verbatim.
///
/// `Full` covers every terminal wide enough for the fixed chrome (`PREFIX`
/// plus the `+A -R` stat segment plus the progress count plus one
/// trailing-space column) to fit, truncating only the label (with a
/// trailing ellipsis) when it doesn't fit — the label never wraps to a
/// second row.
///
/// `Clipped` covers the pathologically narrow remainder, where even the
/// fixed chrome doesn't fit: a hard clip of the unpadded text, not worth a
/// nicer message.
enum BannerLayout {
    Full {
        /// The session label, truncated with a trailing `…` if it didn't fit.
        label: String,
        /// Columns of padding between the stat segment and the
        /// right-aligned count.
        pad: usize,
        /// The review's aggregate added/removed line counts.
        stat: DiffStat,
        /// `accepted/total`.
        count: String,
    },
    Clipped(String),
}

/// Formats a [`DiffStat`] as the banner's fixed `+A -R` segment text (used
/// for both the width math below and the byte-exact [`banner_text`] test
/// helper — [`render`] builds the same two halves as separately colored
/// spans instead).
fn stat_text(stat: DiffStat) -> String {
    format!("+{} -{}", stat.added, stat.removed)
}

/// Builds the banner's text pieces for a `width`-column-wide band. Pure: no
/// ratatui/terminal types, so this is directly unit-testable (via
/// [`banner_text`]) against a plain `width` rather than a real frame.
fn layout(
    label: &str,
    accepted: usize,
    total: usize,
    stat: DiffStat,
    width: usize,
) -> BannerLayout {
    let count = format!("{accepted}/{total}");
    let stat_str = stat_text(stat);
    // PREFIX + " " + stat + count + one trailing-space column (the gap
    // between the stat segment and the count is the variable `pad`, not a
    // fixed column); the fixed-width parts every layout reserves regardless
    // of the label or padding.
    let fixed_len =
        PREFIX.chars().count() + 1 + stat_str.chars().count() + count.chars().count() + 1;

    if fixed_len >= width {
        let full = format!("{PREFIX}{label} {stat_str} {count}");
        return BannerLayout::Clipped(full.chars().take(width).collect());
    }

    let label_len = label.chars().count();
    let label_display = if fixed_len + label_len <= width {
        label.to_string()
    } else {
        // Truncate the label only; the chrome (PREFIX/stat/count)
        // never shrinks. Reserves one column for the ellipsis and, budget
        // permitting, one more so the padding between the stat segment and
        // the count never collapses to zero on top of the truncation.
        let budget = width - fixed_len;
        let keep = budget.saturating_sub(2);
        let truncated: String = label.chars().take(keep).collect();
        format!("{truncated}\u{2026}")
    };

    let used = PREFIX.chars().count()
        + label_display.chars().count()
        + 1
        + stat_str.chars().count()
        + count.chars().count()
        + 1;
    let pad = width.saturating_sub(used);
    BannerLayout::Full {
        label: label_display,
        pad,
        stat,
        count,
    }
}

/// Builds the banner's single-row text for a `width`-column-wide band, for
/// the byte-exact tests below (`render` builds the same pieces from
/// [`layout`] directly, as separately styled spans, rather than going
/// through this concatenated string — hence `#[cfg(test)]`: this exists
/// purely so the truncation/padding contract stays unit-testable against a
/// plain `width`, without a real frame).
#[cfg(test)]
pub(super) fn banner_text(
    label: &str,
    accepted: usize,
    total: usize,
    stat: DiffStat,
    width: u16,
) -> String {
    let width = width as usize;
    if width == 0 {
        return String::new();
    }
    match layout(label, accepted, total, stat, width) {
        BannerLayout::Clipped(s) => s,
        BannerLayout::Full {
            label,
            pad,
            stat,
            count,
        } => {
            format!(
                "{PREFIX}{label} {}{}{count} ",
                stat_text(stat),
                " ".repeat(pad)
            )
        }
    }
}

/// Renders the review banner as a full-width, one-row band at `area`
/// (already sized to `Constraint::Length(1)` by the caller — see
/// [`super::split_banner`]).
#[allow(clippy::too_many_arguments)]
pub(super) fn render(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    label: &str,
    accepted: usize,
    total: usize,
    stat: DiffStat,
    stale: bool,
) {
    let fg = Style::default().fg(theme.review_banner_fg);
    // A stale PR checkout (entered after a fetch failure left a prior
    // worktree) folds a visible marker into the session label so the reviewer
    // can't miss that the checkout may lag the PR's real head.
    let marked = if stale {
        format!("{label} \u{26A0} STALE")
    } else {
        label.to_string()
    };
    let spans = match layout(&marked, accepted, total, stat, area.width as usize) {
        BannerLayout::Clipped(s) => vec![Span::styled(s, fg.add_modifier(Modifier::BOLD))],
        BannerLayout::Full {
            label,
            pad,
            stat,
            count,
        } => vec![
            Span::styled(PREFIX, fg),
            Span::styled(label, fg.add_modifier(Modifier::BOLD)),
            Span::raw(" "),
            // The stat halves use the same kind_added/kind_deleted colors
            // every other diffstat display uses, not the banner's own fg,
            // so the aggregate reads consistently across surfaces.
            Span::styled(
                format!("+{}", stat.added),
                Style::default().fg(theme.kind_added),
            ),
            Span::raw(" "),
            Span::styled(
                format!("-{}", stat.removed),
                Style::default().fg(theme.kind_deleted),
            ),
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
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// The stat used by every test below unless it's specifically exercising
    /// a different one.
    fn sample_stat() -> DiffStat {
        DiffStat {
            added: 7,
            removed: 2,
        }
    }

    /// Flattens a full-width banner render into a plain string.
    fn render_line(label: &str, accepted: usize, total: usize, stale: bool) -> String {
        let backend = TestBackend::new(80, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render(
                    frame,
                    area,
                    &Theme::default(),
                    label,
                    accepted,
                    total,
                    sample_stat(),
                    stale,
                );
            })
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn stale_checkout_renders_a_visible_stale_marker() {
        let line = render_line("redquill/pr/7", 0, 3, true);
        assert!(
            line.contains("STALE"),
            "a stale PR checkout must render a STALE marker: {line:?}"
        );
        assert!(line.contains("redquill/pr/7"));
    }

    #[test]
    fn fresh_checkout_renders_no_stale_marker() {
        let line = render_line("redquill/pr/7", 0, 3, false);
        assert!(
            !line.contains("STALE"),
            "a fresh session must not render a STALE marker: {line:?}"
        );
    }

    #[test]
    fn renders_the_aggregate_added_removed_counts() {
        let line = render_line("redquill/pr/7", 0, 3, false);
        assert!(line.contains("+7"), "aggregate added count: {line:?}");
        assert!(line.contains("-2"), "aggregate removed count: {line:?}");
    }

    #[test]
    fn fits_unchanged_when_width_is_generous() {
        let text = banner_text("feature/thing", 4, 12, sample_stat(), 80);
        let expected = format!(" REVIEWING feature/thing +7 -2{}4/12 ", " ".repeat(45));
        assert_eq!(text, expected);
    }

    #[test]
    fn leading_padding_and_right_aligned_count() {
        let text = banner_text("feature/thing", 4, 12, sample_stat(), 80);
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
    fn renders_the_stat_segment_between_the_label_and_the_count() {
        let text = banner_text("feature/thing", 4, 12, sample_stat(), 80);
        assert!(
            text.contains("feature/thing +7 -2"),
            "stat segment must sit right after the label: {text:?}"
        );
    }

    #[test]
    fn never_exceeds_the_requested_width() {
        for width in [0u16, 1, 5, 10, 20, 30, 34, 35, 40, 79] {
            let text = banner_text(
                "a-very-long-feature-branch-name",
                4,
                12,
                sample_stat(),
                width,
            );
            assert!(
                text.chars().count() <= width as usize,
                "width {width}: {text:?} ({} chars)",
                text.chars().count()
            );
        }
    }

    #[test]
    fn truncates_the_branch_name_with_an_ellipsis_on_a_narrow_terminal() {
        let text = banner_text("a-very-long-feature-branch-name", 4, 12, sample_stat(), 45);
        assert!(
            text.contains('\u{2026}'),
            "narrow banner must truncate the branch name: {text:?}"
        );
        assert!(text.starts_with(" REVIEWING "));
        assert!(
            text.contains("+7 -2"),
            "the stat segment must survive truncation: {text:?}"
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
        let text = banner_text("a-very-long-feature-branch-name", 4, 12, sample_stat(), 20);
        assert!(!text.contains('\n'));
    }

    #[test]
    fn short_branch_name_is_never_truncated_even_when_width_is_tight_around_it() {
        // The branch is short enough that only the surrounding chrome, not
        // the branch itself, would need to shrink -- but chrome is fixed, so
        // this just exercises the "doesn't fit, but budget still covers the
        // whole branch" non-panicking path stays a no-op distinguishable
        // from the truncated case. Width is a few columns wider than the
        // short-branch-name test used before the stat segment existed, since
        // the segment adds fixed-width chrome of its own.
        let text = banner_text("ab", 0, 1, sample_stat(), 45);
        assert!(text.contains("ab"));
        assert!(!text.contains('\u{2026}'));
        assert_eq!(text.chars().count(), 45);
    }
}
