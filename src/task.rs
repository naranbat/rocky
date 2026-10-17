use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::condition::Condition;
use crate::error::Error;
use crate::request::{RequestSpec, Shorthand};
use crate::{project, var_name};

/// One entry of a job's `tasks`: an HTTP or cmd task, a branch, or a loop.
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
    /// An HTTP request.
    pub http: Option<RequestSpec>,
    /// An HTTP request with its method as the key: folded into `http` when resolved.
    pub get: Option<Shorthand>,
    pub post: Option<Shorthand>,
    pub put: Option<Shorthand>,
    pub patch: Option<Shorthand>,
    pub delete: Option<Shorthand>,
    /// Program and arguments, run directly without a shell; `${VAR}` is substituted per element.
    pub cmd: Option<Vec<String>>,
    /// Seconds before the task is stopped and fails: a `cmd` command is killed, an `http`
    /// request (default 30) is abandoned.
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
    #[serde(default, rename = "do", deserialize_with = "named")]
    pub body: Vec<Task>,
    /// Makes this a switch: runs the first case whose key equals this variable, else `default`.
    pub switch: Option<String>,
    #[serde(default, deserialize_with = "cases")]
    pub cases: Vec<Case>,
    #[serde(default, deserialize_with = "named")]
    pub default: Vec<Task>,
    /// Makes this a `set` task: new values for job `states`.
    pub set: Option<Set>,
}

