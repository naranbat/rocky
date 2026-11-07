//! The home screen's Open project pane: a path, and the folders and project files under it.

use std::path::{Path, PathBuf};

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Borders, Paragraph};
use rocky_core::project::SETTINGS_FILE;
use serde_yaml::Value;

use crate::chrome;
use crate::home::{expand, tilde};
use crate::line_input::LineInput;
use crate::new_pane::input;
use crate::theme::{self, fg};

struct Entry {
    name: String,
    path: PathBuf,
    dir: bool,
    /// For a folder with a `rocky.yaml`: how many jobs it lists.
    jobs: Option<usize>,
}

pub struct OpenPane {
    path: LineInput,
    entries: Vec<Entry>,
    cursor: usize,
}

impl OpenPane {
    pub fn new(cwd: &Path) -> OpenPane {
        let mut pane = OpenPane {
            path: LineInput::new(&format!("{}/", tilde(cwd))),
            entries: Vec::new(),
            cursor: 0,
        };
        pane.list();
        pane
    }

    /// The folder listed, and the start of the names shown.
    fn folder(&self) -> (PathBuf, String) {
        let typed = expand(&self.path.text);
        if self.path.text.ends_with('/') || self.path.text.is_empty() {
            return (typed, String::new());
        }
        let prefix = typed
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let parent = typed.parent().unwrap_or(Path::new("/")).to_path_buf();
        (parent, prefix)
    }

    /// Lists the folders and `.yaml` files matching the typed path.
    fn list(&mut self) {
        let (folder, prefix) = self.folder();
        let mut entries: Vec<Entry> = std::fs::read_dir(&folder)
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                let path = entry.ok()?.path();
                let name = path.file_name()?.to_string_lossy().into_owned();
                let dir = path.is_dir();
                let yaml = path.extension().is_some_and(|ext| ext == "yaml");
                let shown = !name.starts_with('.') && name.starts_with(&prefix) && (dir || yaml);
                shown.then(|| Entry {
                    jobs: dir.then(|| job_count(&path)).flatten(),
                    name,
                    path,
                    dir,
                })
            })
            .collect();
        entries.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.cmp(&b.name)));
        self.entries = entries;
        self.cursor = 0;
    }

    fn go_to(&mut self, dir: &Path) {
        self.path.set(&format!("{}/", tilde(dir)));
        self.list();
    }

    /// Edits for `key`; Enter returns the project to open.
    pub fn key(&mut self, key: KeyEvent) -> Result<Option<PathBuf>, String> {
        let selected = self.entries.get(self.cursor);
        match key.code {
            KeyCode::Up => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Down => {
                self.cursor = (self.cursor + 1).min(self.entries.len().saturating_sub(1))
            }
            KeyCode::Tab | KeyCode::Right => {
                if let Some(entry) = selected {
                    if entry.dir {
                        let path = entry.path.clone();
                        self.go_to(&path);
                    } else if key.code == KeyCode::Tab {
                        self.path.set(&tilde(&entry.path));
                        self.list();
                    }
                }
            }
            KeyCode::Left => {
                let (folder, _) = self.folder();
                if let Some(parent) = folder.parent() {
                    self.go_to(parent);
                }
            }
            KeyCode::Enter => {
                let (folder, prefix) = self.folder();
                if prefix.is_empty() && folder.join(SETTINGS_FILE).is_file() {
                    return Ok(Some(folder));
                }
                match selected {
                    Some(entry) if entry.dir && entry.jobs.is_none() => {
                        let path = entry.path.clone();
                        self.go_to(&path);
                    }
                    Some(entry) => return Ok(Some(entry.path.clone())),
                    None => {
                        let typed = expand(&self.path.text);
                        if typed.is_file() {
                            return Ok(Some(typed));
                        }
                        return Err(format!("no {SETTINGS_FILE} in {}", tilde(&typed)));
                    }
                }
            }
            _ => {
                if self.path.key(key) {
                    self.list();
                }
            }
        }
        Ok(None)
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect, focused: bool) {
        let [title, about, _, label, path, _, list, _, button] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(area);
        frame.render_widget(
            Paragraph::new(Line::styled("Open project", theme::bold(theme::TEXT))),
            title,
        );
        frame.render_widget(
            Paragraph::new(Line::styled(
                "A folder with a rocky.yaml, or any project file.",
                fg(theme::MUTED),
            )),
            about,
        );
        frame.render_widget(
            Paragraph::new(Line::styled("Path", fg(theme::MUTED))),
            label,
        );
        input(frame, path, &self.path, focused);

        let height = (self.entries.len() as u16 + 2).min(list.height);
        let list = Rect { height, ..list };
        let rows = height.saturating_sub(2) as usize;
        let top = (self.cursor + 1).saturating_sub(rows);
        let width = list.width.saturating_sub(4);
        let lines: Vec<Line> = self
            .entries
            .iter()
            .enumerate()
            .skip(top)
            .take(rows)
            .map(|(n, entry)| self.row(entry, n == self.cursor && focused, width))
            .collect();
        if !lines.is_empty() {
            frame.render_widget(
                Paragraph::new(lines).block(chrome::lines(Borders::ALL)),
                list,
            );
        }

        let width = chrome::button(frame, button, "↵ Open", true);
        let hint_area = Rect {
            x: button.x + width,
            y: button.y,
            width: button.width.saturating_sub(width),
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::styled("  ⇥ complete · ←→ folders", fg(theme::MUTED))),
            hint_area,
        );
    }

    fn row(&self, entry: &Entry, selected: bool, width: u16) -> Line<'static> {
        let (mark, name, note, note_color) = match (entry.dir, entry.jobs) {
            (true, Some(jobs)) => (
                "▸",
                format!("{}/", entry.name),
                format!(
                    "rocky.yaml · {jobs} job{}",
                    if jobs == 1 { "" } else { "s" }
                ),
                theme::OK,
            ),
            (true, None) => ("▸", format!("{}/", entry.name), String::new(), theme::MUTED),
            (false, _) => (
                "·",
                entry.name.clone(),
                "project file (-f)".to_string(),
                theme::MUTED,
            ),
        };
        let bar = if selected {
            Span::styled("▌", fg(theme::ACCENT))
        } else {
            Span::raw(" ")
        };
        let note_width = width / 4 + 8;
        let line = chrome::spread(
            vec![
                bar,
                Span::styled(format!(" {mark}  "), fg(theme::MUTED)),
                Span::styled(name, fg(theme::TEXT)),
            ],
            vec![Span::styled(
                format!("{note:<w$}", w = note_width as usize),
                fg(note_color),
            )],
            width + 2,
        );
        if selected {
            line.style(Style::new().bg(theme::SELECTED))
        } else {
            line
        }
    }
}

