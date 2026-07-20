//! The review session's imported comment-thread overlay: fetching a PR's
//! existing review threads asynchronously (never blocking session entry),
//! decorating the diff's gutter with a single-cell marker on threaded lines,
//! navigating between threads, and the expandable [`Mode::ThreadView`]
//! overlay that shows one thread's full conversation on demand.
//!
//! The fetched threads live in [`App::thread_overlay`] — read-only, never
//! persisted, never serialized to stdout (see
//! [`crate::forge::ThreadOverlayStore`]). A failed fetch leaves the review
//! running and sets [`App::threads_unavailable`], which drives the one-line
//! "comments unavailable" banner notice.

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::annotate::{Side, Target};
use crate::forge::{Thread, ThreadAnchor, ThreadComment};

use super::app::{App, Mode};
use super::background::TaskId;
use super::rows::Row;
use super::theme::Theme;
use super::time_format::{now_unix, relative_time};

/// A background thread fetch awaiting completion: its [`TaskId`] and the
/// generation captured at spawn (a straggler from before a bump is dropped).
/// Same shape as [`super::review_launcher::InFlightPrCheckout`] minus the
/// finish context, since a thread fetch's result needs none.
#[derive(Debug, Clone, Copy)]
pub(super) struct InFlightThreadFetch {
    pub(super) id: TaskId,
    pub(super) generation: u64,
}

/// The expandable thread overlay's state: which thread (its root id) is
/// shown, and the vertical scroll offset within its conversation.
#[derive(Debug, Clone, Copy)]
pub(super) struct ThreadViewState {
    pub(super) root_id: u64,
    pub(super) scroll: usize,
}

impl App {
    /// Kicks off a background fetch of PR `number`'s comment threads,
    /// single-flight with a generation guard (mirrors [`App::open_finder`]).
    /// Bumping the generation and clearing any in-flight fetch means a
    /// straggler from a previous session/refresh is dropped on arrival. A
    /// no-op for a backend that can't cross a thread boundary (test fakes,
    /// git-less contexts) — the overlay simply stays empty.
    pub(super) fn spawn_thread_fetch(&mut self, number: u64) {
        self.thread_fetch_generation = self.thread_fetch_generation.wrapping_add(1);
        self.thread_fetch_in_flight = None;
        let Some(ops) = self.stage_ops() else {
            return;
        };
        if let Some(fetcher) = ops.async_thread_fetcher() {
            let generation = self.thread_fetch_generation;
            let id = self.thread_fetch_tasks.spawn(move || fetcher(number));
            self.thread_fetch_in_flight = Some(InFlightThreadFetch { id, generation });
        }
    }

    /// Drains a completed background thread fetch (once per event-loop tick,
    /// alongside the other pollers). Drops a stale result — spawned before
    /// `thread_fetch_generation` was last bumped — otherwise applies it.
    pub(super) fn poll_thread_fetch(&mut self) {
        for (id, result) in self.thread_fetch_tasks.poll() {
            let Some(in_flight) = self.thread_fetch_in_flight else {
                continue;
            };
            if in_flight.id != id {
                continue;
            }
            self.thread_fetch_in_flight = None;
            if in_flight.generation != self.thread_fetch_generation {
                continue;
            }
            let outcome = match result {
                Ok(inner) => inner,
                Err(_panic) => Err("thread fetch task panicked".to_string()),
            };
            self.apply_thread_fetch(outcome);
        }
    }

    /// Applies a thread-fetch outcome: a success swaps the overlay in and
    /// clears the unavailable flag; a failure sets the flag (driving the
    /// banner notice) and leaves any prior overlay untouched, so a failed
    /// refresh keeps the last-seen threads rather than blanking them. Rebuilds
    /// rows either way so the new gutter markers show immediately.
    pub(super) fn apply_thread_fetch(&mut self, result: Result<Vec<Thread>, String>) {
        match result {
            Ok(threads) => {
                self.threads_unavailable = false;
                self.thread_overlay.replace(threads);
            }
            Err(_) => {
                self.threads_unavailable = true;
                // The review continues without the threads; a one-line notice
                // (same async-completion status channel the "PR updated" line
                // uses) tells the reviewer why no markers appeared.
                self.set_status_message("comments unavailable \u{2014} reviewing without them");
            }
        }
        self.rebuild_rows();
    }

