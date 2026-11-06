use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::defaults::Defaults;
use crate::error::Error;
use crate::schedule::Schedule;
use crate::task::{self, Task};
use crate::var_name;

/// Named jobs sharing `values` and an `env_file`: a directory with one YAML file per job in
/// `jobs/`, and an optional `rocky.yaml` of shared settings.
#[derive(Debug)]
pub struct Project {
    pub env_file: Option<PathBuf>,
    pub jobs: BTreeMap<String, Job>,
}

/// The optional file of settings shared by every job, in the project directory.
pub const SETTINGS_FILE: &str = "rocky.yaml";
/// The directory of job files, in the project directory.
pub const JOBS_DIR: &str = "jobs";

/// `rocky.yaml` as written.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    #[serde(default, deserialize_with = "unique")]
    values: BTreeMap<String, Value>,
    env_file: Option<PathBuf>,
    #[serde(default)]
    defaults: Defaults,
}

/// Reads a map, rejecting a key that appears twice instead of keeping the last value.
pub(crate) fn unique<'de, D, V>(deserializer: D) -> Result<BTreeMap<String, V>, D::Error>
where
    D: Deserializer<'de>,
    V: Deserialize<'de>,
{
    struct UniqueVisitor<V>(PhantomData<V>);

    impl<'de, V: Deserialize<'de>> Visitor<'de> for UniqueVisitor<V> {
        type Value = BTreeMap<String, V>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a map")
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut entries = BTreeMap::new();
            while let Some(key) = map.next_key::<String>()? {
                if entries.contains_key(&key) {
                    return Err(de::Error::custom(format!("duplicate key `{key}`")));
                }
                let value = map.next_value()?;
                entries.insert(key, value);
            }
            Ok(entries)
        }
    }

    deserializer.deserialize_map(UniqueVisitor(PhantomData))
}

/// Tasks run in order, by `rocky exec`, or by `rocky run` on the job's `schedule`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    /// When `rocky run` runs this job.
    #[serde(default)]
    pub schedule: Option<Schedule>,
    /// Static values for `${NAME}` in this job's tasks. Once loaded, the project's `values`
    /// are merged in, with the job's own winning.
    #[serde(default, deserialize_with = "unique")]
    pub values: BTreeMap<String, Value>,
    /// Fields this job's `cmd` tasks get unless they set them; already applied once loaded.
    #[serde(default)]
    pub defaults: Defaults,
    #[serde(deserialize_with = "task::named")]
    pub tasks: Vec<Task>,
}

impl Project {
    /// Removes and returns the job called `name`.
    pub fn take(&mut self, name: &str) -> Result<Job, Error> {
        self.jobs.remove(name).ok_or_else(|| Error::UnknownJob {
            name: name.to_string(),
            available: self.job_names(),
        })
    }

