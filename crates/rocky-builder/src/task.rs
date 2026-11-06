//! Task lists (a job's `tasks`, or a loop's `do`) as one row per task, and a task's own
//! rows: its name, kind, `when`, the body of its kind, and its last run.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use serde_yaml::{Mapping, Value};

use crate::condition;
use crate::doc;
use crate::edit::Edit;
use crate::form::{Action, Choice, Row};
use crate::legacy;
use crate::node::{self, Path};
use crate::run_view;
use crate::theme;
use crate::{task_cmd, task_file, task_loop, task_trigger};

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Cmd,
    ForEach,
    While,
    Trigger,
    File,
}

impl Kind {
    const ALL: [Kind; 5] = [
        Kind::Cmd,
        Kind::ForEach,
        Kind::While,
        Kind::Trigger,
        Kind::File,
    ];

    /// The kind of a task entry: a path string is a task file.
    pub fn of(task: &Value) -> Option<Kind> {
        if task.is_string() {
            return Some(Kind::File);
        }
        let map = task.as_mapping()?;
        [
            ("cmd", Kind::Cmd),
            ("for_each", Kind::ForEach),
            ("while", Kind::While),
            ("trigger", Kind::Trigger),
            ("file", Kind::File),
        ]
        .into_iter()
        .find(|(key, _)| map.contains_key(*key))
        .map(|(_, kind)| kind)
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Cmd => "Command",
            Kind::ForEach => "For each",
            Kind::While => "While",
            Kind::Trigger => "Trigger",
            Kind::File => "Task file",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Kind::Cmd => "Run a program with args, no shell",
            Kind::ForEach => "Repeat do for every list item",
            Kind::While => "Repeat do while a condition holds",
            Kind::Trigger => "Queue a run of another job",
            Kind::File => "Reuse a YAML file, pass with inputs",
        }
    }

    fn color(self) -> Color {
        match self {
            Kind::Cmd => theme::CMD,
            Kind::ForEach => theme::FOR_EACH,
            Kind::While => theme::WHILE,
            Kind::Trigger => theme::TRIGGER,
            Kind::File => theme::FILE,
        }
    }

    /// A new task of this kind.
    fn template(self, jobs: &[String]) -> Value {
        let step = || {
            doc::map_of([(
                "cmd",
                Value::Sequence(vec![Value::from("echo"), Value::from("${ITEM}")]),
            )])
        };
        match self {
            Kind::Cmd => doc::map_of([(
                "cmd",
                Value::Sequence(vec![Value::from("echo"), Value::from("hello")]),
            )]),
            Kind::ForEach => doc::map_of([
                ("for_each", Value::from("tasks.NAME.stdout")),
                ("as", Value::from("ITEM")),
                ("do", doc::map_of([("step", step())])),
            ]),
            Kind::While => doc::map_of([
                (
                    "while",
                    doc::map_of([
                        ("this", Value::from("tasks.check.ok")),
                        ("eq", Value::from(false)),
                    ]),
                ),
                ("attempts", Value::from(10)),
                (
                    "do",
                    doc::map_of([(
                        "check",
                        doc::map_of([("cmd", Value::Sequence(vec![Value::from("true")]))]),
                    )]),
                ),
            ]),
            Kind::Trigger => doc::map_of([(
                "trigger",
                Value::from(jobs.first().cloned().unwrap_or_default()),
            )]),
            Kind::File => doc::map_of([("file", Value::from("../tasks/task.yaml"))]),
        }
    }
}

/// The kind's label in its color.
fn kind_span(kind: Option<Kind>) -> Span<'static> {
    match kind {
        Some(kind) => Span::styled(
            format!("{:<9}", kind.label()),
            Style::new().fg(kind.color()),
        ),
        None => Span::styled(format!("{:<9}", "Unknown"), theme::error()),
    }
}

