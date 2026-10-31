use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::condition::Condition;
use crate::error::Error;
use crate::{project, schedule, var_name, vars};

/// One entry of a job's `tasks`: a cmd task, a branch, a switch, a loop, or a trigger.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    /// Its key in the enclosing `tasks`, `then`, `else`, `do` or `default` map.
    #[serde(skip)]
    pub name: String,
    /// A task YAML file, relative to the file that declares this task, used in its place:
    /// the entry's value when it is a string, or its `file`.
    #[serde(rename = "file")]
    pub r#use: Option<PathBuf>,
    /// Next to `file`: values for `${NAME}` while that task runs, winning over the job's and
    /// the project's.
    #[serde(default, deserialize_with = "project::unique")]
    pub values: BTreeMap<String, Value>,
    /// Program and arguments, run directly without a shell; `${VAR}` is substituted per element.
    pub cmd: Option<Vec<String>>,
    /// Further programs, each with its arguments, run like `cmd`: `cmd`'s stdout is piped
    /// into the first one's stdin, and so on.
    #[serde(default)]
    pub pipe: Vec<Vec<String>>,
    /// Seconds before the task's command is killed and the task fails: a number, or a
    /// duration such as `30s` or `5m`.
    #[serde(default, deserialize_with = "seconds")]
    pub timeout: Option<u64>,
    /// Directory a `cmd` task runs in, relative to the file that declares this task;
    /// default: the current directory. `${VAR}` is substituted.
    pub cwd: Option<PathBuf>,
    /// Directory of the file that declares this task; `stdout_file` is relative to it.
    #[serde(skip)]
    pub dir: PathBuf,
    /// Directory `cwd` is relative to: `dir`, or that of the file whose `defaults` set `cwd`.
    #[serde(skip)]
    pub cwd_dir: PathBuf,
    /// A `cmd` command's standard input: a string as-is, anything else as JSON.
    /// `${VAR}` is substituted.
    pub stdin: Option<Value>,
    /// Environment variables added to a `cmd` command's inherited environment: a string
    /// as-is, anything else as JSON. `${VAR}` is substituted.
    #[serde(default, deserialize_with = "project::unique")]
    pub env: BTreeMap<String, Value>,
    /// `false` sends a `cmd` command's stdout to /dev/null instead of keeping it in memory;
    /// the result's `stdout` is then `null`. Default `true`.
    pub capture_stdout: Option<bool>,
    /// A file, relative to the file that declares this task, that a `cmd` command's stdout is
    /// written to instead of being kept; the result's `stdout` is then `null`. `${VAR}` is
    /// substituted.
    pub stdout_file: Option<PathBuf>,
    /// Runs a `cmd` task again until it succeeds or its `until` condition holds.
    pub retry: Option<Retry>,
    /// A non-zero exit or a timeout of this `cmd` task doesn't stop the job.
    #[serde(default)]
    pub allow_failure: bool,
    /// On a loop, what a failing `do` task does: stop the job (the default), or skip the rest of
    /// that iteration.
    pub on_failure: Option<OnFailure>,
    /// Makes this a branch: runs `then` if the condition holds, else `else`.
    #[serde(rename = "if")]
    pub condition: Option<Condition>,
    #[serde(default, deserialize_with = "named")]
    pub then: Vec<Task>,
    #[serde(default, rename = "else", deserialize_with = "named")]
    pub otherwise: Vec<Task>,
    /// Makes this a loop: runs `do` once per element of this list variable.
    #[serde(default, deserialize_with = "vars::var_path")]
    pub for_each: Option<String>,
    /// The variable each element is bound to inside `do` (default `ITEM`).
    #[serde(rename = "as")]
    pub item_var: Option<String>,
    /// Makes this a `while` loop: runs `do` again and again while the condition holds.
    #[serde(rename = "while")]
    pub repeat: Option<Condition>,
    /// The most iterations a `while` loop may run before it fails.
    pub attempts: Option<u64>,
    /// The old name of `attempts`, read only to point there.
    #[serde(rename = "max")]
    pub old_max: Option<Value>,
    #[serde(default, rename = "do", deserialize_with = "named")]
    pub body: Vec<Task>,
    /// Makes this a switch: runs the first case whose key equals this variable, else `default`.
    #[serde(default, deserialize_with = "vars::var_path")]
    pub switch: Option<String>,
    #[serde(default, deserialize_with = "cases")]
    pub cases: Vec<Case>,
    #[serde(default, deserialize_with = "named")]
    pub default: Vec<Task>,
    /// Makes this a trigger: queues a run of this job.
    pub trigger: Option<String>,
}

/// A loop's `on_failure`.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum OnFailure {
    /// Stop the job.
    Stop,
    /// Skip the rest of the iteration and go on with the next one.
    Next,
}

/// A `cmd` task's `retry`: run it up to `attempts` times, `delay` seconds apart, until it
/// succeeds or, with `until`, until that condition holds.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Retry {
    /// The most runs, the first one included.
    pub attempts: u64,
    #[serde(default, deserialize_with = "seconds")]
    pub delay: Option<u64>,
    pub until: Option<Condition>,
}

impl Retry {
    /// Checks `attempts` and the `until` condition; the error is a message for the caller's
    /// error type.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.attempts == 0 {
            return Err("`retry` needs `attempts` of at least 1".to_string());
        }
        match &self.until {
            Some(until) => until.validate().map_err(|err| err.to_string()),
            None => Ok(()),
        }
    }
}

/// One `cases` entry of a switch: tasks to run when the variable equals its key.
#[derive(Clone, Debug)]
pub struct Case {
    pub value: Value,
    pub body: Vec<Task>,
}

