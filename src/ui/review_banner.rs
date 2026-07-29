//! The review-session banner: a full-width, single-row band
//! reading ` REVIEWING <branch> — q to end review +A -R` with the
//! `<accepted>/<total>` progress count right-aligned at the row's far edge,
//! shown above everything else in [`super::draw`] whenever
//! [`super::app::App::in_review_session`] is true.
//!
//! [`layout`] is the pure content half (branch/counts/stat/width in, the
//! banner's text pieces out, truncating only the branch name — never
//! wrapping) so it's unit-testable without a terminal; [`banner_text`]
//! concatenates those pieces into the one-line string the existing
//! byte-exact tests assert against; [`render`] is the thin ratatui half that
//! turns the same pieces into styled spans (bold branch, dim hint,
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
/// plus `HINT` plus the `+A -R` stat segment plus the progress count plus one
/// trailing-space column) to fit, truncating only the branch name (with a
/// trailing ellipsis) when it doesn't fit — the branch never wraps to a
/// second row.
///
/// `Clipped` covers the pathologically narrow remainder, where even the
/// fixed chrome doesn't fit: a hard clip of the unpadded text, not worth a
/// nicer message.
enum BannerLayout {
    Full {
        /// The branch name, truncated with a trailing `…` if it didn't fit.
        branch: String,
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
    branch: &str,
    accepted: usize,
    total: usize,
    stat: DiffStat,
    width: usize,
) -> BannerLayout {
    let count = format!("{accepted}/{total}");
    let stat_str = stat_text(stat);
    // PREFIX + HINT + " " + stat + count + one trailing-space column (the
    // gap between the stat segment and the count is the variable `pad`, not
    // a fixed column); the fixed-width parts every layout reserves
    // regardless of the branch name or padding.
    let fixed_len = PREFIX.chars().count()
        + HINT.chars().count()
        + 1
        + stat_str.chars().count()
        + count.chars().count()
        + 1;

    if fixed_len >= width {
        let full = format!("{PREFIX}{branch}{HINT} {stat_str} {count}");
        return BannerLayout::Clipped(full.chars().take(width).collect());
    }

    let branch_len = branch.chars().count();
    let branch_display = if fixed_len + branch_len <= width {
        branch.to_string()
    } else {
        // Truncate the branch name only; the chrome (PREFIX/HINT/stat/count)
        // never shrinks. Reserves one column for the ellipsis and, budget
        // permitting, one more so the padding between the stat segment and
        // the count never collapses to zero on top of the truncation.
        let budget = width - fixed_len;
        let keep = budget.saturating_sub(2);
        let truncated: String = branch.chars().take(keep).collect();
        format!("{truncated}\u{2026}")
    };

    let used = PREFIX.chars().count()
        + branch_display.chars().count()
        + HINT.chars().count()
        + 1
        + stat_str.chars().count()
        + count.chars().count()
        + 1;
    let pad = width.saturating_sub(used);
    BannerLayout::Full {
        branch: branch_display,
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
    branch: &str,
    accepted: usize,
    total: usize,
    stat: DiffStat,
    width: u16,
) -> String {
    let width = width as usize;
    if width == 0 {
        return String::new();
    }
    match layout(branch, accepted, total, stat, width) {
        BannerLayout::Clipped(s) => s,
        BannerLayout::Full {
            branch,
            pad,
            stat,
            count,
        } => {
            format!(
                "{PREFIX}{branch}{HINT} {}{}{count} ",
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
    branch: &str,
    accepted: usize,
    total: usize,
    stat: DiffStat,
    stale: bool,
) {
    let fg = Style::default().fg(theme.review_banner_fg);
    // A stale PR checkout (entered after a fetch failure left a prior
    // worktree) folds a visible marker into the branch label so the reviewer
    // can't miss that the checkout may lag the PR's real head.
    let label = if stale {
        format!("{branch} \u{26A0} STALE")
    } else {
        branch.to_string()
    };
    let branch = label.as_str();
    let spans = match layout(branch, accepted, total, stat, area.width as usize) {
        BannerLayout::Clipped(s) => vec![Span::styled(s, fg.add_modifier(Modifier::BOLD))],
        BannerLayout::Full {
            branch,
            pad,
            stat,
            count,
        } => vec![
            Span::styled(PREFIX, fg),
            Span::styled(branch, fg.add_modifier(Modifier::BOLD)),
            Span::styled(HINT, fg.add_modifier(Modifier::DIM)),
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
    fn render_line(branch: &str, accepted: usize, total: usize, stale: bool) -> String {
        let backend = TestBackend::new(80, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render(
                    frame,
                    area,
                    &Theme::default(),
                    branch,
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
    fn stale_checkout_renders_a_visible_stale_marker_and_a_fresh_one_does_not() {
        let line = render_line("redquill/pr/7", 0, 3, true);
        assert!(
            line.contains("STALE"),
            "a stale PR checkout must render a STALE marker: {line:?}"
        );
        assert!(line.contains("redquill/pr/7"));

        let fresh = render_line("redquill/pr/7", 0, 3, false);
        assert!(
            !fresh.contains("STALE"),
            "a fresh session must not render a STALE marker: {fresh:?}"
        );
    }

    #[test]
    fn renders_the_aggregate_added_removed_counts() {
        let line = render_line("redquill/pr/7", 0, 3, false);
        assert!(line.contains("+7"), "aggregate added count: {line:?}");
        assert!(line.contains("-2"), "aggregate removed count: {line:?}");
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
}
