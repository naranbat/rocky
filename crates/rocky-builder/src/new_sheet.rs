//! The New project sheet: a folder name and location, a basic or full starter, an optional
//! `.env`, and a preview of what will be written.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Align2, Color32, Key, RichText, Stroke};
use rocky_core::new::Level;

use crate::home::{expand, tilde};
use crate::theme;

const FILE: &str = "project.yaml";

pub struct NewSheet {
    name: String,
    location: String,
    full: bool,
    env_file: bool,
    open_after: bool,
    /// Why the last Create failed.
    error: Option<String>,
    focused: bool,
}

/// What the sheet asked for.
pub enum Outcome {
    Cancel,
    /// The project was written at `path`; `open` it in the builder.
    Created {
        path: PathBuf,
        open: bool,
    },
}

impl NewSheet {
    pub fn new(location: &Path) -> NewSheet {
        NewSheet {
            name: String::new(),
            location: tilde(location),
            full: false,
            env_file: true,
            open_after: true,
            error: None,
            focused: false,
        }
    }

    fn folder(&self) -> PathBuf {
        expand(&self.location).join(self.name.trim())
    }

    /// Why the project can't be created as entered, if it can't.
    fn problem(&self) -> Option<String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Some("Enter a folder name.".to_string());
        }
        if name.contains('/') || name == "." || name == ".." {
            return Some("A folder name, without `/`.".to_string());
        }
        if self.location.trim().is_empty() {
            return Some("Choose a location.".to_string());
        }
        let path = self.folder().join(FILE);
        path.exists()
            .then(|| format!("{} already exists.", tilde(&path)))
    }

    fn create(&self) -> Result<PathBuf, String> {
        let path = self.folder().join(FILE);
        let level = if self.full { Level::Full } else { Level::Basic };
        rocky_core::new::project(level, &path, self.env_file)
            .map(|()| path)
            .map_err(|error| error.chain())
    }

    /// Draws the sheet over the window; `Some` once it is done.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<Outcome> {
        let id = egui::Id::unique("new-project");
        let mut outcome = None;
        let modal = egui::Modal::new(id)
            .area(egui::Modal::default_area(id).anchor(Align2::CENTER_TOP, egui::vec2(0.0, 0.0)))
            .backdrop_color(Color32::from_black_alpha(150))
            .frame(
                egui::Frame::new()
                    .fill(theme::SIDEBAR)
                    .stroke(Stroke::new(1.0, theme::BORDER))
                    .corner_radius(egui::CornerRadius {
                        nw: 0,
                        ne: 0,
                        sw: 10,
                        se: 10,
                    })
                    .inner_margin(egui::Margin::same(26)),
            )
            .show(ctx, |ui| {
                ui.set_width(626.0);
                outcome = self.contents(ui);
            });
        if modal.should_close() && outcome.is_none() {
            outcome = Some(Outcome::Cancel);
        }
        outcome
    }

    fn contents(&mut self, ui: &mut egui::Ui) -> Option<Outcome> {
        let mut outcome = None;
        let problem = self.problem();
        let mut create = false;

        ui.horizontal(|ui| {
            theme::logo(ui, 28.0);
            ui.label(RichText::new("New project").strong().size(17.0));
        });
        ui.add_space(14.0);

        theme::section(ui, "NAME", "");
        let name = ui.add(
            egui::TextEdit::singleline(&mut self.name)
                .hint_text("automation")
                .desired_width(f32::INFINITY)
                .margin(egui::vec2(10.0, 8.0)),
        );
        if !self.focused {
            name.request_focus();
            self.focused = true;
        }
        if name.changed() {
            self.error = None;
        }
        create |= name.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
        match (&problem, self.name.trim().is_empty()) {
            (Some(problem), false) => {
                ui.label(RichText::new(problem).color(theme::ERROR).size(12.0));
            }
            _ => theme::hint(ui, "Folder name. The project file is always project.yaml."),
        }
        ui.add_space(10.0);

        theme::section(ui, "LOCATION", "");
        ui.horizontal(|ui| {
            let choose = 84.0;
            ui.add(
                egui::TextEdit::singleline(&mut self.location)
                    .desired_width(ui.available_width() - choose - 8.0)
                    .margin(egui::vec2(10.0, 8.0)),
            );
            let button = egui::Button::new(RichText::new("Choose…").strong())
                .min_size(egui::vec2(choose, 34.0));
            if ui.add(button).clicked()
                && let Some(dir) = rfd::FileDialog::new()
                    .set_title("Where to create the project")
                    .set_directory(expand(&self.location))
                    .pick_folder()
            {
                self.location = tilde(&dir);
            }
        });
        ui.add_space(10.0);

        theme::section(ui, "STARTER", "");
        ui.columns(2, |columns| {
            if starter(
                &mut columns[0],
                !self.full,
                "Basic",
                "The smallest project that runs: one job, one task.",
                "job example → echo hello",
            ) {
                self.full = false;
            }
            if starter(
                &mut columns[1],
                self.full,
                "Full",
                "Every field with a comment: schedule, values, defaults, when, a curl request, for_each and while.",
                "for learning or a template",
            ) {
                self.full = true;
            }
        });
        ui.add_space(10.0);

        theme::section(ui, "OPTIONS", "");
        option(
            ui,
            &mut self.env_file,
            "Create a .env file",
            "Adds env_file: .env for secrets you don't commit",
        );
        option(
            ui,
            &mut self.open_after,
            "Open in the builder after creating",
            "Runs rocky validate first",
        );
        ui.add_space(10.0);

        theme::section(ui, "WILL CREATE", "");
        let folder = self.folder();
        theme::inset()
            .fill(theme::BG)
            .inner_margin(egui::Margin::symmetric(14, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 4.0;
                let exists = if folder.is_dir() { "  (exists)" } else { "" };
                ui.label(format!("{}/{exists}", tilde(&folder)));
                let what = if self.full {
                    "job `example` · every field, commented"
                } else {
                    "job `example` · echo hello"
                };
                let env = folder.join(".env");
                let env = self.env_file.then(|| {
                    if env.exists() {
                        "env_file, kept as is"
                    } else {
                        "env_file, empty"
                    }
                });
                let branch = if env.is_some() { "├─" } else { "└─" };
                tree_line(ui, branch, FILE, what);
                if let Some(env) = env {
                    tree_line(ui, "└─", ".env", env);
                }
            });
        if let Some(error) = &self.error {
            ui.add_space(4.0);
            theme::problem(ui, error);
        }

        ui.add_space(14.0);
        ui.separator();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let level = if self.full { " --level full" } else { "" };
            theme::hint(
                ui,
                &format!("rocky new project {}{level}", tilde(&folder.join(FILE))),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let button =
                    egui::Button::new(RichText::new("Create project").color(theme::BG).strong())
                        .fill(theme::ACCENT)
                        .min_size(egui::vec2(0.0, 34.0));
                let clicked = ui
                    .add_enabled(problem.is_none(), button)
                    .on_disabled_hover_text(problem.as_deref().unwrap_or_default())
                    .clicked();
                create |= clicked;
                let cancel = egui::Button::new(RichText::new("Cancel").strong())
                    .min_size(egui::vec2(0.0, 34.0));
                if ui.add(cancel).clicked() {
                    outcome = Some(Outcome::Cancel);
                }
            });
        });

        if create && problem.is_none() {
            match self.create() {
                Ok(path) => {
                    outcome = Some(Outcome::Created {
                        path,
                        open: self.open_after,
                    })
                }
                Err(error) => self.error = Some(error),
            }
        }
        outcome
    }
}

