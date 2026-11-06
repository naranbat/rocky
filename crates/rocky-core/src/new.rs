use std::io::Write;
use std::path::Path;

use crate::error::Error;
use crate::project::{JOBS_DIR, SETTINGS_FILE};

/// What `rocky new` writes.
pub enum Kind {
    Project,
    Job,
    Task,
}

const JOB: &str = "\
tasks:
  hello:
    cmd: [echo, hello]
";

const TASK_HEADER: &str = "\
# One task in its own file. Add it to a job's `tasks`: `name: path/to/this.yaml`.
# To give it inputs of its own: `name: {file: path/to/this.yaml, with: {USER: bob}}`.
";

const CMD_TASK: &str = "\
cmd: [echo, hello]
";

const FULL_SETTINGS: &str = "\
# Settings shared by every job in jobs/. Every field is shown; delete what you don't need.
# This file is optional.

# Fixed values read as ${NAME} in every job's tasks: any YAML value (string, number, boolean,
# list, object). Names are uppercase letters, digits and `_`. A job's own `values` win over these.
values:
  REGION: us-east-1                 # the `example` job's own REGION wins

# A .env file for values used as ${NAME} (secrets, settings), relative to this file. It is read
# when a run starts, so it must exist. ${USER} in jobs/example.yaml comes from it or from the environment.
# env_file: .env

# Fields every cmd task in every job gets unless it sets them: `timeout`, `cwd` (relative to this
# file), `env` (merged by name) and `retry`. A job's own `defaults` win over these, field by field.
defaults:
  timeout: 1m
";

const FULL_SCHEDULE: &str = "\
# When `rocky run` runs this job; `exec` checks it but doesn't run it. Exactly one of `every` (30s, 5m, 2h;
# optional `delay` before the first run), `hourly` (a minute, 0-59), `daily` (\"09:30\"),
# `weekly` (`{days: [mon, fri], time: \"09:30\"}`) or `at` (\"2027-10-10T09:30:00\"), in local time.
# All but `every` take `run_on_start: true` to also run once when `run` starts; on its own,
# `run_on_start: true` runs the job once when `run` starts.
schedule:
  every: 1h
  delay: 10s
";

const FULL_JOB_NOTE: &str = "\
# Every field is shown, with every kind of task; delete what you don't need.
# ${USER} comes from the project's env_file or the environment.
";

const FULL_TASK_HEADER: &str = "\
# Every field is shown; delete what you don't need. ${USER} comes from the env_file or the environment.
";

const CURL_TASK: &str = "\
cmd: [curl, -sS, --fail-with-body, --json, '@-', https://httpbin.org/anything/login]   # --json sends stdin as the body
stdin:                              # written as JSON
  user: ${USER}
  token: demo-token
  tags: [admin, beta]
timeout: 10s
";

const FULL_CMD_TASK: &str = "\
cmd: [sh, -c, 'printf \"%s, \" \"$GREETING\"; cat']        # program + arguments; no shell unless you call one
pipe:                               # more commands, like `cmd | tr a-z A-Z`: each gets the previous one's stdout
  - [tr, a-z, A-Z]
stdin: ${USER}                      # standard input; a non-string is written as compact JSON
env:                                # added to the command's environment, read as $GREETING; a non-string as compact JSON
  GREETING: hello
cwd: .                              # relative to this file; ${VAR} works (default: your current directory)
timeout: 30s                        # a duration: 30s, 5m, 2h
capture_stdout: true                # false sends stdout to /dev/null instead of keeping it in memory
# stdout_file: out.txt              # or write stdout to this file, relative to this file
retry:                              # optional: run again until it exits 0, at most `attempts` times
  attempts: 1
  delay: 1s                         # wait between runs
  # until: {this: tasks.NAME.stdout, eq: ok}   # or until this holds
allow_failure: false                # true continues after a non-zero exit or a timeout
";

