use std::io::{BufRead, Write};
use std::path::Path;

use crate::error::Error;

/// What `rocky new` writes.
pub enum Kind {
    Project,
    Job,
    Step { http: bool },
}

const PROJECT: &str = "\
# A project: named jobs that share `vars`.
#   rocky exec <this file> example
vars: {}
jobs:
  example:
    steps:
      hello:
        cmd: [echo, hello]
";

const JOB: &str = "\
# A job in its own file. Add it to a project's `jobs`: `name: path/to/this.yaml`.
steps:
  hello:
    cmd: [echo, hello]
";

const STEP_HEADER: &str = "\
# One step in its own file. Add it to a job's `steps`: `name: path/to/this.yaml`.
";

const CMD_STEP: &str = "\
cmd: [echo, hello]
";

const HTTP_STEP: &str = "\
get: https://httpbin.org/get
";

const FULL_PROJECT_HEADER: &str = "\
# A project: named jobs that share `vars`. Every field is shown; delete what you don't need.
#   rocky exec <this file> example   runs one job once
#   rocky run <this file>            runs the jobs in `schedules` until stopped

# Values for ${NAME} in every job. Strings: quote numbers (\"42\"). A `set` step can
# change them at runtime; `run` keeps the new values for every job while it runs.
vars:
  USER: alice

# A .env file for secrets, relative to this file. It is read when a run starts, so it must exist.
# env_file: .env

# When `rocky run` runs each job; `exec` ignores this. Exactly one of `every` (30s, 5m,
# 2h; optional `delay` before the first run), `hourly` (a minute, 0-59), `daily` (\"09:30\"),
# `weekly` ([mon, fri] with `time: \"09:30\"`) or `at` (\"2027-10-10T09:30:00\"), in local time.
# All but `every` take `run_on_start: true` to also run once when `run` starts.
schedules:
  example:
    every: 1h
    delay: 10s

# Job name: its steps, or the path of a job file (`report: jobs/report.yaml`).
jobs:
  example:
";

const FULL_JOB_HEADER: &str = "\
# A job in its own file, with every kind of step. Add it to a project's `jobs`:
# `name: path/to/this.yaml`. ${USER} comes from the project's `vars` (or the environment), unless
# this job or a step sets its own `vars`.
";

const FULL_STEP_HEADER: &str = "\
# Every field is shown; delete what you don't need. ${USER} comes from the project's `vars`.
";

const FULL_HTTP_STEP: &str = "\
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
# vars: {USER: bob}                 # values for this step only, checked before the job's and project's
timeout: 10                         # seconds (default 30)
allow_failure: false                # true continues after a 4xx/5xx, a timeout or a network error
";

const FULL_CMD_STEP: &str = "\
cmd: [sh, -c, 'printf \"%s, \" \"$GREETING\"; tr a-z A-Z']     # program + arguments; no shell unless you call one
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
      - {this: steps.login.status, eq: 200}   # a name or dot path, without ${}
      - {this: steps.shout.stdout, contains: ALICE}   # also ne, gt, gte, lt, lte, matches
      - {this: steps.login.body.json.token, exists: true}   # defined at all; never fails
    then:
      greet:
        cmd: [echo, \"logged in with ${steps.login.body.json.token}\"]
    else:
      complain:
        cmd: [echo, login failed]

  # A switch: runs the steps of the first case whose value equals the variable.
  route:
    switch: steps.login.status
    cases:
      200:
        ok:
          cmd: [echo, ok]
    default:                            # when no case matches
      unexpected:
        cmd: [echo, \"unexpected status ${steps.login.status}\"]

  # A loop: runs `do` once for each element of a list.
  each_tag:
    for_each: steps.login.body.json.tags   # a list, without ${}
    as: TAG                             # default ITEM
    do:
      tag:
        cmd: [echo, \"tag ${TAG}\"]

  # Set state: new values for names in the job's `states`, saved at once for the next run.
  remember:
    set:
      states:
        PREVIOUS_STATUS: ${states.LAST_STATUS}  # every value is read before any is set
        LAST_STATUS: ${steps.login.status}

  # A step file: one step kept in its own file, relative to this file.
  # notify: steps/notify.yaml
";

const FULL_SET_VARS: &str = "
  # Update project vars: new values for names in the project's `vars`, seen by every job.
  rename:
    set:
      vars:
        USER: bob                       # was ${USER}
";

const FULL_SET_VARS_HINT: &str = "
  # Update project vars; needs the project's `vars` to declare USER.
  # rename:
  #   set:
  #     vars:
  #       USER: bob                     # was ${USER}
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
                format!(
                    "{FULL_PROJECT_HEADER}{}",
                    indent(&format!("{}{FULL_SET_VARS}", full_steps()), 4)
                )
            } else {
                PROJECT.to_string()
            },
            format!("run it: rocky exec {} example", path.display()),
        ),
        Kind::Job => {
            let name = path.file_stem().unwrap_or_default().to_string_lossy();
            (
                if full {
                    format!("{FULL_JOB_HEADER}{}{FULL_SET_VARS_HINT}", full_steps())
                } else {
                    JOB.to_string()
                },
                format!(
                    "add it to a project:\n  jobs:\n    {name}: {}",
                    path.display()
                ),
            )
        }
        Kind::Step { http } => {
            let name = path.file_stem().unwrap_or_default().to_string_lossy();
            (
                match (full, http) {
                    (false, true) => format!("{STEP_HEADER}{HTTP_STEP}"),
                    (false, false) => format!("{STEP_HEADER}{CMD_STEP}"),
                    (true, true) => {
                        format!("{STEP_HEADER}{FULL_STEP_HEADER}{FULL_HTTP_STEP}")
                    }
                    (true, false) => {
                        format!("{STEP_HEADER}{FULL_STEP_HEADER}{FULL_CMD_STEP}")
                    }
                },
                format!("add it to a job:\n  steps:\n    {name}: {}", path.display()),
            )
        }
    };
    write_new(path, &text)?;
    Ok(format!("created {}\n{hint}", path.display()))
}