/// A starter card with a radio mark; returns whether it was clicked.
fn starter(ui: &mut egui::Ui, selected: bool, title: &str, detail: &str, note: &str) -> bool {
    let (fill, stroke) = if selected {
        (theme::ACCENT_BG, theme::ACCENT)
    } else {
        (theme::CARD, theme::BORDER)
    };
    let mut clicked = false;
    let card = theme::card()
        .fill(fill)
        .stroke(Stroke::new(1.0, stroke))
        .inner_margin(egui::Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(104.0);
            ui.horizontal_top(|ui| {
                clicked |= ui.radio(selected, "").clicked();
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.label(RichText::new(title).strong().size(14.0));
                    ui.label(RichText::new(detail).color(theme::DIM).size(12.0));
                    ui.add_space(2.0);
                    ui.label(RichText::new(note).size(12.0));
                });
            });
        })
        .response
        .interact(egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    clicked || card.clicked()
}

/// A checkbox with a dim line under its label.
fn option(ui: &mut egui::Ui, checked: &mut bool, label: &str, detail: &str) {
    ui.checkbox(checked, label);
    ui.horizontal(|ui| {
        ui.add_space(ui.spacing().icon_width + ui.spacing().icon_spacing);
        ui.add_space(-ui.spacing().item_spacing.x);
        theme::hint(ui, detail);
    });
}

fn tree_line(ui: &mut egui::Ui, branch: &str, file: &str, what: &str) {
    ui.horizontal(|ui| {
        ui.label(format!("  {branch} {file:<14}"));
        theme::hint(ui, what);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet(dir: &Path, name: &str) -> NewSheet {
        let mut sheet = NewSheet::new(dir);
        sheet.name = name.to_string();
        sheet
    }

    #[test]
    fn creates_the_project_and_env_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut sheet = sheet(dir.path(), "automation");
        sheet.full = true;
        assert_eq!(sheet.problem(), None);
        let path = sheet.create().unwrap();
        assert_eq!(path, dir.path().join("automation/project.yaml"));
        assert!(dir.path().join("automation/.env").is_file());
        rocky_core::project::load(&path).unwrap();
        assert!(sheet.problem().unwrap().contains("already exists"));
    }

    #[test]
    fn checks_the_name() {
        let dir = tempfile::tempdir().unwrap();
        assert!(sheet(dir.path(), " ").problem().is_some());
        assert!(sheet(dir.path(), "a/b").problem().is_some());
        assert!(sheet(dir.path(), "..").problem().is_some());
        assert_eq!(sheet(dir.path(), "ok").problem(), None);
    }
}
