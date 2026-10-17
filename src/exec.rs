use std::collections::BTreeMap;
use std::time::Instant;

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::command::{self, CommandResult};
use crate::error::Error;
use crate::project::Project;
use crate::request::RequestSpec;
use crate::response::Response;
use crate::step::Step;
use crate::{condition, env_file, extract, http, progress, state, vars};

/// What one step produced: an HTTP response, a finished command, or (with `allow_failure`)
/// the timeout or network error that stopped it.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum StepResult {
    Http(Response),
    Cmd(CommandResult),
    Errored { error: String },
}

impl StepResult {
    /// Why the step counts as failed: HTTP 4xx/5xx, a non-zero exit, or an error.
    pub fn failure(&self) -> Option<String> {
        match self {
            StepResult::Http(response) if response.status >= 400 => {
                Some(format!("returned HTTP {}", response.status))
            }
            StepResult::Cmd(result) if result.exit_code != 0 => {
                Some(format!("exited with code {}", result.exit_code))
            }
            StepResult::Errored { error } => Some(error.clone()),
            _ => None,
        }
    }
}

#[cfg(test)]
impl StepResult {
    /// An HTTP result with `status`, no headers and no body.
    pub(crate) fn http(status: u16) -> Self {
        StepResult::Http(Response {
            status,
            headers: BTreeMap::new(),
            body: Value::Null,
            body_encoding: None,
        })
    }

    /// A command result with `exit_code` and no output.
    pub(crate) fn cmd(exit_code: i32) -> Self {
        StepResult::Cmd(CommandResult {
            exit_code,
            stdout: Value::Null,
            stderr: String::new(),
        })
    }
}

pub struct Outcome {
    /// The last step that ran, if any.
    pub result: Option<StepResult>,
    /// Set when a step failed without `allow_failure`; later steps did not run.
    pub failed_step: Option<String>,
}

/// Queued work: a step to run with its label, or a loop's remaining items.
enum Work {
    Step { step: Box<Step>, name: String },
    Loop(Box<Loop>),
}

/// Per remaining item, bind `var` to it and run `body`; then restore `var` to `previous`
/// (`None` unbinds it). Expanded one item at a time, so memory stays O(`body`).
struct Loop {
    body: Vec<Step>,
    var: String,
    items: std::vec::IntoIter<Value>,
    previous: Option<Value>,
}

/// `steps` as work, in order.
fn queued(steps: Vec<Step>) -> impl DoubleEndedIterator<Item = Work> {
    steps.into_iter().map(|step| Work::Step {
        name: step.name.clone(),
        step: Box::new(step),
    })
}