/// Reads whole seconds, written as a number or as a duration such as `30s`, `5m` or `2h`.
pub(crate) fn seconds<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u64>, D::Error> {
    struct SecondsVisitor;

    impl Visitor<'_> for SecondsVisitor {
        type Value = u64;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("seconds, or a duration like `30s`, `5m` or `2h`")
        }

        fn visit_u64<E: de::Error>(self, secs: u64) -> Result<u64, E> {
            Ok(secs)
        }

        fn visit_str<E: de::Error>(self, text: &str) -> Result<u64, E> {
            schedule::parse_duration(text)
                .map(|duration| duration.as_secs())
                .map_err(E::custom)
        }
    }

    deserializer.deserialize_any(SecondsVisitor).map(Some)
}

/// Reads a switch's `cases`: a map of value to the tasks to run, keeping its order.
fn cases<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Case>, D::Error> {
    struct CasesVisitor;

    /// A case's tasks, read like any other map of named tasks.
    struct Body(Vec<Task>);

    impl<'de> Deserialize<'de> for Body {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            named(deserializer).map(Body)
        }
    }

    impl<'de> Visitor<'de> for CasesVisitor {
        type Value = Vec<Case>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a map of value to a map of tasks")
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Vec<Case>, A::Error> {
            let mut cases = Vec::new();
            while let Some(value) = map.next_key::<Value>()? {
                if cases.iter().any(|case: &Case| case.value == value) {
                    return Err(de::Error::custom(format!("duplicate case `{value}`")));
                }
                let Body(body) = map.next_value()?;
                cases.push(Case { value, body });
            }
            Ok(cases)
        }
    }

    deserializer.deserialize_map(CasesVisitor)
}

/// Reads a map of task name to task, or to the path of a task file, keeping its order.
pub fn named<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Task>, D::Error> {
    struct NamedVisitor;

    impl<'de> Visitor<'de> for NamedVisitor {
        type Value = Vec<Task>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a map of task name to a task or a task file path")
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Vec<Task>, A::Error> {
            let mut tasks: Vec<Task> = Vec::new();
            while let Some(name) = map.next_key::<String>()? {
                if tasks.iter().any(|task| task.name == name) {
                    return Err(de::Error::custom(format!("duplicate task `{name}`")));
                }
                let Source(mut task) = map.next_value()?;
                task.name = name;
                tasks.push(task);
            }
            Ok(tasks)
        }
    }

    deserializer.deserialize_map(NamedVisitor)
}

/// Parses `yaml`, a map of task name to task, without resolving task files.
#[cfg(test)]
pub(crate) fn parse_named(yaml: &str) -> Result<Vec<Task>, serde_yaml::Error> {
    named(serde_yaml::Deserializer::from_str(yaml))
}

/// An entry's value: a task file path, or the task's fields inline.
struct Source(Task);

/// Picks the variant by YAML shape, so a mistake in an inline task reports the real cause
/// and line instead of a generic untagged-enum error.
impl<'de> Deserialize<'de> for Source {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct SourceVisitor;

        impl<'de> Visitor<'de> for SourceVisitor {
            type Value = Source;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a task file path or a task")
            }

            fn visit_str<E: de::Error>(self, path: &str) -> Result<Source, E> {
                Ok(Source(Task {
                    r#use: Some(PathBuf::from(path)),
                    ..Task::default()
                }))
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Source, A::Error> {
                Task::deserialize(de::value::MapAccessDeserializer::new(map)).map(Source)
            }
        }

        deserializer.deserialize_any(SourceVisitor)
    }
}

impl Task {
    fn has_switch_fields(&self) -> bool {
        self.switch.is_some() || !self.cases.is_empty() || !self.default.is_empty()
    }

    /// Whether any field other than `values` is set, on an entry that has `file`.
    fn has_fields_besides_values(&self) -> bool {
        self.cmd.is_some()
            || !self.pipe.is_empty()
            || self.timeout.is_some()
            || self.cwd.is_some()
            || self.stdin.is_some()
            || !self.env.is_empty()
            || self.capture_stdout.is_some()
            || self.stdout_file.is_some()
            || self.retry.is_some()
            || self.allow_failure
            || self.on_failure.is_some()
            || self.condition.is_some()
            || !self.then.is_empty()
            || !self.otherwise.is_empty()
            || self.for_each.is_some()
            || self.item_var.is_some()
            || self.repeat.is_some()
            || self.attempts.is_some()
            || self.old_max.is_some()
            || !self.body.is_empty()
            || self.has_switch_fields()
            || self.trigger.is_some()
    }

    fn has_task_fields(&self) -> bool {
        self.cmd.is_some() || self.timeout.is_some() || self.allow_failure
    }
}

/// Validates tasks (recursively through branches and loops) and resolves their paths against `dir`.
pub fn resolve(tasks: &mut [Task], dir: &Path) -> Result<(), Error> {
    resolve_all(tasks, dir, &mut Vec::new())
}

/// [`resolve`], with `stack` holding the task files being loaded, to detect task file cycles.
fn resolve_all(tasks: &mut [Task], dir: &Path, stack: &mut Vec<PathBuf>) -> Result<(), Error> {
    for task in tasks {
        resolve_one(task, dir, stack)?;
    }
    Ok(())
}

