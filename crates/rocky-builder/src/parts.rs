//! The builder edits a project as one YAML tree: the fields of `rocky.yaml` plus
//! `jobs: {name: job}`, one entry per job file. These convert that tree to and from the
//! project's files.

use std::path::Path;

use anyhow::Context as _;
use rocky_core::error::Error;
use rocky_core::project::{self, JOBS_DIR, Project, SETTINGS_FILE};
use serde_yaml::{Mapping, Value};

/// The project's files as text: `rocky.yaml` (`None` when it has no fields) and each job
/// file as (job name, text).
#[derive(Debug, PartialEq)]
pub struct Parts {
    pub settings: Option<String>,
    pub jobs: Vec<(String, String)>,
}

/// Reads the project in `dir` into one tree. A missing `rocky.yaml` or `jobs/` is empty.
pub fn read(dir: &Path) -> anyhow::Result<Value> {
    if !dir.is_dir() {
        anyhow::bail!("{} is not a project folder", dir.display());
    }
    let settings_path = dir.join(SETTINGS_FILE);
    let mut root = match std::fs::read_to_string(&settings_path) {
        Ok(text) => serde_yaml::from_str(&text)
            .with_context(|| format!("cannot parse {}", settings_path.display()))?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Value::Null,
        Err(err) => {
            return Err(err).with_context(|| format!("cannot read {}", settings_path.display()));
        }
    };
    let mut jobs = Mapping::new();
    for (name, path) in project::job_files(dir)? {
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        let job: Value = serde_yaml::from_str(&text)
            .with_context(|| format!("cannot parse {}", path.display()))?;
        jobs.insert(Value::from(name), job);
    }
    crate::doc::as_map(&mut root).insert(Value::from("jobs"), Value::Mapping(jobs));
    Ok(root)
}

/// Splits the tree written as `text` into the project's files.
pub fn split(text: &str) -> Result<Parts, serde_yaml::Error> {
    let mut root: Mapping = match serde_yaml::from_str(text)? {
        Value::Null => Mapping::new(),
        value => serde_yaml::from_value(value)?,
    };
    let jobs = match root.shift_remove("jobs") {
        Some(Value::Mapping(jobs)) => jobs,
        Some(Value::Null) | None => Mapping::new(),
        Some(other) => serde_yaml::from_value(other)?,
    };
    let jobs = jobs
        .iter()
        .map(|(name, job)| Ok((crate::doc::key_text(name), serde_yaml::to_string(job)?)))
        .collect::<Result<_, serde_yaml::Error>>()?;
    let settings = if root.is_empty() {
        None
    } else {
        Some(serde_yaml::to_string(&root)?)
    };
    Ok(Parts { settings, jobs })
}

/// Loads the tree written as `text` with rocky-core, as the project in `dir`.
pub fn load(dir: &Path, text: &str) -> Result<Project, Error> {
    let parts = split(text).map_err(|source| Error::Yaml {
        path: dir.to_path_buf(),
        source,
    })?;
    project::load_parts(dir, parts.settings.as_deref(), &parts.jobs)
}

/// Writes the files of tree `text` that differ from those of `saved` (the tree last read or
/// written) into `dir`, and removes the job files of jobs that are gone.
pub fn write(dir: &Path, saved: &str, text: &str) -> anyhow::Result<()> {
    let old = split(saved)?;
    let new = split(text)?;
    for (name, _) in &new.jobs {
        project::check_job_name(name)?;
    }
    let settings_path = dir.join(SETTINGS_FILE);
    if new.settings != old.settings && (new.settings.is_some() || settings_path.exists()) {
        write_file(&settings_path, new.settings.as_deref().unwrap_or(""))?;
    }
    let jobs_dir = dir.join(JOBS_DIR);
    let job_path = |name: &str| jobs_dir.join(format!("{name}.yaml"));
    for (name, job) in &new.jobs {
        if !old.jobs.contains(&(name.clone(), job.clone())) {
            std::fs::create_dir_all(&jobs_dir)
                .with_context(|| format!("cannot create {}", jobs_dir.display()))?;
            write_file(&job_path(name), job)?;
        }
    }
    for (name, _) in &old.jobs {
        if !new.jobs.iter().any(|(new_name, _)| new_name == name) {
            let path = job_path(name);
            std::fs::remove_file(&path)
                .with_context(|| format!("cannot remove {}", path.display()))?;
        }
    }
    Ok(())
}

fn write_file(path: &Path, text: &str) -> anyhow::Result<()> {
    std::fs::write(path, text).with_context(|| format!("cannot write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_settings_from_jobs() {
        let parts =
            split("values: {A: 1}\njobs:\n  b: {tasks: {t: {cmd: [echo]}}}\n  a: {tasks: {}}\n")
                .unwrap();
        assert_eq!(parts.settings.as_deref(), Some("values:\n  A: 1\n"));
        let names: Vec<_> = parts.jobs.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["b", "a"]);
        assert_eq!(split("jobs: {}\n").unwrap().settings, None);
    }

    #[test]
    fn reads_and_writes_only_what_changed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("jobs")).unwrap();
        let keep = "# a comment kept until this job is edited\ntasks: {t: {cmd: [echo]}}\n";
        std::fs::write(dir.path().join("jobs/keep.yaml"), keep).unwrap();
        std::fs::write(
            dir.path().join("jobs/old.yaml"),
            "tasks: {t: {cmd: [echo]}}\n",
        )
        .unwrap();
        let root = read(dir.path()).unwrap();
        let saved = serde_yaml::to_string(&root).unwrap();

        let mut edited = root.clone();
        let jobs = crate::doc::child_map(&mut edited, "jobs");
        crate::doc::rename(jobs, "old", "new");
        crate::doc::set(
            crate::doc::as_map(&mut edited),
            "env_file",
            Some(".env".into()),
        );
        let text = serde_yaml::to_string(&edited).unwrap();
        write(dir.path(), &saved, &text).unwrap();

        let file = |name: &str| std::fs::read_to_string(dir.path().join(name)).ok();
        assert_eq!(file("jobs/keep.yaml").as_deref(), Some(keep));
        assert_eq!(file("jobs/old.yaml"), None);
        assert!(file("jobs/new.yaml").is_some());
        assert_eq!(file("rocky.yaml").as_deref(), Some("env_file: .env\n"));
        let back = read(dir.path()).unwrap();
        assert_eq!(back["env_file"], ".env");
        assert_eq!(
            crate::doc::keys(back["jobs"].as_mapping().unwrap()),
            ["keep", "new"]
        );
    }
}
