//! A file's text as lines, with a cursor and undo, for editing YAML in place.

use std::ops::Range;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub struct TextEdit {
    lines: Vec<String>,
    /// The cursor: a line, and a character index in it.
    pub row: usize,
    pub col: usize,
    /// Earlier lines and cursors, newest last.
    undo: Vec<(Vec<String>, usize, usize)>,
}

impl TextEdit {
    pub fn new(text: &str) -> TextEdit {
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
        if lines.is_empty() {
            lines.push(String::new());
        }
        TextEdit {
            lines,
            row: 0,
            col: 0,
            undo: Vec::new(),
        }
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// The text, each line ending in a newline; empty when there is nothing.
    pub fn text(&self) -> String {
        if self.lines.len() == 1 && self.lines[0].is_empty() {
            return String::new();
        }
        self.lines.iter().map(|line| format!("{line}\n")).collect()
    }

    /// Saves the current text, so `undo` comes back to it.
    pub fn checkpoint(&mut self) {
        self.undo.push((self.lines.clone(), self.row, self.col));
    }

    /// Goes back to the last checkpoint; false when there is none.
    pub fn undo(&mut self) -> bool {
        let Some((lines, row, col)) = self.undo.pop() else {
            return false;
        };
        self.lines = lines;
        self.place(row, col);
        true
    }

    /// Replaces the text, after a checkpoint.
    pub fn set_text(&mut self, text: &str) {
        let lines = TextEdit::new(text).lines;
        self.set_lines(lines);
    }

    /// Replaces the lines, after a checkpoint.
    pub fn set_lines(&mut self, lines: Vec<String>) {
        self.checkpoint();
        self.lines = if lines.is_empty() {
            vec![String::new()]
        } else {
            lines
        };
        self.place(self.row, self.col);
    }

    /// Moves the cursor, kept inside the text.
    pub fn place(&mut self, row: usize, col: usize) {
        self.row = row.min(self.lines.len() - 1);
        self.col = col.min(self.line_len(self.row));
    }

    /// Edits or moves the cursor for `key`, keeping it on the lines in `rows`. Returns
    /// whether the text changed.
    pub fn key(&mut self, key: KeyEvent, rows: Range<usize>) -> bool {
        let first = rows.start.min(self.lines.len() - 1);
        let last = rows.end.clamp(first + 1, self.lines.len()) - 1;
        match key.code {
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.insert(&c.to_string());
            }
            KeyCode::Tab => self.insert("  "),
            KeyCode::Enter => {
                let line = &mut self.lines[self.row];
                let rest = line.split_off(byte(line, self.col));
                let indent: String = line.chars().take_while(|c| *c == ' ').collect();
                self.col = indent.chars().count();
                self.row += 1;
                self.lines.insert(self.row, format!("{indent}{rest}"));
            }
            KeyCode::Backspace if self.col > 0 => {
                let line = &mut self.lines[self.row];
                line.remove(byte(line, self.col - 1));
                self.col -= 1;
            }
            KeyCode::Backspace if self.row > first => {
                let line = self.lines.remove(self.row);
                self.row -= 1;
                self.col = self.line_len(self.row);
                self.lines[self.row].push_str(&line);
            }
            KeyCode::Delete if self.col < self.line_len(self.row) => {
                let line = &mut self.lines[self.row];
                line.remove(byte(line, self.col));
            }
            KeyCode::Delete if self.row < last => {
                let next = self.lines.remove(self.row + 1);
                self.lines[self.row].push_str(&next);
            }
            code => {
                self.go(code, first, last);
                return false;
            }
        }
        // `Backspace` and `Delete` with nothing to remove fell through to `go` above.
        true
    }

    /// Moves the cursor for `code` between lines `first` and `last`.
    fn go(&mut self, code: KeyCode, first: usize, last: usize) {
        match code {
            KeyCode::Left => self.col = self.col.saturating_sub(1),
            KeyCode::Right => self.col = (self.col + 1).min(self.line_len(self.row)),
            KeyCode::Up if self.row > first => self.place(self.row - 1, self.col),
            KeyCode::Down if self.row < last => self.place(self.row + 1, self.col),
            KeyCode::Home => self.col = 0,
            KeyCode::End => self.col = self.line_len(self.row),
            _ => {}
        }
    }

    fn insert(&mut self, text: &str) {
        let line = &mut self.lines[self.row];
        line.insert_str(byte(line, self.col), text);
        self.col += text.chars().count();
    }

    fn line_len(&self, row: usize) -> usize {
        self.lines[row].chars().count()
    }
}

/// The byte index of character `col` in `line`.
fn byte(line: &str, col: usize) -> usize {
    line.char_indices()
        .nth(col)
        .map_or(line.len(), |(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(edit: &mut TextEdit, code: KeyCode) -> bool {
        let rows = 0..edit.lines().len();
        edit.key(KeyEvent::new(code, KeyModifiers::NONE), rows)
    }

    #[test]
    fn types_and_breaks_lines_keeping_indentation() {
        let mut edit = TextEdit::new("a:\n  b: 1\n");
        edit.place(1, 6);
        assert!(press(&mut edit, KeyCode::Char('2')));
        assert!(press(&mut edit, KeyCode::Enter));
        press(&mut edit, KeyCode::Char('c'));
        assert_eq!(edit.text(), "a:\n  b: 12\n  c\n");
        assert_eq!((edit.row, edit.col), (2, 3));
    }

    #[test]
    fn backspace_joins_lines_and_undo_restores() {
        let mut edit = TextEdit::new("ab\ncd\n");
        edit.checkpoint();
        edit.place(1, 0);
        assert!(press(&mut edit, KeyCode::Backspace));
        assert_eq!(edit.text(), "abcd\n");
        assert_eq!((edit.row, edit.col), (0, 2));
        assert!(edit.undo());
        assert_eq!(edit.text(), "ab\ncd\n");
        assert!(!edit.undo());
    }

    #[test]
    fn stays_inside_its_rows() {
        let mut edit = TextEdit::new("a\nb\nc\n");
        edit.place(1, 0);
        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
        assert!(!edit.key(key(KeyCode::Backspace), 1..2));
        edit.key(key(KeyCode::Up), 1..2);
        edit.key(key(KeyCode::Down), 1..2);
        assert_eq!(edit.row, 1);
        assert_eq!(edit.text(), "a\nb\nc\n");
    }

    #[test]
    fn handles_multibyte_characters() {
        let mut edit = TextEdit::new("é\n");
        edit.place(0, 1);
        press(&mut edit, KeyCode::Char('x'));
        press(&mut edit, KeyCode::Left);
        press(&mut edit, KeyCode::Backspace);
        assert_eq!(edit.text(), "x\n");
    }
}
