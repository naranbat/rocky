#![forbid(unsafe_code)]

use eframe::egui;

fn main() -> eframe::Result {
    eframe::run_native(
        "Rocky Builder",
        eframe::NativeOptions::default(),
        Box::new(|_| Ok(Box::new(Builder))),
    )
}

struct Builder;

impl eframe::App for Builder {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Rocky Builder");
        });
    }
}
