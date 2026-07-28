//! Shared rendering for the git-diffstat-style `+A -R` line-change display
//! (or `bin`, or nothing at all) shown across every changed-file surface:
//! the git panel's Changes tab and bottom counts line, the diff pane's
//! per-file headers and commit-view header, the review banner's aggregate,
//! and the staging panel. One helper so the four colors/spans/widths can't
//! drift between surfaces — [`crate::diff::stat_display`] decides *what*
//! category a file falls into (pure, no ratatui types); this decides how
//! that category *looks*.

use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

use crate::diff::StatDisplay;

use super::theme::Theme;

/// Builds the styled spans for one [`StatDisplay`] plus their total display
/// width, so right-aligned layouts (the git panel's status cluster, the
/// staging panel) can pad correctly. `None` means render nothing — the
/// omitted case.
pub fn stat_display_spans(
    display: StatDisplay,
    theme: &Theme,
) -> Option<(Vec<Span<'static>>, usize)> {
    match display {
        StatDisplay::Omitted => None,
        StatDisplay::Binary => {
            const TEXT: &str = "bin";
            Some((
                vec![Span::styled(
                    TEXT,
                    Style::default()
                        .fg(theme.binary_placeholder)
                        .add_modifier(Modifier::DIM),
                )],
                TEXT.len(),
            ))
        }
        StatDisplay::Counts(stat) => {
            let added = format!("+{}", stat.added);
            let removed = format!("-{}", stat.removed);
            let width = added.chars().count() + 1 + removed.chars().count();
            Some((
                vec![
                    Span::styled(added, Style::default().fg(theme.kind_added)),
                    Span::raw(" "),
                    Span::styled(removed, Style::default().fg(theme.kind_deleted)),
                ],
                width,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::DiffStat;

    #[test]
    fn omitted_renders_nothing() {
        assert_eq!(
            stat_display_spans(StatDisplay::Omitted, &Theme::default()),
            None
        );
    }

    #[test]
    fn binary_renders_a_dim_bin_placeholder() {
        let (spans, width) = stat_display_spans(StatDisplay::Binary, &Theme::default()).unwrap();
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "bin");
        assert_eq!(width, 3);
    }

    #[test]
    fn counts_renders_plus_and_minus_with_a_space_between() {
        let stat = DiffStat {
            added: 12,
            removed: 4,
        };
        let (spans, width) =
            stat_display_spans(StatDisplay::Counts(stat), &Theme::default()).unwrap();
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "+12 -4");
        assert_eq!(width, "+12".len() + 1 + "-4".len());
    }
}
