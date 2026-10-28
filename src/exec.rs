use std::collections::BTreeMap;
use std::time::Instant;

use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};
use serde_json::{Map, Value, json};

use crate::command::{self, CommandResult, Stdout};
use crate::condition::{self, Condition};
use crate::error::Error;
use crate::project::Project;
use crate::task::{OnFailure, Task};
use crate::{env_file, extract, progress, vars};

/// What one task produced: a finished command, or (with `allow_failure`, its own or an
/// enclosing loop's) the timeout that stopped it.
#[derive(Debug)]
pub enum TaskResult {
    Cmd(CommandResult),
    Errored { error: String },
}

impl TaskResult {
    /// Why the task counts as failed: a non-zero exit, or an error.
    pub fn failure(&self) -> Option<String> {
        match self {
            TaskResult::Cmd(result) if result.exit_code != 0 => {
                Some(format!("exited with code {}", result.exit_code))
            }
            TaskResult::Errored { error } => Some(error.clone()),
            _ => None,
        }
    }
}

/// `{ok, exit_code, stdout, stderr}` for a command, `{ok, error}` for an error; `ok` is
/// whether the task didn't fail.
impl Serialize for TaskResult {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let ok = self.failure().is_none();
        match self {
            TaskResult::Cmd(result) => {
                let mut map = serializer.serialize_map(Some(4))?;
                map.serialize_entry("ok", &ok)?;
                map.serialize_entry("exit_code", &result.exit_code)?;
                map.serialize_entry("stdout", &result.stdout)?;
                map.serialize_entry("stderr", &result.stderr)?;
                map.end()
            }
            TaskResult::Errored { error } => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("ok", &ok)?;
                map.serialize_entry("error", error)?;
                map.end()
            }
        }
    }
}

#[cfg(test)]
impl TaskResult {
    /// A command result with `exit_code` and no output.
    pub(crate) fn cmd(exit_code: i32) -> Self {
        TaskResult::Cmd(CommandResult {
            exit_code,
            stdout: Value::Null,
            stderr: String::new(),
        })
    }
}

pub struct Outcome {
    /// The last task that ran, if any.
    pub result: Option<TaskResult>,
    /// Set when a task failed without `allow_failure`; later tasks did not run.
    pub failed_task: Option<String>,
}

impl Outcome {
    /// Why `failed_task` failed, if a task did.
    pub fn reason(&self) -> Option<String> {
        self.failed_task.as_ref()?;
        self.result.as_ref().and_then(TaskResult::failure)
    }
}

/// Queued work: a task to run with its label, a loop's remaining items, or a `while`
/// loop's next condition check.
enum Work {
    Task { task: Box<Task>, name: String },
    Loop(Box<Loop>),
    While(Box<While>),
}

/// Per remaining item, bind `var` to it and run `body`; then restore `var` to `previous`
/// (`None` unbinds it). Expanded one item at a time, so memory stays O(`body`).
/// With `allow_failure`, a failing `body` task skips to the next item.
struct Loop {
    body: Vec<Task>,
    var: String,
    items: std::vec::IntoIter<Value>,
    previous: Option<Value>,
    allow_failure: bool,
}

impl Loop {
    /// Puts `var` back as it was before the loop.
    fn restore(self, values: &mut Map<String, Value>) {
        match self.previous {
            Some(value) => values.insert(self.var, value),
            None => values.remove(&self.var),
        };
    }
}

/// While `condition` holds, run `body` again; fail once `done` reaches `max`.
/// With `allow_failure`, a failing `body` task skips to the next condition check.
struct While {
    name: String,
    condition: Condition,
    body: Vec<Task>,
    max: Option<u64>,
    done: u64,
    allow_failure: bool,
}

impl Work {
    /// Whether this is a loop whose `allow_failure` skips a failing iteration.
    fn allows_failure(&self) -> bool {
        match self {
            Work::Loop(each) => each.allow_failure,
            Work::While(repeat) => repeat.allow_failure,
            Work::Task { .. } => false,
        }
    }
}

/// Drops the rest of the current iteration of the innermost loop with `allow_failure`,
/// restoring the variables of loops left on the way, so that loop moves on to its next
/// iteration.
fn skip_iteration(stack: &mut Vec<Work>, values: &mut Map<String, Value>) {
    while let Some(work) = stack.last() {
        if work.allows_failure() {
            return;
        }
        if let Some(Work::Loop(each)) = stack.pop() {
            each.restore(values);
        }
    }
}

/// `tasks` as work, in order.
fn queued(tasks: Vec<Task>) -> impl DoubleEndedIterator<Item = Work> {
    tasks.into_iter().map(|task| Work::Task {
        name: task.name.clone(),
        task: Box::new(task),
    })
}

