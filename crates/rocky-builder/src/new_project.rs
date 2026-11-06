//! The New project form: a folder name and location, a basic or full starter, an optional
//! `.env`, and a preview of what will be written.

use std::path::{Path, PathBuf};

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use rocky_core::new::Level;
use rocky_core::project::SETTINGS_FILE;

use crate::home::{expand, tilde};
use crate::input::Input;
use crate::theme;

/// The starter job file, relative to the project folder.
const JOB_FILE: &str = "jobs/example.yaml";

/// The form's fields, top to bottom.
const NAME: usize = 0;
const LOCATION: usize = 1;
const STARTER: usize = 2;
const ENV_FILE: usize = 3;
const OPEN_AFTER: usize = 4;
const FIELDS: usize = 5;

pub struct NewProject {
    name: Input,
    location: Input,
    full: bool,
    env_file: bool,
    open_after: bool,
    focus: usize,
    /// Why the last Create failed.
    error: Option<String>,
}

/// What the form asked for.
pub enum Outcome {
    Cancel,
    /// The project was written at `path`; `open` it in the builder.
    Created {
        path: PathBuf,
        open: bool,
    },
}

impl NewProject {
    pub fn new(location: &Path) -> NewProject {
        NewProject {
            name: Input::default(),
            location: Input::new(&tilde(location)),
            full: false,
            env_file: true,
            open_after: true,
            focus: NAME,
            error: None,
        }
    }

    fn folder(&self) -> PathBuf {
        expand(&self.location.text).join(self.name.text.trim())
    }

    /// Why the project can't be created as entered, if it can't.
    fn problem(&self) -> Option<String> {
        let name = self.name.text.trim();
        if name.is_empty() {
            return Some("Enter a folder name.".to_string());
        }
        if name.contains('/') || name == "." || name == ".." {
            return Some("A folder name, without `/`.".to_string());
        }
        if self.location.text.trim().is_empty() {
            return Some("Choose a location.".to_string());
        }
        [JOB_FILE, SETTINGS_FILE]
            .iter()
            .map(|file| self.folder().join(file))
            .find(|path| path.exists())
            .map(|path| format!("{} already exists.", tilde(&path)))
    }

    /// Writes the project and returns its folder.
    fn create(&self) -> Result<PathBuf, String> {
        let folder = self.folder();
        let level = if self.full { Level::Full } else { Level::Basic };
        rocky_core::new::project(level, &folder, self.env_file)
            .map(|_| folder)
            .map_err(|error| error.chain())
    }

