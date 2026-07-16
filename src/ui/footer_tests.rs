use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::diff::FileDiff;
use crate::git::RawFilePatch;
use crate::ui::keymap::KeySeq;
use crate::ui::modal_keys::{
    COMPOSE_HINTS, LIST_KEYS, ModalKeymaps, PEEK_KEYS, STAGING_KEYS, SWITCHER_KEYS,
};
use crate::ui::{App, Mode, Row, dispatch_key};

use super::*;

fn sample_file() -> FileDiff {
    let raw = "\
diff --git a/src/main.rs b/src/main.rs
index 111..222 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,2 +1,2 @@
 fn main() {
-    old();
+    new();
";
    FileDiff::from_patch(&RawFilePatch {
        path: "src/main.rs".to_string(),
        old_path: None,
        raw: raw.to_string(),
        is_binary: false,
    })
    .unwrap()
}

fn app() -> App {
    App::new(vec![sample_file()])
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// The labels a list of [`FooterEntry`] carries, in order — the shape most
/// tests below assert against instead of comparing whole structs.
fn labels(entries: &[FooterEntry]) -> Vec<&'static str> {
    entries.iter().map(|e| e.label).collect()
}

fn keys(entries: &[FooterEntry]) -> Vec<String> {
    entries.iter().map(|e| e.key.clone()).collect()
}

// -- Per-mode hint lists ------------------------------------------------

#[test]
fn normal_mode_hints_match_the_curated_list_in_order() {
    let km = Keymap::default_map();
    let entries = normal_hints(&km, true, true, false, false);
    assert_eq!(
        keys(&entries),
        vec!["j/k", "]", "za", "Space", "S", "c", "/", "`", "?"]
    );
    assert_eq!(
        labels(&entries),
        vec![
            "move",
            "hunk",
            "fold",
            "stage hunk",
            "stage file",
            "comment",
            "search",
            "git panel",
            "help",
        ]
    );
}

/// While a commit view (opened from the git panel's History tab, spec 05
/// Unit 3) is displayed, the Normal strip gains a synthetic `Esc return`
/// hint — `Esc`'s table row is generic ("Close help / cancel selection /
/// return from a commit view"), so this situational label comes from the
/// `viewing_commit` flag rather than the static table.
#[test]
fn normal_mode_hints_gain_esc_return_while_viewing_a_commit() {
    let km = Keymap::default_map();
    let entries = normal_hints(&km, true, true, true, false);
    assert!(labels(&entries).contains(&"return"));
    assert!(keys(&entries).contains(&"Esc".to_string()));
    // Absent otherwise.
    let without = normal_hints(&km, true, true, false, false);
    assert!(!labels(&without).contains(&"return"));
}

#[test]
fn normal_mode_hints_exclude_staging_when_not_allowed() {
    let km = Keymap::default_map();
    let entries = normal_hints(&km, false, true, false, false);
    assert!(!labels(&entries).contains(&"stage hunk"));
    assert!(!labels(&entries).contains(&"stage file"));
    // Everything else survives.
    assert_eq!(entries.len(), 7);
    assert_eq!(labels(&entries).last(), Some(&"help"), "help stays last");
}

/// While a review session is active (spec 08 Unit 2), the Normal strip gains
/// a synthetic `q end review` hint — `q`'s table row carries no
/// `FooterHint` at all outside a review (see `Action::Quit`'s binding in
/// `keymap.rs`), so this entry only exists here, driven by the
/// `review_session` flag rather than the static table.
#[test]
fn normal_mode_hints_gain_q_end_review_during_a_review_session() {
    let km = Keymap::default_map();
    let entries = normal_hints(&km, true, true, false, true);
    assert!(labels(&entries).contains(&"end review"));
    assert!(keys(&entries).contains(&"q".to_string()));
    assert_eq!(labels(&entries).last(), Some(&"help"), "help stays last");
    // Absent outside a review session.
    let without = normal_hints(&km, true, true, false, false);
    assert!(!labels(&without).contains(&"end review"));
}

#[test]
fn visual_mode_hints_are_relabeled_and_synthetic_cancel_is_last_before_help() {
    let km = Keymap::default_map();
    let entries = visual_hints(&km, true);
    assert_eq!(
        labels(&entries),
        vec![
            "extend",
            "comment selection",
            "stage lines",
            "cancel",
            "help"
        ]
    );
    assert_eq!(keys(&entries), vec!["j/k", "c", "Space", "Esc", "?"]);
}

#[test]
fn visual_mode_hints_exclude_stage_lines_when_not_allowed() {
    let km = Keymap::default_map();
    let entries = visual_hints(&km, false);
    assert!(!labels(&entries).contains(&"stage lines"));
    assert_eq!(entries.len(), 4);
}

#[test]
fn panel_mode_hints_match_the_curated_list_in_order() {
    let km = Keymap::default_map();
    let entries = panel_hints(&km, false, false);
    assert_eq!(
        keys(&entries),
        vec!["j/k", "Enter", "f", "p", "P", "c", "`", "Tab", "?"]
    );
    assert_eq!(
        labels(&entries),
        vec![
            "move",
            "open file",
            "fetch",
            "pull",
            "push",
            "commit",
            "close",
            "tab",
            "help"
        ]
    );
}

/// The panel strip's review-session synthetic `q end review` hint — same
/// contract as `normal_mode_hints_gain_q_end_review_during_a_review_session`,
/// for the panel-scope idle strip.
#[test]
fn panel_mode_hints_gain_q_end_review_during_a_review_session() {
    let km = Keymap::default_map();
    let entries = panel_hints(&km, false, true);
    assert!(labels(&entries).contains(&"end review"));
    assert!(keys(&entries).contains(&"q".to_string()));
    assert_eq!(labels(&entries).last(), Some(&"help"), "help stays last");
    let without = panel_hints(&km, false, false);
    assert!(!labels(&without).contains(&"end review"));
}

/// On an unpublished branch (`push_publishes`), the `P` hint relabels to
/// `publish` — same key, same slot, only the label changes — matching what
/// `Action::RemotePush` actually runs in that state (see
/// `App::remote_push_op`).
#[test]
fn panel_push_hint_relabels_to_publish_on_an_unpublished_branch() {
    let km = Keymap::default_map();
    let entries = panel_hints(&km, true, false);
    assert_eq!(
        keys(&entries),
        vec!["j/k", "Enter", "f", "p", "P", "c", "`", "Tab", "?"]
    );
    assert_eq!(
        labels(&entries),
        vec![
            "move",
            "open file",
            "fetch",
            "pull",
            "publish",
            "commit",
            "close",
            "tab",
            "help"
        ]
    );
}

#[test]
fn list_mode_hints_have_no_help_entry() {
    // `?` isn't reachable from Mode::List today (LIST_KEYS doesn't bind it),
    // so the footer must not claim it works — see footer.rs's module doc /
    // the implementation report for this deviation from a literal "every
    // modal mode gets `? help`" reading of the spec.
    let entries = modal_hints(&LIST_KEYS);
    assert_eq!(
        labels(&entries),
        vec!["move", "open", "edit", "delete", "close"]
    );
    assert!(!labels(&entries).contains(&"help"));
}

#[test]
fn staging_mode_hints() {
    let entries = modal_hints(&STAGING_KEYS);
    assert_eq!(labels(&entries), vec!["move", "unstage", "close"]);
}

#[test]
fn peek_mode_hints() {
    let entries = modal_hints(&PEEK_KEYS);
    assert_eq!(labels(&entries), vec!["move", "jump", "close"]);
}

#[test]
fn switcher_mode_hints() {
    let entries = modal_hints(&SWITCHER_KEYS);
    assert_eq!(
        labels(&entries),
        vec!["switch tab", "move", "switch", "close"]
    );
    // "move" must stay MoveDown's own compound label ("j / Down") alone —
    // merging it with MoveUp's ("k / Up") would double up the " / "
    // separators into "j / Down/k / Up". ToggleTab's label lists every bound
    // key (computed from `ModalBinding::key_label`, spec 07 Unit 4 task 5.3 —
    // no longer a hand-curated shorthand), including `Shift-Tab`/`Left`/
    // `Right`, which the old static `"Tab / h / l"` text omitted.
    assert_eq!(
        keys(&entries),
        vec![
            "Tab / Shift-Tab / h / l / Left / Right",
            "j / Down",
            "Enter",
            "Esc"
        ]
    );
}

#[test]
fn compose_mode_hints_are_just_save_and_discard() {
    let entries = modal_hints(&COMPOSE_HINTS);
    assert_eq!(keys(&entries), vec!["Enter", "Esc"]);
    assert_eq!(labels(&entries), vec!["save", "discard"]);
}

#[test]
fn search_mode_has_no_hint_strip() {
    let km = Keymap::default_map();
    let entries = build_hints(
        Mode::Search,
        FooterFlags {
            staging_allowed: true,
            code_intel_allowed: true,
            push_publishes: false,
            viewing_commit: false,
            help_open: false,
            project_search_focus: SearchFocus::Input,
            review_session: false,
        },
        None,
        &km,
        &ModalKeymaps::default(),
    );
    assert!(entries.is_empty());
}

#[test]
fn help_open_hints_are_scroll_filter_close_with_no_help_entry() {
    let entries = help_open_hints(&ModalKeymaps::default());
    assert_eq!(labels(&entries), vec!["scroll", "filter", "close"]);
    assert_eq!(keys(&entries), vec!["j / Down", "/", "Esc / Enter / ?"]);
}

#[test]
fn help_open_takes_precedence_over_the_mode_strip() {
    let km = Keymap::default_map();
    // Even in Mode::Panel (which has its own curated strip), an open help
    // overlay wins.
    let entries = build_hints(
        Mode::Panel {
            cursor: 0,
            tab: crate::ui::app::PanelTab::Changes,
        },
        FooterFlags {
            staging_allowed: true,
            code_intel_allowed: true,
            push_publishes: false,
            viewing_commit: false,
            help_open: true,
            project_search_focus: SearchFocus::Input,
            review_session: false,
        },
        None,
        &km,
        &ModalKeymaps::default(),
    );
    assert_eq!(labels(&entries), vec!["scroll", "filter", "close"]);
}

// -- Pending two-key prefix ------------------------------------------------

#[test]
fn pending_z_shows_za_zb_zt_and_zz_sorted_by_key() {
    let km = Keymap::default_map();
    let entries = pending_hints(&km, key(KeyCode::Char('z')), true);
    assert_eq!(keys(&entries), vec!["za", "zb", "zt", "zz"]);
    assert_eq!(
        labels(&entries),
        vec!["fold", "cursor to bottom", "cursor to top", "center"]
    );
}

#[test]
fn pending_g_shows_every_g_completion_sorted_by_key() {
    let km = Keymap::default_map();
    let entries = pending_hints(&km, key(KeyCode::Char('g')), true);
    assert_eq!(keys(&entries), vec!["g/", "gSpace", "gd", "gg", "gp", "gr"]);
    assert_eq!(
        labels(&entries),
        vec![
            "search",
            "open editor",
            "definition",
            "top",
            "find file",
            "references"
        ]
    );
}

#[test]
fn pending_g_drops_gd_and_gr_when_code_intel_is_disallowed() {
    let km = Keymap::default_map();
    let entries = pending_hints(&km, key(KeyCode::Char('g')), false);
    // `g/` (OpenProjectSearch), `g<Space>` (OpenEditor), `gg` (JumpToTop),
    // and `gp` (OpenFileFinder) aren't code-intel actions, so they survive;
    // `gd`/`gr` don't.
    assert_eq!(keys(&entries), vec!["g/", "gSpace", "gg", "gp"]);
    assert_eq!(
        labels(&entries),
        vec!["search", "open editor", "top", "find file"]
    );
}

#[test]
fn pending_prefix_replaces_the_mode_strip_in_normal_and_visual() {
    let km = Keymap::default_map();
    let g = Some(key(KeyCode::Char('g')));
    let normal = build_hints(
        Mode::Normal,
        FooterFlags {
            staging_allowed: true,
            code_intel_allowed: true,
            push_publishes: false,
            viewing_commit: false,
            help_open: false,
            project_search_focus: SearchFocus::Input,
            review_session: false,
        },
        g,
        &km,
        &ModalKeymaps::default(),
    );
    assert_eq!(keys(&normal), vec!["g/", "gSpace", "gd", "gg", "gp", "gr"]);
    let visual = build_hints(
        Mode::Visual { anchor: 0 },
        FooterFlags {
            staging_allowed: true,
            code_intel_allowed: true,
            push_publishes: false,
            viewing_commit: false,
            help_open: false,
            project_search_focus: SearchFocus::Input,
            review_session: false,
        },
        g,
        &km,
        &ModalKeymaps::default(),
    );
    assert_eq!(keys(&visual), vec!["g/", "gSpace", "gd", "gg", "gp", "gr"]);
}

#[test]
fn build_hints_drops_gd_and_gr_from_the_pending_strip_when_code_intel_is_disallowed() {
    let km = Keymap::default_map();
    let g = Some(key(KeyCode::Char('g')));
    let normal = build_hints(
        Mode::Normal,
        FooterFlags {
            staging_allowed: true,
            code_intel_allowed: false,
            push_publishes: false,
            viewing_commit: false,
            help_open: false,
            project_search_focus: SearchFocus::Input,
            review_session: false,
        },
        g,
        &km,
        &ModalKeymaps::default(),
    );
    assert_eq!(keys(&normal), vec!["g/", "gSpace", "gg", "gp"]);
}

/// A pending prefix is meaningless outside Normal/Visual (the event loop
/// never sets one there), so other modes ignore it defensively rather than
/// showing a stale completions strip.
#[test]
fn pending_prefix_is_ignored_outside_normal_and_visual() {
    let km = Keymap::default_map();
    let g = Some(key(KeyCode::Char('g')));
    let panel = build_hints(
        Mode::Panel {
            cursor: 0,
            tab: crate::ui::app::PanelTab::Changes,
        },
        FooterFlags {
            staging_allowed: true,
            code_intel_allowed: true,
            push_publishes: false,
            viewing_commit: false,
            help_open: false,
            project_search_focus: SearchFocus::Input,
            review_session: false,
        },
        g,
        &km,
        &ModalKeymaps::default(),
    );
    assert_eq!(panel, panel_hints(&km, false, false));
}

#[test]
fn every_two_key_binding_has_a_nonempty_pending_label() {
    let km = Keymap::default_map();
    for b in km
        .bindings()
        .iter()
        .filter(|b| matches!(b.keys, KeySeq::Two(..)))
    {
        let label = b
            .footer
            .map(|h| h.label)
            .unwrap_or_else(|| fallback_pending_label(b.action));
        assert!(
            !label.is_empty(),
            "two-key binding {:?} ({}) has no pending-completion label — \
             add a case to fallback_pending_label or a FooterHint tag",
            b.action,
            b.key_label(),
        );
    }
}

// -- Drift: every mode's strip is non-empty (except Search, by design) -----

#[test]
fn every_mode_produces_a_nonempty_strip_except_search() {
    let km = Keymap::default_map();
    for mode in [
        Mode::Normal,
        Mode::Visual { anchor: 0 },
        Mode::Panel {
            cursor: 0,
            tab: crate::ui::app::PanelTab::Changes,
        },
        Mode::List,
        Mode::Staging,
        Mode::Peek,
        Mode::Switcher,
        Mode::Compose,
        Mode::Finder,
        Mode::ProjectSearch,
    ] {
        let entries = build_hints(
            mode,
            FooterFlags {
                staging_allowed: true,
                code_intel_allowed: true,
                push_publishes: false,
                viewing_commit: false,
                help_open: false,
                project_search_focus: SearchFocus::Input,
                review_session: false,
            },
            None,
            &km,
            &ModalKeymaps::default(),
        );
        assert!(
            !entries.is_empty(),
            "{mode:?} produced an empty footer strip"
        );
    }
    assert!(
        build_hints(
            Mode::Search,
            FooterFlags {
                staging_allowed: true,
                code_intel_allowed: true,
                push_publishes: false,
                viewing_commit: false,
                help_open: false,
                project_search_focus: SearchFocus::Input,
                review_session: false,
            },
            None,
            &km,
            &ModalKeymaps::default(),
        )
        .is_empty()
    );
}

/// Every entry a mode's strip derives from a table carries the key text of
/// some real row (or the atomic pieces of a merged row) in that table/scope
/// — never a fabricated key. Spot-checked across the table-derived modes
/// (the fully synthetic Visual/pending-fallback paths are covered by their
/// own dedicated tests above).
#[test]
fn table_derived_hints_use_real_key_labels() {
    let km = Keymap::default_map();
    for scope in [Scope::Diff, Scope::Panel] {
        let real_labels: Vec<String> = km
            .bindings()
            .iter()
            .filter(|b| b.scope == scope)
            .map(|b| b.key_label())
            .collect();
        let entries = keymap_hints(&km, scope, true, true);
        for e in &entries {
            // Merged entries (`j/k`) join two atomic key labels with `/`; an
            // unmerged entry's key can itself legitimately *be* `/` (the
            // Search binding), so check the whole key first and only fall
            // back to splitting when it isn't a real label as-is.
            let whole_is_real = real_labels.iter().any(|l| l == &e.key);
            let parts_are_real = e
                .key
                .split('/')
                .all(|part| real_labels.iter().any(|l| l == part));
            assert!(
                whole_is_real || parts_are_real,
                "footer key {:?} isn't any real {scope:?}-scope binding's key_label() \
                 (nor a `/`-join of two)",
                e.key,
            );
        }
    }
}

// -- Synthetic hints actually do something (Visual's relabels + Esc-cancel) -

#[test]
fn visual_esc_cancel_actually_returns_to_normal() {
    let mut app = app();
    let keymap = Keymap::default_map();
    let mut pending = None;
    let mut pending_count: Option<usize> = None;
    // Land on a Row::Line, enter Visual, then Esc.
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        key(KeyCode::Char('j')),
    );
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        key(KeyCode::Char('j')),
    );
    assert!(matches!(app.view.rows[app.view.cursor], Row::Line(_)));
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        key(KeyCode::Char('v')),
    );
    assert!(
        matches!(app.mode, Mode::Visual { .. }),
        "v must enter Visual"
    );
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    );
    assert_eq!(
        app.mode,
        Mode::Normal,
        "Esc must cancel Visual back to Normal"
    );
}

