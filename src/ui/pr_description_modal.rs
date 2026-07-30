//! Renders the read-only PR/MR description overlay
//! ([`super::app::Mode::PrDescription`], state in [`super::pr_description`]):
//! a centered bordered block with `#N <title>` as its title (plus a `draft`
//! marker), a secondary meta line (`author · head → base · updated`), a blank
//! line, and then the description body.
//!
//! The body renders as **plain text** — no markdown engine (that would be a
//! new dependency; see the repo's dependency rule). Author line breaks are
//! preserved exactly and long lines soft-wrap to the modal's width through the
//! shared [`super::textwrap`] layout, so every rendered row is one terminal
//! line. That matters for scrolling: because wrapping happens here rather than
//! inside a `Paragraph`, the row count is known, and the caller's scroll offset
//! is clamped against it each frame (the same `Cell` clamp
//! [`super::help::render`] performs) and written back — so a short description
//! never scrolls at all and a long one stops at its last row.
//!
//! Three body states, one of which is always shown — never a blank region:
//! `loading…` while the first fetch is in flight, a dim `no description` for
//! an empty body, and a dim `description unavailable` for a failed read (the
//! silent-degradation contract in [`super::pr_description`]'s module doc).

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Padding, Paragraph};

use crate::forge::PrDetail;

use super::app::{App, Mode};
use super::modal_keys::{ModalBinding, PrDescriptionAction};
use super::pr_description::{PrDescriptionReturn, PrDetailOutcome};
use super::theme::Theme;
use super::time_format::{now_unix, parse_rfc3339_to_unix, relative_time};

