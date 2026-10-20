use std::io::{BufRead, Write};
use std::path::Path;

use crate::error::Error;

/// What `rocky new` writes.
pub enum Kind {
    Project,
    Job,
    Task { http: bool },
}

const PROJECT: &str = "\
# A project: named jobs.
#   rocky exec <this file> example
jobs:
  example:
    tasks:
      hello:
        cmd: [echo, hello]
";

const JOB: &str = "\
# A job in its own file. Add it to a project's `jobs`: `name: path/to/this.yaml`.
tasks:
  hello:
    cmd: [echo, hello]
";

const TASK_HEADER: &str = "\
# One task in its own file. Add it to a job's `tasks`: `name: path/to/this.yaml`.
";

const CMD_TASK: &str = "\
cmd: [echo, hello]
";

const HTTP_TASK: &str = "\
get: https://httpbin.org/get
";

const FULL_PROJECT_HEADER: &str = "\
# A project: named jobs. Every field is shown; delete what you don't need.
#   rocky exec <this file> example   runs one job once
#   rocky run <this file>            runs the jobs that have a `schedule` until stopped

# Fixed values read as ${NAME} in every job's tasks: any YAML value (string, number, boolean,
# list, object). Names are uppercase letters, digits and `_`. A job's own `values` win over these.
values:
  REGION: us-east-1                 # the `example` job's own REGION wins

# A .env file for values used as ${NAME} (secrets, settings), relative to this file. It is read
# when a run starts, so it must exist. ${USER} below comes from it or from the environment.
# env_file: .env

# Job name: its tasks, or the path of a job file (`report: jobs/report.yaml`).
jobs:
  example:
";

const FULL_SCHEDULE: &str = "\
# When `rocky run` runs this job; `exec` checks it but doesn't run it. Exactly one of `every` (30s, 5m, 2h;
# optional `delay` before the first run), `hourly` (a minute, 0-59), `daily` (\"09:30\"),
# `weekly` ([mon, fri] with `time: \"09:30\"`) or `at` (\"2027-10-10T09:30:00\"), in local time.
# All but `every` take `run_on_start: true` to also run once when `run` starts.
schedule:
  every: 1h
  delay: 10s
";

const FULL_JOB_HEADER: &str = "\
# A job in its own file, with every kind of task. Add it to a project's `jobs`:
# `name: path/to/this.yaml`. ${USER} comes from the project's env_file or the environment.
";

const FULL_TASK_HEADER: &str = "\
# Every field is shown; delete what you don't need. ${USER} comes from the env_file or the environment.
";

const FULL_HTTP_TASK: &str = "\
post:                               # or get, put, patch, delete; others: `http:` with `method:`
  url: https://httpbin.org/anything/login   # must include the scheme
  headers:
    Accept: application/json
  query:                            # appended to the url, percent-encoded
    source: rocky
  json:                             # sent as the JSON body; ${...} works in any string
    user: ${USER}
    token: demo-token
    tags: [admin, beta]
  insecure: false                   # true skips TLS certificate checks
  follow_redirects: true            # false returns a 3xx response itself
timeout: 10                         # seconds (default 30)
capture_body: true                  # false reads the body without keeping it in memory
allow_failure: false                # true continues after a 4xx/5xx, a timeout or a network error
";

const FULL_CMD_TASK: &str = "\
cmd: [sh, -c, 'printf \"%s, \" \"$GREETING\"; cat']        # program + arguments; no shell unless you call one
pipe:                               # more commands, like `cmd | tr a-z A-Z`: each gets the previous one's stdout
  - [tr, a-z, A-Z]
stdin: ${USER}                      # standard input; a non-string is written as compact JSON
env:                                # added to the command's environment, read as $GREETING; a non-string as compact JSON
  GREETING: hello
cwd: .                              # relative to this file; ${VAR} works (default: your current directory)
timeout: 5                          # seconds
capture_stdout: true                # false sends stdout to /dev/null instead of keeping it in memory
allow_failure: false                # true continues after a non-zero exit or a timeout
";

