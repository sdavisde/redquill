//! The empty-diff welcome state (spec 05 Unit 5).
//!
//! [`super::diff_view::render`] shows this instead of a blank buffer whenever
//! the active target's review has zero files — most commonly the "agent
//! already committed, working tree is clean" dead end this spec exists to
//! fix. The block is two parts: [`situation`] (one line naming *why* the
//! diff area is empty, worded per [`DiffTarget`] variant) and [`hints`] (a
//! small, fixed set of next-step actions).
//!
//! **Contract:** every hint's key comes from the shared keymap table
//! ([`Keymap::default_map`]) at render time — never a literal — so a
//! remapped key displays correctly here with no code change. [`HINT_SPECS`]
//! names the three actions; [`hints`] resolves each to its current key
//! label, silently dropping (not panicking on) an action whose binding
//! somehow vanished from the table, since a cosmetic hint going missing is
//! not worth crashing the welcome screen over — [`welcome_hints_resolve_for_every_spec`]
//! is what actually catches that drift in CI, per the repository's
//! data-driven-invariants rule.

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::git::{CommitLogEntry, DiffTarget};

use super::help::centered;
use super::keymap::{Action, Keymap, Scope};
use super::theme::Theme;

/// The one-line situation text for `target`: names why the diff area is
/// empty. `active_commit` supplies the opened commit's short SHA for the
/// `Commit` case (the header metadata a History-tab open already carries);
/// falling back to the raw `Commit` payload (e.g. `HEAD`) when it isn't set
/// keeps this total rather than panicking, though in practice a commit is
/// only ever opened from a history row that already has this metadata.
pub(super) fn situation(target: &DiffTarget, active_commit: Option<&CommitLogEntry>) -> String {
    match target {
        DiffTarget::WorkingTree => "No uncommitted changes".to_string(),
        DiffTarget::Staged => "Nothing staged".to_string(),
        DiffTarget::Range(range) => format!("Empty diff for {range}"),
        DiffTarget::Commit(rev) => {
            let short = active_commit
                .map(|commit| commit.short_sha.as_str())
                .unwrap_or(rev.as_str());
            format!("Empty commit diff for {short}")
        }
        // The read-only file view always populates exactly one file on a
        // successful open (see `ui::file_view::App::open_file_view`), so
        // this welcome copy is never actually shown in practice — kept as a
        // real, sensible fallback rather than `unreachable!()`.
        DiffTarget::File(path) => format!("{path} is empty"),
    }
}

/// One resolved action hint: a key label sourced from the keymap table plus
/// its short description. Built by [`hints`].
pub(super) struct Hint {
    pub(super) key: String,
    pub(super) label: &'static str,
}

/// The welcome screen's three next-step hints (spec Unit 5's minimum: open
/// the git panel, switch to the History tab, open help), each named by the
/// `(scope, action)` its key is looked up under — the single source of truth
/// [`hints`] and the drift test both read, so the two can never name
/// different actions.
const HINT_SPECS: [(Scope, Action, &str); 3] = [
    (Scope::Diff, Action::FocusGitPanel, "open the git panel"),
    (
        Scope::Panel,
        Action::TogglePanelTab,
        "switch to the History tab to review recent commits",
    ),
    (Scope::Diff, Action::ToggleHelp, "open help"),
];

/// The key label bound to `action` in `scope`, or `None` if no binding
/// exists there (a table edit dropped or renamed the action).
fn key_for(km: &Keymap, scope: Scope, action: Action) -> Option<String> {
    km.bindings()
        .iter()
        .find(|b| b.scope == scope && b.action == action)
        .map(|b| b.key_label())
}

/// Resolves [`HINT_SPECS`] against `km`, in order. An action whose binding is
/// missing is skipped rather than shown with a blank key — see the module
/// doc's degradation note.
pub(super) fn hints(km: &Keymap) -> Vec<Hint> {
    HINT_SPECS
        .iter()
        .filter_map(|&(scope, action, label)| {
            key_for(km, scope, action).map(|key| Hint { key, label })
        })
        .collect()
}

