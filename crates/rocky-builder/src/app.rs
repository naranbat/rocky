//! The window: top bar, sidebar, status bar, and the job / project / YAML views.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers, RichText, Stroke};
use serde_yaml::Value;

use crate::doc::{self, Doc};
use crate::edit::Edit;
use crate::job;
use crate::project;
use crate::sidebar::{self, Selection};
use crate::theme;
use crate::validate::Validation;
use crate::yaml_view;

const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);

pub struct App {
    doc: Option<Doc>,
    /// The path typed on the open screen.
    open_path: String,
    selection: Selection,
    validation: Validation,
    show_yaml: bool,
    /// Task file previews, re-read after each save or validate.
    previews: HashMap<PathBuf, String>,
    /// The last save or open message, for the status bar; `Err` in red.
    message: Option<Result<String, String>>,
    title: String,
}

impl App {
    pub fn new(cc: &eframe::CreationContext, path: Option<PathBuf>) -> App {
        App::with_context(&cc.egui_ctx, path)
    }

    fn with_context(ctx: &egui::Context, path: Option<PathBuf>) -> App {
        theme::apply(ctx);
        let mut app = App {
            doc: None,
            open_path: String::new(),
            selection: Selection::Project,
            validation: Validation::default(),
            show_yaml: false,
            previews: HashMap::new(),
            message: None,
            title: String::new(),
        };
        if let Some(path) = path {
            app.open(&path);
        }
        app
    }

    fn open(&mut self, path: &Path) {
        match Doc::open(path) {
            Ok(doc) => {
                self.selection = doc
                    .job_names()
                    .first()
                    .map_or(Selection::Project, |name| Selection::Job(name.clone()));
                self.doc = Some(doc);
                self.validation = Validation::default();
                self.previews.clear();
                self.message = None;
            }
            Err(error) => self.message = Some(Err(format!("{error:#}"))),
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }
}

impl App {
    fn draw(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = self.doc.as_mut() else {
            open_screen(ui, self);
            return;
        };
        let text = match doc.text() {
            Ok(text) => text,
            Err(error) => format!("# cannot write YAML: {error}"),
        };
        let dirty = doc.is_dirty(&text);
        let mut validate_now = false;
        let mut save = ui.input_mut(|input| input.consume_shortcut(&SAVE));

        egui::Panel::top("top")
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::symmetric(16, 10)),
            )
            .show(ui, |ui| {
                let actions = top_bar(
                    ui,
                    doc,
                    &self.selection,
                    &self.validation,
                    dirty,
                    &mut self.show_yaml,
                );
                save |= actions.save;
                validate_now |= actions.validate;
            });
        if let Some(Err(error)) = &self.validation.result {
            egui::Panel::top("errors")
                .frame(
                    egui::Frame::new()
                        .fill(theme::ERROR_BG)
                        .stroke(Stroke::new(1.0, theme::ERROR.gamma_multiply(0.5)))
                        .inner_margin(egui::Margin::symmetric(16, 8)),
                )
                .show(ui, |ui| {
                    ui.label(RichText::new(format!("⚠ {error}")).color(theme::ERROR));
                });
        }
        egui::Panel::bottom("status")
            .frame(
                egui::Frame::new()
                    .fill(theme::SIDEBAR)
                    .inner_margin(egui::Margin::symmetric(16, 6)),
            )
            .show(ui, |ui| {
                status_bar(ui, doc, &self.validation, &self.message)
            });
        let mut changed = false;
        egui::Panel::left("sidebar")
            .default_size(240.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::SIDEBAR)
                    .inner_margin(egui::Margin::symmetric(12, 8)),
            )
            .show(ui, |ui| {
                changed |= sidebar::show(ui, doc, &mut self.selection)
            });

        let jobs = doc.job_names();
        let dir = doc.dir().to_path_buf();
        let mut ed = Edit {
            changed,
            jobs: &jobs,
            dir: &dir,
            previews: &mut self.previews,
        };
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::symmetric(28, 20)),
            )
            .show(ui, |ui| {
                if self.show_yaml {
                    yaml_view::show(ui, &text);
                    return;
                }
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        ui.set_max_width(1100.0);
                        main_view(ui, &mut ed, doc, &mut self.selection);
                    });
            });
        if ed.changed {
            ui.ctx().request_repaint();
        }

        if save {
            let saved = doc
                .text()
                .and_then(|text| doc.save(text))
                .map(|()| format!("saved {}", doc.path.display()))
                .map_err(|error| format!("{error:#}"));
            self.message = Some(saved);
            self.previews.clear();
            validate_now = true;
        }
        if validate_now {
            self.previews.clear();
        }
        let text = doc.text().unwrap_or(text);
        if let Some(wait) = self.validation.update(&doc.path, &text, validate_now) {
            ui.ctx().request_repaint_after(wait);
        }

        let title = window_title(doc, &self.selection, doc.is_dirty(&text));
        if title != self.title {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}

