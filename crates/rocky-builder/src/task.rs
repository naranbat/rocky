//! A list of task cards (a job's `tasks`, or a loop's `do`): each card's header, kind
//! switching, reordering and deletion, and the add-task picker.

use eframe::egui::{self, Color32, RichText, Stroke};
use serde_yaml::{Mapping, Value};

use crate::chips;
use crate::condition;
use crate::doc;
use crate::edit::Edit;
use crate::run_view;
use crate::theme;
use crate::{task_cmd, task_file, task_loop, task_trigger};

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Cmd,
    ForEach,
    While,
    Trigger,
    File,
}

impl Kind {
    const ALL: [Kind; 5] = [
        Kind::Cmd,
        Kind::ForEach,
        Kind::While,
        Kind::Trigger,
        Kind::File,
    ];

    /// The kind of a task entry: a path string is a task file.
    pub fn of(task: &Value) -> Option<Kind> {
        if task.is_string() {
            return Some(Kind::File);
        }
        let map = task.as_mapping()?;
        [
            ("cmd", Kind::Cmd),
            ("for_each", Kind::ForEach),
            ("while", Kind::While),
            ("trigger", Kind::Trigger),
            ("file", Kind::File),
        ]
        .into_iter()
        .find(|(key, _)| map.contains_key(*key))
        .map(|(_, kind)| kind)
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Cmd => "Command",
            Kind::ForEach => "For each",
            Kind::While => "While",
            Kind::Trigger => "Trigger",
            Kind::File => "Task file",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Kind::Cmd => "Run a program with args, no shell",
            Kind::ForEach => "Repeat do for every list item",
            Kind::While => "Repeat do while a condition holds",
            Kind::Trigger => "Queue a run of another job",
            Kind::File => "Reuse a YAML file, pass with inputs",
        }
    }

    pub fn color(self) -> Color32 {
        match self {
            Kind::Cmd => theme::CMD,
            Kind::ForEach => theme::FOR_EACH,
            Kind::While => theme::WHILE,
            Kind::Trigger => theme::TRIGGER,
            Kind::File => theme::FILE,
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Kind::Cmd => ">_",
            Kind::ForEach => "⇄",
            Kind::While => "↻",
            Kind::Trigger => "⚡",
            Kind::File => "▤",
        }
    }

    /// A new task of this kind.
    fn template(self, jobs: &[String]) -> Value {
        let step = || {
            doc::map_of([(
                "cmd",
                Value::Sequence(vec![Value::from("echo"), Value::from("${ITEM}")]),
            )])
        };
        match self {
            Kind::Cmd => doc::map_of([(
                "cmd",
                Value::Sequence(vec![Value::from("echo"), Value::from("hello")]),
            )]),
            Kind::ForEach => doc::map_of([
                ("for_each", Value::from("tasks.NAME.stdout")),
                ("as", Value::from("ITEM")),
                ("do", doc::map_of([("step", step())])),
            ]),
            Kind::While => doc::map_of([
                (
                    "while",
                    doc::map_of([
                        ("this", Value::from("tasks.check.ok")),
                        ("eq", Value::from(false)),
                    ]),
                ),
                ("attempts", Value::from(10)),
                (
                    "do",
                    doc::map_of([(
                        "check",
                        doc::map_of([("cmd", Value::Sequence(vec![Value::from("true")]))]),
                    )]),
                ),
            ]),
            Kind::Trigger => doc::map_of([(
                "trigger",
                Value::from(jobs.first().cloned().unwrap_or_default()),
            )]),
            Kind::File => doc::map_of([("file", Value::from("tasks/task.yaml"))]),
        }
    }
}

/// A colored kind icon: the kind's glyph on a tinted square.
pub fn icon(ui: &mut egui::Ui, kind: Kind) {
    egui::Frame::new()
        .fill(kind.color())
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(3, 1))
        .show(ui, |ui| {
            ui.label(
                RichText::new(kind.glyph())
                    .color(theme::BG)
                    .strong()
                    .size(11.0),
            );
        });
}

