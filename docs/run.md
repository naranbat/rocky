# Run

`rocky run <project>` runs every job of the [project](projects.md) that has a `schedule`, until stopped with Ctrl-C, SIGTERM, SIGHUP or SIGQUIT, or until only finished `at` and `run_on_start`-only schedules are left. Jobs without a `schedule` are skipped; run them with `rocky exec`.

To keep `run` going in the background, see [Service](service.md).

```yaml
jobs:
  poll:
    schedule:
      every: 5m
      delay: 10s                     # optional: wait before the first run
    tasks:
      fetch:
        cmd: [curl, -sS, --fail-with-body, https://httpbin.org/get]

  morning-report:
    schedule:
      daily: "09:30"
    tasks:
      greet:
        cmd: [echo, "${USER}"]

  migrate: jobs/migrate.yaml     # a job file; its `schedule` (e.g. `at: "2027-10-10T09:30:00"`) is in the file
```

```sh
rocky run examples/httpbin.yaml
```

Example: [`examples/httpbin.yaml`](../examples/httpbin.yaml) schedules `login`, `mixed` and `conditional`; `health`, `loop`, `poll` and `files` are not scheduled.

## Fields used by `run`

| Field | Notes |
|---|---|
| `schedule` (job) | When to run the job. See [Schedules](schedules.md). |

The job name labels its log lines and errors.

## Startup checks

The whole project is loaded and checked before anything runs. Any error stops `run`:

