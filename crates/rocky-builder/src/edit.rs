//! What every view needs besides the document while it builds its rows.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::results::Shown;
use crate::validate::Problem;

pub struct Edit<'a> {
    /// The project's job names, for trigger pickers.
    pub jobs: &'a [String],
    /// The job files' directory, which task file paths are relative to, for previews.
    pub dir: &'a Path,
    /// Task file contents by path, read once per validation.
    pub previews: &'a mut HashMap<PathBuf, String>,
    /// The job being edited, and the latest run, whose results the rows show.
    pub job: &'a str,
    pub run: Option<Shown<'a>>,
    /// Why the document is invalid, shown on the job or task it is about.
    pub problem: Option<&'a Problem>,
}

impl Edit<'_> {
    /// The text of `file` (relative to the job file), or why it can't be read.
    pub fn preview(&mut self, file: &str) -> &str {
        let path = self.dir.join(file);
        self.previews.entry(path.clone()).or_insert_with(|| {
            std::fs::read_to_string(&path).unwrap_or_else(|error| format!("cannot read: {error}"))
        })
    }
}
