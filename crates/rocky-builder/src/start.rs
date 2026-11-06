//! The start screen, shown until a project is open: create a new project, open one with the
//! system's file panel, open the example, pick a recent project, or drop a project file.

use std::path::{Path, PathBuf};

use eframe::egui::{
    self, Align, Align2, FontId, Key, KeyboardShortcut, Layout, Modifiers, Rect, RichText, Sense,
    Stroke, Ui, vec2,
};
use jiff::{Timestamp, Zoned};

use crate::home::tilde;
use crate::icons::{self, Icon};
use crate::new_sheet::{NewSheet, Outcome};
use crate::recent::{Entry, Recent, Status};
use crate::scheduler;
use crate::theme;
use crate::widgets;

const NEW: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);
const OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);

/// The project opened by Open example, from this source tree.
const EXAMPLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/httpbin.yaml");

/// What the start screen asked for.
pub enum Pick {
    Open(PathBuf),
    /// A project was created at the path, but not opened.
    Created(PathBuf),
    /// Forget a project from the recent list.
    Forget(PathBuf),
}

/// The start screen's state between frames.
#[derive(Default)]
pub struct Start {
    filter: String,
    sheet: Option<NewSheet>,
    /// Where `rocky` is, looked up once.
    rocky: Option<Option<PathBuf>>,
}

impl Start {
    /// Draws the start screen. `error` is the last failure, shown in the status bar.
    pub fn show(&mut self, ui: &mut Ui, recent: &Recent, error: Option<&str>) -> Option<Pick> {
        let mut pick = None;
        let mut new = false;
        let mut open = false;
        if self.sheet.is_none() {
            ui.input_mut(|input| {
                new = input.consume_shortcut(&NEW);
                open = input.consume_shortcut(&OPEN);
            });
            if let Some(path) = ui.input(|input| {
                input
                    .raw
                    .dropped_files
                    .iter()
                    .map(|file| file.path().to_path_buf())
                    .find(|path| !path.as_os_str().is_empty())
            }) {
                pick = Some(Pick::Open(path));
            }
        }

        egui::Panel::bottom("start-status")
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .stroke(Stroke::new(1.0, theme::BORDER))
                    .inner_margin(egui::Margin::symmetric(14, 6)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    match error {
                        Some(error) => {
                            ui.label(RichText::new(error).color(theme::ERROR).size(12.0));
                        }
                        None => theme::hint(ui, "rocky validate runs when a project opens"),
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        theme::hint(ui, "Recent list is stored on this machine");
                    });
                });
            });

        let rocky = self.rocky.get_or_insert_with(scheduler::find_rocky).clone();
        egui::Panel::left("start-actions")
            .exact_size(340.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(theme::SIDEBAR)
                    .stroke(Stroke::new(1.0, theme::BORDER))
                    .inner_margin(egui::Margin::same(40)),
            )
            .show(ui, |ui| {
                let actions = sidebar(ui, rocky.as_deref());
                new |= actions.new;
                open |= actions.open;
                if actions.example {
                    pick = Some(Pick::Open(PathBuf::from(EXAMPLE)));
                }
            });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::symmetric(48, 40)),
            )
            .show(ui, |ui| {
                egui::Panel::bottom("drop")
                    .frame(egui::Frame::new().inner_margin(egui::Margin {
                        top: 16,
                        ..Default::default()
                    }))
                    .show_separator_line(false)
                    .show(ui, drop_zone);
                if let Some(chosen) = recent_list(ui, &mut self.filter, recent) {
                    pick = Some(chosen);
                }
            });

        if open && let Some(path) = open_dialog() {
            pick = Some(Pick::Open(path));
        }
        if new {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default();
            self.sheet = Some(NewSheet::new(&home));
        }
        if let Some(sheet) = &mut self.sheet {
            match sheet.show(ui.ctx()) {
                Some(Outcome::Cancel) => self.sheet = None,
                Some(Outcome::Created { path, open }) => {
                    self.sheet = None;
                    pick = Some(if open {
                        Pick::Open(path)
                    } else {
                        Pick::Created(path)
                    });
                }
                None => {}
            }
        }
        pick
    }
}

