//! The read-only YAML view of the document, as Save would write it.

use eframe::egui::{self, RichText};

use crate::theme;

pub fn show(ui: &mut egui::Ui, text: &str) {
    theme::section(
        ui,
        "YAML",
        "what Save writes · comments in the original file are not kept",
    );
    ui.add_space(4.0);
    egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
        theme::inset().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(text).size(13.0)).selectable(true));
        });
    });
}
