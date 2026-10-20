# Schedules

A job's optional `schedule` tells [`rocky run`](run.md) when to run it. It has exactly one of `every`, `hourly`, `daily`, `weekly` or `at`. `rocky exec` checks it but doesn't run it.

```yaml
jobs:
  poll:
    schedule: {every: 5m}
    tasks: {date: {cmd: [date]}}
  report: jobs/report.yaml                   # its `schedule` goes in the job file
  manual: {tasks: {date: {cmd: [date]}}}     # not scheduled: `exec` only
```

A job has at most one schedule.

| Field | Runs | Example |
| --- | --- | --- |
| `every` | Repeatedly, this far apart. First run right away, or after `delay`. | `every: 30s` |
| `hourly` | Every hour at this minute (0-59). | `hourly: 15` |
| `daily` | Every day at this time. | `daily: "09:30"` |
| `weekly` | On these days of the week, at `time`. | `weekly: [mon, fri]`<br>`time: "09:30"` |
| `at` | Once, at this date and time. | `at: "2027-10-10T09:30:00"` |

## `every` and `delay`

```yaml
schedule:
  every: 5m
  delay: 10s   # optional
```

- A duration is a whole number followed by `s`, `m` or `h`: `30s`, `5m`, `2h`. Combined forms (`1h30m`) and other units (`1d`) are not supported; write `90m` or `24h`.
- `every` must be greater than 0.
- `delay` waits before the first run. Without it, the first run starts as soon as `run` starts. `delay` only works with `every`.
- If a run takes longer than the interval, the missed runs are skipped, and the next run starts at the next interval.

## `hourly`, `daily`, `weekly` and `at`

- They use the machine's **local time zone**.
- `hourly` takes a minute from 0 to 59.
- `daily` takes a time such as `"09:30"` or `"09:30:15"`. Quote it so YAML keeps it as text.
- `weekly` takes a list of days, each one of `mon`, `tue`, `wed`, `thu`, `fri`, `sat` or `sun`, and needs `time`, a time in the same form as `daily`. `time` only works with `weekly`.
- `at` takes a date and time such as `"2027-10-10T09:30:00"`, in local time; fractional seconds (`09:30:00.500`) are allowed. A time-zone offset such as `+09:00` is ignored, and a `Z` suffix fails. If it has already passed when `run` starts, the job is skipped (see [`run_on_start`](#run_on_start)).
- The first run is the next matching time after `run` starts, unless `run_on_start` is set. If a run is still going when later slots pass, those slots are skipped. When clocks change, an `hourly` slot in a repeated hour runs twice, and a slot in a skipped hour runs when the clock reaches it, later than written (`daily: "02:30"` runs at 03:30 on a spring-forward day).

## `run_on_start`

```yaml
schedule:
  weekly: [mon, fri]
  time: "09:30"
  run_on_start: true   # optional, default false
```

`run_on_start: true` runs the job once as soon as `run` starts, then follows the schedule. Slots that pass during that first run are skipped, except an `at` time, which then runs late since it's the only one. It works with `hourly`, `daily`, `weekly` and `at`.

An `at` time that is already in the past when `run` starts is not an error: without `run_on_start`, the job logs a `skipped` line (``{"event":"skipped","reason":"`at` is in the past"}``) and doesn't run, while the other jobs run as usual. With `run_on_start`, the job runs once at start. It doesn't work with `every`, which already runs at start unless `delay` is set.

## Errors

Schedules are checked whenever the project loads, by `exec` as well as `run`. The message is reported inside a YAML error with the line, e.g. ``invalid YAML in project.yaml`` caused by ``jobs.poll: invalid schedule: `5 min` is not a duration like `30s`, `5m` or `2h` at line 3 column 5``.

| Problem | Message |
|---|---|
| none or more than one of `every`/`hourly`/`daily`/`weekly`/`at` | ``invalid schedule: needs exactly one of `every`, `hourly`, `daily`, `weekly` or `at` `` |
| `delay` without `every` | ``invalid schedule: `delay` only works with `every` `` |
| `time` without `weekly` | ``invalid schedule: `time` only works with `weekly` `` |
| `weekly` without `time` | ``invalid schedule: `weekly` needs a `time` like "09:30"`` |
| `weekly: []` | ``invalid schedule: `weekly` needs at least one day`` |
| a bad day | ``invalid schedule: `monday` is not a day like `mon` `` |
| `run_on_start` with `every` | ``invalid schedule: `run_on_start` doesn't work with `every`, which runs at start unless `delay` is set`` |
| a bad duration | ``invalid schedule: `5 min` is not a duration like `30s`, `5m` or `2h` `` |
| `every: 0s` | ``invalid schedule: `every` must be greater than 0`` |
| `hourly` out of range | ``invalid schedule: `hourly: 60` must be a minute 0-59`` (a value above 127 gives a YAML type error instead) |
| a bad `daily`, `time` or `at` | a message showing the expected format |
