//! The submit-review modal and its publish driver: the feature's payoff and
//! its one live forge write. `submit-forge-review` (a review session on a
//! forge PR only) opens [`Mode::SubmitForge`] — a grouped-by-file preview of
//! every unpublished annotation and drafted reply, a capability-driven verdict
//! picker, and an optional summary — and **nothing is sent until the reviewer
//! confirms from here** (the spec's safety boundary). On confirm the batch is
//! resolved on the render thread and handed to a background submit sequence
//! (see [`crate::forge::run_submit_sequence`]) through the fake-able
//! [`super::stage_ops::AsyncForgeSubmitter`] seam, single-flight so a second
//! confirm can't double-publish; each item is marked published as its own
//! write lands, persisted to schema v3 as it goes, and a mid-sequence failure
//! stops with a published/unpublished split reported in the status line — a
//! re-submit then sends only the remainder.
//!
//! Agents never reach the live write: their fakes return the default `None`
//! submitter, so a fake-provider test exercises the marking/persist/split logic
//! via [`App::apply_submit_outcome`] directly without spawning anything.

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::annotate::{Annotation, Classification, Target};
use crate::forge::{Capabilities, SubmitBatch, SubmitReplyItem, SubmitReport, Verdict};
use crate::review::store::ForgeProviderKind;

use super::app::{App, Mode};
use super::background::TaskId;

/// A background submit run awaiting completion: its [`TaskId`] and the
/// generation captured at spawn (a straggler from a superseded run is
/// dropped). Same shape as [`super::forge_threads::InFlightThreadFetch`].
#[derive(Debug, Clone, Copy)]
pub(super) struct InFlightSubmit {
    pub(super) id: TaskId,
    pub(super) generation: u64,
}

/// The submit modal's editable state: the chosen verdict (an index into the
/// provider's supported `verdicts`, so the picker can only land on a verdict
/// that will really work — GitHub offers all three, GitLab v1 has no
/// request-changes), the optional summary body being typed, and the
/// pre-rendered `#N on host/org/repo` target line.
#[derive(Debug, Clone)]
pub(super) struct SubmitForgeState {
    pub(super) verdicts: Vec<Verdict>,
    pub(super) verdict_index: usize,
    pub(super) summary: String,
    pub(super) target_line: String,
    /// An honest one-line disclosure of the provider's submit shape, shown
    /// above the batch when the provider stages a near-atomic draft batch
    /// (GitLab) — `None` for GitHub, whose one visible review POST needs no
    /// caveat.
    pub(super) disclosure: Option<String>,
    /// A transient validation message shown in the modal when a confirm is
    /// blocked (e.g. request-changes with no summary). Cleared the moment the
    /// reviewer edits the verdict or summary.
    pub(super) hint: Option<String>,
}

impl SubmitForgeState {
    /// The currently selected verdict.
    pub(super) fn verdict(&self) -> Verdict {
        self.verdicts
            .get(self.verdict_index)
            .copied()
            .unwrap_or(Verdict::Comment)
    }
}

/// Which verdicts a provider supports, from its [`Capabilities`] — Comment is
/// always offered; Approve and Request changes only when the capability flag
/// is set. Drives the modal's verdict picker so it renders exactly the
/// choices that will really work (FR-17's capability-driven rendering).
pub(super) fn verdicts_for(caps: Capabilities) -> Vec<Verdict> {
    let mut v = vec![Verdict::Comment];
    if caps.can_approve {
        v.push(Verdict::Approve);
    }
    if caps.can_request_changes {
        v.push(Verdict::RequestChanges);
    }
    v
}

/// The provider's submit capabilities, mapped from the review's forge kind.
/// GitHub supports all three verdicts and posts each item visibly (no
/// near-atomic draft staging); GitLab v1 has approve but no request-changes
/// and stages drafts (its submit path is a later unit — the flags leave room).
pub(super) fn capabilities_for(kind: ForgeProviderKind) -> Capabilities {
    match kind {
        ForgeProviderKind::GitHub => Capabilities {
            can_approve: true,
            can_request_changes: true,
            near_atomic_submit: false,
        },
        ForgeProviderKind::GitLab => Capabilities {
            can_approve: true,
            can_request_changes: false,
            near_atomic_submit: true,
        },
    }
}

