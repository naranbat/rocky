---
name: rocky
description: Write, validate and run rocky projects, a project file listing YAML job files whose tasks run commands in order, pass output between tasks, run tasks conditionally, loop, and run on schedules. Use when the user wants to automate repeatable work with rocky, edits a rocky `rocky.yaml`, job or task YAML file, or runs `rocky exec`, `rocky run`, `rocky trigger`, `rocky validate` or `rocky new`.
---

# rocky

rocky runs **projects**: a project file, `rocky.yaml` by default, whose `jobs` map each **job** name to its YAML file (`NAME: jobs/NAME.yaml`), plus settings the jobs share. `exec`, `run` and `validate` load `rocky.yaml` in the **current directory**, or the project file given with `-f FILE`. A job is a map of named **tasks** that run one at a time, in the order written. A task runs a command (no shell), and later tasks read earlier tasks' output. `rocky exec` runs one job once; `rocky run` runs jobs on their `schedule`.

## Workflow

```sh
rocky new project [DIR]                      # starter project: rocky.yaml + jobs/example.yaml in DIR (default .)
rocky new project full --level full          # every field, with comments: the full reference
rocky new job deploy                         # jobs/deploy.yaml; then list it under `jobs` in rocky.yaml
rocky validate                               # load-time checks only, runs nothing
rocky exec JOB                               # run job JOB once, print last task's result as JSON
rocky exec -f other.yaml JOB                 # same, with another project file
rocky exec JOB --verbose                     # also print each command to stderr before it runs
```

1. Write or edit the YAML.
2. Run `rocky validate` and fix every error before going further.
3. Run `rocky exec ... --verbose` to try the job.

There is **no dry run**: `exec` runs the real commands. Before executing a job with side effects (deploys, POST/DELETE requests, writing or deleting files, messages), confirm with the user. `--verbose` prints filled-in arguments, including secrets from the env file.

`rocky new` never overwrites; it fails with `<PATH> already exists`.

## Example

```yaml
# rocky.yaml
values:                      # fixed values, read as ${NAME}; names are UPPERCASE
  BASE: https://httpbin.org
jobs:                        # job name: its file, relative to this file
  report: jobs/report.yaml
```

```yaml
# jobs/report.yaml: job `report`
schedule: {daily: "09:30"}   # only used by `rocky run`
tasks:
  today:
    cmd: [date, +%Y-%m-%d]   # program + arguments, no shell
  users:
    cmd: [curl, -sS, --fail-with-body, --json, '@-', '${BASE}/anything/users']
    stdin:                   # non-string stdin is written as JSON
      users: [{id: 1, name: alice}, {id: 2, name: bob}]
  notify_all:
    for_each: tasks.users.stdout.json.users   # a path; ${} optional
    as: USER
    do:
      notify:
        cmd: [echo, 'hi ${USER.name} on ${tasks.today.stdout}']
  ok:
    when: {this: tasks.notify.exit_code, eq: 0}
    cmd: [echo, done]
```

## Projects and jobs

| File | Fields |
|---|---|
| `rocky.yaml` (or `-f FILE`) | `jobs` (required: name → job file path), `values`, `env_file` (only here; relative to this file), `defaults` |
| a job file (at least one) | `tasks` (required, non-empty map), `schedule`, `values` (win over `rocky.yaml` values), `defaults` (win over `rocky.yaml` defaults) |

- Unknown keys anywhere are a load error.
- `defaults: {timeout, cwd, env, retry}` fills those fields on every cmd task (nested ones too) that doesn't set them; `env` merges by name. A default `cwd` is relative to the file the defaults are in. Use it instead of repeating `timeout`/`cwd: .`/`env` on each task.
- Only jobs listed under `jobs` exist; a job can't be written inline there, and every task is written inline in its job file. To run a job with other values or a schedule, copy its file and list the copy.
- Relative paths (`jobs` paths, `cwd`, `stdout_file`) resolve against the file they're written in, so in a job `cwd: ..` is the project directory; `env_file` against the project directory. Commands **without** `cwd` run in the current directory.

## Tasks

A task is exactly one kind. Any task can also have `when`.