const FULL_CONTROL: &str = "
  # A branch: `then` runs when the condition holds, `else` otherwise. Either may be left out.
  check:
    if:                                 # a list: all must hold (also `all`, `any`, `not`)
      - {this: tasks.login.status, eq: 200}   # a name or dot path, without ${}
      - {this: tasks.shout.stdout, contains: ALICE}   # also ne, gt, gte, lt, lte, matches
      - {this: tasks.login.body.json.token, exists: true}   # defined at all; never fails
    then:
      greet:
        cmd: [echo, \"logged in with ${tasks.login.body.json.token}\"]
    else:
      complain:
        cmd: [echo, login failed]

  # A switch: runs the tasks of the first case whose value equals the variable.
  route:
    switch: tasks.login.status
    cases:
      200:
        ok:
          cmd: [echo, ok]
    default:                            # when no case matches
      unexpected:
        cmd: [echo, \"unexpected status ${tasks.login.status}\"]

  # A loop: runs `do` once for each element of a list.
  each_tag:
    for_each: tasks.login.body.json.tags   # a list, without ${}
    as: TAG                             # default ITEM
    do:
      tag:
        cmd: [echo, \"tag ${TAG}\"]

  # A task file: one task kept in its own file, relative to this file.
  # notify: tasks/notify.yaml
";

/// How much a starter file shows.
#[derive(Clone, Copy, clap::ValueEnum)]
pub enum Level {
    /// The smallest file that runs.
    Basic,
    /// Every field, with comments.
    Full,
}

/// Writes a starter file of `kind` at `path`, creating missing parent directories, and
/// returns a hint on how to use it. Never overwrites an existing file.
pub fn create(kind: Kind, level: Level, path: &Path) -> Result<String, Error> {
    let full = matches!(level, Level::Full);
    let (text, hint) = match kind {
        Kind::Project => (
            if full {
                format!("{FULL_PROJECT_HEADER}{}", indent(&full_tasks(), 4))
            } else {
                PROJECT.to_string()
            },
            format!("run it: rocky exec {} example", path.display()),
        ),
        Kind::Job => {
            let name = path.file_stem().unwrap_or_default().to_string_lossy();
            (
                if full {
                    format!("{FULL_JOB_HEADER}\n{}", full_tasks())
                } else {
                    JOB.to_string()
                },
                format!(
                    "add it to a project:\n  jobs:\n    {name}: {}",
                    path.display()
                ),
            )
        }
        Kind::Task { http } => {
            let name = path.file_stem().unwrap_or_default().to_string_lossy();
            (
                match (full, http) {
                    (false, true) => format!("{TASK_HEADER}{HTTP_TASK}"),
                    (false, false) => format!("{TASK_HEADER}{CMD_TASK}"),
                    (true, true) => {
                        format!("{TASK_HEADER}{FULL_TASK_HEADER}{FULL_HTTP_TASK}")
                    }
                    (true, false) => {
                        format!("{TASK_HEADER}{FULL_TASK_HEADER}{FULL_CMD_TASK}")
                    }
                },
                format!("add it to a job:\n  tasks:\n    {name}: {}", path.display()),
            )
        }
    };
    write_new(path, &text)?;
    Ok(format!("created {}\n{hint}", path.display()))
}

/// Asks on `out` whether a task is HTTP or cmd, reading answers from `input` until one is
/// `http`, `cmd` or empty (cmd). End of input counts as empty.
pub fn ask_http(mut input: impl BufRead, mut out: impl Write) -> Result<bool, Error> {
    write!(out, "task type [cmd/http] (default cmd): ").map_err(Error::Prompt)?;
    loop {
        out.flush().map_err(Error::Prompt)?;
        let mut answer = String::new();
        input.read_line(&mut answer).map_err(Error::Prompt)?;
        match answer.trim().to_lowercase().as_str() {
            "" | "cmd" => return Ok(false),
            "http" => return Ok(true),
            _ => write!(out, "answer cmd or http: ").map_err(Error::Prompt)?,
        }
    }
}

/// A job's `schedule`, and its `tasks` with one task of every kind, using every field.
fn full_tasks() -> String {
    format!(
        "{FULL_SCHEDULE}values:                             # fixed values for this job's tasks, read as ${{NAME}}\n  \
         REGION: eu-west-1\n  \
         LIMITS: {{retries: 3}}             # an object: read inside it with ${{LIMITS.retries}}\n\
         tasks:\n  \
         # An HTTP request. Later tasks read its result by name: ${{tasks.login.status}}.\n  \
         login:\n{}\n  \
         # A command.\n  \
         shout:\n{}\n  \
         # A command reading `values`.\n  \
         region:\n    \
         cmd: [echo, \"${{REGION}}, ${{LIMITS.retries}} retries\"]\n{FULL_CONTROL}",
        indent(FULL_HTTP_TASK, 4),
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
        let project_file = dir.path().join("project.yaml");
        create(Kind::Project, level, &project_file).unwrap();
        let mut project = project::load(&project_file).unwrap();
        let example = project.take("example").unwrap();
        assert_eq!(example.schedule.is_some(), matches!(level, Level::Full));
        if matches!(level, Level::Full) {
            assert_eq!(example.values["REGION"], "eu-west-1");
            assert_eq!(example.values["LIMITS"]["retries"], 3);
        }

        create(Kind::Job, level, &dir.path().join("jobs/w.yaml")).unwrap();
        let task = |http| Kind::Task { http };
        create(task(false), level, &dir.path().join("a/cmd.yaml")).unwrap();
        create(task(true), level, &dir.path().join("a/http.yaml")).unwrap();
        let project_file = dir.path().join("files.yaml");
        std::fs::write(
            &project_file,
            "jobs:\n  w: jobs/w.yaml\n  \
             a: {tasks: {sh: a/cmd.yaml, web: a/http.yaml}}\n",
        )
        .unwrap();
        let mut project = project::load(&project_file).unwrap();
        let w = project.take("w").unwrap();
        assert_eq!(w.schedule.is_some(), matches!(level, Level::Full));
        if matches!(level, Level::Full) {
            assert_eq!(w.values["REGION"], "eu-west-1");
        }
        let a = project.take("a").unwrap();
        assert!(a.tasks[0].cmd.is_some() && a.tasks[1].http.is_some());
        assert_eq!(a.tasks[0].pipe.is_empty(), matches!(level, Level::Basic));
        assert_eq!(a.tasks[0].name, "sh");
        assert_eq!(a.tasks[1].name, "web");
    }

    #[test]
    fn refuses_to_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.yaml");
        std::fs::write(&path, "keep").unwrap();
        let err = create(Kind::Job, Level::Basic, &path).unwrap_err();
        assert!(matches!(err, Error::Exists(_)), "{err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep");
    }

    #[test]
    fn asks_until_cmd_or_http() {
        let ask = |input: &str| ask_http(input.as_bytes(), std::io::sink()).unwrap();
        assert!(ask("http\n"));
        assert!(!ask("cmd\n"));
        assert!(!ask("\n"));
        assert!(!ask(""));
        assert!(ask("ftp\n HTTP \n"));
    }
}
