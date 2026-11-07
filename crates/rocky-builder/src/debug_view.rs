//! A test run of one job, as `rocky exec` runs it: its tasks in order with how each went, and
//! the result of the selected one.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Borders, Paragraph, Wrap};
use serde_json::Value as Json;

use crate::chrome;
use crate::editor::Editor;
use crate::files::Files;
use crate::results::{End, TaskRun};
use crate::run::{Run, Texts};
use crate::summary;
use crate::theme::{self, fg};

/// One task as the run lists it.
struct Row {
    depth: usize,
    name: String,
    /// Its index among the job's top-level tasks; `None` inside a loop.
    top: Option<usize>,
    when: Option<String>,
    /// For a cmd task: its command as written, before `${}` are filled in.
    command: Option<String>,
}

pub enum Next {
    Stay,
    Back,
    /// Open the job's YAML at this task (`None`: the settings).
    Edit(Option<usize>),
}

const TABS: [&str; 3] = ["stdout", "stderr", "raw"];

pub struct Debug {
    pub job: usize,
    name: String,
    rows: Vec<Row>,
    run: Run,
    cursor: usize,
    tab: usize,
    verbose: bool,
}

impl Debug {
    /// Starts running job `job` of `files` as it is now, saved or not.
    pub fn start(files: &Files, job: usize) -> Debug {
        let name = files.jobs[job].name.clone();
        let yaml = files.jobs[job].buffer.yaml().unwrap_or_default();
        let mut top = 0;
        let rows = summary::tasks(yaml.get("tasks"))
            .into_iter()
            .map(|(depth, name, task)| {
                let index = (depth == 0).then(|| {
                    top += 1;
                    top - 1
                });
                let command = (summary::kind(task) == summary::Kind::Cmd)
                    .then(|| summary::command_line(task));
                Row {
                    depth,
                    name,
                    top: index,
                    when: task.get("when").map(summary::condition),
                    command,
                }
            })
            .collect();
        Debug {
            job,
            run: Run::start(texts(files), &name),
            name,
            rows,
            cursor: 0,
            tab: 0,
            verbose: true,
        }
    }

    /// Takes in the run's progress; returns whether it is still running.
    pub fn tick(&mut self) -> bool {
        self.run.poll();
        self.run.is_running()
    }

