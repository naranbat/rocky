//! The `${…}` suggestion list of a text field: shown under the field while the cursor is in
//! an unclosed `${`, driven by ↑ ↓ to choose, ↵ or ⇥ to insert, and esc to close.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::complete::{self, Mode, Var};
use crate::input::Input;
use crate::theme;

/// One field's suggestions: what it can read, and the list's state.
pub struct Suggest {
    vars: Vec<Var>,
    mode: Mode,
    chosen: usize,
    /// Closed with esc until the text changes.
    closed: bool,
}

impl Suggest {
    pub fn new(vars: Vec<Var>, mode: Mode) -> Suggest {
        Suggest {
            vars,
            mode,
            chosen: 0,
            closed: false,
        }
    }

    /// Where the typed prefix starts, and the variables it matches.
    fn shown(&self, input: &Input) -> Option<(usize, Vec<&Var>)> {
        if self.closed {
            return None;
        }
        let start = complete::query(&input.text, input.cursor, self.mode)?;
        let prefix: String = input
            .text
            .chars()
            .skip(start)
            .take(input.cursor.saturating_sub(start))
            .collect();
        let found = complete::matches(&self.vars, &prefix);
        let done = matches!(found.as_slice(), [only] if only.path == prefix);
        (!found.is_empty() && !done).then_some((start, found))
    }

    /// Handles `key` if the list is open and it is one of the list's keys; otherwise leaves
    /// it to the field. Returns whether it was handled.
    pub fn handle(&mut self, input: &mut Input, key: KeyEvent) -> bool {
        let Some((start, found)) = self.shown(input) else {
            return false;
        };
        match key.code {
            KeyCode::Up => self.chosen = self.chosen.saturating_sub(1),
            KeyCode::Down => self.chosen = (self.chosen + 1).min(found.len() - 1),
            KeyCode::Enter | KeyCode::Tab => {
                let path = found[self.chosen.min(found.len() - 1)].path.clone();
                let (text, cursor) = complete::apply(&input.text, start, input.cursor, &path);
                *input = Input { text, cursor };
                self.chosen = 0;
            }
            KeyCode::Esc => self.closed = true,
            _ => return false,
        }
        true
    }

    /// The text changed: show the list again, from the top.
    pub fn edited(&mut self) {
        self.closed = false;
        self.chosen = 0;
    }

    /// Draws the list under `field`, within `bounds`.
    pub fn draw(&self, frame: &mut Frame, input: &Input, field: Rect, bounds: Rect) {
        let Some((_, found)) = self.shown(input) else {
            return;
        };
        let width = found
            .iter()
            .map(|var| var.path.chars().count() + var.detail.chars().count() + 4)
            .max()
            .unwrap_or(0)
            .clamp(24, 72);
        let width = u16::try_from(width).unwrap_or(72).min(bounds.width);
        let height = u16::try_from(found.len() + 2).unwrap_or(u16::MAX);
        let below = field.y + 1;
        let y = if below + height <= bounds.bottom() {
            below
        } else {
            field.y.saturating_sub(height)
        };
        let x = field.x.min(bounds.right().saturating_sub(width));
        let area = Rect::new(x, y, width, height).intersection(bounds);
        let lines: Vec<Line> = found
            .iter()
            .enumerate()
            .map(|(index, var)| {
                let style = if index == self.chosen {
                    theme::accent()
                } else {
                    theme::text()
                };
                Line::from(vec![
                    Span::styled(var.path.clone(), style),
                    Span::styled(format!("  {}", var.detail), theme::dim()),
                ])
            })
            .collect();
        let group = found
            .get(self.chosen)
            .map(|var| var.group.clone())
            .unwrap_or_default();
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::bordered()
                    .title(Span::styled(format!(" {group} "), theme::dim()))
                    .border_style(theme::dim()),
            ),
            area,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(path: &str) -> Var {
        Var {
            path: path.to_string(),
            detail: String::new(),
            group: "G".to_string(),
        }
    }

    /// Types `${BA` and picks a suggestion with ↓ and ↵.
    #[test]
    fn typing_dollar_brace_suggests_and_inserts() {
        let mut suggest = Suggest::new(
            vec![var("BASE"), var("BATCH"), var("tasks.login.ok")],
            Mode::Inline,
        );
        let mut input = Input::new("x ");
        for c in "${BA".chars() {
            let key = KeyEvent::from(KeyCode::Char(c));
            assert!(!suggest.handle(&mut input, key));
            input.handle(key);
            suggest.edited();
        }
        assert_eq!(input.text, "x ${BA");
        assert!(suggest.handle(&mut input, KeyEvent::from(KeyCode::Down)));
        assert!(suggest.handle(&mut input, KeyEvent::from(KeyCode::Enter)));
        assert_eq!(input.text, "x ${BATCH}");
        // Closed now: Enter goes to the field.
        assert!(!suggest.handle(&mut input, KeyEvent::from(KeyCode::Enter)));
    }
}