/// An honest disclosure of the provider's submit shape, for a provider that
/// stages a near-atomic draft batch (GitLab). Deliberately names no version
/// number (Open Question 4): the draft-notes vs. visible-comments split turns
/// on the instance's API vintage, which redquill can't reliably detect and
/// which would drift as a hard-coded threshold — so the copy states both
/// behaviors and lets the outcome speak, rather than promising a version. Read
/// straight from [`Capabilities`], so it tracks whatever a provider declares.
pub(super) fn submit_disclosure(caps: Capabilities) -> Option<String> {
    caps.near_atomic_submit.then(|| {
        "Comments post as private drafts, published together on confirm; \
         older GitLab instances receive them as visible comments instead."
            .to_string()
    })
}

/// The human label for a verdict in the picker.
fn verdict_label(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Comment => "comment",
        Verdict::Approve => "approve",
        Verdict::RequestChanges => "request changes",
    }
}

/// How an annotation publishes, for its preview label: as a positioned review
/// comment, a separate file-level comment, or not at all (worktree-anchored,
/// local-only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PreviewNote {
    /// A `Line`/`Range`/`Hunk` target — rides in the reviews-endpoint array.
    LineComment,
    /// A `File` target — posts as a separate file comment (FR-19).
    FileComment,
    /// A worktree-anchored target — has no forge position, never published.
    LocalOnly,
}

impl PreviewNote {
    /// The classification of `target`'s publish path.
    fn of(target: &Target) -> PreviewNote {
        match target {
            Target::Line { .. } | Target::Range { .. } | Target::Hunk { .. } => {
                PreviewNote::LineComment
            }
            Target::File { .. } => PreviewNote::FileComment,
            Target::WorktreeLine { .. } | Target::WorktreeRange { .. } => PreviewNote::LocalOnly,
        }
    }

    /// The trailing note shown after a preview row, or `None` for a plain
    /// positioned comment (the common case needs no annotation).
    fn label(self) -> Option<&'static str> {
        match self {
            PreviewNote::LineComment => None,
            PreviewNote::FileComment => Some("posts as a separate file comment"),
            PreviewNote::LocalOnly => Some("local-only \u{2014} will not publish"),
        }
    }
}

/// One annotation's preview row: its in-file anchor, classification, a
/// one-line body summary, and how it publishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AnnotationPreview {
    pub(super) anchor: String,
    pub(super) classification: Classification,
    pub(super) summary: String,
    pub(super) note: PreviewNote,
}

/// One file's group of annotation previews, in insertion order within the
/// file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FileGroup {
    pub(super) path: String,
    pub(super) items: Vec<AnnotationPreview>,
}

/// One drafted reply's preview row: which thread it answers and its body
/// summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReplyPreview {
    pub(super) thread_id: u64,
    pub(super) summary: String,
}

/// The whole batch preview, grouped by file with replies listed separately
/// (a reply answers a thread, not a diff line). Pure — built from the
/// unpublished annotation/reply sets — so grouping and labels are unit-tested
/// without constructing a modal.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct SubmitPreview {
    pub(super) groups: Vec<FileGroup>,
    pub(super) replies: Vec<ReplyPreview>,
}

/// The in-file anchor label for an annotation (`path:line`, `path:start-end`,
/// or `path` for a whole-file target), for the preview.
fn anchor_label(target: &Target) -> String {
    match target {
        Target::Line { path, line, .. } => format!("{path}:{line}"),
        Target::Range {
            path, start, end, ..
        } => format!("{path}:{start}-{end}"),
        Target::Hunk { path, start, end } => format!("{path}:{start}-{end}"),
        Target::File { path } => path.clone(),
        Target::WorktreeLine { path, line } => format!("{path}:{line}"),
        Target::WorktreeRange { path, start, end } => format!("{path}:{start}-{end}"),
    }
}

/// A body's first line, for the one-line preview summary.
fn first_line(body: &str) -> String {
    body.lines().next().unwrap_or("").to_string()
}

/// Builds the grouped preview from the unpublished annotations and replies —
/// annotations grouped by file in first-seen order, replies in insertion
/// order. Pure; the caller passes the already-filtered unpublished sets.
pub(super) fn build_preview<'a>(
    annotations: impl Iterator<Item = &'a Annotation>,
    replies: impl Iterator<Item = (u64, &'a str)>,
) -> SubmitPreview {
    let mut groups: Vec<FileGroup> = Vec::new();
    for annotation in annotations {
        let path = annotation.target.path().to_string();
        let item = AnnotationPreview {
            anchor: anchor_label(&annotation.target),
            classification: annotation.classification,
            summary: first_line(&annotation.body),
            note: PreviewNote::of(&annotation.target),
        };
        match groups.iter_mut().find(|g| g.path == path) {
            Some(group) => group.items.push(item),
            None => groups.push(FileGroup {
                path,
                items: vec![item],
            }),
        }
    }
    let replies = replies
        .map(|(thread_id, body)| ReplyPreview {
            thread_id,
            summary: first_line(body),
        })
        .collect();
    SubmitPreview { groups, replies }
}

