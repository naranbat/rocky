//! The recently opened projects, newest first, kept in `~/.config/rocky/recent` on this
//! machine, with what each one looks like now: its jobs, and whether it is valid, running a
//! scheduler, or gone.

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use jiff::Timestamp;
use rocky_core::project;
use serde_json::Value as Json;

use crate::files::Files;

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
    pub names: Vec<String>,
    pub scheduled: usize,
    pub status: Status,
    /// The last task `rocky run` logged, in words.
    pub last_run: Option<String>,
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

/// What the project in directory `path`, or with project file `path`, holds now.
pub fn summarize(path: &Path) -> Summary {
    if !path.exists() {
        return Summary {
            names: Vec::new(),
            scheduled: 0,
            status: Status::Missing,
            last_run: None,
        };
    }
    let file = if path.is_dir() {
        path.join(project::SETTINGS_FILE)
    } else {
        path.to_path_buf()
    };
    let (names, scheduled, status) = match project::load(&file) {
        Ok(project) => {
            let scheduled = project
                .jobs
                .values()
                .filter(|job| job.schedule.is_some())
                .count();
            (project.jobs.into_keys().collect(), scheduled, Status::Valid)
        }
        Err(error) => {
            let (names, scheduled) = count_jobs(path);
            (names, scheduled, Status::Invalid(error.chain()))
        }
    };
    let dir = file.parent().unwrap_or(Path::new("."));
    let status = if UnixStream::connect(dir.join(".rocky/rocky.sock")).is_ok() {
        Status::Running
    } else {
        status
    };
    Summary {
        names,
        scheduled,
        status,
        last_run: last_run(dir),
    }
}

/// The jobs of a project that doesn't load, and how many have a `schedule`.
fn count_jobs(path: &Path) -> (Vec<String>, usize) {
    let Ok(files) = Files::open(path) else {
        return (Vec::new(), 0);
    };
    let scheduled = files
        .jobs
        .iter()
        .filter(|job| {
            job.buffer
                .yaml()
                .is_ok_and(|job| job.get("schedule").is_some())
        })
        .count();
    let names = files.jobs.into_iter().map(|job| job.name).collect();
    (names, scheduled)
}

/// The last task `rocky run` logged in `dir`, in words:
/// `status · ✗ send exited with code 22 · 09:30`.
fn last_run(dir: &Path) -> Option<String> {
    let logs = std::fs::read_dir(dir.join(".rocky/logs")).ok()?;
    let newest = logs
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "log"))
        .max()?;
    let text = std::fs::read_to_string(newest).ok()?;
    text.lines().rev().find_map(|line| {
        let event: Json = serde_json::from_str(line).ok()?;
        if event["event"] != "task" {
            return None;
        }
        let task = event["task"].as_str()?;
        let outcome = match (&event["ok"], event["error"].as_str()) {
            (Json::Bool(true), _) => format!("✓ {task}"),
            (_, Some(error)) => format!("✗ {task} {error}"),
            _ => format!("✗ {task} exited with code {}", event["exit_code"]),
        };
        let job = event["job"].as_str().unwrap_or_default();
        let time = event["time"].as_str().and_then(|time| time.get(11..16));
        Some(format!("{job} · {outcome} · {}", time.unwrap_or_default()))
    })
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
        std::fs::write(
            a.join("rocky.yaml"),
            "jobs: {x: jobs/x.yaml, y: jobs/y.yaml}\n",
        )
        .unwrap();

        let mut recent = Recent::at(Some(file.clone()));
        recent.record(&a).unwrap();
        recent.record(&b).unwrap();
        recent.record(&a).unwrap();

        let recent = Recent::at(Some(file));
        let paths: Vec<_> = recent.entries.iter().map(|e| e.path.clone()).collect();
        assert_eq!(paths, [a, b]);
        let summary = &recent.entries[0].summary;
        assert_eq!((summary.names.len(), summary.scheduled), (2, 1));
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
        std::fs::write(dir.path().join("rocky.yaml"), "jobs: {x: jobs/x.yaml}\n").unwrap();
        let summary = summarize(dir.path());
        assert_eq!((summary.names.len(), summary.scheduled), (1, 1));
        assert!(matches!(summary.status, Status::Invalid(error) if error.contains("cmd")));
    }
}