/// What a task does, in one line.
fn summary(task: &Value) -> String {
    if legacy::is_legacy(task) {
        return "old syntax: if / switch".to_string();
    }
    let count = |task: &Value| {
        task.get("do")
            .and_then(Value::as_mapping)
            .map_or(0, Mapping::len)
    };
    let text = |key: &str| task.get(key).map(doc::scalar_text).unwrap_or_default();
    match Kind::of(task) {
        Some(Kind::Cmd) => {
            let words = |command: &Value| {
                command
                    .as_sequence()
                    .map(|items| {
                        items
                            .iter()
                            .map(doc::scalar_text)
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default()
            };
            let mut line = task.get("cmd").map(words).unwrap_or_default();
            for stage in task
                .get("pipe")
                .and_then(Value::as_sequence)
                .into_iter()
                .flatten()
            {
                line.push_str(&format!(" | {}", words(stage)));
            }
            line
        }
        Some(Kind::ForEach) => {
            let item = task.get("as").map_or("ITEM".to_string(), doc::scalar_text);
            format!("{} as {item} · {} tasks", text("for_each"), count(task))
        }
        Some(Kind::While) => {
            let condition = task.get("while").map(doc::flow).unwrap_or_default();
            format!("{condition} · {} tasks", count(task))
        }
        Some(Kind::Trigger) => format!("queue {}", text("trigger")),
        Some(Kind::File) => match task {
            Value::String(path) => path.clone(),
            _ => text("file"),
        },
        None => doc::flow(task),
    }
}

/// One row per task of the list at `path` (Enter opens it), and a row that adds one. A
/// job's own list (`top`) numbers its tasks.
pub fn list(ed: &mut Edit, root: &Value, path: &Path, top: bool) -> Vec<Row> {
    let tasks = node::get(root, path).and_then(Value::as_mapping);
    let mut rows = Vec::new();
    for (index, (key, task)) in tasks.into_iter().flatten().enumerate() {
        let name = doc::key_text(key);
        let entry = path.key(&name);
        let label = if top {
            format!("{}. {name}", index + 1)
        } else {
            format!("  {name}")
        };
        let mut spans = vec![
            kind_span(Kind::of(task)),
            Span::styled(summary(task), theme::text()),
        ];
        if task.get("when").is_some() {
            spans.push(Span::styled("  when", theme::accent()));
        }
        if ed
            .problem
            .is_some_and(|problem| problem.is_on(ed.job, &name))
        {
            spans.push(Span::styled("  ✗ problem", theme::error()));
        }
        if let Some(marker) = ed.run.and_then(|run| run_view::marker(run, ed.job, &name)) {
            spans.push(Span::raw("  "));
            spans.push(marker);
        }
        rows.push(
            Row::field(&label, Line::from(spans), Action::Open(name.clone()))
                .label_style(theme::text())
                .removable(entry.clone())
                .movable(entry.clone())
                .renamable(entry)
                .for_task(&name),
        );
    }
    let empty = Mapping::new();
    let tasks = tasks.unwrap_or(&empty);
    let choices = Kind::ALL
        .into_iter()
        .map(|kind| {
            let name = doc::fresh_key(tasks, &kind.label().to_lowercase().replace(' ', "_"));
            Choice::new(
                kind.label(),
                kind.description(),
                Action::Set(path.key(&name), Some(kind.template(ed.jobs))),
            )
        })
        .collect();
    rows.push(Row::add(
        "add task",
        Action::Pick("Add task".to_string(), choices),
    ));
    rows
}

/// The rows of task `name` in the task list at `tasks`.
pub fn rows(ed: &mut Edit, root: &Value, tasks: &Path, name: &str) -> Vec<Row> {
    let path = tasks.key(name);
    let Some(task) = node::get(root, &path) else {
        return Vec::new();
    };
    let kind = Kind::of(task);
    let mut rows = vec![
        Row::field(
            "task",
            Line::from(vec![
                Span::styled(name.to_string(), theme::bold()),
                Span::styled(format!("   → tasks.{name}"), theme::dim()),
            ]),
            Action::Rename(path.clone()),
        )
        .renamable(path.clone())
        .for_task(name),
    ];
    let when = task.get("when").cloned();
    let choices = Kind::ALL
        .into_iter()
        .map(|option| {
            let mut new = option.template(ed.jobs);
            if let Some(when) = &when {
                doc::as_map(&mut new).insert(Value::from("when"), when.clone());
            }
            Choice::new(
                option.label(),
                "replaces this task's fields (keeps when)",
                Action::Set(path.clone(), Some(new)),
            )
        })
        .collect();
    rows.push(Row::field(
        "kind",
        Line::from(kind_span(kind)),
        Action::Pick("Kind".to_string(), choices),
    ));
    if let Some(problem) = ed.problem.filter(|problem| problem.is_on(ed.job, name)) {
        rows.push(Row::note(Line::styled(
            problem.message.clone(),
            theme::error(),
        )));
    }
    if legacy::is_legacy(task) {
        rows.extend(old_syntax(root, tasks, name, task));
    } else {
        if task.is_mapping() {
            rows.extend(condition::rows(
                root,
                &path.key("when"),
                "when",
                "otherwise skipped",
                true,
            ));
        }
        match kind {
            Some(Kind::Cmd) => rows.extend(task_cmd::rows(root, &path)),
            Some(Kind::ForEach) => rows.extend(task_loop::for_each(ed, root, &path)),
            Some(Kind::While) => rows.extend(task_loop::while_loop(ed, root, &path)),
            Some(Kind::Trigger) => rows.extend(task_trigger::rows(ed, root, &path)),
            Some(Kind::File) => rows.extend(task_file::rows(ed, root, &path)),
            None => rows.push(Row::hint(
                "Unknown task: no cmd, for_each, while, trigger or file. Pick a kind above, or fix it in the file.",
            )),
        }
    }
    if let Some(run) = ed.run {
        rows.extend(run_view::task_rows(run, ed.job, name));
    }
    rows
}

/// The warning on an old-syntax task, what it would become, and its Convert row.
fn old_syntax(root: &Value, tasks: &Path, name: &str, task: &Value) -> Vec<Row> {
    let field = if task.get("if").is_some() {
        "if"
    } else {
        "switch"
    };
    let mut rows = vec![Row::note(Line::styled(
        format!(
            "Old syntax · `{field}` in task `{name}`. if / then / else and switch were removed."
        ),
        theme::accent(),
    ))];
    let flat = legacy::flatten(name, task);
    for (inner, task) in &flat {
        let when = task.get("when").map(doc::flow).unwrap_or_default();
        rows.push(Row::hint(&format!("becomes {inner} · when {when}")));
    }
    let conflict = node::get(root, tasks)
        .and_then(Value::as_mapping)
        .and_then(|tasks| legacy::conflict(tasks, name));
    match conflict {
        Some(conflict) => rows.push(Row::note(Line::styled(conflict, theme::error()))),
        None if !flat.is_empty() => rows.push(Row::field(
            "",
            Line::styled("Convert to when", theme::accent()),
            Action::Convert(tasks.clone(), name.to_string()),
        )),
        None => {}
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every task the add-task picker inserts is valid as is (task files aside, which name
    /// a file that must exist).
    #[test]
    fn templates_are_valid_tasks() {
        let jobs = ["main".to_string()];
        let tasks: Vec<(String, Value)> = Kind::ALL
            .into_iter()
            .filter(|kind| *kind != Kind::File)
            .map(|kind| (kind.label().to_string(), kind.template(&jobs)))
            .collect();
        let mut map = Mapping::new();
        for (name, task) in tasks {
            assert_eq!(Kind::of(&task).map(Kind::label), Some(name.as_str()));
            map.insert(Value::from(name.replace(' ', "_")), task);
        }
        let project = doc::map_of([(
            "jobs",
            doc::map_of([("main", doc::map_of([("tasks", Value::Mapping(map))]))]),
        )]);
        let text = serde_yaml::to_string(&project).unwrap();
        crate::validate::check(std::path::Path::new("project"), &text).unwrap();
    }
}
