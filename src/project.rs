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

/// Named jobs sharing `values` and an `env_file`, described in a YAML file.
#[derive(Debug)]
pub struct Project {
    pub env_file: Option<PathBuf>,
    pub jobs: BTreeMap<String, Job>,
}

/// A project as written, before job files are loaded.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProject {
    #[serde(default, deserialize_with = "unique")]
    values: BTreeMap<String, Value>,
    env_file: Option<PathBuf>,
    #[serde(default)]
    defaults: Defaults,
    #[serde(deserialize_with = "unique")]
    jobs: BTreeMap<String, JobSource>,
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

/// A `jobs` entry: a job YAML file, the same fields inline, or a job file with its own
/// `schedule` and `values`.
enum JobSource {
    Path(PathBuf),
    Inline(Box<Job>),
    File {
        file: PathBuf,
        schedule: Option<Schedule>,
        values: BTreeMap<String, Value>,
    },
}

/// A `jobs` entry written as a map: an inline job, or `file` with overrides.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JobEntry {
    file: Option<PathBuf>,
    #[serde(default)]
    schedule: Option<Schedule>,
    #[serde(default, deserialize_with = "unique")]
    values: BTreeMap<String, Value>,
    defaults: Option<Defaults>,
    #[serde(default, deserialize_with = "some_named")]
    tasks: Option<Vec<Task>>,
}

fn some_named<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Vec<Task>>, D::Error> {
    task::named(deserializer).map(Some)
}

/// Picks the variant by YAML shape, so a mistake in an inline job reports the real
/// cause and line instead of a generic untagged-enum error.
impl<'de> Deserialize<'de> for JobSource {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct SourceVisitor;

        impl<'de> Visitor<'de> for SourceVisitor {
            type Value = JobSource;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a job file path or a job")
            }

            fn visit_str<E: de::Error>(self, path: &str) -> Result<JobSource, E> {
                Ok(JobSource::Path(PathBuf::from(path)))
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<JobSource, A::Error> {
                let entry = JobEntry::deserialize(de::value::MapAccessDeserializer::new(map))?;
                match (entry.file, entry.tasks) {
                    (Some(_), None) if entry.defaults.is_some() => Err(de::Error::custom(
                        "a job with `file` can't have `defaults`; set them in the job file",
                    )),
                    (Some(file), None) => Ok(JobSource::File {
                        file,
                        schedule: entry.schedule,
                        values: entry.values,
                    }),
                    (None, Some(tasks)) => Ok(JobSource::Inline(Box::new(Job {
                        schedule: entry.schedule,
                        values: entry.values,
                        defaults: entry.defaults.unwrap_or_default(),
                        tasks,
                    }))),
                    (Some(_), Some(_)) => Err(de::Error::custom(
                        "a job can't have both `file` and `tasks`",
                    )),
                    (None, None) => Err(de::Error::missing_field("tasks")),
                }
            }
        }

        deserializer.deserialize_any(SourceVisitor)
    }
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

/// Loads a project and its job files, and validates every job. Relative paths are
/// resolved against the directory of the file that contains them.
pub fn load(path: &Path) -> Result<Project, Error> {
    let raw: RawProject = read_yaml(path)?;
    if raw.jobs.is_empty() {
        return Err(Error::InvalidProject("project has no jobs".to_string()));
    }
    check_names(raw.values.keys())?;
    raw.defaults.validate()?;
    let dir = path.parent().unwrap_or(Path::new(""));
    let defaults = Defaults {
        dir: dir.to_path_buf(),
        ..raw.defaults
    };
    let mut jobs = BTreeMap::new();
    for (name, source) in raw.jobs {
        let job = load_job(source, dir, &raw.values, &defaults).map_err(|source| Error::Job {
            name: name.clone(),
            source: Box::new(source),
        })?;
        jobs.insert(name, job);
    }
    let project = Project {
        env_file: raw.env_file.map(|p| dir.join(p)),
        jobs,
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
        check_triggers(&task.then, project)?;
        check_triggers(&task.otherwise, project)?;
        check_triggers(&task.body, project)?;
        for case in &task.cases {
            check_triggers(&case.body, project)?;
        }
        check_triggers(&task.default, project)?;
    }
    Ok(())
}

