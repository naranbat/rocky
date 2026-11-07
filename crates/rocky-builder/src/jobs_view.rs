//! The Jobs tab: the project's jobs on the left; on the right the selected job's schedule,
//! values, defaults and tasks, in the order they run.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Borders, Paragraph, Wrap};
use serde_yaml::Value;

use crate::chrome;
use crate::editor::{Action, Editor, Target};
use crate::outline;
use crate::schedule;
use crate::summary;
use crate::theme::{self, fg};

/// The jobs shown, as indices in `files.jobs`: those matching the filter, scheduled first.
fn order(editor: &Editor) -> Vec<usize> {
    let filter = editor.filter.text.to_lowercase();
    let mut jobs: Vec<(bool, usize)> = editor
        .files
        .jobs
        .iter()
        .enumerate()
        .filter(|(_, job)| job.name.to_lowercase().contains(&filter))
        .map(|(n, _)| (job_yaml(editor, n).get("schedule").is_none(), n))
        .collect();
    jobs.sort();
    jobs.into_iter().map(|(_, n)| n).collect()
}

/// Job `n`'s file as YAML; null when it doesn't parse.
fn job_yaml(editor: &Editor, n: usize) -> Value {
    editor.files.jobs[n].buffer.yaml().unwrap_or_default()
}

fn task_count(editor: &Editor, n: usize) -> usize {
    let lines = editor.files.jobs[n].buffer.edit.lines();
    outline::job(lines).tasks.len()
}

pub fn key(editor: &mut Editor, key: KeyEvent) -> Action {
    if let Some(input) = &mut editor.new_job {
        match key.code {
            KeyCode::Esc => editor.new_job = None,
            KeyCode::Enter => {
                let name = input.text.trim().to_string();
                match editor.files.add_job(&name) {
                    Ok(n) => {
                        editor.job = n;
                        editor.task = None;
                        editor.new_job = None;
                        editor.filter.set("");
                        editor.message = Some(format!("added jobs/{name}.yaml"));
                    }
                    Err(error) => editor.message = Some(error),
                }
            }
            _ => {
                input.key(key);
            }
        }
        return Action::None;
    }
    if editor.filtering {
        match key.code {
            KeyCode::Esc => {
                editor.filter.set("");
                editor.filtering = false;
            }
            KeyCode::Enter => editor.filtering = false,
            _ => {
                editor.filter.key(key);
                if let Some(first) = order(editor).first() {
                    editor.job = *first;
                    editor.task = None;
                }
            }
        }
        return Action::None;
    }
    if editor.switch_tab(key) {
        return Action::None;
    }
    let tasks = task_count(
        editor,
        editor.job.min(editor.files.jobs.len().saturating_sub(1)),
    );
    let target = Target::Job(editor.job);
    match (key.code, editor.task) {
        (KeyCode::Char('q'), _) => return editor.quit(),
        (KeyCode::Up | KeyCode::Char('k'), None) => step_job(editor, false),
        (KeyCode::Down | KeyCode::Char('j'), None) => step_job(editor, true),
        (KeyCode::Up | KeyCode::Char('k'), Some(task)) => {
            editor.task = Some(task.saturating_sub(1))
        }
        (KeyCode::Down | KeyCode::Char('j'), Some(task)) => {
            editor.task = Some((task + 1).min(tasks.saturating_sub(1)))
        }
        (KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab, None) if tasks > 0 => {
            editor.task = Some(0)
        }
        (KeyCode::Left | KeyCode::Char('h') | KeyCode::Esc | KeyCode::Tab, Some(_)) => {
            editor.task = None
        }
        (KeyCode::Enter, task) => editor.open_yaml(task, false),
        (KeyCode::Char('Y'), task) => editor.open_yaml(task, true),
        (KeyCode::Char('d'), _) => editor.debug(),
        (KeyCode::Char('+'), _) => editor.new_job = Some(Default::default()),
        (KeyCode::Char('/'), _) => editor.filtering = true,
        (KeyCode::Char('u'), _) => editor.undo(target),
        (KeyCode::Char('a'), task) => add_task(editor, task),
        (KeyCode::Char('J'), Some(task)) => move_task(editor, task, false),
        (KeyCode::Char('K'), Some(task)) => move_task(editor, task, true),
        _ => {}
    }
    Action::None
}

