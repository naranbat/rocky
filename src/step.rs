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

/// One entry of a job's `steps`: an HTTP or cmd step, a branch, or a loop.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// Its key in the enclosing `steps`, `then`, `else`, `do` or `default` map.
    #[serde(skip)]
    pub name: String,
    /// A step YAML file, relative to the file that declares this step, used in its place:
    /// the entry's value when it is a string.
    #[serde(skip)]
    pub r#use: Option<PathBuf>,
    /// Values for `${NAME}` in this step and the steps nested in it, checked before the
    /// enclosing steps', job's and project's `vars`.
    #[serde(default, deserialize_with = "project::unique")]
    pub vars: BTreeMap<String, String>,
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
    /// Seconds before the step is stopped and fails: a `cmd` command is killed, an `http`
    /// request (default 30) is abandoned.
    pub timeout: Option<u64>,
    /// Directory a `cmd` step runs in, relative to the file that declares this step;
    /// default: the current directory. `${VAR}` is substituted.
    pub cwd: Option<PathBuf>,
    /// Directory of the file that declares this step; `cwd` is relative to it.
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
    pub then: Vec<Step>,
    #[serde(default, rename = "else", deserialize_with = "named")]
    pub otherwise: Vec<Step>,
    /// Makes this a loop: runs `do` once per element of this list variable.
    pub for_each: Option<String>,
    /// The variable each element is bound to inside `do` (default `ITEM`).
    #[serde(rename = "as")]
    pub item_var: Option<String>,
    #[serde(default, rename = "do", deserialize_with = "named")]
    pub body: Vec<Step>,
    /// Makes this a switch: runs the first case whose key equals this variable, else `default`.
    pub switch: Option<String>,
    #[serde(default, deserialize_with = "cases")]
    pub cases: Vec<Case>,
    #[serde(default, deserialize_with = "named")]
    pub default: Vec<Step>,
    /// Makes this a `set` step: new values for job `states` and project `vars`.
    pub set: Option<Set>,
}

/// A `set` step's body: name to new value; `${VAR}` is substituted.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Set {
    /// Job `states` to set; each is saved to the state file.
    #[serde(default, deserialize_with = "project::unique")]
    pub states: BTreeMap<String, Value>,
    /// Project `vars` to set, in memory only.
    #[serde(default, deserialize_with = "project::unique")]
    pub vars: BTreeMap<String, Value>,
}

/// One `cases` entry of a switch: steps to run when the variable equals its key.
#[derive(Clone, Debug)]
pub struct Case {
    pub value: Value,
    pub body: Vec<Step>,
}

/// Reads a switch's `cases`: a map of value to the steps to run, keeping its order.
fn cases<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Case>, D::Error> {
    struct CasesVisitor;

    /// A case's steps, read like any other map of named steps.
    struct Body(Vec<Step>);

    impl<'de> Deserialize<'de> for Body {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            named(deserializer).map(Body)
        }
    }

    impl<'de> Visitor<'de> for CasesVisitor {
        type Value = Vec<Case>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a map of value to a map of steps")
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

/// Reads a map of step name to step, or to the path of a step file, keeping its order.
pub fn named<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Step>, D::Error> {
    struct NamedVisitor;

    impl<'de> Visitor<'de> for NamedVisitor {
        type Value = Vec<Step>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a map of step name to a step or a step file path")
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Vec<Step>, A::Error> {
            let mut steps: Vec<Step> = Vec::new();
            while let Some(name) = map.next_key::<String>()? {
                if steps.iter().any(|a| a.name == name) {
                    return Err(de::Error::custom(format!("duplicate step `{name}`")));
                }
                let Source(mut step) = map.next_value()?;
                step.name = name;
                steps.push(step);
            }
            Ok(steps)
        }
    }

    deserializer.deserialize_map(NamedVisitor)
}

/// Parses `yaml`, a map of step name to step, without resolving step files.
#[cfg(test)]
pub(crate) fn parse_named(yaml: &str) -> Result<Vec<Step>, serde_yaml::Error> {
    named(serde_yaml::Deserializer::from_str(yaml))
}

/// An entry's value: a step file path, or the step's fields inline.
struct Source(Step);

/// Picks the variant by YAML shape, so a mistake in an inline step reports the real cause
/// and line instead of a generic untagged-enum error.
impl<'de> Deserialize<'de> for Source {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct SourceVisitor;

        impl<'de> Visitor<'de> for SourceVisitor {
            type Value = Source;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a step file path or a step")
            }

            fn visit_str<E: de::Error>(self, path: &str) -> Result<Source, E> {
                Ok(Source(Step {
                    r#use: Some(PathBuf::from(path)),
                    ..Step::default()
                }))
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Source, A::Error> {
                Step::deserialize(de::value::MapAccessDeserializer::new(map)).map(Source)
            }
        }

        deserializer.deserialize_any(SourceVisitor)
    }
}