impl App {
    /// The imperative `submit-forge-review` entry: opens the modal in a forge
    /// PR review session, or leaves a one-line hint everywhere else (a plain
    /// review session with no forge PR, or no review at all — the action is
    /// listed in the Review group but inert outside a PR review, matching the
    /// "no-op with a hint" contract). A no-op with a hint while a prior submit
    /// is still publishing (single-flight).
    pub(super) fn open_submit_forge(&mut self) {
        let Some(forge) = self.review_forge.clone() else {
            self.set_status_message("submit unavailable \u{2014} not a PR review");
            return;
        };
        if self.forge_submit_in_flight.is_some() {
            self.set_status_message("submit already in progress");
            return;
        }
        let caps = capabilities_for(forge.provider);
        let verdicts = verdicts_for(caps);
        let disclosure = submit_disclosure(caps);
        let slug = self
            .stage_ops()
            .and_then(|ops| ops.origin_repo_slug())
            .unwrap_or_else(|| forge.host.clone());
        let target_line = format!("#{} on {}/{}", forge.number, forge.host, slug);
        self.submit_forge = Some(SubmitForgeState {
            verdicts,
            verdict_index: 0,
            summary: String::new(),
            target_line,
            disclosure,
            hint: None,
        });
        self.mode = Mode::SubmitForge;
    }

    /// Closes the modal without submitting anything, returning to Normal.
    pub(super) fn close_submit_forge(&mut self) {
        self.submit_forge = None;
        self.mode = Mode::Normal;
    }

    /// Cycles the verdict picker forward (wrapping), clamped to the provider's
    /// supported set.
    pub(super) fn submit_forge_verdict_next(&mut self) {
        if let Some(state) = self.submit_forge.as_mut()
            && !state.verdicts.is_empty()
        {
            state.verdict_index = (state.verdict_index + 1) % state.verdicts.len();
            state.hint = None;
        }
    }

    /// Cycles the verdict picker backward (wrapping).
    pub(super) fn submit_forge_verdict_prev(&mut self) {
        if let Some(state) = self.submit_forge.as_mut()
            && !state.verdicts.is_empty()
        {
            let n = state.verdicts.len();
            state.verdict_index = (state.verdict_index + n - 1) % n;
            state.hint = None;
        }
    }

    /// Appends a typed character to the summary (the modal's free-text field).
    pub(super) fn submit_forge_insert_char(&mut self, c: char) {
        if let Some(state) = self.submit_forge.as_mut() {
            state.summary.push(c);
            state.hint = None;
        }
    }

    /// Deletes the last summary character.
    pub(super) fn submit_forge_delete_char(&mut self) {
        if let Some(state) = self.submit_forge.as_mut() {
            state.summary.pop();
            state.hint = None;
        }
    }

    /// The confirm gesture: reads the chosen verdict + summary, closes the
    /// modal, and spawns the background submit. The only path that ever begins
    /// a forge write.
    pub(super) fn submit_forge_confirm(&mut self) {
        let Some(state) = self.submit_forge.as_mut() else {
            return;
        };
        let verdict = state.verdict();
        let summary = state.summary.trim().to_string();
        // GitHub rejects a request-changes review with no body; block the
        // confirm and keep the modal open with a hint rather than sending a
        // request the forge will 422.
        if verdict == Verdict::RequestChanges && summary.is_empty() {
            state.hint =
                Some("Request changes needs a summary explaining what to change.".to_string());
            return;
        }
        self.submit_forge = None;
        self.mode = Mode::Normal;
        let batch = self.build_submit_batch(
            verdict,
            if summary.is_empty() {
                None
            } else {
                Some(summary.as_str())
            },
        );
        self.spawn_forge_submit(batch);
    }

    /// Builds the batch one submit run publishes from the still-unpublished
    /// annotation and reply sets: the reviews-endpoint plan (line comments +
    /// verdict + summary, local-only targets excluded, file targets routed to
    /// follow-ups) plus the unpublished replies. `include_review_post` is
    /// `false` once this session's review POST already landed, so a resume
    /// skips it rather than re-delivering the verdict.
    pub(super) fn build_submit_batch(
        &self,
        verdict: Verdict,
        summary: Option<&str>,
    ) -> SubmitBatch {
        let unpublished: Vec<Annotation> = self.annotations.unpublished().cloned().collect();
        let plan = crate::forge::build_review_payload(&unpublished, verdict, summary);
        let replies = self
            .replies
            .unpublished()
            .map(|r| SubmitReplyItem {
                reply_id: r.id,
                thread_id: r.thread_id,
                body: r.body.clone(),
            })
            .collect();
        SubmitBatch {
            plan,
            replies,
            include_review_post: !self.forge_review_submitted,
        }
    }