    /// Overlays the imported-thread gutter markers onto the freshly built
    /// rows: flags every [`Row::Line`] whose exact `(path, side, line)` an
    /// imported thread anchors on. A no-op with an empty overlay, so non-PR
    /// sessions pay nothing. Called once per rebuild from
    /// [`App::rebuild_rows`], after the overlay-free row build.
    pub(super) fn decorate_thread_markers(&mut self) {
        if self.thread_overlay.is_empty() {
            return;
        }
        // Owned snapshots so the overlay/view borrows are released before the
        // rows are mutated below.
        let mut positions: std::collections::HashSet<(String, Side, u32)> =
            std::collections::HashSet::new();
        for thread in self.thread_overlay.iter() {
            if let ThreadAnchor::Position { path, side, line } = &thread.anchor {
                positions.insert((path.clone(), *side, *line));
            }
        }
        if positions.is_empty() {
            return;
        }
        let paths: Vec<String> = self.view.files.iter().map(|f| f.path.clone()).collect();
        let file_of_row = self.view.file_of_row.clone();
        for (i, row) in self.view.rows.iter_mut().enumerate() {
            let Row::Line(line) = row else {
                continue;
            };
            let Some(path) = file_of_row.get(i).and_then(|&fi| paths.get(fi)) else {
                continue;
            };
            let new_hit = line
                .new_line
                .is_some_and(|n| positions.contains(&(path.clone(), Side::New, n)));
            let old_hit = line
                .old_line
                .is_some_and(|o| positions.contains(&(path.clone(), Side::Old, o)));
            line.thread = new_hit || old_hit;
        }
    }

    /// The ids of annotations that must NOT be drawn as local annotation rows
    /// because they have already been published to the forge *and* the forge's
    /// own copy is present in the fetched thread overlay at the same anchor —
    /// so the reviewer sees one authoritative comment, not a duplicate. An
    /// empty set (the common case: no overlay, or nothing published yet), so
    /// [`App::rebuild_rows`] pays nothing outside a PR review with published
    /// annotations. The annotations still exist in the store — editable,
    /// listed, and serialized to stdout — only their in-diff rows are hidden.
    pub(super) fn suppressed_published_annotation_ids(&self) -> std::collections::HashSet<usize> {
        let mut suppressed = std::collections::HashSet::new();
        if self.thread_overlay.is_empty() {
            return suppressed;
        }
        // The anchors the overlay's threads cover, split into line positions
        // and file-level fallbacks.
        let mut positions: std::collections::HashSet<(&str, Side, u32)> =
            std::collections::HashSet::new();
        let mut file_paths: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for thread in self.thread_overlay.iter() {
            match &thread.anchor {
                ThreadAnchor::Position { path, side, line } => {
                    positions.insert((path.as_str(), *side, *line));
                }
                ThreadAnchor::File { path } => {
                    file_paths.insert(path.as_str());
                }
            }
        }
        for annotation in self.annotations.iter() {
            if !annotation.published {
                continue;
            }
            let covered = match &annotation.target {
                Target::Line { path, line, side } => {
                    positions.contains(&(path.as_str(), *side, *line))
                }
                Target::Range {
                    path, start, side, ..
                } => positions.contains(&(path.as_str(), *side, *start)),
                Target::Hunk { path, start, .. } => {
                    positions.contains(&(path.as_str(), Side::New, *start))
                }
                Target::File { path } => file_paths.contains(path.as_str()),
                // Worktree-anchored annotations are local-only — they have no
                // forge line-comment equivalent to be deduped against.
                Target::WorktreeLine { .. } | Target::WorktreeRange { .. } => false,
            };
            if covered {
                suppressed.insert(annotation.id);
            }
        }
        suppressed
    }

    /// The buffer row a thread's anchor resolves to: its exact line for a
    /// resolvable [`ThreadAnchor::Position`] (falling back to the file's
    /// header row when the line isn't in the current buffer — e.g. a
    /// collapsed section), or the file header for a file-level
    /// ([`ThreadAnchor::File`], outdated) thread.
    pub(super) fn thread_anchor_row(&self, thread: &Thread) -> Option<usize> {
        match &thread.anchor {
            ThreadAnchor::Position { path, side, line } => {
                let target = Target::line(path.clone(), *line, *side);
                self.view
                    .anchor_row_in_buffer(&target)
                    .or_else(|| self.header_row_for_path(path))
            }
            ThreadAnchor::File { path } => self.header_row_for_path(path),
        }
    }

    /// The buffer row of `path`'s section header, or `None` when `path` isn't
    /// in the current diff.
    fn header_row_for_path(&self, path: &str) -> Option<usize> {
        let idx = self.view.files.iter().position(|f| f.path == path)?;
        self.view.header_row_of_file.get(idx).copied()
    }