impl Step {
    /// Whether any key besides `vars` is set.
    fn has_fields_besides_vars(&self) -> bool {
        self.has_step_fields()
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

    fn has_step_fields(&self) -> bool {
        self.http.is_some() || self.cmd.is_some() || self.timeout.is_some() || self.allow_failure
    }
}

/// Validates steps (recursively through branches and loops) and resolves their paths against `dir`.
pub fn resolve(steps: &mut [Step], dir: &Path) -> Result<(), Error> {
    resolve_all(steps, dir, &mut Vec::new())
}

/// [`resolve`], with `stack` holding the step files being loaded, to detect step file cycles.
fn resolve_all(steps: &mut [Step], dir: &Path, stack: &mut Vec<PathBuf>) -> Result<(), Error> {
    for step in steps {
        resolve_one(step, dir, stack)?;
    }
    Ok(())
}

fn resolve_one(step: &mut Step, dir: &Path, stack: &mut Vec<PathBuf>) -> Result<(), Error> {
    let name = step.name.clone();
    if !var_name::is_valid(&name) {
        return Err(Error::InvalidStep(format!(
            "step name `{name}` {}",
            var_name::RULE
        )));
    }
    check_names(&name, "vars", step.vars.keys())?;
    resolve_method(step, &name)?;
    if let Some(path) = step.r#use.take() {
        return resolve_use(step, path, dir, stack);
    }
    if let Some(set) = step.set.take() {
        return resolve_set(step, set, &name);
    }
    for (field, set) in [
        ("cwd", step.cwd.is_some()),
        ("stdin", step.stdin.is_some()),
        ("env", !step.env.is_empty()),
        ("capture_stdout", step.capture_stdout.is_some()),
    ] {
        if set && step.cmd.is_none() {
            return Err(Error::InvalidStep(format!(
                "`{name}` has `{field}`, which only works with `cmd`"
            )));
        }
    }
    step.dir = dir.to_path_buf();
    if step.for_each.is_some() {
        return resolve_loop(step, &name, dir, stack);
    }
    if !step.body.is_empty() || step.item_var.is_some() {
        return Err(Error::InvalidStep(format!(
            "`{name}` has `do`/`as` but no `for_each`"
        )));
    }
    if step.switch.is_some() {
        return resolve_switch(step, &name, dir, stack);
    }
    if !step.cases.is_empty() || !step.default.is_empty() {
        return Err(Error::InvalidStep(format!(
            "`{name}` has `cases`/`default` but no `switch`"
        )));
    }
    if step.condition.is_some() {
        return resolve_branch(step, &name, dir, stack);
    }
    if !step.then.is_empty() || !step.otherwise.is_empty() {
        return Err(Error::InvalidStep(format!(
            "`{name}` has `then`/`else` but no `if`"
        )));
    }
    resolve_leaf(step, &name)
}

