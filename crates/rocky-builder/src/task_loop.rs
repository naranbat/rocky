//! The bodies of `for_each` and `while` task cards: their own fields and a nested `do` list.

use eframe::egui::{self, RichText};
use serde_yaml::{Mapping, Value};

use crate::chips;
use crate::condition;
use crate::doc;
use crate::edit::Edit;
use crate::scope;
use crate::task;
use crate::theme;

pub fn for_each(ui: &mut egui::Ui, ed: &mut Edit, map: &mut Mapping) {
    theme::inset().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new("for each in").color(theme::DIM));
            chips::var_field(ui, ed, map, "for_each", "tasks.NAME.stdout", 140.0);
            ui.label(RichText::new("as").color(theme::DIM));
            chips::str_field(ui, ed, map, "as", "ITEM", 40.0);
        });
        theme::hint(
            ui,
            "a list, or a JSON array as text · as is optional, default ITEM",
        );
    });
    on_failure(ui, ed, map, "skip to next item");
    let item = doc::get_str(map, "as").unwrap_or("ITEM").to_string();
    let at = ed.scope.len();
    ed.scope.push(scope::loop_item(&item));
    body(ui, ed, map, theme::FOR_EACH);
    // The item is gone after the loop; what its tasks left in `tasks.*` stays.
    ed.scope.remove(at);
}

pub fn while_loop(ui: &mut egui::Ui, ed: &mut Edit, map: &mut Mapping) {
    theme::inset().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_top(|ui| {
            ui.label(RichText::new("while").color(theme::DIM));
            ui.vertical(|ui| {
                let condition = map
                    .entry(Value::from("while"))
                    .or_insert_with(condition::new_leaf);
                // The loop needs its condition, so its delete button does nothing.
                condition::editor(ui, ed, "while", condition);
            });
        });
        theme::hint(ui, "checked before each run");
    });
    theme::inset().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new("at most").color(theme::DIM));
            chips::number_field(ui, ed, map, "attempts", "∞");
            ui.label(RichText::new("attempts, then fail").color(theme::DIM));
        });
    });
    on_failure(ui, ed, map, "go on with the next run");
    body(ui, ed, map, theme::WHILE);
}

/// The `on_failure` picker: `stop` (the default) or `next`.
fn on_failure(ui: &mut egui::Ui, ed: &mut Edit, map: &mut Mapping, next: &str) {
    let current = doc::get_str(map, "on_failure")
        .unwrap_or("stop")
        .to_string();
    let mut chosen = current.clone();
    ui.horizontal(|ui| {
        ui.label(RichText::new("when a do task fails").color(theme::DIM));
        egui::ComboBox::from_id_salt("on_failure")
            .selected_text(if current == "next" {
                next
            } else {
                "stop the loop"
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut chosen, "stop".to_string(), "stop the loop");
                ui.selectable_value(&mut chosen, "next".to_string(), next);
            });
    });
    if chosen != current {
        doc::set(
            map,
            "on_failure",
            (chosen == "next").then(|| Value::from("next")),
        );
        ed.changed = true;
    }
}

/// The `DO · n TASKS` badge and the nested task list.
fn body(ui: &mut egui::Ui, ed: &mut Edit, map: &mut Mapping, color: egui::Color32) {
    let tasks = map
        .entry(Value::from("do"))
        .or_insert_with(|| Value::Mapping(Mapping::new()));
    let count = tasks.as_mapping().map_or(0, Mapping::len);
    let plural = if count == 1 { "TASK" } else { "TASKS" };
    theme::badge(ui, None, &format!("DO · {count} {plural}"), color);
    ui.push_id("do", |ui| task::list(ui, ed, tasks, false));
}
