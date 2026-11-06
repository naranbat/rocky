//! The schedule picker: one tab per schedule kind, and that kind's fields.

use eframe::egui::{self, RichText};
use serde_yaml::{Mapping, Value};

use crate::chips;
use crate::doc;
use crate::edit::Edit;
use crate::theme;

const KINDS: [&str; 6] = ["every", "hourly", "daily", "weekly", "at", "none"];
const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

/// The schedule's kind: the key that names it, or `none`.
fn kind(schedule: Option<&Value>) -> &'static str {
    let Some(map) = schedule.and_then(Value::as_mapping) else {
        return "none";
    };
    KINDS[..5]
        .iter()
        .find(|kind| map.contains_key(**kind))
        .copied()
        .unwrap_or("none")
}

/// A one-line summary for the job list: `every 5m`, `daily 02:30`, `no schedule`.
pub fn summary(schedule: Option<&Value>) -> String {
    let Some(map) = schedule.and_then(Value::as_mapping) else {
        return "no schedule".to_string();
    };
    match kind(schedule) {
        "weekly" => {
            let weekly = map.get("weekly");
            let days = weekly
                .and_then(|w| w.get("days"))
                .and_then(Value::as_sequence)
                .map(|days| {
                    days.iter()
                        .map(doc::scalar_text)
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();
            let time = weekly
                .and_then(|w| w.get("time"))
                .map(doc::scalar_text)
                .unwrap_or_default();
            format!("weekly {days} {time}")
        }
        "hourly" => format!(
            "hourly :{:0>2}",
            map.get("hourly").map(doc::scalar_text).unwrap_or_default()
        ),
        "none" => "on start".to_string(),
        kind => format!(
            "{kind} {}",
            map.get(kind).map(doc::scalar_text).unwrap_or_default()
        ),
    }
}

/// A new schedule of `kind`, keeping `run_on_start` from the old one where it applies.
fn template(kind: &str, run_on_start: bool) -> Option<Value> {
    let mut map = match kind {
        "every" => return Some(doc::map_of([("every", Value::from("5m"))])),
        "hourly" => doc::map_of([("hourly", Value::from(0))]),
        "daily" => doc::map_of([("daily", Value::from("09:00"))]),
        "weekly" => doc::map_of([(
            "weekly",
            doc::map_of([
                ("days", Value::Sequence(vec![Value::from("mon")])),
                ("time", Value::from("09:00")),
            ]),
        )]),
        "at" => doc::map_of([("at", Value::from("2030-01-01T09:00:00"))]),
        _ if run_on_start => doc::map_of([]),
        _ => return None,
    };
    if run_on_start {
        doc::as_map(&mut map).insert(Value::from("run_on_start"), Value::from(true));
    }
    Some(map)
}

/// Edits the `schedule` of a job (or job file entry) mapping.
pub fn editor(ui: &mut egui::Ui, ed: &mut Edit, job: &mut Mapping) {
    let current = kind(job.get("schedule"));
    let run_on_start = job
        .get("schedule")
        .and_then(|s| s.get("run_on_start"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            for kind in KINDS {
                let selected = kind == current;
                let text =
                    RichText::new(kind).color(if selected { theme::TEXT } else { theme::DIM });
                if ui.selectable_label(selected, text).clicked() && !selected {
                    doc::set(job, "schedule", template(kind, run_on_start));
                    ed.changed = true;
                }
            }
        });
        ui.add_space(4.0);
        if let Some(schedule) = job.get_mut("schedule") {
            fields(ui, ed, current, doc::as_map(schedule));
            if schedule.as_mapping().is_some_and(Mapping::is_empty) {
                doc::set(job, "schedule", None);
            }
        } else {
            theme::hint(ui, "No schedule: runs only with `rocky exec` or a trigger.");
        }
        if let Some(schedule) = job.get("schedule") {
            ui.add_space(4.0);
            theme::inset().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new(format!("schedule: {}", doc::flow(schedule))).color(theme::WHILE),
                );
            });
        }
    });
}

/// The fields of a schedule of `kind`.
fn fields(ui: &mut egui::Ui, ed: &mut Edit, kind: &str, schedule: &mut Mapping) {
    match kind {
        "every" => {
            chips::row(ui, "run every", |ui| {
                chips::str_field(ui, ed, schedule, "every", "5m", 40.0);
                if schedule.contains_key("delay") {
                    ui.label(RichText::new("first run after").color(theme::DIM));
                    chips::str_field(ui, ed, schedule, "delay", "30s", 40.0);
                } else {
                    theme::hint(ui, "first run right away");
                    if chips::add_button(ui, "delay").clicked() {
                        schedule.insert(Value::from("delay"), Value::from("30s"));
                        ed.changed = true;
                    }
                }
            });
        }
        "hourly" => {
            chips::row(ui, "minute", |ui| {
                chips::number_field(ui, ed, schedule, "hourly", "0");
                theme::hint(ui, "past every hour (0-59)");
            });
        }
        "daily" => {
            chips::row(ui, "time", |ui| {
                chips::str_field(ui, ed, schedule, "daily", "09:30", 50.0);
                theme::hint(ui, "local time zone");
            });
        }
        "weekly" => {
            let weekly = schedule
                .entry(Value::from("weekly"))
                .or_insert_with(|| Value::Mapping(Mapping::new()));
            let weekly = doc::as_map(weekly);
            chips::row(ui, "days", |ui| {
                let days = weekly
                    .entry(Value::from("days"))
                    .or_insert_with(|| Value::Sequence(Vec::new()));
                if !days.is_sequence() {
                    *days = Value::Sequence(Vec::new());
                }
                if let Some(days) = days.as_sequence_mut() {
                    for day in DAYS {
                        let on = days.iter().any(|d| d.as_str() == Some(day));
                        let text =
                            RichText::new(day).color(if on { theme::ACCENT } else { theme::DIM });
                        if ui.selectable_label(on, text).clicked() {
                            if on {
                                days.retain(|d| d.as_str() != Some(day));
                            } else {
                                days.push(Value::from(day));
                                days.sort_by_key(|d| {
                                    DAYS.iter().position(|x| d.as_str() == Some(*x))
                                });
                            }
                            ed.changed = true;
                        }
                    }
                }
            });
            chips::row(ui, "time", |ui| {
                chips::str_field(ui, ed, weekly, "time", "09:30", 50.0);
                theme::hint(ui, "local time zone");
            });
        }
        "at" => {
            chips::row(ui, "at", |ui| {
                chips::str_field(ui, ed, schedule, "at", "2026-11-06T09:30:00", 160.0);
                theme::hint(ui, "once; an offset like +09:00 is optional");
            });
        }
        _ => theme::hint(ui, "Runs once when `rocky run` starts."),
    }
    if kind != "every" {
        let mut on = schedule
            .get("run_on_start")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if ui
            .checkbox(
                &mut on,
                "run_on_start · also run once when `rocky run` starts",
            )
            .changed()
        {
            doc::set(schedule, "run_on_start", on.then(|| Value::from(true)));
            ed.changed = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_are_valid_schedules() {
        for kind in KINDS {
            for run_on_start in [false, true] {
                let Some(schedule) = template(kind, run_on_start) else {
                    continue;
                };
                assert_eq!(super::kind(Some(&schedule)), kind);
                let text = format!(
                    "jobs:\n  a:\n    schedule: {}\n    tasks:\n      t: {{cmd: [echo]}}\n",
                    doc::flow(&schedule)
                );
                crate::validate::check(std::path::Path::new("project.yaml"), &text)
                    .unwrap_or_else(|error| panic!("{kind}: {error}\n{text}"));
            }
        }
    }
}
