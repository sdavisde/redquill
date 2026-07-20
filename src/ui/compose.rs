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
    /// `Some(id)` when editing an existing item (submit updates it in place);
    /// `None` when composing a new one (submit adds). In reply mode
    /// ([`thread_id`](Self::thread_id) is `Some`) the id is the draft reply's
    /// id; otherwise it is the annotation's id.
    pub editing_id: Option<usize>,
    /// `Some(thread_root_id)` when this compose is drafting a reply to an
    /// imported PR thread rather than an annotation — `submit_compose`
    /// branches on it, and the modal renders a reply header instead of a
    /// target/classification title. `None` for the ordinary annotation
    /// compose (the vast majority of opens).
    pub thread_id: Option<u64>,
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
            thread_id: None,
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
            thread_id: None,
        }
    }

    /// Starts drafting a brand-new reply to the thread whose root comment id
    /// is `thread_id`. The `target`/`classification` are inert placeholders
    /// in reply mode — a reply answers a thread, not a diff anchor — so they
    /// carry harmless defaults the modal never renders.
    pub fn reply(thread_id: u64) -> ComposeState {
        ComposeState {
            target: Target::file(String::new()),
            classification: Classification::Issue,
            buffer: TextBuffer::new(),
            editing_id: None,
            thread_id: Some(thread_id),
        }
    }

    /// Starts editing an existing draft reply (`reply_id`) to thread
    /// `thread_id`, pre-filled with its body.
    pub fn editing_reply(reply_id: usize, thread_id: u64, body: &str) -> ComposeState {
        ComposeState {
            target: Target::file(String::new()),
            classification: Classification::Issue,
            buffer: TextBuffer::from_str(body),
            editing_id: Some(reply_id),
            thread_id: Some(thread_id),
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

    /// Moves the cursor left by one word. Within a line, lands on the start of
    /// the current or previous word (skipping any whitespace immediately to the
    /// left first, then the same-class run before it). At the start of a line
    /// (after the first) it wraps to the end of the previous line, mirroring
    /// [`move_left`](Self::move_left)'s boundary behavior.
    pub fn move_word_left(&mut self) {
        if self.cursor_col == 0 {
            if self.cursor_row > 0 {
                self.cursor_row -= 1;
                self.cursor_col = self.current_line_len();
            }
            return;
        }
        let chars: Vec<char> = self.lines[self.cursor_row].chars().collect();
        self.cursor_col = word_left_index(&chars, self.cursor_col);
    }

    /// Moves the cursor right by one word. Within a line, skips the same-class
    /// run under the cursor then any trailing whitespace, landing on the next
    /// word's start (or the line end). At the end of a line it wraps to the
    /// start of the next line, mirroring [`move_right`](Self::move_right).
    pub fn move_word_right(&mut self) {
        let len = self.current_line_len();
        if self.cursor_col >= len {
            if self.cursor_row + 1 < self.lines.len() {
                self.cursor_row += 1;
                self.cursor_col = 0;
            }
            return;
        }
        let chars: Vec<char> = self.lines[self.cursor_row].chars().collect();
        self.cursor_col = word_right_index(&chars, self.cursor_col);
    }

    /// Moves the cursor to the start of the current line.
    pub fn move_line_start(&mut self) {
        self.cursor_col = 0;
    }

    /// Moves the cursor to the end of the current line.
    pub fn move_line_end(&mut self) {
        self.cursor_col = self.current_line_len();
    }

    /// Moves the cursor to the very start of the buffer (first line, column 0).
    pub fn move_doc_start(&mut self) {
        self.cursor_row = 0;
        self.cursor_col = 0;
    }

    /// Moves the cursor to the very end of the buffer (last line, past its last
    /// char).
    pub fn move_doc_end(&mut self) {
        self.cursor_row = self.lines.len() - 1;
        self.cursor_col = self.current_line_len();
    }

    /// Deletes the character *at* the cursor, or — at the end of a line before
    /// the last — merges the next line into this one. The mirror image of
    /// [`backspace`](Self::backspace); the cursor never moves.
    pub fn delete_forward(&mut self) {
        let len = self.current_line_len();
        if self.cursor_col < len {
            let byte_idx = char_byte_index(&self.lines[self.cursor_row], self.cursor_col);
            self.lines[self.cursor_row].remove(byte_idx);
        } else if self.cursor_row + 1 < self.lines.len() {
            let next = self.lines.remove(self.cursor_row + 1);
            self.lines[self.cursor_row].push_str(&next);
        }
    }

    /// Deletes from the previous word boundary up to the cursor. At the start
    /// of a line this degrades to a [`backspace`](Self::backspace) (merging
    /// into the previous line).
    pub fn delete_word_back(&mut self) {
        if self.cursor_col == 0 {
            self.backspace();
            return;
        }
        let chars: Vec<char> = self.lines[self.cursor_row].chars().collect();
        let target = word_left_index(&chars, self.cursor_col);
        let start_byte = char_byte_index(&self.lines[self.cursor_row], target);
        let end_byte = char_byte_index(&self.lines[self.cursor_row], self.cursor_col);
        self.lines[self.cursor_row].replace_range(start_byte..end_byte, "");
        self.cursor_col = target;
    }

    /// Deletes from the cursor up to the next word boundary. At the end of a
    /// line this merges the next line into this one (mirror of
    /// [`delete_word_back`](Self::delete_word_back) at a line start). The cursor
    /// never moves.
    pub fn delete_word_forward(&mut self) {
        let len = self.current_line_len();
        if self.cursor_col >= len {
            if self.cursor_row + 1 < self.lines.len() {
                let next = self.lines.remove(self.cursor_row + 1);
                self.lines[self.cursor_row].push_str(&next);
            }
            return;
        }
        let chars: Vec<char> = self.lines[self.cursor_row].chars().collect();
        let target = word_right_index(&chars, self.cursor_col);
        let start_byte = char_byte_index(&self.lines[self.cursor_row], self.cursor_col);
        let end_byte = char_byte_index(&self.lines[self.cursor_row], target);
        self.lines[self.cursor_row].replace_range(start_byte..end_byte, "");
    }
}

/// The three character classes the word-motion boundary model recognizes:
/// runs of the same class are one "word" for the purposes of `Ctrl`/`Alt`
/// word motions and word-wise deletion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharClass {
    Whitespace,
    /// Identifier-ish: `char::is_alphanumeric` plus `_`.
    Word,
    /// Anything else printable (punctuation, symbols).
    Punct,
}

