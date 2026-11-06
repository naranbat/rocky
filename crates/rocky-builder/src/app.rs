//! The builder: the start screen until a project is open; then the top bar, sidebar, main
//! rows and status bar, the overlays, and the keys that drive them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use serde_yaml::Value;

use crate::autocomplete::Suggest;
use crate::complete::Var;
use crate::doc::{self, Doc};
use crate::edit::Edit;
use crate::form::{self, Action, Choice, Field, Row};
use crate::input::{self, Input};
use crate::job;
use crate::legacy;
use crate::node::{self, Path as NodePath};
use crate::project;
use crate::recent::Recent;
use crate::results::Shown;
use crate::run::Run;
use crate::run_view::{self, Output};
use crate::scheduler::{self, Scheduler};
use crate::scope;
use crate::sidebar::{self, Selection};
use crate::start::{self, Start};
use crate::task::{self, Kind};
use crate::theme;
use crate::validate::Validation;
use crate::yaml_view;

/// Which pane the keys go to.
#[derive(Clone, Copy, PartialEq)]
enum Pane {
    Sidebar,
    Main,
}

/// What the keys go to besides the panes: a text field or an overlay.
enum Mode {
    Normal,
    /// Typing the value of the node at `path` (with `field`), or its new name (`None`).
    /// An `added` node is removed again on Esc.
    Edit {
        input: Input,
        path: NodePath,
        field: Option<Field>,
        added: bool,
        suggest: Option<Suggest>,
    },
    Pick {
        title: String,
        choices: Vec<Choice>,
        cursor: usize,
    },
    Confirm {
        question: String,
        then: Confirmed,
    },
    Output(Output),
    Help,
}

/// What a confirmed question does.
enum Confirmed {
    DeleteJob(String),
    Quit,
}

pub struct App {
    doc: Option<Doc>,
    /// The start screen, shown while no project is open.
    start: Start,
    recent: Recent,
    selection: Selection,
    /// The open task, as the names leading to it: a task of the job, then of its `do`, ….
    trail: Vec<String>,
    pane: Pane,
    /// The focused row of the main pane, and how far the pane is scrolled.
    focus: usize,
    offset: usize,
    mode: Mode,
    validation: Validation,
    validate_now: bool,
    show_yaml: bool,
    yaml_scroll: u16,
    /// The env file's contents, re-read after each save or validate.
    previews: HashMap<PathBuf, String>,
    /// The last save or open message, for the status bar; `Err` in red.
    message: Option<Result<String, String>>,
    /// The latest run, finished or not.
    run: Option<Run>,
    /// The scheduler, while started from here, and the saved text it started with.
    scheduler: Option<(Scheduler, String)>,
    pub quit: bool,
}

impl App {
    pub fn new(path: Option<PathBuf>, recent: Recent) -> App {
        let mut app = App {
            doc: None,
            start: Start::default(),
            recent,
            selection: Selection::Project,
            trail: Vec::new(),
            pane: Pane::Sidebar,
            focus: 0,
            offset: 0,
            mode: Mode::Normal,
            validation: Validation::default(),
            validate_now: false,
            show_yaml: false,
            yaml_scroll: 0,
            previews: HashMap::new(),
            message: None,
            run: None,
            scheduler: None,
            quit: false,
        };
        if let Some(path) = path {
            app.open(&path);
        }
        app
    }

    fn open(&mut self, path: &Path) {
        match Doc::open(path) {
            Ok(doc) => {
                self.selection = doc
                    .job_names()
                    .first()
                    .map_or(Selection::Project, |name| Selection::Job(name.clone()));
                self.doc = Some(doc);
                self.trail.clear();
                self.pane = Pane::Sidebar;
                self.focus = 0;
                self.offset = 0;
                self.validation = Validation::default();
                self.previews.clear();
                self.message = None;
                self.run = None;
                self.scheduler = None;
                if let Err(error) = self.recent.record(path) {
                    self.message = Some(Err(error));
                }
            }
            Err(error) => self.message = Some(Err(format!("{error:#}"))),
        }
    }

    /// Takes in what the run and the scheduler reported, and validates after edits. Returns
    /// how long to wait for a key before calling again.
    pub fn tick(&mut self) -> Duration {
        let mut wait = Duration::from_secs(1);
        if let Some(run) = &mut self.run {
            run.poll();
            if run.is_running() {
                wait = Duration::from_millis(100);
            }
        }
        if let Some((scheduler, _)) = &mut self.scheduler {
            scheduler.poll();
            if scheduler.is_running() {
                // New log lines, and the next-run countdown.
                wait = wait.min(Duration::from_millis(500));
            }
        }
        if let Some(doc) = &self.doc {
            let text = doc.text().unwrap_or_default();
            let now = std::mem::take(&mut self.validate_now);
            if now {
                self.previews.clear();
            }
            if let Some(pending) = self.validation.update(&doc.path, &text, now) {
                wait = wait.min(pending);
            }
        }
        wait
    }

    fn job(&self) -> Option<&str> {
        match &self.selection {
            Selection::Job(name) => Some(name),
            Selection::Project => None,
        }
    }

    /// The task list that holds the last task of `trail` in `job`.
    fn tasks_path(job: &str, trail: &[String]) -> NodePath {
        trail
            .iter()
            .fold(job::path(job).key("tasks"), |path, name| {
                path.key(name).key("do")
            })
    }

    /// Drops the end of the trail where its tasks no longer exist.
    fn fix_trail(&mut self) {
        let (Some(doc), Some(job)) = (&self.doc, self.job()) else {
            self.trail.clear();
            return;
        };
        let mut keep = 0;
        while keep < self.trail.len() {
            let path = App::tasks_path(job, &self.trail[..keep]).key(&self.trail[keep]);
            if node::get(&doc.root, &path).is_none() {
                break;
            }
            keep += 1;
        }
        self.trail.truncate(keep);
    }

