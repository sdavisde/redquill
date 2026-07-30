//! The post-submit result view: a read-only modal that names, item by item,
//! what a stopped submit run actually landed. The one-line status a partial
//! failure leaves ("submit stopped: 3 published, 2 not sent — …") is honest
//! about the counts but says nothing about *which* comments made it, which
//! left opening the PR in a browser as the only way to find out. This modal
//! answers that question from the run's own report.
//!
//! Opened by [`super::forge_submit::App::apply_submit_outcome`] only when the
//! run reported a failure; a clean submit keeps the one-line status and no
//! modal. The status line is set either way, so dismissing the modal leaves
//! the outcome recorded where it always was.
//!
//! Every row comes from the [`SubmitReport`] the sequence produced — its
//! attempted set joined against its published and drafted lists (see
//! [`crate::forge::SubmitAttempt`]) — never from a diff of local state, so an
//! item is reported unsent only when the run really never reached it. The
//! labels are [`super::forge_submit`]'s own preview labels, so a comment reads
//! the same before and after it is sent.

use std::cell::Cell;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::annotate::AnnotationStore;
use crate::forge::{ItemOutcome, SubmitReport, ThreadOverlayStore};

use super::app::{App, Mode, ModeOrigin};
use super::draft_reply::DraftReplyStore;
use super::forge_submit::{
    above_marker, annotation_preview, below_marker, reply_preview, reply_preview_label,
    resolve_scroll, wrap_line,
};
use super::theme::Theme;

/// The grouped per-item outcome of one stopped submit run: three lists of
/// display labels plus the diagnostic that stopped it. Pure data built by
/// [`build_result`], so the grouping is unit-tested without a frame.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct SubmitResultView {
    /// Items visible on the forge now.
    pub(super) published: Vec<String>,
    /// Items staged as private GitLab drafts, awaiting a publish.
    pub(super) pending_drafts: Vec<String>,
    /// Items the run never reached — plus, first, the review itself when the
    /// verdict/summary POST is what failed (everything after it is unsent by
    /// construction, so it belongs at the top of this group).
    pub(super) not_sent: Vec<String>,
    /// The one-line diagnostic from the run (already carrying
    /// `submit_error_headline`'s token-scope hint where one applies).
    pub(super) diagnostic: String,
}

impl SubmitResultView {
    /// Whether the run reported no per-item outcome at all — only reachable
    /// when it failed before recording what it meant to send (a panicked
    /// submit task). The modal says so rather than showing three empty groups.
    fn is_itemless(&self) -> bool {
        self.published.is_empty() && self.pending_drafts.is_empty() && self.not_sent.is_empty()
    }
}

/// The modal's live state: the outcome it shows (built once when the report
/// arrives — the stores it was labeled from go on changing) and the scroll
/// offset, clamped by [`render`] against the real rendered line count.
#[derive(Debug)]
pub(super) struct SubmitResultState {
    pub(super) view: SubmitResultView,
    pub(super) scroll: Cell<u16>,
    /// The scrollable body's height, recorded each frame so the page keys page
    /// by a real viewport.
    pub(super) viewport: Cell<u16>,
}

/// The label a row shows for annotation `id`: its preview anchor,
/// classification, and one-line body. Falls back to the bare id if the
/// annotation is somehow gone from the store, so a row is never dropped.
fn annotation_label(id: usize, annotations: &AnnotationStore) -> String {
    match annotations.iter().find(|a| a.id == id) {
        Some(annotation) => {
            let preview = annotation_preview(annotation);
            format!(
                "{}  [{}] {}",
                preview.anchor,
                preview.classification.label(),
                preview.summary
            )
        }
        None => format!("annotation #{id}"),
    }
}

/// The label a row shows for reply `id`: the same "to <author> @ <anchor>"
/// naming the submit preview uses, or the bare thread id when the thread has
/// dropped out of the overlay.
fn reply_label(id: usize, replies: &DraftReplyStore, threads: &ThreadOverlayStore) -> String {
    match replies.get(id) {
        Some(reply) => format!(
            "\u{21b3} {}",
            reply_preview_label(&reply_preview(reply.thread_id, &reply.body, threads))
        ),
        None => format!("\u{21b3} reply #{id}"),
    }
}

