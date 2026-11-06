//! The project-level panels: values, env file, and defaults.

use eframe::egui::{self, RichText};
use serde_yaml::{Mapping, Value};

use crate::chips;
use crate::doc;
use crate::edit::Edit;
use crate::task_cmd;
use crate::theme;

/// Edits the project's `values`, `env_file` and `defaults`.
pub fn editor(ui: &mut egui::Ui, ed: &mut Edit, project: &mut Mapping) {
    ui.heading("Project");
    theme::hint(ui, "Shared by every job, job files included.");
    ui.add_space(12.0);

    theme::section(
        ui,
        "VALUES",
        "read as ${NAME} by every job; a job's own value wins",
    );
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        let values = project
            .entry(Value::from("values"))
            .or_insert_with(|| Value::Mapping(Mapping::new()));
        chips::vars(ui, ed, "values", doc::as_map(values), "NAME");
    });
    if project
        .get("values")
        .and_then(Value::as_mapping)
        .is_some_and(Mapping::is_empty)
    {
        project.shift_remove("values");
    }
    ui.add_space(12.0);

    theme::section(ui, "ENV FILE", "a .env file shared by all jobs");
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        chips::row(ui, "env_file", |ui| {
            chips::str_field(ui, ed, project, "env_file", ".env", 120.0);
            theme::hint(ui, "relative to the project file");
        });
    });
    ui.add_space(12.0);

    theme::section(
        ui,
        "DEFAULTS",
        "every cmd task gets these unless it sets them",
    );
    match project.get_mut("defaults") {
        Some(defaults) => {
            if defaults_editor(ui, ed, doc::as_map(defaults)) {
                doc::set(project, "defaults", None);
                ed.changed = true;
            }
        }
        None => {
            if chips::add_button(ui, "defaults").clicked() {
                project.insert(
                    Value::from("defaults"),
                    doc::map_of([("timeout", Value::from("30s"))]),
                );
                ed.changed = true;
            }
        }
    }
}

/// Edits a `defaults` mapping (timeout, cwd, env, retry). Returns `true` to remove it.
pub fn defaults_editor(ui: &mut egui::Ui, ed: &mut Edit, defaults: &mut Mapping) -> bool {
    let mut remove = false;
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        chips::row(ui, "timeout", |ui| {
            chips::str_field(ui, ed, defaults, "timeout", "30s", 40.0)
        });
        chips::row(ui, "cwd", |ui| {
            chips::str_field(ui, ed, defaults, "cwd", ".", 80.0)
        });
        chips::row(ui, "env", |ui| {
            let env = defaults
                .entry(Value::from("env"))
                .or_insert_with(|| Value::Mapping(Mapping::new()));
            chips::vars(ui, ed, "env", doc::as_map(env), "NAME");
        });
        if defaults
            .get("env")
            .and_then(Value::as_mapping)
            .is_some_and(Mapping::is_empty)
        {
            defaults.shift_remove("env");
        }
        match defaults.get_mut("retry") {
            Some(retry) => {
                let mut drop = false;
                chips::row(ui, "retry", |ui| {
                    drop = task_cmd::retry_editor(ui, ed, doc::as_map(retry))
                });
                if drop {
                    doc::set(defaults, "retry", None);
                    ed.changed = true;
                }
            }
            None => {
                chips::row(ui, "retry", |ui| {
                    if chips::add_button(ui, "retry").clicked() {
                        defaults.insert(
                            Value::from("retry"),
                            doc::map_of([
                                ("attempts", Value::from(3)),
                                ("delay", Value::from("2s")),
                            ]),
                        );
                        ed.changed = true;
                    }
                });
            }
        }
        if ui
            .small_button(RichText::new("remove defaults").color(theme::DIM))
            .clicked()
        {
            remove = true;
        }
    });
    remove
}

/// The sidebar's one-line summary of the project's defaults.
pub fn defaults_summary(project: &Value) -> String {
    let Some(defaults) = project.get("defaults").and_then(Value::as_mapping) else {
        return "none".to_string();
    };
    match defaults.get("timeout") {
        Some(timeout) => format!("timeout {}", doc::scalar_text(timeout)),
        None => format!("{} set", defaults.len()),
    }
}