    /// The main pane's rows for what is selected.
    fn rows(&mut self) -> Vec<Row> {
        self.fix_trail();
        let Some(doc) = &self.doc else {
            return Vec::new();
        };
        let job = self.job().unwrap_or_default().to_string();
        let jobs = doc.job_names();
        let scheduler = self.scheduler.as_ref().map(|(s, _)| s);
        let mut ed = Edit {
            jobs: &jobs,
            job: &job,
            run: shown(self.run.as_ref(), scheduler, &job),
            problem: self.validation.problem(),
        };
        match (&self.selection, self.trail.split_last()) {
            (Selection::Project, _) => project::rows(&doc.root),
            (Selection::Job(job), None) => job::rows(&mut ed, &doc.root, job),
            (Selection::Job(job), Some((name, outer))) => {
                let tasks = App::tasks_path(job, outer);
                task::rows(&mut ed, &doc.root, &tasks, name)
            }
        }
    }

    fn select(&mut self, selection: Selection) {
        self.selection = selection;
        self.trail.clear();
        self.focus = 0;
        self.offset = 0;
    }

    pub fn handle(&mut self, key: KeyEvent) {
        if self.doc.is_none() {
            self.handle_start(key);
            return;
        }
        match std::mem::replace(&mut self.mode, Mode::Normal) {
            Mode::Normal => self.handle_normal(key),
            Mode::Edit {
                mut input,
                path,
                field,
                added,
                mut suggest,
            } => {
                if suggest
                    .as_mut()
                    .is_some_and(|suggest| suggest.handle(&mut input, key))
                {
                    self.mode = Mode::Edit {
                        input,
                        path,
                        field,
                        added,
                        suggest,
                    };
                    return;
                }
                let before = input.text.clone();
                match input.handle(key) {
                    input::Outcome::Done => {
                        if let Err(error) = self.commit(&path, field, &input.text) {
                            self.message = Some(Err(error));
                            self.mode = Mode::Edit {
                                input,
                                path,
                                field,
                                added,
                                suggest,
                            };
                        }
                    }
                    input::Outcome::Cancel => {
                        if added && let Some(doc) = &mut self.doc {
                            node::set(&mut doc.root, &path, None);
                        }
                    }
                    input::Outcome::Edited => {
                        if let Some(suggest) = &mut suggest
                            && input.text != before
                        {
                            suggest.edited();
                        }
                        self.mode = Mode::Edit {
                            input,
                            path,
                            field,
                            added,
                            suggest,
                        }
                    }
                }
            }
            Mode::Pick {
                title,
                choices,
                mut cursor,
            } => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    cursor = cursor.saturating_sub(1);
                    self.mode = Mode::Pick {
                        title,
                        choices,
                        cursor,
                    };
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    cursor = (cursor + 1).min(choices.len().saturating_sub(1));
                    self.mode = Mode::Pick {
                        title,
                        choices,
                        cursor,
                    };
                }
                KeyCode::Enter => {
                    if let Some(choice) = choices.into_iter().nth(cursor) {
                        self.act(choice.action);
                    }
                }
                KeyCode::Esc | KeyCode::Char('q') => {}
                _ => {
                    self.mode = Mode::Pick {
                        title,
                        choices,
                        cursor,
                    }
                }
            },
            Mode::Confirm { question, then } => match key.code {
                KeyCode::Char('y') | KeyCode::Enter => match then {
                    Confirmed::Quit => self.quit = true,
                    Confirmed::DeleteJob(name) => {
                        if let Some(doc) = &mut self.doc {
                            doc::set(doc.jobs_mut(), &name, None);
                            let first = doc.job_names().into_iter().next();
                            self.select(first.map_or(Selection::Project, Selection::Job));
                        }
                    }
                },
                KeyCode::Char('n') | KeyCode::Esc => {}
                _ => self.mode = Mode::Confirm { question, then },
            },
            Mode::Output(mut output) => {
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('o') => return,
                    KeyCode::Tab | KeyCode::Right => output.tab = (output.tab + 1) % 3,
                    KeyCode::BackTab | KeyCode::Left => output.tab = (output.tab + 2) % 3,
                    KeyCode::Up | KeyCode::Char('k') => {
                        output.scroll = output.scroll.saturating_sub(1)
                    }
                    KeyCode::Down | KeyCode::Char('j') => output.scroll += 1,
                    KeyCode::PageUp => output.scroll = output.scroll.saturating_sub(20),
                    KeyCode::PageDown => output.scroll += 20,
                    _ => {}
                }
                self.mode = Mode::Output(output);
            }
            Mode::Help => {}
        }
    }

    fn handle_start(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        let picked = self.start.handle(key, &self.recent);
        let saved = match picked {
            Some(start::Pick::Open(path)) => {
                self.open(&path);
                Ok(())
            }
            Some(start::Pick::Created(path)) => {
                self.message = Some(Ok(format!("created {}", path.display())));
                self.recent.record(&path)
            }
            Some(start::Pick::Forget(path)) => self.recent.remove(&path),
            Some(start::Pick::Refresh) => {
                self.recent.refresh();
                Ok(())
            }
            Some(start::Pick::Quit) => {
                self.quit = true;
                Ok(())
            }
            None => Ok(()),
        };
        if let Err(error) = saved {
            self.message = Some(Err(error));
        }
    }

    fn handle_normal(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('s') if ctrl => return self.save(),
            KeyCode::Char('c') if ctrl => {
                match self.run.as_mut().filter(|run| run.is_running()) {
                    Some(run) => run.stop(),
                    None => self.ask_quit(),
                }
                return;
            }
            KeyCode::Char('q') => return self.ask_quit(),
            KeyCode::Char('?') => {
                self.mode = Mode::Help;
                return;
            }
            KeyCode::Char('y') => {
                self.show_yaml = !self.show_yaml;
                self.yaml_scroll = 0;
                return;
            }
            KeyCode::Char('R') => return self.start_run(None),
            KeyCode::Char('S') => return self.toggle_scheduler(),
            KeyCode::Char('V') => {
                self.validate_now = true;
                return;
            }
            KeyCode::Tab => {
                self.pane = match self.pane {
                    Pane::Sidebar => Pane::Main,
                    Pane::Main => Pane::Sidebar,
                };
                return;
            }
            _ => {}
        }
        match self.pane {
            Pane::Sidebar => self.handle_sidebar(key),
            Pane::Main if self.show_yaml => self.handle_yaml(key),
            Pane::Main => self.handle_main(key),
        }
    }

    fn handle_sidebar(&mut self, key: KeyEvent) {
        let job = self.job().map(str::to_string);
        let Some(doc) = &mut self.doc else {
            return;
        };
        let items = sidebar::items(doc);
        let at = items
            .iter()
            .position(|item| *item == self.selection)
            .unwrap_or(0);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(item) = at.checked_sub(1).and_then(|at| items.get(at)) {
                    self.select(item.clone());
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(item) = items.get(at + 1) {
                    self.select(item.clone());
                }
            }
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => self.pane = Pane::Main,
            KeyCode::Char('a') => {
                let jobs = doc.jobs_mut();
                let name = doc::fresh_key(jobs, "new_job");
                jobs.insert(Value::from(name.as_str()), sidebar::new_job());
                doc.sort_jobs();
                self.select(Selection::Job(name.clone()));
                self.rename(job::path(&name));
            }
            KeyCode::Char('x') | KeyCode::Delete => {
                if let Some(job) = job {
                    self.mode = Mode::Confirm {
                        question: format!("Delete job `{job}`?"),
                        then: Confirmed::DeleteJob(job),
                    };
                }
            }
            KeyCode::Char('r') => {
                if let Some(job) = job {
                    self.rename(job::path(&job));
                }
            }
            _ => {}
        }
    }

    fn handle_yaml(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.yaml_scroll = self.yaml_scroll.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('j') => self.yaml_scroll += 1,
            KeyCode::PageUp => self.yaml_scroll = self.yaml_scroll.saturating_sub(20),
            KeyCode::PageDown => self.yaml_scroll += 20,
            KeyCode::Esc => self.show_yaml = false,
            _ => {}
        }
    }

    fn handle_main(&mut self, key: KeyEvent) {
        let rows = self.rows();
        self.focus = form::settle(&rows, self.focus);
        let Some(row) = rows.get(self.focus) else {
            if matches!(key.code, KeyCode::Esc | KeyCode::Left | KeyCode::Char('h')) {
                self.back();
            }
            return;
        };
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.focus = form::step(&rows, self.focus, -1),
            KeyCode::Down | KeyCode::Char('j') => self.focus = form::step(&rows, self.focus, 1),
            KeyCode::PageUp => {
                self.focus = (0..10).fold(self.focus, |at, _| form::step(&rows, at, -1))
            }
            KeyCode::PageDown => {
                self.focus = (0..10).fold(self.focus, |at, _| form::step(&rows, at, 1))
            }
            KeyCode::Home => self.focus = form::settle(&rows, 0),
            KeyCode::End => self.focus = form::settle(&rows, rows.len()),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => self.act(row.action.clone()),
            KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') | KeyCode::Backspace => self.back(),
            KeyCode::Char('x') | KeyCode::Delete => {
                if let (Some(path), Some(doc)) = (&row.remove, &mut self.doc) {
                    node::set(&mut doc.root, path, None);
                }
            }
            KeyCode::Char('J') | KeyCode::Char('K') => {
                let by = if key.code == KeyCode::Char('J') {
                    1
                } else {
                    -1
                };
                if let (Some(path), Some(doc)) = (&row.shift, &mut self.doc)
                    && let Some(moved) = node::shift(&mut doc.root, path, by)
                {
                    let rows = self.rows();
                    if let Some(at) = rows
                        .iter()
                        .position(|row| row.shift.as_ref() == Some(&moved))
                    {
                        self.focus = at;
                    }
                }
            }
            KeyCode::Char('r') => {
                if let Some(path) = row.rename.clone() {
                    self.rename(path);
                }
            }
            KeyCode::Char('U') => {
                let task = self.trail.first().cloned().or_else(|| row.task.clone());
                if let Some(task) = task {
                    self.start_run(Some(task));
                }
            }
            KeyCode::Char('o') => {
                let task = row.task.clone().or_else(|| self.trail.last().cloned());
                if let Some(task) = task {
                    self.act(Action::Output(task));
                }
            }
            _ => {}
        }
    }

    /// Leaves the open task for the list it is in, or the main pane for the sidebar.
    fn back(&mut self) {
        match self.trail.pop() {
            Some(name) => {
                let rows = self.rows();
                self.focus = rows
                    .iter()
                    .position(|row| matches!(&row.action, Action::Open(open) if *open == name))
                    .unwrap_or(0);
                self.offset = 0;
            }
            None => self.pane = Pane::Sidebar,
        }
    }

    /// Starts editing the name of the entry at `path`.
    fn rename(&mut self, path: NodePath) {
        let name = path.last_key().unwrap_or_default().to_string();
        self.mode = Mode::Edit {
            input: Input::new(&name),
            path,
            field: None,
            added: false,
            suggest: None,
        };
    }

    /// Suggestions for a field: the variables a `${…}` can read where it is.
    fn suggest(&mut self, field: Option<Field>) -> Option<Suggest> {
        let mode = field?.completion()?;
        Some(Suggest::new(self.scope(), mode))
    }

    /// The variables a field of what is shown can read: the project's and job's values and
    /// env names, what each task before the open one left in `tasks.NAME`, and the items of
    /// the loops it is in.
    fn scope(&mut self) -> Vec<Var> {
        let Some(doc) = &self.doc else {
            return Vec::new();
        };
        let job = self.job().unwrap_or_default().to_string();
        let mut vars = scope::base(&doc.root, &job, doc.dir(), &mut self.previews);
        let scheduler = self.scheduler.as_ref().map(|(s, _)| s);
        let run = shown(self.run.as_ref(), scheduler, &job);
        for (at, open) in self.trail.iter().enumerate() {
            let list = App::tasks_path(&job, &self.trail[..at]);
            let tasks = node::get(&doc.root, &list).and_then(Value::as_mapping);
            for (key, task) in tasks.into_iter().flatten() {
                let name = doc::key_text(key);
                if name == *open {
                    if at + 1 < self.trail.len() && Kind::of(task) == Some(Kind::ForEach) {
                        let item = task.get("as").and_then(Value::as_str).unwrap_or("ITEM");
                        vars.push(scope::loop_item(item));
                    }
                    break;
                }
                let result = run
                    .and_then(|run| run.results.task(&job, &name))
                    .and_then(|task| task.last.as_ref())
                    .map(|event| &event["result"]);
                vars.extend(scope::task(&name, task, result));
            }
        }
        vars
    }

    /// Does what a row or a choice asks.
    fn act(&mut self, action: Action) {
        let Some(doc) = &mut self.doc else {
            return;
        };
        match action {
            Action::None => {}
            Action::Edit(path, field) => {
                let text = field.text(node::get(&doc.root, &path));
                self.mode = Mode::Edit {
                    input: Input::new(&text),
                    path,
                    field: Some(field),
                    added: false,
                    suggest: self.suggest(Some(field)),
                };
            }
            Action::Rename(path) => self.rename(path),
            Action::Set(path, value) => {
                node::set(&mut doc.root, &path, value);
            }
            Action::Add(path, value, then) => {
                if node::set(&mut doc.root, &path, Some(value)) {
                    let text = match then {
                        Some(field) => field.text(node::get(&doc.root, &path)),
                        None => path.last_key().unwrap_or_default().to_string(),
                    };
                    self.mode = Mode::Edit {
                        input: Input::new(&text),
                        path,
                        field: then,
                        added: true,
                        suggest: self.suggest(then),
                    };
                }
            }
            Action::Pick(title, choices) => {
                self.mode = Mode::Pick {
                    title,
                    choices,
                    cursor: 0,
                }
            }
            Action::Open(name) => {
                self.trail.push(name);
                self.focus = 0;
                self.offset = 0;
            }
            Action::Output(task) => {
                self.mode = Mode::Output(Output {
                    task,
                    tab: 0,
                    scroll: 0,
                })
            }
            Action::Convert(path, name) => {
                if let Some(tasks) = node::map_mut(&mut doc.root, &path) {
                    legacy::convert(tasks, &name);
                }
            }
        }
    }

    /// Writes typed `text` to the node at `path`: its value through `field`, or with `None`
    /// its name.
    fn commit(&mut self, path: &NodePath, field: Option<Field>, text: &str) -> Result<(), String> {
        let Some(doc) = &mut self.doc else {
            return Ok(());
        };
        let Some(field) = field else {
            let new = text.trim();
            if path.last_key() == Some(new) {
                return Ok(());
            }
            let is_job = *path == job::path(path.last_key().unwrap_or_default());
            if is_job {
                rocky_core::project::check_job_name(new).map_err(|error| error.to_string())?;
            }
            let renamed = node::rename(&mut doc.root, path, new)
                .ok_or_else(|| format!("`{new}` is empty or already used"))?;
            if is_job {
                doc.sort_jobs();
            }
            self.renamed(path, &renamed);
            return Ok(());
        };
        let value = field.parse(text)?;
        node::set(&mut doc.root, path, value);
        Ok(())
    }

    /// Follows a rename of the job or a task on the trail.
    fn renamed(&mut self, old: &NodePath, new: &NodePath) {
        let Some(name) = new.last_key().map(str::to_string) else {
            return;
        };
        let Selection::Job(job) = self.selection.clone() else {
            return;
        };
        if *old == job::path(&job) {
            self.selection = Selection::Job(name);
            return;
        }
        for at in 0..self.trail.len() {
            if App::tasks_path(&job, &self.trail[..at]).key(&self.trail[at]) == *old {
                self.trail[at] = name;
                return;
            }
        }
    }

    fn save(&mut self) {
        let Some(doc) = &mut self.doc else {
            return;
        };
        let saved = doc
            .text()
            .and_then(|text| doc.save(text))
            .map(|()| format!("saved {}", doc.path.display()))
            .map_err(|error| format!("{error:#}"));
        self.message = Some(saved);
        self.validate_now = true;
    }

    fn ask_quit(&mut self) {
        let dirty = self
            .doc
            .as_ref()
            .is_some_and(|doc| doc.text().is_ok_and(|text| doc.is_dirty(&text)));
        if dirty {
            self.mode = Mode::Confirm {
                question: "Quit without saving?".to_string(),
                then: Confirmed::Quit,
            };
        } else {
            self.quit = true;
        }
    }

    /// Runs the selected job (up to task `until`) as shown, unsaved edits included; while
    /// the scheduler is up, a whole-job run is queued on it instead, so runs never overlap.
    fn start_run(&mut self, until: Option<String>) {
        let (Some(doc), Selection::Job(job)) = (&self.doc, &self.selection) else {
            self.message = Some(Err("select a job to run".to_string()));
            return;
        };
        if self.run.as_ref().is_some_and(Run::is_running) {
            self.message = Some(Err("a run is going; ctrl-c stops it".to_string()));
            return;
        }
        if !self.validation.is_valid() {
            self.message = Some(Err("fix the errors before running".to_string()));
            return;
        }
        let scheduler = self
            .scheduler
            .as_ref()
            .map(|(s, _)| s)
            .filter(|s| s.is_running());
        match (until, scheduler) {
            (None, Some(scheduler)) => {
                self.message = Some(
                    scheduler::trigger(&scheduler.socket, job)
                        .map(|()| format!("queued {job} on the scheduler")),
                );
            }
            (until, _) => match doc.text() {
                Ok(text) => self.run = Some(Run::start(&doc.path, &text, job, until.as_deref())),
                Err(error) => self.message = Some(Err(format!("{error:#}"))),
            },
        }
    }

    fn toggle_scheduler(&mut self) {
        let Some(doc) = &self.doc else {
            return;
        };
        if let Some((scheduler, _)) = self.scheduler.as_mut().filter(|(s, _)| s.is_running()) {
            scheduler.stop();
            return;
        }
        let started = scheduler::find_rocky()
            .ok_or_else(|| {
                "can't find the `rocky` binary next to the builder or on PATH; \
                 install it with `cargo install --path crates/rocky`"
                    .to_string()
            })
            .and_then(|rocky| {
                Scheduler::start(&rocky, &doc.path).map_err(|error| format!("{error:#}"))
            });
        match started {
            Ok(started) => {
                self.scheduler = Some((started, doc.saved().to_string()));
                self.message = Some(Ok("scheduler started".to_string()));
            }
            Err(error) => self.message = Some(Err(error)),
        }
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        if self.doc.is_none() {
            let error = match &self.message {
                Some(Err(error)) => Some(error.as_str()),
                _ => None,
            };
            self.start.draw(frame, area, &self.recent, error);
            return;
        }
        let rows = self.rows();
        self.focus = form::settle(&rows, self.focus);
        let Some(doc) = &self.doc else {
            return;
        };
        let text = doc
            .text()
            .unwrap_or_else(|error| format!("# cannot write YAML: {error}"));
        let problem = self.validation.problem();
        let error_height = u16::from(problem.is_some()) * 2;
        let [top, errors, body, status, keys] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(error_height),
            Constraint::Fill(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(area);
        frame.render_widget(Paragraph::new(self.top_bar(doc, &text)).bg(theme::BAR), top);
        if let Some(problem) = problem {
            frame.render_widget(
                Paragraph::new(Line::styled(
                    format!("✗ {}", problem.message),
                    theme::error(),
                ))
                .wrap(Wrap { trim: true }),
                errors,
            );
        }
        let [side, main] =
            Layout::horizontal([Constraint::Length(34), Constraint::Fill(1)]).areas(body);
        let scheduler = self.scheduler.as_ref().map(|(s, _)| s);
        sidebar::draw(
            frame,
            side,
            doc,
            &self.selection,
            self.pane == Pane::Sidebar,
            problem,
            scheduler,
        );
        let main = main.inner(ratatui::layout::Margin::new(1, 0));
        if self.show_yaml {
            let job = match &self.selection {
                Selection::Project => None,
                Selection::Job(job) => Some(job.as_str()),
            };
            let file = yaml_view::file(&text, job);
            yaml_view::draw(frame, main, &file, &mut self.yaml_scroll);
        } else {
            let editing = match &self.mode {
                Mode::Edit { input, .. } if self.pane == Pane::Main => Some(input),
                _ => None,
            };
            let focus = (self.pane == Pane::Main && !rows.is_empty()).then_some(self.focus);
            let field = form::draw(frame, main, &rows, focus, &mut self.offset, editing);
            if let (
                Some(field),
                Mode::Edit {
                    input,
                    suggest: Some(suggest),
                    ..
                },
            ) = (field, &self.mode)
            {
                suggest.draw(frame, input, field, main);
            }
        }
        frame.render_widget(Paragraph::new(self.status_bar(doc)), status);
        frame.render_widget(Paragraph::new(self.key_hints()), keys);
        self.draw_overlay(frame, body);
    }

    fn top_bar(&self, doc: &Doc, text: &str) -> Line<'static> {
        let file = file_name(&doc.path);
        let mut crumb = match &self.selection {
            Selection::Project => format!("{file} / project"),
            Selection::Job(name) => format!("{file} / jobs / {name}"),
        };
        for name in &self.trail {
            crumb.push_str(&format!(" / {name}"));
        }
        let mut spans = vec![
            Span::styled(" rocky ", theme::bold()),
            Span::styled(crumb, theme::dim()),
        ];
        if doc.is_dirty(text) {
            spans.push(Span::styled(" ● unsaved", theme::accent()));
        }
        spans.push(Span::raw("   "));
        spans.push(match &self.validation.result {
            Some(Ok(())) => Span::styled("✓ VALID", theme::ok()),
            Some(Err(_)) => Span::styled("✗ INVALID", theme::error()),
            None => Span::styled("CHECKING", theme::dim()),
        });
        if self.run.as_ref().is_some_and(Run::is_running) {
            spans.push(Span::styled("   ● running · ctrl-c stops", theme::accent()));
        }
        if self.show_yaml {
            spans.push(Span::styled("   YAML view · y closes", theme::accent()));
        }
        Line::from(spans)
    }

    /// The scheduler's state, the next job due, and the last message.
    fn status_bar(&self, doc: &Doc) -> Line<'static> {
        let mut spans = Vec::new();
        match self.scheduler.as_ref().filter(|(s, _)| s.is_running()) {
            Some((scheduler, started_with)) => {
                spans.push(Span::styled(" ● Scheduler running", theme::ok()));
                spans.push(Span::styled(
                    format!(
                        " · {} scheduled jobs · log {}",
                        scheduler.scheduled(),
                        scheduler.log
                    ),
                    theme::dim(),
                ));
                let now = jiff::Zoned::now();
                if let Some((job, at)) = scheduler.next_due(&now) {
                    let wait = Duration::try_from(now.duration_until(&at)).unwrap_or_default();
                    spans.push(Span::styled(
                        format!(" · next: {job} in {}", countdown(wait)),
                        theme::dim(),
                    ));
                }
                if doc.saved() != started_with {
                    spans.push(Span::styled(
                        " · saved since start: tasks apply on the next run, schedules after a restart",
                        theme::accent(),
                    ));
                }
            }
            None => {
                spans.push(Span::styled(" ○ Scheduler stopped", theme::dim()));
                if let Some(exited) = self
                    .scheduler
                    .as_ref()
                    .and_then(|(scheduler, _)| scheduler.exited.as_deref())
                    .filter(|exited| *exited != "stopped")
                {
                    spans.push(Span::styled(format!(" · {exited}"), theme::error()));
                }
            }
        }
        match &self.message {
            Some(Ok(text)) => spans.push(Span::styled(format!(" · {text}"), theme::dim())),
            Some(Err(text)) => spans.push(Span::styled(format!(" · {text}"), theme::error())),
            None => {}
        }
        Line::from(spans)
    }

    fn key_hints(&self) -> Line<'static> {
        let hints = match (&self.mode, self.pane) {
            (Mode::Edit { .. }, _) => "enter keep · esc cancel · ←→ home end move · ctrl-u clear",
            (Mode::Pick { .. }, _) => "↑↓ choose · enter pick · esc cancel",
            (Mode::Confirm { .. }, _) => "y yes · n no",
            (Mode::Output(_), _) => "tab switch · ↑↓ scroll · esc close",
            (Mode::Help, _) => "any key closes",
            (Mode::Normal, Pane::Sidebar) => {
                "↑↓ select · enter edit · a add job · r rename · x delete · tab pane · R run · S scheduler · ctrl-s save · ? help · q quit"
            }
            (Mode::Normal, Pane::Main) => {
                "↑↓ move · enter edit/open · esc back · x remove · r rename · J/K move · R run · U run to here · o output · y yaml · ctrl-s save · ? help"
            }
        };
        Line::styled(format!(" {hints}"), theme::dim())
    }

    fn draw_overlay(&self, frame: &mut Frame, area: Rect) {
        match &self.mode {
            Mode::Normal | Mode::Edit { .. } => {}
            Mode::Pick {
                title,
                choices,
                cursor,
            } => {
                let height = u16::try_from(choices.len() + 2).unwrap_or(u16::MAX);
                let box_area = centered(area, 72, height);
                let width = choices
                    .iter()
                    .map(|choice| choice.label.chars().count() + 2)
                    .max()
                    .unwrap_or(0);
                let lines: Vec<Line> = choices
                    .iter()
                    .enumerate()
                    .map(|(index, choice)| {
                        let on = index == *cursor;
                        Line::from(vec![
                            Span::styled(if on { "› " } else { "  " }, theme::accent()),
                            Span::styled(
                                format!("{:<width$}", choice.label),
                                if on { theme::accent() } else { theme::text() },
                            ),
                            Span::styled(choice.detail.clone(), theme::dim()),
                        ])
                    })
                    .collect();
                frame.render_widget(Clear, box_area);
                frame.render_widget(
                    Paragraph::new(lines).block(
                        Block::bordered()
                            .title(Span::styled(format!(" {title} "), theme::bold()))
                            .border_style(theme::accent()),
                    ),
                    box_area,
                );
            }
            Mode::Confirm { question, .. } => {
                let box_area = centered(area, 48, 3);
                frame.render_widget(Clear, box_area);
                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(question.clone(), theme::text()),
                        Span::styled("  y / n", theme::accent()),
                    ]))
                    .block(Block::bordered().border_style(theme::accent())),
                    box_area,
                );
            }
            Mode::Output(output) => {
                let job = self.job().unwrap_or_default();
                let scheduler = self.scheduler.as_ref().map(|(s, _)| s);
                let run = shown(self.run.as_ref(), scheduler, job);
                let box_area = centered(
                    area,
                    area.width.saturating_sub(8),
                    area.height.saturating_sub(2),
                );
                run_view::draw_output(frame, box_area, run, job, output);
            }
            Mode::Help => {
                let lines: Vec<Line> = HELP
                    .iter()
                    .map(|(keys, what)| {
                        Line::from(vec![
                            Span::styled(format!("{keys:<14}"), theme::accent()),
                            Span::styled(*what, theme::text()),
                        ])
                    })
                    .collect();
                let height = u16::try_from(lines.len() + 2).unwrap_or(u16::MAX);
                let box_area = centered(area, 72, height);
                frame.render_widget(Clear, box_area);
                frame.render_widget(
                    Paragraph::new(lines).block(
                        Block::bordered()
                            .title(Span::styled(" Keys ", theme::bold()))
                            .border_style(theme::accent()),
                    ),
                    box_area,
                );
            }
        }
    }
}

