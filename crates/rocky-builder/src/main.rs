#![forbid(unsafe_code)]

mod app;
mod autocomplete;
mod chips;
mod complete;
mod condition;
mod doc;
mod edit;
mod home;
mod job;
mod legacy;
mod new_sheet;
mod project;
mod recent;
mod results;
mod run;
mod run_view;
mod schedule;
mod scheduler;
mod scope;
mod sidebar;
mod start;
mod task;
mod task_cmd;
mod task_file;
mod task_loop;
mod task_trigger;
mod theme;
mod validate;
mod yaml_view;

use std::path::PathBuf;

use eframe::egui;

/// Opens the builder, on the project file given as the first argument if any.
fn main() -> eframe::Result {
    let path = std::env::args_os().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Rocky Builder")
            .with_inner_size([1280.0, 860.0])
            .with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Rocky Builder",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc, path)))),
    )
}
