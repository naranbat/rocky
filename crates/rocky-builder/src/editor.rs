//! An open project: the tabs (Jobs, Project, Values, Env file), and the job YAML and debug
//! screens opened from the Jobs tab. Holds the files, and what core says of them.

use std::path::Path;
use std::time::Duration;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::text::Span;

use crate::chrome;
use crate::debug_view::{self, Debug};
use crate::file_view;
use crate::files::{Buffer, Files};
use crate::home::tilde;
use crate::job_yaml::JobYaml;
use crate::jobs_view;
use crate::line_input::LineInput;
use crate::problem::{self, Problem};
use crate::theme::{self, fg};

#[derive(Clone, Copy, PartialEq)]
pub enum Tab {
    Jobs,
    Project,
    Values,
    Env,
}

pub enum View {
    Tabs,
    Yaml(JobYaml),
    Debug(Box<Debug>),
}

pub enum Action {
    None,
    Quit,
}

/// A file of the project.
#[derive(Clone, Copy)]
pub enum Target {
    Settings,
    Job(usize),
    Env,
}

pub struct Editor {
    pub files: Files,
    pub tab: Tab,
    pub view: View,
    /// The selected job, an index in `files.jobs`.
    pub job: usize,
    /// The selected task of the job in the Jobs tab, when the task list has the focus.
    pub task: Option<usize>,
    /// Why the project doesn't load, if it doesn't.
    pub problem: Option<Problem>,
    /// Shown in the footer until the next key.
    pub message: Option<String>,
    /// The job list's filter, and whether it is being typed.
    pub filter: LineInput,
    pub filtering: bool,
    /// The name of the job being added.
    pub new_job: Option<LineInput>,
    /// Typing goes into the file of the Project, Values or Env file tab.
    pub editing: bool,
    /// `q` was pressed with unsaved changes: another `q` quits.
    quit_armed: bool,
    /// A file to open in `$EDITOR`, once the terminal is handed over.
    external: Option<Target>,
}

impl Editor {
    pub fn new(files: Files) -> Editor {
        let mut editor = Editor {
            files,
            tab: Tab::Jobs,
            view: View::Tabs,
            job: 0,
            task: None,
            problem: None,
            message: None,
            filter: LineInput::default(),
            filtering: false,
            new_job: None,
            editing: false,
            quit_armed: false,
            external: None,
        };
        editor.check();
        editor
    }

    /// Asks core whether the project loads as it is now.
    fn check(&mut self) {
        self.files.sync();
        self.job = self.job.min(self.files.jobs.len().saturating_sub(1));
        self.problem = self.files.load().err().map(|error| problem::locate(&error));
    }

    pub fn buffer(&self, target: Target) -> Option<&Buffer> {
        match target {
            Target::Settings => Some(&self.files.settings),
            Target::Job(n) => self.files.jobs.get(n).map(|job| &job.buffer),
            Target::Env => self.files.env.as_ref(),
        }
    }

    pub fn buffer_mut(&mut self, target: Target) -> Option<&mut Buffer> {
        match target {
            Target::Settings => Some(&mut self.files.settings),
            Target::Job(n) => self.files.jobs.get_mut(n).map(|job| &mut job.buffer),
            Target::Env => self.files.env.as_mut(),
        }
    }

    /// Undoes the last change to `target`.
    pub fn undo(&mut self, target: Target) {
        let undone = self
            .buffer_mut(target)
            .is_some_and(|buffer| buffer.edit.undo());
        if !undone {
            self.message = Some("nothing to undo".to_string());
        }
    }

    /// Opens `target` in `$EDITOR` once the main loop hands over the terminal.
    pub fn open_external(&mut self, target: Target) {
        self.external = Some(target);
    }

    /// The file to open in `$EDITOR`: its name and text.
    pub fn take_external(&mut self) -> Option<(String, String)> {
        let target = self.external?;
        let buffer = self.buffer(target)?;
        let name = buffer
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        Some((name, buffer.text()))
    }

    /// Takes in what `$EDITOR` returned for the file of `take_external`.
    pub fn finish_external(&mut self, edited: Result<String, String>) {
        let Some(target) = self.external.take() else {
            return;
        };
        match edited {
            Ok(text) => {
                if let Some(buffer) = self.buffer_mut(target)
                    && text != buffer.text()
                {
                    buffer.edit.set_text(&text);
                }
            }
            Err(error) => self.message = Some(error),
        }
        self.check();
    }

    fn save(&mut self) {
        let count = self.files.unsaved();
        self.message = Some(match self.files.save() {
            Ok(()) if count == 0 => "nothing to save".to_string(),
            Ok(()) => format!("saved {count} file{}", if count == 1 { "" } else { "s" }),
            Err(error) => error,
        });
    }

    /// `q`: quits, or with unsaved changes asks for a second `q`.
    pub fn quit(&mut self) -> Action {
        let unsaved = self.files.unsaved();
        if unsaved == 0 || self.quit_armed {
            return Action::Quit;
        }
        self.quit_armed = true;
        self.message = Some(format!("{unsaved} unsaved: ^S to save, q again to quit"));
        Action::None
    }

