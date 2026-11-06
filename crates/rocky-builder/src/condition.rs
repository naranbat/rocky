//! The condition editor: `this · operator · value` rows, and `all` / `any` / `not` groups
//! of them, used for `when`, `while` and `retry.until`.

use eframe::egui::{self, RichText, Stroke};
use serde_yaml::{Mapping, Value};

use crate::chips;
use crate::doc;
use crate::edit::Edit;
use crate::theme;

/// Comparison operators: key, label, and whether it compares numbers.
const OPERATORS: [(&str, &str, bool); 9] = [
    ("gt", "Greater than", true),
    ("gte", "At least", true),
    ("lt", "Less than", true),
    ("lte", "At most", true),
    ("eq", "Equal to", false),
    ("ne", "Not equal to", false),
    ("contains", "Contains", false),
    ("matches", "Matches regex", false),
    ("exists", "Exists", false),
];

/// A new single condition, as added by "add condition".
pub fn new_leaf() -> Value {
    doc::map_of([
        ("this", Value::from("tasks.NAME.ok")),
        ("eq", Value::from(true)),
    ])
}

/// The `when` of a task or job, framed and labeled, if it has one.
pub fn when_block(ui: &mut egui::Ui, ed: &mut Edit, map: &mut Mapping, note: &str) {
    let Some(condition) = map.get_mut("when") else {
        return;
    };
    let mut remove = false;
    egui::Frame::new()
        .fill(theme::ACCENT_BG.gamma_multiply(0.6))
        .stroke(Stroke::new(1.0, theme::ACCENT.gamma_multiply(0.4)))
        .corner_radius(6)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                theme::badge(ui, "WHEN", theme::ACCENT);
                theme::hint(ui, &format!("{} · {note}", summary(condition)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button(RichText::new("REMOVE WHEN").color(theme::DIM).size(11.0))
                        .clicked()
                    {
                        remove = true;
                    }
                });
            });
            editor(ui, ed, "when", condition);
        });
    if remove {
        doc::set(map, "when", None);
        ed.changed = true;
    }
}

/// What a condition asks, in a few words.
fn summary(condition: &Value) -> String {
    match group(condition) {
        Some(("all", items)) => format!("run only if all {} hold", items.len()),
        Some((_, items)) => format!("run if any of {} holds", items.len()),
        None if condition.get("not").is_some() => "run unless it holds".to_string(),
        None => "one condition".to_string(),
    }
}

/// The kind and items of an `all` / `any` group (a list is an `all`), or `None` for a
/// single condition or a `not`.
fn group(condition: &Value) -> Option<(&'static str, &Vec<Value>)> {
    match condition {
        Value::Sequence(items) => Some(("all", items)),
        Value::Mapping(map) if map.len() == 1 => match (map.get("all"), map.get("any")) {
            (Some(Value::Sequence(items)), _) => Some(("all", items)),
            (_, Some(Value::Sequence(items))) => Some(("any", items)),
            _ => None,
        },
        _ => None,
    }
}

/// Like [`group`], mutable.
fn group_mut(condition: &mut Value) -> Option<(&'static str, &mut Vec<Value>)> {
    let kind = group(condition)?.0;
    match condition {
        Value::Sequence(items) => Some((kind, items)),
        Value::Mapping(map) => match map.get_mut(kind) {
            Some(Value::Sequence(items)) => Some((kind, items)),
            _ => None,
        },
        _ => None,
    }
}

/// Edits any condition in place. Returns `true` when its delete button was clicked.
pub fn editor(
    ui: &mut egui::Ui,
    ed: &mut Edit,
    salt: impl egui::AsIdSalt,
    condition: &mut Value,
) -> bool {
    ui.push_id(salt, |ui| {
        if group(condition).is_some() {
            group_editor(ui, ed, condition)
        } else if is_not(condition) {
            not_editor(ui, ed, condition)
        } else {
            leaf_editor(ui, ed, condition)
        }
    })
    .inner
}

/// Whether `condition` is `{not: …}`.
fn is_not(condition: &Value) -> bool {
    condition
        .as_mapping()
        .is_some_and(|map| map.len() == 1 && map.contains_key("not"))
}

/// `NOT` followed by the negated condition; clicking `NOT` unwraps it.
fn not_editor(ui: &mut egui::Ui, ed: &mut Edit, condition: &mut Value) -> bool {
    let mut delete = false;
    let mut unwrap = false;
    ui.horizontal_top(|ui| {
        if theme::badge(ui, "NOT", theme::ERROR)
            .interact(egui::Sense::click())
            .on_hover_text("Remove the NOT")
            .clicked()
        {
            unwrap = true;
        }
        ui.vertical(|ui| {
            if let Some(inner) = condition.get_mut("not") {
                delete = editor(ui, ed, "not", inner);
            }
        });
    });
    if unwrap && let Some(inner) = condition.get("not").cloned() {
        *condition = inner;
        ed.changed = true;
    }
    delete
}