const FULL_CONTROL: &str = "
  # `when`: any task runs only if its condition holds; otherwise it is skipped.
  greet:
    when:                               # a list: all must hold (also `all`, `any`, `not`)
      - {this: tasks.login.exit_code, eq: 0}   # a name or dot path; ${} optional
      - {this: tasks.shout.stdout, contains: ALICE}   # also ne, gt, gte, lt, lte, matches
      - {this: tasks.login.stdout.json.token, exists: true}   # defined at all; never fails
    cmd: [echo, \"logged in with ${tasks.login.stdout.json.token}\"]
  complain:
    when: {this: tasks.greet, exists: false}   # a skipped task has no result
    cmd: [echo, login failed]

  # A loop: runs `do` once for each element of a list.
  each_tag:
    for_each: tasks.login.stdout.json.tags   # a list; ${} optional
    as: TAG                             # default ITEM
    on_failure: next                    # a failing `do` task skips to the next element (default: stop the job)
    do:
      tag:
        cmd: [echo, \"tag ${TAG}\"]

  # A `while` loop: runs `do` again and again while the condition holds, checked before each run.
  poll:
    while:
      any:
        - {this: tasks.status, exists: false}   # not run yet: run `do` at least once
        - {this: tasks.status.stdout, ne: 200}
    attempts: 5                         # optional: fail if still true after 5 runs
    on_failure: stop                    # next: a failing `do` task skips to the next condition check
    do:
      status:                           # httpbin picks 200 or 503 at random
        cmd: [curl, -sS, -o, /dev/null, -w, '%{http_code}', 'https://httpbin.org/status/200,503']

  # A task file: one task kept in its own file, relative to this file. Give it inputs of
  # its own with `with`, read as ${NAME} inside the file; only `file`, `with` and `when` go here.
  # notify: ../tasks/notify.yaml
  # notify_bob: {file: ../tasks/notify.yaml, with: {USER: bob}}

  # A trigger: queues a run of another job of the project (jobs/report.yaml) and goes on without waiting
  # (`exec` runs it after this job). Nothing is saved as tasks.NAME.
  # report: {trigger: report}
";

/// How much a starter file shows.
#[derive(Clone, Copy)]
pub enum Level {
    /// The smallest file that runs.
    Basic,
    /// Every field, with comments.
    Full,
}

/// Writes a starter file of `kind` at `path`, creating missing parent directories, and
/// returns a hint on how to use it. For a project, `path` is its directory (see [`project`]).
/// Never overwrites an existing file.
pub fn create(kind: Kind, level: Level, path: &Path) -> Result<String, Error> {
    let full = matches!(level, Level::Full);
    let name = path.file_stem().unwrap_or_default().to_string_lossy();
    let (text, hint) = match kind {
        Kind::Project => return project(level, path, false),
        Kind::Job => (
            job(&name, level),
            format!("run it from the project directory: rocky exec {name}"),
        ),
        Kind::Task => (
            if full {
                format!("{TASK_HEADER}{FULL_TASK_HEADER}{FULL_CMD_TASK}")
            } else {
                format!("{TASK_HEADER}{CMD_TASK}")
            },
            format!("add it to a job:\n  tasks:\n    {name}: {}", path.display()),
        ),
    };
    write_new(path, &text)?;
    Ok(format!("created {}\n{hint}", path.display()))
}

/// Writes a starter project in directory `dir`: `jobs/example.yaml`, and with
/// [`Level::Full`] or `env_file` a `rocky.yaml`. With `env_file`, the project also reads
/// `.env` in `dir`, which is created empty unless it exists. Never overwrites a file.
/// Returns a hint on how to use it.
pub fn project(level: Level, dir: &Path, env_file: bool) -> Result<String, Error> {
    let job_path = dir.join(JOBS_DIR).join("example.yaml");
    let settings_path = dir.join(SETTINGS_FILE);
    let settings = match (level, env_file) {
        (Level::Basic, false) => None,
        (Level::Basic, true) => Some("env_file: .env\n".to_string()),
        (Level::Full, false) => Some(FULL_SETTINGS.to_string()),
        (Level::Full, true) => {
            Some(FULL_SETTINGS.replacen("# env_file: .env", "env_file: .env", 1))
        }
    };
    let mut paths = vec![job_path.clone()];
    if settings.is_some() {
        paths.push(settings_path.clone());
    }
    if let Some(path) = paths.iter().find(|path| path.exists()) {
        return Err(Error::Exists(path.clone()));
    }
    write_new(&job_path, &job("example", level))?;
    if let Some(text) = settings {
        write_new(&settings_path, &text)?;
    }
    if env_file {
        match write_new(&dir.join(".env"), "") {
            Err(Error::Exists(_)) => {}
            other => other?,
        }
    }
    let created = paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let run = if dir == Path::new(".") {
        "run it: rocky exec example".to_string()
    } else {
        format!("run it from {}: rocky exec example", dir.display())
    };
    Ok(format!("created {created}\n{run}"))
}

/// A starter job file for job `name`.
fn job(name: &str, level: Level) -> String {
    let header = format!("# Job `{name}`: its tasks run in order.\n#   rocky exec {name}\n");
    match level {
        Level::Basic => format!("{header}{JOB}"),
        Level::Full => format!("{header}{FULL_JOB_NOTE}\n{}", full_tasks()),
    }
}

/// A job's `schedule`, and its `tasks` with one task of every kind, using every field.
fn full_tasks() -> String {
    format!(
        "{FULL_SCHEDULE}values:                             # fixed values for this job's tasks, read as ${{NAME}}\n  \
         REGION: eu-west-1\n  \
         LIMITS: {{retries: 3}}             # an object: read inside it with ${{LIMITS.retries}}\n\
         defaults:                           # timeout, cwd, env, retry for this job's cmd tasks; a task's own win\n  \
         env: {{LOG_LEVEL: info}}           # merged by name with each task's `env`\n\
         tasks:\n  \
         # An HTTP request with curl. Later tasks read its result by name; JSON output is parsed:\n  \
         # ${{tasks.login.stdout.json.token}}.\n  \
         login:\n{}\n  \
         # A command.\n  \
         shout:\n{}\n  \
         # A command reading `values`.\n  \
         region:\n    \
         cmd: [echo, \"${{REGION}}, ${{LIMITS.retries}} retries\"]\n{FULL_CONTROL}",
        indent(CURL_TASK, 4),
        indent(FULL_CMD_TASK, 4),
    )
}

/// `text` with every non-empty line indented by `width` spaces.
fn indent(text: &str, width: usize) -> String {
    text.lines()
        .map(|line| match line {
            "" => "\n".to_string(),
            _ => format!("{:width$}{line}\n", ""),
        })
        .collect()
}

fn write_new(path: &Path, text: &str) -> Result<(), Error> {
    let write_err = |source| Error::Write {
        path: path.to_path_buf(),
        source,
    };
    std::fs::create_dir_all(path.parent().unwrap_or(Path::new(""))).map_err(write_err)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| match source.kind() {
            std::io::ErrorKind::AlreadyExists => Error::Exists(path.to_path_buf()),
            _ => write_err(source),
        })?;
    file.write_all(text.as_bytes()).map_err(write_err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project;

    #[test]
    fn templates_load() {
        for level in [Level::Basic, Level::Full] {
            templates_load_at(level);
        }
    }

    fn templates_load_at(level: Level) {
        let dir = tempfile::tempdir().unwrap();
        create(Kind::Project, level, dir.path()).unwrap();
        assert_eq!(
            dir.path().join(SETTINGS_FILE).exists(),
            matches!(level, Level::Full)
        );
        create(Kind::Job, level, &dir.path().join("jobs/w.yaml")).unwrap();
        create(Kind::Task, level, &dir.path().join("tasks/cmd.yaml")).unwrap();
        std::fs::write(
            dir.path().join("jobs/a.yaml"),
            "tasks: {sh: ../tasks/cmd.yaml}\n",
        )
        .unwrap();
        let mut project = project::load(dir.path()).unwrap();
        for name in ["example", "w"] {
            let job = project.take(name).unwrap();
            assert_eq!(job.schedule.is_some(), matches!(level, Level::Full));
            if matches!(level, Level::Full) {
                assert_eq!(job.values["REGION"], "eu-west-1");
                assert_eq!(job.values["LIMITS"]["retries"], 3);
            }
        }
        let a = project.take("a").unwrap();
        assert!(a.tasks[0].cmd.is_some());
        assert_eq!(a.tasks[0].pipe.is_empty(), matches!(level, Level::Basic));
        assert_eq!(a.tasks[0].name, "sh");
    }

    #[test]
    fn project_with_env_file() {
        for level in [Level::Basic, Level::Full] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("p");
            project(level, &path, true).unwrap();
            assert_eq!(std::fs::read_to_string(path.join(".env")).unwrap(), "");
            let loaded = project::load(&path).unwrap();
            assert_eq!(loaded.env_file, Some(path.join(".env")));
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".env"), "KEEP=1\n").unwrap();
        project(Level::Basic, dir.path(), true).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join(".env")).unwrap(),
            "KEEP=1\n"
        );
        let err = project(Level::Basic, dir.path(), true).unwrap_err();
        assert!(matches!(err, Error::Exists(_)), "{err}");
    }

    #[test]
    fn refuses_to_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.yaml");
        std::fs::write(&path, "keep").unwrap();
        let err = create(Kind::Job, Level::Basic, &path).unwrap_err();
        assert!(matches!(err, Error::Exists(_)), "{err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep");

        let settings = dir.path().join(SETTINGS_FILE);
        std::fs::write(&settings, "keep").unwrap();
        let err = create(Kind::Project, Level::Full, dir.path()).unwrap_err();
        assert!(matches!(err, Error::Exists(_)), "{err}");
        assert!(!dir.path().join("jobs/example.yaml").exists());
    }
}
