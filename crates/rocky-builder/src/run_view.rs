//! How a run shows in the editor: a result strip under each task card (exit badge, timing,
//! output tabs) and a status line under the job header.

use std::time::{Duration, Instant};

use eframe::egui::{self, RichText};
use serde_json::Value;

use crate::icons::Icon;
use crate::results::{End, Results, TaskRun};
use crate::theme;
use crate::widgets;

/// One run as the editor shows it: a run started in the builder, or the latest run of a
/// job that the scheduler reported.
#[derive(Clone, Copy)]
pub struct Shown<'a> {
    pub results: &'a Results,
    /// The job the run started with, and the task it stops after.
    pub job: &'a str,
    pub until: Option<&'a str>,
    pub started: Instant,
    pub elapsed: Duration,
    /// Who ran it, when it wasn't the builder: `scheduler`.
    pub by: Option<&'static str>,
}

impl Shown<'_> {
    pub fn is_running(&self) -> bool {
        self.results.end.is_none()
    }

    /// Whether this run reported anything about `job`.
    pub fn covers(&self, job: &str) -> bool {
        self.job == job || self.results.jobs.iter().any(|name| name == job)
    }
}

/// Longest preview of a result shown inline.
const PREVIEW_CHARS: usize = 60;

/// The run's state for `job`, under the job header.
pub fn job_status(ui: &mut egui::Ui, run: Shown, job: &str) {
    let results = run.results;
    if !run.covers(job) {
        return;
    }
    let (runs, failures) = results.job_counts(job);
    let secs = run.elapsed.as_secs_f32();
    let mut upto = run
        .until
        .filter(|_| run.job == job)
        .map(|task| format!(" · up to {task}"))
        .unwrap_or_default();
    if let Some(by) = run.by {
        upto.push_str(&format!(" · by the {by}"));
    }
    ui.horizontal(|ui| {
        if results.job_skipped(job) {
            theme::badge(ui, Some(Icon::Skip), "SKIPPED", theme::DIM);
            theme::hint(ui, "the job's when was false · no task ran");
            return;
        }
        match &results.end {
            None => {
                ui.spinner();
                theme::hint(
                    ui,
                    &format!("RUNNING{upto} · {runs} task runs · {secs:.1}s"),
                );
            }
            Some(End::Done { failed: None }) => {
                theme::badge(ui, Some(Icon::Check), "LAST RUN OK", theme::OK);
                theme::hint(ui, &format!("{runs} task runs in {secs:.2}s{upto}"));
            }
            Some(End::Done {
                failed: Some(reason),
            }) => {
                theme::badge(ui, Some(Icon::Close), "LAST RUN FAILED", theme::ERROR);
                ui.label(RichText::new(reason).color(theme::ERROR).size(12.0));
            }
            Some(End::Error(error)) => {
                theme::badge(ui, Some(Icon::Close), "RUN ERROR", theme::ERROR);
                ui.label(RichText::new(error).color(theme::ERROR).size(12.0));
            }
        }
        if failures > 0 && results.end.is_some() {
            theme::hint(ui, &format!("· {failures} failed runs"));
        }
    });
}

/// The strip under task `name`'s card: what its last run did, and its output on demand.
pub fn task_strip(ui: &mut egui::Ui, run: Shown, job: &str, name: &str) {
    let Some(task) = run.results.task(job, name) else {
        if run.is_running() && run.job == job {
            ui.separator();
            theme::hint(ui, "waiting…");
        }
        return;
    };
    ui.separator();
    ui.horizontal_wrapped(|ui| summary(ui, task, name));
    if let Some(result) = task.last.as_ref().map(|event| &event["result"])
        && !result.is_null()
    {
        output(ui, result);
    }
}

