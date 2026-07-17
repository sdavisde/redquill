//! The context-sensitive footer strip: a 1-2 row band at the bottom of the
//! screen (below the diff/panel/bottom-panel area) that shows the handful of
//! keybinds most relevant to whatever the user is looking at right now.
//!
//! **Contract:**
//! - Hints derive from the shared keymap/modal tables ([`super::keymap::Keymap`],
//!   [`super::modal_keys`]) — never a hardcoded per-mode key/label pair.
//!   Curation (which rows are promoted, and in what order) lives as
//!   [`super::keymap::FooterHint`] tags on those tables' rows (see
//!   [`keymap_hints`]/[`modal_hints`]), plus a small number of explicitly
//!   synthetic hints ([`visual_hints`], [`pending_hints`]'s two-key fallback
//!   labels, the `Esc cancel` in Visual) where the table has no single row to
//!   promote — each of those is documented at its definition and covered by a
//!   drift test.
//! - A strip is built in a fixed **display order** (curation order, `? help`
//!   always last); [`FooterHint::rank`] only decides **drop order** under
//!   width pressure — see [`sort_for_display`] and [`wrap_hints`].
//! - A strip is capped at **2 rows**; hints are dropped lowest-priority-first
//!   (highest rank) until what remains fits, and `? help` (rank 0) is the
//!   last thing ever dropped.
//! - [`footer_height`] is the single place strip height is computed from
//!   width + app state; both `draw()`'s `split_footer` call and the event
//!   loop's viewport-measurement mirror call it, so they can never disagree
//!   (see `super::mod` docs on that mirror).

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crossterm::event::KeyEvent;

use super::app::{App, Mode};
use super::keymap::{Action, FooterHint, Keymap, Scope};
use super::modal_keys::{ModalBinding, ModalKeymaps};
use super::project_search::SearchFocus;
use super::theme::Theme;

/// One hint in the footer strip: a key label plus a short action label,
/// ranked so [`wrap_hints`] knows which to drop first under width pressure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FooterEntry {
    pub(super) rank: u8,
    pub(super) key: String,
    pub(super) label: &'static str,
}

/// Sorts `entries` into display order: ascending rank, except rank `0`
/// (reserved for `? help`) sorts *last* — the escape hatch reads at the end
/// of the strip despite being the highest-priority (last-dropped) hint. A
/// stable sort, so entries built in curation order with equal adjusted rank
/// keep that order.
fn sort_for_display(mut entries: Vec<FooterEntry>) -> Vec<FooterEntry> {
    entries.sort_by_key(|e| if e.rank == 0 { u8::MAX } else { e.rank });
    entries
}

/// Groups `km`'s bindings in `scope` that carry a [`FooterHint`], merging
/// rows that share an identical hint (same rank *and* label) into one entry
/// whose key text joins both rows' key labels with `/` (the `j`/`k` "move"
/// pairing). `staging_allowed`/`code_intel_allowed`/`review_session` hide the
/// same capability-gated rows the help overlay hides (see
/// [`super::help::binding_hidden`]) — `staging_allowed` is `false` on a
/// read-only diff range, `code_intel_allowed` is `false` whenever the
/// target's new side isn't the live working tree, `review_session` is
/// `false` outside a review session (spec 08 Unit 3).
fn keymap_hints(
    km: &Keymap,
    scope: Scope,
    staging_allowed: bool,
    code_intel_allowed: bool,
    review_session: bool,
) -> Vec<FooterEntry> {
    let mut grouped: Vec<(FooterHint, Vec<String>)> = Vec::new();
    for b in km.bindings().iter().filter(|b| b.scope == scope) {
        if super::help::binding_hidden(
            b.action,
            staging_allowed,
            code_intel_allowed,
            review_session,
        ) {
            continue;
        }
        let Some(hint) = b.footer else { continue };
        match grouped.iter_mut().find(|(h, _)| *h == hint) {
            Some((_, keys)) => keys.push(b.key_label()),
            None => grouped.push((hint, vec![b.key_label()])),
        }
    }
    sort_for_display(
        grouped
            .into_iter()
            .map(|(hint, keys)| FooterEntry {
                rank: hint.rank,
                key: keys.join("/"),
                label: hint.label,
            })
            .collect(),
    )
}

