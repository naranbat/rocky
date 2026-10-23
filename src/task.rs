use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::condition::Condition;
use crate::error::Error;
use crate::{project, var_name};

/// One entry of a job's `tasks`: a cmd task, a branch, a switch, or a loop.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    /// Its key in the enclosing `tasks`, `then`, `else`, `do` or `default` map.
    #[serde(skip)]
    pub name: String,
    /// A task YAML file, relative to the file that declares this task, used in its place:
    /// the entry's value when it is a string.
    #[serde(skip)]
    pub r#use: Option<PathBuf>,
    /// Program and arguments, run directly without a shell; `${VAR}` is substituted per element.
    pub cmd: Option<Vec<String>>,
    /// Further programs, each with its arguments, run like `cmd`: `cmd`'s stdout is piped
    /// into the first one's stdin, and so on.
    #[serde(default)]
    pub pipe: Vec<Vec<String>>,
    /// Seconds before the task's command is killed and the task fails.
    pub timeout: Option<u64>,
    /// Directory a `cmd` task runs in, relative to the file that declares this task;
    /// default: the current directory. `${VAR}` is substituted.
    pub cwd: Option<PathBuf>,
    /// Directory of the file that declares this task; `cwd` is relative to it.
    #[serde(skip)]
    pub dir: PathBuf,
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
    #[serde(default)]
    pub allow_failure: bool,
    /// Makes this a branch: runs `then` if the condition holds, else `else`.
    #[serde(rename = "if")]
    pub condition: Option<Condition>,
    #[serde(default, deserialize_with = "named")]
    pub then: Vec<Task>,
    #[serde(default, rename = "else", deserialize_with = "named")]
    pub otherwise: Vec<Task>,
    /// Makes this a loop: runs `do` once per element of this list variable.
    pub for_each: Option<String>,
    /// The variable each element is bound to inside `do` (default `ITEM`).
    #[serde(rename = "as")]
    pub item_var: Option<String>,
    /// Makes this a `while` loop: runs `do` again and again while the condition holds.
    #[serde(rename = "while")]
    pub repeat: Option<Condition>,
    /// The most iterations a `while` loop may run before it fails.
    pub max: Option<u64>,
    #[serde(default, rename = "do", deserialize_with = "named")]
    pub body: Vec<Task>,
    /// Makes this a switch: runs the first case whose key equals this variable, else `default`.
    pub switch: Option<String>,
    #[serde(default, deserialize_with = "cases")]
    pub cases: Vec<Case>,
    #[serde(default, deserialize_with = "named")]
    pub default: Vec<Task>,
}

/// One `cases` entry of a switch: tasks to run when the variable equals its key.
#[derive(Clone, Debug)]
pub struct Case {
    pub value: Value,
    pub body: Vec<Task>,
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
        return resolve_use(task, path, dir, stack);
    }
    for (field, set) in [
        ("cwd", task.cwd.is_some()),
        ("stdin", task.stdin.is_some()),
        ("env", !task.env.is_empty()),
        ("capture_stdout", task.capture_stdout.is_some()),
        ("pipe", !task.pipe.is_empty()),
    ] {
        if set && task.cmd.is_none() {
            return Err(Error::InvalidTask(format!(
                "`{name}` has `{field}`, which only works with `cmd`"
            )));
        }
    }
    task.dir = dir.to_path_buf();
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
    if !task.body.is_empty() || task.max.is_some() {
        return Err(Error::InvalidTask(format!(
            "`{name}` has `do`/`max` but no `for_each` or `while`"
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
    if task.has_task_fields()
        || task.condition.is_some()
        || !task.then.is_empty()
        || !task.otherwise.is_empty()
        || task.has_switch_fields()
        || task.repeat.is_some()
        || task.max.is_some()
    {
        return Err(Error::InvalidTask(format!(
            "`{name}` is a loop (`for_each`) and cannot have `cmd`, `timeout`, `allow_failure`, `if`, `then`, `else`, `switch`, `cases`, `default`, `while` or `max`"
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
    if task.has_task_fields()
        || task.condition.is_some()
        || !task.then.is_empty()
        || !task.otherwise.is_empty()
        || task.has_switch_fields()
    {
        return Err(Error::InvalidTask(format!(
            "`{name}` is a loop (`while`) and cannot have `cmd`, `timeout`, `allow_failure`, `if`, `then`, `else`, `switch`, `cases` or `default`"
        )));
    }
    if task.body.is_empty() {
        return Err(Error::InvalidTask(format!(
            "`{name}` needs a non-empty `do`"
        )));
    }
    if task.max == Some(0) {
        return Err(Error::InvalidTask(format!(
            "`{name}`: `max` must be at least 1"
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
        let no_loop = "`l` has `do`/`max` but no `for_each` or `while`";
        for (yaml, expected) in [
            (format!("l:\n  for_each: X\n{body}  cmd: [echo]\n"), a_loop),
            (
                format!("l:\n  for_each: X\n{body}  allow_failure: true\n"),
                a_loop,
            ),
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
            ("l:\n  cmd: [echo]\n  max: 3\n".to_string(), no_loop),
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
                format!("w:\n{cond}{body}  max: 0\n"),
                "`w`: `max` must be at least 1",
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
            "w:\n  while: {this: X, eq: 1}\n  max: 3\n  do: {a: {cmd: [a], cwd: a}}\n",
        )
        .unwrap();
        assert_eq!(tasks[0].max, Some(3));
        assert_eq!(tasks[0].body[0].dir, dir);
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
    fn rejects_invalid_condition() {
        let msg = invalid_task("r:\n  if: {this: X, gt: ten}\n  then: {a: {cmd: [echo]}}\n");
        assert!(
            msg.starts_with("`r`: invalid condition: `gt` needs a number"),
            "{msg}"
        );
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
