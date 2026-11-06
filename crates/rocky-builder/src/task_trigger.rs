//! The body of a trigger task card: which job to queue.

use eframe::egui::{self, RichText};
use serde_yaml::{Mapping, Value};

use crate::doc;
use crate::edit::Edit;
use crate::theme;

pub fn body(ui: &mut egui::Ui, ed: &mut Edit, map: &mut Mapping) {
    let current = doc::get_str(map, "trigger").unwrap_or_default().to_string();
    let mut chosen = current.clone();
    theme::inset().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new("queue a run of").color(theme::DIM));
            egui::ComboBox::from_id_salt("trigger")
                .selected_text(RichText::new(&current).strong())
                .show_ui(ui, |ui| {
                    for job in ed.jobs {
                        ui.selectable_value(&mut chosen, job.clone(), job);
                    }
                });
            theme::hint(ui, "doesn't wait · saves no result");
        });
    });
    theme::hint(
        ui,
        "A trigger can't fail. Besides when, it takes no other fields.",
    );
    if chosen != current {
        map.insert(Value::from("trigger"), Value::from(chosen));
        ed.changed = true;
    }
}
