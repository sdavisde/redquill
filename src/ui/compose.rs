//! The Compose modal's state: what's being annotated ([`ComposeState`]) and
//! the hand-rolled multi-line text buffer it edits ([`TextBuffer`]). No
//! external textarea dependency — insert, newline, backspace, and arrow-key
//! movement are all implemented here and unit-tested directly.

use crate::annotate::{Classification, Target};

/// The Compose modal's state while open: the target being annotated, the
/// currently selected classification (`Ctrl-t` cycles it), the text being
/// edited, and — when editing an existing annotation rather than creating a
/// new one — the id to write back to on submit.
#[derive(Debug, Clone, PartialEq)]
pub struct ComposeState {
    /// What the annotation being composed is attached to.
    pub target: Target,
    /// The annotation's current classification.
    pub classification: Classification,
    /// The body text being edited.
    pub buffer: TextBuffer,
    /// `Some(id)` when editing an existing annotation (submit calls
    /// `edit`/`set_classification`); `None` when composing a new one
    /// (submit calls `add`).
    pub editing_id: Option<usize>,
}

impl ComposeState {
    /// Starts composing a brand-new annotation on `target`, with an empty
    /// buffer and `Classification::Issue` as the default.
    pub fn new(target: Target) -> ComposeState {
        ComposeState {
            target,
            classification: Classification::Issue,
            buffer: TextBuffer::new(),
            editing_id: None,
        }
    }

    /// Starts composing over an existing annotation's target, body, and
    /// classification, pre-filled for editing.
    pub fn editing(
        id: usize,
        target: Target,
        classification: Classification,
        body: &str,
    ) -> ComposeState {
        ComposeState {
            target,
            classification,
            buffer: TextBuffer::from_str(body),
            editing_id: Some(id),
        }
    }
}

/// A minimal multi-line text buffer with cursor position. Lines are stored
/// as a `Vec<String>` (always at least one, possibly empty); the cursor is
/// a `(row, col)` pair, `col` counted in `char`s (not bytes) so movement and
/// editing stay correct on multi-byte UTF-8 content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextBuffer {
    /// The buffer's lines. Always has at least one (possibly empty) line.
    pub lines: Vec<String>,
    /// 0-based row the cursor is on.
    pub cursor_row: usize,
    /// 0-based char column the cursor is on, within `lines[cursor_row]`.
    pub cursor_col: usize,
}

impl Default for TextBuffer {
    fn default() -> TextBuffer {
        TextBuffer::new()
    }
}

impl TextBuffer {
    /// An empty buffer: one empty line, cursor at the origin.
    pub fn new() -> TextBuffer {
        TextBuffer {
            lines: vec![String::new()],
            cursor_row: 0,
            cursor_col: 0,
        }
    }

    /// Builds a buffer pre-filled with `s`'s content, cursor placed at the
    /// end (matching where a reviewer resumes editing a pre-filled body).
    pub fn from_str(s: &str) -> TextBuffer {
        let lines: Vec<String> = if s.is_empty() {
            vec![String::new()]
        } else {
            s.lines().map(str::to_string).collect()
        };
        let cursor_row = lines.len() - 1;
        let cursor_col = lines[cursor_row].chars().count();
        TextBuffer {
            lines,
            cursor_row,
            cursor_col,
        }
    }

