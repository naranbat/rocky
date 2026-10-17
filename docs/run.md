# Run

`rocky run <project>` runs every job listed in the [project's](projects.md) `schedules`, until stopped with Ctrl-C, SIGTERM, SIGHUP or SIGQUIT, or until only finished `at` schedules are left. Jobs that aren't listed are skipped; run them with `rocky exec`.

```yaml
vars:
  BASE: https://httpbin.org
schedules:
  poll:
    every: 5m
    delay: 10s                       # optional: wait before the first run
  morning-report:
    daily: "09:30"
  migrate:
    at: "2027-10-10T09:30:00"        # once
jobs:
  poll:
    steps:
      get:
        get: "${BASE}/get"

  morning-report:
    steps:
      greet:
        cmd: [echo, "${USER}"]

  migrate: jobs/migrate.yaml     # a job file
```

```sh
rocky run examples/httpbin.yaml
```

Example: [`examples/httpbin.yaml`](../examples/httpbin.yaml) schedules `login`, `mixed` and `conditional`; `health`, `loop` and `files` are not scheduled.

## Fields used by `run`

| Field | Notes |
|---|---|
| `schedules` (project) | Job name → when to run it. See [Schedules](schedules.md). |

The job name labels its log lines and errors.

## Startup checks

The whole project is loaded and checked before anything runs. Any error stops `run`:

- an invalid project, job, step or schedule (see [Validation](projects.md#validation))
- no `schedules`: ``invalid project: project has no `schedules` ``

## Output

Every log line is one JSON object on **stdout**, with `job`, `time` (local time with offset) and `event` fields. The same lines are appended to `logs/YYYY-MM-DD.log` in the current directory, one file per day by each line's date; the `logs` directory is created if missing, and a failed write is reported on stderr without stopping `run`. If stdout is closed (`run | head`), `run` keeps running and logging to the file. Each run writes a line per step, branch or switch decision, loop and `set` as it goes, then a `done` line:

```json
{"job":"poll","time":"2026-10-09T09:30:00+09:00","event":"step","step":"get","status":200,"ms":412,"ok":true}
{"job":"poll","time":"2026-10-09T09:30:00+09:00","event":"done","failed_step":null,"reason":null}
```

| `event` | Other fields |
|---|---|
| `step` | `step` (the step's key), then `status` (HTTP), `exit_code` (cmd) or `error` (a timeout or network error with `allow_failure`), `ms`, `ok`, and `allowed: true` when it failed with `allow_failure` |
| `branch` | `step`, `arm` (`then` or `else`), `steps` (how many entries the chosen arm has; nested steps and loop iterations aren't counted) |
| `switch` | `step`, `case` (the matching case's number, from 1, or `"default"`), `steps` (how many entries the chosen case has) |
| `for_each` | `step`, `var`, `items` (the list length) |
| `set` | `step`, `states` and `vars` (the names it set, `[]` if none; values are left out) |
| `done` | `failed_step`: the step that failed without `allow_failure`, or `null`; `reason`: why it failed (`returned HTTP 500`, `exited with code 3`), or `null` |
| `error` | `error`: why the run stopped, e.g. an undefined variable, a project that became invalid, or a crash |
| `stopped` | `error`: why the job's schedule stopped |
| `skipped` | `reason`: why the job won't run, e.g. its `at` time has passed |

```json
{"job":"poll","time":"2026-10-09T09:30:00+09:00","event":"error","error":"step `get` failed: variable `TOKEN` is not defined (checked steps, states, loop items, vars, env_file, environment)"}
```

A step that ends in an error gets no `step` line; the `error` line names it. See also [Debugging](debugging.md#run-log-lines).

## Behaviour

- **Independent jobs:** each job runs on its own schedule, concurrently with the others.
- **No overlap:** a job never overlaps itself. If a run takes longer than its interval, the missed runs are skipped.
- **Live edits:** the project and its job and step files are re-read on every run, so edits to steps and `vars` apply from the next run, except a project var a `set` has updated, which keeps its updated value until `run` stops (see [Variables](variables.md#updating-vars-at-runtime)). Schedules are read only at startup.
- **Failures don't stop the job:** after a run that failed, errored or crashed, the job keeps its schedule and the other jobs are unaffected. If a job's schedule itself can't be computed, that job stops with a `stopped` line, and the other jobs keep running.
- **Exit:** `run` exits once no job has runs left: every scheduled job is an `at` job that has already run or was skipped, or has stopped after a `stopped` error.
