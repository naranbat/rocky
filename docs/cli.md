# Command line

`rocky` has five subcommands. `exec`, `run` and `validate` take a [project](projects.md) file as their first positional argument; `trigger` asks a running `run` to run a job; `new` writes starter files.

```sh
rocky exec <PROJECT> <JOB>
rocky run <PROJECT>
rocky run <PROJECT> --socket <PATH>
rocky trigger <SOCKET> <JOB>
rocky validate <PROJECT>
rocky new project <PATH>
rocky new job <PATH>
rocky new task <PATH>
rocky new <KIND> <PATH> --level full
rocky --help          # or -h, also per subcommand: rocky exec --help
rocky --version       # or -V
```

## `rocky exec <PROJECT> <JOB>`

Runs one job of a project once and prints the result of the **last task that ran** as pretty-printed JSON on stdout. Job `schedule`s are checked but not run.

The job name is required. An unknown name fails before anything runs and lists the names there are:

```
$ rocky exec project.yaml deploy
Error: no job `deploy` (available: `login`, `report`)
```

## `rocky run <PROJECT>`

Runs every job of the project that has a [`schedule`](schedules.md), until stopped with Ctrl-C, SIGTERM, SIGHUP or SIGQUIT, or until only finished `at` schedules are left. Each run writes [JSON log lines](run.md#output) to stdout. See [Run](run.md).

`--socket <PATH>` also listens on a Unix domain socket, so clients can run any job by sending `{"job": "NAME"}`. With it, no job needs a `schedule` and `run` runs until stopped. See [Triggering jobs over a socket](run.md#triggering-jobs-over-a-socket).

## `rocky trigger <SOCKET> <JOB>`

Asks the `rocky run --socket <SOCKET>` listening on `SOCKET` to run `JOB`, any job of its project. It returns once the run is queued and prints one line; the run logs to `run`'s output. See [Triggering jobs over a socket](run.md#triggering-jobs-over-a-socket).

```
$ rocky trigger /tmp/rocky.sock report
queued `report`
$ rocky trigger /tmp/rocky.sock deploy
Error: no job `deploy` (available: `poll`, `report`)
```

If nothing listens on `SOCKET`, it fails with ``Error: socket <SOCKET>`` and the cause on a `Caused by:` line.

## `rocky validate <PROJECT>`

Loads the project and its job and task files and runs every [load-time check](projects.md#validation), without sending requests or running commands. A valid project prints one line and exits `0`; an invalid one prints the load error on stderr and exits `1`.

```
$ rocky validate project.yaml
project.yaml is valid (jobs: `login`, `report`)
```

## `rocky new <KIND> <PATH>`

Writes a starter file that loads as-is, creating missing parent directories, and prints how to use it. It never overwrites: an existing file fails with ``Error: <PATH> already exists``.

| Kind | Writes |
|---|---|
| `project` | A project with one job, `example`, that runs `echo hello`: `rocky exec <PATH> example` |
| `job` | A [job file](projects.md#job-and-task-files) with one `echo hello` task, to list under a project's `jobs` |
| `task` | A [task file](projects.md#job-and-task-files) to list under a job's `tasks`, e.g. `login: tasks/login.yaml`. It asks `cmd` or `http` (default `cmd`): a cmd task, or an HTTP GET. Scripts can pipe the answer: `echo http \| rocky new task tasks/login.yaml` |

`--level` sets how much the file shows, for every kind:

| Level | Writes |
|---|---|
| `basic` (default) | The smallest file that runs, as in the table above |
| `full` | Every field, each with a comment. A full project or job has a `schedule`, `values`, an HTTP task, a cmd task, a branch, a switch and a loop. A full task is an HTTP or cmd task with all of its fields. Fields that need another file (`env_file`, a task file) are commented out, so the file still loads and runs as-is. Its tasks use `${USER}` from the environment |

```
$ rocky new job jobs/deploy.yaml
created jobs/deploy.yaml
add it to a project:
  jobs:
    deploy: jobs/deploy.yaml
```

## Output streams

- **stdout** holds only results: one JSON document for `exec`, one JSON log line per task, branch, switch, loop, finished run, error and stopped schedule for `run`. It is safe to pipe into `jq`.
- **stderr** holds errors, the `new task` prompt, and `run`'s ``cannot write log file`` messages.

## Errors and exit codes

| Situation | stdout | stderr | Exit code |
|---|---|---|---|
| Success | last task's result, or `null` if no HTTP or cmd task ran (`validate`: ``<PROJECT> is valid (jobs: …)``) | — | `0` |
| A task fails (HTTP 4xx/5xx or non-zero exit) without `allow_failure` | that task's result | ``Error: task `name` returned HTTP 404`` or ``Error: task `name` exited with code 3`` | `1` |
| Any other task error | — | ``Error: task `name` failed`` plus `Caused by:` lines, e.g. ``variable `NAME` is not defined (checked tasks, loop items, values, env_file, environment)``, ``failed to start command `x` `` or ``command `x` timed out after 5s`` | `1` |
| Invalid file (bad YAML, unknown field, invalid task, condition or schedule) | — | the load error; nothing runs | `1` |
| `exec` or `run` stopped with Ctrl-C (SIGINT), SIGTERM, SIGHUP or SIGQUIT | — | — | 128 + the signal number (`130`, `143`, `129`, `131`); running commands, and the processes they started, are killed |
| Unknown job name | — | ``Error: no job `x` (available: `a`, `b`)`` | `1` |
| `trigger`: nothing listens on the socket | — | ``Error: socket <SOCKET>`` plus a `Caused by:` line | `1` |

Tasks are labelled by their key in error messages, at any depth: a task `notify` inside a loop's `do` is ``task `notify` ``.

> YAML errors in any file (project, job or task file) are reported as `invalid YAML in <path>`, with the cause on a `Caused by:` line.