/// Same grouping as [`keymap_hints`], for a modal-mode table
/// ([`super::modal_keys`]) instead of the [`Keymap`]. Takes the *effective*
/// table (`app.modal_keys.*`, spec 07 Unit 4 task 5.4 — defaults plus any
/// `[keys.<mode>]` overrides), not the compiled-in `'static` default, so the
/// footer strip reflects a remap with no additional wiring.
fn modal_hints<A: Copy + Clone>(table: &[ModalBinding<A>]) -> Vec<FooterEntry> {
    let mut grouped: Vec<(FooterHint, String)> = Vec::new();
    for b in table {
        let Some(hint) = b.footer else { continue };
        match grouped.iter_mut().find(|(h, _)| *h == hint) {
            Some((_, key)) => {
                key.push('/');
                key.push_str(&b.key_label());
            }
            None => grouped.push((hint, b.key_label())),
        }
    }
    sort_for_display(
        grouped
            .into_iter()
            .map(|(hint, key)| FooterEntry {
                rank: hint.rank,
                key,
                label: hint.label,
            })
            .collect(),
    )
}

/// The key label of the binding matching `scope`/`action` whose own
/// `key_label()` equals `want` — used to disambiguate two rows sharing an
/// `Action` (`ToggleHelp` has both a `?` and an `Esc` row) without depending
/// on table order. `None` if no such row exists.
fn find_key(km: &Keymap, scope: Scope, action: Action, want: &str) -> Option<String> {
    km.bindings()
        .iter()
        .find(|b| b.scope == scope && b.action == action && b.key_label() == want)
        .map(|b| b.key_label())
}

/// The Normal-mode idle strip: every [`Scope::Diff`] row [`Keymap::default_map`]
/// tags with a [`FooterHint`], merged/sorted by [`keymap_hints`], plus a
/// synthetic `Esc return` hint while a commit view (opened from the git
/// panel's History tab, spec 05 Unit 3) is displayed — `Esc`'s table row has
/// no single fixed label (it also closes help / cancels Visual, see
/// `keymap.rs`'s doc on that row), so this situational label is added here in
/// the [`visual_hints`] mold rather than forced into the static table.
fn normal_hints(
    km: &Keymap,
    staging_allowed: bool,
    code_intel_allowed: bool,
    viewing_commit: bool,
    review_session: bool,
) -> Vec<FooterEntry> {
    let mut entries = keymap_hints(
        km,
        Scope::Diff,
        staging_allowed,
        code_intel_allowed,
        review_session,
    );
    if viewing_commit {
        entries.push(FooterEntry {
            rank: 6,
            key: "Esc".to_string(),
            label: "return",
        });
    }
    if review_session && let Some(key) = find_key(km, Scope::Diff, Action::Quit, "q") {
        // `q`'s table row has no `FooterHint` of its own (see the doc on
        // `Action::Quit`'s binding in `keymap.rs`), since outside a review
        // session it isn't promoted into the idle strip at all — this
        // synthetic entry only exists while reviewing, when `q`'s *meaning*
        // changes (opens the end-review modal rather than quitting), which
        // is worth surfacing here rather than leaving to the always-visible
        // banner text alone.
        entries.push(FooterEntry {
            rank: 9,
            key,
            label: "end review",
        });
    }
    if viewing_commit || review_session {
        entries = sort_for_display(entries);
    }
    entries
}

