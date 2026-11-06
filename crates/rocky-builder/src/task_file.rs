//! The body of a task file card: the file, its `with` inputs, and a preview of the file.

use eframe::egui::{self, RichText};
use serde_yaml::Value;

use crate::chips;
use crate::condition;
use crate::doc;
use crate::edit::Edit;
use crate::theme;

/// Lines of the task file to preview.
const PREVIEW_LINES: usize = 12;

/// `task` is a mapping with `file`, or the `name: path` shorthand string.
pub fn body(ui: &mut egui::Ui, ed: &mut Edit, task: &mut Value) {
    let file = match &*task {
        Value::String(path) => path.clone(),
        other => other.get("file").map(doc::scalar_text).unwrap_or_default(),
    };
    chips::row(ui, "file", |ui| {
        if let Some(new) = chips::text(ui, "file", &file, "tasks/task.yaml", 140.0, None) {
            match &mut *task {
                Value::String(path) => *path = new,
                other => {
                    doc::as_map(other).insert(Value::from("file"), Value::from(new));
                }
            }
            ed.changed = true;
        }
        theme::hint(ui, "relative to this file");
    });
    // `with` and `when` need the mapping form; the shorthand string has neither.
    let mut expand = None;
    match task.as_mapping_mut() {
        Some(map) => {
            chips::row(ui, "with", |ui| {
                let with = map
                    .entry(Value::from("with"))
                    .or_insert_with(|| Value::Mapping(Default::default()));
                chips::vars(ui, ed, "with", doc::as_map(with), "NAME");
            });
            if map
                .get("with")
                .and_then(Value::as_mapping)
                .is_some_and(|with| with.is_empty())
            {
                map.shift_remove("with");
            }
            if !map.contains_key("when") && chips::add_button(ui, "when").clicked() {
                map.insert(Value::from("when"), condition::new_leaf());
                ed.changed = true;
            }
        }
        None => {
            ui.horizontal(|ui| {
                if chips::add_button(ui, "with").clicked() {
                    expand = Some(doc::map_of([
                        ("file", Value::from(file.clone())),
                        ("with", doc::map_of([("NAME", Value::from(""))])),
                    ]));
                }
                if chips::add_button(ui, "when").clicked() {
                    expand = Some(doc::map_of([
                        ("file", Value::from(file.clone())),
                        ("when", condition::new_leaf()),
                    ]));
                }
            });
        }
    }
    if let Some(expanded) = expand {
        *task = expanded;
        ed.changed = true;
    }
    if !file.is_empty() {
        let preview: String = ed
            .preview(&file)
            .lines()
            .take(PREVIEW_LINES)
            .collect::<Vec<_>>()
            .join("\n");
        theme::inset().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(format!("FILE PREVIEW · {file}"))
                    .color(theme::DIM)
                    .size(11.0),
            );
            ui.label(RichText::new(preview).size(12.0));
            theme::hint(
                ui,
                "Only file, with and when go here; edit the rest in the file.",
            );
        });
    }
}