    pub fn key(&mut self, editor: &Editor, key: KeyEvent) -> Next {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.run.stop();
                return Next::Back;
            }
            KeyCode::Up | KeyCode::Char('k') => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.cursor = (self.cursor + 1).min(self.rows.len().saturating_sub(1))
            }
            KeyCode::Tab => self.tab = (self.tab + 1) % TABS.len(),
            KeyCode::BackTab => self.tab = (self.tab + TABS.len() - 1) % TABS.len(),
            KeyCode::Char('v') => self.verbose = !self.verbose,
            KeyCode::Char('r') => {
                self.run.stop();
                *self = Debug::start(&editor.files, self.job);
            }
            KeyCode::Char('e') => {
                self.run.stop();
                // A task inside a loop opens the loop it is in.
                let top = self.rows[..=self.cursor.min(self.rows.len().saturating_sub(1))]
                    .iter()
                    .rev()
                    .find_map(|row| row.top);
                return Next::Edit(top);
            }
            _ => {}
        }
        Next::Stay
    }

    fn task(&self, name: &str) -> Option<&TaskRun> {
        self.run.results.task(&self.name, name)
    }

    pub fn draw(&self, editor: &Editor, frame: &mut Frame) {
        let [top, body, footer] = chrome::screen(frame);
        let file = editor.shown_path(&editor.files.jobs[self.job].buffer.path);
        chrome::top(
            frame,
            top,
            vec![
                Span::styled(
                    format!("{}  ", editor.files.name()),
                    theme::bold(theme::TEXT),
                ),
                Span::styled("Debug ›  ", fg(theme::MUTED)),
                Span::styled(format!("{}  ", self.name), theme::bold(theme::TEXT)),
                Span::styled(file, fg(theme::MUTED)),
            ],
            vec![
                Span::styled("⚠ commands really run · no dry run   ", fg(theme::ACCENT)),
                chrome::validity(editor.problem.is_none()),
            ],
        );
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(60), Constraint::Min(0)]).areas(body);
        frame.render_widget(chrome::lines(Borders::RIGHT), left);
        self.draw_tasks(
            frame,
            Rect {
                width: left.width.saturating_sub(1),
                ..left
            },
        );
        self.draw_result(frame, right);
        let keys = theme::keys(
            &[
                ("↑↓", "task"),
                ("⇥", "stdout / stderr"),
                ("v", "toggle verbose"),
                ("r", "re-run"),
                ("e", "edit task"),
                ("esc", "back to editor"),
            ],
            &["r"],
        );
        editor.footer(frame, footer, keys);
    }

    fn draw_tasks(&self, frame: &mut Frame, area: Rect) {
        let [list, bottom] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(2)]).areas(area);
        let list = chrome::pad(list, 2, 1);
        let width = list.width;
        let seconds = format!("{:.2} s", self.run.elapsed().as_secs_f64());
        let state = match &self.run.results.end {
            None => Span::styled(format!("RUNNING · {seconds}"), fg(theme::ACCENT)),
            Some(End::Done { failed: None }) => {
                Span::styled(format!("FINISHED · {seconds}"), fg(theme::OK))
            }
            Some(End::Done { failed: Some(_) }) => {
                Span::styled(format!("FAILED · {seconds}"), fg(theme::ERROR))
            }
            Some(End::Error(_)) => Span::styled("ERROR", fg(theme::ERROR)),
        };
        let mut lines = vec![chrome::spread(
            vec![chrome::heading("test run · tasks in order")],
            vec![state],
            width,
        )];
        let mut selected_at = 0;
        let mut queued = Vec::new();
        for (n, row) in self.rows.iter().enumerate() {
            if n == self.cursor {
                selected_at = lines.len();
            }
            let run = self.task(&row.name);
            if let Some(job) = run.and_then(|run| run.queued.clone()) {
                queued.push(job);
            }
            let mut row_lines = self.row_lines(row, run, width);
            if n == self.cursor {
                row_lines = row_lines
                    .into_iter()
                    .map(|line| line.style(Style::new().bg(theme::SELECTED)))
                    .collect();
            }
            lines.extend(row_lines);
            lines.push(Line::default());
        }
        if !queued.is_empty() {
            lines.push(Line::styled("┄".repeat(width as usize), fg(theme::LINE)));
            for job in &queued {
                lines.push(chrome::spread(
                    vec![
                        Span::styled("↪  ", fg(theme::ACCENT)),
                        Span::styled(format!("queued: {job}"), theme::bold(theme::TEXT)),
                    ],
                    vec![Span::styled("after job", fg(theme::MUTED))],
                    width,
                ));
            }
            lines.push(Line::styled(
                "   Triggered jobs run after this one, in order.",
                fg(theme::MUTED),
            ));
        }
        match &self.run.results.end {
            Some(End::Error(error))
            | Some(End::Done {
                failed: Some(error),
            }) => {
                lines.push(Line::default());
                lines.push(Line::styled(format!("✗ {error}"), fg(theme::ERROR)));
            }
            _ => {}
        }
        let skip = (selected_at + 5).saturating_sub(list.height as usize);
        frame.render_widget(
            Paragraph::new(lines.into_iter().skip(skip).collect::<Vec<_>>())
                .wrap(Wrap { trim: false }),
            list,
        );

        let count = |test: &dyn Fn(&TaskRun) -> bool| {
            self.rows
                .iter()
                .filter(|row| self.task(&row.name).is_some_and(test))
                .count()
        };
        let ok = count(&|run| run.runs > 0 && run.failures == 0);
        let skipped = count(&|run| run.skipped);
        let failed = count(&|run| run.failures > 0);
        let stats = Line::from(vec![
            Span::styled(format!("{} tasks   ", self.rows.len()), fg(theme::MUTED)),
            Span::styled(format!("{ok} ok   "), fg(theme::OK)),
            Span::styled(format!("{skipped} skipped   "), fg(theme::MUTED)),
            Span::styled(format!("{failed} failed   "), fg(theme::MUTED)),
            Span::styled(format!("{} triggered", queued.len()), fg(theme::MUTED)),
        ]);
        frame.render_widget(
            Paragraph::new(stats).block(
                chrome::lines(Borders::TOP).padding(ratatui::widgets::Padding::horizontal(2)),
            ),
            bottom,
        );
    }

    /// A task's lines: how it went, its `when`, and its command when verbose.
    fn row_lines(&self, row: &Row, run: Option<&TaskRun>, width: u16) -> Vec<Line<'static>> {
        let last = run.and_then(|run| run.last.as_ref());
        let (mark, mark_color, outcome) = match run {
            Some(run) if run.skipped => (
                "–",
                theme::MUTED,
                vec![Span::styled("skipped", fg(theme::MUTED))],
            ),
            Some(run) if run.queued.is_some() => (
                "↪",
                theme::ACCENT,
                vec![Span::styled("queued", fg(theme::ACCENT))],
            ),
            Some(run) if run.items.is_some() => (
                "↻",
                theme::PURPLE,
                vec![Span::styled(
                    format!("{} items", run.items.unwrap_or(0)),
                    fg(theme::PURPLE),
                )],
            ),
            Some(run) if run.check.is_some() && run.runs == 0 => (
                "↻",
                theme::BLUE,
                vec![Span::styled(
                    format!("check {}", run.check.map_or(0, |(n, _)| n)),
                    fg(theme::BLUE),
                )],
            ),
            Some(_) if last.is_some() => {
                let event = last.cloned().unwrap_or_default();
                let ok = event["ok"] == Json::Bool(true);
                let color = if ok { theme::OK } else { theme::ERROR };
                let exit = match event["error"].as_str() {
                    Some(error) => theme::fit(error, 24),
                    None => format!("exit {}", event["exit_code"]),
                };
                let mut spans = vec![Span::styled(exit, fg(color))];
                if let Some(ms) = event["ms"].as_u64() {
                    spans.push(Span::styled(format!("{ms:>8} ms"), fg(theme::MUTED)));
                }
                (if ok { "✓" } else { "✗" }, color, spans)
            }
            _ => ("·", theme::DIM, vec![Span::styled("…", fg(theme::DIM))]),
        };
        let indent = "   ".repeat(row.depth);
        let mut lines = vec![chrome::spread(
            vec![
                Span::styled(format!("{indent}{mark}  "), fg(mark_color)),
                Span::styled(row.name.clone(), theme::bold(theme::TEXT)),
            ],
            outcome,
            width,
        )];
        let pad = format!("{indent}   ");
        if let Some(when) = &row.when {
            let mut spans = vec![Span::styled(format!("{pad}when {when}"), fg(theme::BLUE))];
            match run {
                Some(run) if run.skipped => spans.push(Span::styled(" → false", fg(theme::ERROR))),
                Some(run) if run.runs > 0 || run.items.is_some() || run.check.is_some() => {
                    spans.push(Span::styled(" → true", fg(theme::BLUE)))
                }
                _ => {}
            }
            lines.push(Line::from(spans));
        }
        if self.verbose
            && let Some(command) = &row.command
        {
            let text = format!("{pad}+ [{}] {command}", self.name);
            lines.push(Line::styled(
                theme::fit(&text, width as usize),
                fg(theme::MUTED),
            ));
        }
        lines
    }

    fn draw_result(&self, frame: &mut Frame, area: Rect) {
        let area = chrome::pad(area, 2, 1);
        let [head, tabs, _, body, hint] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .areas(area);
        let name = self
            .rows
            .get(self.cursor)
            .map_or("", |row| row.name.as_str());
        frame.render_widget(
            Paragraph::new(Line::from(chrome::heading(&format!(
                "result · tasks.{name}"
            )))),
            head,
        );
        chrome::tabs(frame, tabs, &TABS.map(str::to_string), self.tab, Vec::new());
        let run = self.task(name);
        let result = run
            .and_then(|run| run.last.as_ref())
            .map(|event| event["result"].clone());
        let text = match &result {
            None => match run {
                None => "No result: the task hasn't run.",
                Some(run) if run.skipped => "No result: the task was skipped.",
                Some(_) => "No result of its own: see the tasks it ran.",
            }
            .to_string(),
            Some(result) => match self.tab {
                0 => match &result["stdout"] {
                    Json::String(text) => text.clone(),
                    Json::Null => result["error"].as_str().unwrap_or_default().to_string(),
                    json => pretty(json),
                },
                1 => result["stderr"].as_str().unwrap_or_default().to_string(),
                _ => pretty(result),
            },
        };
        let color = if result.is_some() {
            theme::TEXT
        } else {
            theme::MUTED
        };
        let lines: Vec<Line> = text
            .lines()
            .map(|line| Line::styled(line.to_string(), fg(color)))
            .collect();
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);
        let jq = if self.tab == 1 { ".stderr" } else { ".stdout" };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("$ ", fg(theme::ACCENT)),
                Span::styled(
                    format!("rocky exec {} -v | jq {jq}", self.name),
                    fg(theme::MUTED),
                ),
            ])),
            hint,
        );
    }
}

fn pretty(json: &Json) -> String {
    serde_json::to_string_pretty(json).unwrap_or_default()
}

fn texts(files: &Files) -> Texts {
    Texts {
        root: files.root.clone(),
        settings: Some(files.settings.text()),
        jobs: files.job_texts(),
    }
}
