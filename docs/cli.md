# Command line

`rocky` has five subcommands. `exec`, `run` and `validate` work on a [project](projects.md): `rocky.yaml` in the **current directory**, or the project file given with `-f <FILE>` (`--file`). `trigger` asks a running `run` to run a job; `new` writes starter files.

```sh
rocky exec <JOB>
rocky exec <JOB> --verbose
rocky exec -f <FILE> <JOB>
rocky run
rocky run -f <FILE>
rocky run --socket <PATH>
rocky run --service <NAME>
rocky trigger <SOCKET> <JOB>
rocky validate
rocky validate -f <FILE>
rocky new project [DIR]
rocky new job <NAME>
rocky new <KIND> ... --level full
rocky --help          # or -h, also per subcommand: rocky exec --help
rocky --version       # or -V
```

## `rocky exec <JOB>`

Runs one job of the project, the file listed as `<JOB>` under its `jobs`, once and prints the result of the **last task that ran** as pretty-printed JSON on stdout. Job `schedule`s are checked but not run.

The job name is required. An unknown name fails before anything runs and lists the names there are:

```
$ rocky exec deploy
Error: no job `deploy` (available: `login`, `report`)
```

Jobs that [`trigger` tasks](triggers.md#trigger-tasks) queue run after the job, one at a time in the order they were triggered, together with any jobs they trigger. They run even when a task of the job exited non-zero. Only the job's own result is printed. If a triggered job's task exits non-zero, `exec` exits `1` with ``Error: triggered job `report`: task `send` exited with code 22``.

An error that stops a job (an undefined variable, a timeout, a missing program; see [Errors and exit codes](#errors-and-exit-codes)) ends `exec` at once: if it is the job itself, nothing is printed and no queued job runs; if it is a triggered job, the error is reported as for the job itself, without the ``triggered job`` prefix, and the jobs still queued don't run.

`--verbose` (or `-v`) prints each command to stderr just before it runs, with `${VAR}`s filled in, so values from the `env_file` (secrets included) show. Pipe stages are joined with `|`, and an argument with spaces or shell characters is single-quoted:

```
$ rocky exec login --verbose > result.json
+ [login] curl -s -H 'Authorization: Bearer abc123' https://example.com/login | jq .token
```

## `rocky run`

Runs every job of the project that has a [`schedule`](schedules.md), until stopped with Ctrl-C, SIGTERM, SIGHUP or SIGQUIT, or until only finished `at` and `run_on_start`-only schedules are left. Each run writes [JSON log lines](run.md#output) to stdout. See [Run](run.md).

`--socket <PATH>` also listens on a Unix domain socket, so clients can run any job by sending `{"job": "NAME"}`. With it, no job needs a `schedule` and `run` runs until stopped. See [Triggering jobs over a socket](triggers.md#triggering-jobs-over-a-socket).

`--service <NAME>` prefixes the log file names with `NAME`: `.rocky/logs/NAME-YYYY-MM-DD.log`, so services sharing a working directory keep separate files. See [Output](run.md#output).

`-f <FILE>` suffixes them with the project file's name without its extension: `rocky run -f nightly.yaml` writes `.rocky/logs/YYYY-MM-DD-nightly.log` (`NAME-YYYY-MM-DD-nightly.log` with `--service NAME`). Without `-f` the names have no suffix.

## `rocky trigger <SOCKET> <JOB>`

Asks the `rocky run --socket <SOCKET>` listening on `SOCKET` to run `JOB`, any job of its project. It returns once the run is queued and prints one line; the run logs to `run`'s output. See [Triggering jobs over a socket](triggers.md#triggering-jobs-over-a-socket).

```
$ rocky trigger /tmp/rocky.sock report
queued `report`
$ rocky trigger /tmp/rocky.sock deploy
Error: no job `deploy` (available: `poll`, `report`)
```

If nothing listens on `SOCKET`, it fails with ``Error: socket <SOCKET>`` and the cause on a `Caused by:` line.

## `rocky validate`

Loads the project file and the job files it lists and runs every [load-time check](projects.md#validation), without running commands. A valid project prints one line and exits `0`; an invalid one prints the load error on stderr and exits `1`.

```
$ rocky validate
project is valid (jobs: `login`, `report`)
```

## `rocky new <KIND>`

Writes starter files that load as-is, creating missing directories, and prints how to use them. It never overwrites: an existing file fails with ``Error: <PATH> already exists``.

| Kind | Writes |
|---|---|
| `project [DIR]` | A project in `DIR` (default: the current directory): a `rocky.yaml` listing one job, `jobs/example.yaml`, that runs `echo hello`: `rocky exec example` there |
| `job <NAME>` | `jobs/NAME.yaml` in the current directory, a [job](projects.md#job-files) with one `echo hello` task. It isn't added to `rocky.yaml`: list it under `jobs` (`NAME: jobs/NAME.yaml`), then `rocky exec NAME` |

`--level` sets how much the file shows, for every kind:

| Level | Writes |
|---|---|
| `basic` (default) | The smallest file that runs, as in the table above |
| `full` | Every field, each with a comment. A full project's `rocky.yaml` also has `values` and `defaults`. A full job has a `schedule`, `values`, `defaults`, a `curl` request, cmd tasks with `when`, a `for_each` loop and a `while` loop. Fields that need another file (`env_file`) or another job (`trigger`) are commented out, so the file still loads and runs as-is. Its tasks use `${USER}` from the environment |

```
$ rocky new job deploy
created jobs/deploy.yaml
add it to the project's `jobs`: deploy: jobs/deploy.yaml
then run it: rocky exec deploy
```

## Output streams

- **stdout** holds only results: one JSON document for `exec`, one JSON log line per [event](run.md#output) for `run`. It is safe to pipe into `jq`.
- **stderr** holds errors, `exec --verbose`'s command lines and `run`'s ``cannot write log file`` messages.

## Errors and exit codes

| Situation | stdout | stderr | Exit code |
|---|---|---|---|
| Success | last task's result, or `null` if no cmd task ran (`validate`: ``project is valid (jobs: …)``) | — | `0` |
| A task fails (non-zero exit) without `allow_failure` | that task's result | ``Error: task `name` exited with code 3`` | `1` |
| `exec`: a triggered job's task fails | the job's own result | ``Error: triggered job `x`: task `name` exited with code 3`` | `1` |
| Any other task error | — | ``Error: task `name` failed`` plus `Caused by:` lines, e.g. ``variable `NAME` is not defined (checked tasks, loop items, values, env_file, environment)``, ``failed to start command `x` `` or ``command `x` timed out after 5s`` | `1` |
| Invalid file (bad YAML, unknown field, invalid task, condition or schedule) | — | the load error; nothing runs | `1` |
| `exec` or `run` stopped with Ctrl-C (SIGINT), SIGTERM, SIGHUP or SIGQUIT | — | — | 128 + the signal number (`130`, `143`, `129`, `131`); running commands, and the processes they started, are killed |
| Unknown job name | — | ``Error: no job `x` (available: `a`, `b`)`` | `1` |
| No project file | — | ``Error: failed to read rocky.yaml`` plus a `Caused by:` line | `1` |
| No `jobs` in the project file | — | ``Error: invalid project: no jobs: add `jobs: {<name>: <path>.yaml}` to rocky.yaml`` | `1` |
| `trigger`: nothing listens on the socket | — | ``Error: socket <SOCKET>`` plus a `Caused by:` line | `1` |

Tasks are labelled by their key in error messages, at any depth: a task `notify` inside a loop's `do` is ``task `notify` ``.

> YAML errors in any file (the project file or a job file) are reported as `invalid YAML in <path>`, with the cause on a `Caused by:` line.
