//! The start screen, shown until a project is open: open one with the system's file panel,
//! create a new one from rocky's starter project, or type a path.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Key, RichText};
use rocky_core::new::{self, Kind, Level};

use crate::theme;

/// What the start screen asked for.
pub enum Pick {
    Open(PathBuf),
    /// Creating a new project failed.
    Failed(String),
}

/// Draws the start screen. `path` is the typed path, kept between frames; `error` is the
/// last failure to show.
pub fn show(ui: &mut egui::Ui, path: &mut String, error: Option<&str>) -> Option<Pick> {
    let mut pick = None;
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(theme::BG)
                .inner_margin(egui::Margin::same(40)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                egui::Frame::new()
                    .fill(theme::ACCENT)
                    .corner_radius(6)
                    .inner_margin(egui::Margin::symmetric(7, 3))
                    .show(ui, |ui| {
                        ui.label(RichText::new("⛰").color(theme::BG).strong());
                    });
                ui.heading("rocky builder");
            });
            theme::hint(ui, "Build, run and schedule rocky projects.");
            ui.add_space(20.0);

            ui.horizontal(|ui| {
                if choice(ui, "Open project…", "Pick a project YAML file") {
                    pick = open_dialog().map(Pick::Open);
                }
                ui.add_space(12.0);
                if choice(ui, "New project…", "Start from rocky's starter project") {
                    pick = new_dialog().map(|created| match created {
                        Ok(path) => Pick::Open(path),
                        Err(error) => Pick::Failed(error),
                    });
                }
            });
            ui.add_space(20.0);

            theme::hint(
                ui,
                "or type a path (also: rocky-builder path/to/project.yaml)",
            );
            ui.horizontal(|ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(path)
                        .hint_text("path/to/project.yaml")
                        .desired_width(420.0),
                );
                let entered =
                    response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
                if (entered || ui.button("Open").clicked()) && !path.trim().is_empty() {
                    pick = Some(Pick::Open(PathBuf::from(path.trim())));
                }
            });
            if let Some(error) = error {
                ui.add_space(8.0);
                ui.label(RichText::new(error).color(theme::ERROR));
            }
        });
    pick
}

/// A large clickable card with a title and a line under it. Returns whether it was clicked.
fn choice(ui: &mut egui::Ui, title: &str, detail: &str) -> bool {
    theme::card()
        .inner_margin(egui::Margin::symmetric(20, 16))
        .show(ui, |ui| {
            ui.set_width(220.0);
            ui.label(RichText::new(title).strong().size(15.0));
            theme::hint(ui, detail);
        })
        .response
        .interact(egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// The system's Open panel, for YAML files. `None` if cancelled.
fn open_dialog() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Open a rocky project")
        .add_filter("YAML", &["yaml", "yml"])
        .pick_file()
}

/// The system's Save panel for a new project, then writes the starter project there.
/// `None` if cancelled; an error if the file exists or can't be written.
fn new_dialog() -> Option<Result<PathBuf, String>> {
    let path = rfd::FileDialog::new()
        .set_title("Create a rocky project")
        .set_file_name("project.yaml")
        .add_filter("YAML", &["yaml", "yml"])
        .save_file()?;
    Some(create(&path))
}

/// Writes rocky's starter project at `path`; never overwrites a file.
fn create(path: &Path) -> Result<PathBuf, String> {
    new::create(Kind::Project, Level::Basic, path)
        .map(|_| path.to_path_buf())
        .map_err(|error| error.chain())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_a_valid_project_and_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new/project.yaml");
        assert_eq!(create(&path).unwrap(), path);
        rocky_core::project::load(&path).unwrap();
        let again = create(&path).unwrap_err();
        assert!(again.contains("already exists"), "{again}");
    }
}