/// Renders the welcome block: `block` (already titled/border-styled by the
/// caller) fills `area`, and the situation line + hints are centered inside
/// it, both horizontally and vertically.
pub(super) fn render(
    frame: &mut Frame,
    area: Rect,
    block: Block<'_>,
    target: &DiffTarget,
    active_commit: Option<&CommitLogEntry>,
    km: &Keymap,
    theme: &Theme,
) {
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line<'static>> = vec![
        Line::from(Span::styled(
            situation(target, active_commit),
            Style::default()
                .fg(theme.help_section_header)
                .add_modifier(Modifier::BOLD),
        )),
        Line::default(),
    ];
    for hint in hints(km) {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", hint.key),
                Style::default()
                    .fg(theme.help_key)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(hint.label),
        ]));
    }

    let width = lines
        .iter()
        .map(|l| l.width() as u16)
        .max()
        .unwrap_or(0)
        .min(inner.width);
    let height = (lines.len() as u16).min(inner.height);
    let content_area = centered(inner, width, height);
    let paragraph = Paragraph::new(lines).alignment(Alignment::Center);
    frame.render_widget(paragraph, content_area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::CommitLogEntry;

    fn commit(short_sha: &str) -> CommitLogEntry {
        CommitLogEntry {
            sha: format!("{short_sha}fullsha"),
            short_sha: short_sha.to_string(),
            subject: "a commit".to_string(),
            author_name: "author".to_string(),
            timestamp: 0,
        }
    }

    // -- situation() per target ------------------------------------------

    #[test]
    fn working_tree_situation_names_no_uncommitted_changes() {
        assert_eq!(
            situation(&DiffTarget::WorkingTree, None),
            "No uncommitted changes"
        );
    }

    #[test]
    fn staged_situation_names_nothing_staged() {
        assert_eq!(situation(&DiffTarget::Staged, None), "Nothing staged");
    }

    #[test]
    fn range_situation_names_the_range_as_typed() {
        assert_eq!(
            situation(&DiffTarget::Range("main..HEAD".to_string()), None),
            "Empty diff for main..HEAD"
        );
    }

    #[test]
    fn commit_situation_uses_the_active_commits_short_sha() {
        let entry = commit("abc1234");
        assert_eq!(
            situation(
                &DiffTarget::Commit("abc1234fullsha".to_string()),
                Some(&entry)
            ),
            "Empty commit diff for abc1234"
        );
    }

    #[test]
    fn commit_situation_falls_back_to_the_target_payload_without_header_metadata() {
        assert_eq!(
            situation(&DiffTarget::Commit("HEAD".to_string()), None),
            "Empty commit diff for HEAD"
        );
    }

    // -- hints() / key sourcing (task 5.2) ---------------------------------

    /// The drift test: every hinted action must resolve to a real key in the
    /// shared table. If an action in `HINT_SPECS` is ever renamed or its
    /// binding removed, `key_for` returns `None` here and this test fails
    /// loudly — the production path (`hints`) would otherwise silently drop
    /// the hint instead, which is correct for a live cosmetic degradation but
    /// wrong for CI to let slide.
    #[test]
    fn welcome_hints_resolve_for_every_spec() {
        let km = Keymap::default_map();
        for &(scope, action, label) in &HINT_SPECS {
            assert!(
                key_for(&km, scope, action).is_some(),
                "no {scope:?} binding for {action:?} (hint {label:?}) — \
                 the shared keymap table no longer has this action"
            );
        }
    }

    #[test]
    fn hints_returns_one_resolved_hint_per_spec_in_order() {
        let km = Keymap::default_map();
        let built = hints(&km);
        assert_eq!(built.len(), HINT_SPECS.len());
        for (hint, &(_, _, label)) in built.iter().zip(HINT_SPECS.iter()) {
            assert_eq!(hint.label, label);
            assert!(!hint.key.is_empty());
        }
    }

    #[test]
    fn hints_keys_match_the_tables_current_bindings() {
        let km = Keymap::default_map();
        let built = hints(&km);
        // FocusGitPanel is bound to the backtick in Scope::Diff today; assert
        // through the table rather than hardcoding "`" twice, but this test
        // still demonstrates a resolved key looks like a real key label, not
        // an empty string or a debug-format action name.
        let focus_panel_key = key_for(&km, Scope::Diff, Action::FocusGitPanel).unwrap();
        assert_eq!(built[0].key, focus_panel_key);
    }
}
