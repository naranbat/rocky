//! A one-line text input with a cursor.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Style;
use ratatui::text::Span;

use crate::theme;

#[derive(Default)]
pub struct LineInput {
    pub text: String,
    /// A character index.
    cursor: usize,
}

impl LineInput {
    pub fn new(text: &str) -> LineInput {
        LineInput {
            text: text.to_string(),
            cursor: text.chars().count(),
        }
    }

    /// Replaces the text, with the cursor at its end.
    pub fn set(&mut self, text: &str) {
        *self = LineInput::new(text);
    }

    /// Edits for `key`; returns whether the text changed.
    pub fn key(&mut self, key: KeyEvent) -> bool {
        let at = |text: &str, n: usize| text.char_indices().nth(n).map_or(text.len(), |(i, _)| i);
        match key.code {
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                let index = at(&self.text, self.cursor);
                self.text.insert(index, c);
                self.cursor += 1;
                true
            }
            KeyCode::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                let index = at(&self.text, self.cursor);
                self.text.remove(index);
                true
            }
            KeyCode::Left => {
                self.cursor = self.cursor.saturating_sub(1);
                false
            }
            KeyCode::Right => {
                self.cursor = (self.cursor + 1).min(self.text.chars().count());
                false
            }
            KeyCode::Home => {
                self.cursor = 0;
                false
            }
            KeyCode::End => {
                self.cursor = self.text.chars().count();
                false
            }
            _ => false,
        }
    }

    /// The text in at most `width` characters, scrolled to show the cursor, which is drawn
    /// when `focused`.
    pub fn spans(&self, focused: bool, width: usize) -> Vec<Span<'static>> {
        let chars: Vec<char> = self.text.chars().collect();
        let start = (self.cursor + 1).saturating_sub(width);
        let shown = |range: std::ops::Range<usize>| -> String {
            chars[range.start.min(chars.len())..range.end.min(chars.len())]
                .iter()
                .collect()
        };
        let end = start + width;
        if !focused {
            return vec![Span::styled(shown(start..end), theme::fg(theme::TEXT))];
        }
        let under = chars
            .get(self.cursor)
            .map_or(" ".to_string(), char::to_string);
        vec![
            Span::styled(shown(start..self.cursor), theme::fg(theme::TEXT)),
            Span::styled(under, Style::new().fg(theme::BG).bg(theme::ACCENT)),
            Span::styled(shown(self.cursor + 1..end), theme::fg(theme::TEXT)),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_at_the_cursor() {
        let mut input = LineInput::new("ac");
        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
        input.key(key(KeyCode::Left));
        assert!(input.key(key(KeyCode::Char('b'))));
        assert_eq!(input.text, "abc");
        input.key(key(KeyCode::End));
        assert!(input.key(key(KeyCode::Backspace)));
        assert_eq!(input.text, "ab");
        input.key(key(KeyCode::Home));
        assert!(!input.key(key(KeyCode::Backspace)));
    }

    #[test]
    fn scrolls_to_show_the_cursor() {
        let texts = |input: &LineInput, width| -> Vec<String> {
            input
                .spans(true, width)
                .into_iter()
                .map(|span| span.content.into_owned())
                .collect()
        };
        let mut input = LineInput::new("abcdef");
        assert_eq!(texts(&input, 4), ["def", " ", ""]);
        input.key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        assert_eq!(texts(&input, 4), ["", "a", "bcd"]);
    }
}