const HELP: [(&str, &str); 18] = [
    ("tab", "switch between the sidebar and the editor"),
    ("↑ ↓  j k", "move"),
    ("enter  →", "edit a field, open a task, or pick"),
    ("esc  ←", "leave a task, or go back to the sidebar"),
    ("x  del", "remove the field, entry or task"),
    ("J  K", "move the entry or task down / up"),
    ("r", "rename the job, task or variable"),
    ("a", "add a job (sidebar)"),
    ("R", "run the job, unsaved edits included"),
    ("U", "run the job up to the focused task"),
    ("ctrl-c", "stop the run; with none, quit"),
    ("o", "the output of a task's last run"),
    ("S", "start or stop the scheduler (rocky run)"),
    ("y", "show the file Save writes: the job's, or rocky.yaml"),
    ("V", "validate now"),
    ("ctrl-s", "save"),
    ("?", "this help"),
    ("q", "quit"),
];

/// A `width` × `height` box in the middle of `area`.
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

/// `2m 41s`, `1h 5m`, `12s`.
fn countdown(wait: Duration) -> String {
    let secs = wait.as_secs();
    match (secs / 3600, secs % 3600 / 60, secs % 60) {
        (0, 0, s) => format!("{s}s"),
        (0, m, s) => format!("{m}m {s}s"),
        (h, m, _) => format!("{h}h {m}m"),
    }
}