/// The focused-git-panel idle strip: every [`Scope::Panel`] row tagged with a
/// [`FooterHint`]. Panel bindings are never staging mutations or code-intel
/// requests, so nothing is gated on either capability.
///
/// `push_publishes` relabels the [`Action::RemotePush`] hint to `publish`
/// when the branch has no upstream (see `App::push_publishes`) — a
/// presentation-side relabel in the [`visual_hints`] mold, because the static
/// table can't carry a state-dependent label; the key and its promotion still
/// come from the table.
fn panel_hints(km: &Keymap, push_publishes: bool, review_session: bool) -> Vec<FooterEntry> {
    // Review-status bindings (spec 08 Unit 3) are diff-scope only, so
    // `review_session` never actually changes what this call returns; passed
    // through for signature consistency with `normal_hints`.
    let mut entries = keymap_hints(km, Scope::Panel, true, true, review_session);
    if push_publishes
        && let Some(hint) = km
            .bindings()
            .iter()
            .find(|b| b.scope == Scope::Panel && b.action == Action::RemotePush)
            .and_then(|b| b.footer)
        && let Some(entry) = entries
            .iter_mut()
            .find(|e| e.rank == hint.rank && e.label == hint.label)
    {
        entry.label = "publish";
    }
    // See `normal_hints`'s identical synthetic entry for why this exists
    // only during a review session.
    if review_session && let Some(key) = find_key(km, Scope::Panel, Action::Quit, "q") {
        entries.push(FooterEntry {
            rank: 9,
            key,
            label: "end review",
        });
        entries = sort_for_display(entries);
    }
    entries
}

/// Visual mode's strip: presentation-side relabels of the same Diff-scope
/// bindings Normal mode uses (`j/k` becomes "extend", `c` becomes "comment
/// selection", `Space` becomes "stage lines"), plus a fully synthetic `Esc
/// cancel` — Visual's Esc-cancel isn't a [`Action`] at all (it's handled
/// directly in `super::dispatch_key`), so there is no table row to derive it
/// from. Visual shares the Diff-scope table with Normal, so this can't reuse
/// [`normal_hints`]'s [`FooterHint`] tags (those carry Normal's labels); the
/// *keys* are still looked up in the table, never hardcoded.
fn visual_hints(km: &Keymap, staging_allowed: bool) -> Vec<FooterEntry> {
    let mut entries = Vec::new();
    let move_key = [
        find_key(km, Scope::Diff, Action::CursorDown, "j"),
        find_key(km, Scope::Diff, Action::CursorUp, "k"),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("/");
    if !move_key.is_empty() {
        entries.push(FooterEntry {
            rank: 1,
            key: move_key,
            label: "extend",
        });
    }
    if let Some(key) = find_key(km, Scope::Diff, Action::Compose, "c") {
        entries.push(FooterEntry {
            rank: 2,
            key,
            label: "comment selection",
        });
    }
    if staging_allowed && let Some(key) = find_key(km, Scope::Diff, Action::ToggleStage, "Space") {
        entries.push(FooterEntry {
            rank: 3,
            key,
            label: "stage lines",
        });
    }
    entries.push(FooterEntry {
        rank: 4,
        key: "Esc".to_string(),
        label: "cancel",
    });
    if let Some(key) = find_key(km, Scope::Diff, Action::ToggleHelp, "?") {
        entries.push(FooterEntry {
            rank: 0,
            key,
            label: "help",
        });
    }
    sort_for_display(entries)
}

/// Short label for a two-key completion whose row carries no [`FooterHint`]
/// (so it isn't promoted into any mode's idle strip) but still needs a label
/// while its prefix is pending — currently `gd`/`gr`/`gg`. A completion that
/// *is* tagged (`za`, tagged for the Normal strip) uses that tag's label
/// instead, via [`pending_hints`] — this is purely the fallback for rows the
/// idle strips don't otherwise promote. A test
/// (`every_two_key_binding_has_a_pending_label`, in `footer` tests) fails if
/// a new two-key binding ships without a case here.
fn fallback_pending_label(action: Action) -> &'static str {
    match action {
        Action::GotoDefinition => "definition",
        Action::GotoReferences => "references",
        Action::JumpToTop => "top",
        Action::RecenterCursor => "center",
        Action::ScrollCursorTop => "cursor to top",
        Action::ScrollCursorBottom => "cursor to bottom",
        Action::OpenFileFinder => "find file",
        Action::OpenProjectSearch => "search",
        Action::OpenEditor => "open editor",
        _ => "",
    }
}