fn step_job(editor: &mut Editor, down: bool) {
    let order = order(editor);
    let at = order.iter().position(|&n| n == editor.job);
    let next = match (at, down) {
        (Some(at), true) => order.get(at + 1),
        (Some(at), false) => at.checked_sub(1).and_then(|at| order.get(at)),
        (None, _) => order.first(),
    };
    if let Some(&next) = next {
        editor.job = next;
    }
}

/// Adds a task after `after` (or last), and selects it.
fn add_task(editor: &mut Editor, after: Option<usize>) {
    let Some(job) = editor.files.jobs.get_mut(editor.job) else {
        return;
    };
    let edit = &mut job.buffer.edit;
    let mut lines = edit.lines().to_vec();
    let name = outline::free_name(&outline::job(&lines));
    match outline::add_task(&mut lines, after, &name) {
        Some(index) => {
            edit.set_lines(lines);
            editor.task = Some(index);
        }
        None => editor.message = Some("`tasks` is written inline: edit it as YAML".to_string()),
    }
}

/// Moves task `task` up or down.
fn move_task(editor: &mut Editor, task: usize, up: bool) {
    let Some(job) = editor.files.jobs.get_mut(editor.job) else {
        return;
    };
    let edit = &mut job.buffer.edit;
    let mut lines = edit.lines().to_vec();
    if let Some(index) = outline::move_task(&mut lines, task, up) {
        edit.set_lines(lines);
        editor.task = Some(index);
    }
}

pub fn keys(editor: &Editor) -> Vec<Span<'static>> {
    if editor.new_job.is_some() {
        return theme::keys(&[("↵", "add job"), ("esc", "cancel")], &["↵"]);
    }
    if editor.filtering {
        return theme::keys(&[("↵", "keep filter"), ("esc", "clear")], &[]);
    }
    theme::keys(
        &[
            ("↑↓", "select"),
            ("→", "tasks"),
            ("↵", "edit"),
            ("^S", "save"),
            ("u", "undo"),
            ("/", "filter"),
            ("q", "quit"),
        ],
        &["^S"],
    )
}

pub fn draw(editor: &Editor, frame: &mut Frame, area: Rect) {
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(32), Constraint::Min(0)]).areas(area);
    frame.render_widget(chrome::lines(Borders::RIGHT), left);
    let left = Rect {
        width: left.width.saturating_sub(1),
        ..left
    };
    draw_jobs(editor, frame, left);
    if editor.files.jobs.is_empty() {
        let note = Line::styled("  No jobs yet: press + to add one.", fg(theme::MUTED));
        frame.render_widget(Paragraph::new(note), chrome::pad(right, 1, 1));
        return;
    }
    draw_job(editor, frame, right);
}

