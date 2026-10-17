use std::cell::RefCell;
use std::collections::BTreeMap;
use std::time::Instant;

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::command::{self, CommandResult};
use crate::error::Error;
use crate::project::Project;
use crate::project_vars::ProjectVars;
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

/// Queued work: a step to run with its label and the `vars` in scope around it, or a
/// loop's remaining items.
enum Work {
    Step {
        step: Box<Step>,
        name: String,
        scope: Scope,
    },
    Loop(Box<Loop>),
}

/// `vars` in scope, innermost already merged over outer: step, job.
type Scope = BTreeMap<String, String>;

/// Per remaining item, bind `var` to it and run `body`; then restore `var` to `previous`
/// (`None` unbinds it). Expanded one item at a time, so memory stays O(`body`).
struct Loop {
    body: Vec<Step>,
    scope: Scope,
    var: String,
    items: std::vec::IntoIter<Value>,
    previous: Option<Value>,
}

/// `steps` as work, in order, each inside `scope`.
fn queued(steps: Vec<Step>, scope: &Scope) -> impl DoubleEndedIterator<Item = Work> + '_ {
    steps.into_iter().map(move |step| Work::Step {
        name: step.name.clone(),
        step: Box::new(step),
        scope: scope.clone(),
    })
}

/// Runs the project's `job` steps in order; later steps read earlier results as
/// `steps.NAME`. A branch's chosen `then`/`else` steps, or a switch's matching case or
/// `default`, run in its place; a loop's `do` steps run once per list element, with the
/// element bound to its `as` variable. A step's `vars` apply to it and the steps nested in it.
/// The job's `states` defaults are read from `states/<job>.json` next to the project (created from
/// the defaults if missing) as `states.NAME`, and each `set.states` writes it back right away.
/// Each `set.vars` sets project `vars` in `project_vars`; runs given the same `project_vars`
/// see each other's updates.
/// With `log`, one JSON line per step, branch or switch decision, and loop is printed to stdout.
pub async fn exec(
    mut project: Project,
    job: &str,
    log: bool,
    project_vars: &ProjectVars,
) -> Result<Outcome, Error> {
    let steps = project.take(job)?;
    let base: Scope = steps.vars;
    // Below `scope`: the project `vars` over the env_file.
    let env = match project.env_file.clone() {
        Some(path) => tokio::task::spawn_blocking(move || env_file::load(&path)).await??,
        None => BTreeMap::new(),
    };
    // Also checks every project var expands before any step runs.
    let (mut outer_version, updated) = project_vars.snapshot();
    let mut outer = outer_vars(&project.vars, &updated, &env)?;

    let mut values = Map::new();
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
    let mut stack: Vec<Work> = queued(steps.steps, &base).rev().collect();
    while let Some(work) = stack.pop() {
        let (mut step, name, mut scope) = match work {
            Work::Step { step, name, scope } => (*step, name, scope),
            Work::Loop(mut each) => {
                if let Some(item) = each.items.next() {
                    values.insert(each.var.clone(), item);
                    let body: Vec<Work> = queued(each.body.clone(), &each.scope).collect();
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
        // Other jobs may have updated project `vars` since the last step.
        if let Some((version, updated)) = project_vars.changed_since(outer_version) {
            outer = outer_vars(&project.vars, &updated, &env)?;
            outer_version = version;
        }
        scope.extend(std::mem::take(&mut step.vars));
        let to_step_error = |source| Error::Step {
            name: name.clone(),
            source: Box::new(source),
        };

        if let Some(set) = step.set.take() {
            // Worked out from the current project `vars` and applied under one lock, so a
            // concurrent `set` of the same var isn't lost. The states are saved first, under the
            // lock too, so a failed save leaves the project `vars` unchanged.
            let (states, set_vars) = project_vars.update_with(|version, current| {
                let rebuilt;
                let outer = if version == outer_version {
                    &outer
                } else {
                    rebuilt = outer_vars(&project.vars, current, &env)?;
                    &rebuilt
                };
                let substitute = |updates: BTreeMap<String, Value>| {
                    let mut set = Map::new();
                    for (var, mut value) in updates {
                        vars::substitute_value(&mut value, &|var| {
                            lookup(var, &values, &scope, outer)
                        })
                        .map_err(to_step_error)?;
                        set.insert(var, value);
                    }
                    Ok::<_, Error>(set)
                };
                let (states, set_vars) = (substitute(set.states)?, substitute(set.vars)?);
                if !states.is_empty() {
                    let mut saved = state.clone();
                    saved.extend(states.clone());
                    state::save(&state_path, &saved).map_err(to_step_error)?;
                }
                Ok(((states, set_vars.clone()), set_vars))
            })?;
            if !states.is_empty() {
                state.extend(states.clone());
                values.insert("states".to_string(), Value::Object(state.clone()));
            }
            // Logged once the states are saved, so a failed save isn't logged as set.
            if log {
                progress::emit(job, progress::set(&name, &states, &set_vars)).await;
            }
            continue;
        }

        if let Some(condition) = step.condition.take() {
            let taken = condition
                .evaluate(&|var| lookup(var, &values, &scope, &outer))
                .map_err(to_step_error)?;
            let chosen = if taken { step.then } else { step.otherwise };
            if log {
                progress::emit(job, progress::branch(&name, taken, chosen.len())).await;
            }
            stack.extend(queued(chosen, &scope).rev());
            continue;
        }

        if let Some(var) = step.switch.take() {
            let value = lookup(&var, &values, &scope, &outer)
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
            stack.extend(queued(chosen, &scope).rev());
            continue;
        }

        if let Some(list_var) = step.for_each.take() {
            let items = match lookup(&list_var, &values, &scope, &outer) {
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
                scope,
                previous: values.get(&var).cloned(),
                var,
                items: items.into_iter(),
            })));
            continue;
        }

        let allow_failure = step.allow_failure;
        let started = Instant::now();
        let result = run_step(step, &mut values, &scope, &outer, &mut clients)
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
    scope: &Scope,
    outer: &Map<String, Value>,
    clients: &mut http::Clients,
) -> Result<StepResult, Error> {
    let request = step.http.take();
    let result = match perform(&step, request, values, scope, outer, clients).await {
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
    scope: &Scope,
    outer: &Map<String, Value>,
    clients: &mut http::Clients,
) -> Result<StepResult, Error> {
    let find = |var: &str| lookup(var, values, scope, outer);
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

/// Looks `var` up in `steps`, `states` and loop items (dot paths such as `steps.login.body.token`
/// included), then the `vars` in `scope`, then the project `vars` and env_file in `outer`
/// (dot paths included), then the process environment.
fn lookup(
    var: &str,
    values: &Map<String, Value>,
    scope: &Scope,
    outer: &Map<String, Value>,
) -> Option<Value> {
    if let Some(value) = extract::lookup(values, var) {
        return Some(value);
    }
    if let Some(text) = scope.get(var) {
        return Some(Value::String(text.clone()));
    }
    extract::lookup(outer, var).or_else(|| std::env::var(var).ok().map(Value::String))
}

/// The project `vars` over `env` (the env_file): each var's `updated` value if `set` has
/// changed it, else its declared value expanded from the current values of the others.
fn outer_vars(
    declared: &BTreeMap<String, String>,
    updated: &Map<String, Value>,
    env: &BTreeMap<String, String>,
) -> Result<Map<String, Value>, Error> {
    let mut outer: Map<String, Value> = env
        .iter()
        .map(|(name, value)| (name.clone(), Value::String(value.clone())))
        .collect();
    let done = RefCell::new(BTreeMap::new());
    for name in declared.keys() {
        let value = match updated.get(name) {
            Some(value) => value.clone(),
            None => Value::String(expand_project_var(
                name,
                declared,
                updated,
                env,
                &[],
                &done,
            )?),
        };
        outer.insert(name.clone(), value);
    }
    Ok(outer)
}

/// The project var `name` with `${NAME}` in its value expanded from the other project vars
/// (their `updated` value if `set` has changed them), then `env` (the env_file), then the
/// process environment. `seen` holds the project vars being expanded, to catch a value that
/// refers back to itself; `done` the ones already expanded, so each is expanded once.
fn expand_project_var(
    name: &str,
    declared: &BTreeMap<String, String>,
    updated: &Map<String, Value>,
    env: &BTreeMap<String, String>,
    seen: &[&str],
    done: &RefCell<BTreeMap<String, String>>,
) -> Result<String, Error> {
    if let Some(value) = done.borrow().get(name) {
        return Ok(value.clone());
    }
    if seen.contains(&name) {
        return Err(Error::CyclicVar(name.to_string()));
    }
    let text = declared
        .get(name)
        .ok_or_else(|| Error::UndefinedVar(name.to_string()))?;
    let seen = [seen, &[name]].concat();
    // `substitute` only learns that a lookup failed, so keep the nested error to return it.
    let nested = RefCell::new(None);
    let find = |var: &str| {
        if !declared.contains_key(var) {
            let value = env.get(var).cloned().or_else(|| std::env::var(var).ok());
            return value.map(Value::String);
        }
        if let Some(value) = updated.get(var) {
            return Some(value.clone());
        }
        match expand_project_var(var, declared, updated, env, &seen, done) {
            Ok(value) => Some(Value::String(value)),
            Err(err) => {
                nested.replace(Some(err));
                None
            }
        }
    };
    let expanded = vars::substitute(text, &find);
    let value = match nested.into_inner() {
        Some(err) => return Err(err),
        None => expanded?,
    };
    done.borrow_mut().insert(name.to_string(), value.clone());
    Ok(value)
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
            vars: BTreeMap::new(),
            env_file: None,
            schedules: BTreeMap::new(),
            jobs: BTreeMap::from([("w".to_string(), job)]),
        }
    }

    async fn exec(project: Project) -> Result<Outcome, Error> {
        exec_with(project, &ProjectVars::default()).await
    }

    /// Runs job `w` with shared project `vars`.
    async fn exec_with(project: Project, vars: &ProjectVars) -> Result<Outcome, Error> {
        super::exec(project, "w", false, vars).await
    }

    /// The failing step's name and its error, for a run that must fail in a step.
    async fn step_error(project: Project) -> (String, Error) {
        match exec(project).await {
            Err(Error::Step { name, source }) => (name, *source),
            Err(err) => panic!("expected a step error, got {err}"),
            Ok(_) => panic!("expected a step error, got success"),
        }
    }

    /// `project` with its project `vars` set to `vars`.
    fn with_vars(mut project: Project, vars: &[(&str, &str)]) -> Project {
        project.vars = vars
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();
        project
    }

    /// Like [`project`], with `vars` set.
    fn project_with_vars(yaml: &str, vars: &[(&str, &str)]) -> Project {
        with_vars(project(yaml), vars)
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
        let outcome = exec(project_with_vars(
            "steps:\n  e:\n    cmd: [echo, '${X}']\n",
            &[("X", "a; echo injected  b")],
        ))
        .await
        .unwrap();
        assert_eq!(stdout(&outcome), &json!("a; echo injected  b"));
    }

    #[tokio::test]
    async fn step_vars_beat_job_vars_beat_project_vars() {
        let project = project_with_vars(
            "vars: {B: job, C: job}\nsteps:\n\
             \x20 print:\n    cmd: [echo, '${B}-${C}-${D}']\n    vars: {C: step}\n",
            &[("B", "project"), ("D", "project")],
        );
        let outcome = exec(project).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("job-step-project"));
    }

    #[tokio::test]
    async fn job_and_step_vars_values_are_literal() {
        let project = project_with_vars(
            &format!(
                "vars: {{BACKEND: '${{PROJECTS}}/backend'}}\n{}\
                 \x20 print:\n    cmd: [echo, '${{SRC}} ${{BACKEND}}']\n    cwd: '${{ROOT}}'\n\
                 \x20   vars: {{SRC: '${{steps.seed.stdout.ID}}/src'}}\n",
                seed(json!({"ID": 7}))
            ),
            &[("ROOT", "${PROJECTS}"), ("PROJECTS", "/")],
        );
        let outcome = exec(project).await.unwrap();
        assert_eq!(
            stdout(&outcome),
            &json!("${steps.seed.stdout.ID}/src ${PROJECTS}/backend")
        );
    }

    #[tokio::test]
    async fn project_vars_referring_to_themselves_error() {
        let err = exec(project_with_vars(
            "steps:\n  e:\n    cmd: [echo, '${A}']\n",
            &[("A", "x${B}"), ("B", "${A}")],
        ))
        .await
        .err()
        .unwrap();
        assert!(matches!(err, Error::CyclicVar(ref v) if v == "A"), "{err}");
    }

    #[tokio::test]
    async fn step_vars_reach_nested_steps_but_not_siblings() {
        let yaml = format!(
            "{}\
             \x20 each:\n    for_each: steps.seed.stdout.IDS\n    vars: {{IDS_SEP: '+', NOPE_P: outer}}\n    do:\n\
             \x20     check:\n        if: {{this: NOPE_P, eq: inner}}\n        vars: {{NOPE_P: inner}}\n        then:\n\
             \x20         add:\n            set: {{vars: {{ACC: '${{ACC}}${{IDS_SEP}}${{NOPE_P}}${{ITEM}}'}}}}\n\
             \x20 print:\n    cmd: [echo, '${{ACC}}']\n",
            seed(json!({"IDS": [1, 2]}))
        );
        let outcome = exec(project_with_vars(&yaml, &[("ACC", "")]))
            .await
            .unwrap();
        assert_eq!(stdout(&outcome), &json!("+inner1+inner2"));

        let (_, err) = step_error(project_with_vars(
            &format!("{yaml}  after:\n    cmd: [echo, '${{NOPE_P}}']\n"),
            &[("ACC", "")],
        ))
        .await;
        assert!(
            matches!(err, Error::UndefinedVar(ref v) if v == "NOPE_P"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn step_files_are_recorded_under_their_key() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("greet.yaml"),
            "vars: {WHO: file}\ncmd: [echo, 'hi ${WHO}']\n",
        )
        .unwrap();
        let project = project_in(
            dir.path(),
            "steps:\n  hello: greet.yaml\n  print:\n    cmd: [echo, '${steps.hello.stdout}!']\n",
        );
        let outcome = exec(project).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("hi file!"));
    }

    #[tokio::test]
    async fn skipped_branch_with_no_step_run_succeeds_without_result() {
        let outcome = exec(project_with_vars(
            "steps:\n  r:\n    if: {this: X, eq: 1}\n    then: {a: {cmd: [echo, hi]}}\n",
            &[("X", "2")],
        ))
        .await
        .unwrap();
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

    /// A job seeded with `N`, then a switch on it that sets project var `PICK` (default
    /// `none`; cases 200, "404" and a duplicate 404, plus a default if `default`), then echoes
    /// `after ${PICK}`.
    fn switch_project(n: Value, default: bool) -> Project {
        let pick = |word: &str| format!("{{pick: {{set: {{vars: {{PICK: {word}}}}}}}}}");
        let default = if default {
            format!("    default: {}\n", pick("other"))
        } else {
            String::new()
        };
        project_with_vars(
            &format!(
                "{}\
             \x20 route:\n    switch: steps.seed.stdout.N\n    cases:\n\
             \x20     200: {}\n\
             \x20     \"404\": {}\n\
             \x20     404: {}\n\
             {default}\
             \x20 after:\n    cmd: [echo, 'after ${{PICK}}']\n",
                seed(json!({ "N": n })),
                pick("ok"),
                pick("missing"),
                pick("second"),
            ),
            &[("PICK", "none")],
        )
    }

    #[tokio::test]
    async fn switch_runs_first_matching_case() {
        for (n, expected) in [
            (json!(200), "after ok"),
            (json!("200"), "after ok"),
            (json!(404), "after missing"),
        ] {
            let outcome = exec(switch_project(n.clone(), true)).await.unwrap();
            assert_eq!(stdout(&outcome), &json!(expected), "{n}");
        }
    }

    #[tokio::test]
    async fn switch_falls_back_to_default_or_runs_nothing() {
        let outcome = exec(switch_project(json!(500), true)).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("after other"));
        let outcome = exec(switch_project(json!(500), false)).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("after none"));
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
        let outcome = exec(project_with_vars(
            &format!(
                "{}\
                     \x20 each:\n    for_each: steps.seed.stdout.IDS\n    as: ID\n    do:\n\
                     \x20     add:\n        set: {{vars: {{ACC: '${{ACC}}-${{ID}}'}}}}\n\
                     \x20 print:\n    cmd: [echo, '${{ACC}}']\n",
                seed(json!({"IDS": [1, 2, 3]}))
            ),
            &[("ACC", "start")],
        ))
        .await
        .unwrap();
        assert_eq!(stdout(&outcome), &json!("start-1-2-3"));
    }

    #[tokio::test]
    async fn for_each_default_item_with_paths_and_nested_branch() {
        let outcome = exec(
            project_with_vars(
                "steps:\n\
                 \x20 list:\n    cmd: [echo, '[{\"id\": 1, \"ok\": true}, {\"id\": 2, \"ok\": false}]']\n\
                 \x20 each:\n    for_each: steps.list.stdout\n    do:\n\
                 \x20     check:\n        if: {this: ITEM.ok, eq: true}\n        then:\n\
                 \x20         plus:\n            set: {vars: {ACC: '${ACC}+${ITEM.id}'}}\n        else:\n\
                 \x20         minus:\n            set: {vars: {ACC: '${ACC}-${ITEM.id}'}}\n\
                 \x20 print:\n    cmd: [echo, '${ACC}']\n",
                &[("ACC", "")],
            ))
        .await
        .unwrap();
        assert_eq!(stdout(&outcome), &json!("+1-2"));
    }

    #[tokio::test]
    async fn nested_loops_inside_branch() {
        let outcome = exec(
            project_with_vars(
                &format!(
                    "{}\
                     \x20 go:\n    if: {{this: steps.seed.stdout.GO, eq: true}}\n    then:\n\
                     \x20     rows:\n        for_each: steps.seed.stdout.ROWS\n        as: ROW\n        do:\n\
                     \x20         cells:\n            for_each: ROW\n            as: CELL\n            do:\n\
                     \x20             add:\n                set: {{vars: {{ACC: '${{ACC}}${{CELL}}'}}}}\n\
                     \x20 print:\n    cmd: [echo, '${{ACC}}']\n",
                    seed(json!({"GO": true, "ROWS": [["a", "b"], ["c"]]}))
                ),
                &[("ACC", "")],
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
        let (name, err) = step_error(project_with_vars(yaml, &[("NOPE_L", "a,b")])).await;
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
            "states: {{LATEST: 2}}\n{}\
             \x20 each:\n    for_each: steps.seed.stdout.IDS\n    as: ID\n    do:\n\
             \x20     check:\n        if: {{this: ID, gt: '${{states.LATEST}}'}}\n        then:\n\
             \x20         plus:\n            set: {{vars: {{ACC: '${{ACC}}+${{ID}}'}}}}\n        else:\n\
             \x20         minus:\n            set: {{vars: {{ACC: '${{ACC}}-${{ID}}'}}}}\n\
             \x20 print:\n    cmd: [echo, '${{ACC}}']\n",
            seed(json!({"IDS": [1, 2, 3, 4]}))
        );
        let outcome = exec(with_vars(project_in(dir.path(), &yaml), &[("ACC", "")]))
            .await
            .unwrap();
        assert_eq!(stdout(&outcome), &json!("-1-2+3+4"));
    }

    #[tokio::test]
    async fn project_vars_using_a_set_var_see_its_new_value() {
        let yaml = "steps:\n\
                    \x20 bump:\n    set: {vars: {B: new}}\n\
                    \x20 print:\n    cmd: [echo, '${A} ${B}']\n";
        let project = project_with_vars(yaml, &[("A", "prefix-${B}"), ("B", "old")]);
        let outcome = exec(project).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("prefix-new new"));
    }

    /// Each var uses the one before twice, so expanding without reuse takes 2^30 lookups.
    #[tokio::test]
    async fn project_vars_used_many_times_are_expanded_once() {
        let mut vars = vec![("V0".to_string(), String::new())];
        for i in 1..=30 {
            vars.push((format!("V{i}"), format!("${{V{0}}}${{V{0}}}", i - 1)));
        }
        let vars: Vec<(&str, &str)> = vars.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
        let project = project_with_vars("steps:\n  e:\n    cmd: [echo, 'x${V30}x']\n", &vars);
        let outcome = exec(project).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("xx"));
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
    async fn set_leaves_vars_unchanged_when_its_states_cannot_be_saved() {
        let dir = tempfile::tempdir().unwrap();
        let states = dir.path().join("states");
        let yaml = format!(
            "states: {{N: 0}}\nsteps:\n\
             \x20 lock:\n    cmd: [chmod, '555', '{}']\n\
             \x20 save:\n    set: {{states: {{N: 1}}, vars: {{V: new}}}}\n",
            states.display()
        );
        let project = with_vars(project_in(dir.path(), &yaml), &[("V", "old")]);
        let project_vars = ProjectVars::default();
        let _writable = Writable(&states);
        assert!(exec_with(project, &project_vars).await.is_err());
        assert_eq!(project_vars.snapshot(), (0, Map::new()));
        assert_eq!(state_file(dir.path()), json!({"N": 0}));
    }

    #[tokio::test]
    async fn set_vars_is_seen_by_later_steps_and_keeps_its_type() {
        let yaml = "steps:\n\
                    \x20 seed:\n    cmd: [echo, '[1, 2]']\n\
                    \x20 keep:\n    set: {vars: {L: '${steps.seed.stdout}', N: 'n=${N}'}}\n\
                    \x20 check:\n    for_each: L\n    do: {e: {cmd: [echo, '${N} ${L.1} ${ITEM}']}}\n";
        let project_vars = ProjectVars::default();
        let project = project_with_vars(yaml, &[("L", "none"), ("N", "0")]);
        let outcome = exec_with(project, &project_vars).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("n=0 2 2"));
        assert_eq!(
            Value::Object(project_vars.snapshot().1),
            json!({"L": [1, 2], "N": "n=0"})
        );
    }

    #[tokio::test]
    async fn runs_with_the_same_project_vars_see_each_others_updates() {
        let project_vars = ProjectVars::default();
        let vars = [("TOKEN", "none")];
        let writer = project_with_vars(
            "steps:\n  keep:\n    set: {vars: {TOKEN: abc}}\n  e:\n    cmd: [echo]\n",
            &vars,
        );
        let reader = || project_with_vars("steps:\n  e:\n    cmd: [echo, '${TOKEN}']\n", &vars);
        let outcome = exec_with(reader(), &project_vars).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("none"));
        exec_with(writer, &project_vars).await.unwrap();
        let outcome = exec_with(reader(), &project_vars).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("abc"));
        let outcome = exec(reader()).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("none"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_set_vars_lose_no_updates() {
        let project_vars = ProjectVars::default();
        let project = |letter: &str| {
            project_with_vars(
                &format!(
                    "{}\
                     \x20 each:\n    for_each: steps.seed.stdout\n    do:\n\
                     \x20     add:\n        set: {{vars: {{N: '${{N}}{letter}'}}}}\n",
                    seed(json!(vec![0; 200]))
                ),
                &[("N", "")],
            )
        };
        let spawn = |project: Project| {
            let project_vars = project_vars.clone();
            tokio::spawn(async move { exec_with(project, &project_vars).await })
        };
        let (a, b) = tokio::join!(spawn(project("a")), spawn(project("b")));
        a.unwrap().unwrap();
        b.unwrap().unwrap();
        let n = project_vars.snapshot().1["N"].as_str().unwrap().len();
        assert_eq!(n, 400);
    }

    #[tokio::test]
    async fn project_vars_expand_from_updated_vars() {
        let project_vars = ProjectVars::default();
        project_vars
            .update_with(|_, _| Ok(((), Map::from_iter([("HOST".to_string(), json!("new"))]))))
            .unwrap();
        let project = project_with_vars(
            "steps:\n  e:\n    cmd: [echo, '${URL}']\n",
            &[("HOST", "orig"), ("URL", "${HOST}/x")],
        );
        let outcome = exec_with(project, &project_vars).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("new/x"));
    }

    #[tokio::test]
    async fn job_vars_shadow_an_updated_project_var() {
        let project = project_with_vars(
            "vars: {A: job}\nsteps:\n\
             \x20 keep:\n    set: {vars: {A: updated, B: updated}}\n\
             \x20 e:\n    cmd: [echo, '${A}-${B}']\n",
            &[("A", "project"), ("B", "project")],
        );
        let outcome = exec(project).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("job-updated"));
    }

    #[tokio::test]
    async fn project_vars_beat_the_env_file() {
        let dir = tempfile::tempdir().unwrap();
        let env = dir.path().join(".env");
        std::fs::write(&env, "A=env\nB=env\n").unwrap();
        let mut project = project_with_vars(
            "steps:\n  e:\n    cmd: [echo, '${A}-${B}']\n",
            &[("A", "project")],
        );
        project.env_file = Some(env);
        let outcome = exec(project).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("project-env"));
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
        let project = project_in(
            dir.path(),
            "steps:\n  keep:\n    set: {vars: {V: new}}\n  e:\n    cmd: [echo]\n",
        );
        exec(with_vars(project, &[("V", "old")])).await.unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn process_environment_is_the_last_fallback() {
        let yaml = "steps:\n  e:\n    cmd: [echo, '${CARGO_PKG_NAME}']\n";
        let outcome = exec(project(yaml)).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("rocky"));
        let outcome = exec(project_with_vars(yaml, &[("CARGO_PKG_NAME", "rb")]))
            .await
            .unwrap();
        assert_eq!(stdout(&outcome), &json!("rb"));
    }

    #[tokio::test]
    async fn loop_item_beats_vars() {
        let outcome = exec(project(&format!(
            "{}\
                 \x20 each:\n    for_each: steps.seed.stdout.IDS\n    as: ID\n    do:\n\
                 \x20     e:\n        cmd: [echo, '${{ID}}']\n        vars: {{ID: shadow}}\n",
            seed(json!({"IDS": [1]}))
        )))
        .await
        .unwrap();
        assert_eq!(stdout(&outcome), &json!(1));
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

        let project = project_with_vars(
            "steps:\n\
             \x20 swap:\n    set: {vars: {A: '${B}', B: '${A}'}}\n\
             \x20 print:\n    cmd: [echo, '${A}-${B}']\n",
            &[("A", "1"), ("B", "2")],
        );
        let outcome = exec(project).await.unwrap();
        assert_eq!(stdout(&outcome), &json!("2-1"));
    }

    #[tokio::test]
    async fn switch_case_keys_are_not_expanded() {
        let yaml = "steps:\n  s:\n    switch: V\n    cases: {'${X}': {lit: {cmd: [echo, literal]}}}\n\
                    \x20   default: {other: {cmd: [echo, other]}}\n";
        for (v, expected) in [("a", "other"), ("$${X}", "literal")] {
            let outcome = exec(project_with_vars(yaml, &[("X", "a"), ("V", v)]))
                .await
                .unwrap();
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