/// Moves a `get`/`post`/`put`/`patch`/`delete` request into `http`, with its method set, and
/// checks that an `http` request has a `method`.
fn resolve_method(step: &mut Step, name: &str) -> Result<(), Error> {
    let shorthands = [
        ("get", step.get.take()),
        ("post", step.post.take()),
        ("put", step.put.take()),
        ("patch", step.patch.take()),
        ("delete", step.delete.take()),
    ];
    for (key, shorthand) in shorthands {
        let Some(Shorthand(mut spec)) = shorthand else {
            continue;
        };
        if step.http.is_some() {
            return Err(Error::InvalidStep(format!(
                "`{name}` must have only one of `http`, `get`, `post`, `put`, `patch`, `delete`"
            )));
        }
        if !spec.method.is_empty() {
            return Err(Error::InvalidStep(format!(
                "`{name}`: `{key}` sets the method, so it can't have `method`"
            )));
        }
        spec.method = key.to_uppercase();
        step.http = Some(spec);
    }
    if step
        .http
        .as_ref()
        .is_some_and(|spec| spec.method.is_empty())
    {
        return Err(Error::InvalidStep(format!(
            "`{name}`: `http` needs a `method`"
        )));
    }
    Ok(())
}

/// Replaces `step` with the step file at `path`, named by `step`'s key.
fn resolve_use(
    step: &mut Step,
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
        return Err(Error::InvalidStep(format!(
            "`{}`: step file cycle through {}",
            step.name,
            path.display()
        )));
    }
    let mut loaded: Step = project::read_yaml(&path)?;
    loaded.name = std::mem::take(&mut step.name);
    let file_dir = path.parent().unwrap_or(Path::new(""));
    stack.push(canonical);
    resolve_one(&mut loaded, file_dir, stack)?;
    stack.pop();
    *step = loaded;
    Ok(())
}

fn resolve_set(step: &mut Step, set: Set, name: &str) -> Result<(), Error> {
    if set.states.is_empty() && set.vars.is_empty() {
        return Err(Error::InvalidStep(format!(
            "`{name}`: `set` needs `states` or `vars`"
        )));
    }
    if step.has_fields_besides_vars() {
        return Err(Error::InvalidStep(format!(
            "`{name}` is a `set` step and can only have `vars`"
        )));
    }
    step.set = Some(set);
    Ok(())
}

fn resolve_loop(
    step: &mut Step,
    name: &str,
    dir: &Path,
    stack: &mut Vec<PathBuf>,
) -> Result<(), Error> {
    if step.has_step_fields()
        || step.condition.is_some()
        || !step.then.is_empty()
        || !step.otherwise.is_empty()
        || step.has_switch_fields()
    {
        return Err(Error::InvalidStep(format!(
            "`{name}` is a loop (`for_each`) and cannot have `http`, `cmd`, `timeout`, `allow_failure`, `if`, `then`, `else`, `switch`, `cases` or `default`"
        )));
    }
    if step.body.is_empty() {
        return Err(Error::InvalidStep(format!(
            "`{name}` needs a non-empty `do`"
        )));
    }
    check_names(name, "as", step.item_var.iter())?;
    resolve_all(&mut step.body, dir, stack)
}

fn resolve_branch(
    step: &mut Step,
    name: &str,
    dir: &Path,
    stack: &mut Vec<PathBuf>,
) -> Result<(), Error> {
    if step.has_step_fields() {
        return Err(Error::InvalidStep(format!(
            "`{name}` is a branch (`if`) and cannot have `http`, `cmd`, `timeout` or `allow_failure`"
        )));
    }
    if step.then.is_empty() && step.otherwise.is_empty() {
        return Err(Error::InvalidStep(format!(
            "`{name}` needs a non-empty `then` or `else`"
        )));
    }
    if let Some(condition) = &step.condition {
        condition
            .validate()
            .map_err(|source| Error::InvalidStep(format!("`{name}`: {source}")))?;
    }
    resolve_all(&mut step.then, dir, stack)?;
    resolve_all(&mut step.otherwise, dir, stack)
}

