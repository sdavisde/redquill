//! The which-key popup: a small, footer-anchored hint that appears once a
//! two-key prefix (`g`, `z`, ...) has been pending for a brief pause,
//! listing its bound continuations so the prefix namespaces are explorable
//! rather than memorized. Content always comes from the keymap's own
//! two-key bindings (see [`super::keymap::Keymap::continuations_for`]) —
//! this module never lists a binding by name.
//!
//! Timing: the render loop already ticks on its own (see [`super::event_loop`]),
//! so no thread or timer lives here. [`should_show`] takes the elapsed time
//! as a plain argument rather than reading a clock itself, which is what
//! keeps it pure and testable without real sleeps.

use std::time::Duration;

use crossterm::event::KeyEvent;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::Theme;

/// The pause, from the moment a prefix goes pending, before the popup
/// appears. A compile-time constant this round — a config knob is a
/// deferred follow-up.
pub const WHICH_KEY_DELAY: Duration = Duration::from_millis(500);

/// Whether the which-key popup should render this frame. A prefix must
/// currently be pending, and the time since it went pending must have
/// reached `threshold`. `elapsed` is supplied by the caller (a stored
/// `Instant`, diffed against `Instant::now()` on the existing render tick)
/// rather than computed here, so this function never touches a clock and
/// stays trivially testable with injected values — no real sleeps, no
/// flakiness.
pub fn should_show(
    pending: Option<KeyEvent>,
    elapsed: Option<Duration>,
    threshold: Duration,
) -> bool {
    pending.is_some() && elapsed.is_some_and(|e| e >= threshold)
}

/// Renders the popup near the footer — anchored bottom-left with its
/// bottom edge just above `footer_top`, never centered, so it reads as an
/// input hint rather than a modal interruption. `rows` are `(key label,
/// description)` pairs in table order (see
/// [`super::keymap::Keymap::continuations_for`]); a no-op if empty, which
/// shouldn't happen for a genuinely pending prefix.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    footer_top: u16,
    rows: &[(String, &'static str)],
    theme: &Theme,
) {
    if rows.is_empty() {
        return;
    }

    let key_width = rows
        .iter()
        .map(|(k, _)| k.chars().count())
        .max()
        .unwrap_or(0);
    let desc_width = rows
        .iter()
        .map(|(_, d)| d.chars().count())
        .max()
        .unwrap_or(0);
    // key column + "   " separator + description column, plus the two
    // border columns.
    let content_width = (key_width + 3 + desc_width) as u16;
    let width = content_width.saturating_add(2).min(area.width.max(1));
    let height = (rows.len() as u16)
        .saturating_add(2)
        .min(area.height.max(1));
    let top = footer_top.saturating_sub(height).max(area.y);
    let popup = Rect {
        x: area.x,
        y: top,
        width,
        height,
    };

    frame.render_widget(Clear, popup);
    let block = Block::default().borders(Borders::ALL);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let lines: Vec<Line> = rows
        .iter()
        .map(|(key, description)| {
            Line::from(vec![
                Span::styled(
                    format!("{key:<key_width$}"),
                    Style::default()
                        .fg(theme.help_key)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("   "),
                Span::raw(*description),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    fn key(code: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(code), KeyModifiers::NONE)
    }

    #[test]
    fn hidden_with_no_prefix_pending_regardless_of_elapsed() {
        assert!(!should_show(
            None,
            Some(Duration::from_secs(10)),
            WHICH_KEY_DELAY
        ));
    }

    #[test]
    fn hidden_below_the_threshold() {
        assert!(!should_show(
            Some(key('g')),
            Some(Duration::from_millis(499)),
            WHICH_KEY_DELAY
        ));
    }

    #[test]
    fn shown_exactly_at_the_threshold() {
        assert!(should_show(
            Some(key('g')),
            Some(Duration::from_millis(500)),
            WHICH_KEY_DELAY
        ));
    }

    #[test]
    fn shown_past_the_threshold() {
        assert!(should_show(
            Some(key('g')),
            Some(Duration::from_secs(2)),
            WHICH_KEY_DELAY
        ));
    }

    #[test]
    fn hidden_when_elapsed_is_not_yet_known() {
        // Defensive: a pending prefix whose `pending_since` hasn't produced
        // an elapsed reading yet (shouldn't happen in practice, since it's
        // set the same tick `pending` becomes `Some`) never shows.
        assert!(!should_show(Some(key('g')), None, WHICH_KEY_DELAY));
    }
}