    /// Spawns the background submit sequence for `batch`, single-flight with a
    /// generation guard (a straggler from a superseded run is dropped on
    /// arrival). A no-op — with a hint — when the backend can't run a live
    /// submit (test fakes, git-less contexts, a non-forge session): nothing is
    /// ever sent off the fake path.
    pub(super) fn spawn_forge_submit(&mut self, batch: SubmitBatch) {
        self.forge_submit_generation = self.forge_submit_generation.wrapping_add(1);
        self.forge_submit_in_flight = None;
        let Some(forge) = self.review_forge.clone() else {
            return;
        };
        // Resolve each still-unpublished reply's GitLab discussion string id
        // from the live thread overlay (GitLab replies target the discussion,
        // not the root note id). Empty for GitHub, which ignores it.
        let reply_discussions: std::collections::HashMap<usize, String> = self
            .replies
            .unpublished()
            .filter_map(|r| {
                self.thread_overlay
                    .find(r.thread_id)
                    .and_then(|t| t.discussion_id.clone())
                    .map(|discussion_id| (r.id, discussion_id))
            })
            .collect();
        let submitter = self.stage_ops().and_then(|ops| {
            ops.async_forge_submitter(
                forge.provider,
                forge.number,
                forge.last_head_sha.clone(),
                reply_discussions,
            )
        });
        let Some(submitter) = submitter else {
            self.set_status_message("submit unavailable (no forge backend)");
            return;
        };
        let generation = self.forge_submit_generation;
        let id = self.forge_submit_tasks.spawn(move || submitter(batch));
        self.forge_submit_in_flight = Some(InFlightSubmit { id, generation });
    }

    /// Drains a completed background submit (once per event-loop tick), drops a
    /// stale result from a superseded run, and otherwise applies it.
    pub(super) fn poll_forge_submit(&mut self) {
        for (id, result) in self.forge_submit_tasks.poll() {
            let Some(in_flight) = self.forge_submit_in_flight else {
                continue;
            };
            if in_flight.id != id {
                continue;
            }
            self.forge_submit_in_flight = None;
            if in_flight.generation != self.forge_submit_generation {
                continue;
            }
            let report = match result {
                Ok(report) => report,
                Err(_panic) => SubmitReport {
                    failure: Some("submit task panicked".to_string()),
                    ..SubmitReport::default()
                },
            };
            self.apply_submit_outcome(report);
        }
    }

    /// Applies a submit report: marks each published annotation/reply, records
    /// whether the review POST has now landed (so a resume skips it), persists
    /// the new published state (schema v3), and sets a status line — either the
    /// clean "review submitted" or the published/unpublished split when a write
    /// failed mid-sequence. Rebuilds rows so a now-published annotation's
    /// in-diff row can defer to its forge copy.
    pub(super) fn apply_submit_outcome(&mut self, report: SubmitReport) {
        for id in &report.published_annotation_ids {
            let _ = self.annotations.set_published(*id, true);
        }
        for id in &report.published_reply_ids {
            self.replies.set_published(*id, true);
        }
        if report.review_submitted {
            self.forge_review_submitted = true;
        }
        self.persist_review_state();
        let published = report.published_annotation_ids.len() + report.published_reply_ids.len();
        let message = match &report.failure {
            None => format!("review submitted \u{2014} {published} item(s) published"),
            Some(diagnostic) => {
                let remaining =
                    self.annotations.unpublished().count() + self.replies.unpublished().count();
                format!(
                    "submit stopped: {published} published, {remaining} unpublished \u{2014} {diagnostic}"
                )
            }
        };
        self.set_status_message(message);
        self.rebuild_rows();
    }
}