fn char_class(c: char) -> CharClass {
    if c.is_whitespace() {
        CharClass::Whitespace
    } else if c.is_alphanumeric() || c == '_' {
        CharClass::Word
    } else {
        CharClass::Punct
    }
}

/// Char index of the next word boundary at or after `col` within `chars`:
/// skips the same-class run under the cursor, then any trailing whitespace, so
/// the result sits on the *start* of the following word (or `chars.len()`).
fn word_right_index(chars: &[char], col: usize) -> usize {
    let mut i = col;
    if i >= chars.len() {
        return chars.len();
    }
    let start = char_class(chars[i]);
    while i < chars.len() && char_class(chars[i]) == start {
        i += 1;
    }
    while i < chars.len() && char_class(chars[i]) == CharClass::Whitespace {
        i += 1;
    }
    i
}

/// Char index of the previous word boundary before `col` within `chars`:
/// skips whitespace immediately to the left, then the same-class run before it,
/// so the result sits on the *start* of the current/previous word (or `0`).
fn word_left_index(chars: &[char], col: usize) -> usize {
    let mut i = col.min(chars.len());
    while i > 0 && char_class(chars[i - 1]) == CharClass::Whitespace {
        i -= 1;
    }
    if i > 0 {
        let cls = char_class(chars[i - 1]);
        while i > 0 && char_class(chars[i - 1]) == cls {
            i -= 1;
        }
    }
    i
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

    // -- Word / line / document motions ------------------------------------

    #[test]
    fn move_word_right_lands_on_next_word_start() {
        let mut buf = TextBuffer::from_str("foo bar baz");
        buf.cursor_col = 0;
        buf.move_word_right();
        assert_eq!(buf.cursor_col, 4); // start of "bar"
        buf.move_word_right();
        assert_eq!(buf.cursor_col, 8); // start of "baz"
        buf.move_word_right();
        assert_eq!(buf.cursor_col, 11); // end of line (no trailing word)
    }

    #[test]
    fn move_word_right_from_mid_word_skips_rest_of_word_then_whitespace() {
        let mut buf = TextBuffer::from_str("foo bar");
        buf.cursor_col = 1; // inside "foo"
        buf.move_word_right();
        assert_eq!(buf.cursor_col, 4); // start of "bar"
    }

    #[test]
    fn move_word_right_treats_punctuation_as_its_own_class() {
        let mut buf = TextBuffer::from_str("a.b c");
        buf.cursor_col = 0; // on "a"
        buf.move_word_right();
        assert_eq!(buf.cursor_col, 1); // on "."
        buf.move_word_right();
        assert_eq!(buf.cursor_col, 2); // on "b"
        buf.move_word_right();
        assert_eq!(buf.cursor_col, 4); // start of "c"
    }

    #[test]
    fn move_word_right_at_line_end_wraps_to_next_line_start() {
        let mut buf = TextBuffer::from_str("ab\ncd");
        buf.cursor_row = 0;
        buf.cursor_col = 2;
        buf.move_word_right();
        assert_eq!(buf.cursor_row, 1);
        assert_eq!(buf.cursor_col, 0);
    }

    #[test]
    fn move_word_left_lands_on_current_or_previous_word_start() {
        let mut buf = TextBuffer::from_str("foo bar baz");
        buf.cursor_col = 11; // end of line
        buf.move_word_left();
        assert_eq!(buf.cursor_col, 8); // start of "baz"
        buf.move_word_left();
        assert_eq!(buf.cursor_col, 4); // start of "bar"
        buf.move_word_left();
        assert_eq!(buf.cursor_col, 0); // start of "foo"
    }

    #[test]
    fn move_word_left_from_mid_word_lands_on_that_words_start() {
        let mut buf = TextBuffer::from_str("foo bar");
        buf.cursor_col = 6; // inside "bar"
        buf.move_word_left();
        assert_eq!(buf.cursor_col, 4); // start of "bar"
    }

    #[test]
    fn move_word_left_at_line_start_wraps_to_previous_line_end() {
        let mut buf = TextBuffer::from_str("ab\ncd");
        buf.cursor_row = 1;
        buf.cursor_col = 0;
        buf.move_word_left();
        assert_eq!(buf.cursor_row, 0);
        assert_eq!(buf.cursor_col, 2);
    }

    #[test]
    fn word_motions_are_char_index_safe_with_multibyte_content() {
        // "héllo wörld" — accents keep the columns as char indices, not bytes.
        let mut buf = TextBuffer::from_str("héllo wörld");
        buf.cursor_col = 0;
        buf.move_word_right();
        assert_eq!(buf.cursor_col, 6); // start of "wörld" (after "héllo ")
        buf.move_word_left();
        assert_eq!(buf.cursor_col, 0); // back to start of "héllo"
    }

    #[test]
    fn delete_word_back_multibyte_removes_the_word_not_bytes() {
        let mut buf = TextBuffer::from_str("héllo wörld");
        buf.cursor_col = 11; // end of line
        buf.delete_word_back();
        assert_eq!(buf.lines[0], "héllo ");
        assert_eq!(buf.cursor_col, 6);
    }

    #[test]
    fn move_line_start_and_end() {
        let mut buf = TextBuffer::from_str("hello");
        buf.cursor_col = 3;
        buf.move_line_start();
        assert_eq!(buf.cursor_col, 0);
        buf.move_line_end();
        assert_eq!(buf.cursor_col, 5);
    }

    #[test]
    fn move_doc_start_and_end() {
        let mut buf = TextBuffer::from_str("one\ntwo\nthree");
        buf.cursor_row = 1;
        buf.cursor_col = 2;
        buf.move_doc_start();
        assert_eq!((buf.cursor_row, buf.cursor_col), (0, 0));
        buf.move_doc_end();
        assert_eq!((buf.cursor_row, buf.cursor_col), (2, 5)); // end of "three"
    }

    #[test]
    fn delete_forward_removes_char_at_cursor() {
        let mut buf = TextBuffer::from_str("abc");
        buf.cursor_col = 1;
        buf.delete_forward();
        assert_eq!(buf.lines[0], "ac");
        assert_eq!(buf.cursor_col, 1); // cursor doesn't move
    }

    #[test]
    fn delete_forward_at_line_end_merges_next_line() {
        let mut buf = TextBuffer::from_str("foo\nbar");
        buf.cursor_row = 0;
        buf.cursor_col = 3; // end of "foo"
        buf.delete_forward();
        assert_eq!(buf.lines, vec!["foobar".to_string()]);
        assert_eq!((buf.cursor_row, buf.cursor_col), (0, 3));
    }

    #[test]
    fn delete_forward_at_buffer_end_is_a_no_op() {
        let mut buf = TextBuffer::from_str("ab");
        buf.cursor_col = 2;
        buf.delete_forward();
        assert_eq!(buf.lines, vec!["ab".to_string()]);
    }

    #[test]
    fn delete_word_back_removes_previous_word() {
        let mut buf = TextBuffer::from_str("foo bar");
        buf.cursor_col = 7; // end
        buf.delete_word_back();
        assert_eq!(buf.lines[0], "foo ");
        assert_eq!(buf.cursor_col, 4);
    }

    #[test]
    fn delete_word_back_at_line_start_merges_like_backspace() {
        let mut buf = TextBuffer::from_str("foo\nbar");
        buf.cursor_row = 1;
        buf.cursor_col = 0;
        buf.delete_word_back();
        assert_eq!(buf.lines, vec!["foobar".to_string()]);
        assert_eq!((buf.cursor_row, buf.cursor_col), (0, 3));
    }

    #[test]
    fn delete_word_forward_removes_next_word() {
        let mut buf = TextBuffer::from_str("foo bar");
        buf.cursor_col = 0;
        buf.delete_word_forward();
        assert_eq!(buf.lines[0], "bar"); // "foo " removed
        assert_eq!(buf.cursor_col, 0);
    }

    #[test]
    fn delete_word_forward_at_line_end_merges_next_line() {
        let mut buf = TextBuffer::from_str("foo\nbar");
        buf.cursor_row = 0;
        buf.cursor_col = 3;
        buf.delete_word_forward();
        assert_eq!(buf.lines, vec!["foobar".to_string()]);
        assert_eq!((buf.cursor_row, buf.cursor_col), (0, 3));
    }

    #[test]
    fn word_index_helpers_handle_empty_and_boundaries() {
        assert_eq!(word_right_index(&[], 0), 0);
        assert_eq!(word_left_index(&[], 0), 0);
        let chars: Vec<char> = "ab".chars().collect();
        assert_eq!(word_right_index(&chars, 2), 2); // already at end
        assert_eq!(word_left_index(&chars, 0), 0); // already at start
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
