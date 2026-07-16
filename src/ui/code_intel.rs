//! Code-intelligence glue: correlating the diff cursor with `gd`/`gr`/`K`
//! LSP requests, routing the responses into the peek overlay, and the
//! peek-overlay navigation. Kept out of [`super::App`] so the coordinator
//! stays thin; these functions take the view's cursor position as input and
//! drive the LSP client (via [`super::lsp_ops::LspClient`]) and the peek
//! state, never blocking the render loop.
//!
//! ## Degradation contract: code-intel is silently absent off the live working tree
//!
//! An LSP server only ever sees the file *as it sits on disk right now* — it
//! has no notion of "the version this diff's new side shows," so a request
//! is only meaningful when that new side *is* the on-disk working tree.
//! [`request`] and [`refresh_peek_preview`] both gate on
//! [`crate::git::DiffTarget::supports_code_intel`] and, when it's `false`
//! (every target but [`crate::git::DiffTarget::WorkingTree`] today — a
//! staged diff shows the index's content, a range diff shows two historical
//! revisions, neither backed by the file at that path on disk), degrade
//! silently: `gd`/`gr`/`K` set the same `"no code intelligence here"` footer
//! message as any other request that can't start (no repo root, missing
//! file, unsupported language), rather than an error. The alternative —
//! resolving the request against on-disk content that doesn't match what's
//! displayed — would silently jump to the wrong line or definition, which is
//! strictly worse than the feature being unavailable (this was the actual
//! bug on range views before this gate existed).
//!
//! The same predicate drives which of `gd`/`gr`/`K` even *appear* in the `?`
//! help overlay and footer strip (see [`super::help::binding_hidden`],
//! consumed by [`super::footer`]) — mirroring how the staging keys already
//! hide on a read-only target — so the degradation is structurally invisible
//! rather than a key that's listed but silently does nothing.
//!
//! Per the repository's error-handling rules, this degrade-silently choice is
//! deliberate and scoped to *this* subsystem only: it does not license
//! swallowing errors elsewhere (a real LSP failure still surfaces via
//! `"lsp: failed"`, see [`handle_event`]).

use crate::diff::LineOrigin;
use crate::highlight::Lang;
use crate::lsp::{LspEvent, LspManager, SourceLocation};

use super::App;
use super::Mode;
use super::diff_view_state::DiffViewState;
use super::peek::{CachedPreview, PeekKind, PeekState};
use super::rows::Row;

/// Derives the `(repo-relative path, 0-based line, UTF-16 character)`
/// position `gd`/`gr`/`K` would request for the view's current cursor.
/// Valid only on [`Row::Line`] rows with a `new_line` (Added/Context — a
/// `Removed` line has no position in the file as it exists on disk). `None`
/// on any other row.
fn code_intel_position(view: &DiffViewState) -> Option<(String, u32, u32)> {
    let file = view.files.get(view.file_of_cursor())?;
    let Row::Line(line) = view.rows.get(view.cursor)? else {
        return None;
    };
    if !matches!(line.origin, LineOrigin::Added | LineOrigin::Context) {
        return None;
    }
    let new_line = line.new_line?;
    let col = view.effective_column().unwrap_or(0);
    let character = utf16_offset(&line.content, col);
    Some((file.path.clone(), new_line - 1, character))
}

/// Issues a `gd`/`gr`/`K` request for the cursor's current position:
/// validates the row and the file's on-disk existence, lazily creates the
/// LSP client against `repo_root` on first use, and records the request as
/// pending. Sets a footer message either way — `"lsp: resolving…"` while
/// awaiting a response, or `"no code intelligence here"` for any case that
/// can't even start a request (invalid row, no repo root, missing file, no
/// server available for this language, or the diff target's new side isn't
/// the live working tree — see the module doc). A new request always
/// supersedes interest in whatever was previously pending.
pub(super) fn request(app: &mut App, kind: PeekKind) {
    if !app.target.supports_code_intel() {
        app.set_status_message("no code intelligence here");
        return;
    }
    let Some((path, line, character)) = code_intel_position(&app.view) else {
        app.set_status_message("no code intelligence here");
        return;
    };
    let Some(root) = app.repo_root.clone() else {
        app.set_status_message("no code intelligence here");
        return;
    };
    let abs_path = root.join(&path);
    if !abs_path.is_file() {
        app.set_status_message("no code intelligence here");
        return;
    }

    if app.lsp.is_none() {
        let commands = super::lsp_config::effective_lsp_commands(&app.config.lsp);
        app.lsp = Some(Box::new(LspManager::with_commands(root, commands)));
    }
    // A new request always cancels interest in whatever was pending.
    app.pending_lsp = None;
    let Some(lsp) = app.lsp.as_mut() else {
        return;
    };
    let request = match kind {
        PeekKind::Definition => lsp.request_definition(&abs_path, line, character),
        PeekKind::References => lsp.request_references(&abs_path, line, character),
        PeekKind::Hover => lsp.request_hover(&abs_path, line, character),
    };
    match request {
        Some(id) => {
            app.pending_lsp = Some((id, kind));
            app.set_status_message("lsp: resolving\u{2026}");
        }
        None => app.set_status_message("no code intelligence here"),
    }
}