/// What the left column's buttons asked for.
struct Actions {
    new: bool,
    open: bool,
    example: bool,
}

/// Rocky's name and what it does, the three ways to start, and whether `rocky` was found.
fn sidebar(ui: &mut Ui, rocky: Option<&Path>) -> Actions {
    theme::logo(ui, 56.0);
    ui.add_space(14.0);
    ui.label(RichText::new("rocky").strong().size(28.0));
    ui.add_space(4.0);
    ui.label(
        RichText::new(
            "Build YAML jobs visually: commands, conditions, loops and schedules, then run them with rocky.",
        )
        .color(theme::DIM)
        .size(13.0),
    );
    ui.add_space(22.0);
    let actions = Actions {
        new: action(
            ui,
            true,
            Icon::Plus,
            "New project…",
            "Start from a basic or full starter",
            "Cmd N",
        ),
        open: action(
            ui,
            false,
            Icon::Folder,
            "Open project…",
            "Pick a project .yaml file",
            "Cmd O",
        ),
        example: Path::new(EXAMPLE).is_file()
            && action(
                ui,
                false,
                Icon::File,
                "Open example",
                "examples/httpbin.yaml",
                "",
            ),
    };
    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        match rocky {
            Some(path) => {
                theme::hint(ui, &tilde(path));
                ui.horizontal(|ui| {
                    widgets::dot(ui, theme::OK, true);
                    ui.label(RichText::new("rocky CLI found").color(theme::OK).size(12.0));
                });
            }
            None => {
                theme::hint(ui, "Running and scheduling need it on PATH");
                ui.horizontal(|ui| {
                    widgets::dot(ui, theme::ERROR, true);
                    ui.label(
                        RichText::new("rocky CLI not found")
                            .color(theme::ERROR)
                            .size(12.0),
                    );
                });
            }
        }
    });
    actions
}