/// What a card asked its list to do.
enum Action {
    Delete,
    MoveUp,
    MoveDown,
    Rename(String),
}

/// Edits a task list: `tasks` of a job or `do` of a loop. `top` is false for loop bodies,
/// which are drawn more compactly and can't hold another loop's numbering.
pub fn list(ui: &mut egui::Ui, ed: &mut Edit, tasks: &mut Value, top: bool) {
    let tasks = doc::as_map(tasks);
    let names = doc::keys(tasks);
    let mut action = None;
    for (index, name) in names.iter().enumerate() {
        let Some(task) = tasks.get_mut(name.as_str()) else {
            continue;
        };
        ui.push_id(("task", index), |ui| {
            ui.horizontal_top(|ui| {
                if top {
                    number(ui, index + 1, task.get("when").is_some());
                }
                ui.vertical(|ui| {
                    if let Some(act) = card(ui, ed, name, task, index, names.len(), top) {
                        action = Some((index, name.clone(), act));
                    }
                });
            });
        });
        ui.add_space(4.0);
    }
    match action {
        Some((_, name, Action::Delete)) => {
            doc::set(tasks, &name, None);
            ed.changed = true;
        }
        Some((index, _, Action::MoveUp)) if index > 0 => {
            doc::swap_with_next(tasks, index - 1);
            ed.changed = true;
        }
        Some((index, _, Action::MoveDown)) => {
            doc::swap_with_next(tasks, index);
            ed.changed = true;
        }
        Some((_, old, Action::Rename(new))) => ed.changed |= doc::rename(tasks, &old, &new),
        _ => {}
    }
    add_row(ui, ed, tasks, top);
}

/// The step number to the left of a top-level card.
fn number(ui: &mut egui::Ui, n: usize, highlighted: bool) {
    let color = if highlighted {
        theme::ACCENT
    } else {
        theme::DIM
    };
    egui::Frame::new()
        .stroke(Stroke::new(1.0, color))
        .corner_radius(12)
        .inner_margin(egui::Margin::symmetric(7, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(n.to_string()).color(color).size(11.0));
        });
}

/// One task card. Returns what the list should do with it, if anything.
fn card(
    ui: &mut egui::Ui,
    ed: &mut Edit,
    name: &str,
    task: &mut Value,
    index: usize,
    count: usize,
    top: bool,
) -> Option<Action> {
    let kind = Kind::of(task);
    let mut action = None;
    let border = if task.get("when").is_some() {
        theme::ACCENT.gamma_multiply(0.6)
    } else {
        theme::BORDER
    };
    theme::card().stroke(Stroke::new(1.0, border)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            if let Some(new) = kind_picker(ui, kind) {
                let when = task.get("when").cloned();
                *task = new.template(ed.jobs);
                if let Some(when) = when {
                    doc::as_map(task).insert(Value::from("when"), when);
                }
                ed.changed = true;
            }
            if let Some(new) = chips::text(ui, "name", name, "name", 40.0, None)
                && !new.is_empty()
            {
                action = Some(Action::Rename(new));
            }
            theme::hint(ui, &format!("→ tasks.{name}"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("🗑").on_hover_text("Delete task").clicked() {
                    action = Some(Action::Delete);
                }
                if top
                    && ui
                        .add_enabled(ed.can_run, egui::Button::new("▶").small())
                        .on_hover_text("Run the job up to this task")
                        .on_disabled_hover_text("Fix the errors, or wait for the current run")
                        .clicked()
                {
                    ed.run_until = Some(name.to_string());
                }
                if ui.add_enabled(index + 1 < count, egui::Button::new("↓").small()).on_hover_text("Move down").clicked() {
                    action = Some(Action::MoveDown);
                }
                if ui.add_enabled(index > 0, egui::Button::new("↑").small()).on_hover_text("Move up").clicked() {
                    action = Some(Action::MoveUp);
                }
            });
        });
        if let Some(map) = task.as_mapping_mut() {
            condition::when_block(ui, ed, map, "otherwise skipped");
        }
        match kind {
            Some(Kind::Cmd) => task_cmd::body(ui, ed, doc::as_map(task)),
            Some(Kind::ForEach) => task_loop::for_each(ui, ed, doc::as_map(task)),
            Some(Kind::While) => task_loop::while_loop(ui, ed, doc::as_map(task)),
            Some(Kind::Trigger) => task_trigger::body(ui, ed, doc::as_map(task)),
            Some(Kind::File) => task_file::body(ui, ed, task),
            None => {
                theme::hint(ui, "Unknown task: no cmd, for_each, while, trigger or file. Pick a kind above, or fix it in the file.");
                ui.label(RichText::new(doc::flow(task)).color(theme::ERROR));
            }
        }
        if let Some(run) = ed.run {
            run_view::task_strip(ui, run, ed.job, name);
        }
    });
    action
}