/// Drains events from the LSP client (if one exists) and routes them. Never
/// blocks; a no-op without a live client. Called once per event loop tick,
/// on both a keypress and a timeout, so responses keep flowing while the
/// user isn't typing.
pub(super) fn poll(app: &mut App) {
    let events = {
        let Some(lsp) = app.lsp.as_mut() else {
            return;
        };
        lsp.poll()
    };
    for event in events {
        handle_event(app, event);
    }
}

/// Routes one [`LspEvent`]: an id that doesn't match the currently pending
/// request is ignored (a stale response, or one superseded by a newer
/// request). A matching event opens the peek overlay (Definition/References
/// with results, or Hover), or sets a footer message instead (`"no results"`
/// for an empty location list, `"lsp: failed"` for [`LspEvent::Failed`]).
fn handle_event(app: &mut App, event: LspEvent) {
    let Some((pending_id, kind)) = app.pending_lsp else {
        return;
    };
    let id = match &event {
        LspEvent::Definition { id, .. } => *id,
        LspEvent::References { id, .. } => *id,
        LspEvent::Hover { id, .. } => *id,
        LspEvent::Failed { id } => *id,
    };
    if id != pending_id {
        return;
    }
    app.pending_lsp = None;

    match event {
        LspEvent::Definition { locations, .. } => open_peek_locations(app, kind, locations),
        LspEvent::References { locations, .. } => open_peek_locations(app, kind, locations),
        LspEvent::Hover { contents, .. } => {
            app.peek = Some(PeekState::hover(contents));
            app.mode = Mode::Peek;
        }
        LspEvent::Failed { .. } => app.set_status_message("lsp: failed"),
    }
}

fn open_peek_locations(app: &mut App, kind: PeekKind, locations: Vec<SourceLocation>) {
    if locations.is_empty() {
        app.set_status_message("no results");
        return;
    }
    app.peek = Some(PeekState::locations(kind, locations));
    app.mode = Mode::Peek;
    refresh_peek_preview(app);
}

/// Populates the preview cache for the currently selected location, if it
/// isn't already cached: reads the file from disk and highlights it
/// (best-effort — an unreadable file or unsupported language leaves it
/// uncached, and the overlay shows "(preview unavailable)"). A no-op for
/// Hover (no location list), once a path is already cached, or whenever the
/// diff target's new side isn't the live working tree — see the module doc
/// — since [`request`] never opens the overlay in that state, but this stays
/// defensive against a target change landing mid-flight.
fn refresh_peek_preview(app: &mut App) {
    if !app.target.supports_code_intel() {
        return;
    }
    let Some(peek) = app.peek.as_ref() else {
        return;
    };
    if matches!(peek.kind, PeekKind::Hover) {
        return;
    }
    let Some(loc) = peek.locations.get(peek.selected) else {
        return;
    };
    let path = loc.path.clone();
    if peek.preview_cache.contains_key(&path) {
        return;
    }
    let Ok(content) = std::fs::read_to_string(&path) else {
        return;
    };
    let lines: Vec<String> = content.lines().map(str::to_string).collect();
    let spans = match path.to_str().and_then(Lang::from_path) {
        Some(lang) => app.highlighter.highlight_lines(lang, &content),
        None => Vec::new(),
    };
    if let Some(peek) = app.peek.as_mut() {
        peek.preview_cache
            .insert(path, CachedPreview { lines, spans });
    }
}

/// Moves the peek selection down one result (Definition/References), or
/// scrolls the hover text down one line. A no-op if no overlay is open.
pub(super) fn peek_move_down(app: &mut App) {
    let Some(peek) = app.peek.as_mut() else {
        return;
    };
    match peek.kind {
        PeekKind::Hover => {
            let max = peek.hover_line_count().saturating_sub(1);
            peek.hover_scroll = (peek.hover_scroll + 1).min(max);
        }
        PeekKind::Definition | PeekKind::References => {
            if !peek.locations.is_empty() {
                peek.selected = (peek.selected + 1).min(peek.locations.len() - 1);
            }
            refresh_peek_preview(app);
        }
    }
}

