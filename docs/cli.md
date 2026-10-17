# Command line

`rocky` has four subcommands. `exec`, `run` and `validate` take a [project](projects.md) file as their first positional argument; `new` writes starter files.

```sh
rocky exec <PROJECT> <JOB>
rocky run <PROJECT>
rocky validate <PROJECT>
rocky new project <PATH>
rocky new job <PATH>
rocky new step <PATH>
rocky new <KIND> <PATH> --level full
rocky --help          # or -h, also per subcommand: rocky exec --help
rocky --version       # or -V
```

## `rocky exec <PROJECT> <JOB>`

Runs one job of a project once and prints the result of the **last step that ran** as pretty-printed JSON on stdout. The project's `schedules` are checked but not run.

The job name is required. An unknown name fails before anything runs and lists the names there are:

```
$ rocky exec project.yaml deploy
Error: no job `deploy` (available: `login`, `report`)
```

## `rocky run <PROJECT>`

Runs every job listed in the project's [`schedules`](schedules.md), until stopped with Ctrl-C, SIGTERM, SIGHUP or SIGQUIT, or until only finished `at` schedules are left. Each run writes [JSON log lines](run.md#output) to stdout. See [Run](run.md).

## `rocky validate <PROJECT>`

Loads the project and its job and step files and runs every [load-time check](projects.md#validation), without sending requests or running commands. A valid project prints one line and exits `0`; an invalid one prints the load error on stderr and exits `1`.

```
$ rocky validate project.yaml
project.yaml is valid (jobs: `login`, `report`)
```

## `rocky new <KIND> <PATH>`

Writes a starter file that loads as-is, creating missing parent directories, and prints how to use it. It never overwrites: an existing file fails with ``Error: <PATH> already exists``.

| Kind | Writes |
|---|---|
| `project` | A project with one job, `example`, that runs `echo hello`: `rocky exec <PATH> example` |
| `job` | A [job file](projects.md#job-and-step-files) with one `echo hello` step, to list under a project's `jobs` |
| `step` | A [step file](projects.md#job-and-step-files) to list under a job's `steps`, e.g. `login: steps/login.yaml`. It asks `cmd` or `http` (default `cmd`): a cmd step, or an HTTP GET. Scripts can pipe the answer: `echo http \| rocky new step steps/login.yaml` |

`--level` sets how much the file shows, for every kind:

| Level | Writes |
|---|---|
| `basic` (default) | The smallest file that runs, as in the table above |
| `full` | Every field, each with a comment. A full project or job has `states`, an HTTP step, a cmd step, a branch, a switch, a loop and a `set` of `states`, and the project adds `vars`, `schedules` and a `set` of `vars`. A full step is an HTTP or cmd step with all of its fields. Fields that need another file (`env_file`, a step file) are commented out, so the file still loads and runs as-is. Its steps use `${USER}` from the project's `vars`; a full project defines it |

```
$ rocky new job jobs/deploy.yaml
created jobs/deploy.yaml
add it to a project:
  jobs:
    deploy: jobs/deploy.yaml
```

## Output streams

- **stdout** holds only results: one JSON document for `exec`, one JSON log line per step, branch, switch, loop, `set`, finished run, error and stopped schedule for `run`. It is safe to pipe into `jq`.
- **stderr** holds errors, the `new step` prompt, and `run`'s ``cannot write log file`` messages.

## Errors and exit codes

| Situation | stdout | stderr | Exit code |
|---|---|---|---|
| Success | last step's result, or `null` if no HTTP or cmd step ran (`validate`: ``<PROJECT> is valid (jobs: …)``) | — | `0` |
| A step fails (HTTP 4xx/5xx or non-zero exit) without `allow_failure` | that step's result | ``Error: step `name` returned HTTP 404`` or ``Error: step `name` exited with code 3`` | `1` |
| Any other step error | — | ``Error: step `name` failed`` plus `Caused by:` lines, e.g. ``variable `NAME` is not defined (checked steps, states, loop items, vars, env_file, environment)``, ``failed to start command `x` `` or ``command `x` timed out after 5s`` | `1` |
| Invalid file (bad YAML, unknown field, invalid step, condition or schedule) | — | the load error; nothing runs | `1` |
| `exec` or `run` stopped with Ctrl-C (SIGINT), SIGTERM, SIGHUP or SIGQUIT | — | — | 128 + the signal number (`130`, `143`, `129`, `131`); running commands, and the processes they started, are killed |
| Unknown job name | — | ``Error: no job `x` (available: `a`, `b`)`` | `1` |

Steps are labelled by their key in error messages, at any depth: a step `notify` inside a loop's `do` is ``step `notify` ``.

> YAML errors in any file (project, job or step file) are reported as `invalid YAML in <path>`, with the cause on a `Caused by:` line.