/// Runs the project's `job` tasks in order; later tasks read earlier results as
/// `tasks.NAME`. A branch's chosen `then`/`else` tasks, or a switch's matching case or
/// `default`, run in its place; a loop's `do` tasks run once per list element, with the
/// element bound to its `as` variable, or, for a `while` loop, as long as its condition holds.
/// A failing task inside a loop with `allow_failure` skips the rest of that iteration.
/// The job's `values` (the project's merged in at load) are read as `${NAME}`.
/// With `log`, one JSON line per task, branch or switch decision, loop, and `while` check is
/// printed to stdout. With `verbose`, each command is printed to stderr before it runs.
pub async fn exec(
    mut project: Project,
    job: &str,
    log: bool,
    verbose: bool,
) -> Result<Outcome, Error> {
    let tasks = project.take(job)?;
    let env: Map<String, Value> = match project.env_file.clone() {
        Some(path) => tokio::task::spawn_blocking(move || env_file::load(&path)).await??,
        None => BTreeMap::new(),
    }
    .into_iter()
    .map(|(name, value)| (name, Value::String(value)))
    .collect();

    let mut values: Map<String, Value> = tasks.values.into_iter().collect();
    let mut last = None;
    // Next work last, so nested tasks are pushed in reverse to run in place.
    let mut stack: Vec<Work> = queued(tasks.tasks).rev().collect();
    while let Some(work) = stack.pop() {
        let (mut task, name) = match work {
            Work::Task { task, name } => (*task, name),
            Work::Loop(mut each) => {
                if let Some(item) = each.items.next() {
                    values.insert(each.var.clone(), item);
                    let body: Vec<Work> = queued(each.body.clone()).collect();
                    stack.push(Work::Loop(each));
                    stack.extend(body.into_iter().rev());
                } else {
                    each.restore(&mut values);
                }
                continue;
            }
            Work::While(mut repeat) => {
                let name = repeat.name.clone();
                let to_task_error = |source| Error::Task {
                    name: name.clone(),
                    source: Box::new(source),
                };
                let holds = repeat
                    .condition
                    .evaluate(&|var| lookup(var, &values, &env))
                    .map_err(to_task_error)?;
                if log {
                    progress::emit(job, progress::repeat(&name, repeat.done + 1, holds)).await;
                }
                if holds {
                    if repeat.max == Some(repeat.done) {
                        return Err(to_task_error(Error::While(format!(
                            "still true after {} attempts",
                            repeat.done
                        ))));
                    }
                    repeat.done += 1;
                    let body: Vec<Work> = queued(repeat.body.clone()).collect();
                    stack.push(Work::While(repeat));
                    stack.extend(body.into_iter().rev());
                }
                continue;
            }
        };
        let to_task_error = |source| Error::Task {
            name: name.clone(),
            source: Box::new(source),
        };

        if let Some(condition) = task.condition.take() {
            let taken = condition
                .evaluate(&|var| lookup(var, &values, &env))
                .map_err(to_task_error)?;
            let chosen = if taken { task.then } else { task.otherwise };
            if log {
                progress::emit(job, progress::branch(&name, taken, chosen.len())).await;
            }
            stack.extend(queued(chosen).rev());
            continue;
        }

        if let Some(var) = task.switch.take() {
            let value = lookup(&var, &values, &env)
                .ok_or_else(|| to_task_error(Error::UndefinedVar(var)))?;
            let matched = task
                .cases
                .into_iter()
                .enumerate()
                .find(|(_, case)| condition::loose_eq(&value, &case.value));
            let (case, chosen) = match matched {
                Some((index, case)) => (Some(index + 1), case.body),
                None => (None, task.default),
            };
            if log {
                progress::emit(job, progress::switch(&name, case, chosen.len())).await;
            }
            stack.extend(queued(chosen).rev());
            continue;
        }

        if let Some(list_var) = task.for_each.take() {
            let items = match lookup(&list_var, &values, &env) {
                Some(Value::Array(items)) => items,
                Some(other) => {
                    return Err(to_task_error(Error::Loop(format!(
                        "`{list_var}` is not a list ({other})"
                    ))));
                }
                None => return Err(to_task_error(Error::UndefinedVar(list_var))),
            };
            if log {
                progress::emit(job, progress::for_each(&name, &list_var, items.len())).await;
            }
            let var = task.item_var.unwrap_or_else(|| "ITEM".to_string());
            stack.push(Work::Loop(Box::new(Loop {
                body: task.body,
                previous: values.get(&var).cloned(),
                var,
                items: items.into_iter(),
                allow_failure: task.on_failure == Some(OnFailure::Next),
            })));
            continue;
        }

        if let Some(condition) = task.repeat.take() {
            stack.push(Work::While(Box::new(While {
                name,
                condition,
                body: task.body,
                max: task.attempts,
                done: 0,
                allow_failure: task.on_failure == Some(OnFailure::Next),
            })));
            continue;
        }

        let allow_failure = task.allow_failure;
        let in_allowing_loop = stack.iter().any(Work::allows_failure);
        let allowed = allow_failure || in_allowing_loop;
        let retry = task.retry.take();
        let attempts = retry.as_ref().map_or(1, |retry| retry.attempts);
        let mut attempt = 1;
        let result = loop {
            let last = attempt >= attempts;
            // Before the last attempt, a timeout is only a reason to try again.
            let tolerated = allowed || !last;
            let started = Instant::now();
            let result = run_task(&task, tolerated, verbose, &mut values, &env)
                .await
                .map_err(to_task_error)?;
            if log {
                progress::emit(
                    job,
                    progress::task(&name, &result, started.elapsed(), tolerated),
                )
                .await;
            }
            let Some(retry) = &retry else {
                break result;
            };
            let done = match &retry.until {
                Some(until) => until
                    .evaluate(&|var| lookup(var, &values, &env))
                    .map_err(to_task_error)?,
                None => result.failure().is_none(),
            };
            if done || (last && retry.until.is_none()) {
                break result;
            }
            if last {
                let err = Error::Retry(format!("`until` still false after {attempts} attempts"));
                if !allowed {
                    return Err(to_task_error(err));
                }
                let result = TaskResult::Errored { error: err.chain() };
                record(&name, &result, &mut values);
                break result;
            }
            if let Some(secs) = retry.delay {
                tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
            }
            attempt += 1;
        };
        if result.failure().is_some() && !allow_failure {
            if !in_allowing_loop {
                return Ok(Outcome {
                    result: Some(result),
                    failed_task: Some(name),
                });
            }
            skip_iteration(&mut stack, &mut values);
        }
        last = Some(result);
    }
    Ok(Outcome {
        result: last,
        failed_task: None,
    })
}

/// Runs one task and records its result as `tasks.<name>` in `values`. With `allowed`, a
/// timeout becomes an [`TaskResult::Errored`] result instead of an error. With `verbose`,
/// the command is printed to stderr first.
async fn run_task(
    task: &Task,
    allowed: bool,
    verbose: bool,
    values: &mut Map<String, Value>,
    env: &Map<String, Value>,
) -> Result<TaskResult, Error> {
    let find = |var: &str| lookup(var, values, env);
    let Some(argv) = &task.cmd else {
        return Err(Error::InvalidTask("must have `cmd`".to_string()));
    };
    let result = match execute(argv, task, verbose, &find).await {
        Err(err @ Error::CommandTimeout { .. }) if allowed => {
            TaskResult::Errored { error: err.chain() }
        }
        result => result?,
    };
    record(&task.name, &result, values);
    Ok(result)
}

/// Saves `result` as `tasks.<name>`.
fn record(name: &str, result: &TaskResult, values: &mut Map<String, Value>) {
    if let Value::Object(tasks) = values.entry("tasks").or_insert_with(|| json!({})) {
        tasks.insert(name.to_string(), json!(result));
    }
}