/// The completions strip shown while a two-key prefix (`z`, `g`, ...) is
/// pending: every [`Scope::Diff`] two-key binding whose first chord matches
/// `prefix`, via [`Keymap::completions_for`] — never a hardcoded per-prefix
/// list, so a newly bound two-key sequence shows up automatically. Sorted by
/// key text for a stable, predictable order. `code_intel_allowed` drops
/// `gd`/`gr` the same way [`super::help::binding_hidden`] hides them from the
/// help overlay, so a pending `g` never advertises an inert code-intel jump.
fn pending_hints(km: &Keymap, prefix: KeyEvent, code_intel_allowed: bool) -> Vec<FooterEntry> {
    // No two-key sequence is a review action (spec 08 Unit 3's bindings are
    // all single-key), so `review_session` is passed as `true` here — a
    // fixed, always-permissive value, not a real flag — purely to satisfy
    // `binding_hidden`'s signature; it can never actually hide a completion.
    let mut entries: Vec<FooterEntry> = km
        .completions_for(Scope::Diff, prefix)
        .into_iter()
        .filter(|b| !super::help::binding_hidden(b.action, true, code_intel_allowed, true))
        .map(|b| FooterEntry {
            rank: 1,
            key: b.key_label(),
            label: b
                .footer
                .map(|h| h.label)
                .unwrap_or_else(|| fallback_pending_label(b.action)),
        })
        .collect();
    entries.sort_by(|a, b| a.key.cmp(&b.key));
    entries
}

/// The minimal strip shown while the help overlay is open: scroll, `/`
/// filter, close — derived from `app.modal_keys.help`'s own [`FooterHint`]
/// tags. No `? help` entry (the overlay is already open, so it would be
/// redundant).
fn help_open_hints(modal_keys: &ModalKeymaps) -> Vec<FooterEntry> {
    modal_hints(&modal_keys.help)
}

/// The capability/state flags [`build_hints`] needs, bundled into one struct
/// so its own parameter count stays under clippy's `too_many_arguments`
/// threshold — these are all independent booleans, not a cohesive type, but
/// grouping them here is cheaper than growing the function signature further
/// (see `super::mod`'s `draw` and [`super::footer_height`] for the two call
/// sites that build one each frame).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FooterFlags {
    /// `false` on a read-only diff range, hiding the inert stage gestures.
    pub(super) staging_allowed: bool,
    /// `false` whenever the target's new side isn't the live working tree,
    /// hiding the inert `gd`/`gr`/`K` gestures.
    pub(super) code_intel_allowed: bool,
    /// Relabels the panel's `RemotePush` hint to `publish` (see
    /// [`panel_hints`]); only consulted in [`Mode::Panel`].
    pub(super) push_publishes: bool,
    /// Adds the synthetic `Esc return` hint to the Normal strip (see
    /// [`normal_hints`]); only consulted in [`Mode::Normal`].
    pub(super) viewing_commit: bool,
    /// `true` while the help overlay is open, short-circuiting to
    /// [`help_open_hints`] regardless of `mode`.
    pub(super) help_open: bool,
    /// Which half of the Project Search view has focus (see
    /// [`SearchFocus`]); only consulted in [`Mode::ProjectSearch`], picking
    /// between `modal_keys.project_search_input` and
    /// `modal_keys.project_search_results` (spec 06 round-1 UX fix).
    pub(super) project_search_focus: SearchFocus,
    /// Whether a review session is active (spec 08 Unit 2,
    /// [`super::app::App::in_review_session`]); adds a synthetic `q end
    /// review` entry to the Normal/Panel idle strips (see
    /// [`normal_hints`]/[`panel_hints`]) — `q`'s *meaning* changes while
    /// reviewing, so its hint changes too, even though outside a review
    /// session `q` carries no footer hint at all.
    pub(super) review_session: bool,
}