    /// ↑ ↓ ⇥ move between fields, space toggles, Enter creates, Esc cancels.
    pub fn handle(&mut self, key: KeyEvent) -> Option<Outcome> {
        match key.code {
            KeyCode::Esc => return Some(Outcome::Cancel),
            KeyCode::Enter => {
                if let Some(problem) = self.problem() {
                    self.error = Some(problem);
                    return None;
                }
                return match self.create() {
                    Ok(path) => Some(Outcome::Created {
                        path,
                        open: self.open_after,
                    }),
                    Err(error) => {
                        self.error = Some(error);
                        None
                    }
                };
            }
            KeyCode::Up | KeyCode::BackTab => self.focus = (self.focus + FIELDS - 1) % FIELDS,
            KeyCode::Down | KeyCode::Tab => self.focus = (self.focus + 1) % FIELDS,
            _ => {
                self.error = None;
                match self.focus {
                    NAME => {
                        self.name.handle(key);
                    }
                    LOCATION => {
                        self.location.handle(key);
                    }
                    field => {
                        if matches!(
                            key.code,
                            KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right
                        ) {
                            let flag = match field {
                                STARTER => &mut self.full,
                                ENV_FILE => &mut self.env_file,
                                _ => &mut self.open_after,
                            };
                            *flag = !*flag;
                        }
                    }
                }
            }
        }
        None
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        let width = area.width.min(80);
        let height = area.height.min(20);
        let area = Rect::new(area.x + (area.width - width) / 2, area.y + 1, width, height);
        frame.render_widget(Clear, area);
        let block = Block::bordered()
            .title(Span::styled(" New project ", theme::bold()))
            .title_bottom(Line::styled(
                " ↑↓ move · space toggle · enter create · esc cancel ",
                theme::dim(),
            ))
            .border_style(theme::accent());
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let [name, location, rest] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Fill(1),
        ])
        .areas(inner);
        for (index, (label, field, area)) in [
            ("name", &self.name, name),
            ("location", &self.location, location),
        ]
        .into_iter()
        .enumerate()
        {
            let [label_area, value] =
                Layout::horizontal([Constraint::Length(12), Constraint::Fill(1)]).areas(area);
            frame.render_widget(Paragraph::new(self.label(index, label)), label_area);
            if self.focus == index {
                field.draw(frame, value);
            } else {
                let text = if field.text.is_empty() && index == NAME {
                    Span::styled("automation", theme::dim())
                } else {
                    Span::styled(field.text.clone(), theme::text())
                };
                frame.render_widget(Paragraph::new(text), value);
            }
        }
        let radio = |on: bool| if on { "(•)" } else { "( )" };
        let check = |on: bool| if on { "[x]" } else { "[ ]" };
        let folder = self.folder();
        let mut lines = vec![
            Line::from(vec![
                self.label(STARTER, "starter"),
                Span::raw(format!(
                    "{} Basic   {} Full",
                    radio(!self.full),
                    radio(self.full)
                )),
            ]),
            Line::styled(
                if self.full {
                    "            Every field with a comment: schedule, values, defaults, when, …"
                } else {
                    "            The smallest project that runs: one job, one task."
                },
                theme::dim(),
            ),
            Line::from(vec![
                self.label(ENV_FILE, ""),
                Span::raw(format!("{} Create a .env file", check(self.env_file))),
            ]),
            Line::from(vec![
                self.label(OPEN_AFTER, ""),
                Span::raw(format!(
                    "{} Open in the builder after creating",
                    check(self.open_after)
                )),
            ]),
            Line::raw(""),
            Line::styled("WILL CREATE", theme::heading()),
        ];
        let exists = if folder.is_dir() { "  (exists)" } else { "" };
        lines.push(Line::raw(format!("{}/{exists}", tilde(&folder))));
        let what = if self.full {
            "job `example` · every field, commented"
        } else {
            "job `example` · echo hello"
        };
        let env = self.env_file.then(|| {
            if folder.join(".env").exists() {
                "env_file, kept as is"
            } else {
                "env_file, empty"
            }
        });
        if self.full || self.env_file {
            let settings = if self.full {
                "values, env_file, defaults"
            } else {
                "env_file: .env"
            };
            lines.push(tree_line("├─", SETTINGS_FILE, settings));
        }
        let branch = if env.is_some() { "├─" } else { "└─" };
        lines.push(tree_line(branch, JOB_FILE, what));
        if let Some(env) = env {
            lines.push(tree_line("└─", ".env", env));
        }
        lines.push(Line::raw(""));
        let level = if self.full { " --level full" } else { "" };
        lines.push(Line::styled(
            format!("rocky new project {}{level}", tilde(&folder)),
            theme::dim(),
        ));
        let problem = self
            .error
            .clone()
            .or_else(|| self.problem().filter(|_| !self.name.text.trim().is_empty()));
        if let Some(problem) = problem {
            lines.push(Line::styled(problem, theme::error()));
        }
        frame.render_widget(Paragraph::new(lines), rest);
    }

    /// A field's label, marked when focused.
    fn label(&self, index: usize, label: &str) -> Span<'static> {
        let text = format!(
            "{:<12}",
            if self.focus == index {
                format!("› {label}")
            } else {
                format!("  {label}")
            }
        );
        let style = if self.focus == index {
            theme::accent()
        } else {
            theme::dim()
        };
        Span::styled(text, style)
    }
}

fn tree_line(branch: &str, file: &str, what: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw(format!("  {branch} {file:<18}")),
        Span::styled(what.to_string(), theme::dim()),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet(dir: &Path, name: &str) -> NewProject {
        let mut sheet = NewProject::new(dir);
        sheet.name = Input::new(name);
        sheet
    }

    #[test]
    fn creates_the_project_and_env_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut sheet = sheet(dir.path(), "automation");
        sheet.full = true;
        assert_eq!(sheet.problem(), None);
        let path = sheet.create().unwrap();
        assert_eq!(path, dir.path().join("automation"));
        assert!(dir.path().join("automation/.env").is_file());
        rocky_core::project::load(&path.join(SETTINGS_FILE)).unwrap();
        assert!(sheet.problem().unwrap().contains("already exists"));
    }

    #[test]
    fn checks_the_name() {
        let dir = tempfile::tempdir().unwrap();
        assert!(sheet(dir.path(), " ").problem().is_some());
        assert!(sheet(dir.path(), "a/b").problem().is_some());
        assert!(sheet(dir.path(), "..").problem().is_some());
        assert_eq!(sheet(dir.path(), "ok").problem(), None);
    }
}
