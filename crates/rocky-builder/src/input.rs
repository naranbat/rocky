//! A one-line text field: the text, a cursor (a char index), and the keys that edit them.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::theme;

#[derive(Clone, Debug, Default)]
pub struct Input {
    pub text: String,
    /// Chars before the cursor.
    pub cursor: usize,
}

/// What a key did to the field.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// Enter: the text is done.
    Done,
    /// Esc: drop the edit.
    Cancel,
    /// The key edited the text or moved the cursor, or did nothing.
    Edited,
}

impl Input {
    /// A field holding `text`, with the cursor at its end.
    pub fn new(text: &str) -> Input {
        Input {
            text: text.to_string(),
            cursor: text.chars().count(),
        }
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    /// The byte offset of char `index`.
    fn byte(&self, index: usize) -> usize {
        self.text
            .char_indices()
            .nth(index)
            .map_or(self.text.len(), |(byte, _)| byte)
    }

    pub fn handle(&mut self, key: KeyEvent) -> Outcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Enter => return Outcome::Done,
            KeyCode::Esc => return Outcome::Cancel,
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.len()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.len(),
            KeyCode::Char('a') if ctrl => self.cursor = 0,
            KeyCode::Char('e') if ctrl => self.cursor = self.len(),
            KeyCode::Char('u') if ctrl => {
                let at = self.byte(self.cursor);
                self.text.replace_range(..at, "");
                self.cursor = 0;
            }
            KeyCode::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                let at = self.byte(self.cursor);
                self.text.remove(at);
            }
            KeyCode::Delete if self.cursor < self.len() => {
                let at = self.byte(self.cursor);
                self.text.remove(at);
            }
            KeyCode::Char(c) if !ctrl => {
                let at = self.byte(self.cursor);
                self.text.insert(at, c);
                self.cursor += 1;
            }
            _ => {}
        }
        Outcome::Edited
    }

    /// Draws the text in `area`, scrolled so the cursor shows, and puts the terminal's
    /// cursor on it.
    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        let width = usize::from(area.width.max(1));
        let skip = (self.cursor + 1).saturating_sub(width);
        let shown: String = self.text.chars().skip(skip).take(width).collect();
        let line = Line::from(Span::styled(shown, theme::accent()));
        frame.render_widget(Paragraph::new(line).style(theme::text()), area);
        let x = area.x + u16::try_from(self.cursor - skip).unwrap_or(area.width);
        frame.set_cursor_position(Position::new(x.min(area.right().saturating_sub(1)), area.y));
    }
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::KeyEvent;

    use super::*;

    fn press(input: &mut Input, code: KeyCode) -> Outcome {
        input.handle(KeyEvent::from(code))
    }

    #[test]
    fn edits_at_the_cursor() {
        let mut input = Input::new("héllo");
        press(&mut input, KeyCode::Left);
        press(&mut input, KeyCode::Backspace);
        press(&mut input, KeyCode::Char('L'));
        assert_eq!(input.text, "hélLo");
        press(&mut input, KeyCode::Home);
        press(&mut input, KeyCode::Delete);
        assert_eq!(input.text, "élLo");
        assert_eq!(press(&mut input, KeyCode::Enter), Outcome::Done);
        assert_eq!(press(&mut input, KeyCode::Esc), Outcome::Cancel);
    }
}
