//! The body of a command task card: `cmd`, option chips, and any optional fields it sets.

use eframe::egui::{self, RichText};
use serde_yaml::{Mapping, Value};

use crate::chips;
use crate::condition;
use crate::doc;
use crate::edit::Edit;
use crate::icons::Icon;
use crate::theme;
use crate::widgets;

/// Optional fields offered by the `+` menu, with the value each starts with.
fn optional() -> [(&'static str, Value); 10] {
    [
        ("stdin", Value::from("")),
        ("env", doc::map_of([("NAME", Value::from(""))])),
        ("cwd", Value::from(".")),
        (
            "pipe",
            Value::Sequence(vec![Value::Sequence(vec![Value::from("cat")])]),
        ),
        ("stdout_file", Value::from("out.json")),
        ("capture_stdout", Value::from(false)),
        ("timeout", Value::from("30s")),
        (
            "retry",
            doc::map_of([("attempts", Value::from(3)), ("delay", Value::from("2s"))]),
        ),
        ("allow_failure", Value::from(true)),
        ("when", condition::new_leaf()),
    ]
}

pub fn body(ui: &mut egui::Ui, ed: &mut Edit, task: &mut Mapping) {
    chips::row(ui, "cmd", |ui| {
        let cmd = task
            .entry(Value::from("cmd"))
            .or_insert_with(|| Value::Sequence(Vec::new()));
        chips::args(ui, ed, cmd);
    });
    if task.contains_key("pipe") {
        pipe(ui, ed, task);
    }
    chips::row(ui, "options", |ui| options(ui, ed, task));
    if let Some(stdin) = task.get_mut("stdin") {
        chips::row(ui, "stdin", |ui| {
            chips::yaml_field(ui, ed, "stdin", stdin);
            theme::hint(ui, "text, or YAML sent as JSON");
        });
    }
    if let Some(env) = task.get_mut("env") {
        chips::row(ui, "env", |ui| {
            chips::vars(ui, ed, "env", doc::as_map(env), "NAME")
        });
    }
    for (key, hint) in [
        ("cwd", "relative to the project file"),
        ("stdout_file", "write stdout here; result has no stdout"),
    ] {
        if task.contains_key(key) {
            chips::row(ui, key, |ui| {
                chips::str_field(ui, ed, task, key, key, 120.0);
                theme::hint(ui, hint);
            });
        }
    }
    if task.contains_key("capture_stdout") {
        chips::row(ui, "capture", |ui| {
            let mut on = task
                .get("capture_stdout")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            if ui.checkbox(&mut on, "capture_stdout").changed() {
                task.insert(Value::from("capture_stdout"), Value::from(on));
                ed.changed = true;
            }
        });
    }
    if let Some(retry) = task.get_mut("retry") {
        let mut remove = false;
        chips::row(ui, "retry", |ui| {
            remove = retry_editor(ui, ed, doc::as_map(retry))
        });
        if remove {
            doc::set(task, "retry", None);
            ed.changed = true;
        }
    }
}

/// Chips for timeout and allow_failure, a retry summary, and the `+` menus.
fn options(ui: &mut egui::Ui, ed: &mut Edit, task: &mut Mapping) {
    ui.horizontal_wrapped(|ui| {
        if task.contains_key("timeout") {
            ui.label(RichText::new("timeout").color(theme::DIM));
            chips::str_field(ui, ed, task, "timeout", "30s", 30.0);
        }
        if let Some(on) = task.get("allow_failure").and_then(Value::as_bool) {
            let text = format!("allow_failure {}", if on { "on" } else { "off" });
            if ui.button(text).on_hover_text("Click to toggle").clicked() {
                task.insert(Value::from("allow_failure"), Value::from(!on));
                ed.changed = true;
            }
        }
        let missing: Vec<(&str, Value)> = optional()
            .into_iter()
            .filter(|(key, _)| !task.contains_key(*key))
            .collect();
        if !missing.is_empty() {
            let label = missing
                .iter()
                .map(|(key, _)| *key)
                .collect::<Vec<_>>()
                .join(" · ");
            ui.menu_button(
                RichText::new(format!("+ {label}"))
                    .color(theme::DIM)
                    .size(12.0),
                |ui| {
                    for (key, value) in missing {
                        if ui.button(key).clicked() {
                            task.insert(Value::from(key), value);
                            ed.changed = true;
                            ui.close();
                        }
                    }
                },
            );
        }
        for key in [
            "timeout",
            "allow_failure",
            "stdin",
            "env",
            "cwd",
            "pipe",
            "stdout_file",
            "capture_stdout",
        ] {
            if task.contains_key(key)
                && ui
                    .small_button(
                        RichText::new(format!("× {key}"))
                            .color(theme::DIM)
                            .size(11.0),
                    )
                    .clicked()
            {
                doc::set(task, key, None);
                ed.changed = true;
            }
        }
    });
}

/// `pipe`: the commands stdout flows through, one row of argument chips each.
fn pipe(ui: &mut egui::Ui, ed: &mut Edit, task: &mut Mapping) {
    let Some(Value::Sequence(stages)) = task.get_mut("pipe") else {
        return;
    };
    let mut remove = None;
    for (index, stage) in stages.iter_mut().enumerate() {
        ui.push_id(("pipe", index), |ui| {
            chips::row(ui, if index == 0 { "pipe" } else { "" }, |ui| {
                ui.label(RichText::new("|").color(theme::DIM));
                chips::args(ui, ed, stage);
                if widgets::icon_button(ui, Icon::Close, theme::DIM, "Remove this stage").clicked()
                {
                    remove = Some(index);
                }
            });
        });
    }
    if let Some(index) = remove {
        stages.remove(index);
        ed.changed = true;
    }
    chips::row(ui, "", |ui| {
        if chips::add_button(ui, "pipe stage").clicked() {
            stages.push(Value::Sequence(vec![Value::from("cat")]));
            ed.changed = true;
        }
    });
}

/// `retry`: attempts, delay, and an optional `until` condition. Returns `true` to remove it.
pub fn retry_editor(ui: &mut egui::Ui, ed: &mut Edit, retry: &mut Mapping) -> bool {
    let mut remove = false;
    theme::inset().show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("run up to").color(theme::DIM));
            chips::number_field(ui, ed, retry, "attempts", "3");
            ui.label(RichText::new("times, waiting").color(theme::DIM));
            chips::str_field(ui, ed, retry, "delay", "0s", 30.0);
            if widgets::icon_button(ui, Icon::Close, theme::DIM, "Remove retry").clicked() {
                remove = true;
            }
        });
        if let Some(until) = retry.get_mut("until") {
            let mut delete = false;
            ui.horizontal_top(|ui| {
                ui.label(RichText::new("until").color(theme::DIM));
                ui.vertical(|ui| delete = condition::editor(ui, ed, "until", until));
            });
            if delete {
                doc::set(retry, "until", None);
                ed.changed = true;
            }
        } else {
            ui.horizontal(|ui| {
                if chips::add_button(ui, "until").clicked() {
                    retry.insert(Value::from("until"), condition::new_leaf());
                    ed.changed = true;
                }
                theme::hint(ui, "Without until, a run is enough when it exits 0.");
            });
        }
    });
    remove
}
