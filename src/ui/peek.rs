//! State for the LSP peek overlay ([`super::app::Mode::Peek`]): the results
//! of a `gd`/`gr`/`K` request (a location list, or hover text), which
//! result is selected, and a per-path preview cache (file lines plus
//! syntax-highlight spans) so moving the selection within one open overlay
//! never re-reads or re-highlights a file it has already shown.

use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;

use crate::highlight::TokenKind;
use crate::lsp::SourceLocation;

/// Which of the three code-intelligence requests a [`PeekState`] is
/// displaying results for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeekKind {
    Definition,
    References,
    Hover,
}

/// One file's cached preview: its lines and per-line syntax-highlight
/// spans (empty spans if the language has no highlighter, or the read
/// failed — the preview then renders plain).
#[derive(Debug, Clone, Default)]
pub struct CachedPreview {
    pub lines: Vec<String>,
    pub spans: Vec<Vec<(Range<usize>, TokenKind)>>,
}

/// The peek overlay's state: either a location list (Definition/References)
/// or hover text, plus a per-path preview cache so re-selecting a location
/// already shown this session doesn't hit disk or the highlighter again.
pub struct PeekState {
    pub kind: PeekKind,
    /// Populated for Definition/References; empty for Hover.
    pub locations: Vec<SourceLocation>,
    /// The focused index into `locations`.
    pub selected: usize,
    /// The hover text; empty for Definition/References.
    pub hover_text: String,
    /// The hover body's scroll offset, in raw (unwrapped) lines.
    pub hover_scroll: usize,
    pub preview_cache: HashMap<PathBuf, CachedPreview>,
}

impl PeekState {
    /// A Definition/References overlay over `locations` (must be
    /// non-empty — callers should treat an empty result as "no results"
    /// rather than opening the overlay).
    pub fn locations(kind: PeekKind, locations: Vec<SourceLocation>) -> PeekState {
        PeekState {
            kind,
            locations,
            selected: 0,
            hover_text: String::new(),
            hover_scroll: 0,
            preview_cache: HashMap::new(),
        }
    }

    /// A Hover overlay over `text`.
    pub fn hover(text: String) -> PeekState {
        PeekState {
            kind: PeekKind::Hover,
            locations: Vec::new(),
            selected: 0,
            hover_text: text,
            hover_scroll: 0,
            preview_cache: HashMap::new(),
        }
    }

    /// The number of raw (unwrapped) lines in the hover text, for scroll
    /// clamping. At least 1, even for empty text.
    pub fn hover_line_count(&self) -> usize {
        self.hover_text.lines().count().max(1)
    }
}