fn resolve_one(task: &mut Task, dir: &Path, stack: &mut Vec<PathBuf>) -> Result<(), Error> {
    let name = task.name.clone();
    if !var_name::is_valid(&name) {
        return Err(Error::InvalidTask(format!(
            "task name `{name}` {}",
            var_name::RULE
        )));
    }
    if let Some(path) = task.r#use.take() {
        if task.has_fields_besides_values() {
            return Err(Error::InvalidTask(format!(
                "`{name}` has `file`, so its other fields go in that file; only `values` can be next to it"
            )));
        }
        if let Some(var) = task.values.keys().find(|var| !var_name::is_valid_var(var)) {
            return Err(Error::InvalidTask(format!(
                "`{name}`: `values` name `{var}` {}",
                var_name::VAR_RULE
            )));
        }
        let values = std::mem::take(&mut task.values);
        resolve_use(task, path, dir, stack)?;
        // Over those of a `file` entry the loaded file may itself be.
        task.values.extend(values);
        return Ok(());
    }
    if !task.values.is_empty() {
        return Err(Error::InvalidTask(format!(
            "`{name}` has `values`, which only works next to `file`"
        )));
    }
    for (field, set) in [
        ("cwd", task.cwd.is_some()),
        ("stdin", task.stdin.is_some()),
        ("env", !task.env.is_empty()),
        ("capture_stdout", task.capture_stdout.is_some()),
        ("pipe", !task.pipe.is_empty()),
        ("retry", task.retry.is_some()),
        ("stdout_file", task.stdout_file.is_some()),
    ] {
        if set && task.cmd.is_none() {
            return Err(Error::InvalidTask(format!(
                "`{name}` has `{field}`, which only works with `cmd`"
            )));
        }
    }
    task.dir = dir.to_path_buf();
    task.cwd_dir = dir.to_path_buf();
    if task.old_max.is_some() {
        return Err(Error::InvalidTask(format!(
            "`{name}`: `max` is now `attempts`"
        )));
    }
    if task.trigger.is_some() {
        return resolve_trigger(task, &name);
    }
    if task.for_each.is_some() {
        return resolve_loop(task, &name, dir, stack);
    }
    if task.item_var.is_some() {
        return Err(Error::InvalidTask(format!(
            "`{name}` has `as` but no `for_each`"
        )));
    }
    if task.repeat.is_some() {
        return resolve_while(task, &name, dir, stack);
    }
    if !task.body.is_empty() || task.attempts.is_some() {
        return Err(Error::InvalidTask(format!(
            "`{name}` has `do`/`attempts` but no `for_each` or `while`"
        )));
    }
    if task.on_failure.is_some() {
        return Err(Error::InvalidTask(format!(
            "`{name}` has `on_failure`, which only works on loops (`for_each`, `while`)"
        )));
    }
    if task.switch.is_some() {
        return resolve_switch(task, &name, dir, stack);
    }
    if !task.cases.is_empty() || !task.default.is_empty() {
        return Err(Error::InvalidTask(format!(
            "`{name}` has `cases`/`default` but no `switch`"
        )));
    }
    if task.condition.is_some() {
        return resolve_branch(task, &name, dir, stack);
    }
    if !task.then.is_empty() || !task.otherwise.is_empty() {
        return Err(Error::InvalidTask(format!(
            "`{name}` has `then`/`else` but no `if`"
        )));
    }
    resolve_leaf(task, &name)
}

/// Replaces `task` with the task file at `path`, named by `task`'s key.
fn resolve_use(
    task: &mut Task,
    path: PathBuf,
    dir: &Path,
    stack: &mut Vec<PathBuf>,
) -> Result<(), Error> {
    let path = dir.join(path);
    let canonical = path.canonicalize().map_err(|source| Error::Io {
        path: path.clone(),
        source,
    })?;
    if stack.contains(&canonical) {
        return Err(Error::InvalidTask(format!(
            "`{}`: task file cycle through {}",
            task.name,
            path.display()
        )));
    }
    let mut loaded: Task = project::read_yaml(&path)?;
    loaded.name = std::mem::take(&mut task.name);
    let file_dir = path.parent().unwrap_or(Path::new(""));
    stack.push(canonical);
    resolve_one(&mut loaded, file_dir, stack)?;
    stack.pop();
    *task = loaded;
    Ok(())
}

fn resolve_loop(
    task: &mut Task,
    name: &str,
    dir: &Path,
    stack: &mut Vec<PathBuf>,
) -> Result<(), Error> {
    if task.allow_failure {
        return Err(Error::InvalidTask(format!(
            "`{name}` is a loop (`for_each`): `allow_failure` only works on cmd tasks; use `on_failure: next` to skip to the next iteration when a `do` task fails"
        )));
    }
    if task.cmd.is_some()
        || task.timeout.is_some()
        || task.condition.is_some()
        || !task.then.is_empty()
        || !task.otherwise.is_empty()
        || task.has_switch_fields()
        || task.repeat.is_some()
        || task.attempts.is_some()
    {
        return Err(Error::InvalidTask(format!(
            "`{name}` is a loop (`for_each`) and cannot have `cmd`, `timeout`, `if`, `then`, `else`, `switch`, `cases`, `default`, `while` or `attempts`"
        )));
    }
    if task.body.is_empty() {
        return Err(Error::InvalidTask(format!(
            "`{name}` needs a non-empty `do`"
        )));
    }
    if let Some(var) = &task.item_var
        && !var_name::is_valid_var(var)
    {
        return Err(Error::InvalidTask(format!(
            "`{name}`: `as` name `{var}` {}",
            var_name::VAR_RULE
        )));
    }
    resolve_all(&mut task.body, dir, stack)
}

