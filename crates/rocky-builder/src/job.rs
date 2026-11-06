//! The job rows: header, schedule, values, defaults, and tasks. Jobs kept in their own
//! file show only what the project entry sets (`file`, `schedule`, `values`).

use ratatui::text::{Line, Span};
use serde_yaml::{Mapping, Value};

use crate::doc;
use crate::edit::Edit;
use crate::form::{Action, Field, Row};
use crate::node::{self, Path};
use crate::project;
use crate::run_view;
use crate::schedule;
use crate::task;
use crate::theme;
use crate::vars;

/// The job list's one-line description of a job.
pub fn summary(job: &Value) -> String {
    match job {
        Value::String(path) => format!("no schedule · {path}"),
        job => {
            let schedule = schedule::summary(job.get("schedule"));
            match job.get("file") {
                Some(file) => format!("{schedule} · {}", doc::scalar_text(file)),
                None => {
                    let count = job
                        .get("tasks")
                        .and_then(Value::as_mapping)
                        .map_or(0, Mapping::len);
                    format!("{schedule} · {count} tasks")
                }
            }
        }
    }
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
    let file = match job {
        Value::String(file) => Some(file.clone()),
        job => job.get("file").map(doc::scalar_text),
    };
    let kind = match &file {
        Some(file) => format!("JOB FILE · {file}"),
        None => "INLINE JOB".to_string(),
    };
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
    match file {
        Some(file) => rows.extend(file_job(root, &path, job, &file)),
        None => rows.extend(inline_job(ed, root, &path, job)),
    }
    rows
}

fn inline_job(ed: &mut Edit, root: &Value, path: &Path, job: &Value) -> Vec<Row> {
    let mut rows = vec![Row::heading("SCHEDULE", "when `rocky run` runs this job")];
    rows.extend(schedule::rows(root, &path.key("schedule")));
    rows.push(Row::heading("VALUES", "read as ${NAME}"));
    rows.extend(vars::rows(root, &path.key("values"), "value"));
    rows.push(Row::heading(
        "DEFAULTS",
        "this job's cmd tasks get these unless they set them",
    ));
    rows.extend(project::defaults(
        root,
        &path.key("defaults"),
        "job defaults",
    ));
    let count = job
        .get("tasks")
        .and_then(Value::as_mapping)
        .map_or(0, Mapping::len);
    rows.push(Row::heading(
        &format!("TASKS · {count}"),
        "run in order · a task with when runs only if it holds",
    ));
    rows.extend(task::list(ed, root, &path.key("tasks"), true));
    rows
}

/// A project entry pointing at a job file: its path, and the schedule and values it sets
/// over the file's own.
fn file_job(root: &Value, path: &Path, job: &Value, file: &str) -> Vec<Row> {
    let mut rows = Vec::new();
    match job {
        Value::String(_) => {
            rows.push(Row::edit("file", root, path.clone(), Field::Text));
            // The shorthand `name: path` has no room for overrides; `{file: path}` does.
            rows.push(Row::add(
                "set schedule or values here",
                Action::Set(
                    path.clone(),
                    Some(doc::map_of([("file", Value::from(file))])),
                ),
            ));
        }
        _ => rows.push(Row::edit("file", root, path.key("file"), Field::Text)),
    }
    rows.push(Row::hint(
        "This job's tasks live in its file; open it in an editor to change them.",
    ));
    if job.is_mapping() {
        rows.push(Row::heading("SCHEDULE", "set here, it replaces the file's"));
        rows.extend(schedule::rows(root, &path.key("schedule")));
        rows.push(Row::heading("VALUES", "set here, they win over the file's"));
        rows.extend(vars::rows(root, &path.key("values"), "value"));
    }
    rows
}
