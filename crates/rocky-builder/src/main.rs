#![forbid(unsafe_code)]

mod app;
mod chrome;
mod code;
mod debug_view;
mod editor;
mod external;
mod file_view;
mod files;
mod highlight;
mod home;
mod home_view;
mod job_yaml;
mod jobs_view;
mod line_input;
mod new_pane;
mod open_pane;
mod outline;
mod problem;
mod recent;
mod recent_pane;
mod results;
mod run;
mod schedule;
mod summary;
mod textedit;
mod theme;

use std::path::PathBuf;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyEventKind};

use crate::app::App;
use crate::recent::Recent;

/// Opens the builder in the terminal, on the project given as the first argument if any: a
/// directory with a `rocky.yaml`, or a project file.
fn main() -> anyhow::Result<()> {
    let path = std::env::args_os().nth(1).map(PathBuf::from);
    let mut app = App::new(path, Recent::load());
    // Restores the terminal on a panic too.
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app);
    ratatui::restore();
    result
}

fn run(terminal: &mut DefaultTerminal, app: &mut App) -> anyhow::Result<()> {
    while !app.quit {
        if let Some((name, text)) = app.take_external() {
            // The editor gets the terminal until it exits.
            ratatui::restore();
            let edited = external::edit(&external::editor(), &name, &text);
            *terminal = ratatui::init();
            app.finish_external(edited);
        }
        let wait = app.tick();
        terminal.draw(|frame| app.draw(frame))?;
        if event::poll(wait)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            app.handle(key);
        }
    }
    Ok(())
}