/// A full-width button with an icon, a title, a line under it and a shortcut; `primary` is
/// filled yellow. Returns whether it was clicked.
fn action(ui: &mut Ui, primary: bool, icon: Icon, title: &str, detail: &str, keys: &str) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 60.0), Sense::click());
    let hovered = response.hovered();
    let (fill, text, dim) = match (primary, hovered) {
        (true, false) => (theme::ACCENT, theme::BG, theme::BG.gamma_multiply(0.75)),
        (true, true) => (
            theme::lerp(theme::ACCENT, egui::Color32::WHITE, 0.12),
            theme::BG,
            theme::BG.gamma_multiply(0.75),
        ),
        (false, false) => (theme::SIDEBAR, theme::TEXT, theme::DIM),
        (false, true) => (theme::CARD, theme::TEXT, theme::DIM),
    };
    let painter = ui.painter();
    painter.rect(
        rect,
        4,
        fill,
        Stroke::new(1.0, if primary { fill } else { theme::BORDER_STRONG }),
        egui::StrokeKind::Inside,
    );
    let left = rect.left() + 16.0;
    let at = Rect::from_center_size(egui::pos2(left + 6.0, rect.center().y), vec2(14.0, 14.0));
    icons::paint(painter, at, icon, text);
    painter.text(
        egui::pos2(left + 26.0, rect.center().y - 9.0),
        Align2::LEFT_CENTER,
        title,
        FontId::monospace(14.0),
        text,
    );
    painter.text(
        egui::pos2(left + 26.0, rect.center().y + 10.0),
        Align2::LEFT_CENTER,
        detail,
        FontId::monospace(12.0),
        dim,
    );
    painter.text(
        egui::pos2(rect.right() - 16.0, rect.center().y),
        Align2::RIGHT_CENTER,
        keys,
        FontId::monospace(11.0),
        dim,
    );
    ui.add_space(2.0);
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// The recent projects matching `filter`, a row each, with the problems of invalid ones.
fn recent_list(ui: &mut Ui, filter: &mut String, recent: &Recent) -> Option<Pick> {
    let mut pick = None;
    ui.horizontal(|ui| {
        ui.label(RichText::new("RECENT PROJECTS").strong().size(13.0));
        theme::hint(ui, &recent.entries.len().to_string());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let field = ui.add(
                egui::TextEdit::singleline(filter)
                    .hint_text("Filter by name or path")
                    .desired_width(280.0)
                    .margin(egui::Margin {
                        left: 30,
                        right: 10,
                        top: 8,
                        bottom: 8,
                    }),
            );
            let at = Rect::from_center_size(
                egui::pos2(field.rect.left() + 16.0, field.rect.center().y),
                vec2(12.0, 12.0),
            );
            icons::paint(ui.painter(), at, Icon::Search, theme::DIM);
        });
    });
    ui.add_space(14.0);
    if recent.entries.is_empty() {
        theme::hint(
            ui,
            "No recent projects yet: create one, open a project file, or drop one below.",
        );
        return None;
    }

    let needle = filter.trim().to_lowercase();
    let shown: Vec<&Entry> = recent
        .entries
        .iter()
        .filter(|entry| {
            needle.is_empty()
                || name(&entry.path).to_lowercase().contains(&needle)
                || tilde(&entry.path).to_lowercase().contains(&needle)
        })
        .collect();
    let columns = Columns::new(ui.available_rect_before_wrap());
    header(ui, &columns);
    let now = Zoned::now();
    egui::ScrollArea::vertical()
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            for entry in &shown {
                if let Some(chosen) = row(ui, &columns, entry, &now) {
                    pick = Some(chosen);
                }
            }
            if shown.is_empty() {
                theme::hint(ui, "No recent project matches the filter.");
            }
            for entry in &shown {
                if let Status::Invalid(error) = &entry.summary.status {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.add_space(columns.name - columns.left);
                        theme::problem(
                            ui,
                            &format!(
                                "{} · {error} — opens with the problem highlighted",
                                name(&entry.path)
                            ),
                        );
                    });
                }
            }
        });
    pick
}

/// Where each column of the recent list starts, for a list `rect` wide.
struct Columns {
    left: f32,
    name: f32,
    jobs: f32,
    status: f32,
    /// Where the opened time ends.
    opened: f32,
    right: f32,
}

impl Columns {
    fn new(rect: Rect) -> Columns {
        let width = rect.width();
        Columns {
            left: rect.left(),
            name: rect.left() + 66.0,
            jobs: rect.left() + width * 0.457,
            status: rect.left() + width * 0.676,
            opened: rect.right() - 60.0,
            right: rect.right(),
        }
    }
}

fn header(ui: &mut Ui, columns: &Columns) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    let y = rect.center().y;
    for (x, align, text) in [
        (columns.name, Align2::LEFT_CENTER, "NAME"),
        (columns.jobs, Align2::LEFT_CENTER, "JOBS"),
        (columns.status, Align2::LEFT_CENTER, "STATUS"),
        (columns.opened, Align2::RIGHT_CENTER, "OPENED"),
    ] {
        ui.painter().text(
            egui::pos2(x, y),
            align,
            text,
            FontId::monospace(11.0),
            theme::DIM,
        );
    }
    ui.add_space(4.0);
}