/// Runs the project's `job` steps in order; later steps read earlier results as
/// `steps.NAME`. A branch's chosen `then`/`else` steps, or a switch's matching case or
/// `default`, run in its place; a loop's `do` steps run once per list element, with the
/// element bound to its `as` variable.
/// The project's `values`, then the job's (which win), are read as `${NAME}`.
/// The job's `states` defaults are read from `states/<job>.json` next to the project (created from
/// the defaults if missing) as `states.NAME`, and each `set.states` writes it back right away.
/// With `log`, one JSON line per step, branch or switch decision, and loop is printed to stdout.
pub async fn exec(mut project: Project, job: &str, log: bool) -> Result<Outcome, Error> {
    let steps = project.take(job)?;
    let env: Map<String, Value> = match project.env_file.clone() {
        Some(path) => tokio::task::spawn_blocking(move || env_file::load(&path)).await??,
        None => BTreeMap::new(),
    }
    .into_iter()
    .map(|(name, value)| (name, Value::String(value)))
    .collect();

    let mut values: Map<String, Value> = project.values.into_iter().chain(steps.values).collect();
    let state_path = project.dir.join("states").join(format!("{job}.json"));
    let mut state = Map::new();
    if !steps.states.is_empty() {
        let (path, defaults) = (state_path.clone(), steps.states.clone());
        state = tokio::task::spawn_blocking(move || state::load(&path, &defaults)).await??;
        values.insert("states".to_string(), Value::Object(state.clone()));
    }
    let mut last = None;
    let mut clients = http::Clients::default();
    // Next work last, so nested steps are pushed in reverse to run in place.
    let mut stack: Vec<Work> = queued(steps.steps).rev().collect();
    while let Some(work) = stack.pop() {
        let (mut step, name) = match work {
            Work::Step { step, name } => (*step, name),
            Work::Loop(mut each) => {
                if let Some(item) = each.items.next() {
                    values.insert(each.var.clone(), item);
                    let body: Vec<Work> = queued(each.body.clone()).collect();
                    stack.push(Work::Loop(each));
                    stack.extend(body.into_iter().rev());
                } else {
                    match each.previous {
                        Some(value) => values.insert(each.var, value),
                        None => values.remove(&each.var),
                    };
                }
                continue;
            }
        };
        let to_step_error = |source| Error::Step {
            name: name.clone(),
            source: Box::new(source),
        };

        if let Some(set) = step.set.take() {
            let mut states = Map::new();
            for (var, mut value) in set.states {
                vars::substitute_value(&mut value, &|var| lookup(var, &values, &env))
                    .map_err(to_step_error)?;
                states.insert(var, value);
            }
            let mut saved = state.clone();
            saved.extend(states.clone());
            state::save(&state_path, &saved).map_err(to_step_error)?;
            state = saved;
            values.insert("states".to_string(), Value::Object(state.clone()));
            // Logged once the states are saved, so a failed save isn't logged as set.
            if log {
                progress::emit(job, progress::set(&name, &states)).await;
            }
            continue;
        }

        if let Some(condition) = step.condition.take() {
            let taken = condition
                .evaluate(&|var| lookup(var, &values, &env))
                .map_err(to_step_error)?;
            let chosen = if taken { step.then } else { step.otherwise };
            if log {
                progress::emit(job, progress::branch(&name, taken, chosen.len())).await;
            }
            stack.extend(queued(chosen).rev());
            continue;
        }

        if let Some(var) = step.switch.take() {
            let value = lookup(&var, &values, &env)
                .ok_or_else(|| to_step_error(Error::UndefinedVar(var)))?;
            let matched = step
                .cases
                .into_iter()
                .enumerate()
                .find(|(_, case)| condition::loose_eq(&value, &case.value));
            let (case, chosen) = match matched {
                Some((index, case)) => (Some(index + 1), case.body),
                None => (None, step.default),
            };
            if log {
                progress::emit(job, progress::switch(&name, case, chosen.len())).await;
            }
            stack.extend(queued(chosen).rev());
            continue;
        }

        if let Some(list_var) = step.for_each.take() {
            let items = match lookup(&list_var, &values, &env) {
                Some(Value::Array(items)) => items,
                Some(other) => {
                    return Err(to_step_error(Error::Loop(format!(
                        "`{list_var}` is not a list ({other})"
                    ))));
                }
                None => return Err(to_step_error(Error::UndefinedVar(list_var))),
            };
            if log {
                progress::emit(job, progress::for_each(&name, &list_var, items.len())).await;
            }
            let var = step.item_var.unwrap_or_else(|| "ITEM".to_string());
            stack.push(Work::Loop(Box::new(Loop {
                body: step.body,
                previous: values.get(&var).cloned(),
                var,
                items: items.into_iter(),
            })));
            continue;
        }

        let allow_failure = step.allow_failure;
        let started = Instant::now();
        let result = run_step(step, &mut values, &env, &mut clients)
            .await
            .map_err(to_step_error)?;
        if log {
            progress::emit(
                job,
                progress::step(&name, &result, started.elapsed(), allow_failure),
            )
            .await;
        }
        if result.failure().is_some() && !allow_failure {
            return Ok(Outcome {
                result: Some(result),
                failed_step: Some(name),
            });
        }
        last = Some(result);
    }
    Ok(Outcome {
        result: last,
        failed_step: None,
    })
}