/// Builds the footer strip's hints for the current app state — the pure core
/// [`super::footer_height`] and `super::draw`'s rendering both call, over
/// explicit inputs rather than `&App` so it's unit-testable without
/// constructing a whole app. `pending` is only consulted in
/// [`Mode::Normal`]/[`Mode::Visual`] (the only modes that ever have a pending
/// two-key prefix — see `super::event_loop`). `modal_keys` is `app`'s
/// effective (post-`[keys.<mode>]`-override) tables, spec 07 Unit 4 task 5.4.
/// See [`FooterFlags`] for the rest.
pub(super) fn build_hints(
    mode: Mode,
    flags: FooterFlags,
    pending: Option<KeyEvent>,
    km: &Keymap,
    modal_keys: &ModalKeymaps,
) -> Vec<FooterEntry> {
    let FooterFlags {
        staging_allowed,
        code_intel_allowed,
        push_publishes,
        viewing_commit,
        help_open,
        project_search_focus,
        review_session,
    } = flags;
    if help_open {
        return help_open_hints(modal_keys);
    }
    if let Some(prefix) = pending
        && matches!(mode, Mode::Normal | Mode::Visual { .. })
    {
        return pending_hints(km, prefix, code_intel_allowed);
    }
    match mode {
        Mode::Normal => normal_hints(
            km,
            staging_allowed,
            code_intel_allowed,
            viewing_commit,
            review_session,
        ),
        Mode::Visual { .. } => visual_hints(km, staging_allowed),
        Mode::Panel { .. } => panel_hints(km, push_publishes, review_session),
        Mode::List => modal_hints(&modal_keys.list),
        // Review sessions repurpose `Mode::Staging` as the accepted-files
        // panel (spec 08 Unit 5) — see `super::help::modal_sections`'s
        // identical swap for the `?` overlay.
        Mode::Staging if review_session => modal_hints(&modal_keys.accepted_panel),
        Mode::Staging => modal_hints(&modal_keys.staging),
        Mode::Peek => modal_hints(&modal_keys.peek),
        Mode::Switcher => modal_hints(&modal_keys.switcher),
        Mode::Compose => modal_hints(&modal_keys.compose),
        Mode::CommitMessage => modal_hints(&modal_keys.commit_message),
        Mode::Finder => modal_hints(&modal_keys.finder),
        Mode::ProjectSearch => match project_search_focus {
            SearchFocus::Input => modal_hints(&modal_keys.project_search_input),
            SearchFocus::Results => modal_hints(&modal_keys.project_search_results),
        },
        // The search input occupies the footer itself; no hint strip.
        Mode::Search => Vec::new(),
        Mode::EndReview { .. } => modal_hints(&modal_keys.end_review),
        Mode::ConfirmRemoteOp { .. } => modal_hints(&modal_keys.confirm_remote_op),
        Mode::ReviewBranch => modal_hints(&modal_keys.review_branch),
    }
}

/// The printed width of one hint (`"key label"`), used by [`flow`] to decide
/// where lines break. Plain `chars().count()`, matching the cursor-position
/// math elsewhere in this crate (mod.rs's search-input cursor, help.rs's
/// filter cursor) — good enough for the ASCII-heavy key/label text these
/// tables carry.
fn hint_width(e: &FooterEntry) -> usize {
    e.key.chars().count() + 1 + e.label.chars().count()
}

/// Flows `entries` (already in display order) across as many rows as needed
/// for `width`, one leading space per row and a `" · "` separator between
/// hints on the same row — never splitting a hint, never leaving a dangling
/// separator at a line break. Unbounded (may produce more than 2 rows); the
/// 2-row cap and drop logic live in [`wrap_hints`], which calls this.
fn flow<'a>(entries: &[&'a FooterEntry], width: usize) -> Vec<Vec<&'a FooterEntry>> {
    let mut lines: Vec<Vec<&FooterEntry>> = vec![Vec::new()];
    let mut cur_width = 1usize;
    for &e in entries {
        let w = hint_width(e);
        if lines.last().is_some_and(Vec::is_empty) {
            lines.last_mut().expect("just checked non-empty").push(e);
            cur_width = 1 + w;
            continue;
        }
        let needed = cur_width + 3 + w;
        if needed <= width {
            lines
                .last_mut()
                .expect("flow always has a current line")
                .push(e);
            cur_width = needed;
        } else {
            lines.push(vec![e]);
            cur_width = 1 + w;
        }
    }
    lines
}