| Kind | Keys |
|---|---|
| cmd | `cmd`, optionally `pipe` (see [Cmd tasks](#cmd-tasks)) |
| loop | `for_each`/`as`/`do` or `while`/`attempts`/`do`, optionally `on_failure` (see [Loops](#loops)) |
| trigger | `trigger: JOB`; no other fields but `when` (see [Triggers](#triggers)) |

- Tasks are a **map**, never a list. The key is the name; there is no `name` field. Keys are identifiers (`user_id`), `tasks` is reserved, and keys are unique within one map.
- There is no `if`/`then`/`else` or `switch`; use `when` on each task.
- A cmd task's result is saved as `tasks.<key>`: `{ok, exit_code, stdout, stderr}`. Later tasks read it with a dot path, e.g. `${tasks.login.stdout.user.id}`. `ok` is `false` after any failure (non-zero exit, timeout, exhausted `retry`).
- A non-zero exit stops the job unless `allow_failure: true`. A timeout under `allow_failure` saves `{ok: false, error: "..."}` instead. Test for any failure with `{this: tasks.x.ok, eq: false}`.
- Undefined variables, missing programs and a missing `cwd` stop the job even with `allow_failure`.

## Cmd tasks

| Field | Meaning |
|---|---|
| `cmd` | Required non-empty list: `[curl, -sS, URL]`. A string like `"ls -la"` is rejected. |
| `pipe` | Further commands, each a list, piped like `cmd \| a \| b` (pipefail semantics). |
| `stdin` | Standard input. A string as-is; any other value as compact JSON. Default: empty. |
| `env` | Map of extra environment variables for the command. |
| `cwd` | Working directory; relative to the YAML file. |
| `timeout` | A duration (`30s`, `5m`, `2h`); the command (and its children) are killed and the task fails. |
| `capture_stdout` | `false` discards stdout (result `stdout` is `null`). |
| `stdout_file` | Write stdout to this file (created or replaced) instead; result `stdout` is `null`. Not with `capture_stdout`. |
| `retry` | `{attempts: 5, delay: 2s, until: COND}`: run again until it exits 0 (or `until` holds), at most `attempts` times. Use it for polling instead of `while`. |
| `allow_failure` | `true` keeps the job going after a non-zero exit or timeout. |

`stdout` is parsed as JSON when the whole output is valid JSON, otherwise text with trailing newlines trimmed.

No shell is involved. To write a file, use `stdout_file`:

```yaml
save:
  cmd: [cat]
  stdin: ${tasks.fetch.stdout}
  stdout_file: "out/${DATE}.json"
```

For other redirects or globs call a shell, and pass values as positional arguments, never inside the script: `cmd: [sh, -c, 'ls "$1"/*.json', _, "${DIR}"]`.

For HTTP use `curl -sS --fail-with-body` so 4xx/5xx exit non-zero; `--json '@-'` sends `stdin` as a JSON body.

## Variables

- `${NAME}` or a dot path `${tasks.login.stdout.user.id}`, `${ITEMS.0.id}` (numeric segment = array index).
- Expanded in `cmd`/`pipe` arguments, `cwd`, strings in `stdin` and `env` values, and condition operator values (except `matches`). Each `${...}` stays one argument. Project and job `values` are not expanded.
- Lookup order: loop variables and `tasks.*` → job `values` → project `values` → `env_file` → process environment. Undefined fails the task.
- `env_file` lines are `KEY=value` strings, for secrets. They only fill `${...}`; a command sees one in its environment only if listed under `env`: `env: {TOKEN: '${API_TOKEN}'}`.
- A `stdin` field or condition operand that is exactly `${NAME}` keeps its JSON type (`null`, boolean, number, list or object); anywhere else values render as text.
- `$${` is a literal `${`. In `sh -c` scripts reading a variable from `env`, write `$NAME`.
- `values` names and `as` names must start with an uppercase letter or `_`, then contain only uppercase letters, digits and `_` (`[A-Z_][A-Z0-9_]*`).

## Writing YAML

- Quote `${...}` inside flow collections (`{...}` or `[...]`), quote numeric command arguments (`[sleep, "5"]`), and quote times, dates and strings containing YAML punctuation.
- Durations are a whole number and one unit, `s`, `m` or `h`: `90m`, not `1h30m`, `1.5m` or `30`.
- `null` is data in `values`, `stdin`, `env` values and `eq`/`ne`/`contains` operands. Omit optional configuration such as `schedule`, `when`, `cwd` or `retry` instead of writing `null`; explicit `null` there is rejected. `stdin: null` writes the JSON text `null`.
- Removed forms fail to load: integer-second durations, `weekly` and `time` as siblings (use `weekly: {days, time}`), task files (`name: path.yaml`, `file`, `with`; write the task inline), `if`/`switch` (use `when`), `when` on a job (put it on each task), `max` on `while` (use `attempts`).

## Conditions

`when` skips a task unless its condition holds. A skipped task saves no result, so `tasks.NAME` stays undefined (test with `exists`). The same conditions are used by `while` and `retry`'s `until`.

```yaml
big:                                 # runs only if it holds, else is skipped
  when: {this: tasks.stats.stdout.count, gt: 10}
  cmd: [echo, big]
small:                               # no else: write the opposite condition
  when: {this: tasks.stats.stdout.count, lte: 10}
  cmd: [echo, small]
```

`{this: PATH, OP: value}` with one or more of `eq`, `ne`, `gt`, `gte`, `lt`, `lte`, `contains`, `matches` (regex), `exists` (`true`/`false`, can't be combined). Combine with `all: [...]`, `any: [...]`, `not: cond`, or a bare list (= `all`). A numeric string equals the same number (`"200"` eq `200`), but two strings compare as text (`"1.1"` does not eq `"1.10"`).

`this` and `for_each` name one variable: `tasks.x.stdout` and `${tasks.x.stdout}` are the same (inside `{...}`, quote the `${}` form).

## Loops

```yaml
each:                                # for_each: value must be a JSON list
  for_each: tasks.users.stdout.items
  as: USER                           # default ITEM
  on_failure: next                   # a failing `do` task skips to the next element (default: stop)
  do:
    ping: {cmd: [echo, '${USER.id}']}

wait:                                # while: checked before each iteration
  while: {this: tasks.check.stdout, ne: 200}
  attempts: 10                       # fails the task if still true after 10 runs (like retry's)
  do:
    pause: {cmd: [sleep, '5']}
    check: {cmd: [curl, -sS, -o, /dev/null, -w, '%{http_code}', '${BASE}/health']}
```

Loops have no `cmd`, `timeout` or `allow_failure` (that's for cmd tasks); a loop uses `on_failure: next` to skip a failing iteration. Nothing runs in parallel. Results set inside loops stay available afterwards.

## Triggers

`trigger: JOB` queues a run of another job of the project (listed under `jobs`) (scheduled or not) and the job goes on without waiting. It saves no result and can't fail.

- In `rocky run`, the queued run starts right away (after any run of that job already going).
- In `rocky exec`, queued jobs run after the job, in order, even if a task exited non-zero (but not after an error such as an undefined variable or a timeout); only the job's own result is printed.
- A job that triggers itself without a `when` never stops.

## Schedules and `rocky run`

A job's `schedule` has exactly one of:

| Field | Example |
|---|---|
| `every` (+ optional `delay`) | `{every: 5m, delay: 10s}` |
| `hourly` | `{hourly: 15}` (minute 0-59) |
| `daily` | `{daily: "09:30"}`, quoted |
| `weekly` | `{weekly: {days: [mon, fri], time: "09:30"}}` |
| `at` | `{at: "2027-10-10T09:30:00"}`, local time, no `Z` |

`run_on_start: true` (not with `every`) also runs once at startup; on its own it runs the job once at startup. Times are local.

```sh
rocky run                                         # runs scheduled jobs until stopped
rocky run --socket /tmp/rocky.sock                # also accept requests; no schedule needed
rocky run -f nightly.yaml                         # another project file; logs YYYY-MM-DD-nightly.log
rocky trigger /tmp/rocky.sock report              # queue a run of `report`
```

`run` writes one JSON log line per event to stdout and to `.rocky/logs/YYYY-MM-DD.log` in the current directory. The project is re-read on each run; schedules only at startup. A job never overlaps itself. Without `--socket`, `run` exits once only finished `at` and `run_on_start`-only jobs are left.

## Errors and exit codes

- Success: exit `0`, stdout is the last task's result (or `null` if no cmd task ran).
- A task exits non-zero without `allow_failure`: its result on stdout, ``Error: task `name` exited with code N`` on stderr, exit `1`.
- Undefined variable, missing program, missing `cwd`: ``Error: task `name` failed`` with `Caused by:` lines, exit `1`, even with `allow_failure`.
- Invalid file: load error on stderr, nothing runs, exit `1`. Read the `Caused by:` lines; they name the job and task.
- Stopped by Ctrl-C (SIGINT), SIGTERM, SIGHUP or SIGQUIT: `128 + signal` (Ctrl-C is `130`); running commands are killed. SIGKILL can't be caught, so it can't trigger cleanup.

stdout only ever holds results, so `rocky exec job | jq .stdout` is safe.

## Full documentation

For details beyond this summary, read the page in rocky's `docs/` directory: `projects.md`, `yaml.md`, `tasks.md`, `cmd-tasks.md`, `variables.md`, `conditions.md`, `loops.md`, `triggers.md`, `cli.md`, `run.md` (log events), `schedules.md`, `service.md` (systemd/launchd), `debugging.md`.
