use unicode_width::UnicodeWidthStr;

/// A single-line text box. `cursor` is a byte index that always sits on a
/// character boundary.
#[derive(Debug, Default)]
pub struct Input {
    text: String,
    cursor: usize,
}

impl Input {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Where the cursor is, in terminal columns.
    pub fn cursor_column(&self) -> usize {
        self.text[..self.cursor].width()
    }

    pub fn insert(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    pub fn backspace(&mut self) {
        if let Some(c) = self.text[..self.cursor].chars().next_back() {
            self.cursor -= c.len_utf8();
            self.text.remove(self.cursor);
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.text.len() {
            self.text.remove(self.cursor);
        }
    }

    pub fn left(&mut self) {
        if let Some(c) = self.text[..self.cursor].chars().next_back() {
            self.cursor -= c.len_utf8();
        }
    }

    pub fn right(&mut self) {
        if let Some(c) = self.text[self.cursor..].chars().next() {
            self.cursor += c.len_utf8();
        }
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.text.len();
    }

    /// Ctrl+W: delete the word before the cursor (and the spaces after it).
    pub fn delete_word(&mut self) {
        let before = &self.text[..self.cursor];
        let trimmed = before.trim_end();
        let start = trimmed.rfind(char::is_whitespace).map_or(0, |i| i + 1);
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
    }

    pub fn set(&mut self, text: String) {
        self.cursor = text.len();
        self.text = text;
    }

    /// Empties the box and returns what was in it.
    pub fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing_with_multibyte_characters() {
        let mut input = Input::default();
        for c in "héllo 🎉".chars() {
            input.insert(c);
        }
        input.left();
        input.backspace(); // removes the space before 🎉
        assert_eq!(input.text(), "héllo🎉");
        input.home();
        input.right();
        input.delete(); // removes é
        assert_eq!(input.text(), "hllo🎉");
        assert_eq!(input.cursor_column(), 1);
    }

    #[test]
    fn delete_word_removes_the_previous_word() {
        let mut input = Input::default();
        input.set("hello big world  ".into());
        input.delete_word();
        assert_eq!(input.text(), "hello big ");
        input.delete_word();
        assert_eq!(input.text(), "hello ");
    }

    #[test]
    fn take_clears_the_box() {
        let mut input = Input::default();
        input.set("hi".into());
        assert_eq!(input.take(), "hi");
        assert!(input.is_empty());
        input.insert('x'); // cursor was reset, so this doesn't panic
        assert_eq!(input.text(), "x");
    }
}
