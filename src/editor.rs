//! A single-line text editor used for inline editing, search and tag prompts.

use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Default)]
pub struct LineEditor {
    pub buf: String,
    /// Byte offset of the cursor; always on a char boundary.
    pub cursor: usize,
}

impl LineEditor {
    pub fn new(text: &str, cursor_at_end: bool) -> Self {
        LineEditor {
            buf: text.to_string(),
            cursor: if cursor_at_end { text.len() } else { 0 },
        }
    }

    pub fn insert(&mut self, c: char) {
        self.buf.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    pub fn insert_str(&mut self, s: &str) {
        self.buf.insert_str(self.cursor, s);
        self.cursor += s.len();
    }

    fn prev_boundary(&self) -> usize {
        self.buf[..self.cursor]
            .chars()
            .next_back()
            .map_or(0, |c| self.cursor - c.len_utf8())
    }

    fn next_boundary(&self) -> usize {
        self.buf[self.cursor..]
            .chars()
            .next()
            .map_or(self.buf.len(), |c| self.cursor + c.len_utf8())
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            let p = self.prev_boundary();
            self.buf.replace_range(p..self.cursor, "");
            self.cursor = p;
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.buf.len() {
            let n = self.next_boundary();
            self.buf.replace_range(self.cursor..n, "");
        }
    }

    pub fn left(&mut self) {
        self.cursor = self.prev_boundary();
    }

    pub fn right(&mut self) {
        self.cursor = self.next_boundary();
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.buf.len();
    }

    fn word_start_before(&self) -> usize {
        let before = &self.buf[..self.cursor];
        let trimmed = before.trim_end();
        let ws = trimmed.rfind(char::is_whitespace).map_or(0, |i| i + 1);
        ws.min(trimmed.len())
    }

    pub fn word_left(&mut self) {
        self.cursor = self.word_start_before();
    }

    pub fn word_right(&mut self) {
        let after = &self.buf[self.cursor..];
        let skip_word = after.find(char::is_whitespace).unwrap_or(after.len());
        let rest = &after[skip_word..];
        let skip_ws = rest.len() - rest.trim_start().len();
        self.cursor += skip_word + skip_ws;
    }

    pub fn delete_word_back(&mut self) {
        let start = self.word_start_before();
        self.buf.replace_range(start..self.cursor, "");
        self.cursor = start;
    }

    pub fn kill_to_end(&mut self) {
        self.buf.truncate(self.cursor);
    }

    pub fn kill_to_start(&mut self) {
        self.buf.replace_range(..self.cursor, "");
        self.cursor = 0;
    }

    /// Display column of the cursor.
    pub fn cursor_col(&self) -> usize {
        self.buf[..self.cursor].width()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_respect_unicode_boundaries() {
        let mut e = LineEditor::new("a☐b", true);
        e.left();
        e.left();
        assert_eq!(e.cursor, 1);
        e.delete();
        assert_eq!(e.buf, "ab");
        e.insert('é');
        assert_eq!(e.buf, "aéb");
        e.backspace();
        assert_eq!(e.buf, "ab");
        assert_eq!(e.cursor, 1);
    }

    #[test]
    fn word_motions() {
        let mut e = LineEditor::new("buy some milk @today", true);
        e.word_left();
        assert_eq!(&e.buf[e.cursor..], "@today");
        e.delete_word_back();
        assert_eq!(e.buf, "buy some @today");
        e.home();
        e.word_right();
        assert_eq!(&e.buf[e.cursor..], "some @today");
        e.kill_to_end();
        assert_eq!(e.buf, "buy ");
    }
}
