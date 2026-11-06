use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::condition::Condition;
use crate::error::Error;
use crate::{project, schedule, var_name, vars};

/// One entry of a job's `tasks`: a cmd task, a loop, or a trigger, each with an optional `when`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    /// Its key in the enclosing `tasks` or `do` map.
    #[serde(skip)]
    pub name: String,
    /// Program and arguments, run directly without a shell; `${VAR}` is substituted per element.
    #[serde(default, deserialize_with = "command")]
    pub cmd: Option<Vec<String>>,
    /// Further programs, each with its arguments, run like `cmd`: `cmd`'s stdout is piped
    /// into the first one's stdin, and so on.
    #[serde(default, deserialize_with = "pipeline")]
    pub pipe: Vec<Vec<String>>,
    /// Duration before the task's command is killed and the task fails, such as `30s` or `5m`.
    #[serde(default, deserialize_with = "duration")]
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
    /// The task runs only if this condition holds when it is reached; otherwise it is skipped.
    pub when: Option<Condition>,
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
    #[serde(default, deserialize_with = "duration")]
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

/// Reads an optional command while rejecting YAML scalars that serde would coerce to strings.
fn command<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Vec<String>>, D::Error> {
    Option::<Vec<Value>>::deserialize(deserializer)?
        .map(strings)
        .transpose()
}

/// Reads pipe stages while rejecting YAML scalars that serde would coerce to strings.
fn pipeline<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Vec<String>>, D::Error> {
    Vec::<Vec<Value>>::deserialize(deserializer)?
        .into_iter()
        .map(strings)
        .collect()
}

fn strings<E: de::Error>(values: Vec<Value>) -> Result<Vec<String>, E> {
    values
        .into_iter()
        .map(|value| match value {
            Value::String(value) => Ok(value),
            _ => Err(E::custom("command arguments must be strings")),
        })
        .collect()
}

/// Reads a duration such as `30s`, `5m` or `2h` as whole seconds.
pub(crate) fn duration<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u64>, D::Error> {
    struct DurationVisitor;

    impl Visitor<'_> for DurationVisitor {
        type Value = u64;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a duration like `30s`, `5m` or `2h`")
        }

        fn visit_str<E: de::Error>(self, text: &str) -> Result<u64, E> {
            schedule::parse_duration(text)
                .map(|duration| duration.as_secs())
                .map_err(E::custom)
        }
    }

    deserializer.deserialize_str(DurationVisitor).map(Some)
}

/// Reads a map of task name to task, keeping its order.
pub fn named<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Task>, D::Error> {
    struct NamedVisitor;

    impl<'de> Visitor<'de> for NamedVisitor {
        type Value = Vec<Task>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a map of task name to a task")
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Vec<Task>, A::Error> {
            let mut tasks: Vec<Task> = Vec::new();
            while let Some(name) = map.next_key::<String>()? {
                if tasks.iter().any(|task| task.name == name) {
                    return Err(de::Error::custom(format!("duplicate task `{name}`")));
                }
                let mut task: Task = map.next_value()?;
                task.name = name;
                tasks.push(task);
            }
            Ok(tasks)
        }
    }

    deserializer.deserialize_map(NamedVisitor)
}

/// Parses `yaml`, a map of task name to task, without resolving it.
#[cfg(test)]
pub(crate) fn parse_named(yaml: &str) -> Result<Vec<Task>, serde_yaml::Error> {
    named(serde_yaml::Deserializer::from_str(yaml))
}

impl Task {
    /// Whether any field other than `when` is set.
    fn has_fields_besides_when(&self) -> bool {
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
            || self.for_each.is_some()
            || self.item_var.is_some()
            || self.repeat.is_some()
            || self.attempts.is_some()
            || self.old_max.is_some()
            || !self.body.is_empty()
            || self.trigger.is_some()
    }
}

/// Validates tasks (recursively through loops) and resolves their paths against `dir`.
pub fn resolve(tasks: &mut [Task], dir: &Path) -> Result<(), Error> {
    for task in tasks {
        resolve_one(task, dir)?;
    }
    Ok(())
}

