//! Small editing widgets shared by every form: buffered text fields, argument chips and
//! `NAME = value` chips.

use std::sync::Arc;

use eframe::egui::{
    self, Color32, FontId, Galley, Margin, RichText, Stroke, TextBuffer, TextEdit,
    text::{LayoutJob, TextFormat},
};
use serde_yaml::{Mapping, Value};

use crate::autocomplete;
use crate::complete::{Mode, Var};
use crate::doc;
use crate::edit::Edit;
use crate::theme;

/// A one-line text field over text that is derived from the document every frame. While it
/// has focus, the typed text is kept in egui memory, so writing it back (and re-deriving it)
/// on each keystroke never moves the cursor. Returns the new text when it changed.
pub fn text(
    ui: &mut egui::Ui,
    salt: impl egui::AsIdSalt,
    current: &str,
    hint: &str,
    min_width: f32,
    color: Option<Color32>,
) -> Option<String> {
    field(ui, salt, current, hint, min_width, color, None)
}

/// Like [`text`], suggesting `vars` for `${…}` (or, in [`Mode::Whole`], for the whole field).
fn field(
    ui: &mut egui::Ui,
    salt: impl egui::AsIdSalt,
    current: &str,
    hint: &str,
    min_width: f32,
    color: Option<Color32>,
    complete: Option<(&[Var], Mode)>,
) -> Option<String> {
    let id = ui.scope_id().with(salt);
    let field = id.with("field");
    let mut buffer: String = ui
        .data_mut(|data| data.get_temp(id))
        .unwrap_or_else(|| current.to_string());
    let mut picked =
        complete.is_some_and(|(vars, mode)| autocomplete::keys(ui, field, &mut buffer, vars, mode));
    let mut layouter = |ui: &egui::Ui, text: &dyn TextBuffer, _wrap: f32| -> Arc<Galley> {
        ui.fonts_mut(|fonts| fonts.layout_job(highlight(text.as_str(), color)))
    };
    let output = TextEdit::singleline(&mut buffer)
        .id(field)
        .hint_text(RichText::new(hint).color(theme::DIM))
        .desired_width(min_width)
        .clip_text(false)
        .margin(Margin::symmetric(6, 3))
        .layouter(&mut layouter)
        .show(ui);
    if let Some((vars, mode)) = complete {
        let cursor = output.cursor_range.map(|range| range.primary.index.0);
        picked |= autocomplete::popup(ui, field, &mut buffer, vars, mode, &output.response, cursor);
    }
    let changed = output.response.changed() || picked;
    if output.response.has_focus() || picked {
        ui.data_mut(|data| data.insert_temp(id, buffer.clone()));
    } else {
        ui.data_mut(|data| data.remove::<String>(id));
    }
    changed.then_some(buffer)
}

/// A multi-line field over any YAML value: plain text stays a string; text that parses as
/// a YAML mapping or list (such as a JSON body) becomes one.
pub fn yaml_field(ui: &mut egui::Ui, ed: &mut Edit, salt: impl egui::AsIdSalt, value: &mut Value) {
    let current = match &*value {
        Value::Mapping(_) | Value::Sequence(_) => serde_yaml::to_string(&*value)
            .map(|text| text.trim_end().to_string())
            .unwrap_or_default(),
        other => doc::scalar_text(other),
    };
    let id = ui.scope_id().with(salt);
    let mut buffer: String = ui
        .data_mut(|data| data.get_temp(id))
        .unwrap_or_else(|| current.clone());
    let response = ui.add(
        TextEdit::multiline(&mut buffer)
            .id(id.with("field"))
            .desired_rows(1)
            .desired_width(ui.available_width().min(420.0))
            .hint_text(RichText::new("status: up").color(theme::DIM)),
    );
    if response.has_focus() {
        ui.data_mut(|data| data.insert_temp(id, buffer.clone()));
    } else {
        ui.data_mut(|data| data.remove::<String>(id));
    }
    if response.changed() {
        *value = match serde_yaml::from_str::<Value>(&buffer) {
            Ok(parsed @ (Value::Mapping(_) | Value::Sequence(_))) => parsed,
            _ => doc::parse_scalar(&buffer),
        };
        ed.changed = true;
    }
}