fn resolve_switch(
    step: &mut Step,
    name: &str,
    dir: &Path,
    stack: &mut Vec<PathBuf>,
) -> Result<(), Error> {
    if step.has_step_fields()
        || step.condition.is_some()
        || !step.then.is_empty()
        || !step.otherwise.is_empty()
    {
        return Err(Error::InvalidStep(format!(
            "`{name}` is a switch (`switch`) and cannot have `http`, `cmd`, `timeout`, `allow_failure`, `if`, `then` or `else`"
        )));
    }
    if step.cases.is_empty() {
        return Err(Error::InvalidStep(format!(
            "`{name}` needs a non-empty `cases`"
        )));
    }
    for case in &mut step.cases {
        if case.body.is_empty() {
            return Err(Error::InvalidStep(format!(
                "`{name}` case {} needs at least one step",
                case.value
            )));
        }
        resolve_all(&mut case.body, dir, stack)?;
    }
    resolve_all(&mut step.default, dir, stack)
}

/// Checks an `http` or `cmd` step.
fn resolve_leaf(step: &Step, name: &str) -> Result<(), Error> {
    match (&step.http, &step.cmd) {
        (Some(_), None) => {}
        (None, Some(argv)) if !argv.is_empty() => {}
        (None, Some(_)) => {
            return Err(Error::InvalidStep(format!("`{name}` has an empty `cmd`")));
        }
        _ => {
            return Err(Error::InvalidStep(format!(
                "`{name}` must have exactly one of `http` (or `get`, `post`, `put`, `patch`, `delete`) or `cmd`"
            )));
        }
    }
    Ok(())
}