/// Flows `entries` across **at most 2 rows** for `width`. If they don't fit
/// even alone, drops hints lowest-priority-first (highest
/// [`FooterHint::rank`] first, ties broken by the drop candidate closest to
/// the end of the list) until they do; rank `0` (`? help`) is never the
/// chosen drop while any other entry remains, so it survives to the end.
pub(super) fn wrap_hints(entries: &[FooterEntry], width: u16) -> Vec<Vec<&FooterEntry>> {
    let width = width as usize;
    let mut candidates: Vec<&FooterEntry> = entries.iter().collect();
    loop {
        let lines = flow(&candidates, width);
        if lines.len() <= 2 || candidates.len() <= 1 {
            return lines;
        }
        let drop_at = candidates
            .iter()
            .enumerate()
            .max_by_key(|(i, e)| (e.rank, *i))
            .map(|(i, _)| i)
            .expect("candidates.len() > 1, checked above");
        candidates.remove(drop_at);
    }
}

/// The number of rows the footer strip needs for `app`'s current state at
/// `width` — the single computation both `draw()`'s `split_footer` call and
/// the event loop's viewport-measurement mirror use, so they can never
/// disagree (rust-best-practices: derived state has one rebuild point).
/// `1` whenever the search input, a remote-op spinner, or a transient status
/// message would occupy the footer instead (those never grow past one row).
pub(super) fn footer_height(
    width: u16,
    app: &App,
    keymap: &Keymap,
    pending: Option<KeyEvent>,
) -> u16 {
    if matches!(app.mode, Mode::Search)
        || app.running_op_label().is_some()
        || app.config_warning_visible()
        || app.status_message.is_some()
    {
        return 1;
    }
    let staging_allowed = app.target.staging_mode() != crate::git::StagingMode::ReadOnly;
    let code_intel_allowed = app.target.supports_code_intel();
    let entries = build_hints(
        app.mode,
        FooterFlags {
            staging_allowed,
            code_intel_allowed,
            push_publishes: app.push_publishes(),
            viewing_commit: app.viewing_commit(),
            help_open: app.help_open,
            project_search_focus: app.project_search_focus(),
            review_session: app.in_review_session(),
        },
        pending,
        keymap,
        &app.modal_keys,
    );
    if entries.is_empty() {
        return 1;
    }
    (wrap_hints(&entries, width).len() as u16).clamp(1, 2)
}

/// Renders one flowed row of hints into a styled [`Line`]: keys bold in
/// `theme.help_key`, labels in `theme.footer_text`, hints joined by `" · "`
/// (matching the emphasis style already used by `help.rs`'s `key_line` and
/// `git_panel.rs`'s `remote_keys_line`).
fn render_line(hints: &[&FooterEntry], theme: &Theme) -> Line<'static> {
    let key_style = Style::default()
        .fg(theme.help_key)
        .add_modifier(Modifier::BOLD);
    let label_style = Style::default().fg(theme.footer_text);
    let mut spans = vec![Span::raw(" ")];
    for (i, h) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" \u{b7} ", label_style));
        }
        spans.push(Span::styled(h.key.clone(), key_style));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(h.label, label_style));
    }
    Line::from(spans)
}

/// Renders `entries` into 1-2 [`Line`]s ready for a `Paragraph`, wrapped to
/// `width` via [`wrap_hints`].
pub(super) fn render_hint_strip(
    entries: &[FooterEntry],
    width: u16,
    theme: &Theme,
) -> Vec<Line<'static>> {
    wrap_hints(entries, width)
        .into_iter()
        .map(|line| render_line(&line, theme))
        .collect()
}

#[cfg(test)]
#[path = "footer_tests.rs"]
mod tests;