/// Lays out `text`, coloring each `${…}` reference in the accent color.
fn highlight(text: &str, color: Option<Color32>) -> LayoutJob {
    let font = FontId::monospace(13.0);
    let plain = TextFormat::simple(font.clone(), color.unwrap_or(theme::TEXT));
    let var = TextFormat::simple(font, theme::ACCENT);
    let mut job = LayoutJob::default();
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        let end = rest[start..]
            .find('}')
            .map_or(rest.len(), |end| start + end + 1);
        job.append(&rest[..start], 0.0, plain.clone());
        job.append(&rest[start..end], 0.0, var.clone());
        rest = &rest[end..];
    }
    job.append(rest, 0.0, plain);
    job
}

/// A text field over the string at `key` of `map`; empty text removes the key.
pub fn str_field(
    ui: &mut egui::Ui,
    ed: &mut Edit,
    map: &mut Mapping,
    key: &str,
    hint: &str,
    width: f32,
) {
    let current = doc::get_str(map, key).unwrap_or_default().to_string();
    let complete = Some((ed.scope.as_slice(), Mode::Inline));
    if let Some(new) = field(ui, key, &current, hint, width, None, complete) {
        doc::set(map, key, (!new.is_empty()).then(|| Value::from(new)));
        ed.changed = true;
    }
}

/// A field whose whole text is a variable path (`this`, `for_each`), with suggestions.
pub fn var_field(
    ui: &mut egui::Ui,
    ed: &mut Edit,
    map: &mut Mapping,
    key: &str,
    hint: &str,
    width: f32,
) {
    let current = doc::get_str(map, key).unwrap_or_default().to_string();
    let complete = Some((ed.scope.as_slice(), Mode::Whole));
    if let Some(new) = field(ui, key, &current, hint, width, None, complete) {
        doc::set(map, key, (!new.is_empty()).then(|| Value::from(new)));
        ed.changed = true;
    }
}

/// A text field over a scalar value: numbers and booleans as typed, anything else a string.
pub fn scalar_field(
    ui: &mut egui::Ui,
    ed: &mut Edit,
    salt: impl egui::AsIdSalt,
    value: &mut Value,
) {
    let color = match &*value {
        Value::Number(_) | Value::Bool(_) => Some(theme::NUMBER),
        _ => None,
    };
    let current = doc::scalar_text(value);
    let complete = Some((ed.scope.as_slice(), Mode::Inline));
    if let Some(new) = field(ui, salt, &current, "value", 40.0, color, complete) {
        *value = doc::parse_scalar(&new);
        ed.changed = true;
    }
}

/// A field for a whole number at `key` (such as `attempts`); empty text removes it.
pub fn number_field(ui: &mut egui::Ui, ed: &mut Edit, map: &mut Mapping, key: &str, hint: &str) {
    let current = map.get(key).map(doc::scalar_text).unwrap_or_default();
    if let Some(new) = text(ui, key, &current, hint, 28.0, Some(theme::NUMBER)) {
        let value = match new.trim().parse::<u64>() {
            Ok(number) => Some(Value::from(number)),
            Err(_) if new.trim().is_empty() => None,
            Err(_) => Some(Value::from(new)),
        };
        doc::set(map, key, value);
        ed.changed = true;
    }
}

/// A command as argument chips: the program first (in the command color), then each
/// argument. An argument left empty is removed when it loses focus.
pub fn args(ui: &mut egui::Ui, ed: &mut Edit, command: &mut Value) {
    if !command.is_sequence() {
        *command = Value::Sequence(Vec::new());
        ed.changed = true;
    }
    let Some(items) = command.as_sequence_mut() else {
        return;
    };
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let mut remove = None;
        for (index, item) in items.iter_mut().enumerate() {
            let current = doc::scalar_text(item);
            let color = (index == 0).then_some(theme::CMD);
            let hint = if index == 0 { "program" } else { "arg" };
            let complete = Some((ed.scope.as_slice(), Mode::Inline));
            if let Some(new) = field(ui, ("arg", index), &current, hint, 16.0, color, complete) {
                *item = Value::from(new);
                ed.changed = true;
            }
            let focused = ui.memory(|memory| {
                memory.has_focus(ui.scope_id().with(("arg", index)).with("field"))
            });
            if current.is_empty() && !focused && index > 0 {
                remove = Some(index);
            }
        }
        if let Some(index) = remove {
            items.remove(index);
            ed.changed = true;
        }
        if ui
            .small_button("+")
            .on_hover_text("Add an argument")
            .clicked()
        {
            items.push(Value::from(""));
            ed.changed = true;
            let id = ui.scope_id().with(("arg", items.len() - 1)).with("field");
            ui.memory_mut(|memory| memory.request_focus(id));
        }
    });
}

