//! The job rows: header, schedule, values, defaults, and tasks.

use ratatui::text::{Line, Span};
use rocky_core::project::JOBS_DIR;
use serde_yaml::{Mapping, Value};

use crate::edit::Edit;
use crate::form::{Action, Row};
use crate::node::{self, Path};
use crate::project;
use crate::run_view;
use crate::schedule;
use crate::task;
use crate::theme;
use crate::vars;

/// The job list's one-line description of a job.
pub fn summary(job: &Value) -> String {
    let schedule = schedule::summary(job.get("schedule"));
    format!("{schedule} · {} tasks", task_count(job))
}

/// How many tasks a job has, nested ones aside.
pub fn task_count(job: &Value) -> usize {
    job.get("tasks")
        .and_then(Value::as_mapping)
        .map_or(0, Mapping::len)
}

/// Where job `name` sits in the document.
pub fn path(name: &str) -> Path {
    Path::default().key("jobs").key(name)
}

pub fn rows(ed: &mut Edit, root: &Value, name: &str) -> Vec<Row> {
    let path = path(name);
    let Some(job) = node::get(root, &path) else {
        return Vec::new();
    };
    let kind = format!("{JOBS_DIR}/{name}.yaml");
    let mut rows = vec![
        Row::field(
            "job",
            Line::from(vec![
                Span::styled(name.to_string(), theme::bold()),
                Span::styled(format!("   {kind} · {}", summary(job)), theme::dim()),
            ]),
            Action::Rename(path.clone()),
        )
        .renamable(path.clone()),
    ];
    if let Some(problem) = ed
        .problem
        .filter(|p| p.job.as_deref() == Some(name) && p.task.is_none())
    {
        rows.push(Row::note(Line::styled(
            problem.message.clone(),
            theme::error(),
        )));
    }
    if let Some(line) = ed.run.and_then(|run| run_view::job_status(run, name)) {
        rows.push(Row::note(line));
    }
    rows.extend(job_rows(ed, root, &path, job));
    rows
}

fn job_rows(ed: &mut Edit, root: &Value, path: &Path, job: &Value) -> Vec<Row> {
    let mut rows = vec![Row::heading("SCHEDULE", "")];
    rows.extend(schedule::rows(root, &path.key("schedule")));
    rows.push(Row::heading("VALUES", ""));
    rows.extend(vars::rows(root, &path.key("values"), "value"));
    rows.push(Row::heading("DEFAULTS", ""));
    rows.extend(project::defaults(
        root,
        &path.key("defaults"),
        "job defaults",
    ));
    let count = job
        .get("tasks")
        .and_then(Value::as_mapping)
        .map_or(0, Mapping::len);
    rows.push(Row::heading(&format!("TASKS · {count}"), ""));
    rows.extend(task::list(ed, root, &path.key("tasks"), true));
    rows
}