#[test]
fn visual_comment_selection_actually_opens_compose() {
    let mut app = app();
    let keymap = Keymap::default_map();
    let mut pending = None;
    let mut pending_count: Option<usize> = None;
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        key(KeyCode::Char('j')),
    );
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        key(KeyCode::Char('j')),
    );
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        key(KeyCode::Char('v')),
    );
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        key(KeyCode::Char('c')),
    );
    assert_eq!(app.mode, Mode::Compose, "Visual `c` must open Compose");
}

#[test]
fn visual_stage_lines_actually_does_something() {
    let mut app = app();
    let keymap = Keymap::default_map();
    let mut pending = None;
    let mut pending_count: Option<usize> = None;
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        key(KeyCode::Char('j')),
    );
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        key(KeyCode::Char('j')),
    );
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        key(KeyCode::Char('v')),
    );
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        key(KeyCode::Char(' ')),
    );
    // No git backend attached, so staging degrades to a footer message —
    // still an observable effect proving Space is live in Visual mode.
    assert!(app.status_message.is_some(), "Visual Space must act");
}

/// `?` genuinely toggles help from the focused git panel now (see the new
/// panel-scope `ToggleHelp` binding in keymap.rs), and once open, `j`/`k`
/// scroll the overlay rather than moving the panel cursor underneath it.
#[test]
fn panel_help_hint_is_real_and_shadows_panel_dispatch() {
    let mut app = app();
    app.mode = Mode::Panel {
        cursor: 0,
        tab: crate::ui::app::PanelTab::Changes,
    };
    let keymap = Keymap::default_map();
    let mut pending = None;
    let mut pending_count: Option<usize> = None;
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        key(KeyCode::Char('?')),
    );
    assert!(app.help_open, "`?` must open help from the panel");
    assert!(matches!(app.mode, Mode::Panel { .. }), "mode stays Panel");
    let scroll_before = app.help_scroll.get();
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        key(KeyCode::Char('j')),
    );
    assert!(
        app.help_scroll.get() > scroll_before,
        "j must scroll help, not move the panel cursor, while help is open over the panel"
    );
    dispatch_key(
        &mut app,
        &keymap,
        &mut pending,
        &mut pending_count,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    );
    assert!(!app.help_open, "Esc must close help");
    assert!(
        matches!(app.mode, Mode::Panel { .. }),
        "closing help returns to the panel, not Normal"
    );
}