/// Moves the peek selection up one result, or scrolls hover text up one
/// line. A no-op if no overlay is open.
pub(super) fn peek_move_up(app: &mut App) {
    let Some(peek) = app.peek.as_mut() else {
        return;
    };
    match peek.kind {
        PeekKind::Hover => peek.hover_scroll = peek.hover_scroll.saturating_sub(1),
        PeekKind::Definition | PeekKind::References => {
            peek.selected = peek.selected.saturating_sub(1);
            refresh_peek_preview(app);
        }
    }
}

/// Closes the peek overlay, returning to [`Mode::Normal`].
pub(super) fn close_peek(app: &mut App) {
    app.peek = None;
    app.mode = Mode::Normal;
}

/// Applies the Peek-mode `Enter` gesture: for Definition/References, jumps
/// the diff cursor to the closest row for the selected result's new-side
/// line and closes the overlay if the result's file is one of the diff's
/// files, or sets a `"not in diff"` footer message (v1 — full cross-file
/// browsing is out of scope) otherwise. A no-op for Hover.
pub(super) fn peek_enter(app: &mut App) {
    let Some(peek) = &app.peek else {
        return;
    };
    if !matches!(peek.kind, PeekKind::Definition | PeekKind::References) {
        return;
    }
    let Some(loc) = peek.locations.get(peek.selected) else {
        return;
    };
    let target_path = loc.path.clone();
    let target_line = loc.line + 1; // 0-based LSP line -> 1-based new_line

    let file_index = app.view.files.iter().position(|f| {
        app.repo_root
            .as_ref()
            .map(|root| root.join(&f.path) == target_path)
            .unwrap_or(false)
    });

    let Some(file_index) = file_index else {
        app.set_status_message("not in diff");
        return;
    };

    // Expand the target section if collapsed, rebuild, then land within that
    // file's row span so the closest-line search never picks another file's
    // row.
    let path = app.view.files[file_index].path.clone();
    app.view.set_collapsed(&path, false);
    app.rebuild_rows();
    let (start, end) = app.view.section_span(file_index);
    let local = closest_row_for_new_line(&app.view.rows[start..end], target_line).unwrap_or(0);
    app.view.cursor = start + local;
    app.view.scroll = 0;
    app.view.ensure_visible();
    close_peek(app);
}

/// Converts a 0-based char index within `content` to its UTF-16 code-unit
/// offset, matching the LSP position convention (`gd`/`gr`/`K` requests use
/// this to convert the column cursor's char index into a wire position).
/// Characters outside the Basic Multilingual Plane (e.g. most emoji) count
/// as 2 UTF-16 units, per [`char::len_utf16`].
fn utf16_offset(content: &str, char_index: usize) -> u32 {
    content
        .chars()
        .take(char_index)
        .map(char::len_utf16)
        .sum::<usize>() as u32
}

