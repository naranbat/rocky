use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::error::Error;
use crate::schedule::Schedule;
use crate::task::{self, Task};
use crate::{state, var_name};

/// Named jobs sharing `values` and an `env_file`, described in a YAML file.
#[derive(Debug)]
pub struct Project {
    /// The project file's directory; job state files go in its `states/`.
    pub dir: PathBuf,
    pub env_file: Option<PathBuf>,
    /// Job name to when `rocky run` runs it.
    pub schedules: BTreeMap<String, Schedule>,
    pub jobs: BTreeMap<String, Job>,
}

/// A project as written, before job files are loaded.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProject {
    #[serde(default, deserialize_with = "unique")]
    values: BTreeMap<String, Value>,
    env_file: Option<PathBuf>,
    #[serde(default, deserialize_with = "unique")]
    schedules: BTreeMap<String, Schedule>,
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

/// A `jobs` entry: a job YAML file, or the same fields inline.
enum JobSource {
    Path(PathBuf),
    Inline(Job),
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
                Job::deserialize(de::value::MapAccessDeserializer::new(map)).map(JobSource::Inline)
            }
        }

        deserializer.deserialize_any(SourceVisitor)
    }
}

/// Tasks run in order, by `rocky exec`, or by `rocky run` on the project's `schedules`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    /// Static values for `${NAME}` in this job's tasks. Once loaded, the project's `values`
    /// are merged in, with the job's own winning.
    #[serde(default, deserialize_with = "unique")]
    pub values: BTreeMap<String, Value>,
    /// State names to their defaults, kept across runs in `states/<job>.json`.
    #[serde(default, deserialize_with = "unique")]
    pub states: BTreeMap<String, Value>,
    #[serde(deserialize_with = "task::named")]
    pub tasks: Vec<Task>,
}

impl Project {
    /// Removes and returns the job called `name`.
    pub fn take(&mut self, name: &str) -> Result<Job, Error> {
        self.jobs.remove(name).ok_or_else(|| Error::UnknownJob {
            name: name.to_string(),
            available: self
                .jobs
                .keys()
                .map(|n| format!("`{n}`"))
                .collect::<Vec<_>>()
                .join(", "),
        })
    }
}

/// Loads a project and its job files, and validates every job. Relative paths are
/// resolved against the directory of the file that contains them.
pub fn load(path: &Path) -> Result<Project, Error> {
    let raw: RawProject = read_yaml(path)?;
    if raw.jobs.is_empty() {
        return Err(Error::InvalidProject("project has no jobs".to_string()));
    }
    if let Some(name) = raw
        .schedules
        .keys()
        .find(|name| !raw.jobs.contains_key(*name))
    {
        return Err(Error::InvalidProject(format!(
            "schedule for unknown job `{name}`"
        )));
    }
    check_names(
        "values",
        raw.values.keys(),
        var_name::is_valid_var,
        var_name::VAR_RULE,
    )?;
    let dir = path.parent().unwrap_or(Path::new(""));
    let mut jobs = BTreeMap::new();
    for (name, source) in raw.jobs {
        let job = load_job(source, dir, &raw.values).map_err(|source| Error::Job {
            name: name.clone(),
            source: Box::new(source),
        })?;
        jobs.insert(name, job);
    }
    Ok(Project {
        dir: dir.to_path_buf(),
        env_file: raw.env_file.map(|p| dir.join(p)),
        schedules: raw.schedules,
        jobs,
    })
}

/// Reads a job file if `source` is a path, validates its tasks, and merges the project's
/// `values` under its own.
fn load_job(source: JobSource, dir: &Path, values: &BTreeMap<String, Value>) -> Result<Job, Error> {
    let (mut job, dir) = match source {
        JobSource::Path(path) => {
            let path = dir.join(path);
            let job = read_yaml(&path)?;
            (job, path.parent().unwrap_or(Path::new("")).to_path_buf())
        }
        JobSource::Inline(job) => (job, dir.to_path_buf()),
    };
    if job.tasks.is_empty() {
        return Err(Error::InvalidProject("job has no tasks".to_string()));
    }
    check_names(
        "values",
        job.values.keys(),
        var_name::is_valid_var,
        var_name::VAR_RULE,
    )?;
    check_names(
        "states",
        job.states.keys(),
        var_name::is_valid,
        var_name::RULE,
    )?;
    task::resolve(&mut job.tasks, &dir)?;
    state::check(&job.tasks, &job.states)?;
    job.values = values.clone().into_iter().chain(job.values).collect();
    Ok(job)
}