/// A `set` task's body: name to new value; `${VAR}` is substituted.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Set {
    /// Job `states` to set; each is saved to the state file.
    #[serde(default, deserialize_with = "project::unique")]
    pub states: BTreeMap<String, Value>,
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
                if tasks.iter().any(|a| a.name == name) {
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
    /// Whether any key is set.
    fn has_fields(&self) -> bool {
        self.has_task_fields()
            || self.cwd.is_some()
            || self.stdin.is_some()
            || !self.env.is_empty()
            || self.capture_stdout.is_some()
            || self.condition.is_some()
            || !self.then.is_empty()
            || !self.otherwise.is_empty()
            || self.for_each.is_some()
            || self.item_var.is_some()
            || !self.body.is_empty()
            || self.has_switch_fields()
            || self.set.is_some()
    }

    fn has_switch_fields(&self) -> bool {
        self.switch.is_some() || !self.cases.is_empty() || !self.default.is_empty()
    }

    fn has_task_fields(&self) -> bool {
        self.http.is_some() || self.cmd.is_some() || self.timeout.is_some() || self.allow_failure
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
    resolve_method(task, &name)?;
    if let Some(path) = task.r#use.take() {
        return resolve_use(task, path, dir, stack);
    }
    if let Some(set) = task.set.take() {
        return resolve_set(task, set, &name);
    }
    for (field, set) in [
        ("cwd", task.cwd.is_some()),
        ("stdin", task.stdin.is_some()),
        ("env", !task.env.is_empty()),
        ("capture_stdout", task.capture_stdout.is_some()),
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
    if !task.body.is_empty() || task.item_var.is_some() {
        return Err(Error::InvalidTask(format!(
            "`{name}` has `do`/`as` but no `for_each`"
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

/// Moves a `get`/`post`/`put`/`patch`/`delete` request into `http`, with its method set, and
/// checks that an `http` request has a `method`.
fn resolve_method(task: &mut Task, name: &str) -> Result<(), Error> {
    let shorthands = [
        ("get", task.get.take()),
        ("post", task.post.take()),
        ("put", task.put.take()),
        ("patch", task.patch.take()),
        ("delete", task.delete.take()),
    ];
    for (key, shorthand) in shorthands {
        let Some(Shorthand(mut spec)) = shorthand else {
            continue;
        };
        if task.http.is_some() {
            return Err(Error::InvalidTask(format!(
                "`{name}` must have only one of `http`, `get`, `post`, `put`, `patch`, `delete`"
            )));
        }
        if !spec.method.is_empty() {
            return Err(Error::InvalidTask(format!(
                "`{name}`: `{key}` sets the method, so it can't have `method`"
            )));
        }
        spec.method = key.to_uppercase();
        task.http = Some(spec);
    }
    if task
        .http
        .as_ref()
        .is_some_and(|spec| spec.method.is_empty())
    {
        return Err(Error::InvalidTask(format!(
            "`{name}`: `http` needs a `method`"
        )));
    }
    Ok(())
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

fn resolve_set(task: &mut Task, set: Set, name: &str) -> Result<(), Error> {
    if set.states.is_empty() {
        return Err(Error::InvalidTask(format!(
            "`{name}`: `set` needs `states`"
        )));
    }
    if task.has_fields() {
        return Err(Error::InvalidTask(format!(
            "`{name}` is a `set` task and can't have other fields"
        )));
    }
    task.set = Some(set);
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
    {
        return Err(Error::InvalidTask(format!(
            "`{name}` is a loop (`for_each`) and cannot have `http`, `cmd`, `timeout`, `allow_failure`, `if`, `then`, `else`, `switch`, `cases` or `default`"
        )));
    }
    if task.body.is_empty() {
        return Err(Error::InvalidTask(format!(
            "`{name}` needs a non-empty `do`"
        )));
    }
    check_names(name, "as", task.item_var.iter())?;
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
            "`{name}` is a branch (`if`) and cannot have `http`, `cmd`, `timeout` or `allow_failure`"
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
            "`{name}` is a switch (`switch`) and cannot have `http`, `cmd`, `timeout`, `allow_failure`, `if`, `then` or `else`"
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

/// Checks an `http` or `cmd` task.
fn resolve_leaf(task: &Task, name: &str) -> Result<(), Error> {
    match (&task.http, &task.cmd) {
        (Some(_), None) => {}
        (None, Some(argv)) if !argv.is_empty() => {}
        (None, Some(_)) => {
            return Err(Error::InvalidTask(format!("`{name}` has an empty `cmd`")));
        }
        _ => {
            return Err(Error::InvalidTask(format!(
                "`{name}` must have exactly one of `http` (or `get`, `post`, `put`, `patch`, `delete`) or `cmd`"
            )));
        }
    }
    Ok(())
}

/// Checks that every name in the `field` (`as`) of task `name` is a valid var name.
fn check_names<'a>(
    name: &str,
    field: &str,
    mut vars: impl Iterator<Item = &'a String>,
) -> Result<(), Error> {
    match vars.find(|var| !var_name::is_valid_var(var)) {
        Some(var) => Err(Error::InvalidTask(format!(
            "`{name}`: `{field}` name `{var}` {}",
            var_name::VAR_RULE
        ))),
        None => Ok(()),
    }
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
            "login:\n  http: {method: GET, url: x}\n\
             get:\n  http:\n    method: GET\n    url: ${BASE}\n  allow_failure: true\n\
             echo:\n  cmd: [echo, '${BASE}']\n  cwd: work\n",
        )
        .unwrap();
        let login = &tasks[0];
        assert!(!login.allow_failure);
        assert_eq!(tasks[1].name, "get");
        assert!(tasks[1].allow_failure);
        assert_eq!(tasks[2].cwd, Some("work".into()));
        assert_eq!(tasks[2].dir, dir);
    }

    #[test]
    fn keeps_map_order_and_rejects_lists_and_duplicates() {
        let tasks = parse_named("z: {cmd: [echo]}\na: {cmd: [echo]}\nm: {cmd: [echo]}\n").unwrap();
        let names: Vec<_> = tasks.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["z", "a", "m"]);
        let err = parse_named("- cmd: [echo]\n").unwrap_err();
        assert!(err.to_string().contains("a map of task name"), "{err}");
        let err = parse_named("a: {cmd: [echo]}\na: {cmd: [echo]}\n").unwrap_err();
        assert!(err.to_string().contains("duplicate"), "{err}");
        let err = parse_named("a: {name: b, cmd: [echo]}\n").unwrap_err();
        assert!(err.to_string().contains("unknown field `name`"), "{err}");
    }

    #[test]
    fn task_needs_exactly_one_kind() {
        let neither = "`a` must have exactly one of `http`";
        for (yaml, expected) in [
            ("a: {}\n", neither),
            (
                "a:\n  http: {method: GET, url: x}\n  cmd: [echo]\n",
                neither,
            ),
            ("a:\n  cmd: []\n", "`a` has an empty `cmd`"),
        ] {
            let msg = invalid_task(yaml);
            assert!(msg.contains(expected), "{yaml}: {msg}");
        }
    }

    #[test]
    fn method_keys_become_http() {
        let tasks =
            resolve_here("a:\n  get: x\nb:\n  post: {url: y, json: {k: 1}}\nc:\n  delete: z\n")
                .unwrap();
        let specs: Vec<_> = tasks.iter().filter_map(|a| a.http.as_ref()).collect();
        assert_eq!(specs.len(), 3);
        assert_eq!(
            (specs[0].method.as_str(), specs[0].url.as_str()),
            ("GET", "x")
        );
        assert_eq!(
            (specs[1].method.as_str(), specs[1].url.as_str()),
            ("POST", "y")
        );
        assert_eq!(specs[1].json, Some(serde_json::json!({"k": 1})));
        assert_eq!(
            (specs[2].method.as_str(), specs[2].url.as_str()),
            ("DELETE", "z")
        );
    }

    #[test]
    fn rejects_conflicting_or_missing_methods() {
        let several = "`a` must have only one of `http`, `get`";
        for (yaml, expected) in [
            ("a:\n  get: x\n  post: y\n", several),
            ("a:\n  get: x\n  http: {method: GET, url: x}\n", several),
            (
                "a:\n  post: {method: GET, url: x}\n",
                "`a`: `post` sets the method, so it can't have `method`",
            ),
            ("a:\n  http: {url: x}\n", "`a`: `http` needs a `method`"),
            (
                "a:\n  put: x\n  cmd: [echo]\n",
                "`a` must have exactly one of `http`",
            ),
            (
                "a:\n  if: {this: X, eq: 1}\n  then: {b: {cmd: [echo]}}\n  patch: x\n",
                "`a` is a branch (`if`) and cannot have `http`",
            ),
        ] {
            let msg = invalid_task(yaml);
            assert!(msg.contains(expected), "{yaml}: {msg}");
        }
    }

    #[test]
    fn http_must_be_inline() {
        assert!(parse_named("a:\n  http: req.yaml\n").is_err());
    }

    #[test]
    fn cmd_options_need_cmd() {
        for (yaml, field) in [
            ("a:\n  http: {method: GET, url: x}\n  cwd: w\n", "cwd"),
            ("a:\n  http: {method: GET, url: x}\n  stdin: y\n", "stdin"),
            ("a:\n  http: {method: GET, url: x}\n  env: {A: y}\n", "env"),
            (
                "a:\n  http: {method: GET, url: x}\n  capture_stdout: false\n",
                "capture_stdout",
            ),
        ] {
            let msg = invalid_task(yaml);
            assert!(
                msg.contains(&format!("`a` has `{field}`, which only works with `cmd`")),
                "{yaml}: {msg}"
            );
        }
    }

    #[test]
    fn timeout_works_on_every_task_kind_but_not_branches_or_loops() {
        resolve_here(
            "h:\n  http: {method: GET, url: x}\n  timeout: 5\ns:\n  cmd: [echo]\n  timeout: 5\n",
        )
        .unwrap();
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
                "a:\n  for_each: X\n  as: states\n  do: {b: {cmd: [echo]}}\n",
                "as",
                "states",
            ),
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
            ("states:\n  cmd: [echo]\n", "states"),
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
                "r:\n  if: {this: X, gt: 1}\n  then: {a: {http: {method: GET, url: x}}}\n  http: {method: GET, url: x}\n",
                branch,
            ),
            (
                "r:\n  if: {this: X, gt: 1}\n  then: {a: {http: {method: GET, url: x}}}\n  cmd: [echo]\n",
                branch,
            ),
            (
                "r:\n  if: {this: X, gt: 1}\n",
                "`r` needs a non-empty `then` or `else`",
            ),
            (
                "r:\n  http: {method: GET, url: x}\n  then: {b: {http: {method: GET, url: x}}}\n",
                "`r` has `then`/`else` but no `if`",
            ),
        ] {
            let msg = invalid_task(yaml);
            assert!(msg.contains(expected), "{yaml}: {msg}");
        }
    }

    #[test]
    fn rejects_invalid_loops() {
        let body = "  do: {a: {http: {method: GET, url: x}}}\n";
        let a_loop = "`l` is a loop (`for_each`) and cannot have";
        let no_loop = "`l` has `do`/`as` but no `for_each`";
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
            (
                format!("l:\n  http: {{method: GET, url: x}}\n{body}"),
                no_loop,
            ),
            (
                "l:\n  http: {method: GET, url: x}\n  as: X\n".to_string(),
                no_loop,
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
        let case = "  cases: {1: {a: {http: {method: GET, url: x}}}}\n";
        let switch = "`s` is a switch (`switch`) and cannot have";
        let no_cases = "`s` needs a non-empty `cases`";
        let no_switch = "`s` has `cases`/`default` but no `switch`";
        for (yaml, expected) in [
            (format!("s:\n  switch: X\n{case}  cmd: [echo]\n"), switch),
            (format!("s:\n  switch: X\n{case}  allow_failure: true\n"), switch),
            (format!("s:\n  switch: X\n{case}  if: {{this: X, eq: 1}}\n"), switch),
            ("s:\n  switch: X\n".to_string(), no_cases),
            (
                "s:\n  switch: X\n  default: {a: {http: {method: GET, url: x}}}\n".to_string(),
                no_cases,
            ),
            (
                "s:\n  switch: X\n  cases: {1: {}}\n".to_string(),
                "`s` case 1 needs at least one task",
            ),
            (format!("s:\n{case}"), no_switch),
            (
                "s:\n  http: {method: GET, url: x}\n  default: {b: {http: {method: GET, url: x}}}\n"
                    .to_string(),
                no_switch,
            ),
            (
                format!(
                    "s:\n  for_each: X\n  do: {{a: {{http: {{method: GET, url: x}}}}}}\n  switch: X\n{case}"
                ),
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
            parse_named(
                "s:\n  switch: X\n  cases: [{match: 1, do: {a: {http: {method: GET, url: x}}}}]\n"
            )
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
        std::fs::write(sub.join("login.yaml"), "http: {method: GET, url: x}\n").unwrap();
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
    fn set_goes_alone() {
        let tasks = resolve_here("s:\n  set: {states: {N: '${A}'}}\n").unwrap();
        let set = tasks[0].set.as_ref().unwrap();
        assert_eq!(set.states["N"], Value::from("${A}"));
        for yaml in [
            "s:\n  set: {states: {N: 1}}\n  cmd: [echo]\n",
            "s:\n  set: {states: {N: 1}}\n  allow_failure: true\n",
            "s:\n  set: {states: {N: 1}}\n  if: {this: X, eq: 1}\n  then: {a: {cmd: [echo]}}\n",
            "s:\n  set: {states: {N: 1}}\n  for_each: X\n  do: {a: {cmd: [echo]}}\n",
            "s:\n  set: {states: {N: 1}}\n  cwd: w\n",
            "s:\n  set: {states: {N: 1}}\n  env: {A: b}\n",
        ] {
            let msg = invalid_task(yaml);
            assert!(
                msg.contains("`s` is a `set` task and can't have other fields"),
                "{yaml}: {msg}"
            );
        }
    }

    #[test]
    fn set_needs_states() {
        for yaml in ["s:\n  set: {}\n", "s:\n  set: {states: {}}\n"] {
            let msg = invalid_task(yaml);
            assert!(msg.contains("`s`: `set` needs `states`"), "{yaml}: {msg}");
        }
        assert!(parse_named("s:\n  set: {state: {N: 1}}\n").is_err());
        assert!(parse_named("s:\n  states_update: {N: 1}\n").is_err());
    }

    #[test]
    fn rejects_invalid_condition() {
        let msg = invalid_task(
            "r:\n  if: {this: X, gt: ten}\n  then: {a: {http: {method: GET, url: x}}}\n",
        );
        assert!(
            msg.starts_with("`r`: invalid condition: `gt` needs a number"),
            "{msg}"
        );
    }
}