fn draw_jobs(editor: &Editor, frame: &mut Frame, area: Rect) {
    let [list, bottom] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(chrome::BOTTOM_HEIGHT),
    ])
    .areas(area);
    let width = list.width as usize;
    let name_width = 14;
    let tasks_width = 6;
    let schedule_width = width.saturating_sub(5 + name_width + tasks_width + 1);
    let header = format!(
        "     {:<name_width$}{:<schedule_width$}{:>tasks_width$}",
        "JOB", "SCHEDULE", "TASKS"
    );
    let mut lines = vec![Line::default(), Line::styled(header, fg(theme::MUTED))];
    let order = order(editor);
    // Each row has a blank line above and below it.
    let rows = (list.height as usize).saturating_sub(2) / 3;
    let at = order.iter().position(|&n| n == editor.job).unwrap_or(0);
    let top = (at + 1).saturating_sub(rows);
    for &n in order.iter().skip(top).take(rows) {
        let job = &editor.files.jobs[n];
        let yaml = job_yaml(editor, n);
        let summary = schedule::summary(yaml.get("schedule"));
        let (glyph, color) = if job.buffer.is_dirty() {
            ("●", theme::ACCENT)
        } else if editor.problem.as_ref().and_then(|p| p.job.as_deref()) == Some(job.name.as_str())
        {
            ("✗", theme::ERROR)
        } else if summary.is_some() {
            ("◆", theme::OK)
        } else {
            ("○", theme::MUTED)
        };
        let schedule = summary.unwrap_or_else(|| "— not scheduled".to_string());
        let selected = n == editor.job;
        let line = Line::from(vec![
            Span::styled(format!("{glyph}  "), fg(color)),
            Span::styled(
                format!("{:<name_width$}", theme::fit(&job.name, name_width - 1)),
                theme::bold(theme::TEXT),
            ),
            Span::styled(
                format!(
                    "{:<schedule_width$}",
                    theme::fit(&schedule, schedule_width.saturating_sub(1))
                ),
                fg(theme::MUTED),
            ),
            Span::styled(
                format!("{:>tasks_width$}", task_count(editor, n)),
                fg(theme::MUTED),
            ),
        ]);
        lines.extend(chrome::row(vec![line], selected, list.width));
    }
    frame.render_widget(Paragraph::new(lines), list);

    let first = match (
        &editor.new_job,
        editor.filtering || !editor.filter.text.is_empty(),
    ) {
        (Some(input), _) => {
            let mut spans = vec![Span::styled("+ new job → jobs/", fg(theme::MUTED))];
            spans.extend(input.spans(true, (bottom.width as usize).saturating_sub(24)));
            spans.push(Span::styled(".yaml", fg(theme::MUTED)));
            Line::from(spans)
        }
        (None, true) => {
            let mut spans = vec![Span::styled("/ ", fg(theme::ACCENT))];
            spans.extend(
                editor
                    .filter
                    .spans(editor.filtering, (bottom.width as usize).saturating_sub(4)),
            );
            Line::from(spans)
        }
        (None, false) => Line::from(vec![
            Span::styled(" + ", Style::new().fg(theme::BG).bg(theme::MUTED)),
            Span::styled(" new job → jobs/NAME.yaml", fg(theme::MUTED)),
        ]),
    };
    let scheduled = (0..editor.files.jobs.len())
        .filter(|&n| job_yaml(editor, n).get("schedule").is_some())
        .count();
    let mut stats = format!("{} jobs · {scheduled} scheduled", editor.files.jobs.len());
    if let Some(env) = &editor.files.env {
        stats.push_str(&format!(" · env_file {}", editor.shown_path(&env.path)));
    }
    frame.render_widget(
        Paragraph::new(vec![first, Line::styled(stats, fg(theme::MUTED))]).block(chrome::bottom()),
        bottom,
    );
}

fn draw_job(editor: &Editor, frame: &mut Frame, area: Rect) {
    let job = &editor.files.jobs[editor.job];
    let [head, body, bottom] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(2),
    ])
    .areas(area);
    let problem = editor.problem_in(&job.name);
    let status = if job.buffer.is_dirty() {
        Span::styled("● modified · ^S to save", fg(theme::ACCENT))
    } else if problem.is_some() {
        Span::styled("✗ saved · invalid", fg(theme::ERROR))
    } else {
        Span::styled("✓ saved · valid", fg(theme::OK))
    };
    let head_inner = chrome::pad(head, 2, 1);
    frame.render_widget(chrome::lines(Borders::BOTTOM), head);
    frame.render_widget(
        Paragraph::new(chrome::spread(
            vec![
                Span::styled(format!("{}  ", job.name), theme::bold(theme::TEXT)),
                Span::styled(editor.shown_path(&job.buffer.path), fg(theme::MUTED)),
            ],
            vec![status],
            head_inner.width,
        )),
        Rect {
            height: 1,
            ..head_inner
        },
    );

    let body = chrome::pad(body, 2, 1);
    let yaml = match job.buffer.yaml() {
        Ok(yaml) => yaml,
        Err(error) => {
            let text = Line::styled(format!("✗ {error}"), fg(theme::ERROR));
            frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), body);
            return;
        }
    };
    let row = |label: &str, value: Option<&Value>, none: &str, color| {
        let value = match value {
            Some(value) => Span::styled(summary::entries(value), fg(color)),
            None => Span::styled(none.to_string(), fg(theme::MUTED)),
        };
        Line::from(vec![
            Span::styled(format!("{label:<14}"), fg(theme::MUTED)),
            value,
        ])
    };
    let mut lines = vec![
        row(
            "schedule",
            yaml.get("schedule"),
            "— (exec or trigger only)",
            theme::OK,
        ),
        row("values", yaml.get("values"), "—", theme::TEXT),
        row("defaults", yaml.get("defaults"), "—", theme::TEXT),
        Line::default(),
    ];
    if let Some(problem) = problem {
        let message = theme::fit(&format!("✗ {}", problem.message), body.width as usize);
        lines.push(Line::styled(message, fg(theme::ERROR)));
        lines.push(Line::default());
    }
    lines.push(chrome::spread(
        vec![chrome::heading("tasks · run in this order")],
        vec![chrome::heading("kind")],
        body.width,
    ));
    let height = (lines.len() as u16).min(body.height);
    frame.render_widget(Paragraph::new(lines), Rect { height, ..body });
    let tasks_area = Rect {
        y: body.y + height,
        height: body.height.saturating_sub(height),
        ..body
    };
    draw_tasks(editor, frame, tasks_area, &yaml);

    frame.render_widget(
        Paragraph::new(Line::from(theme::keys(
            &[
                ("a", "add task"),
                ("J K", "move"),
                ("↵", "edit task YAML"),
                ("Y", "whole job YAML"),
                ("d", "debug this job"),
            ],
            &[],
        )))
        .block(chrome::lines(Borders::TOP).padding(ratatui::widgets::Padding::horizontal(2))),
        bottom,
    );
}

