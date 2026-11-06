# Projects

A project is a **directory**: every file in its `jobs/` folder is one **job**, named by its file (`jobs/report.yaml` is job `report`), and an optional `rocky.yaml` holds settings shared by every job. A job is a map of named **[tasks](tasks.md)** that `rocky exec` executes in order, or that `rocky run` runs on its [schedule](schedules.md). A task runs a **command** (an HTTP request is a `curl` command), and every task can use the result of any earlier task. `exec` prints the result of the last task that ran as JSON.

```
my-project/
  rocky.yaml          # optional: values, env_file, defaults shared by every job
  jobs/
    report.yaml       # job `report`
    health.yaml       # job `health`
```

`rocky` always works on the project in the current directory, so run it from there.

```yaml
# jobs/report.yaml
tasks:
  login:                       # the task's name
    cmd: [curl, -sS, --fail-with-body, --json, '@-', https://httpbin.org/anything/login]
    stdin:                     # sent as the JSON body
      token: demo-token
    # curl's JSON output is parsed; httpbin echoes the request body back
    # under `json`, so the token is at stdout.json.token

  today:
    cmd: [date, +%Y-%m-%d]     # a command: program + arguments, no shell involved
    env:                       # added to the command's environment
      TZ: UTC

  shout:
    cmd: [tr, a-z, A-Z]
    stdin: ${USER}             # the command's standard input; USER from the environment

  save:                        # write a file
    cmd: [cat]
    stdout_file: ../report.json   # relative to this file: the project directory
    stdin:
      date: ${tasks.today.stdout}
      loud: ${tasks.shout.stdout}

  send:
    cmd: [curl, -sS, --fail-with-body, -H, 'Authorization: Bearer ${tasks.login.stdout.json.token}', --json, '@-', https://httpbin.org/anything/report]
    stdin:
      date: ${tasks.today.stdout}
```

```sh
rocky exec report | jq .stdout.json   # {"date": "2026-10-09"}
```

Examples, each a project directory to run from: [`httpbin/`](../examples/httpbin/) (jobs `login`, `mixed`, `conditional`, `health`, `loop`, `poll`, `report` and `status`, sharing an env file), [`env/`](../examples/env/) (jobs `user` and `greet`), [`pipe/`](../examples/pipe/) (job `words`), [`trigger/`](../examples/trigger/) (jobs `check`, `alert` and `report`), [`values/`](../examples/values/) (job `greet`).

## Job files

- Every `*.yaml` file directly in `jobs/` is a job, named by its file name without `.yaml`. Other files and sub-directories in `jobs/` are ignored.
- A project needs at least one job file.
- To run one job with different values or schedules, copy its file.

## `rocky.yaml` fields

The file is optional; without it, every job has only its own settings.

| Field | Notes |
|---|---|
| `values` | Map of uppercase name → fixed value (any YAML value: string, number, boolean, list or object), read as `${NAME}` by every job. See [Variables](variables.md#values). |
| `env_file` | A `.env` file shared by all jobs, relative to the project directory. See [Variables](variables.md#env-file). |
| `defaults` | Fields every cmd task in every job gets unless it sets them. See [Defaults](#defaults). |

## Job fields

| Field | Notes |
|---|---|
| `schedule` | Optional. When `rocky run` runs this job. See [Schedules](schedules.md). `exec` checks it but doesn't run it. |
| `values` | Map of uppercase name → fixed value, read as `${NAME}` by this job's tasks; wins over a `rocky.yaml` value of the same name. See [Variables](variables.md#values). |
| `defaults` | Fields this job's cmd tasks get unless they set them; win over `rocky.yaml`'s. See [Defaults](#defaults). |
| `tasks` | Required, and must not be empty. Map of task name → task. Tasks run in the order they are written. See [Tasks](tasks.md). |

## Defaults

`defaults` sets `timeout`, `cwd`, `env` and `retry` for every cmd task that doesn't set them itself, so they aren't repeated on each task:

```yaml
# rocky.yaml
defaults:                    # the project's: every job
  timeout: 30s
  cwd: .                     # relative to this file: the project directory
```

```yaml
# jobs/api.yaml
defaults:                    # the job's: win over the project's, field by field
  env:
    TOKEN: ${API_TOKEN}
  retry: {attempts: 3, delay: 2s}
tasks:
  users:
    cmd: [curl, -sS, --fail-with-body, -H, 'Authorization: Bearer ${TOKEN}', '${BASE}/users']
    timeout: 5m              # its own value wins
  cached:
    cmd: [cat, users.json]   # runs in the project directory
```

| Field | Notes |
|---|---|
| `timeout` | Like a task's [`timeout`](cmd-tasks.md#options). |
| `cwd` | Like a task's `cwd`, but relative to the file the `defaults` are written in (`rocky.yaml` or a job file), not the task's file. `${...}` is substituted when the task runs. |
| `env` | Merged by name: a task gets every default name it doesn't set itself. |
| `retry` | Like a task's [`retry`](cmd-tasks.md#retry). |

- A task's own value wins. A job's `defaults` win over the project's, field by field, and their `env` is merged by name.
- Defaults reach every cmd task of the job, inside loops too. Loops themselves don't take them.
- Other fields, such as `allow_failure` or `stdout_file`, aren't allowed: unknown keys are an error.

## Execution

- Tasks run one at a time, in order. There is no parallelism.
- Loops run their nested tasks in place, then continue with the next task. A task whose `when` doesn't hold is skipped.
- The printed result is the last cmd task that ran, including one inside a loop. If no cmd task ran at all (every task skipped), the printed result is `null`.

## Validation

The whole project, `rocky.yaml` and every job file including nested loops, is checked when it loads, before any task runs. `rocky validate` runs these checks alone, without running anything. These fail immediately:

- no job file in `jobs/` (or no `jobs/`), an empty `tasks`, an empty `cmd` or `do`
- `tasks` or `do` written as a list instead of a map, or two tasks with the same key in one map
- a task without `cmd` that isn't a loop or trigger
- a trigger with any other field, or whose job isn't a job of the project
- `pipe`, `cwd`, `stdin`, `env`, `capture_stdout`, `stdout_file` or `retry` without `cmd`; both `stdout_file` and `capture_stdout`; `timeout` on a loop; `allow_failure` on a loop (use `on_failure: next`), or `on_failure` on anything but a loop; `do` without `for_each` or `while`; `as` without `for_each`; `attempts` without `while`
- a name that appears twice in a cmd task's `env` or in `values`
- a [variable name](variables.md#names) in `as` that isn't uppercase (`A-Z`, `0-9`, `_`), or a task key that isn't an identifier or is `tasks`
- an invalid [condition](conditions.md) or [schedule](schedules.md)
- unknown keys anywhere, including `jobs` or `schedules` in `rocky.yaml` and `vars` anywhere

Errors inside a job are reported under its name:

```
Error: job `report`

Caused by:
    invalid task: `today` has an empty `cmd`
```

Some problems only show up when the job runs: a missing `env_file` (loaded when the job starts), an undefined variable (when its task runs).

## Paths

Relative paths are resolved against the directory of **the file they are written in**: `rocky.yaml` (the project directory) or a job file (`jobs/`). That covers `cwd` and `stdout_file`, so a job's `cwd: ..` is the project directory; a `cwd` from [`defaults`](#defaults) is relative to the file the defaults are in, and `env_file` to the project directory. Commands in `cmd:` tasks without `cwd` run in your current directory, which is the project directory when you run `rocky` from there.