/// Asks on `out` whether a step is HTTP or cmd, reading answers from `input` until one is
/// `http`, `cmd` or empty (cmd). End of input counts as empty.
pub fn ask_http(mut input: impl BufRead, mut out: impl Write) -> Result<bool, Error> {
    write!(out, "step type [cmd/http] (default cmd): ").map_err(Error::Prompt)?;
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

/// A job's `steps` with one step of every kind, using every field.
fn full_steps() -> String {
    format!(
        "# vars: {{USER: bob}}                 # values for this job's steps, checked before the project's\n\
         states:                             # kept between runs in states/<job>.json, read as ${{states.NAME}}\n  \
         LAST_STATUS: none                 # the default, used until `set` changes it\n  \
         PREVIOUS_STATUS: none\n\
         steps:\n  \
         # An HTTP request. Later steps read its result by name: ${{steps.login.status}}.\n  \
         login:\n{}\n  \
         # A command.\n  \
         shout:\n{}{FULL_CONTROL}",
        indent(FULL_HTTP_STEP, 4),
        indent(FULL_CMD_STEP, 4),
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
        let book = dir.path().join("project.yaml");
        create(Kind::Project, level, &book).unwrap();
        let mut project = project::load(&book).unwrap();
        let example = project.take("example").unwrap();
        if matches!(level, Level::Full) {
            assert!(
                example
                    .steps
                    .iter()
                    .any(|a| a.set.as_ref().is_some_and(|s| s.vars.contains_key("USER")))
            );
        }

        create(Kind::Job, level, &dir.path().join("jobs/w.yaml")).unwrap();
        let step = |http| Kind::Step { http };
        create(step(false), level, &dir.path().join("a/cmd.yaml")).unwrap();
        create(step(true), level, &dir.path().join("a/http.yaml")).unwrap();
        let book = dir.path().join("files.yaml");
        std::fs::write(
            &book,
            "jobs:\n  w: jobs/w.yaml\n  \
             a: {steps: {sh: a/cmd.yaml, web: a/http.yaml}}\n",
        )
        .unwrap();
        let mut project = project::load(&book).unwrap();
        let w = project.take("w").unwrap();
        if matches!(level, Level::Full) {
            assert!(w.states.contains_key("LAST_STATUS"));
            assert!(w.steps.iter().any(|a| {
                a.set
                    .as_ref()
                    .is_some_and(|s| s.states.contains_key("LAST_STATUS"))
            }));
        }
        let a = project.take("a").unwrap();
        assert!(a.steps[0].cmd.is_some() && a.steps[1].http.is_some());
        assert_eq!(a.steps[0].name, "sh");
        assert_eq!(a.steps[1].name, "web");
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
