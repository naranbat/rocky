//! An open project as the text of its files: the project file (`rocky.yaml`, or the file
//! given), each job file it lists, and its env file. Each is edited as text, checked with
//! core as it is, and saved only when it changed.

use std::path::{Path, PathBuf};

use rocky_core::error::Error;
use rocky_core::project::{self, JOBS_DIR, Project, SETTINGS_FILE};
use serde_yaml::Value;

use crate::outline;
use crate::textedit::TextEdit;

/// A new job's file.
const NEW_JOB: &str = "tasks:\n  hello:\n    cmd: [echo, hello]\n";

pub struct Buffer {
    pub path: PathBuf,
    /// The text on disk, as the editor writes it back.
    saved: String,
    pub edit: TextEdit,
}

impl Buffer {
    /// The file at `path`; empty when there is none yet.
    fn read(path: PathBuf) -> Buffer {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        Buffer::new(path, &text, &text)
    }

    fn new(path: PathBuf, saved: &str, text: &str) -> Buffer {
        Buffer {
            path,
            saved: TextEdit::new(saved).text(),
            edit: TextEdit::new(text),
        }
    }

    pub fn text(&self) -> String {
        self.edit.text()
    }

    pub fn is_dirty(&self) -> bool {
        self.edit.text() != self.saved
    }

    fn save(&mut self) -> Result<(), String> {
        let text = self.edit.text();
        let failed =
            |error: std::io::Error| format!("cannot save {}: {error}", self.path.display());
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(failed)?;
        }
        std::fs::write(&self.path, &text).map_err(failed)?;
        self.saved = text;
        Ok(())
    }

    /// The text parsed as YAML.
    pub fn yaml(&self) -> Result<Value, String> {
        serde_yaml::from_str(&self.text()).map_err(|error| error.to_string())
    }
}

pub struct Job {
    pub name: String,
    pub buffer: Buffer,
}

pub struct Files {
    /// The project's directory: the project file's.
    pub root: PathBuf,
    pub settings: Buffer,
    /// In the order the project file lists them.
    pub jobs: Vec<Job>,
    /// The `env_file` the project file names, if any.
    pub env: Option<Buffer>,
}

impl Files {
    /// Opens the project in directory `path`, or with project file `path`.
    pub fn open(path: &Path) -> Result<Files, String> {
        let file = if path.is_dir() {
            path.join(SETTINGS_FILE)
        } else {
            path.to_path_buf()
        };
        if !file.is_file() {
            return Err(format!("no project file at {}", file.display()));
        }
        let file = std::path::absolute(&file).map_err(|error| error.to_string())?;
        let root = file.parent().unwrap_or(Path::new("/")).to_path_buf();
        let mut files = Files {
            root,
            settings: Buffer::read(file),
            jobs: Vec::new(),
            env: None,
        };
        files.sync();
        Ok(files)
    }

    /// The project's name: its directory's, or its project file's when that has another name.
    pub fn name(&self) -> String {
        let file = &self.settings.path;
        let stem = if file.file_name().is_some_and(|name| name == SETTINGS_FILE) {
            self.root.file_name()
        } else {
            file.file_stem()
        };
        stem.unwrap_or_default().to_string_lossy().into_owned()
    }

    /// Opens the job and env files the project file lists now, keeping those already open.
    pub fn sync(&mut self) {
        let settings = self.settings.yaml().unwrap_or_default();
        let listed: Vec<(String, PathBuf)> = settings
            .get("jobs")
            .and_then(Value::as_mapping)
            .into_iter()
            .flatten()
            .filter_map(|(name, path)| {
                Some((name.as_str()?.to_string(), self.root.join(path.as_str()?)))
            })
            .collect();
        let mut old = std::mem::take(&mut self.jobs);
        for (name, path) in listed {
            let buffer = match old.iter().position(|job| job.name == name) {
                Some(index) => old.remove(index).buffer,
                None => Buffer::read(path),
            };
            self.jobs.push(Job { name, buffer });
        }
        let env = settings
            .get("env_file")
            .and_then(Value::as_str)
            .map(|path| self.root.join(path));
        if self.env.as_ref().map(|env| &env.path) != env.as_ref() {
            self.env = env.map(Buffer::read);
        }
    }

    /// The project as core loads it from the current text.
    pub fn load(&self) -> Result<Project, Error> {
        project::load_parts(&self.root, Some(&self.settings.text()), &self.job_texts())
    }

