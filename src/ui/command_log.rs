//! The command log: a bounded, in-memory record of every git command
//! redquill ran on the user's behalf (currently the three remote operations),
//! rendered lazygit-style in a toggleable bottom pane so the tool's git
//! activity is fully transparent.
//!
//! The log stores at most [`COMMAND_LOG_CAPACITY`] entries; once full, the
//! oldest is evicted as a new one arrives. Ordering is newest-last (append
//! order), matching how the pane renders them top-to-bottom. Nothing is
//! persisted — the log lives only for the session.

use std::collections::VecDeque;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::app::App;
use super::theme::Theme;

/// The maximum number of entries the command log retains. Older entries are
/// evicted oldest-first once this many are present.
pub const COMMAND_LOG_CAPACITY: usize = 50;

/// One finished git command: the command line that ran, whether it succeeded,
/// its exit code (if it exited via one), and its captured stdout/stderr.
///
/// Only git's own output is stored here — never any environment contents —
/// per the spec's security considerations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandLogEntry {
    /// The command line as run, e.g. `git fetch`.
    pub command_line: String,
    /// Whether the process exited successfully.
    pub success: bool,
    /// The exit code, if the process exited via one (`None` if signalled or
    /// never spawned).
    pub code: Option<i32>,
    /// Captured standard output (lossy UTF-8).
    pub stdout: String,
    /// Captured standard error (lossy UTF-8); also carries a spawn-failure
    /// message when the process could not be started.
    pub stderr: String,
}

impl CommandLogEntry {
    /// A short exit-status label for the log pane: `exit 0`, `exit 128`, or
    /// `failed` when the process never produced an exit code.
    pub fn exit_status(&self) -> String {
        match self.code {
            Some(code) => format!("exit {code}"),
            None => "failed".to_string(),
        }
    }
}

/// A bounded FIFO of [`CommandLogEntry`]s, newest last, capped at
/// [`COMMAND_LOG_CAPACITY`].
#[derive(Debug, Default)]
pub struct CommandLog {
    entries: VecDeque<CommandLogEntry>,
}

impl CommandLog {
    /// Creates an empty log.
    pub fn new() -> CommandLog {
        CommandLog {
            entries: VecDeque::new(),
        }
    }

    /// Appends `entry` as the newest. If the log is already at capacity, the
    /// oldest entry is evicted first, so the length never exceeds
    /// [`COMMAND_LOG_CAPACITY`].
    pub fn push(&mut self, entry: CommandLogEntry) {
        if self.entries.len() >= COMMAND_LOG_CAPACITY {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    /// The entries in newest-last order (append order).
    pub fn entries(&self) -> impl Iterator<Item = &CommandLogEntry> {
        self.entries.iter()
    }

    /// The number of retained entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the log holds no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Builds the rendered lines for `log`, newest last: a header line per entry
/// (command line + exit status, colored by success), then any stderr and
/// stdout lines beneath it.
fn log_lines(log: &CommandLog, theme: &Theme) -> Vec<Line<'static>> {
    if log.is_empty() {
        return vec![Line::from(Span::styled(
            "no git commands run yet".to_string(),
            Style::default().fg(theme.footer_text),
        ))];
    }
    let mut lines: Vec<Line> = Vec::new();
    for entry in log.entries() {
        let status_color = if entry.success {
            theme.kind_added
        } else {
            theme.kind_deleted
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("$ {}", entry.command_line),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(entry.exit_status(), Style::default().fg(status_color)),
        ]));
        for out_line in entry.stderr.lines() {
            lines.push(Line::from(Span::styled(
                format!("  {out_line}"),
                Style::default().fg(theme.removed_fg),
            )));
        }
        for out_line in entry.stdout.lines() {
            lines.push(Line::from(Span::styled(
                format!("  {out_line}"),
                Style::default().fg(theme.context_fg),
            )));
        }
    }
    lines
}