/// What the top bar's buttons asked for.
struct TopActions {
    save: bool,
    validate: bool,
}

fn top_bar(
    ui: &mut egui::Ui,
    doc: &Doc,
    selection: &Selection,
    validation: &Validation,
    dirty: bool,
    show_yaml: &mut bool,
) -> TopActions {
    let mut actions = TopActions {
        save: false,
        validate: false,
    };
    ui.horizontal(|ui| {
        egui::Frame::new()
            .fill(theme::ACCENT)
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(7, 3))
            .show(ui, |ui| {
                ui.label(RichText::new("⛰").color(theme::BG).strong());
            });
        ui.label(RichText::new("rocky").strong().size(15.0));
        let file = file_name(&doc.path);
        let crumb = match selection {
            Selection::Project => format!("{file} / project"),
            Selection::Job(name) => format!("{file} / jobs / {name}"),
        };
        theme::hint(ui, &crumb);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let run = egui::Button::new(RichText::new("▶ Run job").color(theme::BG).strong())
                .fill(theme::ACCENT);
            ui.add_enabled(false, run).on_disabled_hover_text(
                "Running jobs from the builder is coming next; use `rocky exec` for now.",
            );
            if ui.button("Validate").clicked() {
                actions.validate = true;
            }
            if ui.selectable_label(*show_yaml, "<> YAML").clicked() {
                *show_yaml = !*show_yaml;
            }
            let save = if dirty { "● Save ⌘S" } else { "Save ⌘S" };
            if ui.button(save).clicked() {
                actions.save = true;
            }
            match &validation.result {
                Some(Ok(())) => theme::badge(ui, "✔ VALID", theme::OK),
                Some(Err(_)) => theme::badge(ui, "✖ INVALID", theme::ERROR),
                None => theme::badge(ui, "CHECKING", theme::DIM),
            };
        });
    });
    actions
}

fn status_bar(
    ui: &mut egui::Ui,
    doc: &Doc,
    validation: &Validation,
    message: &Option<Result<String, String>>,
) {
    let jobs = doc.root.get("jobs").and_then(Value::as_mapping);
    let scheduled = jobs.map_or(0, |jobs| {
        jobs.values()
            .filter(|job| job.get("schedule").is_some())
            .count()
    });
    ui.horizontal(|ui| {
        theme::hint(ui, &format!("{scheduled} scheduled jobs"));
        match message {
            Some(Ok(text)) => theme::hint(ui, &format!("· {text}")),
            Some(Err(text)) => {
                ui.label(
                    RichText::new(format!("· {text}"))
                        .color(theme::ERROR)
                        .size(12.0),
                );
            }
            None => {}
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let file = file_name(&doc.path);
            let (text, color) = if validation.is_valid() {
                (format!("{file} valid"), theme::OK)
            } else {
                (format!("{file} invalid"), theme::ERROR)
            };
            ui.label(RichText::new(text).color(color).size(12.0));
            theme::hint(ui, &doc.path.display().to_string());
        });
    });
}

/// The project panel or the selected job's editor.
fn main_view(ui: &mut egui::Ui, ed: &mut Edit, doc: &mut Doc, selection: &mut Selection) {
    let name = match selection {
        Selection::Project => {
            project::editor(ui, ed, doc::as_map(&mut doc.root));
            return;
        }
        Selection::Job(name) => name.clone(),
    };
    let jobs = doc.jobs_mut();
    let Some(job_value) = jobs.get_mut(name.as_str()) else {
        *selection = Selection::Project;
        return;
    };
    match job::editor(ui, ed, &name, job_value) {
        Some(job::Action::Rename(new)) => {
            if doc::rename(jobs, &name, &new) {
                *selection = Selection::Job(new);
                ed.changed = true;
            }
        }
        Some(job::Action::Delete) => {
            doc::set(jobs, &name, None);
            *selection = doc::keys(jobs)
                .first()
                .map_or(Selection::Project, |first| Selection::Job(first.clone()));
            ed.changed = true;
        }
        None => {}
    }
}