/// The row naming the review itself in the not-sent group.
const REVIEW_NOT_POSTED: &str = "the review itself (verdict + summary) was not posted";

/// Groups one run's report into the three outcome lists, labeling each item
/// through the submit preview's own naming. Annotations come before replies
/// within a group, each in the run's send order.
pub(super) fn build_result(
    report: &SubmitReport,
    annotations: &AnnotationStore,
    replies: &DraftReplyStore,
    threads: &ThreadOverlayStore,
) -> SubmitResultView {
    let mut view = SubmitResultView {
        diagnostic: report.failure.clone().unwrap_or_default(),
        ..SubmitResultView::default()
    };
    if report.review_post_not_sent() {
        view.not_sent.push(REVIEW_NOT_POSTED.to_string());
    }
    let rows = report
        .annotation_outcomes()
        .into_iter()
        .map(|(id, outcome)| (annotation_label(id, annotations), outcome))
        .chain(
            report
                .reply_outcomes()
                .into_iter()
                .map(|(id, outcome)| (reply_label(id, replies, threads), outcome)),
        );
    for (label, outcome) in rows {
        match outcome {
            ItemOutcome::Published => view.published.push(label),
            ItemOutcome::PendingDraft => view.pending_drafts.push(label),
            ItemOutcome::NotSent => view.not_sent.push(label),
        }
    }
    view
}

impl App {
    /// Opens the result modal on a stopped run's report, capturing whatever
    /// mode the reviewer was in so dismissing restores it exactly (the
    /// [`ModeOrigin`] contract every other modal uses). The report arrives
    /// asynchronously, so this can interrupt any mode; the interrupted mode's
    /// own state lives on `App` and survives the round trip untouched.
    pub(super) fn open_submit_result(&mut self, report: &SubmitReport) {
        let view = build_result(
            report,
            &self.annotations,
            &self.replies,
            &self.thread_overlay,
        );
        self.submit_result = Some(SubmitResultState {
            view,
            scroll: Cell::new(0),
            viewport: Cell::new(0),
        });
        self.mode = Mode::SubmitResult {
            origin: ModeOrigin::capture(self.mode),
        };
    }

    /// Dismisses the result modal, restoring the mode it interrupted.
    pub(super) fn close_submit_result(&mut self) {
        let origin = match self.mode {
            Mode::SubmitResult { origin } => origin,
            _ => ModeOrigin::Normal,
        };
        self.submit_result = None;
        self.mode = origin.restore();
    }

    /// Dismisses the modal and reopens the submit modal for another pass. The
    /// batch is rebuilt there from the still-unpublished items, so a retry
    /// re-sends nothing that already landed.
    pub(super) fn submit_result_retry(&mut self) {
        self.close_submit_result();
        self.open_submit_forge();
    }

    /// Scrolls the result list down one line. [`render`] clamps the offset to
    /// the content, so an overshoot here can't run off the end.
    pub(super) fn submit_result_scroll_down(&mut self) {
        if let Some(state) = self.submit_result.as_ref() {
            state.scroll.set(state.scroll.get().saturating_add(1));
        }
    }

    /// Scrolls the result list up one line.
    pub(super) fn submit_result_scroll_up(&mut self) {
        if let Some(state) = self.submit_result.as_ref() {
            state.scroll.set(state.scroll.get().saturating_sub(1));
        }
    }

    /// Scrolls down a full viewport (the height the last frame recorded).
    pub(super) fn submit_result_page_down(&mut self) {
        if let Some(state) = self.submit_result.as_ref() {
            let page = state.viewport.get().max(1);
            state.scroll.set(state.scroll.get().saturating_add(page));
        }
    }

    /// Scrolls up a full viewport.
    pub(super) fn submit_result_page_up(&mut self) {
        if let Some(state) = self.submit_result.as_ref() {
            let page = state.viewport.get().max(1);
            state.scroll.set(state.scroll.get().saturating_sub(page));
        }
    }
}