fn resolve_one(task: &mut Task, dir: &Path) -> Result<(), Error> {
    let name = task.name.clone();
    if !var_name::is_valid(&name) {
        return Err(Error::InvalidTask(format!(
            "task name `{name}` {}",
            var_name::RULE
        )));
    }
    if let Some(condition) = &task.when {
        condition
            .validate()
            .map_err(|source| Error::InvalidTask(format!("`{name}`: {source}")))?;
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
        return resolve_loop(task, &name, dir);
    }
    if task.item_var.is_some() {
        return Err(Error::InvalidTask(format!(
            "`{name}` has `as` but no `for_each`"
        )));
    }
    if task.repeat.is_some() {
        return resolve_while(task, &name, dir);
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
    resolve_leaf(task, &name)
}

fn resolve_loop(task: &mut Task, name: &str, dir: &Path) -> Result<(), Error> {
    if task.allow_failure {
        return Err(Error::InvalidTask(format!(
            "`{name}` is a loop (`for_each`): `allow_failure` only works on cmd tasks; use `on_failure: next` to skip to the next iteration when a `do` task fails"
        )));
    }
    if task.cmd.is_some()
        || task.timeout.is_some()
        || task.repeat.is_some()
        || task.attempts.is_some()
    {
        return Err(Error::InvalidTask(format!(
            "`{name}` is a loop (`for_each`) and cannot have `cmd`, `timeout`, `while` or `attempts`"
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
    resolve(&mut task.body, dir)
}

fn resolve_while(task: &mut Task, name: &str, dir: &Path) -> Result<(), Error> {
    if task.allow_failure {
        return Err(Error::InvalidTask(format!(
            "`{name}` is a loop (`while`): `allow_failure` only works on cmd tasks; use `on_failure: next` to skip to the next iteration when a `do` task fails"
        )));
    }
    if task.cmd.is_some() || task.timeout.is_some() {
        return Err(Error::InvalidTask(format!(
            "`{name}` is a loop (`while`) and cannot have `cmd` or `timeout`"
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
    resolve(&mut task.body, dir)
}

/// Checks a trigger, which has no other fields. Whether its job exists is checked once the
/// whole project is loaded.
fn resolve_trigger(task: &mut Task, name: &str) -> Result<(), Error> {
    let job = task.trigger.take();
    let has_others = task.has_fields_besides_when();
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
        for yaml in [
            "a: {cmd: [sleep, 5]}\n",
            "a: {cmd: [echo], pipe: [[head, 5]]}\n",
        ] {
            let err = parse_named(yaml).unwrap_err();
            assert!(
                err.to_string()
                    .contains("command arguments must be strings"),
                "{yaml}: {err}"
            );
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
    fn timeout_works_on_cmd_tasks_but_not_loops() {
        resolve_here("s:\n  cmd: [echo]\n  timeout: 5s\n").unwrap();
        for yaml in [
            "a:\n  for_each: X\n  do: {b: {cmd: [echo]}}\n  timeout: 1s\n",
            "a:\n  while: {this: X, eq: 1}\n  do: {b: {cmd: [echo]}}\n  timeout: 1s\n",
        ] {
            let msg = invalid_task(yaml);
            assert!(
                msg.contains("`a` is a") && msg.contains("`timeout`"),
                "{yaml}: {msg}"
            );
        }
    }

    #[test]
    fn timeout_takes_a_duration() {
        for (timeout, secs) in [("5s", 5), ("2m", 120), ("1h", 3600)] {
            let tasks =
                resolve_here(&format!("s:\n  cmd: [echo]\n  timeout: {timeout}\n")).unwrap();
            assert_eq!(tasks[0].timeout, Some(secs), "{timeout}");
        }
        for timeout in ["5", "5 min", "-1", "1.5"] {
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
            (
                "a:\n  cmd: [echo]\n  retry: {attempts: 2, delay: 2}\n",
                "a duration",
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
    fn when_works_on_every_kind_of_task() {
        let tasks = resolve_here(
            "a:\n  when: {this: X, eq: 1}\n  cmd: [echo]\n\
             l:\n  when: [{this: X, eq: 1}]\n  for_each: X\n  do: {b: {cmd: [echo]}}\n\
             w:\n  when: {this: X, eq: 1}\n  while: {this: X, eq: 1}\n  do: {c: {cmd: [echo]}}\n\
             t:\n  when: {this: X, eq: 1}\n  trigger: report\n",
        )
        .unwrap();
        assert!(tasks.iter().all(|task| task.when.is_some()));
    }

    #[test]
    fn if_and_switch_are_unknown_fields() {
        for (yaml, field) in [
            (
                "r:\n  if: {this: X, gt: 1}\n  then: {a: {cmd: [echo]}}\n",
                "if",
            ),
            (
                "s:\n  switch: X\n  cases: {1: {a: {cmd: [echo]}}}\n",
                "switch",
            ),
        ] {
            let err = parse_named(yaml).unwrap_err().to_string();
            assert!(
                err.contains(&format!("unknown field `{field}`")) && err.contains("`when`"),
                "{yaml}: {err}"
            );
        }
    }

    #[test]
    fn rejects_invalid_loops() {
        let body = "  do: {a: {cmd: [echo]}}\n";
        let a_loop = "`l` is a loop (`for_each`) and cannot have";
        let no_loop = "`l` has `do`/`attempts` but no `for_each` or `while`";
        for (yaml, expected) in [
            (format!("l:\n  for_each: X\n{body}  cmd: [echo]\n"), a_loop),
            (format!("l:\n  for_each: X\n{body}  timeout: 1s\n"), a_loop),
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
            (format!("w:\n{cond}{body}  timeout: 1s\n"), a_while),
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
            "a:\n  when: {this: X, eq: 1}\n  cmd: [a]\n  on_failure: next\n",
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
    fn triggers_take_no_other_fields() {
        let tasks = resolve_here("t:\n  trigger: report\n").unwrap();
        assert_eq!(tasks[0].trigger.as_deref(), Some("report"));
        for yaml in [
            "t:\n  trigger: report\n  timeout: 5s\n",
            "t:\n  trigger: report\n  allow_failure: true\n",
            "t:\n  trigger: report\n  cmd: [echo]\n",
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
        let msg = invalid_task("b:\n  for_each: X\n  do: {t: {trigger: r, cmd: [echo]}}\n");
        assert!(msg.contains("`t` is a trigger"), "{msg}");
    }

    #[test]
    fn rejects_invalid_condition() {
        let msg = invalid_task("r:\n  when: {this: X, gt: ten}\n  cmd: [echo]\n");
        assert!(
            msg.starts_with("`r`: invalid condition: `gt` needs a number"),
            "{msg}"
        );
    }

    #[test]
    fn for_each_takes_a_bare_or_wrapped_path() {
        let tasks =
            resolve_here("l:\n  for_each: ${tasks.a.stdout}\n  do: {a: {cmd: [echo]}}\n").unwrap();
        assert_eq!(tasks[0].for_each.as_deref(), Some("tasks.a.stdout"));
        let err = parse_named("l:\n  for_each: a${X}\n  do: {a: {cmd: [echo]}}\n").unwrap_err();
        assert!(err.to_string().contains("must name one variable"), "{err}");
    }

    #[test]
    fn rejects_a_task_that_is_not_a_map() {
        for yaml in ["a: 5\n", "a: tasks/login.yaml\n"] {
            let err = parse_named(yaml).unwrap_err();
            assert!(err.to_string().contains("invalid type"), "{yaml}: {err}");
        }
        let err = parse_named("a: {file: tasks/login.yaml}\n").unwrap_err();
        assert!(err.to_string().contains("unknown field `file`"), "{err}");
    }
}