/// The run whose results the rows of `job` show: the newer of the builder's latest run and
/// the scheduler's latest run of `job`.
fn shown<'a>(
    run: Option<&'a Run>,
    scheduler: Option<&'a Scheduler>,
    job: &str,
) -> Option<Shown<'a>> {
    let local = run.map(Run::shown).filter(|run| run.covers(job));
    let scheduled = scheduler.and_then(|scheduler| scheduler.shown(job));
    match (local, scheduled) {
        (Some(local), Some(scheduled)) => Some(if local.started >= scheduled.started {
            local
        } else {
            scheduled
        }),
        (local, scheduled) => local.or(scheduled),
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::KeyEvent;

    use super::*;

    fn terminal() -> Terminal<TestBackend> {
        Terminal::new(TestBackend::new(140, 50)).unwrap()
    }

    fn draw(app: &mut App, terminal: &mut Terminal<TestBackend>) -> String {
        app.tick();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        format!("{:?}", terminal.backend().buffer())
    }

    fn press(app: &mut App, code: KeyCode) {
        app.handle(KeyEvent::from(code));
    }

    fn ctrl(app: &mut App, c: char) {
        app.handle(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
    }

    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    /// Focuses the first row of the main pane that `pick` accepts.
    fn focus(app: &mut App, pick: impl Fn(&Row) -> bool) {
        app.pane = Pane::Main;
        let rows = app.rows();
        app.focus = rows.iter().position(pick).expect("no such row");
    }

    /// Writes the project tree `yaml` into `dir` as its `rocky.yaml` and job files.
    fn write_project(dir: &Path, yaml: &str) {
        crate::parts::write(dir, "", yaml).unwrap();
    }

    fn label_is(name: &str) -> impl Fn(&Row) -> bool + '_ {
        move |row| row.label.content.trim() == name
    }

    /// Draws the open task, then every task nested in it.
    fn view_tasks(app: &mut App, terminal: &mut Terminal<TestBackend>) {
        let rows = app.rows();
        for row in rows {
            if let Action::Open(name) = row.action {
                app.trail.push(name);
                draw(app, terminal);
                view_tasks(app, terminal);
                app.trail.pop();
            }
        }
    }

    /// Draws every view (the project, each job and each task in it, and the YAML) of the
    /// project at `path`, and checks that just looking at it leaves the document unchanged
    /// and valid.
    fn view_all(path: &Path) {
        let mut app = App::new(Some(path.to_path_buf()), Recent::none());
        let mut terminal = terminal();
        let doc = app.doc.as_ref().unwrap();
        let before = doc.text().unwrap();
        for view in sidebar::items(doc) {
            app.select(view);
            for yaml in [false, true] {
                app.show_yaml = yaml;
                draw(&mut app, &mut terminal);
            }
            view_tasks(&mut app, &mut terminal);
        }
        let after = app.doc.as_ref().unwrap().text().unwrap();
        assert_eq!(before, after, "{}", path.display());
        assert!(
            app.validation.is_valid(),
            "{}: {:?}",
            path.display(),
            app.validation.result
        );
    }

    #[test]
    fn viewing_every_example_changes_nothing() {
        let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let mut viewed = 0;
        for entry in std::fs::read_dir(examples).unwrap() {
            let path = entry.unwrap().path();
            if path.join("jobs").is_dir() {
                view_all(&path);
                viewed += 1;
            }
        }
        assert!(viewed >= 5);
    }

    /// Without a project, the start screen draws, and a path that doesn't exist shows why
    /// it can't be opened.
    #[test]
    fn start_screen_reports_a_bad_path() {
        let mut app = App::new(Some(PathBuf::from("/no/such/project")), Recent::none());
        let screen = draw(&mut app, &mut terminal());
        assert!(app.doc.is_none());
        assert!(matches!(&app.message, Some(Err(error)) if error.contains("no/such")));
        assert!(screen.contains("no/such"));
    }

    /// An old-syntax task is invalid, flagged on its row, and shown with its fix; viewing
    /// it changes nothing, and Convert fixes it.
    #[test]
    fn draws_old_syntax_with_its_problem() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "jobs:\n  j:\n    tasks:\n      route: {if: {this: A, eq: 1}, then: {b: {cmd: [echo]}}}\n";
        write_project(dir.path(), yaml);
        let mut app = App::new(Some(dir.path().to_path_buf()), Recent::none());
        let mut terminal = terminal();
        assert!(draw(&mut app, &mut terminal).contains("problem"));
        let problem = app.validation.problem().unwrap();
        assert!(problem.is_on("j", "route"));
        app.trail.push("route".to_string());
        assert!(draw(&mut app, &mut terminal).contains("Convert to when"));
        let doc = app.doc.as_ref().unwrap();
        assert!(!doc.is_dirty(&doc.text().unwrap()));
        focus(&mut app, |row| matches!(row.action, Action::Convert(..)));
        press(&mut app, KeyCode::Enter);
        app.validate_now = true;
        draw(&mut app, &mut terminal);
        assert!(app.validation.is_valid(), "{:?}", app.validation.result);
    }

    /// Draws a job while it runs and after, with results on every row and the output pane.
    #[test]
    fn draws_a_run_to_the_end() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "\
jobs:
  main:
    tasks:
      list: {cmd: [echo, '[1, 2]']}
      each:
        for_each: tasks.list.stdout
        do:
          show: {cmd: [echo, '${ITEM}']}
      fail: {cmd: [sh, -c, 'echo oops >&2; exit 3'], allow_failure: true}
      never: {cmd: [echo], when: {this: tasks.fail.ok, eq: true}}
      page: {trigger: other}
  other:
    tasks:
      only: {cmd: [echo, done]}
