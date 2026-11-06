//! What every editor widget needs besides the YAML node it edits.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct Edit<'a> {
    /// Set by any widget that changed the document this frame.
    pub changed: bool,
    /// The project's job names, for trigger pickers.
    pub jobs: &'a [String],
    /// The project file's directory, for task file previews.
    pub dir: &'a Path,
    /// Task file contents by path, read once per validation.
    pub previews: &'a mut HashMap<PathBuf, String>,
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