/// One task's box in the job detail.
struct TaskBox<'a> {
    /// Its row, from the top of the list, and its height.
    y: u16,
    height: u16,
    number: usize,
    depth: usize,
    name: &'a str,
    task: &'a Value,
    when: Option<String>,
    selected: bool,
}

/// The tasks in boxes, loops' tasks indented under them.
fn draw_tasks(editor: &Editor, frame: &mut Frame, area: Rect, yaml: &Value) {
    let tasks = summary::tasks(yaml.get("tasks"));
    let mut boxes = Vec::new();
    let mut top_index = 0;
    let mut y = 0;
    for (n, (depth, name, task)) in tasks.iter().enumerate() {
        let when = task.get("when").map(summary::condition);
        let height = if when.is_some() { 4 } else { 3 };
        let selected = *depth == 0 && editor.task == Some(top_index);
        if *depth == 0 {
            top_index += 1;
        }
        boxes.push(TaskBox {
            y,
            height,
            number: n + 1,
            depth: *depth,
            name,
            task,
            when,
            selected,
        });
        y += height;
    }
    // Scrolls so that the selected task shows.
    let bottom = boxes
        .iter()
        .find(|b| b.selected)
        .map_or(0, |b| b.y + b.height);
    let offset = bottom.saturating_sub(area.height);
    for TaskBox {
        y,
        height,
        number,
        depth,
        name,
        task,
        when,
        selected,
    } in boxes
    {
        let Some(y) = y.checked_sub(offset) else {
            continue;
        };
        if y + height > area.height {
            break;
        }
        let indent = depth as u16 * 3;
        let rect = Rect {
            x: area.x + indent,
            y: area.y + y,
            width: area.width.saturating_sub(indent),
            height,
        };
        let kind = summary::kind(task);
        let inner_width = rect.width.saturating_sub(4);
        let kind_width = kind.name().len() + 1;
        let text = format!("{name} {}", summary::line(task));
        let text = theme::fit(&text, (inner_width as usize).saturating_sub(kind_width + 4));
        let (name_part, rest) = text.split_at(text.find(' ').unwrap_or(text.len()));
        let mut lines = vec![chrome::spread(
            vec![
                Span::styled(format!("{number:<3}"), fg(theme::MUTED)),
                Span::styled(name_part.to_string(), theme::bold(theme::TEXT)),
                Span::styled(rest.to_string(), fg(theme::TEXT)),
            ],
            vec![Span::styled(kind.name(), fg(kind.color()))],
            inner_width,
        )];
        if let Some(when) = when {
            lines.push(Line::styled(format!("   when {when}"), fg(theme::BLUE)));
        }
        let border = if selected { theme::ACCENT } else { theme::LINE };
        let block = chrome::lines(Borders::ALL)
            .border_style(fg(border))
            .padding(ratatui::widgets::Padding::horizontal(1));
        let block = if selected {
            block.style(Style::new().bg(theme::SELECTED))
        } else {
            block
        };
        frame.render_widget(Paragraph::new(lines).block(block), rect);
    }
}