/// Runs one step and records its result as `steps.<name>` in `values`. With `allow_failure`, a
/// timeout or network error becomes an [`StepResult::Errored`] result instead of an error.
async fn run_step(
    mut step: Step,
    values: &mut Map<String, Value>,
    env: &Map<String, Value>,
    clients: &mut http::Clients,
) -> Result<StepResult, Error> {
    let request = step.http.take();
    let result = match perform(&step, request, values, env, clients).await {
        // A request that couldn't be built (a bad URL or header) is a mistake in the step,
        // not a network error.
        Err(err @ (Error::Timeout(_) | Error::CommandTimeout { .. }))
        | Err(err @ Error::Http(_))
            if step.allow_failure && !matches!(&err, Error::Http(e) if e.is_builder()) =>
        {
            StepResult::Errored { error: err.chain() }
        }
        result => result?,
    };
    if let Value::Object(steps) = values.entry("steps").or_insert_with(|| json!({})) {
        steps.insert(step.name.clone(), json!(result));
    }
    Ok(result)
}

/// Performs an HTTP (`request`) or cmd step.
async fn perform(
    step: &Step,
    request: Option<RequestSpec>,
    values: &Map<String, Value>,
    env: &Map<String, Value>,
    clients: &mut http::Clients,
) -> Result<StepResult, Error> {
    let find = |var: &str| lookup(var, values, env);
    match (request, &step.cmd) {
        (Some(spec), None) => send(spec, step.timeout, &find, clients).await,
        (None, Some(argv)) => execute(argv, step, &find).await,
        _ => Err(Error::InvalidStep(
            "must have exactly one of `http` or `cmd`".to_string(),
        )),
    }
}

/// Sends the request, stopping it after `timeout` seconds (default 30).
async fn send(
    spec: RequestSpec,
    timeout: Option<u64>,
    find: &(dyn Fn(&str) -> Option<Value> + Sync),
    clients: &mut http::Clients,
) -> Result<StepResult, Error> {
    let spec = vars::apply_with(spec, find)?;
    let response = http::send(clients, &spec, timeout).await?;
    Ok(StepResult::Http(response))
}

