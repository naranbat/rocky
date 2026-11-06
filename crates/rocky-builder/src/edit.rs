//! What every editor widget needs besides the YAML node it edits.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::complete::Var;
use crate::run_view::Shown;
use crate::validate::Problem;

pub struct Edit<'a> {
    /// Set by any widget that changed the document this frame.
    pub changed: bool,
    /// The project's job names, for trigger pickers.
    pub jobs: &'a [String],
    /// The project file's directory, for task file previews.
    pub dir: &'a Path,
    /// Task file contents by path, read once per validation.
    pub previews: &'a mut HashMap<PathBuf, String>,
    /// The job being edited, and the latest run, whose results the cards show.
    pub job: &'a str,
    pub run: Option<Shown<'a>>,
    /// Whether a run can start now (the document is valid and nothing is running).
    pub can_run: bool,
    /// Set by a card's ▶: run the job up to this task.
    pub run_until: Option<String>,
    /// Why the document is invalid, shown on the card or job it is about.
    pub problem: Option<&'a Problem>,
    /// The variables a `${…}` can read where the editor is drawing now; task lists add each
    /// task's after drawing it, so a card only offers earlier tasks.
    pub scope: Vec<Var>,
}

impl Edit<'_> {
    /// The text of `file` (relative to the project), or why it can't be read.
    pub fn preview(&mut self, file: &str) -> &str {
        let path = self.dir.join(file);
        self.previews.entry(path.clone()).or_insert_with(|| {
            std::fs::read_to_string(&path).unwrap_or_else(|error| format!("cannot read: {error}"))
        })
    }
}