- an invalid project, job, task or schedule (see [Validation](projects.md#validation))
- no job with a `schedule`: ``invalid project: no job has a `schedule` `` (allowed with `--socket`)
- with `--socket`: a socket another `rocky run` still listens on (``socket <PATH> is in use by another process``), or one that can't be created

## Output

Every log line is one JSON object on **stdout**, with `job`, `time` (local time with offset) and `event` fields. The same lines are appended to `.rocky/logs/YYYY-MM-DD.log` in the current directory, one file per day by each line's date; the `.rocky/logs` directory is created if missing, and a failed write is reported on stderr without stopping `run`. If stdout is closed (`run | head`), `run` keeps running and logging to the file. Each run writes a line per task, branch or switch decision, loop and `while` check as it goes, then a `done` line:

```json
{"job":"poll","time":"2026-10-09T09:30:00+09:00","event":"task","task":"fetch","exit_code":0,"ms":412,"ok":true}
{"job":"poll","time":"2026-10-09T09:30:00+09:00","event":"done","failed_task":null,"reason":null}
```

| `event` | Other fields |
|---|---|
| `task` | `task` (the task's key), then `exit_code` or `error` (a timeout with `allow_failure`), `ms`, `ok`, and `allowed: true` when it failed with `allow_failure`, or inside a loop with `on_failure: next` |
| `branch` | `task`, `arm` (`then` or `else`), `tasks` (how many entries the chosen arm has; nested tasks and loop iterations aren't counted) |
| `switch` | `task`, `case` (the matching case's number, from 1, or `"default"`), `tasks` (how many entries the chosen case has) |
| `for_each` | `task`, `var`, `items` (the list length) |
| `while` | `task`, `iteration` (which check this is, from 1), `holds` (whether the condition held, so `do` runs again) |
| `done` | `failed_task`: the task that failed without `allow_failure`, or `null`; `reason`: why it failed (`exited with code 3`), or `null` |
| `error` | `error`: why the run stopped, e.g. an undefined variable, a project that became invalid, or a crash |
| `stopped` | `error`: why the job's schedule stopped |
| `skipped` | `reason`: why the job won't run, e.g. its `at` time has passed |
| `trigger` | `task`, `target`: a [`trigger` task](#triggering-jobs-from-a-task) queued a run of the job `target` |
| `triggered` | — (a client of the [socket](#triggering-jobs-over-a-socket) or a [`trigger` task](#triggering-jobs-from-a-task) asked for a run; the run's own lines follow once it starts) |

```json
{"job":"poll","time":"2026-10-09T09:30:00+09:00","event":"error","error":"task `get` failed: variable `TOKEN` is not defined (checked tasks, loop items, values, env_file, environment)"}
```

A task that ends in an error gets no `task` line; the `error` line names it. See also [Debugging](debugging.md#run-log-lines).

## Behaviour

- **Independent jobs:** each job runs on its own schedule, concurrently with the others.
- **No overlap:** a job never overlaps itself. If a run takes longer than its interval, the missed runs are skipped. A run [triggered over the socket](#triggering-jobs-over-a-socket) while the job is running waits for that run to finish, and so does a scheduled run that comes due during a triggered one.
- **Live edits:** the project and its job and task files are re-read on every run, so edits to tasks and the env file apply from the next run. Schedules are read only at startup.
- **Failures don't stop the job:** after a run that failed, errored or crashed, the job keeps its schedule and the other jobs are unaffected. If a job's schedule itself can't be computed, that job stops with a `stopped` line, and the other jobs keep running.
- **Exit:** with `--socket`, `run` runs until stopped. Without it, `run` exits once no job has runs left: every scheduled job is an `at` job that has already run or was skipped, or has stopped after a `stopped` error, and no run a [`trigger` task](#triggering-jobs-from-a-task) queued is waiting or still going.

## Triggering jobs from a task

A task with `trigger` queues a run of another job of the project, scheduled or not, and the job goes on with its next task without waiting. It can go anywhere a task can, so a branch can trigger a job only when something happened:

```yaml
jobs:
  backup:
    schedule: {daily: "03:00"}
    tasks:
      dump:
        cmd: [pg_dump, -f, backup.sql, app]
        allow_failure: true
      check:
        if: {this: tasks.dump.ok, eq: false}
        then:
          alert: {trigger: notify}   # queue a run of `notify`
  notify:
    tasks:
      send:
        cmd: [curl, -sS, --fail-with-body, -d, 'backup failed', https://example.com/alert]
```

- **In `run`:** the queued run starts right away, like one [triggered over the socket](#triggering-jobs-over-a-socket): it logs a `triggered` line, then its own lines, and it waits for any run of that job already going. The triggering job logs a `trigger` line. No `--socket` is needed.
- **In `rocky exec`:** the queued jobs run after the job, one at a time in the order they were triggered, together with any jobs they trigger. stdout is still the job's own result. See [`rocky exec`](cli.md#rocky-exec-project-job).
- **Results:** a trigger saves nothing as `tasks.<name>` and can't fail; the triggered job's results stay in its own run.
- **Loops:** a job that triggers itself without a condition never stops.
- A trigger has no other fields, and its job must be in the project's `jobs`; both are checked when the project loads.

Example: [`examples/trigger.yaml`](../examples/trigger.yaml), whose `check` job triggers `alert` when the disk is nearly full and `report` every time.

## Triggering jobs over a socket

`rocky run <project> --socket <PATH>` also listens on a Unix domain socket at `PATH`. A client can then run **any** job of the project, scheduled or not. With `--socket`, no job needs a `schedule`.

Each request is one line of JSON, `{"job": "NAME"}`. Each request gets one JSON line back. A connection can send any number of requests. [`rocky trigger`](cli.md#rocky-trigger-socket-job) sends one request for you:

```sh
$ rocky run project.yaml --socket /tmp/rocky.sock &
$ rocky trigger /tmp/rocky.sock report
queued `report`
```

Any other client works too:

```sh
$ echo '{"job": "report"}' | socat - UNIX-CONNECT:/tmp/rocky.sock
{"ok":true}
$ echo '{"job": "deploy"}' | socat - UNIX-CONNECT:/tmp/rocky.sock
{"error":"no job `deploy` (available: `poll`, `report`)"}
```

| Reply | When |
|---|---|
| `{"ok":true}` | The job exists and a run of it is queued |
| `{"error":"..."}` | The request isn't `{"job": "NAME"}` (``invalid request: …``), the job doesn't exist, or the project no longer loads |

The reply doesn't wait for the run. A `triggered` line is logged, and the run then logs its lines and a `done` or `error` line like a scheduled run. A request for a job that is already running is queued: it starts once the runs before it have finished, in the order they arrived.

- **Access:** the socket is created readable and writable by its owner only (`0600`). Anyone who can connect can run any job.
- **Stale socket:** stopping `run` leaves the socket file behind. The next `run` replaces it, unless another process still listens on it.