fn resolve_while(
    task: &mut Task,
    name: &str,
    dir: &Path,
    stack: &mut Vec<PathBuf>,
) -> Result<(), Error> {
    if task.allow_failure {
        return Err(Error::InvalidTask(format!(
            "`{name}` is a loop (`while`): `allow_failure` only works on cmd tasks; use `on_failure: next` to skip to the next iteration when a `do` task fails"
        )));
    }
    if task.cmd.is_some()
        || task.timeout.is_some()
        || task.condition.is_some()
        || !task.then.is_empty()
        || !task.otherwise.is_empty()
        || task.has_switch_fields()
    {
        return Err(Error::InvalidTask(format!(
            "`{name}` is a loop (`while`) and cannot have `cmd`, `timeout`, `if`, `then`, `else`, `switch`, `cases` or `default`"
        )));
    }
    if task.body.is_empty() {
        return Err(Error::InvalidTask(format!(
            "`{name}` needs a non-empty `do`"
        )));
    }
    if task.attempts == Some(0) {
        return Err(Error::InvalidTask(format!(
            "`{name}`: `attempts` must be at least 1"
        )));
    }
    if let Some(condition) = &task.repeat {
        condition
            .validate()
            .map_err(|source| Error::InvalidTask(format!("`{name}`: {source}")))?;
    }
    resolve_all(&mut task.body, dir, stack)
}

fn resolve_branch(
    task: &mut Task,
    name: &str,
    dir: &Path,
    stack: &mut Vec<PathBuf>,
) -> Result<(), Error> {
    if task.has_task_fields() {
        return Err(Error::InvalidTask(format!(
            "`{name}` is a branch (`if`) and cannot have `cmd`, `timeout` or `allow_failure`"
        )));
    }
    if task.then.is_empty() && task.otherwise.is_empty() {
        return Err(Error::InvalidTask(format!(
            "`{name}` needs a non-empty `then` or `else`"
        )));
    }
    if let Some(condition) = &task.condition {
        condition
            .validate()
            .map_err(|source| Error::InvalidTask(format!("`{name}`: {source}")))?;
    }
    resolve_all(&mut task.then, dir, stack)?;
    resolve_all(&mut task.otherwise, dir, stack)
}

fn resolve_switch(
    task: &mut Task,
    name: &str,
    dir: &Path,
    stack: &mut Vec<PathBuf>,
) -> Result<(), Error> {
    if task.has_task_fields()
        || task.condition.is_some()
        || !task.then.is_empty()
        || !task.otherwise.is_empty()
    {
        return Err(Error::InvalidTask(format!(
            "`{name}` is a switch (`switch`) and cannot have `cmd`, `timeout`, `allow_failure`, `if`, `then` or `else`"
        )));
    }
    if task.cases.is_empty() {
        return Err(Error::InvalidTask(format!(
            "`{name}` needs a non-empty `cases`"
        )));
    }
    for case in &mut task.cases {
        if case.body.is_empty() {
            return Err(Error::InvalidTask(format!(
                "`{name}` case {} needs at least one task",
                case.value
            )));
        }
        resolve_all(&mut case.body, dir, stack)?;
    }
    resolve_all(&mut task.default, dir, stack)
}

/// Checks a trigger, which has no other fields. Whether its job exists is checked once the
/// whole project is loaded.
fn resolve_trigger(task: &mut Task, name: &str) -> Result<(), Error> {
    let job = task.trigger.take();
    let has_others = task.has_fields_besides_values();
    task.trigger = job;
    if has_others {
        return Err(Error::InvalidTask(format!(
            "`{name}` is a trigger (`trigger`) and cannot have other fields"
        )));
    }
    Ok(())
}

