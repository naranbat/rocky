#![forbid(unsafe_code)]

mod app;
mod autocomplete;
mod complete;
mod condition;
mod doc;
mod edit;
mod form;
mod home;
mod input;
mod job;
mod legacy;
mod new_project;
mod node;
mod parts;
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
mod vars;
mod yaml_view;

use std::path::PathBuf;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyEventKind};

use crate::app::App;
use crate::recent::Recent;

/// Opens the builder in the terminal, on the project directory given as the first argument if any.
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
