# Triggers

A job can run when another job asks for it, not only on its [schedule](schedules.md):

- a [`trigger` task](#trigger-tasks) in a job queues a run of another job of the project;
- a client of [`rocky run --socket`](#triggering-jobs-over-a-socket), such as `rocky trigger`, asks a running `run` for a job.

Either way, any job of the project can be triggered, scheduled or not.

## Trigger tasks

A task with `trigger` queues a run of another job of the project, and the job goes on with its next task without waiting. It can go anywhere a task can, and with [`when`](conditions.md) it triggers a job only when something happened:

```yaml
# jobs/backup.yaml
schedule: {daily: "03:00"}
tasks:
  dump:
    cmd: [pg_dump, -f, backup.sql, app]
    allow_failure: true
  alert:
    when: {this: tasks.dump.ok, eq: false}
    trigger: notify              # queue a run of `notify` (jobs/notify.yaml)
```

```yaml
# jobs/notify.yaml
tasks:
  send:
    cmd: [curl, -sS, --fail-with-body, -d, 'backup failed', https://example.com/alert]
```

- **In `run`:** the queued run starts right away, like one [triggered over the socket](#triggering-jobs-over-a-socket): it logs a `triggered` line, then its own lines, and it waits for any run of that job already going. The triggering job logs a `trigger` line. No `--socket` is needed.
- **In `rocky exec`:** the queued jobs run after the job, one at a time in the order they were triggered, together with any jobs they trigger. They run even when a task of the job exited non-zero, but not after an error such as an undefined variable or a timeout. stdout is still the job's own result. See [`rocky exec`](cli.md#rocky-exec-job).
- **Results:** a trigger saves nothing as `tasks.<name>` and can't fail; the triggered job's results stay in its own run.
- **Loops:** a job that triggers itself without a condition never stops.
- A trigger has no other fields, and its job must be a job of the project (a file in `jobs/`); both are checked when the project loads.

Example: [`examples/trigger/`](../examples/trigger/), whose `check` job triggers `alert` when the disk is nearly full and `report` every time.

## Triggering jobs over a socket

`rocky run --socket <PATH>` also listens on a Unix domain socket at `PATH`. A client can then run **any** job of the project, scheduled or not. With `--socket`, no job needs a `schedule`, and `run` runs until stopped.

Each request is one line of JSON, `{"job": "NAME"}`. Each request gets one JSON line back. A connection can send any number of requests. [`rocky trigger`](cli.md#rocky-trigger-socket-job) sends one request for you:

```sh
$ rocky run --socket /tmp/rocky.sock &
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
- **Stale socket:** stopping `run` leaves the socket file behind. The next `run` replaces it, unless another process still listens on it; then `run` stops at [startup](run.md#startup-checks).