/// How many jobs the `rocky.yaml` in `dir` lists; `None` without one.
fn job_count(dir: &Path) -> Option<usize> {
    let text = std::fs::read_to_string(dir.join(SETTINGS_FILE)).ok()?;
    let settings: Value = serde_yaml::from_str(&text).unwrap_or_default();
    Some(
        settings
            .get("jobs")
            .and_then(Value::as_mapping)
            .map_or(0, |jobs| jobs.len()),
    )
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::KeyModifiers;

    use super::*;

    fn press(pane: &mut OpenPane, code: KeyCode) -> Result<Option<PathBuf>, String> {
        pane.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn lists_projects_and_opens_one() {
        let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let examples = std::fs::canonicalize(examples).unwrap();
        let mut pane = OpenPane::new(&examples);
        let httpbin = pane.entries.iter().find(|e| e.name == "httpbin").unwrap();
        assert_eq!(httpbin.jobs, Some(8));

        for c in "htt".chars() {
            press(&mut pane, KeyCode::Char(c)).unwrap();
        }
        assert_eq!(pane.entries.len(), 1);
        assert_eq!(
            press(&mut pane, KeyCode::Enter),
            Ok(Some(examples.join("httpbin")))
        );

        press(&mut pane, KeyCode::Tab).unwrap();
        assert!(pane.path.text.ends_with("httpbin/"));
        assert_eq!(
            press(&mut pane, KeyCode::Enter),
            Ok(Some(examples.join("httpbin/")))
        );
        press(&mut pane, KeyCode::Left).unwrap();
        assert!(pane.entries.iter().any(|e| e.name == "values"));
    }
}
