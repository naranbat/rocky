//! The left sidebar: the project's settings and the list of jobs.

use eframe::egui::{self, RichText, Stroke};
use serde_yaml::{Mapping, Value};

use crate::doc::{self, Doc};
use crate::job;
use crate::project;
use crate::theme;
use crate::validate::Problem;

/// What the main area shows.
#[derive(Clone, PartialEq)]
pub enum Selection {
    Project,
    Job(String),
}

/// Draws the sidebar. Returns `true` when it changed the document (a job was added).
pub fn show(
    ui: &mut egui::Ui,
    doc: &mut Doc,
    selection: &mut Selection,
    problem: Option<&Problem>,
) -> bool {
    let mut changed = false;
    ui.add_space(8.0);
    theme::hint(ui, "PROJECT");
    let values = doc
        .root
        .get("values")
        .and_then(Value::as_mapping)
        .map_or(0, Mapping::len);
    let env_file = doc
        .root
        .get("env_file")
        .map(doc::scalar_text)
        .unwrap_or_else(|| "none".into());
    let defaults = project::defaults_summary(&doc.root);
    let on_project = *selection == Selection::Project;
    for (label, value) in [
        ("Values", values.to_string()),
        ("Env file", env_file),
        ("Defaults", defaults),
    ] {
        if row(ui, label, &value, on_project).clicked() {
            *selection = Selection::Project;
        }
    }
    ui.add_space(16.0);

    let names = doc.job_names();
    ui.horizontal(|ui| {
        theme::hint(ui, &format!("JOBS · {}", names.len()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("+").on_hover_text("Add a job").clicked() {
                let jobs = doc.jobs_mut();
                let name = doc::fresh_key(jobs, "new_job");
                jobs.insert(Value::from(name.clone()), new_job());
                *selection = Selection::Job(name);
                changed = true;
            }
        });
    });
    egui::ScrollArea::vertical().show(ui, |ui| {
        for name in &names {
            let Some(job) = doc
                .root
                .get("jobs")
                .and_then(|jobs| jobs.get(name.as_str()))
            else {
                continue;
            };
            let selected = *selection == Selection::Job(name.clone());
            let broken = problem.is_some_and(|problem| problem.job.as_ref() == Some(name));
            if job_item(ui, name, job, selected, broken).clicked() {
                *selection = Selection::Job(name.clone());
            }
        }
    });
    changed
}

/// A job with one starter task, as added by `+`.
fn new_job() -> Value {
    doc::map_of([(
        "tasks",
        doc::map_of([(
            "hello",
            doc::map_of([(
                "cmd",
                Value::Sequence(vec![Value::from("echo"), Value::from("hello")]),
            )]),
        )]),
    )])
}

/// A clickable `label ……… value` row.
fn row(ui: &mut egui::Ui, label: &str, value: &str, selected: bool) -> egui::Response {
    ui.horizontal(|ui| {
        ui.set_width(ui.available_width());
        let color = if selected { theme::ACCENT } else { theme::TEXT };
        ui.label(RichText::new(label).color(color));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            theme::hint(ui, value);
        });
    })
    .response
    .interact(egui::Sense::click())
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// One job in the list: its name, and a dot (filled when scheduled, a cross when it has a
/// problem) with its summary.
fn job_item(
    ui: &mut egui::Ui,
    name: &str,
    job: &Value,
    selected: bool,
    broken: bool,
) -> egui::Response {
    let scheduled = job.get("schedule").is_some();
    let frame = if selected {
        egui::Frame::new()
            .fill(theme::CARD)
            .stroke(Stroke::new(1.0, theme::BORDER))
    } else {
        egui::Frame::new()
    };
    frame
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(name).strong());
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let (dot, color) = if broken {
                    ("✖", theme::ERROR)
                } else if scheduled {
                    ("●", theme::OK)
                } else {
                    ("○", theme::DIM)
                };
                ui.label(RichText::new(dot).color(color).size(10.0));
                theme::hint(ui, &job::summary(job));
            });
        })
        .response
        .interact(egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}