    /// Joins the buffer's lines into a single `\n`-separated string.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    fn current_line_len(&self) -> usize {
        self.lines[self.cursor_row].chars().count()
    }

    /// Inserts `c` at the cursor and advances the cursor past it.
    pub fn insert_char(&mut self, c: char) {
        let byte_idx = char_byte_index(&self.lines[self.cursor_row], self.cursor_col);
        self.lines[self.cursor_row].insert(byte_idx, c);
        self.cursor_col += 1;
    }

    /// Splits the current line at the cursor into two lines, moving the
    /// cursor to the start of the new (second) line.
    pub fn newline(&mut self) {
        let byte_idx = char_byte_index(&self.lines[self.cursor_row], self.cursor_col);
        let rest = self.lines[self.cursor_row].split_off(byte_idx);
        self.lines.insert(self.cursor_row + 1, rest);
        self.cursor_row += 1;
        self.cursor_col = 0;
    }

    /// Deletes the character before the cursor, or — at the start of a line
    /// after the first — merges this line into the previous one.
    pub fn backspace(&mut self) {
        if self.cursor_col > 0 {
            let byte_idx = char_byte_index(&self.lines[self.cursor_row], self.cursor_col - 1);
            self.lines[self.cursor_row].remove(byte_idx);
            self.cursor_col -= 1;
        } else if self.cursor_row > 0 {
            let removed = self.lines.remove(self.cursor_row);
            self.cursor_row -= 1;
            self.cursor_col = self.current_line_len();
            self.lines[self.cursor_row].push_str(&removed);
        }
    }

    /// Moves left one char, wrapping to the end of the previous line at the
    /// start of a line.
    pub fn move_left(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        } else if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.cursor_col = self.current_line_len();
        }
    }

    /// Moves right one char, wrapping to the start of the next line at the
    /// end of a line.
    pub fn move_right(&mut self) {
        if self.cursor_col < self.current_line_len() {
            self.cursor_col += 1;
        } else if self.cursor_row + 1 < self.lines.len() {
            self.cursor_row += 1;
            self.cursor_col = 0;
        }
    }

    /// Moves up one row, clamping the column to the target line's length.
    pub fn move_up(&mut self) {
        if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.cursor_col = self.cursor_col.min(self.current_line_len());
        }
    }

    /// Moves down one row, clamping the column to the target line's length.
    pub fn move_down(&mut self) {
        if self.cursor_row + 1 < self.lines.len() {
            self.cursor_row += 1;
            self.cursor_col = self.cursor_col.min(self.current_line_len());
        }
    }
}