/// Runs `argv`, piped into `task`'s `pipe` stages, with `task`'s `stdin`, `env`, `cwd`,
/// `timeout`, and `capture_stdout` or `stdout_file`. With `verbose`, prints the command to stderr first.
async fn execute(
    argv: &[String],
    task: &Task,
    verbose: bool,
    find: &(dyn Fn(&str) -> Option<Value> + Sync),
) -> Result<TaskResult, Error> {
    let substitute = |text: &str| vars::substitute(text, find);
    let cwd = task
        .cwd
        .as_deref()
        .map(|path| substitute(&path.to_string_lossy()).map(|path| task.cwd_dir.join(path)))
        .transpose()?;
    let stages = std::iter::once(argv)
        .chain(task.pipe.iter().map(Vec::as_slice))
        .map(|argv| argv.iter().map(|arg| substitute(arg)).collect())
        .collect::<Result<Vec<_>, _>>()?;
    if verbose {
        eprintln!("+ [{}] {}", task.name, command::display(&stages));
    }
    let stdin = render(task.stdin.clone(), find)?;
    let mut rendered = BTreeMap::new();
    for (name, value) in &task.env {
        if let Some(text) = render(Some(value.clone()), find)? {
            rendered.insert(name.clone(), text);
        }
    }
    let stdout = match &task.stdout_file {
        Some(path) => {
            let path = task.dir.join(substitute(&path.to_string_lossy())?);
            let file = tokio::fs::File::create(&path)
                .await
                .map_err(|source| Error::Io { path, source })?;
            Stdout::File(file.into_std().await)
        }
        None if task.capture_stdout == Some(false) => Stdout::Discard,
        None => Stdout::Capture,
    };
    let result = command::run(
        &stages,
        stdin.as_deref(),
        &rendered,
        cwd.as_deref(),
        task.timeout,
        stdout,
    )
    .await?;
    Ok(TaskResult::Cmd(result))
}

/// Substitutes `${VAR}` in `value` and turns it into text: a string as-is, anything else as JSON.
fn render(
    value: Option<Value>,
    find: &(dyn Fn(&str) -> Option<Value> + Sync),
) -> Result<Option<String>, Error> {
    let Some(mut value) = value else {
        return Ok(None);
    };
    vars::substitute_value(&mut value, find)?;
    Ok(Some(extract::text(&value)))
}