/// The row in `rows` whose `new_line` is closest to `target_line` (ties
/// broken toward the earlier row). `None` if `rows` has no `Line` row with
/// a `new_line` at all. `pub(super)` so [`super::file_view`] can reuse it for
/// the read-only file view's open-at-line support (spec 06 Unit 1) — the
/// same "land on the nearest line" need `peek_enter` has.
pub(super) fn closest_row_for_new_line(rows: &[Row], target_line: u32) -> Option<usize> {
    rows.iter()
        .enumerate()
        .filter_map(|(i, r)| match r {
            Row::Line(l) => l.new_line.map(|n| (i, n)),
            _ => None,
        })
        .min_by_key(|&(_, n)| (i64::from(n) - i64::from(target_line)).abs())
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::Mutex;

    use super::*;
    use crate::diff::FileDiff;
    use crate::git::{DiffTarget, RawFilePatch};
    use crate::lsp::RequestId;
    use crate::ui::Action;
    use crate::ui::LspClient;
    use crate::ui::rows::LineRow;

    fn file(path: &str, hunk_count: usize) -> FileDiff {
        let mut raw = format!(
            "diff --git a/{path} b/{path}\nindex 1..2 100644\n--- a/{path}\n+++ b/{path}\n"
        );
        for h in 0..hunk_count {
            let start = 1 + h * 10;
            raw.push_str(&format!("@@ -{start},1 +{start},1 @@\n-old{h}\n+new{h}\n"));
        }
        FileDiff::from_patch(&RawFilePatch {
            path: path.to_string(),
            old_path: None,
            raw,
            is_binary: false,
        })
        .unwrap()
    }

    fn file_with_raw(path: &str, raw: &str) -> FileDiff {
        FileDiff::from_patch(&RawFilePatch {
            path: path.to_string(),
            old_path: None,
            raw: raw.to_string(),
            is_binary: false,
        })
        .unwrap()
    }

    // -- UTF-16 offset conversion (for LSP position derivation) ----------------

    #[test]
    fn utf16_offset_ascii_matches_char_index() {
        assert_eq!(utf16_offset("hello", 3), 3);
    }

    #[test]
    fn utf16_offset_multibyte_bmp_char_counts_as_one_unit() {
        // 'é' is 2 bytes in UTF-8 but a single UTF-16 code unit.
        assert_eq!(utf16_offset("café", 4), 4);
    }

    #[test]
    fn utf16_offset_surrogate_pair_counts_as_two_units() {
        // An emoji outside the BMP is one `char` but 2 UTF-16 code units.
        let content = "a\u{1F600}b";
        assert_eq!(utf16_offset(content, 0), 0); // before 'a'
        assert_eq!(utf16_offset(content, 1), 1); // before the emoji
        assert_eq!(utf16_offset(content, 2), 3); // after the emoji (1 + 2)
    }

    // -- LSP: gd/gr/K request routing and event handling ------------------------

    #[derive(Debug, Clone, PartialEq)]
    enum LspCall {
        Definition(PathBuf, u32, u32),
        References(PathBuf, u32, u32),
        Hover(PathBuf, u32, u32),
    }

    struct FakeLsp {
        calls: Arc<Mutex<Vec<LspCall>>>,
        next_id: u64,
        deny: bool,
        poll_queue: Arc<Mutex<std::collections::VecDeque<Vec<LspEvent>>>>,
        shutdown_called: Arc<Mutex<bool>>,
    }

    impl FakeLsp {
        fn record(&mut self, call: LspCall) -> Option<RequestId> {
            if self.deny {
                return None;
            }
            self.next_id += 1;
            self.calls.lock().unwrap().push(call);
            Some(RequestId(self.next_id))
        }
    }

    impl LspClient for FakeLsp {
        fn request_definition(
            &mut self,
            path: &std::path::Path,
            line: u32,
            character: u32,
        ) -> Option<RequestId> {
            self.record(LspCall::Definition(path.to_path_buf(), line, character))
        }

        fn request_references(
            &mut self,
            path: &std::path::Path,
            line: u32,
            character: u32,
        ) -> Option<RequestId> {
            self.record(LspCall::References(path.to_path_buf(), line, character))
        }

        fn request_hover(
            &mut self,
            path: &std::path::Path,
            line: u32,
            character: u32,
        ) -> Option<RequestId> {
            self.record(LspCall::Hover(path.to_path_buf(), line, character))
        }

        fn poll(&mut self) -> Vec<LspEvent> {
            self.poll_queue
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_default()
        }

        fn shutdown(self: Box<Self>) {
            *self.shutdown_called.lock().unwrap() = true;
        }
    }

    /// A diff over `path` with rows: FileHeader(0) HunkHeader(1)
    /// context "fn main() {" new_line=1 (2) removed "    old();" (3) added
    /// "    new();" new_line=2 (4).
    fn lsp_fixture_raw() -> &'static str {
        "\
diff --git a/src/main.rs b/src/main.rs
index 1..2 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,2 +1,2 @@
 fn main() {
-    old();
+    new();
"
    }

    /// An `App` over `src/main.rs`, whose path also exists as a real file
    /// under a fresh tempdir (`gd`/`gr`/`K` check the file exists on disk),
    /// wired to a `FakeLsp` via `inject_lsp_client`. Returns the app, the
    /// tempdir (kept alive so the file keeps existing), and handles to
    /// inspect issued calls / feed scripted `poll()` responses.
    #[allow(clippy::type_complexity)]
    fn lsp_test_app() -> (
        App,
        tempfile::TempDir,
        Arc<Mutex<Vec<LspCall>>>,
        Arc<Mutex<std::collections::VecDeque<Vec<LspEvent>>>>,
    ) {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join("src")).expect("mkdir");
        std::fs::write(
            tmp.path().join("src/main.rs"),
            "fn main() {\n    new();\n}\n",
        )
        .expect("write fixture");

        let mut app = App::new(vec![file_with_raw("src/main.rs", lsp_fixture_raw())]);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let poll_queue = Arc::new(Mutex::new(std::collections::VecDeque::new()));
        let fake = FakeLsp {
            calls: Arc::clone(&calls),
            next_id: 0,
            deny: false,
            poll_queue: Arc::clone(&poll_queue),
            shutdown_called: Arc::new(Mutex::new(false)),
        };
        app.inject_lsp_client(Box::new(fake), tmp.path().to_path_buf());
        (app, tmp, calls, poll_queue)
    }

    /// Moves the cursor onto the fixture's added line (`    new();`,
    /// new_line 2) — the only row `code_intel_position` accepts.
    fn move_to_added_line(app: &mut App) {
        for _ in 0..4 {
            app.apply(Action::CursorDown);
        }
    }

    #[test]
    fn gd_on_removed_line_sets_no_code_intelligence_message() {
        let (mut app, _tmp, calls, _poll) = lsp_test_app();
        app.apply(Action::CursorDown); // hunk header
        app.apply(Action::CursorDown); // context line
        app.apply(Action::CursorDown); // removed line
        assert!(matches!(
            app.view.rows[app.view.cursor],
            Row::Line(LineRow {
                origin: LineOrigin::Removed,
                ..
            })
        ));
        app.apply(Action::GotoDefinition);
        assert!(calls.lock().unwrap().is_empty());
        assert_eq!(
            app.status_message.as_deref(),
            Some("no code intelligence here")
        );
    }

    #[test]
    fn gd_on_header_row_sets_no_code_intelligence_message() {
        let (mut app, _tmp, calls, _poll) = lsp_test_app();
        assert!(matches!(
            app.view.rows[app.view.cursor],
            Row::FileHeader { .. }
        ));
        app.apply(Action::GotoDefinition);
        assert!(calls.lock().unwrap().is_empty());
        assert_eq!(
            app.status_message.as_deref(),
            Some("no code intelligence here")
        );
    }

    #[test]
    fn gd_on_missing_file_sets_no_code_intelligence_message() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        // Deliberately no file written under `tmp` at "src/main.rs".
        let mut app = App::new(vec![file_with_raw("src/main.rs", lsp_fixture_raw())]);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let fake = FakeLsp {
            calls: Arc::clone(&calls),
            next_id: 0,
            deny: false,
            poll_queue: Arc::new(Mutex::new(std::collections::VecDeque::new())),
            shutdown_called: Arc::new(Mutex::new(false)),
        };
        app.inject_lsp_client(Box::new(fake), tmp.path().to_path_buf());
        move_to_added_line(&mut app);
        app.apply(Action::GotoDefinition);
        assert!(calls.lock().unwrap().is_empty());
        assert_eq!(
            app.status_message.as_deref(),
            Some("no code intelligence here")
        );
    }

    #[test]
    fn gd_without_repo_root_sets_no_code_intelligence_message() {
        let mut app = App::new(vec![file_with_raw("src/main.rs", lsp_fixture_raw())]);
        move_to_added_line(&mut app);
        app.apply(Action::GotoDefinition);
        assert_eq!(
            app.status_message.as_deref(),
            Some("no code intelligence here")
        );
    }

    #[test]
    fn gd_on_valid_row_dispatches_request_and_sets_resolving_message() {
        let (mut app, tmp, calls, _poll) = lsp_test_app();
        move_to_added_line(&mut app);
        app.apply(Action::GotoDefinition);
        assert_eq!(
            calls.lock().unwrap()[0],
            LspCall::Definition(tmp.path().join("src/main.rs"), 1, 0)
        );
        assert_eq!(
            app.status_message.as_deref(),
            Some("lsp: resolving\u{2026}")
        );
    }

    // -- Capability gate: no code-intel off the live working tree ----------

    #[test]
    fn gd_on_a_staged_target_sets_no_code_intelligence_message_without_dispatching() {
        let (mut app, _tmp, calls, _poll) = lsp_test_app();
        app.target = DiffTarget::Staged;
        move_to_added_line(&mut app);
        app.apply(Action::GotoDefinition);
        assert!(
            calls.lock().unwrap().is_empty(),
            "a Staged target must never reach the LSP client"
        );
        assert_eq!(
            app.status_message.as_deref(),
            Some("no code intelligence here")
        );
    }

    #[test]
    fn gr_and_k_are_also_gated_on_a_range_target() {
        let (mut app, _tmp, calls, _poll) = lsp_test_app();
        app.target = DiffTarget::Range("main..HEAD".to_string());
        move_to_added_line(&mut app);
        app.apply(Action::GotoReferences);
        app.apply(Action::Hover);
        assert!(
            calls.lock().unwrap().is_empty(),
            "a Range target must never reach the LSP client for gr or K either"
        );
        assert_eq!(
            app.status_message.as_deref(),
            Some("no code intelligence here")
        );
    }

    #[test]
    fn peek_preview_refresh_is_a_noop_on_a_non_worktree_target() {
        // Defense in depth: even if a peek overlay were already open (e.g. a
        // target change landed mid-flight), refresh_peek_preview must not
        // read from disk on a target `request` would have refused.
        let mut app = App::new(vec![file_with_raw("src/main.rs", lsp_fixture_raw())]);
        app.target = DiffTarget::Staged;
        app.peek = Some(PeekState::locations(
            PeekKind::Definition,
            vec![SourceLocation {
                path: PathBuf::from("/does/not/matter.rs"),
                line: 0,
                character: 0,
            }],
        ));
        refresh_peek_preview(&mut app);
        assert!(
            app.peek.as_ref().unwrap().preview_cache.is_empty(),
            "must not populate the preview cache on a non-worktree target"
        );
    }

    #[test]
    fn gr_and_k_dispatch_their_own_request_kinds() {
        let (mut app, tmp, calls, _poll) = lsp_test_app();
        move_to_added_line(&mut app);
        app.apply(Action::GotoReferences);
        assert_eq!(
            calls.lock().unwrap()[0],
            LspCall::References(tmp.path().join("src/main.rs"), 1, 0)
        );
        app.apply(Action::Hover);
        assert_eq!(
            calls.lock().unwrap()[1],
            LspCall::Hover(tmp.path().join("src/main.rs"), 1, 0)
        );
    }

    #[test]
    fn gd_uses_the_column_cursor_for_the_character_offset() {
        let (mut app, tmp, calls, _poll) = lsp_test_app();
        move_to_added_line(&mut app); // "    new();" -- col 4 is 'n'
        for _ in 0..4 {
            app.apply(Action::CursorRight);
        }
        app.apply(Action::GotoDefinition);
        assert_eq!(
            calls.lock().unwrap()[0],
            LspCall::Definition(tmp.path().join("src/main.rs"), 1, 4)
        );
    }

    #[test]
    fn second_request_supersedes_interest_in_the_first_pending_id() {
        let (mut app, _tmp, _calls, poll_queue) = lsp_test_app();
        move_to_added_line(&mut app);

        app.apply(Action::GotoDefinition);
        let first_id = app.pending_lsp.expect("pending after gd").0;
        app.apply(Action::GotoReferences);
        let second_id = app.pending_lsp.expect("pending after gr").0;
        assert_ne!(first_id, second_id);

        // A response for the superseded first id must be ignored.
        poll_queue
            .lock()
            .unwrap()
            .push_back(vec![LspEvent::Definition {
                id: first_id,
                locations: vec![SourceLocation {
                    path: PathBuf::from("/tmp/unused.rs"),
                    line: 0,
                    character: 0,
                }],
            }]);
        poll(&mut app);
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.peek.is_none());

        // The second (References) request's own response still opens the
        // overlay.
        poll_queue
            .lock()
            .unwrap()
            .push_back(vec![LspEvent::References {
                id: second_id,
                locations: vec![SourceLocation {
                    path: PathBuf::from("/tmp/unused.rs"),
                    line: 0,
                    character: 0,
                }],
            }]);
        poll(&mut app);
        assert_eq!(app.mode, Mode::Peek);
    }

    #[test]
    fn unrelated_event_id_is_ignored() {
        let (mut app, _tmp, _calls, poll_queue) = lsp_test_app();
        move_to_added_line(&mut app);
        app.apply(Action::GotoDefinition);
        let real_id = app.pending_lsp.expect("pending after gd").0;

        poll_queue
            .lock()
            .unwrap()
            .push_back(vec![LspEvent::Definition {
                id: RequestId(real_id.0 + 999),
                locations: vec![SourceLocation {
                    path: PathBuf::from("/tmp/unused.rs"),
                    line: 0,
                    character: 0,
                }],
            }]);
        poll(&mut app);
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.pending_lsp.map(|(id, _)| id), Some(real_id));
    }

    #[test]
    fn empty_definition_result_sets_no_results_message() {
        let (mut app, _tmp, _calls, poll_queue) = lsp_test_app();
        move_to_added_line(&mut app);
        app.apply(Action::GotoDefinition);
        let id = app.pending_lsp.expect("pending after gd").0;

        poll_queue
            .lock()
            .unwrap()
            .push_back(vec![LspEvent::Definition {
                id,
                locations: vec![],
            }]);
        poll(&mut app);
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.status_message.as_deref(), Some("no results"));
    }

    #[test]
    fn failed_event_sets_footer_message_and_does_not_open_overlay() {
        let (mut app, _tmp, _calls, poll_queue) = lsp_test_app();
        move_to_added_line(&mut app);
        app.apply(Action::Hover);
        let id = app.pending_lsp.expect("pending after K").0;

        poll_queue
            .lock()
            .unwrap()
            .push_back(vec![LspEvent::Failed { id }]);
        poll(&mut app);
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.status_message.as_deref(), Some("lsp: failed"));
    }

    #[test]
    fn hover_event_opens_peek_overlay_with_contents() {
        let (mut app, _tmp, _calls, poll_queue) = lsp_test_app();
        move_to_added_line(&mut app);
        app.apply(Action::Hover);
        let id = app.pending_lsp.expect("pending after K").0;

        poll_queue.lock().unwrap().push_back(vec![LspEvent::Hover {
            id,
            contents: "some docs".to_string(),
        }]);
        poll(&mut app);
        assert_eq!(app.mode, Mode::Peek);
        assert_eq!(app.peek.as_ref().unwrap().hover_text, "some docs");
    }

    #[test]
    fn take_lsp_client_returns_the_injected_client_once() {
        let (mut app, _tmp, _calls, _poll) = lsp_test_app();
        assert!(app.take_lsp_client().is_some());
        assert!(app.take_lsp_client().is_none());
    }

    // -- Peek overlay -------------------------------------------------------

    fn source_loc(path: &std::path::Path, line: u32) -> SourceLocation {
        SourceLocation {
            path: path.to_path_buf(),
            line,
            character: 0,
        }
    }

    #[test]
    fn peek_move_down_and_up_clamp_selection() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.peek = Some(PeekState::locations(
            PeekKind::References,
            vec![
                source_loc(std::path::Path::new("/tmp/a.rs"), 0),
                source_loc(std::path::Path::new("/tmp/b.rs"), 1),
            ],
        ));
        app.mode = Mode::Peek;

        peek_move_down(&mut app);
        assert_eq!(app.peek.as_ref().unwrap().selected, 1);
        peek_move_down(&mut app); // clamped at last
        assert_eq!(app.peek.as_ref().unwrap().selected, 1);
        peek_move_up(&mut app);
        assert_eq!(app.peek.as_ref().unwrap().selected, 0);
        peek_move_up(&mut app); // clamped at first
        assert_eq!(app.peek.as_ref().unwrap().selected, 0);
    }

    #[test]
    fn hover_scroll_moves_and_clamps() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.peek = Some(PeekState::hover("one\ntwo\nthree".to_string()));
        app.mode = Mode::Peek;

        peek_move_down(&mut app);
        assert_eq!(app.peek.as_ref().unwrap().hover_scroll, 1);
        for _ in 0..5 {
            peek_move_down(&mut app);
        }
        assert_eq!(app.peek.as_ref().unwrap().hover_scroll, 2); // clamped
        peek_move_up(&mut app);
        assert_eq!(app.peek.as_ref().unwrap().hover_scroll, 1);
        peek_move_up(&mut app);
        peek_move_up(&mut app);
        assert_eq!(app.peek.as_ref().unwrap().hover_scroll, 0); // clamped at 0
    }

    #[test]
    fn close_peek_returns_to_normal() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.peek = Some(PeekState::hover("x".to_string()));
        app.mode = Mode::Peek;
        close_peek(&mut app);
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.peek.is_none());
    }

    #[test]
    fn peek_enter_jumps_into_diff_when_path_matches_a_diff_file() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let mut app = App::new(vec![file_with_raw("src/main.rs", lsp_fixture_raw())]);
        app.set_repo_root(tmp.path().to_path_buf());
        app.peek = Some(PeekState::locations(
            PeekKind::Definition,
            vec![source_loc(&tmp.path().join("src/main.rs"), 1)], // 0-based -> new_line 2
        ));
        app.mode = Mode::Peek;

        peek_enter(&mut app);

        assert_eq!(app.mode, Mode::Normal);
        assert!(app.peek.is_none());
        let Row::Line(line) = &app.view.rows[app.view.cursor] else {
            panic!("expected cursor on a line row");
        };
        assert_eq!(line.new_line, Some(2));
    }

    #[test]
    fn peek_enter_on_unrelated_path_shows_not_in_diff_and_stays_open() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let mut app = App::new(vec![file_with_raw("src/main.rs", lsp_fixture_raw())]);
        app.set_repo_root(tmp.path().to_path_buf());
        app.peek = Some(PeekState::locations(
            PeekKind::Definition,
            vec![source_loc(&tmp.path().join("other.rs"), 0)],
        ));
        app.mode = Mode::Peek;

        peek_enter(&mut app);

        assert_eq!(app.mode, Mode::Peek);
        assert_eq!(app.status_message.as_deref(), Some("not in diff"));
    }

    #[test]
    fn peek_enter_is_a_noop_for_hover() {
        let mut app = App::new(vec![file("a.rs", 1)]);
        app.peek = Some(PeekState::hover("docs".to_string()));
        app.mode = Mode::Peek;
        peek_enter(&mut app);
        assert_eq!(app.mode, Mode::Peek);
        assert!(app.peek.is_some());
    }

    // -- Multibuffer LSP integration (task 4.3) -----------------------------

    /// A two-line hunk (`fn main() {` context, `old()`->`new()`) for `path`,
    /// whose added line has `new_line == 2` — the row `code_intel_position`
    /// accepts.
    fn added_line_raw(path: &str) -> String {
        format!(
            "diff --git a/{path} b/{path}\nindex 1..2 100644\n--- a/{path}\n+++ b/{path}\n@@ -1,2 +1,2 @@\n fn main() {{\n-    old();\n+    new();\n"
        )
    }

    #[test]
    fn code_intel_position_derives_path_from_the_cursor_row_owning_file() {
        // The cursor in the *second* section must issue its request against
        // the second file's path, not the first's.
        let tmp = tempfile::TempDir::new().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join("src")).expect("mkdir");
        std::fs::write(tmp.path().join("src/a.rs"), "fn main() {\n    new();\n}\n")
            .expect("write a");
        std::fs::write(tmp.path().join("src/b.rs"), "fn main() {\n    new();\n}\n")
            .expect("write b");

        let mut app = App::new(vec![
            file_with_raw("src/a.rs", &added_line_raw("src/a.rs")),
            file_with_raw("src/b.rs", &added_line_raw("src/b.rs")),
        ]);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let fake = FakeLsp {
            calls: Arc::clone(&calls),
            next_id: 0,
            deny: false,
            poll_queue: Arc::new(Mutex::new(std::collections::VecDeque::new())),
            shutdown_called: Arc::new(Mutex::new(false)),
        };
        app.inject_lsp_client(Box::new(fake), tmp.path().to_path_buf());

        app.apply(Action::NextFile); // cursor onto b.rs's section header
        for _ in 0..4 {
            app.apply(Action::CursorDown); // down to b.rs's added line
        }
        let Row::Line(l) = &app.view.rows[app.view.cursor] else {
            panic!("expected cursor on a line row");
        };
        assert_eq!(l.new_line, Some(2));

        app.apply(Action::GotoDefinition);
        assert_eq!(
            calls.lock().unwrap()[0],
            LspCall::Definition(tmp.path().join("src/b.rs"), 1, 0)
        );
    }

    #[test]
    fn peek_enter_expands_a_collapsed_target_section() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let mut app = App::new(vec![
            file("a.rs", 1),
            file_with_raw("src/main.rs", lsp_fixture_raw()),
        ]);
        app.set_repo_root(tmp.path().to_path_buf());
        // Collapse the target section; peek_enter must re-expand it, scroll
        // to it, and land the cursor on the target line within its span.
        app.view.set_collapsed("src/main.rs", true);
        app.rebuild_rows();
        assert!(app.view.is_collapsed("src/main.rs"));

        app.peek = Some(PeekState::locations(
            PeekKind::Definition,
            vec![source_loc(&tmp.path().join("src/main.rs"), 1)], // 0-based -> new_line 2
        ));
        app.mode = Mode::Peek;

        peek_enter(&mut app);

        assert!(!app.view.is_collapsed("src/main.rs")); // expanded on jump
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.view.file_of_cursor(), 1);
        let Row::Line(line) = &app.view.rows[app.view.cursor] else {
            panic!("expected cursor on a line row");
        };
        assert_eq!(line.new_line, Some(2));
    }
}