/// One recent project: click to open it, `···` for more.
fn row(ui: &mut Ui, columns: &Columns, entry: &Entry, now: &Zoned) -> Option<Pick> {
    let missing = matches!(entry.summary.status, Status::Missing);
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), 62.0),
        if missing {
            Sense::hover()
        } else {
            Sense::click()
        },
    );
    let mut pick = response.clicked().then(|| Pick::Open(entry.path.clone()));
    let response = if missing {
        response.on_hover_text("The file is gone; remove it from the list with ⋯")
    } else {
        response
            .on_hover_text(format!("Open {}", entry.path.display()))
            .on_hover_cursor(egui::CursorIcon::PointingHand)
    };
    let painter = ui.painter();
    if response.hovered() {
        painter.rect(
            rect,
            4,
            theme::CARD,
            Stroke::new(1.0, theme::BORDER_STRONG),
            egui::StrokeKind::Inside,
        );
    }
    let fade = |color: egui::Color32| {
        if missing {
            color.gamma_multiply(0.5)
        } else {
            color
        }
    };
    let y = rect.center().y;

    let icon = Rect::from_center_size(egui::pos2(rect.left() + 33.0, y), vec2(38.0, 38.0));
    painter.rect(
        icon,
        4,
        theme::BG,
        Stroke::new(1.0, theme::BORDER_STRONG),
        egui::StrokeKind::Inside,
    );
    icons::paint(
        painter,
        icon.shrink(11.0),
        Icon::File,
        fade(theme::TEXT_DIM),
    );

    let text = |x: f32, y: f32, align: Align2, text: &str, size: f32, color| {
        painter.text(
            egui::pos2(x, y),
            align,
            text,
            FontId::monospace(size),
            fade(color),
        );
    };
    text(
        columns.name,
        y - 9.0,
        Align2::LEFT_CENTER,
        &name(&entry.path),
        14.0,
        theme::TEXT,
    );
    text(
        columns.name,
        y + 10.0,
        Align2::LEFT_CENTER,
        &tilde(&entry.path),
        12.0,
        theme::DIM,
    );

    let summary = &entry.summary;
    let jobs = if missing {
        "—".to_string()
    } else {
        format!(
            "{} · {} scheduled",
            plural(summary.jobs, "job"),
            summary.scheduled
        )
    };
    text(
        columns.jobs,
        y,
        Align2::LEFT_CENTER,
        &jobs,
        12.0,
        theme::TEXT,
    );

    let (dot, status, color) = match &summary.status {
        Status::Running => ("●", "running · socket", theme::WHILE),
        Status::Valid => ("●", "valid", theme::OK),
        Status::Invalid(_) => ("●", "invalid", theme::ERROR),
        Status::Missing => ("○", "file not found", theme::DIM),
    };
    text(columns.status, y, Align2::LEFT_CENTER, dot, 9.0, color);
    text(
        columns.status + 14.0,
        y,
        Align2::LEFT_CENTER,
        status,
        12.0,
        color,
    );
    text(
        columns.opened,
        y,
        Align2::RIGHT_CENTER,
        &ago(entry.opened, now),
        12.0,
        theme::DIM,
    );

    let more = Rect::from_center_size(egui::pos2(columns.right - 30.0, y), vec2(22.0, 22.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(more), |ui| {
        widgets::more_menu(ui, "More", |ui| {
            if ui.button("Remove from recent").clicked() {
                pick = Some(Pick::Forget(entry.path.clone()));
            }
            if ui.button("Copy path").clicked() {
                ui.ctx().copy_text(entry.path.display().to_string());
            }
        });
    });
    pick
}

/// A dashed box to drop a project file on; it lights up while a file is dragged over.
fn drop_zone(ui: &mut Ui) {
    let hovering = ui.input(|input| !input.raw.hovered_files.is_empty());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 120.0), Sense::hover());
    let color = if hovering {
        theme::ACCENT
    } else {
        theme::BORDER
    };
    if hovering {
        ui.painter().rect_filled(rect, 4, theme::ACCENT_BG);
    }
    let corners = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
        rect.left_top(),
    ];
    ui.painter().extend(egui::Shape::dashed_line(
        &corners,
        Stroke::new(1.0, color),
        4.0,
        4.0,
    ));
    let color = if hovering { theme::ACCENT } else { theme::DIM };
    let text = ui.painter().text(
        rect.center() + vec2(11.0, 0.0),
        Align2::CENTER_CENTER,
        "Drop a project .yaml file here to open it",
        FontId::monospace(13.0),
        color,
    );
    let at = Rect::from_center_size(
        egui::pos2(text.left() - 12.0, rect.center().y),
        vec2(14.0, 14.0),
    );
    icons::paint(ui.painter(), at, Icon::Folder, color);
}

