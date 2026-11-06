//! The job editor: header, schedule, values, defaults, `when`, and tasks. Jobs kept in their
//! own file show only what the project entry sets (`file`, `schedule`, `values`).

use eframe::egui::{self, RichText};
use serde_yaml::{Mapping, Value};

use crate::chips;
use crate::condition;
use crate::doc;
use crate::edit::Edit;
use crate::project;
use crate::run_view;
use crate::schedule;
use crate::task;
use crate::theme;

/// What the job editor asked the app to do with the job.
pub enum Action {
    Rename(String),
    Delete,
}

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

pub fn editor(ui: &mut egui::Ui, ed: &mut Edit, name: &str, job: &mut Value) -> Option<Action> {
    let file = match &*job {
        Value::String(path) => Some(path.clone()),
        job => job.get("file").map(doc::scalar_text),
    };
    let action = header(ui, name, job, file.as_deref());
    if let Some(run) = ed.run {
        run_view::job_status(ui, run, name);
    }
    ui.add_space(8.0);
    match file {
        Some(file) => file_job(ui, ed, job, &file),
        None => inline_job(ui, ed, doc::as_map(job)),
    }
    action
}

/// The job's icon, editable name, kind line and delete button.
fn header(ui: &mut egui::Ui, name: &str, job: &Value, file: Option<&str>) -> Option<Action> {
    let mut action = None;
    ui.horizontal(|ui| {
        egui::Frame::new()
            .fill(theme::ACCENT)
            .corner_radius(8)
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.label(RichText::new("◷").color(theme::BG).size(20.0));
            });
        ui.vertical(|ui| {
            ui.style_mut().override_text_style = Some(egui::TextStyle::Heading);
            if let Some(new) = chips::text(ui, "job name", name, "job name", 120.0, None)
                && !new.is_empty()
            {
                action = Some(Action::Rename(new));
            }
            ui.style_mut().override_text_style = None;
            let kind = match file {
                Some(file) => format!("JOB FILE · {file}"),
                None => "INLINE JOB".to_string(),
            };
            theme::hint(ui, &format!("{kind} · {}", summary(job).to_uppercase()));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button(RichText::new("🗑 Delete job").color(theme::DIM))
                .clicked()
            {
                action = Some(Action::Delete);
            }
        });
    });
    action
}

fn inline_job(ui: &mut egui::Ui, ed: &mut Edit, job: &mut Mapping) {
    theme::section(ui, "SCHEDULE", "when `rocky run` runs this job");
    schedule::editor(ui, ed, job);
    ui.add_space(8.0);

    theme::section(ui, "VALUES", "read as ${NAME}");
    values(ui, ed, job);
    ui.add_space(8.0);

    if let Some(defaults) = job.get_mut("defaults") {
        theme::section(
            ui,
            "DEFAULTS",
            "this job's cmd tasks get these unless they set them",
        );
        if project::defaults_editor(ui, ed, doc::as_map(defaults)) {
            doc::set(job, "defaults", None);
            ed.changed = true;
        }
        ui.add_space(8.0);
    }

    theme::section(ui, "JOB WHEN", "optional · checked before the first task");
    if job.contains_key("when") {
        condition::when_block(ui, ed, job, "false → no task runs");
    } else {
        ui.horizontal(|ui| {
            if chips::add_button(ui, "ADD JOB CONDITION").clicked() {
                job.insert(Value::from("when"), condition::new_leaf());
                ed.changed = true;
            }
            theme::hint(
                ui,
                "Reads values, .env and environment only; no tasks.* yet.",
            );
        });
    }
    if !job.contains_key("defaults") && chips::add_button(ui, "job defaults").clicked() {
        job.insert(
            Value::from("defaults"),
            doc::map_of([("timeout", Value::from("30s"))]),
        );
        ed.changed = true;
    }
    ui.add_space(8.0);

    let count = job
        .get("tasks")
        .and_then(Value::as_mapping)
        .map_or(0, Mapping::len);
    theme::section(
        ui,
        &format!("TASKS · {count}"),
        "run in order · a task with when runs only if it holds",
    );
    let tasks = job
        .entry(Value::from("tasks"))
        .or_insert_with(|| Value::Mapping(Mapping::new()));
    task::list(ui, ed, tasks, true);
}

/// A project entry pointing at a job file: its path, and the schedule and values it sets
/// over the file's own.
fn file_job(ui: &mut egui::Ui, ed: &mut Edit, job: &mut Value, file: &str) {
    let mut expand = false;
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        chips::row(ui, "file", |ui| {
            if let Some(new) = chips::text(ui, "file", file, "jobs/job.yaml", 160.0, None) {
                match &mut *job {
                    Value::String(path) => *path = new,
                    other => {
                        doc::as_map(other).insert(Value::from("file"), Value::from(new));
                    }
                }
                ed.changed = true;
            }
            theme::hint(ui, "relative to the project file");
        });
        theme::hint(ui, "This job's tasks live in its file; open it in an editor to change them. Editing job files here is planned.");
        // The shorthand `name: path` has no room for overrides; `{file: path}` does.
        if job.is_string() && chips::add_button(ui, "set schedule or values here").clicked() {
            expand = true;
        }
    });
    if expand {
        *job = doc::map_of([("file", Value::from(file))]);
        ed.changed = true;
    }
    let Some(map) = job.as_mapping_mut() else {
        return;
    };
    ui.add_space(8.0);
    theme::section(ui, "SCHEDULE", "set here, it replaces the file's");
    schedule::editor(ui, ed, map);
    ui.add_space(8.0);
    theme::section(ui, "VALUES", "set here, they win over the file's");
    values(ui, ed, map);
}

/// The `values` chips of a job, removing `values` when it ends up empty.
fn values(ui: &mut egui::Ui, ed: &mut Edit, job: &mut Mapping) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        let values = job
            .entry(Value::from("values"))
            .or_insert_with(|| Value::Mapping(Mapping::new()));
        chips::vars(ui, ed, "values", doc::as_map(values), "NAME");
    });
    if job
        .get("values")
        .and_then(Value::as_mapping)
        .is_some_and(Mapping::is_empty)
    {
        job.shift_remove("values");
    }
}
