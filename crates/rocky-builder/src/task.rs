//! A list of task cards (a job's `tasks`, or a loop's `do`): each card's header, kind
//! switching, reordering and deletion, and the add-task picker.

use eframe::egui::{self, Color32, Popup, RichText, Stroke};
use serde_yaml::{Mapping, Value};

use crate::chips;
use crate::condition;
use crate::doc;
use crate::edit::Edit;
use crate::icons::Icon;
use crate::legacy;
use crate::run_view;
use crate::scope;
use crate::theme;
use crate::widgets;
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

    fn color(self) -> Color32 {
        match self {
            Kind::Cmd => theme::CMD,
            Kind::ForEach => theme::FOR_EACH,
            Kind::While => theme::WHILE,
            Kind::Trigger => theme::TRIGGER,
            Kind::File => theme::FILE,
        }
    }

    fn icon(self) -> Icon {
        match self {
            Kind::Cmd => Icon::Terminal,
            Kind::ForEach => Icon::Loop,
            Kind::While => Icon::While,
            Kind::Trigger => Icon::Bolt,
            Kind::File => Icon::File,
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

/// A colored kind icon: the kind's icon on a tinted square.
fn icon(ui: &mut egui::Ui, kind: Kind, size: f32) {
    widgets::kind_badge(ui, kind.icon(), kind.color(), size);
}

/// What a card asked its list to do.
enum Action {
    Delete,
    Rename(String),
    /// Replace an old-syntax branch or switch with plain tasks.
    Convert,
}

/// A card being dragged by its handle: which list it is in, and where.
#[derive(Clone, Copy, PartialEq)]
struct Dragged {
    list: egui::Id,
    from: usize,
}

/// Edits a task list: `tasks` of a job or `do` of a loop. `top` is false for loop bodies,
/// which are drawn without step numbers or run buttons.
pub fn list(ui: &mut egui::Ui, ed: &mut Edit, tasks: &mut Value, top: bool) {
    let tasks = doc::as_map(tasks);
    let names = doc::keys(tasks);
    let list_id = ui.scope_id().with("tasks");
    let mut action = None;
    let mut moved = None;
    for (index, name) in names.iter().enumerate() {
        let legacy = tasks
            .get(name.as_str())
            .filter(|task| legacy::is_legacy(task))
            .map(|_| legacy::conflict(tasks, name));
        let Some(task) = tasks.get_mut(name.as_str()) else {
            continue;
        };
        let drag = Dragged {
            list: list_id,
            from: index,
        };
        let row = ui.push_id(("task", index), |ui| {
            ui.horizontal_top(|ui| {
                if top {
                    number(ui, index + 1, task.get("when").is_some());
                }
                ui.vertical(|ui| {
                    if let Some(act) = card(ui, ed, name, task, top, drag, legacy) {
                        action = Some((name.clone(), act));
                    }
                });
            })
            .response
        });
        if let Some(from) = drop_target(ui, &row.inner, list_id, index) {
            moved = Some((from, index));
        }
        // Later cards can read what this task leaves in `tasks.NAME`.
        let result = ed
            .run
            .and_then(|run| run.results.task(ed.job, name))
            .and_then(|run| run.last.as_ref())
            .map(|event| &event["result"]);
        if let Some(task) = tasks.get(name.as_str()) {
            let vars = scope::task(name, task, result);
            ed.scope.extend(vars);
        }
        ui.add_space(4.0);
    }
    if let Some((from, to)) = moved {
        doc::move_entry(tasks, from, to);
        ed.changed = true;
    }
    match action {
        Some((name, Action::Delete)) => {
            doc::set(tasks, &name, None);
            ed.changed = true;
        }
        Some((old, Action::Rename(new))) => ed.changed |= doc::rename(tasks, &old, &new),
        Some((name, Action::Convert)) => ed.changed |= legacy::convert(tasks, &name),
        None => {}
    }
    add_row(ui, ed, tasks, top);
}

/// While a card of this list is dragged over the card at `index`, marks where it would
/// land; when it is dropped there, returns where it came from.
fn drop_target(ui: &egui::Ui, row: &egui::Response, list: egui::Id, index: usize) -> Option<usize> {
    let hovered = row.dnd_hover_payload::<Dragged>()?;
    if hovered.list != list || hovered.from == index {
        return None;
    }
    let y = if hovered.from > index {
        row.rect.top() - 2.0
    } else {
        row.rect.bottom() + 2.0
    };
    ui.painter()
        .hline(row.rect.x_range(), y, Stroke::new(2.0, theme::ACCENT));
    row.dnd_release_payload::<Dragged>()
        .map(|dropped| dropped.from)
}

/// The step number to the left of a top-level card, level with its header.
fn number(ui: &mut egui::Ui, n: usize, highlighted: bool) {
    let color = if highlighted {
        theme::ACCENT
    } else {
        theme::DIM
    };
    let (rect, _) = ui.allocate_exact_size(egui::vec2(24.0, 48.0), egui::Sense::hover());
    let center = egui::pos2(rect.center().x, rect.top() + 25.0);
    ui.painter()
        .circle(center, 11.0, theme::BG, Stroke::new(1.0, color));
    ui.painter().text(
        center,
        egui::Align2::CENTER_CENTER,
        n.to_string(),
        theme::font(theme::SMALL),
        color,
    );
}

/// One task card. Returns what the list should do with it, if anything. `legacy` is set
/// for a task in the removed `if` / `switch` syntax: `None` when it can be converted, or why not.
fn card(
    ui: &mut egui::Ui,
    ed: &mut Edit,
    name: &str,
    task: &mut Value,
    top: bool,
    drag: Dragged,
    legacy: Option<Option<String>>,
) -> Option<Action> {
    let kind = Kind::of(task);
    let mut action = None;
    let problem = ed.problem.filter(|problem| problem.is_on(ed.job, name));
    let border = if problem.is_some() {
        theme::ERROR
    } else if task.get("when").is_some() {
        theme::ACCENT.gamma_multiply(0.6)
    } else {
        theme::BORDER
    };
    theme::card().stroke(Stroke::new(1.0, border)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.dnd_drag_source(ui.scope_id().with("drag"), drag, |ui| {
                widgets::icon(ui, Icon::Grip, 14.0, theme::FAINT)
                    .on_hover_cursor(egui::CursorIcon::Grab);
            })
            .response
            .on_hover_text("Drag to reorder");
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
                if widgets::icon_button(ui, Icon::Trash, theme::DIM, "Delete task").clicked() {
                    action = Some(Action::Delete);
                }
                if top
                    && ui
                        .add_enabled_ui(ed.can_run, |ui| {
                            widgets::icon_button(ui, Icon::Play, theme::OK, "Run the job up to this task")
                        })
                        .inner
                        .on_disabled_hover_text("Fix the errors, or wait for the current run")
                        .clicked()
                {
                    ed.run_until = Some(name.to_string());
                }
            });
        });
        if let Some(problem) = problem {
            theme::problem(ui, &problem.message);
        }
        if let Some(conflict) = legacy {
            if old_syntax(ui, name, task, conflict.as_deref()) {
                action = Some(Action::Convert);
            }
            return;
        }
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