/// Checks a `cmd` task.
fn resolve_leaf(task: &Task, name: &str) -> Result<(), Error> {
    match &task.cmd {
        Some(argv) if argv.is_empty() => {
            return Err(Error::InvalidTask(format!("`{name}` has an empty `cmd`")));
        }
        Some(_) if task.pipe.iter().any(Vec::is_empty) => {
            return Err(Error::InvalidTask(format!(
                "`{name}` has an empty stage in `pipe`"
            )));
        }
        Some(_) => {}
        None => {
            return Err(Error::InvalidTask(format!("`{name}` must have `cmd`")));
        }
    }
    if task.stdout_file.is_some() && task.capture_stdout.is_some() {
        return Err(Error::InvalidTask(format!(
            "`{name}` has both `stdout_file` and `capture_stdout`"
        )));
    }
    if let Some(retry) = &task.retry {
        retry
            .validate()
            .map_err(|msg| Error::InvalidTask(format!("`{name}`: {msg}")))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve_in(dir: &Path, yaml: &str) -> Result<Vec<Task>, Error> {
        let mut tasks = parse_named(yaml).unwrap();
        resolve(&mut tasks, dir)?;
        Ok(tasks)
    }

    /// The tasks in `yaml`, resolved against the current directory.
    fn resolve_here(yaml: &str) -> Result<Vec<Task>, Error> {
        resolve_in(Path::new(""), yaml)
    }

    /// The message of the invalid-task error that resolving `yaml` must give.
    fn invalid_task(yaml: &str) -> String {
        match resolve_here(yaml) {
            Err(Error::InvalidTask(msg)) => msg,
            Err(err) => panic!("{yaml}: expected an invalid task, got {err}"),
            Ok(_) => panic!("{yaml}: expected an invalid task, got none"),
        }
    }

    #[test]
    fn resolves_paths_against_project_dir() {
        let dir = Path::new("/project");
        let tasks = resolve_in(
            dir,
            "login:\n  cmd: [echo]\n\
             base:\n  cmd: [echo, '${BASE}']\n  allow_failure: true\n\
             echo:\n  cmd: [echo, '${BASE}']\n  cwd: work\n",
        )
        .unwrap();
        let login = &tasks[0];
        assert!(!login.allow_failure);
        assert_eq!(tasks[1].name, "base");
        assert!(tasks[1].allow_failure);
        assert_eq!(tasks[2].cwd, Some("work".into()));
        assert_eq!(tasks[2].dir, dir);
    }

    #[test]
    fn keeps_map_order_and_rejects_lists_and_duplicates() {
        let tasks = parse_named("z: {cmd: [echo]}\na: {cmd: [echo]}\nm: {cmd: [echo]}\n").unwrap();
        let names: Vec<_> = tasks.iter().map(|task| task.name.as_str()).collect();
        assert_eq!(names, ["z", "a", "m"]);
        let err = parse_named("- cmd: [echo]\n").unwrap_err();
        assert!(err.to_string().contains("a map of task name"), "{err}");
        let err = parse_named("a: {cmd: [echo]}\na: {cmd: [echo]}\n").unwrap_err();
        assert!(err.to_string().contains("duplicate"), "{err}");
        let err = parse_named("a: {name: b, cmd: [echo]}\n").unwrap_err();
        assert!(err.to_string().contains("unknown field `name`"), "{err}");
    }

    #[test]
    fn task_needs_a_non_empty_cmd() {
        for (yaml, expected) in [
            ("a: {}\n", "`a` must have `cmd`"),
            ("a:\n  cmd: []\n", "`a` has an empty `cmd`"),
            (
                "a:\n  cmd: [echo]\n  stdout_file: out.txt\n  capture_stdout: false\n",
                "`a` has both `stdout_file` and `capture_stdout`",
            ),
            (
                "a:\n  cmd: [echo]\n  pipe: [[cat], []]\n",
                "`a` has an empty stage in `pipe`",
            ),
        ] {
            let msg = invalid_task(yaml);
            assert!(msg.contains(expected), "{yaml}: {msg}");
        }
    }

    #[test]
    fn cmd_options_need_cmd() {
        for (yaml, field) in [
            ("a:\n  cwd: w\n", "cwd"),
            ("a:\n  stdin: y\n", "stdin"),
            ("a:\n  env: {A: y}\n", "env"),
            ("a:\n  capture_stdout: false\n", "capture_stdout"),
            ("a:\n  pipe: [[cat]]\n", "pipe"),
            ("a:\n  retry: {attempts: 2}\n", "retry"),
            ("a:\n  stdout_file: out.txt\n", "stdout_file"),
        ] {
            let msg = invalid_task(yaml);
            assert!(
                msg.contains(&format!("`a` has `{field}`, which only works with `cmd`")),
                "{yaml}: {msg}"
            );
        }
    }

    #[test]
    fn timeout_works_on_cmd_tasks_but_not_branches_or_loops() {
        resolve_here("s:\n  cmd: [echo]\n  timeout: 5\n").unwrap();
        for yaml in [
            "a:\n  if: {this: X, eq: 1}\n  then: {b: {cmd: [echo]}}\n  timeout: 1\n",
            "a:\n  for_each: X\n  do: {b: {cmd: [echo]}}\n  timeout: 1\n",
        ] {
            let msg = invalid_task(yaml);
            assert!(
                msg.contains("`a` is a") && msg.contains("`timeout`"),
                "{yaml}: {msg}"
            );
        }
    }

    #[test]
    fn timeout_takes_seconds_or_a_duration() {
        for (timeout, secs) in [("5", 5), ("2s", 2), ("2m", 120), ("1h", 3600)] {
            let tasks =
                resolve_here(&format!("s:\n  cmd: [echo]\n  timeout: {timeout}\n")).unwrap();
            assert_eq!(tasks[0].timeout, Some(secs), "{timeout}");
        }
        for timeout in ["5 min", "-1", "1.5"] {
            let err =
                parse_named(&format!("s:\n  cmd: [echo]\n  timeout: {timeout}\n")).unwrap_err();
            assert!(err.to_string().contains("timeout"), "{timeout}: {err}");
        }
    }

    #[test]
    fn retry_needs_attempts_and_a_valid_condition() {
        let tasks = resolve_here(
            "a:\n  cmd: [echo]\n  retry: {attempts: 3, delay: 2s, until: {this: '${tasks.a.stdout}', eq: ok}}\n",
        )
        .unwrap();
        let retry = tasks[0].retry.as_ref().unwrap();
        assert_eq!((retry.attempts, retry.delay), (3, Some(2)));
        for (yaml, expected) in [
            (
                "a:\n  cmd: [echo]\n  retry: {attempts: 0}\n",
                "`a`: `retry` needs `attempts` of at least 1",
            ),
            (
                "a:\n  cmd: [echo]\n  retry: {attempts: 2, until: {this: X, gt: ten}}\n",
                "`a`: invalid condition: `gt` needs a number",
            ),
        ] {
            let msg = invalid_task(yaml);
            assert!(msg.contains(expected), "{yaml}: {msg}");
        }
        for (yaml, expected) in [
            (
                "a:\n  cmd: [echo]\n  retry: {delay: 1s}\n",
                "missing field `attempts`",
            ),
            (
                "a:\n  cmd: [echo]\n  retry: {attempts: 2, max: 3}\n",
                "unknown field `max`",
            ),
            (
                "a:\n  cmd: [echo]\n  retry: {attempts: 2, delay: soon}\n",
                "is not a duration",
            ),
        ] {
            let err = parse_named(yaml).unwrap_err();
            assert!(err.to_string().contains(expected), "{yaml}: {err}");
        }
    }

    #[test]
    fn rejects_invalid_loop_names() {
        for (yaml, field, var) in [
            (
                "a:\n  for_each: X\n  as: a.b\n  do: {b: {cmd: [echo]}}\n",
                "as",
                "a.b",
            ),
            (
                "a:\n  for_each: X\n  as: ''\n  do: {b: {cmd: [echo]}}\n",
                "as",
                "",
            ),
            (
                "a:\n  for_each: X\n  as: item\n  do: {b: {cmd: [echo]}}\n",
                "as",
                "item",
            ),
        ] {
            assert_eq!(
                invalid_task(yaml),
                format!("`a`: `{field}` name `{var}` {}", var_name::VAR_RULE),
                "{yaml}"
            );
        }
    }

    #[test]
    fn rejects_invalid_task_name() {
        for (yaml, name) in [
            ("a.b:\n  cmd: [echo]\n", "a.b"),
            ("a.b: x.yaml\n", "a.b"),
            ("a-b:\n  cmd: [echo]\n", "a-b"),
            ("1a:\n  cmd: [echo]\n", "1a"),
        ] {
            assert_eq!(
                invalid_task(yaml),
                format!("task name `{name}` {}", var_name::RULE),
                "{yaml}"
            );
        }
    }

    #[test]
    fn resolves_paths_inside_nested_branches_and_loops() {
        let dir = Path::new("/project");
        let tasks = resolve_in(
            dir,
            "r:\n  if: {this: X, gt: 1}\n  then:\n    a:\n      cmd: [a]\n      cwd: a\n  else:\n\
             \x20   l:\n      for_each: IDS\n      do:\n        b:\n          cmd: [b]\n          cwd: b\n",
        )
        .unwrap();
        assert_eq!(tasks[0].then[0].dir, dir);
        assert_eq!(tasks[0].otherwise[0].body[0].dir, dir);
    }

    #[test]
    fn rejects_mixed_branch_and_task_fields() {
        let branch = "`r` is a branch (`if`) and cannot have";
        for (yaml, expected) in [
            (
                "r:\n  if: {this: X, gt: 1}\n  then: {a: {cmd: [echo]}}\n  cmd: [echo]\n",
                branch,
            ),
            (
                "r:\n  if: {this: X, gt: 1}\n",
                "`r` needs a non-empty `then` or `else`",
            ),
            (
                "r:\n  cmd: [echo]\n  then: {b: {cmd: [echo]}}\n",
                "`r` has `then`/`else` but no `if`",
            ),
        ] {
            let msg = invalid_task(yaml);
            assert!(msg.contains(expected), "{yaml}: {msg}");
        }
    }

    #[test]
    fn rejects_invalid_loops() {
        let body = "  do: {a: {cmd: [echo]}}\n";
        let a_loop = "`l` is a loop (`for_each`) and cannot have";
        let no_loop = "`l` has `do`/`attempts` but no `for_each` or `while`";
        for (yaml, expected) in [
            (format!("l:\n  for_each: X\n{body}  cmd: [echo]\n"), a_loop),
            (format!("l:\n  for_each: X\n{body}  timeout: 1\n"), a_loop),
            (
                format!("l:\n  for_each: X\n{body}  if: {{this: X, eq: 1}}\n"),
                a_loop,
            ),
            (
                "l:\n  for_each: X\n  do: {}\n".to_string(),
                "`l` needs a non-empty `do`",
            ),
            (format!("l:\n  cmd: [echo]\n{body}"), no_loop),
            (
                "l:\n  cmd: [echo]\n  as: X\n".to_string(),
                "`l` has `as` but no `for_each`",
            ),
            ("l:\n  cmd: [echo]\n  attempts: 3\n".to_string(), no_loop),
            (format!("l:\n  for_each: X\n{body}  attempts: 3\n"), a_loop),
            (
                format!("l:\n  for_each: X\n{body}  while: {{this: X, eq: 1}}\n"),
                a_loop,
            ),
            (
                "each:\n  for_each: X\n  do:\n    l:\n      cmd: []\n".to_string(),
                "`l` has an empty `cmd`",
            ),
        ] {
            let msg = invalid_task(&yaml);
            assert!(msg.contains(expected), "{yaml}: {msg}");
        }
    }

    #[test]
    fn rejects_invalid_while_loops() {
        let body = "  do: {a: {cmd: [echo]}}\n";
        let cond = "  while: {this: X, eq: 1}\n";
        let a_while = "`w` is a loop (`while`) and cannot have";
        for (yaml, expected) in [
            (format!("w:\n{cond}{body}  cmd: [echo]\n"), a_while),
            (format!("w:\n{cond}{body}  timeout: 1\n"), a_while),
            (
                format!("w:\n{cond}{body}  if: {{this: X, eq: 1}}\n"),
                a_while,
            ),
            (format!("w:\n{cond}{body}  switch: X\n"), a_while),
            (
                format!("w:\n{cond}{body}  as: Y\n"),
                "`w` has `as` but no `for_each`",
            ),
            (format!("w:\n{cond}"), "`w` needs a non-empty `do`"),
            (
                format!("w:\n{cond}{body}  attempts: 0\n"),
                "`w`: `attempts` must be at least 1",
            ),
            (
                format!("w:\n{cond}{body}  max: 3\n"),
                "`w`: `max` is now `attempts`",
            ),
            (
                format!("w:\n  while: {{this: X, gt: ten}}\n{body}"),
                "`w`: invalid condition: `gt` needs a number",
            ),
            (
                format!("w:\n{cond}  do:\n    l:\n      cmd: []\n"),
                "`l` has an empty `cmd`",
            ),
        ] {
            let msg = invalid_task(&yaml);
            assert!(msg.contains(expected), "{yaml}: {msg}");
        }
    }

    #[test]
    fn resolves_paths_inside_while_loops() {
        let dir = Path::new("/project");
        let tasks = resolve_in(
            dir,
            "w:\n  while: {this: X, eq: 1}\n  attempts: 3\n  do: {a: {cmd: [a], cwd: a}}\n",
        )
        .unwrap();
        assert_eq!(tasks[0].attempts, Some(3));
        assert_eq!(tasks[0].body[0].dir, dir);
    }

    #[test]
    fn loops_take_on_failure_instead_of_allow_failure() {
        let tasks = resolve_here(
            "l:\n  for_each: X\n  on_failure: next\n  do: {a: {cmd: [a]}}\n\
             w:\n  while: {this: X, eq: 1}\n  on_failure: stop\n  do: {b: {cmd: [b]}}\n",
        )
        .unwrap();
        assert_eq!(tasks[0].on_failure, Some(OnFailure::Next));
        assert_eq!(tasks[1].on_failure, Some(OnFailure::Stop));
        for yaml in [
            "l:\n  for_each: X\n  allow_failure: true\n  do: {a: {cmd: [a]}}\n",
            "w:\n  while: {this: X, eq: 1}\n  allow_failure: true\n  do: {b: {cmd: [b]}}\n",
        ] {
            let msg = invalid_task(yaml);
            assert!(msg.contains("use `on_failure: next`"), "{yaml}: {msg}");
        }
        for yaml in [
            "a:\n  cmd: [a]\n  on_failure: next\n",
            "r:\n  if: {this: X, eq: 1}\n  then: {a: {cmd: [a]}}\n  on_failure: next\n",
            "s:\n  switch: X\n  cases: {1: {a: {cmd: [a]}}}\n  on_failure: next\n",
        ] {
            let msg = invalid_task(yaml);
            assert!(
                msg.contains("`on_failure`, which only works on loops"),
                "{yaml}: {msg}"
            );
        }
        let err = parse_named("l:\n  for_each: X\n  on_failure: skip\n  do: {a: {cmd: [a]}}\n")
            .unwrap_err();
        assert!(err.to_string().contains("unknown variant `skip`"), "{err}");
    }

    #[test]
    fn resolves_paths_inside_switch_cases() {
        let dir = Path::new("/project");
        let tasks = resolve_in(
            dir,
            "s:\n  switch: X\n  cases:\n    1: {a: {cmd: [a], cwd: a}}\n\
             \x20 default: {b: {cmd: [b], cwd: b}}\n",
        )
        .unwrap();
        assert_eq!(tasks[0].cases[0].body[0].dir, dir);
        assert_eq!(tasks[0].default[0].dir, dir);
    }

    #[test]
    fn rejects_invalid_switches() {
        let case = "  cases: {1: {a: {cmd: [echo]}}}\n";
        let switch = "`s` is a switch (`switch`) and cannot have";
        let no_cases = "`s` needs a non-empty `cases`";
        let no_switch = "`s` has `cases`/`default` but no `switch`";
        for (yaml, expected) in [
            (format!("s:\n  switch: X\n{case}  cmd: [echo]\n"), switch),
            (
                format!("s:\n  switch: X\n{case}  allow_failure: true\n"),
                switch,
            ),
            (
                format!("s:\n  switch: X\n{case}  if: {{this: X, eq: 1}}\n"),
                switch,
            ),
            ("s:\n  switch: X\n".to_string(), no_cases),
            (
                "s:\n  switch: X\n  default: {a: {cmd: [echo]}}\n".to_string(),
                no_cases,
            ),
            (
                "s:\n  switch: X\n  cases: {1: {}}\n".to_string(),
                "`s` case 1 needs at least one task",
            ),
            (format!("s:\n{case}"), no_switch),
            (
                "s:\n  cmd: [echo]\n  default: {b: {cmd: [echo]}}\n".to_string(),
                no_switch,
            ),
            (
                format!("s:\n  for_each: X\n  do: {{a: {{cmd: [echo]}}}}\n  switch: X\n{case}"),
                "`s` is a loop (`for_each`) and cannot have",
            ),
            (
                "w:\n  switch: X\n  cases:\n    1: {s: {cmd: []}}\n".to_string(),
                "`s` has an empty `cmd`",
            ),
        ] {
            let msg = invalid_task(&yaml);
            assert!(msg.contains(expected), "{yaml}: {msg}");
        }
        assert!(
            parse_named("s:\n  switch: X\n  cases: [{match: 1, do: {a: {cmd: [echo]}}}]\n")
                .is_err()
        );
        let err = parse_named(
            "s:\n  switch: X\n  cases: {1: {a: {cmd: [echo]}}, 1: {b: {cmd: [echo]}}}\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("duplicate case `1`"), "{err}");
    }

    #[test]
    fn triggers_take_no_other_fields() {
        let tasks = resolve_here("t:\n  trigger: report\n").unwrap();
        assert_eq!(tasks[0].trigger.as_deref(), Some("report"));
        for yaml in [
            "t:\n  trigger: report\n  timeout: 5\n",
            "t:\n  trigger: report\n  allow_failure: true\n",
            "t:\n  trigger: report\n  if: {this: X, eq: 1}\n  then: {a: {cmd: [echo]}}\n",
            "t:\n  trigger: report\n  for_each: X\n  do: {a: {cmd: [echo]}}\n",
            "t:\n  trigger: report\n  on_failure: next\n",
        ] {
            let msg = invalid_task(yaml);
            assert!(
                msg.contains("`t` is a trigger (`trigger`) and cannot have other fields"),
                "{yaml}: {msg}"
            );
        }
        let msg = invalid_task("t:\n  trigger: report\n  stdin: x\n");
        assert!(msg.contains("only works with `cmd`"), "{msg}");
        let msg =
            invalid_task("b:\n  if: {this: X, eq: 1}\n  then: {t: {trigger: r, cmd: [echo]}}\n");
        assert!(msg.contains("`t` is a trigger"), "{msg}");
    }

    #[test]
    fn task_files_load_relative_to_themselves_named_by_key() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("tasks");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("login.yaml"), "cmd: [echo]\n").unwrap();
        std::fs::write(
            sub.join("each.yaml"),
            "for_each: IDS\ndo:\n  inner: login.yaml\n  echo:\n    cmd: [echo]\n    cwd: w\n",
        )
        .unwrap();
        let tasks = resolve_in(
            dir.path(),
            "login: tasks/login.yaml\nagain: tasks/login.yaml\neach: tasks/each.yaml\n",
        )
        .unwrap();
        assert_eq!(tasks[0].name, "login");
        assert_eq!(tasks[1].name, "again");
        assert_eq!(tasks[2].name, "each");
        assert_eq!(tasks[2].body[0].name, "inner");
        assert_eq!(tasks[2].body[1].dir, sub);
    }

    #[test]
    fn task_files_reject_names_missing_files_and_cycles() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("named.yaml"), "name: a\ncmd: [echo]\n").unwrap();
        let err = resolve_in(dir.path(), "a: named.yaml\n").unwrap_err();
        assert!(
            matches!(&err, Error::Yaml { path, .. } if path.ends_with("named.yaml")),
            "{err}"
        );
        let err = resolve_in(dir.path(), "a: nope.yaml\n").unwrap_err();
        assert!(matches!(err, Error::Io { .. }), "{err}");

        std::fs::write(
            dir.path().join("a.yaml"),
            "if: {this: X, eq: 1}\nthen: {b: b.yaml}\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("b.yaml"), "for_each: X\ndo: {a: a.yaml}\n").unwrap();
        let err = resolve_in(dir.path(), "a: a.yaml\n").unwrap_err();
        assert!(
            matches!(&err, Error::InvalidTask(msg) if msg.contains("task file cycle through")),
            "{err}"
        );
    }

    #[test]
    fn task_file_entries_take_values() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("greet.yaml"), "cmd: [echo, '${WHO}']\n").unwrap();
        std::fs::write(
            dir.path().join("wrap.yaml"),
            "file: greet.yaml\nvalues: {WHO: inner, KEEP: inner}\n",
        )
        .unwrap();
        let tasks = resolve_in(
            dir.path(),
            "a: {file: greet.yaml, values: {WHO: a}}\nb: {file: greet.yaml}\n\
             c: {file: wrap.yaml, values: {WHO: outer}}\n",
        )
        .unwrap();
        let values = |task: &Task| serde_json::to_value(&task.values).unwrap();
        assert_eq!(
            tasks[0].cmd.as_deref(),
            Some(&["echo".to_string(), "${WHO}".to_string()][..])
        );
        assert_eq!(values(&tasks[0]), serde_json::json!({"WHO": "a"}));
        assert!(tasks[1].values.is_empty());
        assert_eq!(
            values(&tasks[2]),
            serde_json::json!({"WHO": "outer", "KEEP": "inner"})
        );

        for (yaml, expected) in [
            (
                "a: {file: greet.yaml, cmd: [echo]}\n",
                "`a` has `file`, so its other fields go in that file",
            ),
            (
                "a: {file: greet.yaml, timeout: 5}\n",
                "`a` has `file`, so its other fields go in that file",
            ),
            (
                "a: {cmd: [echo], values: {X: 1}}\n",
                "`a` has `values`, which only works next to `file`",
            ),
            (
                "a: {file: greet.yaml, values: {who: 1}}\n",
                "`a`: `values` name `who`",
            ),
        ] {
            match resolve_in(dir.path(), yaml) {
                Err(Error::InvalidTask(msg)) => assert!(msg.contains(expected), "{yaml}: {msg}"),
                other => panic!("{yaml}: expected an invalid task, got {other:?}"),
            }
        }
    }

    #[test]
    fn rejects_invalid_condition() {
        let msg = invalid_task("r:\n  if: {this: X, gt: ten}\n  then: {a: {cmd: [echo]}}\n");
        assert!(
            msg.starts_with("`r`: invalid condition: `gt` needs a number"),
            "{msg}"
        );
    }

    #[test]
    fn for_each_and_switch_take_a_bare_or_wrapped_path() {
        let tasks = resolve_here(
            "l:\n  for_each: ${tasks.a.stdout}\n  do: {a: {cmd: [echo]}}\n\
             s:\n  switch: ${X}\n  cases: {1: {b: {cmd: [echo]}}}\n",
        )
        .unwrap();
        assert_eq!(tasks[0].for_each.as_deref(), Some("tasks.a.stdout"));
        assert_eq!(tasks[1].switch.as_deref(), Some("X"));
        for yaml in [
            "l:\n  for_each: a${X}\n  do: {a: {cmd: [echo]}}\n",
            "s:\n  switch: ${X}${Y}\n  cases: {1: {b: {cmd: [echo]}}}\n",
        ] {
            let err = parse_named(yaml).unwrap_err();
            assert!(
                err.to_string().contains("must name one variable"),
                "{yaml}: {err}"
            );
        }
    }

    #[test]
    fn rejects_a_task_that_is_neither_a_path_nor_a_map() {
        let err = parse_named("a: 5\n").unwrap_err();
        assert!(
            err.to_string().contains("a task file path or a task"),
            "{err}"
        );
    }
}