/// `NAME = value` chips for a mapping of variables (`values`, `env`, `with`), with remove
/// buttons and a `+` that adds a new name.
pub fn vars(
    ui: &mut egui::Ui,
    ed: &mut Edit,
    salt: impl egui::AsIdSalt,
    map: &mut Mapping,
    base: &str,
) {
    ui.push_id(salt, |ui| {
        ui.horizontal_wrapped(|ui| {
            let names = doc::keys(map);
            let mut remove = None;
            let mut rename = None;
            for (index, name) in names.iter().enumerate() {
                ui.push_id(index, |ui| {
                    egui::Frame::new()
                        .fill(theme::FIELD)
                        .stroke(Stroke::new(1.0, theme::BORDER))
                        .corner_radius(6)
                        .inner_margin(Margin::symmetric(4, 2))
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            if let Some(new) =
                                text(ui, "name", name, "NAME", 24.0, Some(theme::ACCENT))
                            {
                                rename = Some((name.clone(), new));
                            }
                            ui.label(RichText::new("=").color(theme::DIM));
                            if let Some(value) = map.get_mut(name.as_str()) {
                                scalar_field(ui, ed, "value", value);
                            }
                            if ui.small_button("×").on_hover_text("Remove").clicked() {
                                remove = Some(name.clone());
                            }
                        });
                });
            }
            if let Some((old, new)) = rename
                && doc::rename(map, &old, &new)
            {
                ed.changed = true;
            }
            if let Some(name) = remove {
                doc::set(map, &name, None);
                ed.changed = true;
            }
            if ui.small_button("+").on_hover_text("Add a name").clicked() {
                let name = doc::fresh_key(map, base);
                map.insert(Value::from(name), Value::from(""));
                ed.changed = true;
            }
        });
    });
}

/// A small button with a leading `+`, for adding an optional part.
pub fn add_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(
            RichText::new(format!("+ {label}"))
                .color(theme::DIM)
                .size(12.0),
        )
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0, theme::BORDER)),
    )
}

/// A labeled row: a dim fixed-width label, then the contents.
pub fn row<R>(ui: &mut egui::Ui, label: &str, contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.add_sized(
            [72.0, 22.0],
            egui::Label::new(RichText::new(label).color(theme::DIM)),
        );
        contents(ui)
    })
    .inner
}

#[cfg(test)]
mod tests {
    use eframe::egui::{Event, Key, Modifiers, RawInput};

    use super::*;

    fn var(path: &str) -> Var {
        Var {
            path: path.to_string(),
            detail: String::new(),
            group: "G".to_string(),
        }
    }

    fn key(key: Key) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    /// Types into a focused field and picks a suggestion with ↓ and ↵.
    #[test]
    fn typing_dollar_brace_suggests_and_inserts() {
        let ctx = egui::Context::default();
        let vars = [var("BASE"), var("BATCH"), var("tasks.login.ok")];
        let mut value = String::from("x ");
        let frame = |events: Vec<Event>, value: &mut String| {
            let input = RawInput {
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                let id = ui.scope_id().with("f").with("field");
                ui.memory_mut(|memory| memory.request_focus(id));
                if let Some(new) =
                    field(ui, "f", value, "", 40.0, None, Some((&vars, Mode::Inline)))
                {
                    *value = new;
                }
            });
            output.textures_delta.clear();
        };
        frame(vec![], &mut value);
        frame(vec![key(Key::End)], &mut value);
        frame(vec![Event::Text("${BA".to_string())], &mut value);
        assert_eq!(value, "x ${BA");
        frame(vec![key(Key::ArrowDown)], &mut value);
        frame(vec![key(Key::Enter)], &mut value);
        assert_eq!(value, "x ${BATCH}");
    }
}
