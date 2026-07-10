//! The help overlay: a centered box listing every binding, grouped, plus
//! the Compose-mode, List-mode, and Staging-panel key hints that aren't in
//! the [`Keymap`] table (those modes handle keys modally, bypassing the
//! table — see [`super::handle_compose_key`]/[`super::handle_list_key`]/
//! [`super::handle_staging_key`]).

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::keymap::{Action, Binding, Keymap};
use super::theme::Theme;

/// Static key hints for a mode that isn't driven by the [`Keymap`] table.
const COMPOSE_HINTS: &[(&str, &str)] = &[
    ("Enter", "Submit"),
    ("Esc", "Cancel"),
    ("Ctrl-j", "Insert newline"),
    ("Ctrl-t", "Cycle classification"),
    ("Backspace", "Delete character"),
    ("Left/Right/Up/Down", "Move within text"),
];

const LIST_HINTS: &[(&str, &str)] = &[
    ("j / k", "Move focus"),
    ("Enter", "Jump to annotation"),
    ("e", "Edit"),
    ("d", "Delete"),
    ("a / Esc", "Close panel"),
];

const STAGING_HINTS: &[(&str, &str)] = &[
    ("j / k", "Move focus"),
    ("Space / Enter", "Unstage file"),
    ("s / Esc", "Close panel"),
];

const SEARCH_HINTS: &[(&str, &str)] = &[
    ("Enter", "Confirm search"),
    ("Esc", "Cancel (clears pattern if buffer empty)"),
    ("Backspace", "Delete character"),
];

const PEEK_HINTS: &[(&str, &str)] = &[
    ("j / k", "Move selection (or scroll hover text)"),
    ("Enter", "Jump to location (definition/references)"),
    ("Esc / q", "Close"),
];

/// Which help-overlay group an [`Action`] belongs to.
fn group_of(action: Action) -> &'static str {
    use Action::*;
    match action {
        CursorDown | CursorUp | CursorLeft | CursorRight | WordForward | WordBackward
        | HalfPageDown | HalfPageUp | NextHunk | PrevHunk | NextFile | PrevFile => "Navigation",
        EnterVisual | Compose => "Annotate",
        ToggleStage | ToggleStagingPanel => "Stage",
        Search | SearchNext | SearchPrev => "Search",
        ToggleList | ToggleHelp | ToggleView => "Panels",
        GotoDefinition | GotoReferences | Hover => "Code intelligence",
        Quit | QuitDiscard => "Quit",
    }
}

/// Centers a `width` x `height` rect inside `area`.
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [area] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    area
}

fn section_header(label: &str, theme: &Theme) -> Line<'static> {
    Line::from(Span::styled(
        label.to_string(),
        Style::default()
            .fg(theme.help_section_header)
            .add_modifier(Modifier::BOLD),
    ))
}

fn key_line(key: &str, description: &str, key_width: usize, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{key:>key_width$}"),
            Style::default()
                .fg(theme.help_key)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::raw(description.to_string()),
    ])
}

/// Renders the help overlay, centered over `area`. Bindings from the
/// [`Keymap`] table are grouped Navigation / Annotate / Panels / Quit, with
/// Compose-mode and List-mode hints appended below (those modes bypass the
/// table entirely, so they aren't in it).
pub fn render(frame: &mut Frame, area: Rect, keymap: &Keymap, theme: &Theme) {
    let bindings = keymap.bindings();
    let key_width = bindings
        .iter()
        .map(|b| b.key_label().len())
        .chain(COMPOSE_HINTS.iter().map(|(k, _)| k.len()))
        .chain(LIST_HINTS.iter().map(|(k, _)| k.len()))
        .chain(STAGING_HINTS.iter().map(|(k, _)| k.len()))
        .chain(SEARCH_HINTS.iter().map(|(k, _)| k.len()))
        .chain(PEEK_HINTS.iter().map(|(k, _)| k.len()))
        .max()
        .unwrap_or(0);

    let mut lines: Vec<Line> = Vec::new();
    for group in [
        "Navigation",
        "Annotate",
        "Stage",
        "Search",
        "Panels",
        "Code intelligence",
        "Quit",
    ] {
        let group_bindings: Vec<&Binding> = bindings
            .iter()
            .filter(|b| group_of(b.action) == group)
            .collect();
        if group_bindings.is_empty() {
            continue;
        }
        lines.push(section_header(group, theme));
        for b in group_bindings {
            lines.push(key_line(&b.key_label(), b.description, key_width, theme));
        }
        lines.push(Line::from(""));
    }

    lines.push(section_header("Compose mode", theme));
    for (key, desc) in COMPOSE_HINTS {
        lines.push(key_line(key, desc, key_width, theme));
    }
    lines.push(Line::from(""));

    lines.push(section_header("List mode", theme));
    for (key, desc) in LIST_HINTS {
        lines.push(key_line(key, desc, key_width, theme));
    }
    lines.push(Line::from(""));

    lines.push(section_header("Staging panel", theme));
    for (key, desc) in STAGING_HINTS {
        lines.push(key_line(key, desc, key_width, theme));
    }
    lines.push(Line::from(""));

    lines.push(section_header("Search input", theme));
    for (key, desc) in SEARCH_HINTS {
        lines.push(key_line(key, desc, key_width, theme));
    }
    lines.push(Line::from(""));

    lines.push(section_header("Peek mode", theme));
    for (key, desc) in PEEK_HINTS {
        lines.push(key_line(key, desc, key_width, theme));
    }

    let height = (lines.len() as u16 + 2).min(area.height);
    let width = (lines.iter().map(|l| l.width()).max().unwrap_or(0) as u16 + 4).min(area.width);
    let popup = centered(area, width, height);

    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .title("help")
        .title_alignment(Alignment::Center);
    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, popup);
}