/// Centers a `width_pct`% x `height_pct`% rect inside `area` (same helper
/// shape as [`super::forge_threads`]'s `centered`).
fn centered(area: Rect, width_pct: u16, height_pct: u16) -> Rect {
    let [area] = Layout::horizontal([Constraint::Percentage(width_pct)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::vertical([Constraint::Percentage(height_pct)])
        .flex(Flex::Center)
        .areas(area);
    area
}

/// Renders the submit-review modal, centered over `area`. A no-op when the
/// modal isn't open. Shows the target line, the verdict picker (selected
/// verdict emphasized), the grouped-by-file batch preview with local-only and
/// file-comment labels, the summary being typed, and an unmistakable
/// nothing-sent-until-confirm footer.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(state) = &app.submit_forge else {
        return;
    };
    let theme = &app.theme;
    let popup = centered(area, 72, 72);
    frame.render_widget(Clear, popup);

    let preview = build_preview(
        app.annotations.unpublished(),
        app.replies
            .unpublished()
            .map(|r| (r.thread_id, r.body.as_str())),
    );

    let mut lines: Vec<Line> = Vec::new();

    // Target line — which PR this lands on.
    lines.push(Line::from(Span::styled(
        format!("Submitting review to {}", state.target_line),
        Style::default()
            .fg(theme.hunk_header)
            .add_modifier(Modifier::BOLD),
    )));
    // Honest submit-shape disclosure (GitLab's draft/visible split).
    if let Some(disclosure) = &state.disclosure {
        lines.push(Line::from(Span::styled(
            disclosure.clone(),
            Style::default()
                .fg(theme.gutter)
                .add_modifier(Modifier::DIM),
        )));
    }
    lines.push(Line::from(String::new()));

    // Verdict picker — the selected one emphasized.
    let mut verdict_spans: Vec<Span> =
        vec![Span::styled("Verdict: ", Style::default().fg(theme.gutter))];
    for (i, verdict) in state.verdicts.iter().enumerate() {
        let selected = i == state.verdict_index;
        let style = if selected {
            Style::default()
                .fg(theme.help_key)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            Style::default().fg(theme.footer_text)
        };
        verdict_spans.push(Span::styled(
            format!(" {} ", verdict_label(*verdict)),
            style,
        ));
    }
    lines.push(Line::from(verdict_spans));
    lines.push(Line::from(String::new()));

    // The batch preview, grouped by file.
    if preview.groups.is_empty() && preview.replies.is_empty() {
        lines.push(Line::from(Span::styled(
            "No unpublished comments or replies \u{2014} a verdict-only review.",
            Style::default()
                .fg(theme.gutter)
                .add_modifier(Modifier::DIM),
        )));
    }
    for group in &preview.groups {
        lines.push(Line::from(Span::styled(
            group.path.clone(),
            Style::default()
                .fg(theme.help_section_header)
                .add_modifier(Modifier::BOLD),
        )));
        for item in &group.items {
            let mut spans = vec![
                Span::styled(
                    format!("  {}  ", item.anchor),
                    Style::default().fg(theme.gutter),
                ),
                Span::styled(
                    format!("[{}] ", item.classification.label()),
                    Style::default().fg(theme.hunk_header),
                ),
                Span::styled(
                    item.summary.clone(),
                    Style::default().fg(theme.annotation_text),
                ),
            ];
            if let Some(note) = item.note.label() {
                spans.push(Span::styled(
                    format!("  ({note})"),
                    Style::default()
                        .fg(theme.gutter)
                        .add_modifier(Modifier::DIM),
                ));
            }
            lines.push(Line::from(spans));
        }
    }
    if !preview.replies.is_empty() {
        lines.push(Line::from(String::new()));
        lines.push(Line::from(Span::styled(
            "Replies to threads",
            Style::default()
                .fg(theme.help_section_header)
                .add_modifier(Modifier::BOLD),
        )));
        for reply in &preview.replies {
            lines.push(Line::from(Span::styled(
                format!("  \u{21b3} thread {}: {}", reply.thread_id, reply.summary),
                Style::default().fg(theme.annotation_text),
            )));
        }
    }

    // Summary field.
    lines.push(Line::from(String::new()));
    let summary_display = if state.summary.is_empty() {
        Span::styled(
            "(optional summary \u{2014} type to add)",
            Style::default()
                .fg(theme.gutter)
                .add_modifier(Modifier::DIM),
        )
    } else {
        Span::styled(
            state.summary.clone(),
            Style::default().fg(theme.annotation_text),
        )
    };
    lines.push(Line::from(vec![
        Span::styled("Summary: ", Style::default().fg(theme.gutter)),
        summary_display,
    ]));

    // A blocked-confirm validation hint (e.g. request-changes with no summary).
    if let Some(hint) = &state.hint {
        lines.push(Line::from(String::new()));
        lines.push(Line::from(Span::styled(
            hint.clone(),
            Style::default()
                .fg(theme.status_message)
                .add_modifier(Modifier::BOLD),
        )));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Submit review \u{2014} nothing is sent until you confirm")
        .title_bottom(Line::from(
            " Enter submit  Esc cancel  Tab/Shift-Tab verdict  type summary ",
        ));
    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, popup);
}

#[cfg(test)]
#[path = "forge_submit_tests.rs"]
mod tests;