/// Renders the command log into `area` (the bottom-panel slot), newest last.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!("command log ({})", app.command_log.len()));
    let lines = log_lines(&app.command_log, &app.theme);
    // Show the tail (newest) when the log is taller than the pane.
    let visible = area.height.saturating_sub(2) as usize;
    let scroll = lines.len().saturating_sub(visible);
    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((scroll as u16, 0));
    frame.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(cmd: &str, code: i32) -> CommandLogEntry {
        CommandLogEntry {
            command_line: cmd.to_string(),
            success: code == 0,
            code: Some(code),
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    #[test]
    fn push_appends_newest_last() {
        let mut log = CommandLog::new();
        log.push(entry("git fetch", 0));
        log.push(entry("git push", 1));
        let cmds: Vec<&str> = log.entries().map(|e| e.command_line.as_str()).collect();
        assert_eq!(cmds, vec!["git fetch", "git push"]);
    }

    #[test]
    fn len_and_is_empty_track_contents() {
        let mut log = CommandLog::new();
        assert!(log.is_empty());
        assert_eq!(log.len(), 0);
        log.push(entry("git fetch", 0));
        assert!(!log.is_empty());
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn evicts_oldest_at_capacity() {
        let mut log = CommandLog::new();
        // Push one more than capacity, tagging each by index so we can see
        // which survived.
        for i in 0..=COMMAND_LOG_CAPACITY {
            log.push(entry(&format!("git fetch {i}"), 0));
        }
        assert_eq!(log.len(), COMMAND_LOG_CAPACITY);
        let first = log.entries().next().unwrap();
        // Entry 0 was evicted; the oldest survivor is entry 1.
        assert_eq!(first.command_line, "git fetch 1");
        let last = log.entries().last().unwrap();
        assert_eq!(
            last.command_line,
            format!("git fetch {COMMAND_LOG_CAPACITY}")
        );
    }

    #[test]
    fn eviction_holds_the_cap_across_many_pushes() {
        let mut log = CommandLog::new();
        for i in 0..(COMMAND_LOG_CAPACITY * 3) {
            log.push(entry(&format!("git fetch {i}"), 0));
        }
        assert_eq!(log.len(), COMMAND_LOG_CAPACITY);
    }

    // -- Rendering ----------------------------------------------------------

    fn render_log(app: &App) -> String {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 60, 12);
        terminal.draw(|frame| render(frame, area, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn render_shows_a_failed_entry_with_its_stderr_and_exit_code() {
        use crate::diff::FileDiff;
        use crate::git::RawFilePatch;
        let raw = "diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n-old\n+new\n";
        let mut app = App::new(vec![
            FileDiff::from_patch(&RawFilePatch {
                path: "a.rs".to_string(),
                old_path: None,
                raw: raw.to_string(),
                is_binary: false,
            })
            .unwrap(),
        ]);
        app.command_log.push(CommandLogEntry {
            command_line: "git push".to_string(),
            success: false,
            code: Some(1),
            stdout: String::new(),
            stderr: "! [rejected] main -> main (non-fast-forward)".to_string(),
        });
        let content = render_log(&app);
        assert!(content.contains("command log (1)"));
        assert!(content.contains("git push"));
        assert!(content.contains("exit 1"));
        assert!(content.contains("non-fast-forward"));
    }

    #[test]
    fn render_empty_log_shows_a_hint() {
        use crate::diff::FileDiff;
        use crate::git::RawFilePatch;
        let raw = "diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n-old\n+new\n";
        let app = App::new(vec![
            FileDiff::from_patch(&RawFilePatch {
                path: "a.rs".to_string(),
                old_path: None,
                raw: raw.to_string(),
                is_binary: false,
            })
            .unwrap(),
        ]);
        let content = render_log(&app);
        assert!(content.contains("no git commands run yet"));
    }

    #[test]
    fn exit_status_label_reflects_the_code() {
        assert_eq!(entry("git fetch", 0).exit_status(), "exit 0");
        assert_eq!(entry("git push", 1).exit_status(), "exit 1");
        let spawn_failure = CommandLogEntry {
            command_line: "git pull".to_string(),
            success: false,
            code: None,
            stdout: String::new(),
            stderr: "boom".to_string(),
        };
        assert_eq!(spawn_failure.exit_status(), "failed");
    }
}
