//! A single line within a hunk and which side of the diff it belongs to.

/// Which side of a diff a line belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineOrigin {
    /// Present, unchanged, on both sides.
    Context,
    /// Present only on the new side.
    Added,
    /// Present only on the old side.
    Removed,
}

/// One line of a hunk's body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    /// Which side this line belongs to.
    pub origin: LineOrigin,
    /// 1-based line number on the old side, or `None` if the line doesn't
    /// exist there (added lines).
    pub old_line: Option<u32>,
    /// 1-based line number on the new side, or `None` if the line doesn't
    /// exist there (removed lines).
    pub new_line: Option<u32>,
    /// The line's text, without the leading `+`/`-`/` ` marker and without
    /// the trailing newline.
    pub content: String,
    /// Whether this line is immediately followed by a
    /// `\ No newline at end of file` marker in the raw patch, i.e. the file
    /// does not end with a trailing newline on this line's side.
    pub no_newline: bool,
}