// -- Wrapping ---------------------------------------------------------------

#[test]
fn generous_width_fits_the_whole_normal_strip_on_one_line() {
    let km = Keymap::default_map();
    let entries = normal_hints(&km, true, true, false, false);
    let lines = wrap_hints(&entries, 120);
    assert_eq!(lines.len(), 1);
    let shown: usize = lines.iter().map(Vec::len).sum();
    assert_eq!(shown, entries.len(), "no hint dropped at generous width");
}

#[test]
fn medium_width_wraps_to_two_lines_without_splitting_a_hint() {
    let km = Keymap::default_map();
    let entries = normal_hints(&km, true, true, false, false);
    let lines = wrap_hints(&entries, 60);
    assert_eq!(lines.len(), 2);
    let shown: usize = lines.iter().map(Vec::len).sum();
    assert_eq!(shown, entries.len(), "no hint dropped at medium width");
    // Every hint's full "key label" text fits within `width` on its own row
    // (i.e. nothing was truncated mid-hint).
    for line in &lines {
        for e in line {
            assert!(hint_width(e) <= 60);
        }
    }
}

#[test]
fn narrow_width_drops_lowest_priority_hints_but_keeps_help() {
    let km = Keymap::default_map();
    let entries = normal_hints(&km, true, true, false, false);
    let lines = wrap_hints(&entries, 20);
    assert!(lines.len() <= 2);
    let shown: usize = lines.iter().map(Vec::len).sum();
    assert!(shown < entries.len(), "narrow width must drop some hints");
    let shown_labels: Vec<&str> = lines.iter().flatten().map(|e| e.label).collect();
    assert!(
        shown_labels.contains(&"help"),
        "`? help` must survive narrow-width dropping"
    );
}