    /// Each job's name and current text.
    pub fn job_texts(&self) -> Vec<(String, String)> {
        self.jobs
            .iter()
            .map(|job| (job.name.clone(), job.buffer.text()))
            .collect()
    }

    fn buffers(&self) -> impl Iterator<Item = &Buffer> {
        std::iter::once(&self.settings)
            .chain(self.jobs.iter().map(|job| &job.buffer))
            .chain(self.env.as_ref())
    }

    /// How many files have unsaved changes.
    pub fn unsaved(&self) -> usize {
        self.buffers().filter(|buffer| buffer.is_dirty()).count()
    }

    /// Writes every changed file.
    pub fn save(&mut self) -> Result<(), String> {
        let buffers = std::iter::once(&mut self.settings)
            .chain(self.jobs.iter_mut().map(|job| &mut job.buffer))
            .chain(self.env.as_mut());
        for buffer in buffers {
            if buffer.is_dirty() {
                buffer.save()?;
            }
        }
        Ok(())
    }

    /// Adds job `name` in `jobs/NAME.yaml`, listed in the project file. Returns its index.
    pub fn add_job(&mut self, name: &str) -> Result<usize, String> {
        project::check_job_name(name).map_err(|error| error.to_string())?;
        if self.jobs.iter().any(|job| job.name == name) {
            return Err(format!("there is already a job `{name}`"));
        }
        let file = format!("{JOBS_DIR}/{name}.yaml");
        let path = self.root.join(&file);
        if path.exists() {
            return Err(format!("{file} already exists"));
        }
        let mut lines = self.settings.edit.lines().to_vec();
        if !outline::add_to_block(&mut lines, "jobs", format!("  {name}: {file}")) {
            return Err(format!(
                "`jobs` is written inline: add `{name}: {file}` to it in 2 Project"
            ));
        }
        self.settings.edit.set_lines(lines);
        self.jobs.push(Job {
            name: name.to_string(),
            buffer: Buffer::new(path, "", NEW_JOB),
        });
        Ok(self.jobs.len() - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("jobs")).unwrap();
        std::fs::write(
            dir.path().join("jobs/a.yaml"),
            "tasks:\n  t:\n    cmd: [echo]\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("rocky.yaml"),
            "env_file: x.env\njobs:\n  a: jobs/a.yaml\n",
        )
        .unwrap();
        dir
    }

    #[test]
    fn opens_edits_and_saves_only_changed_files() {
        let dir = project();
        let mut files = Files::open(dir.path()).unwrap();
        assert_eq!(files.jobs.len(), 1);
        assert!(files.env.as_ref().is_some_and(|env| env.text().is_empty()));
        assert_eq!(files.unsaved(), 0);
        assert!(files.load().is_ok());

        files.jobs[0]
            .buffer
            .edit
            .set_text("tasks:\n  t:\n    cmd: []\n");
        assert_eq!(files.unsaved(), 1);
        assert!(files.load().is_err());
        files.save().unwrap();
        assert_eq!(files.unsaved(), 0);
        assert!(!dir.path().join("x.env").exists());
        let saved = std::fs::read_to_string(dir.path().join("jobs/a.yaml")).unwrap();
        assert!(saved.contains("cmd: []"));
    }

    #[test]
    fn adds_a_job_that_loads_once_saved() {
        let dir = project();
        let mut files = Files::open(dir.path()).unwrap();
        assert_eq!(files.add_job("b"), Ok(1));
        assert!(files.add_job("b").is_err());
        assert!(files.add_job("../x").is_err());
        assert_eq!(files.unsaved(), 2);
        files.save().unwrap();
        let project = project::load(&dir.path().join("rocky.yaml")).unwrap();
        assert_eq!(project.jobs.keys().collect::<Vec<_>>(), ["a", "b"]);
    }

    #[test]
    fn follows_edits_to_the_job_list() {
        let dir = project();
        let mut files = Files::open(dir.path()).unwrap();
        assert_eq!(
            files.name(),
            dir.path().file_name().unwrap().to_string_lossy()
        );
        files
            .settings
            .edit
            .set_text("jobs:\n  a: jobs/a.yaml\n  c: jobs/c.yaml\n");
        files.sync();
        let names: Vec<_> = files.jobs.iter().map(|job| job.name.as_str()).collect();
        assert_eq!(names, ["a", "c"]);
        assert!(files.env.is_none());
        assert!(Files::open(&dir.path().join("missing")).is_err());
    }
}