/// Checks that every name declared in `field` passes `is_valid`, described by `rule`.
fn check_names<'a>(
    field: &str,
    mut names: impl Iterator<Item = &'a String>,
    is_valid: fn(&str) -> bool,
    rule: &str,
) -> Result<(), Error> {
    match names.find(|name| !is_valid(name)) {
        Some(name) => Err(Error::InvalidProject(format!(
            "`{field}` name `{name}` {rule}"
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
             schedules: {a: {every: 5m}}\n\
             jobs:\n\
             \x20 a:\n    tasks:\n      e:\n        cmd: [echo]\n        cwd: work\n\
             \x20 b:\n    tasks: {e: {cmd: [echo]}}\n",
        )
        .unwrap();
        assert_eq!(project.env_file, Some(dir.path().join(".env")));
        let a = project.take("a").unwrap();
        assert!(project.schedules.contains_key("a"));
        assert_eq!(a.tasks[0].dir, dir.path());
        let b = project.take("b").unwrap();
        assert!(b.tasks[0].cmd.is_some() && !project.schedules.contains_key("b"));
    }

    #[test]
    fn loads_job_files_relative_to_themselves() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("jobs");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("w.yaml"), "tasks: {e: echo.yaml}\n").unwrap();
        std::fs::write(sub.join("echo.yaml"), "cmd: [echo]\ncwd: w\n").unwrap();
        let mut project = load_in(dir.path(), "jobs:\n  w: jobs/w.yaml\n").unwrap();
        let w = project.take("w").unwrap();
        assert_eq!(w.tasks[0].dir, sub);

        let err = load_in(dir.path(), "jobs:\n  m: nope.yaml\n").unwrap_err();
        assert!(
            matches!(&err, Error::Job { name, source } if name == "m" && matches!(**source, Error::Io { .. })),
            "{err}"
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
    fn checks_state_names() {
        let dir = tempfile::tempdir().unwrap();
        let mut project = load_in(
            dir.path(),
            "jobs:\n  w:\n    states: {N: 0, L: [a]}\n    tasks: {s: {set: {states: {N: 1}}}}\n",
        )
        .unwrap();
        assert_eq!(project.dir, dir.path());
        assert_eq!(
            project.take("w").unwrap().states["L"],
            serde_json::json!(["a"])
        );
        for (yaml, bad) in [
            (
                "jobs:\n  w:\n    states: {N: 0}\n    tasks: {r: {if: {this: X, eq: 1}, then: {s: {set: {states: {M: 1}}}}}}\n",
                "`M`",
            ),
            (
                "jobs:\n  w:\n    tasks: {s: {set: {states: {M: 1}}}}\n",
                "`M`",
            ),
            (
                "jobs:\n  w:\n    states: {a.b: 0}\n    tasks: {e: {cmd: [echo]}}\n",
                "`a.b`",
            ),
            (
                "jobs:\n  w:\n    states: {a-b: 0}\n    tasks: {e: {cmd: [echo]}}\n",
                "`a-b`",
            ),
        ] {
            let err = load_in(dir.path(), yaml).unwrap_err();
            assert!(
                matches!(&err, Error::Job { name, source } if name == "w" && source.to_string().contains(bad)),
                "{yaml}: {err}"
            );
        }
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
            "jobs:\n  w: {states: {N: 0}, tasks: {s: {set: {states: {N: 1}, vars: {A: 1}}}}}\n",
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
                format!("schedules: {{w: {{every: 1s}}, w: {{every: 2s}}}}\njobs: {{{w}}}\n"),
                "schedules: duplicate key `w`",
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
                "jobs:\n  w: {states: {N: 0, N: 1}, tasks: {e: {cmd: [echo]}}}\n"
                    .to_string(),
                "jobs.w.states: duplicate key `N`",
            ),
            (
                "jobs:\n  w: {tasks: {s: {set: {states: {A: 1, A: 2}}}}}\n".to_string(),
                "jobs.w.tasks.s.set.states: duplicate key `A`",
            ),
            (
                "jobs:\n  w: {tasks: {g: {http: {method: GET, url: x, headers: {A: '1', A: '2'}}}}}\n"
                    .to_string(),
                "jobs.w.tasks.g.http.headers: duplicate key `A`",
            ),
            (
                "jobs:\n  w: {tasks: {g: {http: {method: GET, url: x, query: {A: '1', A: '2'}}}}}\n"
                    .to_string(),
                "jobs.w.tasks.g.http.query: duplicate key `A`",
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
            "jobs:\n  w: {state: {N: 0}, tasks: {e: {cmd: [echo]}}}\n",
            "jobs:\n  w: {states: {N: 0}, tasks: {s: {set_state: {N: 1}}}}\n",
            "jobs:\n  w: {schedule: {every: 1s}, tasks: {e: {cmd: [echo]}}}\n",
        ] {
            assert!(
                matches!(load_in(dir.path(), yaml), Err(Error::Yaml { .. })),
                "{yaml}"
            );
        }
        let err = load_in(
            dir.path(),
            "schedules: {x: {every: 1s}}\njobs: {w: {tasks: {e: {cmd: [echo]}}}}\n",
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid project: schedule for unknown job `x`"
        );
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
}
