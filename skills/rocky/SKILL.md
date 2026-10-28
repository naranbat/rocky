---
name: rocky
description: Write, validate and run rocky projects, YAML files of jobs whose tasks run commands in order, pass output between tasks, branch, loop, and run on schedules. Use when the user wants to automate repeatable work with rocky, edits a rocky project, job or task YAML file, or runs `rocky exec`, `rocky run`, `rocky trigger`, `rocky validate` or `rocky new`.
---

# rocky

rocky runs **projects**: a YAML file of named **jobs**. A job is a map of named **tasks** that run one at a time, in the order written. A task runs a command (no shell), and later tasks read earlier tasks' output. `rocky exec` runs one job once; `rocky run` runs jobs on their `schedule`.

## Workflow

```sh
rocky new project project.yaml               # starter project with job `example`
rocky new project full.yaml --level full     # every field, with comments: the full reference
rocky new job jobs/deploy.yaml               # a job in its own file
rocky new task tasks/login.yaml              # a task in its own file
rocky validate project.yaml                  # load-time checks only, runs nothing
rocky exec project.yaml JOB                  # run JOB once, print last task's result as JSON
rocky exec project.yaml JOB --verbose        # also print each command to stderr before it runs
```

1. Write or edit the YAML.
2. Run `rocky validate` and fix every error before going further.
3. Run `rocky exec ... --verbose` to try the job.

There is **no dry run**: `exec` runs the real commands. Before executing a job with side effects (deploys, POST/DELETE requests, writing or deleting files, messages), confirm with the user. `--verbose` prints filled-in arguments, including secrets from the env file.

`rocky new` never overwrites; it fails with `<PATH> already exists`.

## Example

```yaml
values:                          # fixed values, read as ${NAME}; names are UPPERCASE
  BASE: https://httpbin.org
jobs:
  report:
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
      check:
        if: {this: tasks.notify.exit_code, eq: 0}
        then:
          ok:
            cmd: [echo, done]
```

## Projects, jobs, tasks

| Level | Fields |
|---|---|
| project | `jobs` (required), `values`, `env_file` (only here; relative to the project file), `defaults` |
| job | `tasks` (required, non-empty map), `schedule`, `values` (win over project values), `defaults` (win over project defaults) |
| task | exactly one kind: cmd (`cmd`), branch (`if`), switch (`switch`), loop (`for_each` or `while`), or a string path to a task file |

- Tasks are a **map**, never a list. The key is the name; there is no `name` field. Keys are identifiers (`user_id`), `tasks` is reserved, and keys are unique within one map.
- Unknown keys anywhere are a load error.
- `defaults: {timeout, cwd, env, retry}` fills those fields on every cmd task (nested ones and task files too) that doesn't set them; `env` merges by name. A default `cwd` is relative to the file the defaults are in. Use it instead of repeating `timeout`/`cwd: .`/`env` on each task.
- Job file: `jobs: {report: jobs/report.yaml}`; the file holds `tasks`, `schedule`, `values`, `defaults`. Task file: `tasks: {login: tasks/login.yaml}`; the file holds one task, no `name`.
- Reuse one job file as several jobs: `jobs: {news: {file: jobs/fetch.yaml, schedule: {every: 1h}, values: {TARGET: https://x}}}`. Only `file`, `schedule`, `values` are allowed there.
- Relative paths (`cwd`, task and job files) resolve against the file they're written in; `env_file` and job file paths against the project file. Commands **without** `cwd` run in the current directory.

## Cmd tasks

| Field | Meaning |
|---|---|
| `cmd` | Required non-empty list: `[curl, -sS, URL]`. A string like `"ls -la"` is rejected. |
| `pipe` | Further commands, each a list, piped like `cmd \| a \| b` (pipefail semantics). |
| `stdin` | Standard input. A string as-is; any other value as compact JSON. Default: empty. |
| `env` | Map of extra environment variables for the command. |
| `cwd` | Working directory; relative to the YAML file. |
| `timeout` | Seconds (`30`) or a duration (`30s`, `5m`, `2h`); the command (and its children) are killed and the task fails. |
| `capture_stdout` | `false` discards stdout (result `stdout` is `null`). |
| `stdout_file` | Write stdout to this file (created or replaced) instead; result `stdout` is `null`. Not with `capture_stdout`. |
| `retry` | `{attempts: 5, delay: 2s, until: COND}`: run again until it exits 0 (or `until` holds), at most `attempts` times. Use it for polling instead of `while`. |
| `allow_failure` | `true` keeps the job going after a non-zero exit or timeout. |

Result, saved as `tasks.<key>`: `{ok, exit_code, stdout, stderr}`. `stdout` is parsed as JSON when the whole output is valid JSON, otherwise text with trailing newlines trimmed. A timeout under `allow_failure` saves `{ok: false, error: "..."}` instead. Test for any failure with `{this: tasks.x.ok, eq: false}`.

