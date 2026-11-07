//! The home screen's New project pane: a directory, a level, and the files it will create.

use std::path::{Path, PathBuf};

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Borders, Paragraph};
use rocky_core::new::{self, Level};

use crate::chrome;
use crate::home::{expand, tilde};
use crate::line_input::LineInput;
use crate::theme::{self, fg};

pub struct NewPane {
    dir: LineInput,
    full: bool,
}

impl NewPane {
    pub fn new(cwd: &Path) -> NewPane {
        NewPane {
            dir: LineInput::new(&format!("{}/", tilde(cwd))),
            full: false,
        }
    }

    /// Edits for `key`; Enter creates the project and returns its directory.
    pub fn key(&mut self, key: KeyEvent) -> Result<Option<PathBuf>, String> {
        match key.code {
            KeyCode::Tab | KeyCode::BackTab => self.full = !self.full,
            KeyCode::Enter => {
                let dir = expand(&self.dir.text);
                let level = if self.full { Level::Full } else { Level::Basic };
                new::project(level, &dir, false).map_err(|error| error.chain())?;
                return Ok(Some(dir));
            }
            _ => {
                self.dir.key(key);
            }
        }
        Ok(None)
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect, focused: bool) {
        let [
            title,
            about,
            _,
            dir_label,
            dir,
            _,
            level_label,
            level,
            _,
            tree_label,
            tree,
            _,
            button,
        ] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(6),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .areas(area);
        let text = |area, line: Line<'static>, frame: &mut Frame| {
            frame.render_widget(Paragraph::new(line), area)
        };
        text(
            title,
            Line::styled("New project", theme::bold(theme::TEXT)),
            frame,
        );
        text(
            about,
            Line::styled(
                "Writes starter files that load as-is. Never overwrites.",
                fg(theme::MUTED),
            ),
            frame,
        );
        text(
            dir_label,
            Line::styled("Directory", fg(theme::MUTED)),
            frame,
        );
        input(frame, dir, &self.dir, focused);

        text(level_label, Line::styled("Level", fg(theme::MUTED)), frame);
        let options = [
            (false, "basic — smallest file that runs"),
            (true, "full — every field, commented"),
        ];
        let mut x = level.x;
        for (full, label) in options {
            let chosen = full == self.full;
            let label = format!("◉ {label}");
            let width = (label.chars().count() as u16 + 4).min(level.right().saturating_sub(x));
            let area = Rect { x, width, ..level };
            let border = if chosen && focused {
                theme::ACCENT
            } else {
                theme::LINE
            };
            let style = if chosen {
                theme::bold(theme::TEXT)
            } else {
                fg(theme::MUTED)
            };
            frame.render_widget(
                Paragraph::new(Line::styled(label, style))
                    .block(chrome::lines(Borders::ALL).border_style(fg(border))),
                area,
            );
            x += width + 1;
        }

        text(
            tree_label,
            Line::styled("Will create", fg(theme::MUTED)),
            frame,
        );
        let name = expand(&self.dir.text)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (settings_note, job_note) = if self.full {
            ("every field, commented", "every task kind")
        } else {
            ("lists one job", "echo hello")
        };
        let file = |branch: &str, name: &str, note: &str| {
            Line::from(vec![
                Span::styled(branch.to_string(), fg(theme::MUTED)),
                Span::styled(format!("{name:<14}"), fg(theme::OK)),
                Span::styled(note.to_string(), fg(theme::MUTED)),
            ])
        };
        let lines = vec![
            Line::styled(format!("{name}/"), fg(theme::TEXT)),
            file("├── ", "rocky.yaml", settings_note),
            Line::styled("└── jobs/", fg(theme::TEXT)),
            file("    └── ", "example.yaml", job_note),
        ];
        frame.render_widget(
            Paragraph::new(lines).block(
                chrome::lines(Borders::ALL).padding(ratatui::widgets::Padding::horizontal(1)),
            ),
            tree,
        );

        let width = chrome::button(frame, button, "↵ Create & open", true);
        let level = if self.full { " --level full" } else { "" };
        let hint = format!(
            "  $ rocky new project {}{level}",
            self.dir.text.trim_end_matches('/')
        );
        let hint_area = Rect {
            x: button.x + width,
            y: button.y,
            width: button.width.saturating_sub(width),
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::styled(hint, fg(theme::MUTED))),
            hint_area,
        );
    }
}

/// A framed text input, `› typed`, outlined in the accent while `focused`.
pub fn input(frame: &mut Frame, area: Rect, input: &LineInput, focused: bool) {
    let border = if focused { theme::ACCENT } else { theme::LINE };
    let mut spans = vec![Span::styled("› ", fg(theme::ACCENT))];
    // Inside the frame, its padding and `› `.
    let width = area.width.saturating_sub(6) as usize;
    spans.extend(input.spans(focused, width));
    frame.render_widget(
        Paragraph::new(Line::from(spans)).block(
            chrome::lines(Borders::ALL)
                .border_style(fg(border))
                .padding(ratatui::widgets::Padding::horizontal(1)),
        ),
        area,
    );
}