/// The kind dropdown in a card's header. Returns the newly picked kind.
fn kind_picker(ui: &mut egui::Ui, kind: Option<Kind>) -> Option<Kind> {
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if let Some(kind) = kind {
            icon(ui, kind);
        }
        ui.menu_button(
            RichText::new(format!("{} ⏷", kind.map_or("Unknown", Kind::label))).strong(),
            |ui| {
                for option in Kind::ALL {
                    if ui
                        .add_enabled(Some(option) != kind, egui::Button::new(option.label()))
                        .on_hover_text("Replaces this task's fields (keeps when)")
                        .clicked()
                    {
                        picked = Some(option);
                        ui.close();
                    }
                }
            },
        );
    });
    picked
}

/// The `+ ADD TASK` picker and, on a job, `USE TASK FILE`.
fn add_row(ui: &mut egui::Ui, ed: &mut Edit, tasks: &mut Mapping, top: bool) {
    let mut add = None;
    ui.horizontal(|ui| {
        ui.menu_button(RichText::new("+ ADD TASK").strong().size(12.0), |ui| {
            for kind in Kind::ALL {
                let clicked = ui
                    .horizontal(|ui| {
                        icon(ui, kind);
                        ui.vertical(|ui| {
                            let response =
                                ui.selectable_label(false, RichText::new(kind.label()).strong());
                            theme::hint(ui, kind.description());
                            response.clicked()
                        })
                        .inner
                    })
                    .inner;
                if clicked {
                    add = Some(kind);
                    ui.close();
                }
            }
            ui.separator();
            theme::hint(ui, "Need a condition? Add `when` to any task.");
        });
        if top && chips::add_button(ui, "USE TASK FILE").clicked() {
            add = Some(Kind::File);
        }
    });
    if let Some(kind) = add {
        let name = doc::fresh_key(tasks, &kind.label().to_lowercase().replace(' ', "_"));
        tasks.insert(Value::from(name), kind.template(ed.jobs));
        ed.changed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every task the add-task picker inserts is valid as is (task files aside, which name
    /// a file that must exist).
    #[test]
    fn templates_are_valid_tasks() {
        let jobs = ["main".to_string()];
        let tasks: Vec<(String, Value)> = Kind::ALL
            .into_iter()
            .filter(|kind| *kind != Kind::File)
            .map(|kind| (kind.label().to_string(), kind.template(&jobs)))
            .collect();
        let mut map = Mapping::new();
        for (name, task) in tasks {
            assert_eq!(Kind::of(&task).map(Kind::label), Some(name.as_str()));
            map.insert(Value::from(name.replace(' ', "_")), task);
        }
        let project = doc::map_of([(
            "jobs",
            doc::map_of([("main", doc::map_of([("tasks", Value::Mapping(map))]))]),
        )]);
        let text = serde_yaml::to_string(&project).unwrap();
        crate::validate::check(std::path::Path::new("project.yaml"), &text).unwrap();
    }
}
