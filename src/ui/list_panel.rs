//! The annotation list panel: every annotation in insertion order, one line
//! each (`path:line-range (side) [classification] first line of body`),
//! scrollable, with the focused row highlighted. Toggled with `a`. Supports
//! a `/` fuzzy filter (spec 12 FR-7..FR-9): [`filter_label`] is the plain
//! text each annotation is matched against, built from the same summary/body
//! text this module renders so "what you see is what you can filter on."

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

use crate::annotate::{Annotation, Side, Target};
use crate::forge::{Thread, ThreadAnchor};

use super::app::App;
use super::draft_reply::DraftReply;
use super::keymap::{Action, Keymap, Scope};
use super::theme::Theme;

/// One row of the annotation-list panel: either an annotation (by store id)
/// or a drafted reply (by reply id). Annotations always precede replies, in
/// each store's insertion order, so the panel index space is a stable
/// concatenation the filter and every verb translate through
/// [`App::list_real_index`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ListEntry {
    /// An annotation, by its [`AnnotationStore`](crate::annotate::AnnotationStore) id.
    Annotation(usize),
    /// A drafted reply, by its [`DraftReplyStore`](super::draft_reply::DraftReplyStore) id.
    Reply(usize),
}

fn side_marker(side: Side) -> &'static str {
    match side {
        Side::New => "(+)",
        Side::Old => "(-)",
    }
}

/// The `path:line-range (side)` summary of an annotation's target.
fn target_summary(target: &Target) -> String {
    match target {
        Target::Line {
            path, line, side, ..
        } => format!("{path}:{line} {}", side_marker(*side)),
        Target::Range {
            path,
            start,
            end,
            side,
            ..
        } => format!("{path}:{start}-{end} {}", side_marker(*side)),
        Target::Hunk {
            path, start, end, ..
        } => {
            format!("{path}:{start}-{end} {}", side_marker(Side::New))
        }
        Target::File { path } => path.clone(),
        Target::WorktreeLine { path, line } => format!("{path}:{line} (=)"),
        Target::WorktreeRange { path, start, end } => format!("{path}:{start}-{end} (=)"),
    }
}

/// The bracket tag marking a row (annotation or reply) already published to
/// the forge — styled with the same color the git panel uses for "already
/// landed" state (`staged_indicator`), so a published row reads as visually
/// settled the way a fully-staged file does.
const PUBLISHED_TAG: &str = "[published] ";

fn published_span(theme: &Theme) -> Span<'static> {
    Span::styled(PUBLISHED_TAG, Style::default().fg(theme.staged_indicator))
}

fn item_line(annotation: &Annotation, theme: &Theme) -> Line<'static> {
    let first_line = annotation.body.lines().next().unwrap_or("");
    let mut spans = Vec::new();
    if annotation.published {
        spans.push(published_span(theme));
    }
    spans.push(Span::raw(format!(
        "{} ",
        target_summary(&annotation.target)
    )));
    spans.push(Span::styled(
        format!("[{}] ", annotation.classification.label()),
        Style::default().fg(theme.classification_tag),
    ));
    spans.push(Span::raw(first_line.to_string()));
    Line::from(spans)
}

/// The plain-text label [`super::list_filter`]'s fuzzy matcher ranks an
/// annotation against — the same summary/first-line text [`item_line`]
/// renders, minus styling, so "what you see is what you can filter on."
pub(super) fn filter_label(annotation: &Annotation) -> String {
    let first_line = annotation.body.lines().next().unwrap_or("");
    let published_tag = if annotation.published {
        PUBLISHED_TAG
    } else {
        ""
    };
    format!(
        "{published_tag}{} [{}] {first_line}",
        target_summary(&annotation.target),
        annotation.classification.label()
    )
}

/// Where a reply's thread anchors, for the panel row's location column: the
/// resolved `path:line` (or file) when the thread is in the current overlay,
/// falling back to `#<thread_id>` when it isn't (an outdated fetch, or a
/// resumed reply whose thread hasn't been re-fetched yet).
fn reply_where(reply: &DraftReply, thread: Option<&Thread>) -> String {
    match thread.map(|t| &t.anchor) {
        Some(ThreadAnchor::Position { path, line, .. }) => format!("{path}:{line}"),
        Some(ThreadAnchor::File { path }) => path.clone(),
        None => format!("#{}", reply.thread_id),
    }
}

