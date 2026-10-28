use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;

use crate::error::Error;
use crate::project;
use crate::task::{self, Retry, Task};

/// A project's or job's `defaults`: fields every `cmd` task in its jobs gets unless it sets
/// them itself. `env` is merged by name instead.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    #[serde(default, deserialize_with = "task::seconds")]
    pub timeout: Option<u64>,
    /// Relative to `dir`.
    pub cwd: Option<PathBuf>,
    /// Directory of the file these defaults are written in.
    #[serde(skip)]
    pub dir: PathBuf,
    #[serde(default, deserialize_with = "project::unique")]
    pub env: BTreeMap<String, Value>,
    pub retry: Option<Retry>,
}

impl Defaults {
    pub fn validate(&self) -> Result<(), Error> {
        match &self.retry {
            Some(retry) => retry
                .validate()
                .map_err(|msg| Error::InvalidProject(format!("`defaults`: {msg}"))),
            None => Ok(()),
        }
    }

    /// These defaults, with `outer`'s filling the fields they don't set.
    pub fn over(mut self, outer: &Defaults) -> Defaults {
        self.timeout = self.timeout.or(outer.timeout);
        if self.cwd.is_none() {
            self.cwd.clone_from(&outer.cwd);
            self.dir.clone_from(&outer.dir);
        }
        merge_env(&mut self.env, &outer.env);
        if self.retry.is_none() {
            self.retry.clone_from(&outer.retry);
        }
        self
    }

    /// Fills in the `cmd` tasks in `tasks`, including those nested in branches, switches and
    /// loops.
    pub fn apply(&self, tasks: &mut [Task]) {
        for task in tasks {
            if task.cmd.is_some() {
                task.timeout = task.timeout.or(self.timeout);
                if task.cwd.is_none()
                    && let Some(cwd) = &self.cwd
                {
                    task.cwd = Some(cwd.clone());
                    task.cwd_dir.clone_from(&self.dir);
                }
                merge_env(&mut task.env, &self.env);
                if task.retry.is_none() {
                    task.retry.clone_from(&self.retry);
                }
            }
            self.apply(&mut task.then);
            self.apply(&mut task.otherwise);
            self.apply(&mut task.body);
            self.apply(&mut task.default);
            for case in &mut task.cases {
                self.apply(&mut case.body);
            }
        }
    }
}

/// Adds the entries of `outer` whose names `env` doesn't have.
fn merge_env(env: &mut BTreeMap<String, Value>, outer: &BTreeMap<String, Value>) {
    for (name, value) in outer {
        env.entry(name.clone()).or_insert_with(|| value.clone());
    }
}