/// An `all` / `any` group: a kind picker, its conditions, and add buttons.
fn group_editor(ui: &mut egui::Ui, ed: &mut Edit, condition: &mut Value) -> bool {
    let mut delete = false;
    theme::inset().show(ui, |ui| {
        let Some((kind, _)) = group(condition) else {
            return;
        };
        let mut new_kind = kind;
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("kind")
                .selected_text(RichText::new(kind.to_uppercase()).strong())
                .width(60.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut new_kind, "all", "ALL · every one holds");
                    ui.selectable_value(&mut new_kind, "any", "ANY · at least one holds");
                });
            let count = group(condition).map_or(0, |(_, items)| items.len());
            theme::hint(ui, &format!("{count} conditions"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("🗑").on_hover_text("Delete group").clicked() {
                    delete = true;
                }
                if ui
                    .small_button("NOT")
                    .on_hover_text("Negate this group")
                    .clicked()
                {
                    *condition = doc::map_of([("not", condition.clone())]);
                    ed.changed = true;
                }
            });
        });
        if new_kind != kind
            && let Some((_, items)) = group(condition)
        {
            *condition = doc::map_of([(new_kind, Value::Sequence(items.clone()))]);
            ed.changed = true;
        }
        let Some((_, items)) = group_mut(condition) else {
            return;
        };
        let mut remove = None;
        for (index, item) in items.iter_mut().enumerate() {
            ui.horizontal_top(|ui| {
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    if editor(ui, ed, index, item) {
                        remove = Some(index);
                    }
                });
            });
        }
        if let Some(index) = remove {
            items.remove(index);
            ed.changed = true;
        }
        ui.horizontal(|ui| {
            if chips::add_button(ui, "ADD CONDITION").clicked() {
                items.push(new_leaf());
                ed.changed = true;
            }
            if chips::add_button(ui, "ADD GROUP").clicked() {
                items.push(doc::map_of([(
                    "any",
                    Value::Sequence(vec![new_leaf(), new_leaf()]),
                )]));
                ed.changed = true;
            }
        });
    });
    delete
}

/// One `this · operator · value` row. Several operators make a range (`gt 1, lt 10`).
fn leaf_editor(ui: &mut egui::Ui, ed: &mut Edit, condition: &mut Value) -> bool {
    let mut delete = false;
    let mut wrap_in = None;
    let map = doc::as_map(condition);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        chips::str_field(ui, ed, map, "this", "tasks.NAME.stdout", 120.0);
        ui.label(RichText::new("is").color(theme::DIM));
        let present: Vec<&str> = OPERATORS
            .iter()
            .map(|(key, _, _)| *key)
            .filter(|key| map.contains_key(*key))
            .collect();
        if present.is_empty() {
            map.insert(Value::from("eq"), Value::from(true));
            ed.changed = true;
        }
        for key in &present {
            ui.push_id(key, |ui| {
                operator_editor(ui, ed, map, key, present.len() > 1)
            });
        }
        let exists = map.contains_key("exists");
        if !exists
            && let Some((key, _, _)) = OPERATORS[..8]
                .iter()
                .find(|(key, _, _)| !map.contains_key(*key))
            && ui
                .small_button("+")
                .on_hover_text("Add another operator to make a range")
                .clicked()
        {
            map.insert(Value::from(*key), Value::from(""));
            ed.changed = true;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button("🗑")
                .on_hover_text("Delete condition")
                .clicked()
            {
                delete = true;
            }
            ui.menu_button(RichText::new("⋯").color(theme::DIM), |ui| {
                if ui.button("Negate (NOT)").clicked() {
                    wrap_in = Some("not");
                }
                if ui.button("Wrap in ALL group").clicked() {
                    wrap_in = Some("all");
                }
                if ui.button("Wrap in ANY group").clicked() {
                    wrap_in = Some("any");
                }
            });
        });
    });
    match wrap_in {
        Some("not") => *condition = doc::map_of([("not", condition.clone())]),
        Some(kind) => *condition = doc::map_of([(kind, Value::Sequence(vec![condition.clone()]))]),
        None => {}
    }
    if wrap_in.is_some() {
        ed.changed = true;
    }
    delete
}

/// The operator picker and value of one operator of a leaf.
fn operator_editor(
    ui: &mut egui::Ui,
    ed: &mut Edit,
    map: &mut Mapping,
    key: &str,
    removable: bool,
) {
    let label = OPERATORS
        .iter()
        .find(|(op, _, _)| *op == key)
        .map_or(key, |(_, label, _)| label);
    let mut chosen = key;
    egui::ComboBox::from_id_salt("op")
        .selected_text(label)
        .width(110.0)
        .show_ui(ui, |ui| {
            ui.label(RichText::new("NUMBERS").color(theme::DIM).size(11.0));
            for (op, label, numeric) in OPERATORS {
                if !numeric && op == "eq" {
                    ui.label(RichText::new("ANY VALUE").color(theme::DIM).size(11.0));
                }
                let taken = op != key && map.contains_key(op);
                ui.add_enabled_ui(!taken, |ui| {
                    ui.selectable_value(&mut chosen, op, format!("{label:<14}{op:>9}"));
                });
            }
        });
    if chosen != key {
        if chosen == "exists" {
            map.retain(|k, _| k.as_str() == Some("this"));
            map.insert(Value::from("exists"), Value::from(true));
        } else if key == "exists" {
            map.shift_remove("exists");
            map.insert(Value::from(chosen), Value::from(""));
        } else {
            doc::rename(map, key, chosen);
        }
        ed.changed = true;
        return;
    }
    if key == "exists" {
        let mut exists = map.get("exists").and_then(Value::as_bool).unwrap_or(true);
        egui::ComboBox::from_id_salt("exists")
            .selected_text(exists.to_string())
            .width(60.0)
            .show_ui(ui, |ui| {
                ed.changed |= ui.selectable_value(&mut exists, true, "true").changed();
                ed.changed |= ui.selectable_value(&mut exists, false, "false").changed();
            });
        map.insert(Value::from("exists"), Value::from(exists));
    } else if let Some(value) = map.get_mut(key) {
        chips::scalar_field(ui, ed, "value", value);
        if removable
            && ui
                .small_button(RichText::new("×").color(theme::DIM))
                .on_hover_text("Remove operator")
                .clicked()
        {
            map.shift_remove(key);
            ed.changed = true;
        }
    }
}