    pub fn key(&mut self, key: KeyEvent) -> Action {
        self.message = None;
        let armed = std::mem::take(&mut self.quit_armed);
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
            self.save();
            return Action::None;
        }
        let action = match std::mem::replace(&mut self.view, View::Tabs) {
            View::Tabs => {
                // Only a second `q` in a row quits.
                self.quit_armed = armed && key.code == KeyCode::Char('q');
                match self.tab {
                    Tab::Jobs => jobs_view::key(self, key),
                    _ => file_view::key(self, key),
                }
            }
            View::Yaml(mut yaml) => {
                if !yaml.key(self, key) {
                    self.job = yaml.job;
                    self.task = yaml.part;
                } else {
                    self.view = View::Yaml(yaml);
                }
                Action::None
            }
            View::Debug(mut debug) => match debug.key(self, key) {
                debug_view::Next::Stay => {
                    self.view = View::Debug(debug);
                    Action::None
                }
                debug_view::Next::Back => Action::None,
                debug_view::Next::Edit(part) => {
                    self.view = View::Yaml(JobYaml::new(self, debug.job, part, false));
                    Action::None
                }
            },
        };
        self.check();
        action
    }

    /// Keeps a debug run going; returns how long to wait for a key.
    pub fn tick(&mut self) -> Duration {
        let running = match &mut self.view {
            View::Debug(debug) => debug.tick(),
            _ => false,
        };
        if running {
            Duration::from_millis(100)
        } else {
            Duration::from_secs(1)
        }
    }

    pub fn draw(&self, frame: &mut Frame) {
        match &self.view {
            View::Tabs => self.draw_tabs(frame),
            View::Yaml(yaml) => yaml.draw(self, frame),
            View::Debug(debug) => debug.draw(self, frame),
        }
    }

    fn draw_tabs(&self, frame: &mut Frame) {
        let [top, body, footer] = chrome::screen(frame);
        chrome::top(
            frame,
            top,
            vec![
                Span::styled(format!("{}  ", self.files.name()), theme::bold(theme::TEXT)),
                Span::styled(tilde(&self.files.settings.path), fg(theme::MUTED)),
            ],
            self.status(),
        );
        let tabs = Rect {
            y: body.y + 1,
            height: 2,
            ..body
        };
        let labels = ["1 Jobs", "2 Project", "3 Values", "4 Env file"].map(str::to_string);
        let selected = [Tab::Jobs, Tab::Project, Tab::Values, Tab::Env]
            .iter()
            .position(|tab| *tab == self.tab)
            .unwrap_or(0);
        chrome::tabs(frame, tabs, &labels, selected, Vec::new());
        let body = Rect {
            y: body.y + 3,
            height: body.height.saturating_sub(3),
            ..body
        };
        let keys = match self.tab {
            Tab::Jobs => {
                jobs_view::draw(self, frame, body);
                jobs_view::keys(self)
            }
            _ => {
                file_view::draw(self, frame, body);
                file_view::keys(self)
            }
        };
        self.footer(frame, footer, keys);
    }

    /// The top bar's right side: `● 2 unsaved  ✓ valid`.
    pub fn status(&self) -> Vec<Span<'static>> {
        let mut spans = Vec::new();
        let unsaved = self.files.unsaved();
        if unsaved > 0 {
            spans.push(Span::styled(
                format!("● {unsaved} unsaved   "),
                fg(theme::ACCENT),
            ));
        }
        spans.push(chrome::validity(self.problem.is_none()));
        spans
    }

    /// The footer: `keys`, with the message on the right.
    pub fn footer(&self, frame: &mut Frame, area: Rect, keys: Vec<Span<'static>>) {
        let message = self
            .message
            .iter()
            .map(|message| Span::styled(message.clone(), fg(theme::ACCENT)))
            .collect();
        chrome::bar(frame, area, keys, message);
    }

    /// `path` as shown: relative to the project's folder.
    pub fn shown_path(&self, path: &Path) -> String {
        path.strip_prefix(&self.files.root)
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| tilde(path))
    }

    /// The problem, if it is in job `name` or in no job.
    pub fn problem_in(&self, name: &str) -> Option<&Problem> {
        self.problem.as_ref().filter(|problem| problem.is_in(name))
    }

    /// Switches tab for keys `1` to `4`; returns whether `key` was one.
    pub fn switch_tab(&mut self, key: KeyEvent) -> bool {
        let tab = match key.code {
            KeyCode::Char('1') => Tab::Jobs,
            KeyCode::Char('2') => Tab::Project,
            KeyCode::Char('3') => Tab::Values,
            KeyCode::Char('4') => Tab::Env,
            _ => return false,
        };
        self.tab = tab;
        true
    }

    /// Starts a debug run of the selected job.
    pub fn debug(&mut self) {
        if self.files.jobs.is_empty() {
            return;
        }
        self.view = View::Debug(Box::new(Debug::start(&self.files, self.job)));
    }

    /// Opens the selected job's YAML: the task `part` (or its settings), or the whole file.
    pub fn open_yaml(&mut self, part: Option<usize>, whole: bool) {
        if self.files.jobs.is_empty() {
            return;
        }
        self.view = View::Yaml(JobYaml::new(self, self.job, part, whole));
    }
}
