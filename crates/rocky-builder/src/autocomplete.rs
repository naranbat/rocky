//! The `${…}` suggestion popup of a text field: shown under the field while the cursor is
//! in an unclosed `${`, driven by ↑ ↓ to choose, ↵ or ⇥ to insert, esc to close, or a click.

use eframe::egui::{
    self, Id, Key, Modifiers, RichText,
    text::{CCursor, CCursorRange},
    text_edit::TextEditState,
};

use crate::complete::{self, Mode, Var};
use crate::theme;

/// One field's popup state, kept between frames.
#[derive(Clone, Default)]
struct State {
    selected: usize,
    /// The cursor's char index when the field last had focus.
    cursor: usize,
    /// The text esc closed the popup on; it stays closed until the text changes.
    dismissed: Option<String>,
    /// Whether the pointer was over the popup, so a click on it, which takes focus from
    /// the field, still lands.
    pointer_in_popup: bool,
}

fn state(ui: &egui::Ui, field: Id) -> State {
    ui.data_mut(|data| data.get_temp(field.with("complete")))
        .unwrap_or_default()
}

fn store(ui: &egui::Ui, field: Id, state: State) {
    ui.data_mut(|data| data.insert_temp(field.with("complete"), state));
}

/// The query start and the suggestions for `buffer` with the cursor at `cursor`, if the
/// popup should show.
fn open<'a>(
    state: &State,
    buffer: &str,
    vars: &'a [Var],
    mode: Mode,
) -> Option<(usize, Vec<&'a Var>)> {
    if state.dismissed.as_deref() == Some(buffer) {
        return None;
    }
    let start = complete::query(buffer, state.cursor, mode)?;
    let prefix: String = buffer
        .chars()
        .skip(start)
        .take(state.cursor - start)
        .collect();
    let found = complete::matches(vars, &prefix);
    // Nothing to offer, or the only match is exactly what is already there.
    if found.is_empty() || (found.len() == 1 && found[0].path == prefix) {
        return None;
    }
    Some((start, found))
}

/// Inserts `var` at the query, moving the field's cursor after it.
fn pick(ui: &egui::Ui, field: Id, buffer: &mut String, start: usize, state: &mut State, var: &Var) {
    let (text, position) = complete::apply(buffer, start, state.cursor, &var.path);
    *buffer = text;
    state.cursor = position;
    state.selected = 0;
    let mut edit = TextEditState::load(ui.ctx(), field).unwrap_or_default();
    edit.cursor
        .set_char_range(Some(CCursorRange::one(CCursor::new(position))));
    edit.store(ui.ctx(), field);
}

/// Before the field is drawn: takes the keys an open popup uses, so the field doesn't see
/// them, and applies ↵ / ⇥. Returns `true` when it changed `buffer`.
pub fn keys(ui: &egui::Ui, field: Id, buffer: &mut String, vars: &[Var], mode: Mode) -> bool {
    if !ui.memory(|memory| memory.has_focus(field)) {
        return false;
    }
    let mut state = state(ui, field);
    let Some((start, found)) = open(&state, buffer, vars, mode) else {
        return false;
    };
    let mut changed = false;
    ui.input_mut(|input| {
        if input.consume_key(Modifiers::NONE, Key::ArrowDown) {
            state.selected = (state.selected + 1) % found.len();
        }
        if input.consume_key(Modifiers::NONE, Key::ArrowUp) {
            state.selected = (state.selected + found.len() - 1) % found.len();
        }
        if input.consume_key(Modifiers::NONE, Key::Escape) {
            state.dismissed = Some(buffer.clone());
        }
        if input.consume_key(Modifiers::NONE, Key::Enter)
            || input.consume_key(Modifiers::NONE, Key::Tab)
        {
            changed = true;
        }
    });
    if changed && let Some(var) = found.get(state.selected.min(found.len() - 1)) {
        let var = (*var).clone();
        pick(ui, field, buffer, start, &mut state, &var);
    }
    store(ui, field, state);
    changed
}

/// After the field is drawn: remembers its cursor, shows the popup under it, and applies a
/// clicked suggestion. Returns `true` when it changed `buffer`.
pub fn popup(
    ui: &mut egui::Ui,
    field: Id,
    buffer: &mut String,
    vars: &[Var],
    mode: Mode,
    response: &egui::Response,
    cursor: Option<usize>,
) -> bool {
    let mut state = state(ui, field);
    let focused = response.has_focus();
    if focused && let Some(cursor) = cursor {
        state.cursor = cursor;
    }
    if !focused && !state.pointer_in_popup {
        store(ui, field, State::default());
        return false;
    }
    let Some((start, found)) = open(&state, buffer, vars, mode) else {
        state.pointer_in_popup = false;
        store(ui, field, state);
        return false;
    };
    let selected = state.selected.min(found.len() - 1);
    let mut clicked = None;
    let shown = egui::Popup::new(
        field.with("popup"),
        ui.ctx().clone(),
        response.rect,
        ui.layer_id(),
    )
    .open(true)
    .close_behavior(egui::PopupCloseBehavior::IgnoreClicks)
    .width(360.0)
    .show(|ui| {
        let mut group = "";
        for (index, var) in found.iter().enumerate() {
            if var.group != group {
                group = &var.group;
                ui.label(RichText::new(group).color(theme::DIM).size(11.0));
            }
            let row = ui
                .horizontal(|ui| {
                    ui.set_width(ui.available_width());
                    let text = RichText::new(&var.path).color(if index == selected {
                        theme::ACCENT
                    } else {
                        theme::TEXT
                    });
                    ui.label(text);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(&var.detail).color(theme::NUMBER).size(12.0));
                    });
                })
                .response
                .interact(egui::Sense::click());
            if index == selected {
                ui.painter().rect_stroke(
                    row.rect.expand(2.0),
                    4,
                    egui::Stroke::new(1.0, theme::BORDER),
                    egui::StrokeKind::Outside,
                );
            }
            if row.clicked() {
                clicked = Some((*var).clone());
            }
        }
        ui.separator();
        theme::hint(ui, "↑ ↓ choose · ↵ insert · esc close");
    });
    state.pointer_in_popup = shown.is_some_and(|shown| shown.response.contains_pointer());
    let changed = match clicked {
        Some(var) => {
            pick(ui, field, buffer, start, &mut state, &var);
            state.pointer_in_popup = false;
            ui.memory_mut(|memory| memory.request_focus(field));
            true
        }
        None => false,
    };
    store(ui, field, state);
    changed
}