/// Looks `var` up in `tasks`, loop items and `values` (dot paths such as
/// `tasks.login.stdout.token` included), then the env_file in `env`, then the process environment.
fn lookup(var: &str, values: &Map<String, Value>, env: &Map<String, Value>) -> Option<Value> {
    extract::lookup(values, var)
        .or_else(|| env.get(var).cloned())
        .or_else(|| std::env::var(var).ok().map(Value::String))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::project::Job;

    /// A project with one job `w`, whose fields are `yaml`, with paths relative to a shared dir.
    fn project(yaml: &str) -> Project {
        project_in(&std::env::temp_dir(), yaml)
    }

    /// Like [`project`], with the job's paths relative to `dir`.
    fn project_in(dir: &Path, yaml: &str) -> Project {
        let mut job: Job = serde_yaml::from_str(yaml).unwrap();
        crate::task::resolve(&mut job.tasks, dir).unwrap();
        Project {
            env_file: None,
            jobs: BTreeMap::from([("w".to_string(), job)]),
        }
    }

    async fn exec(project: Project) -> Result<Outcome, Error> {
        super::exec(project, "w", false, false).await
    }

    /// The failing task's name and its error, for a run that must fail in a task.
    async fn task_error(project: Project) -> (String, Error) {
        match exec(project).await {
            Err(Error::Task { name, source }) => (name, *source),
            Err(err) => panic!("expected a task error, got {err}"),
            Ok(_) => panic!("expected a task error, got success"),
        }
    }

    /// Like [`project`], with an env_file in a temporary dir (kept alive by the returned
    /// guard) holding `env`, each value single-quoted so it is read literally.
    fn project_with_env(yaml: &str, env: &[(&str, &str)]) -> (Project, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        let text: String = env
            .iter()
            .map(|(name, value)| format!("{name}='{value}'\n"))
            .collect();
        std::fs::write(&path, text).unwrap();
        let mut project = project(yaml);
        project.env_file = Some(path);
        (project, dir)
    }

    /// A cmd task's fields, indented by `indent`, that append `text` to `acc` in the task's dir.
    fn append(indent: usize, text: &str) -> String {
        let pad = " ".repeat(indent);
        format!("{pad}cmd: [sh, -c, 'printf %s \"$0\" >> acc', '{text}']\n{pad}cwd: .\n")
    }

    /// A first task `seed` that prints `values`, so later tasks read them as
    /// `tasks.seed.stdout.NAME`, keeping their JSON type.
    fn seed(values: Value) -> String {
        format!("tasks:\n  seed:\n    cmd: [echo, '{values}']\n")
    }

    fn stdout(outcome: &Outcome) -> &Value {
        match &outcome.result {
            Some(TaskResult::Cmd(result)) => &result.stdout,
            _ => panic!("expected a cmd result"),
        }
    }

    #[tokio::test]
    async fn substituted_values_stay_one_argument() {
        let (project, _dir) = project_with_env(
            "tasks:\n  e:\n    cmd: [echo, '${X}']\n",
            &[("X", "a; echo injected  b")],
        );
        let outcome = exec(project).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("a; echo injected  b"));
    }

    #[tokio::test]
    async fn task_files_are_recorded_under_their_key() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("greet.yaml"), "cmd: [echo, 'hi file']\n").unwrap();
        let project = project_in(
            dir.path(),
            "tasks:\n  hello: greet.yaml\n  print:\n    cmd: [echo, '${tasks.hello.stdout}!']\n",
        );
        let outcome = exec(project).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("hi file!"));
    }

    #[tokio::test]
    async fn skipped_branch_with_no_task_run_succeeds_without_result() {
        let (project, _dir) = project_with_env(
            "tasks:\n  r:\n    if: {this: X, eq: 1}\n    then: {a: {cmd: [echo, hi]}}\n",
            &[("X", "2")],
        );
        let outcome = exec(project).await.unwrap();
        assert!(outcome.result.is_none());
        assert_eq!(outcome.failed_task, None);
    }

    /// Both runs wait for a 1s timeout, so they run concurrently.
    #[tokio::test]
    async fn timeout_fails_unless_allowed_then_exists_sees_the_error() {
        let allowed = "tasks:\n\
                       \x20 slow:\n    cmd: [sleep, '5']\n    timeout: 1\n    allow_failure: true\n\
                       \x20 check:\n    if: {this: tasks.slow.error, exists: true}\n\
                       \x20   then: {errored: {cmd: [echo, '${tasks.slow.error}']}}\n    else: {ok: {cmd: [echo, ok]}}\n";
        let (allowed, failed) = tokio::join!(
            exec(project(allowed)),
            exec(project(
                "tasks:\n  slow:\n    cmd: [sleep, '5']\n    timeout: 1\n"
            )),
        );
        let outcome = allowed.unwrap();
        assert_eq!(outcome.failed_task, None);
        assert_eq!(
            stdout(&outcome),
            &json!("command `sleep` timed out after 1s")
        );
        let Err(Error::Task { source, .. }) = failed else {
            panic!("expected a task error");
        };
        assert!(matches!(*source, Error::CommandTimeout { .. }), "{source}");
    }

    #[tokio::test]
    async fn branch_runs_chosen_arm() {
        let outcome = exec(
            project(&format!(
                "{}  size:\n    if: {{this: tasks.seed.stdout.N, gt: 3}}\n    then: {{big: {{cmd: [echo, big]}}}}\n    else: {{small: {{cmd: [echo, small]}}}}\n",
                seed(json!({"N": 5}))
            )))
        .await
        .unwrap();
        assert_eq!(stdout(&outcome), &json!("big"));
    }

    /// A job seeded with `N`, then a switch on it that echoes the case it took (cases 200,
    /// "404" and a duplicate 404, plus a default if `default`).
    fn switch_project(n: Value, default: bool) -> Project {
        let pick = |word: &str| format!("{{pick: {{cmd: [echo, {word}]}}}}");
        let default = if default {
            format!("    default: {}\n", pick("other"))
        } else {
            String::new()
        };
        project(&format!(
            "{}\
             \x20 route:\n    switch: tasks.seed.stdout.N\n    cases:\n\
             \x20     200: {}\n\
             \x20     \"404\": {}\n\
             \x20     404: {}\n\
             {default}",
            seed(json!({ "N": n })),
            pick("ok"),
            pick("missing"),
            pick("second"),
        ))
    }

    #[tokio::test]
    async fn switch_runs_first_matching_case() {
        for (n, expected) in [
            (json!(200), "ok"),
            (json!("200"), "ok"),
            (json!(404), "missing"),
        ] {
            let outcome = exec(switch_project(n.clone(), true)).await.unwrap();
            assert_eq!(stdout(&outcome), &json!(expected), "{n}");
        }
    }

    #[tokio::test]
    async fn switch_falls_back_to_default_or_runs_nothing() {
        let outcome = exec(switch_project(json!(500), true)).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("other"));
        let outcome = exec(switch_project(json!(500), false)).await.unwrap();
        assert_eq!(stdout(&outcome), &json!({"N": 500}));
    }

    #[tokio::test]
    async fn switch_on_undefined_var_errors() {
        let (name, err) = task_error(project(
            "tasks:\n  s:\n    switch: NOPE_UNDEFINED\n    cases: {1: {e: {cmd: [echo]}}}\n",
        ))
        .await;
        assert_eq!(name, "s");
        assert!(matches!(err, Error::UndefinedVar(_)), "{err}");
    }

    #[tokio::test]
    async fn for_each_runs_body_per_item_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let outcome = exec(project_in(
            dir.path(),
            &format!(
                "{}\
                     \x20 each:\n    for_each: tasks.seed.stdout.IDS\n    as: ID\n    do:\n\
                     \x20     add:\n{}\
                     \x20 print:\n    cmd: [cat, acc]\n    cwd: .\n",
                seed(json!({"IDS": [1, 2, 3]})),
                append(8, "-${ID}")
            ),
        ))
        .await
        .unwrap();
        assert_eq!(stdout(&outcome), &json!("-1-2-3"));
    }

    #[tokio::test]
    async fn for_each_list_may_be_written_with_braces() {
        let outcome = exec(project(&format!(
            "{}  each:\n    for_each: ${{tasks.seed.stdout.IDS}}\n    do: {{e: {{cmd: [echo, '${{ITEM}}']}}}}\n",
            seed(json!({"IDS": [1, 2]}))
        )))
        .await
        .unwrap();
        assert_eq!(stdout(&outcome), &json!(2));
    }

    #[tokio::test]
    async fn for_each_default_item_with_paths_and_nested_branch() {
        let dir = tempfile::tempdir().unwrap();
        let outcome = exec(project_in(
            dir.path(),
            &format!(
                "tasks:\n\
                 \x20 list:\n    cmd: [echo, '[{{\"id\": 1, \"ok\": true}}, {{\"id\": 2, \"ok\": false}}]']\n\
                 \x20 each:\n    for_each: tasks.list.stdout\n    do:\n\
                 \x20     check:\n        if: {{this: ITEM.ok, eq: true}}\n        then:\n\
                 \x20         plus:\n{}\
                 \x20       else:\n\
                 \x20         minus:\n{}\
                 \x20 print:\n    cmd: [cat, acc]\n    cwd: .\n",
                append(12, "+${ITEM.id}"),
                append(12, "-${ITEM.id}")
            ),
        ))
        .await
        .unwrap();
        assert_eq!(stdout(&outcome), &json!("+1-2"));
    }

    #[tokio::test]
    async fn nested_loops_inside_branch() {
        let dir = tempfile::tempdir().unwrap();
        let outcome = exec(project_in(
            dir.path(),
            &format!(
                "{}\
                 \x20 go:\n    if: {{this: tasks.seed.stdout.GO, eq: true}}\n    then:\n\
                 \x20     rows:\n        for_each: tasks.seed.stdout.ROWS\n        as: ROW\n        do:\n\
                 \x20         cells:\n            for_each: ROW\n            as: CELL\n            do:\n\
                 \x20             add:\n{}\
                 \x20 print:\n    cmd: [cat, acc]\n    cwd: .\n",
                seed(json!({"GO": true, "ROWS": [["a", "b"], ["c"]]})),
                append(16, "${CELL}")
            ),
        ))
        .await
        .unwrap();
        assert_eq!(stdout(&outcome), &json!("abc"));
    }

    #[tokio::test]
    async fn for_each_restores_or_removes_item_var() {
        let outcome = exec(
            project(&format!(
                "{}\
                 \x20 outer:\n    for_each: tasks.seed.stdout.OUTER\n    as: X\n    do:\n\
                 \x20     inner:\n        for_each: tasks.seed.stdout.INNER\n        as: X\n        do: {{e: {{cmd: [echo, '${{X}}']}}}}\n\
                 \x20     print:\n        cmd: [echo, '${{X}}']\n",
                seed(json!({"OUTER": ["before"], "INNER": [1, 2]}))
            )))
        .await
        .unwrap();
        assert_eq!(stdout(&outcome), &json!("before"));

        let (_, err) = task_error(
            project(&format!(
                "{}\
                 \x20 each:\n    for_each: tasks.seed.stdout.IDS\n    as: NOPE_X\n    do: {{e: {{cmd: [echo, '${{NOPE_X}}']}}}}\n\
                 \x20 print:\n    cmd: [echo, '${{NOPE_X}}']\n",
                seed(json!({"IDS": [1]}))
            )))
        .await;
        assert!(
            matches!(err, Error::UndefinedVar(ref v) if v == "NOPE_X"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn for_each_needs_a_defined_list() {
        let yaml = "tasks:\n  each:\n    for_each: NOPE_L\n    do: {e: {cmd: [echo]}}\n";
        let (strings, _dir) = project_with_env(yaml, &[("NOPE_L", "a,b")]);
        let (name, err) = task_error(strings).await;
        assert_eq!(name, "each");
        assert!(matches!(err, Error::Loop(_)), "{err}");
        let (name, err) = task_error(project(yaml)).await;
        assert_eq!(name, "each");
        assert!(matches!(err, Error::UndefinedVar(_)), "{err}");
    }

    /// A job that sets `tasks.count.stdout` to 0, then runs `wait`: `while` it is below 3
    /// (with `attempts`, if given), append `x` to `acc` and set it to `acc`'s length; then `after`
    /// task.
    fn while_project(dir: &Path, start: u32, max: Option<u32>, after: &str) -> Project {
        let max = max.map_or(String::new(), |max| format!("    attempts: {max}\n"));
        project_in(
            dir,
            &format!(
                "tasks:\n  count:\n    cmd: [echo, '{start}']\n\
                 \x20 wait:\n    while: {{this: tasks.count.stdout, lt: 3}}\n{max}    do:\n\
                 \x20     count:\n        cmd: [sh, -c, 'printf x >> acc; wc -c < acc']\n        cwd: .\n\
                 \x20 after:\n{after}"
            ),
        )
    }

    #[tokio::test]
    async fn while_repeats_until_the_condition_is_false() {
        let dir = tempfile::tempdir().unwrap();
        let cat = "    cmd: [cat, acc]\n    cwd: .\n";
        let outcome = exec(while_project(dir.path(), 0, None, cat)).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("xxx"));
        // Reaching `attempts` just as the condition turns false is fine.
        std::fs::remove_file(dir.path().join("acc")).unwrap();
        let outcome = exec(while_project(dir.path(), 0, Some(3), cat))
            .await
            .unwrap();
        assert_eq!(stdout(&outcome), &json!("xxx"));
    }

    #[tokio::test]
    async fn while_that_is_false_at_first_runs_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let echo = "    cmd: [echo, done]\n";
        let outcome = exec(while_project(dir.path(), 5, None, echo))
            .await
            .unwrap();
        assert_eq!(stdout(&outcome), &json!("done"));
        assert!(!dir.path().join("acc").exists());
    }

    #[tokio::test]
    async fn while_fails_when_still_true_after_its_attempts() {
        let dir = tempfile::tempdir().unwrap();
        let echo = "    cmd: [echo, never]\n";
        let (name, err) = task_error(while_project(dir.path(), 0, Some(2), echo)).await;
        assert_eq!(name, "wait");
        assert_eq!(err.to_string(), "while: still true after 2 attempts");
    }

    #[tokio::test]
    async fn stdout_file_gets_the_output_relative_to_the_task_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("out.txt"), "old contents, replaced\n").unwrap();
        let outcome = exec(project_in(
            dir.path(),
            "values: {NAME: out}\n\
             tasks:\n  save:\n    cmd: [echo, '{\"a\": 1}']\n    pipe: [[cat]]\n    stdout_file: '${NAME}.txt'\n",
        ))
        .await
        .unwrap();
        assert_eq!(stdout(&outcome), &Value::Null);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("out.txt")).unwrap(),
            "{\"a\": 1}\n"
        );
    }

    #[tokio::test]
    async fn stdout_file_in_a_missing_directory_errors() {
        let dir = tempfile::tempdir().unwrap();
        let (name, err) = task_error(project_in(
            dir.path(),
            "tasks:\n  save:\n    cmd: [echo]\n    stdout_file: nope/out.txt\n    allow_failure: true\n",
        ))
        .await;
        assert_eq!(name, "save");
        assert!(
            matches!(&err, Error::Io { path, .. } if path.ends_with("nope/out.txt")),
            "{err}"
        );
    }

    /// A job whose `count` task appends `x` to `acc` and prints `acc`'s length, with `fields`
    /// (such as `retry`) added; it exits 1 until the length reaches 3 when `failing`.
    fn retry_project(dir: &Path, failing: bool, fields: &str) -> Project {
        let check = if failing { "; test $n -ge 3" } else { "" };
        project_in(
            dir,
            &format!(
                "tasks:\n  count:\n    cmd: [sh, -c, 'printf x >> acc; n=$(wc -c < acc); echo $n{check}']\n\
                 \x20   cwd: .\n{fields}"
            ),
        )
    }

    #[tokio::test]
    async fn retry_runs_until_the_command_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let fields = "    retry: {attempts: 5}\n";
        let outcome = exec(retry_project(dir.path(), true, fields)).await.unwrap();
        assert_eq!(outcome.failed_task, None);
        assert_eq!(stdout(&outcome), &json!(3));
    }

    #[tokio::test]
    async fn retry_without_until_fails_like_its_last_attempt() {
        let dir = tempfile::tempdir().unwrap();
        let fields = "    retry: {attempts: 2}\n";
        let outcome = exec(retry_project(dir.path(), true, fields)).await.unwrap();
        assert_eq!(outcome.failed_task.as_deref(), Some("count"));
        assert_eq!(outcome.reason().as_deref(), Some("exited with code 1"));
        assert_eq!(stdout(&outcome), &json!(2));
    }

    #[tokio::test]
    async fn retry_runs_until_the_condition_holds() {
        let dir = tempfile::tempdir().unwrap();
        let fields = "    retry: {attempts: 5, until: {this: tasks.count.stdout, gte: 3}}\n";
        let outcome = exec(retry_project(dir.path(), false, fields))
            .await
            .unwrap();
        assert_eq!(stdout(&outcome), &json!(3));
    }

    #[tokio::test]
    async fn retry_with_until_fails_after_its_attempts() {
        let dir = tempfile::tempdir().unwrap();
        let fields = "    retry: {attempts: 2, until: {this: tasks.count.stdout, gte: 3}}\n";
        let (name, err) = task_error(retry_project(dir.path(), false, fields)).await;
        assert_eq!(name, "count");
        assert_eq!(
            err.to_string(),
            "retry: `until` still false after 2 attempts"
        );

        std::fs::remove_file(dir.path().join("acc")).unwrap();
        let fields = format!(
            "{fields}    allow_failure: true\n  after:\n    cmd: [echo, '${{tasks.count.error}}']\n"
        );
        let outcome = exec(retry_project(dir.path(), false, &fields))
            .await
            .unwrap();
        assert_eq!(
            stdout(&outcome),
            &json!("retry: `until` still false after 2 attempts")
        );
    }

    /// The first attempt sleeps past the 1s timeout.
    #[tokio::test]
    async fn retry_tries_again_after_a_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let outcome = exec(project_in(
            dir.path(),
            "tasks:\n  slow:\n    cmd: [sh, -c, 'test -e acc || { touch acc; sleep 5; }; echo ok']\n\
             \x20   cwd: .\n    timeout: 1s\n    retry: {attempts: 2}\n",
        ))
        .await
        .unwrap();
        assert_eq!(stdout(&outcome), &json!("ok"));
    }

    #[tokio::test]
    async fn while_on_undefined_var_errors() {
        let (name, err) = task_error(project(
            "tasks:\n  wait:\n    while: {this: NOPE_W, eq: 1}\n    do: {e: {cmd: [echo]}}\n",
        ))
        .await;
        assert_eq!(name, "wait");
        assert!(
            matches!(err, Error::UndefinedVar(ref v) if v == "NOPE_W"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn failing_task_in_loop_stops_job() {
        let outcome = exec(
            project(&format!(
                "{}\
                 \x20 each:\n    for_each: tasks.seed.stdout.CODES\n    do:\n      check:\n        cmd: [sh, -c, 'exit ${{ITEM}}']\n\
                 \x20 never:\n    cmd: [echo, never]\n",
                seed(json!({"CODES": [0, 5, 0]}))
            )))
        .await
        .unwrap();
        assert_eq!(outcome.failed_task.as_deref(), Some("check"));
        assert_eq!(
            outcome
                .result
                .as_ref()
                .and_then(TaskResult::failure)
                .as_deref(),
            Some("exited with code 5")
        );
    }

    #[tokio::test]
    async fn failing_task_in_on_failure_next_loop_skips_the_iteration() {
        let dir = tempfile::tempdir().unwrap();
        let outcome = exec(project_in(
            dir.path(),
            &format!(
                "{}\
                 \x20 each:\n    for_each: tasks.seed.stdout.IDS\n    on_failure: next\n    do:\n\
                 \x20     check:\n        cmd: [test, '${{ITEM}}', '!=', '2']\n\
                 \x20     add:\n{}\
                 \x20 print:\n    cmd: [sh, -c, 'cat acc; echo \":$0\"', '${{tasks.check.exit_code}}']\n    cwd: .\n",
                seed(json!({"IDS": [1, 2, 3]})),
                append(8, "${ITEM}")
            ),
        ))
        .await
        .unwrap();
        assert_eq!(outcome.failed_task, None);
        assert_eq!(stdout(&outcome), &json!("13:0"));
    }

    #[tokio::test]
    async fn failure_in_inner_loop_skips_to_the_on_failure_next_loop() {
        let dir = tempfile::tempdir().unwrap();
        let outcome = exec(project_in(
            dir.path(),
            &format!(
                "{}\
                 \x20 rows:\n    for_each: tasks.seed.stdout.ROWS\n    as: ROW\n    on_failure: next\n    do:\n\
                 \x20     cells:\n        for_each: ROW\n        as: CELL\n        do:\n\
                 \x20         check:\n            cmd: [test, '${{CELL}}', '!=', b]\n\
                 \x20         add:\n{}\
                 \x20     end:\n{}\
                 \x20 print:\n    if: {{this: CELL, exists: false}}\n    then:\n\
                 \x20     cat:\n        cmd: [cat, acc]\n        cwd: .\n",
                seed(json!({"ROWS": [["a", "b", "x"], ["c", "d"]]})),
                append(12, "${CELL}"),
                append(8, "|")
            ),
        ))
        .await
        .unwrap();
        assert_eq!(stdout(&outcome), &json!("acd|"));
    }

    #[tokio::test]
    async fn failing_task_in_on_failure_next_while_rechecks_the_condition() {
        let dir = tempfile::tempdir().unwrap();
        let outcome = exec(project_in(
            dir.path(),
            &format!(
                "tasks:\n  count:\n    cmd: [echo, '0']\n\
                 \x20 wait:\n    while: {{this: tasks.count.stdout, lt: 3}}\n    attempts: 3\n    on_failure: next\n    do:\n\
                 \x20     count:\n        cmd: [sh, -c, 'printf x >> acc; wc -c < acc']\n        cwd: .\n\
                 \x20     fail:\n        cmd: ['false']\n\
                 \x20     never:\n{}\
                 \x20 print:\n    cmd: [cat, acc]\n    cwd: .\n",
                append(8, "y")
            ),
        ))
        .await
        .unwrap();
        assert_eq!(stdout(&outcome), &json!("xxx"));
    }

    #[tokio::test]
    async fn timeout_in_on_failure_next_loop_is_recorded() {
        let outcome = exec(project(&format!(
            "{}\
             \x20 each:\n    for_each: tasks.seed.stdout.IDS\n    on_failure: next\n    do:\n\
             \x20     slow:\n        cmd: [sleep, '5']\n        timeout: 1\n\
             \x20 print:\n    cmd: [echo, '${{tasks.slow.error}}']\n",
            seed(json!({"IDS": [1]}))
        )))
        .await
        .unwrap();
        assert_eq!(
            stdout(&outcome),
            &json!("command `sleep` timed out after 1s")
        );
    }

    #[tokio::test]
    async fn undefined_var_in_on_failure_next_loop_still_errors() {
        let (name, err) = task_error(project(&format!(
            "{}\
             \x20 each:\n    for_each: tasks.seed.stdout.IDS\n    on_failure: next\n    do:\n\
             \x20     e:\n        cmd: [echo, '${{NOPE_L}}']\n",
            seed(json!({"IDS": [1]}))
        )))
        .await;
        assert_eq!(name, "e");
        assert!(matches!(err, Error::UndefinedVar(_)), "{err}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn cmd_task_does_not_block_other_tasks() {
        let cmd = tokio::spawn(exec(project("tasks:\n  sleep:\n    cmd: [sleep, '1']\n")));
        let started = Instant::now();
        let timer = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            started.elapsed()
        });
        let elapsed = timer.await.unwrap();
        cmd.await.unwrap().unwrap();
        assert!(elapsed.as_millis() < 500, "timer took {elapsed:?}");
    }

    #[tokio::test]
    async fn capture_stdout_false_discards_stdout() {
        let outcome = exec(project(
            "tasks:\n  e:\n    cmd: [echo, hi]\n    capture_stdout: false\n",
        ))
        .await
        .unwrap();
        assert_eq!(stdout(&outcome), &Value::Null);
    }

    #[tokio::test]
    async fn cmd_errors_carry_task_name() {
        let (name, err) = task_error(project(
            "tasks:\n  r:\n    if: {this: NOPE_UNDEFINED, exists: false}\n    then:\n\
                 \x20     inner:\n        cmd: [echo, '${NOPE_UNDEFINED}']\n",
        ))
        .await;
        assert_eq!(name, "inner");
        assert!(
            matches!(err, Error::UndefinedVar(ref v) if v == "NOPE_UNDEFINED"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn named_tasks_are_readable_by_path_and_stdin() {
        let outcome = exec(
            project(
                "tasks:\n\
                 \x20 produce:\n    cmd: [echo, '{\"items\": [1, 2, 3]}']\n\
                 \x20 piped:\n    cmd: [cat]\n    stdin: ${tasks.produce.stdout}\n\
                 \x20 check:\n    if: {this: tasks.piped.stdout.items.2, eq: 3}\n    then:\n\
                 \x20     print:\n        cmd: [echo, 'first=${tasks.produce.stdout.items.0} code=${tasks.piped.exit_code}']\n",
            ))
        .await
        .unwrap();
        assert_eq!(stdout(&outcome), &json!("first=1 code=0"));
    }

    #[tokio::test]
    async fn failed_task_with_allow_failure_is_recorded() {
        let outcome = exec(project(
            "tasks:\n\
                 \x20 check:\n    cmd: [sh, -c, 'echo nope >&2; exit 4']\n    allow_failure: true\n\
                 \x20 print:\n    cmd: [echo, '${tasks.check.exit_code}:${tasks.check.stderr}']\n",
        ))
        .await
        .unwrap();
        assert_eq!(outcome.failed_task, None);
        assert_eq!(stdout(&outcome), &json!("4:nope"));
    }

    #[tokio::test]
    async fn stdin_is_substituted_and_non_strings_become_json() {
        let outcome = exec(
            project(&format!(
                "{}  cat:\n    cmd: [cat]\n    stdin: {{n: '${{tasks.seed.stdout.N}}', s: 'x=${{tasks.seed.stdout.N}}'}}\n",
                seed(json!({"N": 5}))
            )))
        .await
        .unwrap();
        assert_eq!(stdout(&outcome), &json!({"n": 5, "s": "x=5"}));
    }

    #[tokio::test]
    async fn pipe_stages_are_substituted_and_fed_stdin() {
        let outcome = exec(
            project(&format!(
                "{}  pipe:\n    cmd: [cat]\n    pipe: [[tr, a, '${{tasks.seed.stdout.TO}}'], [rev]]\n    stdin: abc\n",
                seed(json!({"TO": "z"}))
            )))
        .await
        .unwrap();
        assert_eq!(stdout(&outcome), &json!("cbz"));
    }

    #[tokio::test]
    async fn env_is_substituted_and_non_strings_become_json() {
        let outcome = exec(
            project(&format!(
                "{}  env:\n    cmd: [sh, -c, 'printf \"%s|%s\" \"$A\" \"$B\"']\n    env: {{A: 'x=${{tasks.seed.stdout.N}}', B: {{n: '${{tasks.seed.stdout.N}}'}}}}\n",
                seed(json!({"N": 5}))
            )))
        .await
        .unwrap();
        assert_eq!(stdout(&outcome), &json!("x=5|{\"n\":5}"));
    }

    #[tokio::test]
    async fn cmd_writes_then_reads_a_file_relative_to_project_dir() {
        let dir = tempfile::tempdir().unwrap();
        let project = project_in(
            dir.path(),
            &format!(
                "{}\
                 \x20 write:\n    cmd: [sh, -c, 'cat > \"$1\"', _, '${{tasks.seed.stdout.NAME}}.json']\n    stdin: {{n: '${{tasks.seed.stdout.N}}'}}\n    cwd: .\n\
                 \x20 read:\n    cmd: [cat, out.json]\n    cwd: .\n\
                 \x20 print:\n    cmd: [echo, '${{tasks.read.stdout.n}}']\n",
                seed(json!({"NAME": "out", "N": 7}))
            ),
        );
        let outcome = exec(project).await.unwrap();
        assert_eq!(stdout(&outcome), &json!(7));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("out.json")).unwrap(),
            "{\"n\":7}"
        );
    }

    #[tokio::test]
    async fn cmd_cwd_substitutes_vars() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("work")).unwrap();
        let project = project_in(
            dir.path(),
            &format!(
                "{}\
                 \x20 rel:\n    cmd: [touch, rel]\n    cwd: '${{tasks.seed.stdout.SUB}}'\n\
                 \x20 abs:\n    cmd: [touch, abs]\n    cwd: '${{tasks.seed.stdout.ABS}}'\n",
                seed(json!({"SUB": "work", "ABS": other.path()}))
            ),
        );
        exec(project).await.unwrap();
        assert!(dir.path().join("work/rel").exists());
        assert!(other.path().join("abs").exists());
    }

    #[tokio::test]
    async fn condition_compares_loop_item_with_value() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = format!(
            "values: {{LATEST: 2}}\n{}\
             \x20 each:\n    for_each: tasks.seed.stdout.IDS\n    as: ID\n    do:\n\
             \x20     check:\n        if: {{this: ID, gt: '${{LATEST}}'}}\n        then:\n\
             \x20         plus:\n{}\
             \x20       else:\n\
             \x20         minus:\n{}\
             \x20 print:\n    cmd: [cat, acc]\n    cwd: .\n",
            seed(json!({"IDS": [1, 2, 3, 4]})),
            append(12, "+${ID}"),
            append(12, "-${ID}")
        );
        let outcome = exec(project_in(dir.path(), &yaml)).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("-1-2+3+4"));
    }

    #[tokio::test]
    async fn env_file_beats_the_process_environment() {
        let yaml = "tasks:\n  e:\n    cmd: [echo, '${CARGO_PKG_NAME}']\n";
        let outcome = exec(project(yaml)).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("rocky"));
        let (project, _dir) = project_with_env(yaml, &[("CARGO_PKG_NAME", "rb")]);
        let outcome = exec(project).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("rb"));
    }

    #[tokio::test]
    async fn loop_item_beats_the_env_file() {
        let (project, _dir) = project_with_env(
            &format!(
                "{}\
                 \x20 each:\n    for_each: tasks.seed.stdout.IDS\n    as: ID\n    do:\n\
                 \x20     e:\n        cmd: [echo, '${{ID}}']\n",
                seed(json!({"IDS": [1]}))
            ),
            &[("ID", "shadow")],
        );
        let outcome = exec(project).await.unwrap();
        assert_eq!(stdout(&outcome), &json!(1));
    }

    #[tokio::test]
    async fn values_keep_their_type_and_are_not_expanded() {
        let project = project(
            "values: {A: a, B: 1, CONF: {n: 2, tags: [x]}, LIT: '${UNDEFINED}'}\n\
             tasks:\n  e:\n    cmd: [cat]\n    stdin: {text: '${A} ${B} ${CONF.n} ${LIT}', conf: '${CONF}'}\n",
        );
        let outcome = exec(project).await.unwrap();
        assert_eq!(
            stdout(&outcome),
            &json!({"text": "a 1 2 ${UNDEFINED}", "conf": {"n": 2, "tags": ["x"]}})
        );
    }

    #[tokio::test]
    async fn values_beat_the_env_file_and_loop_items_beat_values() {
        let (project, _dir) = project_with_env(
            &format!(
                "values: {{ID: value, TOKEN: value}}\n{}\
                 \x20 each:\n    for_each: tasks.seed.stdout.IDS\n    as: ID\n    do:\n\
                 \x20     e:\n        cmd: [echo, '${{ID}}']\n\
                 \x20 after:\n    cmd: [echo, '${{ID}}-${{TOKEN}}']\n",
                seed(json!({"IDS": [1]}))
            ),
            &[("ID", "env"), ("TOKEN", "env")],
        );
        let outcome = exec(project).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("value-value"));
    }

    #[tokio::test]
    async fn default_cwd_is_the_current_directory_not_the_project_dir() {
        let dir = tempfile::tempdir().unwrap();
        let outcome = exec(project_in(dir.path(), "tasks:\n  e:\n    cmd: [pwd]\n"))
            .await
            .unwrap();
        let printed = std::path::PathBuf::from(stdout(&outcome).as_str().unwrap());
        assert_eq!(
            printed.canonicalize().unwrap(),
            std::env::current_dir().unwrap().canonicalize().unwrap()
        );
    }

    #[tokio::test]
    async fn switch_case_keys_are_not_expanded() {
        let yaml = "tasks:\n  s:\n    switch: V\n    cases: {'${X}': {lit: {cmd: [echo, literal]}}}\n\
                    \x20   default: {other: {cmd: [echo, other]}}\n";
        for (v, expected) in [("a", "other"), ("${X}", "literal")] {
            let (project, _dir) = project_with_env(yaml, &[("X", "a"), ("V", v)]);
            let outcome = exec(project).await.unwrap();
            assert_eq!(stdout(&outcome), &json!(expected), "{v}");
        }
    }

    #[test]
    fn failure_is_a_non_zero_exit_or_an_error() {
        assert_eq!(TaskResult::cmd(0).failure(), None);
        assert_eq!(
            TaskResult::cmd(3).failure().as_deref(),
            Some("exited with code 3")
        );
        let errored = TaskResult::Errored {
            error: "timed out".to_string(),
        };
        assert_eq!(errored.failure().as_deref(), Some("timed out"));
    }

    #[test]
    fn results_say_whether_they_are_ok() {
        assert_eq!(
            json!(TaskResult::cmd(0)),
            json!({"ok": true, "exit_code": 0, "stdout": null, "stderr": ""})
        );
        assert_eq!(json!(TaskResult::cmd(3))["ok"], json!(false));
        let errored = TaskResult::Errored {
            error: "timed out".to_string(),
        };
        assert_eq!(json!(errored), json!({"ok": false, "error": "timed out"}));
    }

    #[tokio::test]
    async fn later_tasks_read_ok() {
        let outcome = exec(project(
            "tasks:\n  bad:\n    cmd: [sh, -c, 'exit 3']\n    allow_failure: true\n\
             \x20 good:\n    cmd: [echo]\n\
             \x20 report:\n    cmd: [echo, '${tasks.bad.ok} ${tasks.good.ok}']\n",
        ))
        .await
        .unwrap();
        assert_eq!(stdout(&outcome), &json!("false true"));
    }
}