    /// The root id of the thread to act on from the cursor: the thread whose
    /// [`ThreadAnchor::Position`] exactly matches the cursor line, else the
    /// first thread anchored anywhere in the cursor's file (covering a header/
    /// hunk row and file-level threads). `None` when the cursor's file has no
    /// thread.
    fn thread_at_cursor(&self) -> Option<u64> {
        let cursor = self.view.cursor;
        let file_idx = *self.view.file_of_row.get(cursor)?;
        let path = self.view.files.get(file_idx)?.path.clone();
        if let Some(Row::Line(line)) = self.view.rows.get(cursor) {
            for thread in self.thread_overlay.for_path(&path) {
                if let ThreadAnchor::Position {
                    side, line: tline, ..
                } = &thread.anchor
                {
                    let hit = (*side == Side::New && line.new_line == Some(*tline))
                        || (*side == Side::Old && line.old_line == Some(*tline));
                    if hit {
                        return Some(thread.id);
                    }
                }
            }
        }
        self.thread_overlay.for_path(&path).map(|t| t.id).next()
    }

    /// The message shown when the reviewer reaches for threads but finds
    /// none: the persistent "comments unavailable" notice when the last fetch
    /// failed (so a reviewer is never silently blind to feedback that exists
    /// on the PR — the FR-13 intent, re-surfaced on demand rather than only
    /// once at fetch time), or the plain `absent` hint when threads genuinely
    /// aren't there.
    fn no_threads_message(&self, absent: &'static str) -> &'static str {
        if self.threads_unavailable {
            "comments unavailable \u{2014} reviewing without them"
        } else {
            absent
        }
    }

    /// Opens the thread overlay (`T`) on the thread at the cursor, or leaves a
    /// status hint when the cursor isn't on a threaded line/file (naming the
    /// fetch failure when that's why nothing is here).
    pub(super) fn open_thread_view(&mut self) {
        match self.thread_at_cursor() {
            Some(root_id) => {
                self.thread_view = Some(ThreadViewState { root_id, scroll: 0 });
                self.mode = Mode::ThreadView;
            }
            None => {
                let msg = self.no_threads_message("no comment thread here");
                self.set_status_message(msg);
            }
        }
    }

    /// Closes the thread overlay, returning to [`Mode::Normal`].
    pub(super) fn close_thread_view(&mut self) {
        self.thread_view = None;
        self.mode = Mode::Normal;
    }

    /// `r` in the thread overlay: closes the overlay and opens Compose in
    /// reply mode, targeting the open thread's root. A no-op when no thread
    /// is open (the key is only reachable from [`Mode::ThreadView`]).
    pub(super) fn open_reply_compose(&mut self) {
        let Some(tv) = self.thread_view.take() else {
            return;
        };
        self.compose = Some(super::compose::ComposeState::reply(tv.root_id));
        self.mode = Mode::Compose;
    }

    /// Scrolls the open thread overlay down one line (clamped by the render's
    /// own overscroll handling).
    pub(super) fn thread_view_scroll_down(&mut self) {
        if let Some(tv) = self.thread_view.as_mut() {
            tv.scroll = tv.scroll.saturating_add(1);
        }
    }

    /// Scrolls the open thread overlay up one line.
    pub(super) fn thread_view_scroll_up(&mut self) {
        if let Some(tv) = self.thread_view.as_mut() {
            tv.scroll = tv.scroll.saturating_sub(1);
        }
    }

    /// Moves the diff cursor to the next imported thread's anchor row
    /// (wrapping to the first past the last), or leaves a status hint when
    /// none exist.
    pub(super) fn next_thread(&mut self) {
        self.jump_thread(true);
    }

    /// Moves the diff cursor to the previous imported thread's anchor row
    /// (wrapping to the last before the first).
    pub(super) fn prev_thread(&mut self) {
        self.jump_thread(false);
    }

    fn jump_thread(&mut self, forward: bool) {
        if self.thread_overlay.is_empty() {
            let msg = self.no_threads_message("no comment threads");
            self.set_status_message(msg);
            return;
        }
        let mut rows: Vec<usize> = self
            .thread_overlay
            .iter()
            .filter_map(|t| self.thread_anchor_row(t))
            .collect();
        rows.sort_unstable();
        rows.dedup();
        if rows.is_empty() {
            self.set_status_message("no comment threads in view");
            return;
        }
        let cursor = self.view.cursor;
        let target = if forward {
            rows.iter()
                .find(|&&r| r > cursor)
                .copied()
                .or_else(|| rows.first().copied())
        } else {
            rows.iter()
                .rev()
                .find(|&&r| r < cursor)
                .copied()
                .or_else(|| rows.last().copied())
        };
        if let Some(row) = target {
            self.view.cursor = row;
            self.view.ensure_visible();
        }
    }
}

/// Parses an RFC 3339 timestamp (`YYYY-MM-DDThh:mm:ss…`, the shape GitHub's
/// `created_at` uses) into a Unix timestamp in seconds. Timezone-naive: a
/// trailing `Z`/offset is ignored, which is accurate for the `Z`-suffixed
/// UTC values GitHub returns and harmless (a few hours' skew at worst) for
/// the cosmetic relative-time label otherwise. `None` on any shape it can't
/// read, so the caller falls back to the raw string.
fn parse_rfc3339_to_unix(s: &str) -> Option<i64> {
    let (date, time) = s.split_once('T')?;
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;
    // Trim the timezone / fractional-second suffix off the time.
    let time = &time[..time.find(['Z', '+', '.']).unwrap_or(time.len())];
    let mut time_parts = time.split(':');
    let hour: i64 = time_parts.next()?.parse().ok()?;
    let minute: i64 = time_parts.next()?.parse().ok()?;
    let second: i64 = time_parts.next().unwrap_or("0").parse().ok()?;
    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// Days since the Unix epoch for a proleptic-Gregorian date (Howard
/// Hinnant's `days_from_civil`, public domain — the inverse of
/// [`super::time_format`]'s `civil_from_days`).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let d = d as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Centers a `width_pct`% x `height_pct`% rect inside `area` (same helper
/// shape as [`super::peek_overlay`]'s).
fn centered(area: Rect, width_pct: u16, height_pct: u16) -> Rect {
    let [area] = Layout::horizontal([Constraint::Percentage(width_pct)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::vertical([Constraint::Percentage(height_pct)])
        .flex(Flex::Center)
        .areas(area);
    area
}

/// The relative-time label for a comment (`3d ago`), falling back to the raw
/// provider timestamp when it can't be parsed.
fn comment_time(created_at: &str) -> String {
    parse_rfc3339_to_unix(created_at)
        .map(|ts| relative_time(now_unix(), ts))
        .unwrap_or_else(|| created_at.to_string())
}

/// Appends one comment's lines: a bold `author · when` header, then its body
/// lines. `reply` nests the block under its root with an arrow prefix and
/// indentation.
fn push_comment(
    lines: &mut Vec<Line<'static>>,
    comment: &ThreadComment,
    reply: bool,
    theme: &Theme,
) {
    let (header_prefix, body_prefix) = if reply {
        ("  \u{21b3} ", "    ")
    } else {
        ("", "")
    };
    lines.push(Line::from(Span::styled(
        format!(
            "{header_prefix}{} \u{00b7} {}",
            comment.author,
            comment_time(&comment.created_at)
        ),
        Style::default()
            .fg(theme.hunk_header)
            .add_modifier(Modifier::BOLD),
    )));
    for body_line in comment.body.lines() {
        lines.push(Line::from(Span::styled(
            format!("{body_prefix}{body_line}"),
            Style::default().fg(theme.annotation_text),
        )));
    }
    if comment.body.is_empty() {
        lines.push(Line::from(String::new()));
    }
    lines.push(Line::from(String::new()));
}

/// Renders the expandable thread overlay, centered over `area`. A no-op when
/// no thread is open or its root id is no longer in the overlay (e.g. a
/// refresh dropped it). The full conversation renders in order — root first,
/// replies nested — with the resolved/outdated state in the title; scrolling
/// keeps the body on demand rather than reflowing the diff underneath.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(tv) = &app.thread_view else {
        return;
    };
    let Some(thread) = app.thread_overlay.find(tv.root_id) else {
        return;
    };
    let popup = centered(area, 70, 60);
    frame.render_widget(Clear, popup);

    let mut lines: Vec<Line> = Vec::new();
    let anchor = match &thread.anchor {
        ThreadAnchor::Position { path, line, .. } => format!("{path}:{line}"),
        ThreadAnchor::File { path } => format!("{path} (file-level)"),
    };
    lines.push(Line::from(Span::styled(
        anchor,
        Style::default()
            .fg(app.theme.gutter)
            .add_modifier(Modifier::DIM),
    )));
    lines.push(Line::from(String::new()));
    push_comment(&mut lines, &thread.root, false, &app.theme);
    for reply in &thread.replies {
        push_comment(&mut lines, reply, true, &app.theme);
    }

    let state = if thread.resolved {
        " [resolved]"
    } else if thread.outdated {
        " [outdated]"
    } else {
        ""
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!("thread{state}"))
        .title_bottom(Line::from(" j/k scroll  r reply  Esc/q close "));
    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((tv.scroll as u16, 0));
    frame.render_widget(paragraph, popup);
}

#[cfg(test)]
#[path = "forge_threads_tests.rs"]
mod tests;
