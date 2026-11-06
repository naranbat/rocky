# Run

`rocky run` runs every job of the [project](projects.md) in the current directory that has a [`schedule`](schedules.md), until stopped with Ctrl-C, SIGTERM, SIGHUP or SIGQUIT, or until only finished `at` and `run_on_start`-only schedules are left. Jobs without a `schedule` are skipped; run them with `rocky exec`, a [`trigger` task](triggers.md#trigger-tasks), or over a [socket](triggers.md#triggering-jobs-over-a-socket).

To keep `run` going in the background, see [Service](service.md).

```yaml
# jobs/poll.yaml
schedule:
  every: 5m
  delay: 10s                     # optional: wait before the first run
tasks:
  fetch:
    cmd: [curl, -sS, --fail-with-body, https://httpbin.org/get]
```

```yaml
# jobs/morning-report.yaml
schedule:
  daily: "09:30"
tasks:
  greet:
    cmd: [echo, "${USER}"]
```

```sh
cd examples/httpbin && rocky run
```

Example: [`examples/httpbin/`](../examples/httpbin/) schedules `login`, `mixed`, `conditional` and `status`; `health`, `loop`, `poll` and `report` are not scheduled.

## Startup checks

The whole project is loaded and checked before anything runs. Any error stops `run`:

- an invalid project, job, task or schedule (see [Validation](projects.md#validation))
- no job with a `schedule`: ``invalid project: no job has a `schedule` `` (allowed with `--socket`)
- with `--socket`: a socket another `rocky run` still listens on (``socket <PATH> is in use by another process``), or one that can't be created

## Behaviour

- **Independent jobs:** each job runs on its own schedule, concurrently with the others. The job name labels its log lines and errors.
- **No overlap:** a job never overlaps itself. If a run takes longer than its interval, the missed runs are skipped. A [triggered](triggers.md) run that arrives while the job is running waits for that run to finish, and so does a scheduled run that comes due during a triggered one.
- **Live edits:** `rocky.yaml` and the job files are re-read on every run, so edits to tasks and the env file apply from the next run. Schedules are read only at startup.
- **Failures don't stop the job:** after a run that failed, errored or crashed, the job keeps its schedule and the other jobs are unaffected. If a job's schedule itself can't be computed, that job stops with a `stopped` line, and the other jobs keep running.
- **Exit:** with `--socket`, `run` runs until stopped. Without it, `run` exits once no job has runs left: every scheduled job is an `at` or `run_on_start`-only job that has already run or was skipped, or has stopped after a `stopped` error, and no run a [`trigger` task](triggers.md#trigger-tasks) queued is waiting or still going.

## Output

Every log line is one JSON object on **stdout**, with `job`, `time` (local time with offset) and `event` fields. The same lines are appended to `.rocky/logs/YYYY-MM-DD.log` in the current directory, one file per day by each line's date (`.rocky/logs/NAME-YYYY-MM-DD.log` with `--service NAME`); the `.rocky/logs` directory is created if missing, and a failed write is reported on stderr without stopping `run`. If stdout is closed (`run | head`), `run` keeps running and logging to the file. Each run writes a line per task, skipped task, loop, `while` check and `trigger` task as it goes, then a `done` line, or an `error` line if it stopped on an error:

```json
{"job":"poll","time":"2026-10-09T09:30:00+09:00","event":"task","task":"fetch","exit_code":0,"ms":412,"ok":true}
{"job":"poll","time":"2026-10-09T09:30:00+09:00","event":"done","failed_task":null,"reason":null}
```

| `event` | Meaning and other fields |
|---|---|
| `task` | A cmd task finished, or one run of a task with [`retry`](cmd-tasks.md#retry) did. `task` (the task's key), then `exit_code` or `error` (a timeout with `allow_failure`), `ms`, `ok`, and `allowed: true` when it failed and the job continued: with `allow_failure`, inside a loop with `on_failure: next`, or on a `retry` run before the last. `error` also appears for a timeout in those cases |
| `skip` | `task`, for a task whose [`when`](conditions.md) doesn't hold |
| `for_each` | A [`for_each` loop](loops.md#for_each) started: `task`, `var` (the `for_each` path, e.g. `tasks.users.stdout.items`), `items` (the list length) |
| `while` | A [`while` loop](loops.md#while) checked its condition: `task`, `iteration` (which check this is, from 1), `holds` (whether the condition held, so `do` runs again) |
| `trigger` | `task`, `target`: a [`trigger` task](triggers.md#trigger-tasks) queued a run of the job `target` |
| `triggered` | — (a client of the [socket](triggers.md#triggering-jobs-over-a-socket) or a [`trigger` task](triggers.md#trigger-tasks) asked for a run; the run's own lines follow once it starts) |
| `done` | The run finished. `failed_task`: the task that failed without `allow_failure`, or `null`; `reason`: why it failed (`exited with code 3`), or `null` |
| `error` | `error`: why the run stopped, e.g. an undefined variable, a project that became invalid, or a crash |
| `stopped` | `error`: why the job's schedule stopped |
| `skipped` | `reason`: why the job won't run, e.g. its `at` time has passed |

```json
{"job":"poll","time":"2026-10-09T09:30:00+09:00","event":"error","error":"task `get` failed: variable `TOKEN` is not defined (checked tasks, loop items, values, env_file, environment)"}
```

A task that can't start or finish as a command result (for example, an undefined variable or a missing program) gets no `task` line; the `error` line names it. A program-reported failure, such as `curl` exiting non-zero after a DNS or connection error, does get a `task` line with `ok: false` and its `exit_code`. See also [Debugging](debugging.md).