/// One rendered row for a drafted reply: a `↳` reply marker, the thread's
/// location, a `[reply]` tag (styled like a classification tag so replies
/// sit visually alongside annotations), and the reply's first body line.
fn reply_item_line(reply: &DraftReply, thread: Option<&Thread>, theme: &Theme) -> Line<'static> {
    let first_line = reply.body.lines().next().unwrap_or("");
    let mut spans = Vec::new();
    if reply.published {
        spans.push(published_span(theme));
    }
    spans.push(Span::raw(format!(
        "\u{21b3} {} ",
        reply_where(reply, thread)
    )));
    spans.push(Span::styled(
        "[reply] ".to_string(),
        Style::default().fg(theme.classification_tag),
    ));
    spans.push(Span::raw(first_line.to_string()));
    Line::from(spans)
}

/// The plain-text filter label for a drafted reply — the same text
/// [`reply_item_line`] renders, minus styling, including the `reply` marker
/// so a `/` filter for "reply" surfaces exactly the drafted replies.
pub(super) fn reply_filter_label(reply: &DraftReply, thread: Option<&Thread>) -> String {
    let first_line = reply.body.lines().next().unwrap_or("");
    let published_tag = if reply.published { PUBLISHED_TAG } else { "" };
    format!(
        "{published_tag}\u{21b3} {} [reply] {first_line}",
        reply_where(reply, thread)
    )
}

/// Renders the annotation list panel into `area`. An empty store (and no
/// active filter) renders a hint line instead of an empty list; the hint's
/// key is resolved from `keymap` (diff scope, [`Action::Compose`]) rather
/// than hardcoded, so a `[keys.diff]` remap can't leave this text naming a
/// stale key — an unbound action falls back to generic wording rather than
/// showing no key at all.
///
/// A `/` filter (spec 12 FR-7..FR-9) adds a one-row chrome line above the
/// list showing the live/locked query (styled like the help overlay's own
/// filter line), narrows the rendered rows to the filtered view, and shows a
/// "no matches" hint in place of a blank list when a locked filter matches
/// nothing.
pub fn render(frame: &mut Frame, area: Rect, app: &App, keymap: &Keymap) {
    let block = Block::default().borders(Borders::ALL).title("notes");

    if app.annotations.is_empty() && app.replies.is_empty() && app.list_filter.is_none() {
        let text = match keymap.label_for(Scope::Diff, Action::Compose) {
            Some(key) => format!("no annotations yet — press {key} to add one"),
            None => "no annotations yet".to_string(),
        };
        let hint = Paragraph::new(text).block(block);
        frame.render_widget(hint, area);
        return;
    }

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let (chrome_area, list_area) = match app.list_filter.as_ref() {
        Some(_) => {
            let [chrome, list] =
                Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
            (Some(chrome), list)
        }
        None => (None, inner),
    };

    if let (Some(chrome_area), Some(filter)) = (chrome_area, app.list_filter.as_ref()) {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                super::list_filter::chrome_text(filter),
                Style::default().fg(app.theme.search_prompt),
            ))),
            chrome_area,
        );
    }

    if let Some(filter) = app.list_filter.as_ref().filter(|f| f.is_empty()) {
        let hint = Paragraph::new(super::list_filter::empty_hint(filter));
        frame.render_widget(hint, list_area);
        return;
    }

    // The unified entry order (annotations, then replies) the filter indices
    // and every list verb translate through.
    let entries = app.list_entries();
    let entry_item = |entry: &ListEntry| -> ListItem<'static> {
        match *entry {
            ListEntry::Annotation(id) => match app.annotations.iter().find(|a| a.id == id) {
                Some(a) => ListItem::new(item_line(a, &app.theme)),
                None => ListItem::new(Line::from(String::new())),
            },
            ListEntry::Reply(id) => match app.replies.get(id) {
                Some(reply) => {
                    let thread = app.thread_overlay.find(reply.thread_id);
                    ListItem::new(reply_item_line(reply, thread, &app.theme))
                }
                None => ListItem::new(Line::from(String::new())),
            },
        }
    };
    let items: Vec<ListItem> = match app.list_filter.as_ref() {
        Some(filter) => filter
            .indices()
            .iter()
            .filter_map(|&i| entries.get(i))
            .map(entry_item)
            .collect(),
        None => entries.iter().map(entry_item).collect(),
    };
    let list = List::new(items).highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = ListState::default();
    state.select(Some(app.list_cursor));
    frame.render_stateful_widget(list, list_area, &mut state);
}