/// The warning on an old-syntax task, what it would become, and its Convert button.
/// Returns `true` when Convert was clicked.
fn old_syntax(ui: &mut egui::Ui, name: &str, task: &Value, conflict: Option<&str>) -> bool {
    let mut convert = false;
    let field = if task.get("if").is_some() {
        "if"
    } else {
        "switch"
    };
    egui::Frame::new()
        .fill(theme::ACCENT_BG)
        .stroke(Stroke::new(1.0, theme::ACCENT.gamma_multiply(0.5)))
        .corner_radius(6)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                widgets::icon(ui, Icon::Warning, 12.0, theme::ACCENT);
                ui.label(
                    RichText::new(format!(
                        "Old syntax · `{field}` in task `{name}`. if / then / else and switch were removed."
                    ))
                    .color(theme::ACCENT),
                );
            });
            let flat = legacy::flatten(name, task);
            for (inner, task) in &flat {
                let when = task.get("when").map(doc::flow).unwrap_or_default();
                theme::hint(ui, &format!("becomes {inner} · when {when}"));
            }
            ui.horizontal(|ui| {
                let button = egui::Button::new(RichText::new("Convert to when").color(theme::ACCENT));
                if ui
                    .add_enabled(conflict.is_none() && !flat.is_empty(), button)
                    .on_hover_text("Each task gets the conditions it ran under; they are checked when it is reached")
                    .clicked()
                {
                    convert = true;
                }
                if let Some(conflict) = conflict {
                    ui.label(RichText::new(conflict).color(theme::ERROR).size(12.0));
                }
            });
        });
    convert
}

/// The kind dropdown in a card's header. Returns the newly picked kind.
fn kind_picker(ui: &mut egui::Ui, kind: Option<Kind>) -> Option<Kind> {
    let mut picked = None;
    let id = ui.scope_id().with("kind");
    let leading = kind.map(|kind| (kind.icon(), kind.color()));
    let chip = widgets::chip(ui, id, leading, kind.map_or("Unknown", Kind::label));
    Popup::menu(&chip).show(|ui| {
        for option in Kind::ALL {
            let row = ui.add_enabled_ui(Some(option) != kind, |ui| {
                ui.horizontal(|ui| {
                    icon(ui, option, 16.0);
                    ui.selectable_label(Some(option) == kind, option.label())
                })
                .inner
            });
            if row
                .inner
                .on_hover_text("Replaces this task's fields (keeps when)")
                .clicked()
            {
                picked = Some(option);
                ui.close();
            }
        }
    });
    picked
}

/// The `+ ADD TASK` picker and, on a job, `USE TASK FILE`.
fn add_row(ui: &mut egui::Ui, ed: &mut Edit, tasks: &mut Mapping, top: bool) {
    let mut add = None;
    ui.horizontal(|ui| {
        let button = widgets::ghost_button(ui, Some(Icon::Plus), "Add task");
        Popup::menu(&button).show(|ui| {
            for kind in Kind::ALL {
                let clicked = ui
                    .horizontal(|ui| {
                        icon(ui, kind, 22.0);
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