/// Reads a job file if `source` names one, applies the entry's `schedule` and `values` over
/// the file's, validates its tasks, merges the project's `values` under its own, and applies
/// its `defaults`, over the project's, to its tasks.
fn load_job(
    source: JobSource,
    dir: &Path,
    values: &BTreeMap<String, Value>,
    defaults: &Defaults,
) -> Result<Job, Error> {
    let (mut job, dir) = match source {
        JobSource::Path(path) => read_job(&dir.join(path))?,
        JobSource::Inline(job) => (*job, dir.to_path_buf()),
        JobSource::File {
            file,
            schedule,
            values,
        } => {
            let (mut job, dir) = read_job(&dir.join(file))?;
            job.schedule = schedule.or(job.schedule);
            job.values.extend(values);
            (job, dir)
        }
    };
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

/// Reads a job file, returning it with the directory its relative paths resolve against.
fn read_job(path: &Path) -> Result<(Job, PathBuf), Error> {
    let job = read_yaml(path)?;
    Ok((job, path.parent().unwrap_or(Path::new("")).to_path_buf()))
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

/// Reads and parses a YAML file.
pub(crate) fn read_yaml<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, Error> {
    let text = std::fs::read_to_string(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    serde_yaml::from_str(&text).map_err(|source| Error::Yaml {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_in(dir: &Path, yaml: &str) -> Result<Project, Error> {
        let path = dir.join("project.yaml");
        std::fs::write(&path, yaml).unwrap();
        load(&path)
    }

    #[test]
    fn loads_every_example_project() {
        let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
        let mut loaded = 0;
        for entry in std::fs::read_dir(examples).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "yaml") {
                load(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                loaded += 1;
            }
        }
        assert!(loaded > 0);
    }

    #[test]
    fn loads_jobs_and_resolves_paths_against_project_dir() {
        let dir = tempfile::tempdir().unwrap();
        let mut project = load_in(
            dir.path(),
            "env_file: .env\n\
             jobs:\n\
             \x20 a:\n    schedule: {every: 5m}\n    tasks:\n      e:\n        cmd: [echo]\n        cwd: work\n\
             \x20 b:\n    tasks: {e: {cmd: [echo]}}\n",
        )
        .unwrap();
        assert_eq!(project.env_file, Some(dir.path().join(".env")));
        let a = project.take("a").unwrap();
        assert!(a.schedule.is_some());
        assert_eq!(a.tasks[0].dir, dir.path());
        let b = project.take("b").unwrap();
        assert!(b.tasks[0].cmd.is_some() && b.schedule.is_none());
    }

    #[test]
    fn loads_job_files_relative_to_themselves() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("jobs");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(
            sub.join("w.yaml"),
            "schedule: {every: 1s}\ntasks: {e: echo.yaml}\n",
        )
        .unwrap();
        std::fs::write(sub.join("echo.yaml"), "cmd: [echo]\ncwd: w\n").unwrap();
        let mut project = load_in(dir.path(), "jobs:\n  w: jobs/w.yaml\n").unwrap();
        let w = project.take("w").unwrap();
        assert_eq!(w.tasks[0].dir, sub);
        assert!(w.schedule.is_some());

        let err = load_in(dir.path(), "jobs:\n  m: nope.yaml\n").unwrap_err();
        assert!(
            matches!(&err, Error::Job { name, source } if name == "m" && matches!(**source, Error::Io { .. })),
            "{err}"
        );
    }

    #[test]
    fn reuses_a_job_file_with_its_own_schedule_and_values() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("jobs");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(
            sub.join("fetch.yaml"),
            "schedule: {every: 1s}\nvalues: {TARGET: file, N: 1}\ntasks: {e: {cmd: [echo], cwd: w}}\n",
        )
        .unwrap();
        let mut project = load_in(
            dir.path(),
            "values: {TARGET: project, BASE: b}\n\
             jobs:\n\
             \x20 a: {file: jobs/fetch.yaml, values: {TARGET: a}}\n\
             \x20 b: {file: jobs/fetch.yaml, schedule: {every: 1h}, values: {TARGET: b}}\n\
             \x20 c: {file: jobs/fetch.yaml}\n",
        )
        .unwrap();
        let values = |job: &Job| serde_json::to_value(&job.values).unwrap();
        let a = project.take("a").unwrap();
        assert_eq!(
            values(&a),
            serde_json::json!({"TARGET": "a", "N": 1, "BASE": "b"})
        );
        assert_eq!(a.tasks[0].dir, sub);
        let b = project.take("b").unwrap();
        assert_eq!(values(&b)["TARGET"], "b");
        assert_ne!(format!("{:?}", a.schedule), format!("{:?}", b.schedule));
        let c = project.take("c").unwrap();
        assert_eq!(values(&c)["TARGET"], "file");
        assert_eq!(format!("{:?}", a.schedule), format!("{:?}", c.schedule));
    }

    #[test]
    fn rejects_bad_job_file_entries() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("j.yaml"), "tasks: {e: {cmd: [echo]}}\n").unwrap();
        for (yaml, expected) in [
            (
                "jobs:\n  w: {file: j.yaml, tasks: {e: {cmd: [echo]}}}\n",
                "can't have both `file` and `tasks`",
            ),
            ("jobs:\n  w: {values: {A: x}}\n", "missing field `tasks`"),
            (
                "jobs:\n  w: {file: j.yaml, cmd: [echo]}\n",
                "unknown field `cmd`",
            ),
        ] {
            let err = load_in(dir.path(), yaml).unwrap_err();
            let cause = std::error::Error::source(&err).unwrap().to_string();
            assert!(cause.contains(expected), "{yaml}: {cause}");
        }
        let err = load_in(dir.path(), "jobs:\n  w: {file: j.yaml, values: {a: x}}\n").unwrap_err();
        assert!(
            matches!(&err, Error::Job { name, source } if name == "w" && source.to_string().contains("`a`")),
            "{err}"
        );
        let err = load_in(dir.path(), "jobs:\n  w: {file: nope.yaml}\n").unwrap_err();
        assert!(
            matches!(&err, Error::Job { source, .. } if matches!(**source, Error::Io { .. })),
            "{err}"
        );
    }

    #[test]
    fn triggers_must_name_a_job_of_the_project() {
        let dir = tempfile::tempdir().unwrap();
        load_in(
            dir.path(),
            "jobs:\n  a: {tasks: {t: {trigger: b}}}\n  b: {tasks: {e: {cmd: [echo]}}}\n",
        )
        .unwrap();
        let err = load_in(
            dir.path(),
            "jobs:\n  a:\n    tasks:\n      l:\n        for_each: X\n        do:\n\
             \x20         r: {if: {this: X, eq: 1}, then: {t: {trigger: nope}}}\n",
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
        let mut project = load_in(
            dir.path(),
            "jobs:\n  a: {tasks: {e: {cmd: [echo]}}}\n  b: {tasks: {e: {cmd: [echo]}}}\n",
        )
        .unwrap();
        let err = project.take("c").unwrap_err();
        assert_eq!(err.to_string(), "no job `c` (available: `a`, `b`)");
    }

    #[test]
    fn task_errors_name_the_job() {
        let dir = tempfile::tempdir().unwrap();
        let err = load_in(dir.path(), "jobs:\n  w:\n    tasks: {e: {cmd: []}}\n").unwrap_err();
        assert!(
            matches!(&err, Error::Job { name, source } if name == "w" && matches!(**source, Error::InvalidTask(_))),
            "{err}"
        );
        let err = load_in(dir.path(), "jobs:\n  w: {tasks: {}}\n").unwrap_err();
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
            "values: {BASE: https://x, N: 3, CONF: {a: [1]}}\n\
             jobs:\n  w: {values: {N: 4, _K2: k}, tasks: {e: {cmd: [echo]}}}\n",
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&project.take("w").unwrap().values).unwrap(),
            serde_json::json!({"BASE": "https://x", "N": 4, "CONF": {"a": [1]}, "_K2": "k"})
        );

        let err = load_in(
            dir.path(),
            "values: {a: x}\njobs:\n  w: {tasks: {e: {cmd: [echo]}}}\n",
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            format!("invalid project: `values` name `a` {}", var_name::VAR_RULE)
        );
        let err = load_in(
            dir.path(),
            "jobs:\n  w: {values: {A-B: x}, tasks: {e: {cmd: [echo]}}}\n",
        )
        .unwrap_err();
        assert!(
            matches!(&err, Error::Job { name, source } if name == "w" && source.to_string().contains("`A-B`")),
            "{err}"
        );
    }

    #[test]
    fn rejects_vars() {
        let dir = tempfile::tempdir().unwrap();
        for yaml in [
            "vars: {A: x}\njobs:\n  w: {tasks: {e: {cmd: [echo]}}}\n",
            "jobs:\n  w: {vars: {A: x}, tasks: {e: {cmd: [echo]}}}\n",
            "jobs:\n  w: {tasks: {e: {vars: {A: x}, cmd: [echo]}}}\n",
        ] {
            let err = load_in(dir.path(), yaml).unwrap_err();
            let cause = std::error::Error::source(&err).unwrap().to_string();
            assert!(cause.contains("unknown field `vars`"), "{yaml}: {cause}");
        }
    }

    #[test]
    fn rejects_duplicate_keys() {
        let dir = tempfile::tempdir().unwrap();
        let w = "w: {tasks: {e: {cmd: [echo]}}}";
        for (yaml, expected) in [
            (
                format!("jobs:\n  {w}\n  w: other.yaml\n"),
                "jobs: duplicate key `w`",
            ),
            (
                format!("values: {{A: x, A: y}}\njobs: {{{w}}}\n"),
                "values: duplicate key `A`",
            ),
            (
                "jobs:\n  w: {values: {A: x, A: y}, tasks: {e: {cmd: [echo]}}}\n".to_string(),
                "jobs.w.values: duplicate key `A`",
            ),
            (
                "jobs:\n  w: {tasks: {g: {cmd: [echo], env: {A: '1', A: '2'}}}}\n".to_string(),
                "jobs.w.tasks.g.env: duplicate key `A`",
            ),
        ] {
            let err = load_in(dir.path(), &yaml).unwrap_err();
            let cause = std::error::Error::source(&err).unwrap().to_string();
            assert!(cause.starts_with(expected), "{yaml}: {cause}");
        }
    }

    #[test]
    fn rejects_empty_and_invalid_files() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            load_in(dir.path(), "jobs: {}\n").unwrap_err(),
            Error::InvalidProject(_)
        ));
        for yaml in [
            "jobs:\n  w: {tasks: {e: {cmd: [echo], extract: {}}}}\n",
            "jobs: [\n",
            "schedules: {w: {every: 1s}}\njobs:\n  w: {tasks: {e: {cmd: [echo]}}}\n",
        ] {
            assert!(
                matches!(load_in(dir.path(), yaml), Err(Error::Yaml { .. })),
                "{yaml}"
            );
        }
        let err = load_in(
            dir.path(),
            "jobs:\n  w:\n    tasks: {e: {cmd: [echo]}}\n    actoins: 1\n",
        )
        .unwrap_err();
        let cause = std::error::Error::source(&err).unwrap().to_string();
        assert!(
            cause.starts_with("jobs.w: unknown field `actoins`"),
            "{cause}"
        );
        let err = load_in(dir.path(), "jobs: [\n").unwrap_err();
        assert!(err.to_string().contains("project.yaml"), "{err}");
    }

    #[test]
    fn rejects_a_job_that_is_neither_a_path_nor_a_map() {
        let dir = tempfile::tempdir().unwrap();
        let err = load_in(dir.path(), "jobs:\n  a: 5\n").unwrap_err();
        assert!(err.chain().contains("a job file path or a job"), "{err}");
    }

    #[test]
    fn defaults_fill_cmd_tasks_and_layer_job_over_project() {
        let dir = tempfile::tempdir().unwrap();
        let mut project = load_in(
            dir.path(),
            "defaults:\n  timeout: 30s\n  env: {A: project, B: project}\n  retry: {attempts: 2}\n\
             jobs:\n  w:\n    defaults:\n      timeout: 5\n      env: {B: job, C: job}\n    tasks:\n\
             \x20     plain: {cmd: [echo]}\n\
             \x20     own: {cmd: [echo], timeout: 1, env: {C: task}, retry: {attempts: 9}}\n\
             \x20     branch:\n        if: {this: X, eq: 1}\n        then: {nested: {cmd: [echo]}}\n",
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
        let branch = &job.tasks[2];
        assert_eq!(branch.timeout, None);
        assert!(branch.env.is_empty());
        assert_eq!(branch.then[0].timeout, Some(5));
    }

    #[test]
    fn default_cwd_is_relative_to_the_file_with_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let jobs = dir.path().join("jobs");
        let tasks = dir.path().join("tasks");
        std::fs::create_dir(&jobs).unwrap();
        std::fs::create_dir(&tasks).unwrap();
        std::fs::write(tasks.join("t.yaml"), "cmd: [echo]\n").unwrap();
        std::fs::write(
            jobs.join("own.yaml"),
            "defaults: {cwd: work}\ntasks:\n  t: ../tasks/t.yaml\n",
        )
        .unwrap();
        std::fs::write(jobs.join("inherit.yaml"), "tasks:\n  t: ../tasks/t.yaml\n").unwrap();
        let mut project = load_in(
            dir.path(),
            "defaults: {cwd: .}\njobs:\n  own: jobs/own.yaml\n  inherit: jobs/inherit.yaml\n\
             \x20 set:\n    tasks: {t: {cmd: [echo], cwd: mine}}\n",
        )
        .unwrap();
        let task = |project: &mut Project, name: &str| project.take(name).unwrap().tasks.remove(0);
        let own = task(&mut project, "own");
        assert_eq!(
            (own.cwd, own.cwd_dir, own.dir),
            (Some("work".into()), jobs.clone(), jobs.join("../tasks"))
        );
        let inherit = task(&mut project, "inherit");
        assert_eq!(
            (inherit.cwd, inherit.cwd_dir),
            (Some(".".into()), dir.path().to_path_buf())
        );
        let set = task(&mut project, "set");
        assert_eq!(
            (set.cwd, set.cwd_dir),
            (Some("mine".into()), dir.path().to_path_buf())
        );
    }

    #[test]
    fn rejects_invalid_defaults() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("j.yaml"), "tasks: {t: {cmd: [echo]}}\n").unwrap();
        let job = "jobs:\n  w: {tasks: {t: {cmd: [echo]}}}\n";
        for (yaml, expected) in [
            (format!("defaults: {{allow_failure: true}}\n{job}"), "unknown field `allow_failure`"),
            (format!("defaults: {{timeout: soon}}\n{job}"), "is not a duration"),
            (
                format!("defaults: {{retry: {{attempts: 0}}}}\n{job}"),
                "`defaults`: `retry` needs `attempts` of at least 1",
            ),
            (
                "jobs:\n  w:\n    defaults: {retry: {attempts: 0}}\n    tasks: {t: {cmd: [echo]}}\n"
                    .to_string(),
                "`defaults`: `retry` needs `attempts` of at least 1",
            ),
            (
                "jobs:\n  w: {file: j.yaml, defaults: {timeout: 1}}\n".to_string(),
                "can't have `defaults`",
            ),
        ] {
            let err = load_in(dir.path(), &yaml).unwrap_err();
            assert!(err.chain().contains(expected), "{yaml}: {}", err.chain());
        }
    }
}