    /// The job names, each in backticks, joined with `, `.
    pub fn job_names(&self) -> String {
        self.jobs
            .keys()
            .map(|n| format!("`{n}`"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Loads the project in directory `root`: its `rocky.yaml`, if any, and every job file in
/// `jobs/`, and validates every job. Relative paths are resolved against the directory of the
/// file that contains them.
pub fn load(root: &Path) -> Result<Project, Error> {
    let settings_path = root.join(SETTINGS_FILE);
    let settings = match std::fs::read_to_string(&settings_path) {
        Ok(text) => Some(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(Error::Io {
                path: settings_path,
                source,
            });
        }
    };
    let mut jobs = Vec::new();
    for (name, path) in job_files(root)? {
        jobs.push((name, read_text(&path)?));
    }
    load_parts(root, settings.as_deref(), &jobs)
}

/// The job files of the project in `root`: every `*.yaml` file directly in `jobs/`, as
/// (job name, path) sorted by name. A missing `jobs/` has none.
pub fn job_files(root: &Path) -> Result<Vec<(String, PathBuf)>, Error> {
    let dir = root.join(JOBS_DIR);
    let io = |source| Error::Io {
        path: dir.clone(),
        source,
    };
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(io(err)),
    };
    let mut files = Vec::new();
    for entry in entries {
        let path = entry.map_err(io)?.path();
        if path.extension().is_some_and(|e| e == "yaml")
            && path.is_file()
            && let Some(name) = path.file_stem().and_then(|s| s.to_str())
        {
            files.push((name.to_string(), path.clone()));
        }
    }
    files.sort();
    Ok(files)
}

/// Like [`load`], with the contents of `rocky.yaml` (`None` if there is none) and of each
/// job file (job name, text) given instead of read from disk.
pub fn load_parts(
    root: &Path,
    settings: Option<&str>,
    jobs: &[(String, String)],
) -> Result<Project, Error> {
    if jobs.is_empty() {
        return Err(Error::InvalidProject(format!(
            "no jobs: add {JOBS_DIR}/<name>.yaml"
        )));
    }
    let settings: Settings = match settings {
        Some(text) => parse_yaml(&root.join(SETTINGS_FILE), text)?,
        None => Settings::default(),
    };
    check_names(settings.values.keys())?;
    settings.defaults.validate()?;
    let defaults = Defaults {
        dir: root.to_path_buf(),
        ..settings.defaults
    };
    let jobs_dir = root.join(JOBS_DIR);
    let mut loaded = BTreeMap::new();
    for (name, text) in jobs {
        check_job_name(name)?;
        let job = load_job(
            &jobs_dir.join(format!("{name}.yaml")),
            text,
            &settings.values,
            &defaults,
        )
        .map_err(|source| Error::Job {
            name: name.clone(),
            source: Box::new(source),
        })?;
        loaded.insert(name.clone(), job);
    }
    let project = Project {
        env_file: settings.env_file.map(|p| root.join(p)),
        jobs: loaded,
    };
    for (name, job) in &project.jobs {
        check_triggers(&job.tasks, &project).map_err(|source| Error::Job {
            name: name.clone(),
            source: Box::new(source),
        })?;
    }
    Ok(project)
}

/// Checks that every `trigger` in `tasks`, nested ones included, names a job of `project`.
fn check_triggers(tasks: &[Task], project: &Project) -> Result<(), Error> {
    for task in tasks {
        if let Some(job) = &task.trigger
            && !project.jobs.contains_key(job)
        {
            return Err(Error::Task {
                name: task.name.clone(),
                source: Box::new(Error::UnknownJob {
                    name: job.clone(),
                    available: project.job_names(),
                }),
            });
        }
        check_triggers(&task.body, project)?;
    }
    Ok(())
}

/// Parses the job file at `path` from `text`, validates its tasks, merges the project's
/// `values` under its own, and applies its `defaults`, over the project's, to its tasks.
fn load_job(
    path: &Path,
    text: &str,
    values: &BTreeMap<String, Value>,
    defaults: &Defaults,
) -> Result<Job, Error> {
    let mut job: Job = parse_yaml(path, text)?;
    let dir = path.parent().unwrap_or(Path::new("")).to_path_buf();
    if job.tasks.is_empty() {
        return Err(Error::InvalidProject("job has no tasks".to_string()));
    }
    check_names(job.values.keys())?;
    task::resolve(&mut job.tasks, &dir)?;
    job.values = values.clone().into_iter().chain(job.values).collect();
    job.defaults.validate()?;
    job.defaults.dir = dir;
    job.defaults = std::mem::take(&mut job.defaults).over(defaults);
    job.defaults.apply(&mut job.tasks);
    Ok(job)
}

/// Checks that `name` can be a job file's name in `jobs/`: not empty, without `/` or `\`,
/// and not starting with `.`.
pub fn check_job_name(name: &str) -> Result<(), Error> {
    if name.is_empty() || name.starts_with('.') || name.contains(['/', '\\']) {
        return Err(Error::InvalidJobName(name.to_string()));
    }
    Ok(())
}

/// Checks that every `values` name is a valid var name.
fn check_names<'a>(mut names: impl Iterator<Item = &'a String>) -> Result<(), Error> {
    match names.find(|name| !var_name::is_valid_var(name)) {
        Some(name) => Err(Error::InvalidProject(format!(
            "`values` name `{name}` {}",
            var_name::VAR_RULE
        ))),
        None => Ok(()),
    }
}

/// Reads a file to a string.
fn read_text(path: &Path) -> Result<String, Error> {
    std::fs::read_to_string(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Parses YAML `text`, naming `path` in the error.
fn parse_yaml<T: serde::de::DeserializeOwned>(path: &Path, text: &str) -> Result<T, Error> {
    serde_yaml::from_str(text).map_err(|source| Error::Yaml {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a project into `dir`: `settings` as `rocky.yaml` (if any) and each job file,
    /// then loads it.
    fn load_in(
        dir: &Path,
        settings: Option<&str>,
        jobs: &[(&str, &str)],
    ) -> Result<Project, Error> {
        if let Some(text) = settings {
            std::fs::write(dir.join(SETTINGS_FILE), text).unwrap();
        }
        let jobs_dir = dir.join(JOBS_DIR);
        std::fs::create_dir_all(&jobs_dir).unwrap();
        for (name, text) in jobs {
            std::fs::write(jobs_dir.join(format!("{name}.yaml")), text).unwrap();
        }
        load(dir)
    }

    const ECHO: &str = "tasks: {e: {cmd: [echo]}}\n";

    #[test]
    fn load_parts_reads_text_in_place_of_the_files() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/httpbin");
        let settings = std::fs::read_to_string(root.join(SETTINGS_FILE)).unwrap();
        let jobs: Vec<_> = job_files(&root)
            .unwrap()
            .into_iter()
            .map(|(name, path)| (name, std::fs::read_to_string(path).unwrap()))
            .collect();
        let from_text = load_parts(&root, Some(&settings), &jobs).unwrap();
        assert_eq!(from_text.job_names(), load(&root).unwrap().job_names());
        let err = load_parts(&root, None, &[("a".into(), "tasks: [unclosed".into())]).unwrap_err();
        assert!(
            matches!(&err, Error::Job { source, .. } if matches!(&**source, Error::Yaml { path, .. } if path.ends_with("jobs/a.yaml"))),
            "{err}"
        );
    }

    #[test]
    fn loads_every_example_project() {
        let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let mut loaded = 0;
        for entry in std::fs::read_dir(examples).unwrap() {
            let path = entry.unwrap().path();
            // Skip `.rocky` log directories left by `rocky run`.
            if path.is_dir() && path.join(JOBS_DIR).is_dir() {
                load(&path).unwrap_or_else(|e| panic!("{}: {}", path.display(), e.chain()));
                loaded += 1;
            }
        }
        assert!(loaded > 0);
    }

    #[test]
    fn names_jobs_by_file_and_resolves_paths() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("jobs.yaml"), "not a job dir").unwrap();
        std::fs::create_dir_all(dir.path().join("jobs/sub")).unwrap();
        std::fs::write(dir.path().join("jobs/sub/x.yaml"), "nope").unwrap();
        std::fs::write(dir.path().join("jobs/notes.txt"), "nope").unwrap();
        std::fs::write(dir.path().join("jobs/echo.yml"), "nope").unwrap();
        let mut project = load_in(
            dir.path(),
            Some("env_file: .env\n"),
            &[
                (
                    "a",
                    "schedule: {every: 5m}\ntasks:\n  e: {cmd: [echo], cwd: w}\n",
                ),
                ("b", ECHO),
            ],
        )
        .unwrap();
        assert_eq!(project.job_names(), "`a`, `b`");
        assert_eq!(project.env_file, Some(dir.path().join(".env")));
        let a = project.take("a").unwrap();
        assert!(a.schedule.is_some());
        assert_eq!(a.tasks[0].dir, dir.path().join("jobs"));
        let b = project.take("b").unwrap();
        assert_eq!(b.tasks[0].dir, dir.path().join("jobs"));
        assert!(b.schedule.is_none());
    }

    #[test]
    fn settings_file_is_optional() {
        let dir = tempfile::tempdir().unwrap();
        let project = load_in(dir.path(), None, &[("w", ECHO)]).unwrap();
        assert_eq!(project.env_file, None);
        let dir = tempfile::tempdir().unwrap();
        load_in(dir.path(), Some("# nothing yet\n"), &[("w", ECHO)]).unwrap();
    }

    #[test]
    fn job_names_must_be_file_names() {
        for name in ["", ".hidden", "a/b", "../x", "a\\b"] {
            let err = load_parts(Path::new(""), None, &[(name.into(), ECHO.into())]).unwrap_err();
            assert!(
                err.to_string().contains("must be a file name"),
                "{name}: {err}"
            );
        }
        load_parts(
            Path::new(""),
            None,
            &[("nightly-report".into(), ECHO.into())],
        )
        .unwrap();
    }

    #[test]
    fn needs_at_least_one_job_file() {
        let dir = tempfile::tempdir().unwrap();
        let err = load(dir.path()).unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid project: no jobs: add jobs/<name>.yaml"
        );
        let err = load_in(dir.path(), None, &[]).unwrap_err();
        assert!(matches!(err, Error::InvalidProject(_)), "{err}");
    }

    #[test]
    fn rejects_jobs_in_the_settings_file() {
        let dir = tempfile::tempdir().unwrap();
        let err = load_in(
            dir.path(),
            Some("jobs:\n  w: {tasks: {e: {cmd: [echo]}}}\n"),
            &[("w", ECHO)],
        )
        .unwrap_err();
        assert!(
            matches!(&err, Error::Yaml { path, .. } if path.ends_with(SETTINGS_FILE)),
            "{err}"
        );
        assert!(
            err.chain().contains("unknown field `jobs`"),
            "{}",
            err.chain()
        );
    }

    #[test]
    fn triggers_must_name_a_job_of_the_project() {
        let dir = tempfile::tempdir().unwrap();
        load_in(
            dir.path(),
            None,
            &[("a", "tasks: {t: {trigger: b}}\n"), ("b", ECHO)],
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let err = load_in(
            dir.path(),
            None,
            &[(
                "a",
                "tasks:\n  l:\n    for_each: X\n    do:\n      t: {when: {this: X, eq: 1}, trigger: nope}\n",
            )],
        )
        .unwrap_err();
        assert_eq!(
            err.chain(),
            "job `a`: task `t` failed: no job `nope` (available: `a`)"
        );
    }

    #[test]
    fn unknown_job_lists_available_names() {
        let dir = tempfile::tempdir().unwrap();
        let mut project = load_in(dir.path(), None, &[("a", ECHO), ("b", ECHO)]).unwrap();
        let err = project.take("c").unwrap_err();
        assert_eq!(err.to_string(), "no job `c` (available: `a`, `b`)");
    }

    #[test]
    fn task_errors_name_the_job() {
        let dir = tempfile::tempdir().unwrap();
        let err = load_in(dir.path(), None, &[("w", "tasks: {e: {cmd: []}}\n")]).unwrap_err();
        assert!(
            matches!(&err, Error::Job { name, source } if name == "w" && matches!(**source, Error::InvalidTask(_))),
            "{err}"
        );
        let dir = tempfile::tempdir().unwrap();
        let err = load_in(dir.path(), None, &[("w", "tasks: {}\n")]).unwrap_err();
        assert!(
            matches!(&err, Error::Job { name, .. } if name == "w"),
            "{err}"
        );
    }

    #[test]
    fn loads_and_checks_values_names() {
        let dir = tempfile::tempdir().unwrap();
        let mut project = load_in(
            dir.path(),
            Some("values: {BASE: https://x, N: 3, CONF: {a: [1]}}\n"),
            &[("w", "values: {N: 4, _K2: k}\ntasks: {e: {cmd: [echo]}}\n")],
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&project.take("w").unwrap().values).unwrap(),
            serde_json::json!({"BASE": "https://x", "N": 4, "CONF": {"a": [1]}, "_K2": "k"})
        );

        let dir = tempfile::tempdir().unwrap();
        let err = load_in(dir.path(), Some("values: {a: x}\n"), &[("w", ECHO)]).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!("invalid project: `values` name `a` {}", var_name::VAR_RULE)
        );
        let dir = tempfile::tempdir().unwrap();
        let err = load_in(
            dir.path(),
            None,
            &[("w", "values: {A-B: x}\ntasks: {e: {cmd: [echo]}}\n")],
        )
        .unwrap_err();
        assert!(
            matches!(&err, Error::Job { name, source } if name == "w" && source.to_string().contains("`A-B`")),
            "{err}"
        );
    }

    #[test]
    fn rejects_vars_and_job_when() {
        for (settings, job, field) in [
            (Some("vars: {A: x}\n"), ECHO, "vars"),
            (None, "vars: {A: x}\ntasks: {e: {cmd: [echo]}}\n", "vars"),
            (None, "tasks: {e: {vars: {A: x}, cmd: [echo]}}\n", "vars"),
            (
                None,
                "when: {this: X, eq: 1}\ntasks: {e: {cmd: [echo]}}\n",
                "when",
            ),
            (None, "tasks: {e: {cmd: [echo]}}\nactoins: 1\n", "actoins"),
            (Some("schedules: {w: {every: 1s}}\n"), ECHO, "schedules"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let err = load_in(dir.path(), settings, &[("w", job)]).unwrap_err();
            assert!(
                err.chain().contains(&format!("unknown field `{field}`")),
                "{job}: {}",
                err.chain()
            );
        }
    }

    #[test]
    fn rejects_duplicate_keys() {
        for (settings, job, expected) in [
            (
                Some("values: {A: x, A: y}\n"),
                ECHO,
                "values: duplicate key `A`",
            ),
            (
                None,
                "values: {A: x, A: y}\ntasks: {e: {cmd: [echo]}}\n",
                "values: duplicate key `A`",
            ),
            (
                None,
                "tasks: {g: {cmd: [echo], env: {A: '1', A: '2'}}}\n",
                "tasks.g.env: duplicate key `A`",
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let err = load_in(dir.path(), settings, &[("w", job)]).unwrap_err();
            assert!(err.chain().contains(expected), "{job}: {}", err.chain());
        }
    }

    #[test]
    fn rejects_invalid_yaml() {
        for job in ["tasks: {e: {cmd: [echo], extract: {}}}\n", "tasks: [\n"] {
            let dir = tempfile::tempdir().unwrap();
            let err = load_in(dir.path(), None, &[("w", job)]).unwrap_err();
            assert!(
                matches!(&err, Error::Job { source, .. } if matches!(**source, Error::Yaml { .. })),
                "{job}"
            );
            assert!(err.chain().contains("w.yaml"), "{}", err.chain());
        }
    }

    #[test]
    fn defaults_fill_cmd_tasks_and_layer_job_over_project() {
        let dir = tempfile::tempdir().unwrap();
        let mut project = load_in(
            dir.path(),
            Some("defaults:\n  timeout: 30s\n  env: {A: project, B: project}\n  retry: {attempts: 2}\n"),
            &[(
                "w",
                "defaults:\n  timeout: 5s\n  env: {B: job, C: job}\ntasks:\n\
                 \x20 plain: {cmd: [echo]}\n\
                 \x20 own: {cmd: [echo], timeout: 1s, env: {C: task}, retry: {attempts: 9}}\n\
                 \x20 each:\n    for_each: X\n    do: {nested: {cmd: [echo]}}\n",
            )],
        )
        .unwrap();
        let job = project.take("w").unwrap();
        let env = |task: &Task| serde_json::to_value(&task.env).unwrap();
        let plain = &job.tasks[0];
        assert_eq!(plain.timeout, Some(5));
        assert_eq!(
            env(plain),
            serde_json::json!({"A": "project", "B": "job", "C": "job"})
        );
        assert_eq!(plain.retry.as_ref().map(|r| r.attempts), Some(2));
        let own = &job.tasks[1];
        assert_eq!(own.timeout, Some(1));
        assert_eq!(
            env(own),
            serde_json::json!({"A": "project", "B": "job", "C": "task"})
        );
        assert_eq!(own.retry.as_ref().map(|r| r.attempts), Some(9));
        let each = &job.tasks[2];
        assert_eq!(each.timeout, None);
        assert!(each.env.is_empty());
        assert_eq!(each.body[0].timeout, Some(5));
    }

    #[test]
    fn default_cwd_is_relative_to_the_file_with_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let jobs = dir.path().join("jobs");
        let mut project = load_in(
            dir.path(),
            Some("defaults: {cwd: .}\n"),
            &[
                ("own", "defaults: {cwd: work}\ntasks:\n  t: {cmd: [echo]}\n"),
                ("inherit", "tasks:\n  t: {cmd: [echo]}\n"),
                ("set", "tasks: {t: {cmd: [echo], cwd: mine}}\n"),
            ],
        )
        .unwrap();
        let task = |project: &mut Project, name: &str| project.take(name).unwrap().tasks.remove(0);
        let own = task(&mut project, "own");
        assert_eq!(
            (own.cwd, own.cwd_dir, own.dir),
            (Some("work".into()), jobs.clone(), jobs.clone())
        );
        let inherit = task(&mut project, "inherit");
        assert_eq!(
            (inherit.cwd, inherit.cwd_dir),
            (Some(".".into()), dir.path().to_path_buf())
        );
        let set = task(&mut project, "set");
        assert_eq!((set.cwd, set.cwd_dir), (Some("mine".into()), jobs.clone()));
    }

    #[test]
    fn rejects_invalid_defaults() {
        for (settings, job, expected) in [
            (
                Some("defaults: {allow_failure: true}\n"),
                ECHO,
                "unknown field `allow_failure`",
            ),
            (
                Some("defaults: {timeout: soon}\n"),
                ECHO,
                "is not a duration",
            ),
            (Some("defaults: {timeout: 30}\n"), ECHO, "a duration"),
            (
                Some("defaults: {retry: {attempts: 0}}\n"),
                ECHO,
                "`defaults`: `retry` needs `attempts` of at least 1",
            ),
            (
                None,
                "defaults: {retry: {attempts: 0}}\ntasks: {t: {cmd: [echo]}}\n",
                "`defaults`: `retry` needs `attempts` of at least 1",
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let err = load_in(dir.path(), settings, &[("w", job)]).unwrap_err();
            assert!(err.chain().contains(expected), "{job}: {}", err.chain());
        }
    }
}