/// The badge and one-line summary of a task's runs.
fn summary(ui: &mut egui::Ui, task: &TaskRun, name: &str) {
    if task.skipped {
        theme::badge(ui, Some(Icon::Skip), "skipped", theme::DIM);
        theme::hint(ui, &format!("when was false · tasks.{name} stays unset"));
        return;
    }
    if let Some(target) = &task.queued {
        theme::badge(
            ui,
            Some(Icon::Check),
            &format!("queued {target}"),
            theme::OK,
        );
        theme::hint(ui, &format!("{target} runs after this job"));
        return;
    }
    if let Some(items) = task.items {
        theme::badge(
            ui,
            Some(Icon::Loop),
            &format!("{items} items"),
            theme::FOR_EACH,
        );
        theme::hint(ui, "each item ran do; see its tasks below");
        return;
    }
    if let Some((checks, holds)) = task.check {
        theme::badge(
            ui,
            Some(Icon::While),
            &format!("{checks} checks"),
            theme::WHILE,
        );
        let state = if holds {
            "still true"
        } else {
            "condition false, loop done"
        };
        theme::hint(ui, state);
        return;
    }
    let Some(last) = &task.last else {
        return;
    };
    let ok = last["ok"] == Value::Bool(true);
    let label = match last.get("exit_code") {
        Some(code) => format!("exit {code}"),
        None => "error".to_string(),
    };
    let (icon, color) = if ok {
        (Icon::Check, theme::OK)
    } else {
        (Icon::Close, theme::ERROR)
    };
    theme::badge(ui, Some(icon), &label, color);
    let ms = last["ms"].as_u64().unwrap_or_default();
    let mut text = format!(
        "{} · {:.2}s",
        if ok { "ok" } else { "failed" },
        ms as f64 / 1000.0
    );
    if last["allowed"] == Value::Bool(true) {
        text.push_str(" · allowed to fail");
    }
    if task.runs > 1 {
        text.push_str(&format!(
            " · {} of {} runs ok",
            task.runs - task.failures,
            task.runs
        ));
    }
    if let Some(error) = last["error"].as_str() {
        text.push_str(&format!(" · {error}"));
    } else if let Some(stdout) = last["result"].get("stdout") {
        text.push_str(&format!(" · tasks.{name}.stdout = {}", preview(stdout)));
    }
    theme::hint(ui, &text);
}

/// A value on one short line.
fn preview(value: &Value) -> String {
    let text = match value {
        Value::String(text) => text.lines().next().unwrap_or_default().to_string(),
        Value::Null => "(none)".to_string(),
        other => other.to_string(),
    };
    if text.chars().count() > PREVIEW_CHARS {
        format!("{}…", text.chars().take(PREVIEW_CHARS).collect::<String>())
    } else {
        text
    }
}

/// A collapsed `View stdout` toggle, opening stdout / stderr / result tabs with Copy.
fn output(ui: &mut egui::Ui, result: &Value) {
    let id = ui.make_persistent_id("output");
    let (mut open, mut tab) =
        ui.data_mut(|data| data.get_temp::<(bool, u8)>(id).unwrap_or((false, 0)));
    let (toggle, chevron) = if open {
        ("Hide output", Icon::ChevronUp)
    } else {
        ("View output", Icon::Chevron)
    };
    let button = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        widgets::icon(ui, Icon::Terminal, 12.0, theme::DIM);
        ui.label(RichText::new(toggle).color(theme::TEXT_DIM).size(12.0));
        widgets::icon(ui, chevron, 10.0, theme::DIM);
    });
    if button
        .response
        .interact(egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        open = !open;
    }
    if open {
        let stderr = result["stderr"].as_str().unwrap_or_default();
        let tabs = [
            "stdout".to_string(),
            match stderr.lines().count() {
                0 => "stderr · empty".to_string(),
                1 => "stderr · 1 line".to_string(),
                n => format!("stderr · {n} lines"),
            },
            "result JSON".to_string(),
        ];
        let text = match tab {
            0 => match &result["stdout"] {
                Value::String(text) => text.clone(),
                other => serde_json::to_string_pretty(other).unwrap_or_default(),
            },
            1 => stderr.to_string(),
            _ => serde_json::to_string_pretty(result).unwrap_or_default(),
        };
        theme::inset().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                for (index, label) in tabs.iter().enumerate() {
                    if ui
                        .selectable_label(tab as usize == index, label.as_str())
                        .clicked()
                    {
                        tab = index as u8;
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::icon_button(ui, Icon::Copy, theme::DIM, "Copy").clicked() {
                        ui.ctx().copy_text(text.clone());
                    }
                    if tab == 0 && !result["stdout"].is_string() && !result["stdout"].is_null() {
                        theme::hint(ui, "parsed as JSON");
                    }
                });
            });
            ui.separator();
            egui::ScrollArea::vertical()
                .max_height(240.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(text).color(theme::NUMBER).size(12.0))
                            .selectable(true),
                    );
                });
        });
    }
    ui.data_mut(|data| data.insert_temp(id, (open, tab)));
}