No shell is involved. To write a file, use `stdout_file` (relative to the YAML file; result `stdout` is `null`):

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
- Expanded in `cmd`/`pipe` arguments, `cwd`, strings in `stdin` and `env` values, and condition operator values (except `matches`). Each `${...}` stays one argument.
- Lookup order: loop variables and `tasks.*` → job `values` → project `values` → `env_file` → process environment. Undefined fails the task.
- `env_file` lines are `KEY=value` strings, for secrets. They only fill `${...}`; a command sees one in its environment only if listed under `env`: `env: {TOKEN: ${API_TOKEN}}`.
- A `stdin` field that is exactly `${NAME}` keeps its JSON type (number, list, object); anywhere else values render as text.
- `$${` is a literal `${`. In `sh -c` scripts reading a variable from `env`, write `$NAME`.
- `values` names and `as` names must be uppercase (`A-Z`, `0-9`, `_`).

## Control flow

`if`/`while` (`this:`), `switch` and `for_each` name one variable: `tasks.x.stdout` and `${tasks.x.stdout}` are the same (inside `{...}`, quote the `${}` form).

```yaml
route:                               # branch
  if: {this: tasks.stats.stdout.count, gt: 10}
  then: {big: {cmd: [echo, big]}}
  else: {small: {cmd: [echo, small]}}

status:                              # switch: first matching case, compared like `eq`
  switch: tasks.check.stdout
  cases:
    200: {up: {cmd: [echo, up]}}
    503: {down: {cmd: [echo, down]}}
  default: {other: {cmd: [echo, other]}}

wait:                                # while: checked before each iteration
  while: {this: tasks.check.stdout, ne: 200}
  attempts: 10                       # fails the task if still true after 10 runs (like retry's)
  do:
    pause: {cmd: [sleep, '5']}
    check: {cmd: [curl, -sS, -o, /dev/null, -w, '%{http_code}', '${BASE}/health']}

each:                                # for_each: value must be a JSON list
  for_each: tasks.users.stdout.items
  as: USER                           # default ITEM
  on_failure: next                   # a failing `do` task skips to the next element (default: stop)
  do:
    ping: {cmd: [echo, '${USER.id}']}
```

Conditions: `{this: PATH, OP: value}` with one or more of `eq`, `ne`, `gt`, `gte`, `lt`, `lte`, `contains`, `matches` (regex), `exists` (`true`/`false`, can't be combined). Combine with `all: [...]`, `any: [...]`, `not: cond`, or a bare list (= `all`). Numeric strings compare as numbers (`"200"` eq `200`).

Branch, switch and loops have no `cmd`, `timeout` or `allow_failure` (that's for cmd tasks); a loop uses `on_failure: next` to skip a failing iteration. Nothing runs in parallel. Results set inside branches and loops stay available afterwards.

## Schedules and `rocky run`

A job's `schedule` has exactly one of:

| Field | Example |
|---|---|
| `every` (+ optional `delay`) | `{every: 5m, delay: 10s}`; units `s`, `m`, `h` only (`90m`, not `1h30m`) |
| `hourly` | `{hourly: 15}` (minute 0-59) |
| `daily` | `{daily: "09:30"}`, quoted |
| `weekly` + `time` | `{weekly: [mon, fri], time: "09:30"}` |
| `at` | `{at: "2027-10-10T09:30:00"}`, local time, no `Z` |

`run_on_start: true` (not with `every`) also runs once at startup. Times are local.

```sh
rocky run project.yaml                            # runs scheduled jobs until stopped
rocky run project.yaml --socket /tmp/rocky.sock   # also accept requests; no schedule needed
rocky trigger /tmp/rocky.sock report              # queue a run of `report`
```

`run` writes one JSON log line per event to stdout and to `logs/YYYY-MM-DD.log` in the current directory. The project is re-read on each run; schedules only at startup. To keep it running in the background, see the systemd/launchd notes in rocky's `docs/service.md`.

## Errors and exit codes

- Success: exit `0`, stdout is the last task's result (or `null` if no cmd task ran).
- A task exits non-zero without `allow_failure`: its result on stdout, ``Error: task `name` exited with code N`` on stderr, exit `1`.
- Undefined variable, missing program, missing `cwd`: ``Error: task `name` failed`` with `Caused by:` lines, exit `1`, even with `allow_failure`.
- Invalid file: load error on stderr, nothing runs, exit `1`. Read the `Caused by:` lines; they name the job and task.
- Stopped by a signal: `128 + signal` (Ctrl-C is `130`); running commands are killed.

stdout only ever holds results, so `rocky exec p.yaml job | jq .stdout` is safe.