";
        write_project(dir.path(), yaml);
        let mut app = App::new(Some(dir.path().to_path_buf()), Recent::none());
        let mut terminal = terminal();
        app.select(Selection::Job("main".to_string()));
        draw(&mut app, &mut terminal);
        press(&mut app, KeyCode::Char('R'));
        assert!(app.run.is_some(), "{:?}", app.message);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while app.run.as_ref().is_some_and(Run::is_running) && std::time::Instant::now() < deadline
        {
            draw(&mut app, &mut terminal);
            std::thread::sleep(Duration::from_millis(20));
        }
        let screen = draw(&mut app, &mut terminal);
        assert!(screen.contains("LAST RUN OK"), "{screen}");
        assert!(screen.contains("skipped"));
        let results = &app.run.as_ref().unwrap().results;
        assert!(matches!(
            results.end,
            Some(crate::results::End::Done { failed: None })
        ));
        assert_eq!(results.task("main", "show").unwrap().runs, 2);
        assert_eq!(results.task("main", "fail").unwrap().failures, 1);
        assert!(results.task("main", "never").unwrap().skipped);
        assert!(results.task("other", "only").is_some());
        focus(&mut app, |row| row.task.as_deref() == Some("fail"));
        press(&mut app, KeyCode::Char('o'));
        press(&mut app, KeyCode::Tab);
        assert!(draw(&mut app, &mut terminal).contains("oops"));
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(
            app.doc.as_ref().unwrap().text().unwrap(),
            serde_yaml::to_string(&serde_yaml::from_str::<serde_yaml::Value>(yaml).unwrap())
                .unwrap()
        );
    }

    /// Covers what the examples don't: while, any, exists, weekly and at schedules,
    /// defaults, capture_stdout, and a `when` list.
    #[test]
    fn viewing_every_field_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        write_project(
            dir.path(),
            "\
values: {BASE: 'https://example.com'}
defaults: {timeout: 30s, env: {A: b}, retry: {attempts: 2, delay: 1s}}
jobs:
  weekly:
    schedule: {weekly: {days: [mon, fri], time: '09:30'}, run_on_start: true}
    defaults: {cwd: .}
    tasks:
      check: {cmd: [echo, '200'], capture_stdout: true}
      wait:
        while: {any: [{this: '${tasks.check.stdout}', ne: 200}, {not: {this: X, exists: true}}]}
        attempts: 3
        on_failure: next
        do:
          pause: {cmd: [sleep, '0']}
      alice:
        cmd: [echo, 'hello ${BASE}']
        when: [{this: '${tasks.check.ok}', eq: true}, {this: '${tasks.check.stdout}', gt: 1, lt: 300}]
  once:
    schedule: {at: '2030-01-01T09:00:00'}
    tasks:
      go: {trigger: weekly}
",
        );
        view_all(dir.path());
    }

    /// Reorders, removes, opens and edits tasks with keys, then saves.
    #[test]
    fn edits_tasks_with_keys_and_saves() {
        let dir = tempfile::tempdir().unwrap();
        write_project(
            dir.path(),
            "values: {WHO: bob}\njobs:\n  main:\n    tasks:\n      a: {cmd: [echo, hi]}\n      b: {cmd: [echo]}\n",
        );
        let mut app = App::new(Some(dir.path().to_path_buf()), Recent::none());
        let mut terminal = terminal();
        draw(&mut app, &mut terminal);
        let tasks = |app: &App| {
            let root = &app.doc.as_ref().unwrap().root;
            doc::keys(root["jobs"]["main"]["tasks"].as_mapping().unwrap())
        };

        focus(&mut app, |row| row.task.as_deref() == Some("b"));
        press(&mut app, KeyCode::Char('K'));
        assert_eq!(tasks(&app), ["b", "a"]);
        focus(&mut app, |row| row.task.as_deref() == Some("a"));
        press(&mut app, KeyCode::Char('x'));
        assert_eq!(tasks(&app), ["b"]);

        focus(&mut app, |row| row.task.as_deref() == Some("b"));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.trail, ["b"]);
        focus(&mut app, label_is("cmd"));
        press(&mut app, KeyCode::Enter);
        ctrl(&mut app, 'u');
        type_text(&mut app, "printf");
        press(&mut app, KeyCode::Enter);
        focus(&mut app, |row| row.value.to_string() == "+ arg");
        press(&mut app, KeyCode::Enter);
        // `${W` suggests WHO; Tab inserts it.
        type_text(&mut app, "${W");
        assert!(draw(&mut app, &mut terminal).contains("project value"));
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Enter);
        let cmd = &app.doc.as_ref().unwrap().root["jobs"]["main"]["tasks"]["b"]["cmd"];
        assert_eq!(doc::flow(cmd), "[printf, ${WHO}]");

        // Renaming the open task follows it.
        focus(&mut app, label_is("task"));
        press(&mut app, KeyCode::Enter);
        ctrl(&mut app, 'u');
        type_text(&mut app, "greet");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.trail, ["greet"]);
        press(&mut app, KeyCode::Esc);
        assert!(app.trail.is_empty());

        focus(&mut app, |row| row.value.to_string() == "+ add task");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        assert_eq!(tasks(&app), ["greet", "command"]);

        ctrl(&mut app, 's');
        let saved = std::fs::read_to_string(dir.path().join("jobs/main.yaml")).unwrap();
        assert!(
            saved.contains("printf") && saved.contains("command"),
            "{saved}"
        );
        draw(&mut app, &mut terminal);
        assert!(app.validation.is_valid(), "{:?}", app.validation.result);
        press(&mut app, KeyCode::Char('q'));
        assert!(app.quit);
    }

    /// Adds, renames and deletes jobs from the sidebar; quitting with unsaved edits asks.
    #[test]
    fn manages_jobs_from_the_sidebar() {
        let dir = tempfile::tempdir().unwrap();
        write_project(dir.path(), "jobs:\n  a: {tasks: {t: {cmd: [echo]}}}\n");
        let mut app = App::new(Some(dir.path().to_path_buf()), Recent::none());
        press(&mut app, KeyCode::Char('a'));
        ctrl(&mut app, 'u');
        type_text(&mut app, "nightly");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.selection, Selection::Job("nightly".to_string()));
        // A job's name is its file's: `../x` would leave `jobs/`.
        press(&mut app, KeyCode::Char('r'));
        ctrl(&mut app, 'u');
        type_text(&mut app, "../x");
        press(&mut app, KeyCode::Enter);
        assert!(matches!(&app.message, Some(Err(error)) if error.contains("must be a file name")));
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.doc.as_ref().unwrap().job_names(), ["a", "nightly"]);
        // Added and renamed jobs take their place by name.
        press(&mut app, KeyCode::Char('a'));
        ctrl(&mut app, 'u');
        type_text(&mut app, "backup");
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.doc.as_ref().unwrap().job_names(),
            ["a", "backup", "nightly"]
        );
        press(&mut app, KeyCode::Char('r'));
        ctrl(&mut app, 'u');
        type_text(&mut app, "z");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.doc.as_ref().unwrap().job_names(), ["a", "nightly", "z"]);
        assert_eq!(app.selection, Selection::Job("z".to_string()));
        press(&mut app, KeyCode::Char('x'));
        press(&mut app, KeyCode::Char('y'));
        app.select(Selection::Job("nightly".to_string()));
        press(&mut app, KeyCode::Char('q'));
        press(&mut app, KeyCode::Char('n'));
        assert!(!app.quit);
        press(&mut app, KeyCode::Char('x'));
        press(&mut app, KeyCode::Char('y'));
        assert_eq!(app.doc.as_ref().unwrap().job_names(), ["a"]);
        assert_eq!(app.selection, Selection::Job("a".to_string()));
        // Back to what is saved: nothing to ask.
        press(&mut app, KeyCode::Char('q'));
        assert!(app.quit);
    }
}