/// Checks that every name in the `field` (`vars` or `as`) of step `name` is a valid var name.
fn check_names<'a>(
    name: &str,
    field: &str,
    mut vars: impl Iterator<Item = &'a String>,
) -> Result<(), Error> {
    match vars.find(|var| !var_name::is_valid_var(var)) {
        Some(var) => Err(Error::InvalidStep(format!(
            "`{name}`: `{field}` name `{var}` {}",
            var_name::VAR_RULE
        ))),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve_in(dir: &Path, yaml: &str) -> Result<Vec<Step>, Error> {
        let mut steps = parse_named(yaml).unwrap();
        resolve(&mut steps, dir)?;
        Ok(steps)
    }

    /// The steps in `yaml`, resolved against the current directory.
    fn resolve_here(yaml: &str) -> Result<Vec<Step>, Error> {
        resolve_in(Path::new(""), yaml)
    }

    /// The message of the invalid-step error that resolving `yaml` must give.
    fn invalid_step(yaml: &str) -> String {
        match resolve_here(yaml) {
            Err(Error::InvalidStep(msg)) => msg,
            Err(err) => panic!("{yaml}: expected an invalid step, got {err}"),
            Ok(_) => panic!("{yaml}: expected an invalid step, got none"),
        }
    }

    #[test]
    fn resolves_paths_against_project_dir() {
        let dir = Path::new("/project");
        let steps = resolve_in(
            dir,
            "login:\n  http: {method: GET, url: x}\n\
             get:\n  http:\n    method: GET\n    url: ${BASE}\n  allow_failure: true\n\
             echo:\n  cmd: [echo, '${BASE}']\n  cwd: work\n",
        )
        .unwrap();
        let login = &steps[0];
        assert!(!login.allow_failure);
        assert_eq!(steps[1].name, "get");
        assert!(steps[1].allow_failure);
        assert_eq!(steps[2].cwd, Some("work".into()));
        assert_eq!(steps[2].dir, dir);
    }

    #[test]
    fn keeps_map_order_and_rejects_lists_and_duplicates() {
        let steps = parse_named("z: {cmd: [echo]}\na: {cmd: [echo]}\nm: {cmd: [echo]}\n").unwrap();
        let names: Vec<_> = steps.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["z", "a", "m"]);
        let err = parse_named("- cmd: [echo]\n").unwrap_err();
        assert!(err.to_string().contains("a map of step name"), "{err}");
        let err = parse_named("a: {cmd: [echo]}\na: {cmd: [echo]}\n").unwrap_err();
        assert!(err.to_string().contains("duplicate"), "{err}");
        let err = parse_named("a: {name: b, cmd: [echo]}\n").unwrap_err();
        assert!(err.to_string().contains("unknown field `name`"), "{err}");
    }

    #[test]
    fn step_needs_exactly_one_kind() {
        let neither = "`a` must have exactly one of `http`";
        for (yaml, expected) in [
            ("a: {}\n", neither),
            (
                "a:\n  http: {method: GET, url: x}\n  cmd: [echo]\n",
                neither,
            ),
            ("a:\n  cmd: []\n", "`a` has an empty `cmd`"),
        ] {
            let msg = invalid_step(yaml);
            assert!(msg.contains(expected), "{yaml}: {msg}");
        }
    }

    #[test]
    fn method_keys_become_http() {
        let steps =
            resolve_here("a:\n  get: x\nb:\n  post: {url: y, json: {k: 1}}\nc:\n  delete: z\n")
                .unwrap();
        let specs: Vec<_> = steps.iter().filter_map(|a| a.http.as_ref()).collect();
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
            let msg = invalid_step(yaml);
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
            let msg = invalid_step(yaml);
            assert!(
                msg.contains(&format!("`a` has `{field}`, which only works with `cmd`")),
                "{yaml}: {msg}"
            );
        }
    }

    #[test]
    fn timeout_works_on_every_step_kind_but_not_branches_or_loops() {
        resolve_here(
            "h:\n  http: {method: GET, url: x}\n  timeout: 5\ns:\n  cmd: [echo]\n  timeout: 5\n",
        )
        .unwrap();
        for yaml in [
            "a:\n  if: {this: X, eq: 1}\n  then: {b: {cmd: [echo]}}\n  timeout: 1\n",
            "a:\n  for_each: X\n  do: {b: {cmd: [echo]}}\n  timeout: 1\n",
        ] {
            let msg = invalid_step(yaml);
            assert!(
                msg.contains("`a` is a") && msg.contains("`timeout`"),
                "{yaml}: {msg}"
            );
        }
    }

    #[test]
    fn rejects_invalid_loop_and_vars_names() {
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
            ("a:\n  cmd: [echo]\n  vars: {1A: x}\n", "vars", "1A"),
            ("a:\n  cmd: [echo]\n  vars: {lower: x}\n", "vars", "lower"),
            (
                "a:\n  for_each: X\n  as: item\n  do: {b: {cmd: [echo]}}\n",
                "as",
                "item",
            ),
        ] {
            assert_eq!(
                invalid_step(yaml),
                format!("`a`: `{field}` name `{var}` {}", var_name::VAR_RULE),
                "{yaml}"
            );
        }
    }

    #[test]
    fn rejects_invalid_step_name() {
        for (yaml, name) in [
            ("a.b:\n  cmd: [echo]\n", "a.b"),
            ("a.b: x.yaml\n", "a.b"),
            ("a-b:\n  cmd: [echo]\n", "a-b"),
            ("1a:\n  cmd: [echo]\n", "1a"),
            ("states:\n  cmd: [echo]\n", "states"),
        ] {
            assert_eq!(
                invalid_step(yaml),
                format!("step name `{name}` {}", var_name::RULE),
                "{yaml}"
            );
        }
    }

    #[test]
    fn resolves_paths_inside_nested_branches_and_loops() {
        let dir = Path::new("/project");
        let steps = resolve_in(
            dir,
            "r:\n  if: {this: X, gt: 1}\n  then:\n    a:\n      cmd: [a]\n      cwd: a\n  else:\n\
             \x20   l:\n      for_each: IDS\n      do:\n        b:\n          cmd: [b]\n          cwd: b\n",
        )
        .unwrap();
        assert_eq!(steps[0].then[0].dir, dir);
        assert_eq!(steps[0].otherwise[0].body[0].dir, dir);
    }

    #[test]
    fn rejects_mixed_branch_and_step_fields() {
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
            let msg = invalid_step(yaml);
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
            let msg = invalid_step(&yaml);
            assert!(msg.contains(expected), "{yaml}: {msg}");
        }
    }

    #[test]
    fn resolves_paths_inside_switch_cases() {
        let dir = Path::new("/project");
        let steps = resolve_in(
            dir,
            "s:\n  switch: X\n  cases:\n    1: {a: {cmd: [a], cwd: a}}\n\
             \x20 default: {b: {cmd: [b], cwd: b}}\n",
        )
        .unwrap();
        assert_eq!(steps[0].cases[0].body[0].dir, dir);
        assert_eq!(steps[0].default[0].dir, dir);
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
                "`s` case 1 needs at least one step",
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
            let msg = invalid_step(&yaml);
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
    fn step_files_load_relative_to_themselves_named_by_key() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("steps");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("login.yaml"), "http: {method: GET, url: x}\n").unwrap();
        std::fs::write(
            sub.join("each.yaml"),
            "for_each: IDS\ndo:\n  inner: login.yaml\n  echo:\n    cmd: [echo]\n    cwd: w\n",
        )
        .unwrap();
        let steps = resolve_in(
            dir.path(),
            "login: steps/login.yaml\nagain: steps/login.yaml\neach: steps/each.yaml\n",
        )
        .unwrap();
        assert_eq!(steps[0].name, "login");
        assert_eq!(steps[1].name, "again");
        assert_eq!(steps[2].name, "each");
        assert_eq!(steps[2].body[0].name, "inner");
        assert_eq!(steps[2].body[1].dir, sub);
    }

    #[test]
    fn step_files_reject_names_missing_files_and_cycles() {
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
            matches!(&err, Error::InvalidStep(msg) if msg.contains("step file cycle through")),
            "{err}"
        );
    }

    #[test]
    fn set_only_goes_with_vars() {
        let steps =
            resolve_here("s:\n  vars: {A: b}\n  set: {states: {N: '${A}'}, vars: {M: 1}}\n")
                .unwrap();
        let set = steps[0].set.as_ref().unwrap();
        assert_eq!(set.states["N"], Value::from("${A}"));
        assert_eq!(set.vars["M"], Value::from(1));
        for yaml in [
            "s:\n  set: {states: {N: 1}}\n  cmd: [echo]\n",
            "s:\n  set: {states: {N: 1}}\n  allow_failure: true\n",
            "s:\n  set: {states: {N: 1}}\n  if: {this: X, eq: 1}\n  then: {a: {cmd: [echo]}}\n",
            "s:\n  set: {vars: {N: 1}}\n  for_each: X\n  do: {a: {cmd: [echo]}}\n",
            "s:\n  set: {vars: {N: 1}}\n  cwd: w\n",
            "s:\n  set: {vars: {N: 1}}\n  env: {A: b}\n",
        ] {
            let msg = invalid_step(yaml);
            assert!(
                msg.contains("`s` is a `set` step and can only have `vars`"),
                "{yaml}: {msg}"
            );
        }
    }

    #[test]
    fn set_needs_states_or_vars() {
        for yaml in ["s:\n  set: {}\n", "s:\n  set: {states: {}}\n"] {
            let msg = invalid_step(yaml);
            assert!(
                msg.contains("`s`: `set` needs `states` or `vars`"),
                "{yaml}: {msg}"
            );
        }
        assert!(parse_named("s:\n  set: {state: {N: 1}}\n").is_err());
        assert!(parse_named("s:\n  states_update: {N: 1}\n").is_err());
    }

    #[test]
    fn rejects_invalid_condition() {
        let msg = invalid_step(
            "r:\n  if: {this: X, gt: ten}\n  then: {a: {http: {method: GET, url: x}}}\n",
        );
        assert!(
            msg.starts_with("`r`: invalid condition: `gt` needs a number"),
            "{msg}"
        );
    }
}