/// Centers a `width_pct`% x `height_pct`% rect inside `area` — the same
/// two-axis `Flex::Center` sizing [`super::forge_threads`]' overlay uses.
fn centered(area: Rect, width_pct: u16, height_pct: u16) -> Rect {
    let [area] = Layout::horizontal([Constraint::Percentage(width_pct)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::vertical([Constraint::Percentage(height_pct)])
        .flex(Flex::Center)
        .areas(area);
    area
}

/// The block title: `#N <title>`, with ` (draft)` appended for a draft PR so
/// the state is visible before the reviewer commits to reading the diff.
fn title_text(detail: &PrDetail) -> String {
    let mut text = if detail.title.is_empty() {
        format!("#{}", detail.number)
    } else {
        format!("#{} {}", detail.number, detail.title)
    };
    if detail.is_draft {
        text.push_str(" (draft)");
    }
    text
}

/// The secondary meta line: `author · head → base · <relative time>`. The
/// timestamp relatives through the shared [`relative_time`] helper (matching
/// the PR picker rows and the thread overlay), falling back to the raw
/// provider string when it doesn't parse — the same degrade the picker row
/// takes rather than blanking the field.
fn meta_text(detail: &PrDetail, now: i64) -> String {
    let when = parse_rfc3339_to_unix(&detail.updated_at)
        .map(|ts| relative_time(now, ts))
        .unwrap_or_else(|| detail.updated_at.clone());
    format!(
        "{} \u{b7} {} \u{2192} {} \u{b7} {}",
        detail.author, detail.head_ref, detail.base_ref, when
    )
}

/// Soft-wraps `text`'s logical lines at `width` columns, preserving every
/// author line break (an empty line stays an empty row) — one returned string
/// per rendered terminal row, which is what makes the scroll clamp exact.
fn wrapped(text: &str, width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    for line in text.split('\n') {
        for (start, end) in super::textwrap::wrap_ranges(line, width) {
            rows.push(line.chars().skip(start).take(end - start).collect());
        }
    }
    rows
}

/// Clamps a requested scroll offset to the last full screen of `total` rows in
/// a `viewport`-row region. Content that fits never scrolls (max offset 0), so
/// a short description can't be scrolled off the top; a `u16::MAX` request
/// lands exactly on the last screen. Pure — the render pass calls this and
/// writes the result back into the caller's `Cell`.
fn clamp_scroll(requested: u16, total: u16, viewport: u16) -> u16 {
    requested.min(total.saturating_sub(viewport))
}

/// The body region's rows for the overlay's current state: the wrapped
/// description, or one of the three single-line states (loading / empty /
/// unavailable) styled as secondary text so they read as chrome rather than as
/// the author's prose.
fn body_lines(
    outcome: Option<&PrDetailOutcome>,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let dim = Style::default()
        .fg(theme.footer_text)
        .add_modifier(Modifier::DIM);
    match outcome {
        None => vec![Line::from(Span::styled("loading\u{2026}", dim))],
        Some(PrDetailOutcome::Unavailable) => {
            vec![Line::from(Span::styled("description unavailable", dim))]
        }
        Some(PrDetailOutcome::Ready(detail)) if detail.body.trim().is_empty() => {
            vec![Line::from(Span::styled("no description", dim))]
        }
        Some(PrDetailOutcome::Ready(detail)) => wrapped(&detail.body, width)
            .into_iter()
            .map(|row| {
                Line::from(Span::styled(
                    row,
                    Style::default().fg(theme.annotation_text),
                ))
            })
            .collect(),
    }
}

/// The bottom-border hint line, keys read from the *effective* table so a
/// remap shows up here with no extra wiring. `StartReview` is dropped when the
/// overlay was opened from inside a review session: `Enter` genuinely does
/// nothing there (the PR is already open), and advertising it would be
/// untruthful — the same "don't hint a key that does nothing visible here"
/// rule the launcher's tab-scoped footer follows.
fn hint_line(table: &[ModalBinding<PrDescriptionAction>], ret: PrDescriptionReturn) -> String {
    table
        .iter()
        .filter(|b| {
            b.action != PrDescriptionAction::StartReview
                || matches!(ret, PrDescriptionReturn::Launcher { .. })
        })
        .filter_map(|b| {
            let hint = b.footer?;
            let key = b.keys.first()?.label();
            Some(format!("{key} {}", hint.label))
        })
        .collect::<Vec<_>>()
        .join("  ")
}

/// Renders the description overlay, centered over `area`. A no-op outside
/// [`Mode::PrDescription`], or when no overlay state is present.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Mode::PrDescription { ret } = app.mode else {
        return;
    };
    let Some(state) = app.pr_description.as_ref() else {
        return;
    };
    let popup = centered(area, 70, 60);
    frame.render_widget(Clear, popup);

    let outcome = app.pr_detail_for(state.number);
    let title = match outcome {
        Some(PrDetailOutcome::Ready(detail)) => title_text(detail),
        // Before the body lands (or after a failed read) the number is all
        // that's known for certain — never another PR's cached title.
        _ => format!("#{}", state.number),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .padding(Padding::horizontal(1))
        .title(format!(" {title} "))
        .title_bottom(Line::from(format!(
            " {} ",
            hint_line(&app.modal_keys.pr_description, ret)
        )));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let width = inner.width as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(PrDetailOutcome::Ready(detail)) = outcome {
        for row in wrapped(&meta_text(detail, now_unix()), width) {
            lines.push(Line::from(Span::styled(
                row,
                Style::default().fg(app.theme.dir_prefix),
            )));
        }
        lines.push(Line::from(String::new()));
    }
    lines.extend(body_lines(outcome, width, &app.theme));

    // Clamp the caller's offset now that both the content height and the
    // viewport are known, and record the height for future paging.
    let total = lines.len() as u16;
    let offset = clamp_scroll(state.scroll.get(), total, inner.height);
    state.scroll.set(offset);
    state.viewport.set(inner.height);

    frame.render_widget(Paragraph::new(lines).scroll((offset, 0)), inner);
}

#[cfg(test)]
#[path = "pr_description_modal_tests.rs"]
mod tests;
