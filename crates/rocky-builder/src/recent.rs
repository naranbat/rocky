//! The recently opened projects, newest first, kept in `~/.config/rocky/recent` on this
//! machine, with what each one looks like now: its jobs, and whether it is valid, running a
//! scheduler, or gone.

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use jiff::Timestamp;
use serde_yaml::Value;

/// How many projects the list keeps.
const KEEP: usize = 20;

/// One recently opened project.
pub struct Entry {
    pub path: PathBuf,
    pub opened: Timestamp,
    pub summary: Summary,
}

/// A project's jobs and state, read when the list is loaded or changed.
pub struct Summary {
    pub jobs: usize,
    pub scheduled: usize,
    pub status: Status,
}

pub enum Status {
    /// A scheduler (`rocky run --socket .rocky/rocky.sock`) answers in its folder.
    Running,
    Valid,
    /// Core's error chain.
    Invalid(String),
    Missing,
}

pub struct Recent {
    /// The list file; `None` without a home folder, when nothing is remembered.
    file: Option<PathBuf>,
    pub entries: Vec<Entry>,
}

impl Recent {
    /// The list stored on this machine; empty if there is none yet or it can't be read.
    pub fn load() -> Recent {
        let file = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".config")))
            .map(|config| config.join("rocky/recent"));
        Recent::at(file)
    }

    /// An empty list that is never saved.
    #[cfg(test)]
    pub fn none() -> Recent {
        Recent::at(None)
    }

    fn at(file: Option<PathBuf>) -> Recent {
        let text = file
            .as_deref()
            .and_then(|file| std::fs::read_to_string(file).ok())
            .unwrap_or_default();
        let entries = text
            .lines()
            .filter_map(|line| {
                let (opened, path) = line.split_once('\t')?;
                let path = PathBuf::from(path);
                Some(Entry {
                    opened: opened.parse().ok()?,
                    summary: summarize(&path),
                    path,
                })
            })
            .collect();
        Recent { file, entries }
    }

    /// Puts `path` first, opened now.
    pub fn record(&mut self, path: &Path) -> Result<(), String> {
        let path = std::path::absolute(path).map_err(|error| error.to_string())?;
        self.entries.retain(|entry| entry.path != path);
        self.entries.insert(
            0,
            Entry {
                opened: Timestamp::now(),
                summary: summarize(&path),
                path,
            },
        );
        self.entries.truncate(KEEP);
        self.save()
    }

    pub fn remove(&mut self, path: &Path) -> Result<(), String> {
        self.entries.retain(|entry| entry.path != path);
        self.save()
    }

    /// Reads every project's summary again.
    pub fn refresh(&mut self) {
        for entry in &mut self.entries {
            entry.summary = summarize(&entry.path);
        }
    }

    fn save(&self) -> Result<(), String> {
        let Some(file) = &self.file else {
            return Ok(());
        };
        let text: String = self
            .entries
            .iter()
            .map(|entry| format!("{}\t{}\n", entry.opened, entry.path.display()))
            .collect();
        let failed = |error: std::io::Error| format!("cannot save {}: {error}", file.display());
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(failed)?;
        }
        std::fs::write(file, text).map_err(failed)
    }
}

/// What the project in directory `path` holds now.
pub fn summarize(path: &Path) -> Summary {
    if !path.is_dir() {
        return Summary {
            jobs: 0,
            scheduled: 0,
            status: Status::Missing,
        };
    }
    let (jobs, scheduled, status) = match rocky_core::project::load(path) {
        Ok(project) => {
            let scheduled = project
                .jobs
                .values()
                .filter(|job| job.schedule.is_some())
                .count();
            (project.jobs.len(), scheduled, Status::Valid)
        }
        Err(error) => {
            let (jobs, scheduled) = count_jobs(path);
            (jobs, scheduled, Status::Invalid(error.chain()))
        }
    };
    let socket = path.join(".rocky/rocky.sock");
    let status = if UnixStream::connect(socket).is_ok() {
        Status::Running
    } else {
        status
    };
    Summary {
        jobs,
        scheduled,
        status,
    }
}

/// The jobs of a project that doesn't load, and those with a `schedule`.
fn count_jobs(dir: &Path) -> (usize, usize) {
    let root = crate::parts::read(dir).unwrap_or_default();
    let Some(jobs) = root.get("jobs").and_then(Value::as_mapping) else {
        return (0, 0);
    };
    let scheduled = jobs
        .values()
        .filter(|job| job.get("schedule").is_some())
        .count();
    (jobs.len(), scheduled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_newest_first_and_survives_a_reload() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config/recent");
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::create_dir_all(a.join("jobs")).unwrap();
        std::fs::write(
            a.join("jobs/x.yaml"),
            "schedule: {every: 1m}\ntasks: {t: {cmd: [echo]}}\n",
        )
        .unwrap();
        std::fs::write(a.join("jobs/y.yaml"), "tasks: {t: {cmd: [echo]}}\n").unwrap();

        let mut recent = Recent::at(Some(file.clone()));
        recent.record(&a).unwrap();
        recent.record(&b).unwrap();
        recent.record(&a).unwrap();

        let recent = Recent::at(Some(file));
        let paths: Vec<_> = recent.entries.iter().map(|e| e.path.clone()).collect();
        assert_eq!(paths, [a, b]);
        let summary = &recent.entries[0].summary;
        assert_eq!((summary.jobs, summary.scheduled), (2, 1));
        assert!(matches!(summary.status, Status::Valid));
        assert!(matches!(recent.entries[1].summary.status, Status::Missing));
    }

    #[test]
    fn an_invalid_project_still_counts_its_jobs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("jobs")).unwrap();
        std::fs::write(
            dir.path().join("jobs/x.yaml"),
            "schedule: {every: 1m}\ntasks: {t: {cmd: []}}\n",
        )
        .unwrap();
        let summary = summarize(dir.path());
        assert_eq!((summary.jobs, summary.scheduled), (1, 1));
        assert!(matches!(summary.status, Status::Invalid(error) if error.contains("cmd")));
    }
}