/// Runs `argv` with `step`'s `stdin`, `env`, `cwd`, `timeout` and `capture_stdout`.
async fn execute(
    argv: &[String],
    step: &Step,
    find: &(dyn Fn(&str) -> Option<Value> + Sync),
) -> Result<StepResult, Error> {
    let substitute = |text: &str| vars::substitute(text, find);
    let cwd = step
        .cwd
        .as_deref()
        .map(|path| substitute(&path.to_string_lossy()).map(|path| step.dir.join(path)))
        .transpose()?;
    let argv = argv
        .iter()
        .map(|arg| substitute(arg))
        .collect::<Result<Vec<_>, _>>()?;
    let stdin = render(step.stdin.clone(), find)?;
    let mut rendered = BTreeMap::new();
    for (name, value) in &step.env {
        if let Some(text) = render(Some(value.clone()), find)? {
            rendered.insert(name.clone(), text);
        }
    }
    let result = command::run(
        &argv,
        stdin.as_deref(),
        &rendered,
        cwd.as_deref(),
        step.timeout,
        step.capture_stdout.unwrap_or(true),
    )
    .await?;
    Ok(StepResult::Cmd(result))
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

/// Looks `var` up in `steps`, `states`, loop items and `values` (dot paths such as
/// `steps.login.body.token` included), then the env_file in `env`, then the process environment.
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

    /// A project with one job `w`, whose fields are `yaml`. Its dir is shared, so the
    /// job may not declare `states`; use [`project_in`] with a temporary dir for those.
    fn project(yaml: &str) -> Project {
        let project = project_in(&std::env::temp_dir(), yaml);
        assert!(project.jobs["w"].states.is_empty(), "{yaml}");
        project
    }

    /// Like [`project`], with the job's paths relative to `dir`.
    fn project_in(dir: &Path, yaml: &str) -> Project {
        let mut job: Job = serde_yaml::from_str(yaml).unwrap();
        crate::step::resolve(&mut job.steps, dir).unwrap();
        Project {
            dir: dir.to_path_buf(),
            values: BTreeMap::new(),
            env_file: None,
            schedules: BTreeMap::new(),
            jobs: BTreeMap::from([("w".to_string(), job)]),
        }
    }

    async fn exec(project: Project) -> Result<Outcome, Error> {
        super::exec(project, "w", false).await
    }

    /// The failing step's name and its error, for a run that must fail in a step.
    async fn step_error(project: Project) -> (String, Error) {
        match exec(project).await {
            Err(Error::Step { name, source }) => (name, *source),
            Err(err) => panic!("expected a step error, got {err}"),
            Ok(_) => panic!("expected a step error, got success"),
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

    /// Makes `dir` writable again when dropped, so a test that took write access away
    /// still leaves a removable temp dir if it fails.
    struct Writable<'a>(&'a Path);

    impl Drop for Writable<'_> {
        fn drop(&mut self) {
            let mode = std::os::unix::fs::PermissionsExt::from_mode(0o755);
            let _ = std::fs::set_permissions(self.0, mode);
        }
    }

    /// A first step `seed` that prints `values`, so later steps read them as
    /// `steps.seed.stdout.NAME`, keeping their JSON type.
    fn seed(values: Value) -> String {
        format!("steps:\n  seed:\n    cmd: [echo, '{values}']\n")
    }

    fn stdout(outcome: &Outcome) -> &Value {
        match &outcome.result {
            Some(StepResult::Cmd(result)) => &result.stdout,
            _ => panic!("expected a cmd result"),
        }
    }

    #[tokio::test]
    async fn substituted_values_stay_one_argument() {
        let (project, _dir) = project_with_env(
            "steps:\n  e:\n    cmd: [echo, '${X}']\n",
            &[("X", "a; echo injected  b")],
        );
        let outcome = exec(project).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("a; echo injected  b"));
    }

    #[tokio::test]
    async fn step_files_are_recorded_under_their_key() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("greet.yaml"), "cmd: [echo, 'hi file']\n").unwrap();
        let project = project_in(
            dir.path(),
            "steps:\n  hello: greet.yaml\n  print:\n    cmd: [echo, '${steps.hello.stdout}!']\n",
        );
        let outcome = exec(project).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("hi file!"));
    }

    #[tokio::test]
    async fn skipped_branch_with_no_step_run_succeeds_without_result() {
        let (project, _dir) = project_with_env(
            "steps:\n  r:\n    if: {this: X, eq: 1}\n    then: {a: {cmd: [echo, hi]}}\n",
            &[("X", "2")],
        );
        let outcome = exec(project).await.unwrap();
        assert!(outcome.result.is_none());
        assert_eq!(outcome.failed_step, None);
    }

    /// Both runs wait for a 1s timeout, so they run concurrently.
    #[tokio::test]
    async fn timeout_fails_unless_allowed_then_exists_sees_the_error() {
        let allowed = "steps:\n\
                       \x20 slow:\n    cmd: [sleep, '5']\n    timeout: 1\n    allow_failure: true\n\
                       \x20 check:\n    if: {this: steps.slow.error, exists: true}\n\
                       \x20   then: {errored: {cmd: [echo, '${steps.slow.error}']}}\n    else: {ok: {cmd: [echo, ok]}}\n";
        let (allowed, failed) = tokio::join!(
            exec(project(allowed)),
            exec(project(
                "steps:\n  slow:\n    cmd: [sleep, '5']\n    timeout: 1\n"
            )),
        );
        let outcome = allowed.unwrap();
        assert_eq!(outcome.failed_step, None);
        assert_eq!(
            stdout(&outcome),
            &json!("command `sleep` timed out after 1s")
        );
        let Err(Error::Step { source, .. }) = failed else {
            panic!("expected a step error");
        };
        assert!(matches!(*source, Error::CommandTimeout { .. }), "{source}");
    }

    #[tokio::test]
    async fn branch_runs_chosen_arm() {
        let outcome = exec(
            project(&format!(
                "{}  size:\n    if: {{this: steps.seed.stdout.N, gt: 3}}\n    then: {{big: {{cmd: [echo, big]}}}}\n    else: {{small: {{cmd: [echo, small]}}}}\n",
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
             \x20 route:\n    switch: steps.seed.stdout.N\n    cases:\n\
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
        let (name, err) = step_error(project(
            "steps:\n  s:\n    switch: NOPE_UNDEFINED\n    cases: {1: {e: {cmd: [echo]}}}\n",
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
                "states: {{ACC: start}}\n{}\
                     \x20 each:\n    for_each: steps.seed.stdout.IDS\n    as: ID\n    do:\n\
                     \x20     add:\n        set: {{states: {{ACC: '${{states.ACC}}-${{ID}}'}}}}\n\
                     \x20 print:\n    cmd: [echo, '${{states.ACC}}']\n",
                seed(json!({"IDS": [1, 2, 3]}))
            ),
        ))
        .await
        .unwrap();
        assert_eq!(stdout(&outcome), &json!("start-1-2-3"));
    }

    #[tokio::test]
    async fn for_each_default_item_with_paths_and_nested_branch() {
        let dir = tempfile::tempdir().unwrap();
        let outcome = exec(project_in(
            dir.path(),
            "states: {ACC: ''}\nsteps:\n\
             \x20 list:\n    cmd: [echo, '[{\"id\": 1, \"ok\": true}, {\"id\": 2, \"ok\": false}]']\n\
             \x20 each:\n    for_each: steps.list.stdout\n    do:\n\
             \x20     check:\n        if: {this: ITEM.ok, eq: true}\n        then:\n\
             \x20         plus:\n            set: {states: {ACC: '${states.ACC}+${ITEM.id}'}}\n        else:\n\
             \x20         minus:\n            set: {states: {ACC: '${states.ACC}-${ITEM.id}'}}\n\
             \x20 print:\n    cmd: [echo, '${states.ACC}']\n",
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
                "states: {{ACC: ''}}\n{}\
                 \x20 go:\n    if: {{this: steps.seed.stdout.GO, eq: true}}\n    then:\n\
                 \x20     rows:\n        for_each: steps.seed.stdout.ROWS\n        as: ROW\n        do:\n\
                 \x20         cells:\n            for_each: ROW\n            as: CELL\n            do:\n\
                 \x20             add:\n                set: {{states: {{ACC: '${{states.ACC}}${{CELL}}'}}}}\n\
                 \x20 print:\n    cmd: [echo, '${{states.ACC}}']\n",
                seed(json!({"GO": true, "ROWS": [["a", "b"], ["c"]]}))
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
                 \x20 outer:\n    for_each: steps.seed.stdout.OUTER\n    as: X\n    do:\n\
                 \x20     inner:\n        for_each: steps.seed.stdout.INNER\n        as: X\n        do: {{e: {{cmd: [echo, '${{X}}']}}}}\n\
                 \x20     print:\n        cmd: [echo, '${{X}}']\n",
                seed(json!({"OUTER": ["before"], "INNER": [1, 2]}))
            )))
        .await
        .unwrap();
        assert_eq!(stdout(&outcome), &json!("before"));

        let (_, err) = step_error(
            project(&format!(
                "{}\
                 \x20 each:\n    for_each: steps.seed.stdout.IDS\n    as: NOPE_X\n    do: {{e: {{cmd: [echo, '${{NOPE_X}}']}}}}\n\
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
        let yaml = "steps:\n  each:\n    for_each: NOPE_L\n    do: {e: {cmd: [echo]}}\n";
        let (strings, _dir) = project_with_env(yaml, &[("NOPE_L", "a,b")]);
        let (name, err) = step_error(strings).await;
        assert_eq!(name, "each");
        assert!(matches!(err, Error::Loop(_)), "{err}");
        let (name, err) = step_error(project(yaml)).await;
        assert_eq!(name, "each");
        assert!(matches!(err, Error::UndefinedVar(_)), "{err}");
    }

    #[tokio::test]
    async fn failing_step_in_loop_stops_job() {
        let outcome = exec(
            project(&format!(
                "{}\
                 \x20 each:\n    for_each: steps.seed.stdout.CODES\n    do:\n      check:\n        cmd: [sh, -c, 'exit ${{ITEM}}']\n\
                 \x20 never:\n    cmd: [echo, never]\n",
                seed(json!({"CODES": [0, 5, 0]}))
            )))
        .await
        .unwrap();
        assert_eq!(outcome.failed_step.as_deref(), Some("check"));
        assert_eq!(
            outcome
                .result
                .as_ref()
                .and_then(StepResult::failure)
                .as_deref(),
            Some("exited with code 5")
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn cmd_step_does_not_block_other_tasks() {
        let cmd = tokio::spawn(exec(project("steps:\n  sleep:\n    cmd: [sleep, '1']\n")));
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
            "steps:\n  e:\n    cmd: [echo, hi]\n    capture_stdout: false\n",
        ))
        .await
        .unwrap();
        assert_eq!(stdout(&outcome), &Value::Null);
    }

    #[tokio::test]
    async fn cmd_errors_carry_step_name() {
        let (name, err) = step_error(project(
            "steps:\n  r:\n    if: {this: NOPE_UNDEFINED, exists: false}\n    then:\n\
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
    async fn named_steps_are_readable_by_path_and_stdin() {
        let outcome = exec(
            project(
                "steps:\n\
                 \x20 produce:\n    cmd: [echo, '{\"items\": [1, 2, 3]}']\n\
                 \x20 piped:\n    cmd: [cat]\n    stdin: ${steps.produce.stdout}\n\
                 \x20 check:\n    if: {this: steps.piped.stdout.items.2, eq: 3}\n    then:\n\
                 \x20     print:\n        cmd: [echo, 'first=${steps.produce.stdout.items.0} code=${steps.piped.exit_code}']\n",
            ))
        .await
        .unwrap();
        assert_eq!(stdout(&outcome), &json!("first=1 code=0"));
    }

    #[tokio::test]
    async fn failed_step_with_allow_failure_is_recorded() {
        let outcome = exec(project(
            "steps:\n\
                 \x20 check:\n    cmd: [sh, -c, 'echo nope >&2; exit 4']\n    allow_failure: true\n\
                 \x20 print:\n    cmd: [echo, '${steps.check.exit_code}:${steps.check.stderr}']\n",
        ))
        .await
        .unwrap();
        assert_eq!(outcome.failed_step, None);
        assert_eq!(stdout(&outcome), &json!("4:nope"));
    }

    #[tokio::test]
    async fn stdin_is_substituted_and_non_strings_become_json() {
        let outcome = exec(
            project(&format!(
                "{}  cat:\n    cmd: [cat]\n    stdin: {{n: '${{steps.seed.stdout.N}}', s: 'x=${{steps.seed.stdout.N}}'}}\n",
                seed(json!({"N": 5}))
            )))
        .await
        .unwrap();
        assert_eq!(stdout(&outcome), &json!({"n": 5, "s": "x=5"}));
    }

    #[tokio::test]
    async fn env_is_substituted_and_non_strings_become_json() {
        let outcome = exec(
            project(&format!(
                "{}  env:\n    cmd: [sh, -c, 'printf \"%s|%s\" \"$A\" \"$B\"']\n    env: {{A: 'x=${{steps.seed.stdout.N}}', B: {{n: '${{steps.seed.stdout.N}}'}}}}\n",
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
                 \x20 write:\n    cmd: [sh, -c, 'cat > \"$1\"', _, '${{steps.seed.stdout.NAME}}.json']\n    stdin: {{n: '${{steps.seed.stdout.N}}'}}\n    cwd: .\n\
                 \x20 read:\n    cmd: [cat, out.json]\n    cwd: .\n\
                 \x20 print:\n    cmd: [echo, '${{steps.read.stdout.n}}']\n",
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
                 \x20 rel:\n    cmd: [touch, rel]\n    cwd: '${{steps.seed.stdout.SUB}}'\n\
                 \x20 abs:\n    cmd: [touch, abs]\n    cwd: '${{steps.seed.stdout.ABS}}'\n",
                seed(json!({"SUB": "work", "ABS": other.path()}))
            ),
        );
        exec(project).await.unwrap();
        assert!(dir.path().join("work/rel").exists());
        assert!(other.path().join("abs").exists());
    }

    #[tokio::test]
    async fn binary_body_encoding_is_recorded() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0xff, 0xfe]))
            .mount(&server)
            .await;
        let project = project(&format!(
            "steps:\n  img:\n    http: {{method: GET, url: \"{}\"}}\n  print:\n    cmd: [echo, \"${{steps.img.body_encoding}}\"]\n",
            server.uri()
        ));
        let outcome = exec(project).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("base64"));
    }

    /// `states/w.json` in `dir`, parsed.
    fn state_file(dir: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(dir.join("states/w.json")).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn state_persists_across_runs() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "states: {COUNT: 0, KEEP: k}\n\
                    steps:\n\
                    \x20 inc:\n    cmd: [expr, '${states.COUNT}', +, '1']\n\
                    \x20 save:\n    set: {states: {COUNT: '${steps.inc.stdout}'}}\n\
                    \x20 check:\n    if: {this: states.COUNT, gt: 1}\n    then: {again: {cmd: [echo, 'again ${states.COUNT} ${states.KEEP}']}}\n    else: {first: {cmd: [echo, 'first ${states.COUNT} ${states.KEEP}']}}\n";
        let outcome = exec(project_in(dir.path(), yaml)).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("first 1 k"));
        assert_eq!(state_file(dir.path()), json!({"COUNT": 1, "KEEP": "k"}));
        let outcome = exec(project_in(dir.path(), yaml)).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("again 2 k"));
        assert_eq!(state_file(dir.path()), json!({"COUNT": 2, "KEEP": "k"}));
    }

    #[tokio::test]
    async fn condition_compares_loop_item_with_state() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = format!(
            "states: {{LATEST: 2, ACC: ''}}\n{}\
             \x20 each:\n    for_each: steps.seed.stdout.IDS\n    as: ID\n    do:\n\
             \x20     check:\n        if: {{this: ID, gt: '${{states.LATEST}}'}}\n        then:\n\
             \x20         plus:\n            set: {{states: {{ACC: '${{states.ACC}}+${{ID}}'}}}}\n        else:\n\
             \x20         minus:\n            set: {{states: {{ACC: '${{states.ACC}}-${{ID}}'}}}}\n\
             \x20 print:\n    cmd: [echo, '${{states.ACC}}']\n",
            seed(json!({"IDS": [1, 2, 3, 4]}))
        );
        let outcome = exec(project_in(dir.path(), &yaml)).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("-1-2+3+4"));
    }

    #[tokio::test]
    async fn allow_failure_does_not_hide_an_invalid_url() {
        let (_, err) = step_error(project(
            "steps:\n  bad:\n    get: 'not a url'\n    allow_failure: true\n",
        ))
        .await;
        assert!(matches!(err, Error::Http(_)), "{err}");
    }

    #[tokio::test]
    async fn set_fails_when_its_states_cannot_be_saved() {
        let dir = tempfile::tempdir().unwrap();
        let states = dir.path().join("states");
        let yaml = format!(
            "states: {{N: 0}}\nsteps:\n\
             \x20 lock:\n    cmd: [chmod, '555', '{}']\n\
             \x20 save:\n    set: {{states: {{N: 1}}}}\n",
            states.display()
        );
        let project = project_in(dir.path(), &yaml);
        let _writable = Writable(&states);
        assert!(exec(project).await.is_err());
        assert_eq!(state_file(dir.path()), json!({"N": 0}));
    }

    #[tokio::test]
    async fn state_is_saved_before_a_later_failure() {
        let dir = tempfile::tempdir().unwrap();
        let outcome = exec(project_in(
            dir.path(),
            "states: {N: 0}\n\
                 steps:\n\
                 \x20 save:\n    set: {states: {N: 'n=${states.N}'}}\n\
                 \x20 bad:\n    cmd: [sh, -c, 'exit 1']\n",
        ))
        .await
        .unwrap();
        assert_eq!(outcome.failed_step.as_deref(), Some("bad"));
        assert_eq!(state_file(dir.path()), json!({"N": "n=0"}));
    }

    #[tokio::test]
    async fn job_without_state_writes_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let project = project_in(dir.path(), "steps:\n  e:\n    cmd: [echo]\n");
        exec(project).await.unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn env_file_beats_the_process_environment() {
        let yaml = "steps:\n  e:\n    cmd: [echo, '${CARGO_PKG_NAME}']\n";
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
                 \x20 each:\n    for_each: steps.seed.stdout.IDS\n    as: ID\n    do:\n\
                 \x20     e:\n        cmd: [echo, '${{ID}}']\n",
                seed(json!({"IDS": [1]}))
            ),
            &[("ID", "shadow")],
        );
        let outcome = exec(project).await.unwrap();
        assert_eq!(stdout(&outcome), &json!(1));
    }

    #[tokio::test]
    async fn job_values_beat_project_values_and_keep_their_type() {
        let mut project = project(
            "values: {A: job, CONF: {n: 2, tags: [x]}}\n\
             steps:\n  e:\n    cmd: [cat]\n    stdin: {text: '${A} ${B} ${CONF.n} ${LIT}', conf: '${CONF}'}\n",
        );
        project.values = BTreeMap::from([
            ("A".to_string(), json!("project")),
            ("B".to_string(), json!(1)),
            ("LIT".to_string(), json!("${UNDEFINED}")),
        ]);
        let outcome = exec(project).await.unwrap();
        assert_eq!(
            stdout(&outcome),
            &json!({"text": "job 1 2 ${UNDEFINED}", "conf": {"n": 2, "tags": ["x"]}})
        );
    }

    #[tokio::test]
    async fn values_beat_the_env_file_and_loop_items_beat_values() {
        let (mut project, _dir) = project_with_env(
            &format!(
                "{}\
                 \x20 each:\n    for_each: steps.seed.stdout.IDS\n    as: ID\n    do:\n\
                 \x20     e:\n        cmd: [echo, '${{ID}}']\n\
                 \x20 after:\n    cmd: [echo, '${{ID}}-${{TOKEN}}']\n",
                seed(json!({"IDS": [1]}))
            ),
            &[("ID", "env"), ("TOKEN", "env")],
        );
        project.values = BTreeMap::from([
            ("ID".to_string(), json!("value")),
            ("TOKEN".to_string(), json!("value")),
        ]);
        let outcome = exec(project).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("value-value"));
    }

    #[tokio::test]
    async fn default_cwd_is_the_current_directory_not_the_project_dir() {
        let dir = tempfile::tempdir().unwrap();
        let outcome = exec(project_in(dir.path(), "steps:\n  e:\n    cmd: [pwd]\n"))
            .await
            .unwrap();
        let printed = std::path::PathBuf::from(stdout(&outcome).as_str().unwrap());
        assert_eq!(
            printed.canonicalize().unwrap(),
            std::env::current_dir().unwrap().canonicalize().unwrap()
        );
    }

    #[tokio::test]
    async fn set_works_out_every_value_before_setting_any() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "states: {A: 1, B: 2}\n\
                    steps:\n\
                    \x20 swap:\n    set: {states: {A: '${states.B}', B: '${states.A}'}}\n\
                    \x20 print:\n    cmd: [echo, '${states.A}-${states.B}']\n";
        let outcome = exec(project_in(dir.path(), yaml)).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("2-1"));
        assert_eq!(state_file(dir.path()), json!({"A": 2, "B": 1}));
    }

    #[tokio::test]
    async fn switch_case_keys_are_not_expanded() {
        let yaml = "steps:\n  s:\n    switch: V\n    cases: {'${X}': {lit: {cmd: [echo, literal]}}}\n\
                    \x20   default: {other: {cmd: [echo, other]}}\n";
        for (v, expected) in [("a", "other"), ("${X}", "literal")] {
            let (project, _dir) = project_with_env(yaml, &[("X", "a"), ("V", v)]);
            let outcome = exec(project).await.unwrap();
            assert_eq!(stdout(&outcome), &json!(expected), "{v}");
        }
    }

    #[test]
    fn failure_is_http_400_or_more_a_non_zero_exit_or_an_error() {
        assert_eq!(StepResult::http(399).failure(), None);
        assert_eq!(
            StepResult::http(400).failure().as_deref(),
            Some("returned HTTP 400")
        );
        assert_eq!(
            StepResult::http(404).failure().as_deref(),
            Some("returned HTTP 404")
        );
        assert_eq!(StepResult::cmd(0).failure(), None);
        assert_eq!(
            StepResult::cmd(3).failure().as_deref(),
            Some("exited with code 3")
        );
        let errored = StepResult::Errored {
            error: "timed out".to_string(),
        };
        assert_eq!(errored.failure().as_deref(), Some("timed out"));
    }
}