/// The screen shown before a project is open: a path field and Open.
fn open_screen(ui: &mut egui::Ui, app: &mut App) {
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(theme::BG).inner_margin(egui::Margin::same(40)))
        .show(ui, |ui| {
            ui.heading("rocky builder");
            theme::hint(ui, "Open a project file. You can also pass it as an argument: rocky-builder project.yaml");
            ui.add_space(12.0);
            let mut open = false;
            ui.horizontal(|ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut app.open_path)
                        .hint_text("path/to/project.yaml")
                        .desired_width(420.0),
                );
                open |= response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
                open |= ui.button("Open").clicked();
            });
            if open {
                let path = PathBuf::from(app.open_path.trim());
                app.open(&path);
            }
            if let Some(Err(error)) = &app.message {
                ui.label(RichText::new(error).color(theme::ERROR));
            }
        });
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn window_title(doc: &Doc, selection: &Selection, dirty: bool) -> String {
    let file = file_name(&doc.path);
    let mark = if dirty { " ●" } else { "" };
    match selection {
        Selection::Project => format!("{file}{mark}"),
        Selection::Job(name) => format!("{name} — {file}{mark}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Draws every view (the project, each job, and the YAML) of the project at `path`, and
    /// checks that just looking at it leaves the document unchanged and valid.
    fn view_all(path: &Path) {
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Some(path.to_path_buf()));
        let doc = app.doc.as_ref().unwrap();
        let before = doc.text().unwrap();
        let views: Vec<Selection> = std::iter::once(Selection::Project)
            .chain(doc.job_names().into_iter().map(Selection::Job))
            .collect();
        for view in views {
            for yaml in [false, true] {
                app.selection = view.clone();
                app.show_yaml = yaml;
                for _ in 0..2 {
                    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.draw(ui));
                    output.textures_delta.clear();
                }
            }
        }
        let after = app.doc.as_ref().unwrap().text().unwrap();
        assert_eq!(before, after, "{}", path.display());
        assert!(
            app.validation.is_valid(),
            "{}: {:?}",
            path.display(),
            app.validation.result
        );
    }

    #[test]
    fn viewing_every_example_changes_nothing() {
        let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let mut viewed = 0;
        for entry in std::fs::read_dir(examples).unwrap() {
            let path = entry.unwrap().path();
            if path
                .extension()
                .is_some_and(|extension| extension == "yaml")
            {
                view_all(&path);
                viewed += 1;
            }
        }
        assert!(viewed > 5);
    }

    /// Covers what the examples don't: while, any, exists, weekly and at schedules,
    /// defaults, capture_stdout, and task files with `with` and `when`.
    #[test]
    fn viewing_every_field_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("tasks")).unwrap();
        std::fs::write(
            dir.path().join("tasks/greet.yaml"),
            "cmd: [echo, 'hello ${WHO}']\n",
        )
        .unwrap();
        let path = dir.path().join("project.yaml");
        std::fs::write(
            &path,
            "\
values: {BASE: 'https://example.com'}
defaults: {timeout: 30s, env: {A: b}, retry: {attempts: 2, delay: 1s}}
jobs:
  weekly:
    schedule: {weekly: {days: [mon, fri], time: '09:30'}, run_on_start: true}
    defaults: {cwd: .}
    tasks:
      check: {cmd: [echo, '200'], capture_stdout: true}
      wait:
        while: {any: [{this: '${tasks.check.stdout}', ne: 200}, {not: {this: X, exists: true}}]}
        attempts: 3
        on_failure: next
        do:
          pause: {cmd: [sleep, '0']}
      alice:
        file: tasks/greet.yaml
        with: {WHO: alice}
        when: [{this: '${tasks.check.ok}', eq: true}, {this: '${tasks.check.stdout}', gt: 1, lt: 300}]
      bob: tasks/greet.yaml
  once:
    schedule: {at: '2030-01-01T09:00:00'}
    tasks:
      go: {trigger: weekly}
",
        )
        .unwrap();
        view_all(&path);
    }
}