#[test]
fn dropping_never_removes_help_while_anything_else_remains() {
    // An adversarially narrow width: keeps forcing drops down to the wire.
    let km = Keymap::default_map();
    let entries = normal_hints(&km, true, true, false, false);
    let lines = wrap_hints(&entries, 8);
    let shown_labels: Vec<&str> = lines.iter().flatten().map(|e| e.label).collect();
    assert!(shown_labels.contains(&"help"));
}

// -- footer_height / split_footer wiring ------------------------------------

#[test]
fn footer_height_is_one_row_when_status_message_is_set() {
    let km = Keymap::default_map();
    let mut a = app();
    a.set_status_message("hi");
    assert_eq!(footer_height(20, &a, &km, None), 1);
}

#[test]
fn footer_height_matches_wrap_hints_row_count() {
    let km = Keymap::default_map();
    let a = app();
    let entries = build_hints(
        a.mode,
        FooterFlags {
            staging_allowed: true,
            code_intel_allowed: true,
            push_publishes: false,
            viewing_commit: false,
            help_open: a.help_open,
            project_search_focus: a.project_search_focus(),
            review_session: false,
        },
        None,
        &km,
        &a.modal_keys,
    );
    let expected = wrap_hints(&entries, 60).len() as u16;
    assert_eq!(footer_height(60, &a, &km, None), expected);
}