/// The system's Open panel, for YAML files. `None` if cancelled.
fn open_dialog() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Open a rocky project")
        .add_filter("YAML", &["yaml", "yml"])
        .pick_file()
}

/// A project's name: its folder for `project.yaml`, else the file's name without `.yaml`.
fn name(path: &Path) -> String {
    let part = if path.file_name().is_some_and(|file| file == "project.yaml") {
        path.parent().and_then(Path::file_name)
    } else {
        path.file_stem()
    };
    part.map(|part| part.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn plural(count: usize, noun: &str) -> String {
    match count {
        1 => format!("1 {noun}"),
        _ => format!("{count} {noun}s"),
    }
}

/// When a project was opened: `just now`, `2 min ago`, `5 h ago`, `yesterday`, `3 days ago`,
/// then the date.
fn ago(opened: Timestamp, now: &Zoned) -> String {
    let then = opened.to_zoned(now.time_zone().clone());
    let secs = now.timestamp().as_second() - opened.as_second();
    let days = (now.date() - then.date()).get_days();
    match (secs, days) {
        (..60, _) => "just now".to_string(),
        (..3600, _) => format!("{} min ago", secs / 60),
        (_, 0) => format!("{} h ago", secs / 3600),
        (_, 1) => "yesterday".to_string(),
        (_, ..7) => format!("{days} days ago"),
        _ if then.year() == now.year() => then.strftime("%b %-d").to_string(),
        _ => then.strftime("%b %-d, %Y").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Draws valid, invalid and missing projects, the filter, and the New project sheet.
    #[test]
    fn draws_the_list_and_the_sheet() {
        let dir = tempfile::tempdir().unwrap();
        let valid = dir.path().join("valid.yaml");
        let invalid = dir.path().join("invalid.yaml");
        std::fs::write(&valid, "jobs:\n  a: {tasks: {t: {cmd: [echo]}}}\n").unwrap();
        std::fs::write(&invalid, "jobs:\n  a: {tasks: {t: {cmd: []}}}\n").unwrap();
        let mut recent = Recent::none();
        for path in [&valid, &invalid, &dir.path().join("gone.yaml")] {
            recent.record(path).unwrap();
        }
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut start = Start::default();
        for (filter, sheet) in [("", false), ("inval", false), ("", true)] {
            start.filter = filter.to_string();
            if sheet {
                start.sheet = Some(NewSheet::new(dir.path()));
            }
            for _ in 0..2 {
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    assert!(start.show(ui, &recent, None).is_none());
                });
                output.textures_delta.clear();
            }
        }
        assert!(start.sheet.is_some());
    }

    #[test]
    fn names_and_times() {
        assert_eq!(name(Path::new("/w/automation/project.yaml")), "automation");
        assert_eq!(name(Path::new("/src/examples/httpbin.yaml")), "httpbin");
        let now: Zoned = "2026-11-06T15:00:00[UTC]".parse().unwrap();
        let at = |text: &str| ago(text.parse::<Timestamp>().unwrap(), &now);
        assert_eq!(at("2026-11-06T14:59:30Z"), "just now");
        assert_eq!(at("2026-11-06T14:58:00Z"), "2 min ago");
        assert_eq!(at("2026-11-06T10:00:00Z"), "5 h ago");
        assert_eq!(at("2026-11-05T10:00:00Z"), "yesterday");
        assert_eq!(at("2026-11-03T10:00:00Z"), "3 days ago");
        assert_eq!(at("2026-10-28T10:00:00Z"), "Oct 28");
        assert_eq!(at("2025-10-28T10:00:00Z"), "Oct 28, 2025");
    }
}