/// One outcome group's block of lines: a counted header in the group's own
/// style, then its rows. Nothing is emitted for an empty group — a
/// "published (0)" header would be noise on a run that published nothing.
fn group_lines(
    header: String,
    rows: &[String],
    header_style: Style,
    row_style: Style,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![Line::from(Span::styled(header, header_style))];
    lines.extend(
        rows.iter()
            .map(|row| Line::from(Span::styled(format!("  {row}"), row_style))),
    );
    lines.push(Line::from(String::new()));
    lines
}

/// Builds the modal's body: the three outcome groups top to bottom (what
/// landed first, so the reassuring half is read before the bad news), then the
/// diagnostic headline. Split out from [`render`] so the rendered line count —
/// what the scroll math clamps against — is a value the caller holds.
fn build_lines(view: &SubmitResultView, theme: &Theme) -> Vec<Line<'static>> {
    let dim = Style::default()
        .fg(theme.gutter)
        .add_modifier(Modifier::DIM);
    let mut lines: Vec<Line> = Vec::new();
    lines.extend(group_lines(
        format!("\u{2713} published ({})", view.published.len()),
        &view.published,
        Style::default()
            .fg(theme.added_fg)
            .add_modifier(Modifier::BOLD),
        Style::default().fg(theme.annotation_text),
    ));
    lines.extend(group_lines(
        format!(
            "\u{25cc} pending draft ({}) \u{2014} submit again to publish",
            view.pending_drafts.len()
        ),
        &view.pending_drafts,
        Style::default()
            .fg(theme.hunk_header)
            .add_modifier(Modifier::BOLD),
        Style::default().fg(theme.annotation_text),
    ));
    lines.extend(group_lines(
        format!("\u{2717} not sent ({})", view.not_sent.len()),
        &view.not_sent,
        Style::default()
            .fg(theme.removed_fg)
            .add_modifier(Modifier::BOLD),
        Style::default().fg(theme.annotation_text),
    ));
    if view.is_itemless() {
        lines.push(Line::from(Span::styled(
            "The run reported no per-item outcome \u{2014} check the PR before resubmitting.",
            dim,
        )));
        lines.push(Line::from(String::new()));
    }
    if !view.diagnostic.is_empty() {
        lines.push(Line::from(Span::styled(
            view.diagnostic.clone(),
            Style::default()
                .fg(theme.status_message)
                .add_modifier(Modifier::BOLD),
        )));
    }
    lines
}

/// Renders the result modal, centered over `area`. A no-op when it isn't open.
/// A list taller than the modal scrolls (`j`/`k`, the arrows, and the page
/// keys) with a marker row naming how many lines are hidden in each direction,
/// so no outcome is clipped silently.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(state) = &app.submit_result else {
        return;
    };
    let theme = &app.theme;
    let popup = super::forge_submit::centered(area, 72, 72);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Submit stopped \u{2014} what landed and what didn't")
        .title_bottom(Line::from(
            " Enter/Esc dismiss  U submit again  j/k scroll ",
        ));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let lines: Vec<Line> = build_lines(&state.view, theme)
        .into_iter()
        .flat_map(|line| wrap_line(&line, inner.width as usize))
        .collect();

    let total = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    let view = resolve_scroll(total, inner.height, state.scroll.get());
    state.scroll.set(view.offset);
    state.viewport.set(view.body_height.max(1));

    // Two rows means `resolve_scroll` reserved the marker rows; otherwise the
    // whole box is body.
    let body = if inner.height.saturating_sub(view.body_height) == 2 {
        let [top, body, bottom] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .areas(inner);
        if let Some(text) = above_marker(view.hidden_above) {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    text,
                    Style::default()
                        .fg(theme.gutter)
                        .add_modifier(Modifier::DIM),
                ))),
                top,
            );
        }
        if let Some(text) = below_marker(view.hidden_below) {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    text,
                    Style::default()
                        .fg(theme.hunk_header)
                        .add_modifier(Modifier::BOLD),
                ))),
                bottom,
            );
        }
        body
    } else {
        inner
    };

    frame.render_widget(Paragraph::new(lines).scroll((view.offset, 0)), body);
}

#[cfg(test)]
#[path = "forge_submit_result_tests.rs"]
mod tests;