/// The byte index in `s` of char index `char_idx` (or `s.len()` if
/// `char_idx` is at or past the end) — lets [`TextBuffer`] index/mutate
/// `String`s by char position without ever slicing mid-codepoint.
fn char_byte_index(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_buffer_is_one_empty_line_at_origin() {
        let buf = TextBuffer::new();
        assert_eq!(buf.lines, vec![String::new()]);
        assert_eq!(buf.cursor_row, 0);
        assert_eq!(buf.cursor_col, 0);
        assert_eq!(buf.text(), "");
    }

    #[test]
    fn insert_char_advances_cursor() {
        let mut buf = TextBuffer::new();
        buf.insert_char('h');
        buf.insert_char('i');
        assert_eq!(buf.lines[0], "hi");
        assert_eq!(buf.cursor_col, 2);
    }

    #[test]
    fn insert_char_inserts_at_cursor_not_always_at_end() {
        let mut buf = TextBuffer::from_str("ac");
        buf.cursor_col = 1;
        buf.insert_char('b');
        assert_eq!(buf.lines[0], "abc");
        assert_eq!(buf.cursor_col, 2);
    }

    #[test]
    fn newline_splits_line_at_cursor() {
        let mut buf = TextBuffer::from_str("hello world");
        buf.cursor_col = 5;
        buf.newline();
        assert_eq!(buf.lines, vec!["hello".to_string(), " world".to_string()]);
        assert_eq!(buf.cursor_row, 1);
        assert_eq!(buf.cursor_col, 0);
        assert_eq!(buf.text(), "hello\n world");
    }

    #[test]
    fn backspace_deletes_char_before_cursor() {
        let mut buf = TextBuffer::from_str("abc");
        buf.backspace();
        assert_eq!(buf.lines[0], "ab");
        assert_eq!(buf.cursor_col, 2);
    }

    #[test]
    fn backspace_at_start_of_line_merges_with_previous() {
        let mut buf = TextBuffer::from_str("foo\nbar");
        buf.cursor_row = 1;
        buf.cursor_col = 0;
        buf.backspace();
        assert_eq!(buf.lines, vec!["foobar".to_string()]);
        assert_eq!(buf.cursor_row, 0);
        assert_eq!(buf.cursor_col, 3);
    }

    #[test]
    fn backspace_at_origin_is_a_no_op() {
        let mut buf = TextBuffer::new();
        buf.backspace();
        assert_eq!(buf.lines, vec![String::new()]);
        assert_eq!(buf.cursor_row, 0);
        assert_eq!(buf.cursor_col, 0);
    }

    #[test]
    fn backspace_across_lines_then_insert_joins_correctly() {
        let mut buf = TextBuffer::from_str("one\ntwo");
        buf.cursor_row = 1;
        buf.cursor_col = 0;
        buf.backspace();
        buf.insert_char('X');
        assert_eq!(buf.lines, vec!["oneXtwo".to_string()]);
    }

    #[test]
    fn move_left_and_right_within_a_line() {
        let mut buf = TextBuffer::from_str("abc");
        buf.move_left();
        assert_eq!(buf.cursor_col, 2);
        buf.move_left();
        buf.move_left();
        assert_eq!(buf.cursor_col, 0);
        // Clamped at start.
        buf.move_left();
        assert_eq!(buf.cursor_col, 0);
        buf.move_right();
        assert_eq!(buf.cursor_col, 1);
    }

    #[test]
    fn move_left_wraps_to_end_of_previous_line() {
        let mut buf = TextBuffer::from_str("ab\ncd");
        buf.cursor_row = 1;
        buf.cursor_col = 0;
        buf.move_left();
        assert_eq!(buf.cursor_row, 0);
        assert_eq!(buf.cursor_col, 2);
    }

    #[test]
    fn move_right_wraps_to_start_of_next_line() {
        let mut buf = TextBuffer::from_str("ab\ncd");
        buf.cursor_row = 0;
        buf.cursor_col = 2;
        buf.move_right();
        assert_eq!(buf.cursor_row, 1);
        assert_eq!(buf.cursor_col, 0);
    }

    #[test]
    fn move_right_at_last_line_end_is_a_no_op() {
        let mut buf = TextBuffer::from_str("ab");
        buf.cursor_col = 2;
        buf.move_right();
        assert_eq!(buf.cursor_row, 0);
        assert_eq!(buf.cursor_col, 2);
    }

    #[test]
    fn move_up_and_down_clamp_column_to_shorter_line() {
        let mut buf = TextBuffer::from_str("longline\nhi");
        buf.cursor_row = 0;
        buf.cursor_col = 8;
        buf.move_down();
        assert_eq!(buf.cursor_row, 1);
        assert_eq!(buf.cursor_col, 2); // clamped to "hi".len()
        buf.move_up();
        assert_eq!(buf.cursor_row, 0);
        assert_eq!(buf.cursor_col, 2);
    }

    #[test]
    fn move_up_at_first_row_is_a_no_op() {
        let mut buf = TextBuffer::from_str("a\nb");
        buf.move_up();
        assert_eq!(buf.cursor_row, 0);
    }

    #[test]
    fn move_down_at_last_row_is_a_no_op() {
        let mut buf = TextBuffer::from_str("a\nb");
        buf.cursor_row = 1;
        buf.move_down();
        assert_eq!(buf.cursor_row, 1);
    }

    #[test]
    fn from_str_places_cursor_at_end() {
        let buf = TextBuffer::from_str("one\ntwo\nthree");
        assert_eq!(buf.cursor_row, 2);
        assert_eq!(buf.cursor_col, 5);
    }

    #[test]
    fn from_str_empty_is_one_empty_line() {
        let buf = TextBuffer::from_str("");
        assert_eq!(buf.lines, vec![String::new()]);
        assert_eq!(buf.cursor_row, 0);
        assert_eq!(buf.cursor_col, 0);
    }

    #[test]
    fn insert_char_is_char_index_safe_with_multibyte_content() {
        let mut buf = TextBuffer::from_str("héllo");
        buf.cursor_col = 2; // after "hé"
        buf.insert_char('!');
        assert_eq!(buf.lines[0], "hé!llo");
    }

    #[test]
    fn compose_state_new_defaults_to_issue_and_empty_buffer() {
        let target = Target::file("a.rs");
        let compose = ComposeState::new(target.clone());
        assert_eq!(compose.target, target);
        assert_eq!(compose.classification, Classification::Issue);
        assert_eq!(compose.buffer.text(), "");
        assert_eq!(compose.editing_id, None);
    }

    #[test]
    fn compose_state_editing_prefills_body_and_classification() {
        let target = Target::file("a.rs");
        let compose = ComposeState::editing(3, target.clone(), Classification::Nit, "hello");
        assert_eq!(compose.target, target);
        assert_eq!(compose.classification, Classification::Nit);
        assert_eq!(compose.buffer.text(), "hello");
        assert_eq!(compose.editing_id, Some(3));
    }
}
