//! A job file as YAML: its settings and tasks on the left; on the right one part alone, or
//! the whole file with that part highlighted. Edited in place, checked as it is typed.

use std::ops::Range;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Borders, Paragraph};
use serde_yaml::Value;

use crate::chrome;
use crate::code::{self, Code};
use crate::editor::{Editor, Target};
use crate::outline::{self, Outline};
use crate::schedule;
use crate::summary::{self, SETTINGS_FIELDS};
use crate::theme::{self, fg};

pub struct JobYaml {
    pub job: usize,
    /// The task in focus; `None` for the settings.
    pub part: Option<usize>,
    /// Show the whole file, rather than the part alone.
    whole: bool,
    /// Typing goes into the file.
    editing: bool,
}

impl JobYaml {
    pub fn new(editor: &Editor, job: usize, part: Option<usize>, whole: bool) -> JobYaml {
        let mut yaml = JobYaml {
            job,
            part,
            whole,
            editing: false,
        };
        yaml.clamp(&yaml.outline(editor));
        yaml
    }

    fn outline(&self, editor: &Editor) -> Outline {
        outline::job(self.lines(editor))
    }

    fn lines<'a>(&self, editor: &'a Editor) -> &'a [String] {
        editor
            .buffer(Target::Job(self.job))
            .map_or(&[], |buffer| buffer.edit.lines())
    }

    /// Keeps the part on a task that exists.
    fn clamp(&mut self, outline: &Outline) {
        if let Some(part) = self.part {
            self.part = outline
                .tasks
                .len()
                .checked_sub(1)
                .map(|last| part.min(last));
        }
    }

    fn focus(&self, outline: &Outline) -> Range<usize> {
        match self.part.and_then(|n| outline.tasks.get(n)) {
            Some(task) => task.lines.clone(),
            None => outline.settings.clone(),
        }
    }

    /// Handles `key`; false when it goes back to the jobs.
    pub fn key(&mut self, editor: &mut Editor, key: KeyEvent) -> bool {
        let target = Target::Job(self.job);
        let outline = self.outline(editor);
        let focus = self.focus(&outline);
        let whole = self.whole;
        let Some(edit) = editor.buffer_mut(target).map(|buffer| &mut buffer.edit) else {
            return false;
        };
        if self.editing {
            if key.code == KeyCode::Esc {
                self.editing = false;
            } else {
                let rows = if whole {
                    0..edit.lines().len()
                } else {
                    focus.start..focus.end.max(edit.row + 1)
                };
                edit.key(key, rows);
            }
            self.clamp(&self.outline(editor));
            return true;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return false,
            KeyCode::Up | KeyCode::Char('k') => {
                self.part = self.part.and_then(|n| n.checked_sub(1));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let next = self.part.map_or(0, |n| n + 1);
                if next < outline.tasks.len() {
                    self.part = Some(next);
                }
            }
            KeyCode::Tab | KeyCode::BackTab => self.whole = !self.whole,
            KeyCode::Char('i') | KeyCode::Enter => {
                edit.checkpoint();
                let row = outline::start_line(edit.lines(), focus);
                edit.place(row, usize::MAX);
                self.editing = true;
            }
            KeyCode::Char('u') => editor.undo(target),
            KeyCode::Char('e') => editor.open_external(target),
            KeyCode::Char('a') => {
                let mut lines = edit.lines().to_vec();
                let name = outline::free_name(&outline);
                match outline::add_task(&mut lines, self.part, &name) {
                    Some(index) => {
                        edit.set_lines(lines);
                        self.part = Some(index);
                    }
                    None => editor.message = Some("`tasks` is written inline".to_string()),
                }
            }
            KeyCode::Char(c @ ('J' | 'K')) => {
                if let Some(part) = self.part {
                    let mut lines = edit.lines().to_vec();
                    if let Some(index) = outline::move_task(&mut lines, part, c == 'K') {
                        edit.set_lines(lines);
                        self.part = Some(index);
                    }
                }
            }
            _ => {}
        }
        self.clamp(&self.outline(editor));
        true
    }

    pub fn draw(&self, editor: &Editor, frame: &mut Frame) {
        let Some(job) = editor.files.jobs.get(self.job) else {
            return;
        };
        let file = editor.shown_path(&job.buffer.path);
        let [top, body, footer] = chrome::screen(frame);
        let mut status = Vec::new();
        if job.buffer.is_dirty() {
            status.push(Span::styled("● modified   ", fg(theme::ACCENT)));
        }
        status.push(chrome::validity(editor.problem.is_none()));
        chrome::top(
            frame,
            top,
            vec![
                Span::styled(
                    format!("{}  ", editor.files.name()),
                    theme::bold(theme::TEXT),
                ),
                Span::styled("Jobs ›  ", fg(theme::MUTED)),
                Span::styled(format!("{}  ", job.name), theme::bold(theme::TEXT)),
                Span::styled(file.clone(), fg(theme::MUTED)),
            ],
            status,
        );
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(25), Constraint::Min(0)]).areas(body);
        frame.render_widget(chrome::lines(Borders::RIGHT), left);
        let outline = self.outline(editor);
        self.draw_parts(
            frame,
            Rect {
                width: left.width.saturating_sub(1),
                ..left
            },
            &outline,
        );
        self.draw_code(editor, frame, right, &outline, &file);

        let keys = theme::keys(
            &[
                ("↑↓", "task"),
                ("⇥", "part / whole job"),
                ("i", "edit"),
                ("esc", "stop editing"),
                ("^S", "save"),
                ("u", "undo"),
                ("e", "$EDITOR"),
                ("q", "back to jobs"),
            ],
            &["⇥"],
        );
        editor.footer(frame, footer, keys);
    }

    fn draw_parts(&self, frame: &mut Frame, area: Rect, outline: &Outline) {
        let [list, bottom] = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(chrome::BOTTOM_HEIGHT),
        ])
        .areas(area);
        let width = list.width.saturating_sub(2);
        let range = |lines: &Range<usize>| {
            if lines.is_empty() {
                "—".to_string()
            } else {
                format!("{}–{}", lines.start + 1, lines.end)
            }
        };
        let row = |selected: bool, number: String, name: &str, lines: &Range<usize>| {
            let line = chrome::spread(
                vec![
                    Span::styled(format!("{number:<3}"), fg(theme::MUTED)),
                    Span::styled(
                        // Leaves room for the number and the range.
                        theme::fit(name, (width as usize).saturating_sub(16)),
                        theme::bold(theme::TEXT),
                    ),
                ],
                vec![Span::styled(format!("{} ", range(lines)), fg(theme::MUTED))],
                width.saturating_sub(2),
            );
            chrome::row(vec![line], selected, list.width)
        };
        let mut lines = vec![
            Line::default(),
            Line::from(vec![Span::raw("  "), chrome::heading("job settings")]),
        ];
        lines.extend(row(
            self.part.is_none(),
            "≡".to_string(),
            "schedule · values · defaults",
            &outline.settings,
        ));
        lines.push(Line::default());
        lines.push(Line::from(vec![Span::raw("  "), chrome::heading("tasks")]));
        for (n, task) in outline.tasks.iter().enumerate() {
            lines.extend(row(
                self.part == Some(n),
                (n + 1).to_string(),
                &task.name,
                &task.lines,
            ));
        }
        frame.render_widget(Paragraph::new(lines), list);
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(theme::key("a", "add task below", false)),
                Line::from(theme::key("J K", "move task", false)),
            ])
            .block(chrome::bottom()),
            bottom,
        );
    }

    fn draw_code(
        &self,
        editor: &Editor,
        frame: &mut Frame,
        area: Rect,
        outline: &Outline,
        file: &str,
    ) {
        let [_, tabs, _, code_area, status] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .areas(area);
        let task = self.part.and_then(|n| outline.tasks.get(n));
        let part_label = match task {
            Some(task) => format!("Task: {}", task.name),
            None => "Job settings".to_string(),
        };
        let focus = self.focus(outline);
        let lines = self.lines(editor);
        let info = if self.whole {
            let name = task.map_or("settings", |task| task.name.as_str());
            format!("{file} · {} lines · {name} highlighted", lines.len())
        } else if focus.is_empty() {
            format!("{file} · none yet")
        } else {
            format!(
                "lines {}–{} of {file} · rest hidden",
                focus.start + 1,
                focus.end
            )
        };
        chrome::tabs(
            frame,
            tabs,
            &[part_label, "Whole job YAML".to_string()],
            usize::from(self.whole),
            vec![Span::styled(format!("{info} "), fg(theme::MUTED))],
        );
        let cursor = editor
            .buffer(Target::Job(self.job))
            .filter(|_| self.editing)
            .map(|buffer| (buffer.edit.row, buffer.edit.col));
        code::draw(
            frame,
            Rect {
                x: code_area.x + 1,
                width: code_area.width.saturating_sub(1),
                ..code_area
            },
            &Code {
                lines,
                focus,
                only_focus: !self.whole,
                cursor,
            },
        );

        let (text, ok) = self.status(editor, task.map(|task| task.name.as_str()));
        let fields = match task {
            Some(task) => {
                let yaml = editor.files.jobs[self.job]
                    .buffer
                    .yaml()
                    .unwrap_or_default();
                let value = yaml.get("tasks").and_then(|tasks| tasks.get(&task.name));
                value.map_or(summary::Kind::Unknown, summary::kind).fields()
            }
            None => SETTINGS_FIELDS,
        };
        let inner = chrome::pad(status, 2, 0);
        let fields = format!("fields: {fields}");
        let fields_width = (inner.width / 2) as usize;
        let text = theme::fit(
            &text,
            (inner.width as usize).saturating_sub(fields_width + 2),
        );
        let color = if ok { theme::OK } else { theme::ERROR };
        frame.render_widget(chrome::lines(Borders::TOP), status);
        frame.render_widget(
            Paragraph::new(chrome::spread(
                vec![Span::styled(text, fg(color))],
                vec![Span::styled(
                    theme::fit(&fields, fields_width),
                    fg(theme::MUTED),
                )],
                inner.width,
            )),
            Rect {
                y: inner.y + 1,
                height: 1,
                ..inner
            },
        );
    }

    /// What core says of the part in focus, and whether it is fine.
    fn status(&self, editor: &Editor, task: Option<&str>) -> (String, bool) {
        let job = &editor.files.jobs[self.job];
        if let Some(problem) = editor.problem_in(&job.name) {
            return (format!("✗ {}", problem.message), false);
        }
        let yaml = job.buffer.yaml().unwrap_or_default();
        if self.whole {
            let count = yaml
                .get("tasks")
                .and_then(Value::as_mapping)
                .map_or(0, |tasks| tasks.len());
            return (format!("✓ job loads · {count} tasks"), true);
        }
        match task {
            Some(name) => {
                let kind = yaml
                    .get("tasks")
                    .and_then(|tasks| tasks.get(name))
                    .map_or(summary::Kind::Unknown, summary::kind);
                (format!("✓ valid {} task", kind.name()), true)
            }
            None => match schedule::summary(yaml.get("schedule")) {
                Some(schedule) => (format!("✓ schedule ok · {schedule}"), true),
                None => ("✓ settings ok · not scheduled".to_string(), true),
            },
        }
    }
}
