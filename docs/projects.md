# Projects

A project is a YAML file of named **jobs**. A job is a map of named **[tasks](tasks.md)** that `rocky exec` executes in order, or that `rocky run` runs on its [schedule](schedules.md). Jobs and tasks are both named by their key, and can be written inline or kept in their own YAML files (see [Job and task files](files.md)). A task runs a **command** (an HTTP request is a `curl` command), and every task can use the result of any earlier task. `exec` prints the result of the last task that ran as JSON.

```yaml
jobs:
  report:
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
        stdout_file: report.json   # relative to the project file
        stdin:
          date: ${tasks.today.stdout}
          loud: ${tasks.shout.stdout}

      send:
        cmd: [curl, -sS, --fail-with-body, -H, 'Authorization: Bearer ${tasks.login.stdout.json.token}', --json, '@-', https://httpbin.org/anything/report]
        stdin:
          date: ${tasks.today.stdout}
```

```sh
rocky exec project.yaml report | jq .stdout.json   # {"date": "2026-10-09"}
```

Examples: [`httpbin.yaml`](../examples/httpbin.yaml) (jobs `login`, `mixed`, `conditional`, `health`, `loop`, `poll`, and `files` from [`jobs/files.yaml`](../examples/jobs/files.yaml)), [`imports.yaml`](../examples/imports.yaml) (jobs `health`, `report` and `files`, each from its own file), [`env-file.yaml`](../examples/env-file.yaml) (job `user`), [`cmd-env.yaml`](../examples/cmd-env.yaml) (job `greet`).

## Project fields

| Field | Notes |
|---|---|
| `values` | Map of uppercase name → fixed value (any YAML value: string, number, boolean, list or object), read as `${NAME}` by every job. See [Variables](variables.md#values). |
| `env_file` | A `.env` file shared by all jobs, relative to the project file. See [Variables](variables.md#env-file). |
| `defaults` | Fields every cmd task in every job gets unless it sets them. See [Defaults](#defaults). |
| `jobs` | Required, at least one. Map of job name → an inline job, a job-file path, or a job file with its own `schedule` and `values` (see [Reusing a job file](files.md#reusing-a-job-file)). |

## Job fields

| Field | Notes |
|---|---|
| `schedule` | Optional. When `rocky run` runs this job. See [Schedules](schedules.md). `exec` checks it but doesn't run it. |
| `values` | Map of uppercase name → fixed value, read as `${NAME}` by this job's tasks; wins over a project value of the same name. See [Variables](variables.md#values). |
| `defaults` | Fields this job's cmd tasks get unless they set them; win over the project's. See [Defaults](#defaults). |
| `tasks` | Required, and must not be empty. Map of task name → an inline task, a task-file path, or a task file with its own `with` inputs. Tasks run in the order they are written. See [Tasks](tasks.md). |

## Defaults

`defaults` sets `timeout`, `cwd`, `env` and `retry` for every cmd task that doesn't set them itself, so they aren't repeated on each task:

```yaml
defaults:                    # the project's: every job, job files included
  timeout: 30s
  cwd: .                     # relative to this file
jobs:
  api:
    defaults:                # the job's: win over the project's, field by field
      env:
        TOKEN: ${API_TOKEN}
      retry: {attempts: 3, delay: 2s}
    tasks:
      users:
        cmd: [curl, -sS, --fail-with-body, -H, 'Authorization: Bearer ${TOKEN}', '${BASE}/users']
        timeout: 5m          # its own value wins
      cached:
        cmd: [cat, users.json]   # runs in the project file's directory
```

| Field | Notes |
|---|---|
| `timeout` | Like a task's [`timeout`](cmd-tasks.md#options). |
| `cwd` | Like a task's `cwd`, but relative to the file the `defaults` are written in (the project or a job file), not the task's file. `${...}` is substituted when the task runs. |
| `env` | Merged by name: a task gets every default name it doesn't set itself. |
| `retry` | Like a task's [`retry`](cmd-tasks.md#retry). |

- A task's own value wins. A job's `defaults` win over the project's, field by field, and their `env` is merged by name.
- Defaults reach every cmd task of the job: inside loops, and in task files. Loops themselves don't take them.
- Other fields, such as `allow_failure` or `stdout_file`, aren't allowed: unknown keys are an error.
- A [job file entry with `file`](files.md#reusing-a-job-file) can't have `defaults`; put them in the job file.

## Execution

- Tasks run one at a time, in order. There is no parallelism.
- Loops run their nested tasks in place, then continue with the next task. A task whose `when` doesn't hold is skipped.
- The printed result is the last cmd task that ran, including one inside a loop. If no cmd task ran at all (every task skipped), the printed result is `null`.

## Validation

The whole project, every job including nested loops, is checked when it loads, before any task runs. `rocky validate <PROJECT>` runs these checks alone, without running anything. These fail immediately:

- no `jobs`, an empty `tasks`, an empty `cmd` or `do`
- `tasks` or `do` written as a list instead of a map, or two tasks with the same key in one map
- a task without `cmd` that isn't a loop, trigger or task file
- a trigger with any other field, or whose job isn't in the project's `jobs`
- a job entry with both `file` and `tasks`, or with neither
- `pipe`, `cwd`, `stdin`, `env`, `capture_stdout`, `stdout_file` or `retry` without `cmd`; both `stdout_file` and `capture_stdout`; `timeout` on a loop; `allow_failure` on a loop (use `on_failure: next`), or `on_failure` on anything but a loop; `do` without `for_each` or `while`; `as` without `for_each`; `attempts` without `while`
- a missing task or job file, a task file with a `name` field, or a task file that uses itself, directly or through other files
- a task entry with `file` and any field but `with`, or `with` without `file`
- a name that appears twice in `jobs` or a cmd task's `env`
- a [variable name](variables.md#names) in `as` that isn't uppercase (`A-Z`, `0-9`, `_`), or a task key that isn't an identifier or is `tasks`
- an invalid [condition](conditions.md) or [schedule](schedules.md)
- unknown keys anywhere, including `schedules` in the project and `vars` anywhere

Errors inside a job are reported under its name:

```
Error: job `report`

Caused by:
    invalid task: `today` has an empty `cmd`
```

Some problems only show up when the job runs: a missing `env_file` (loaded when the job starts), an undefined variable (when its task runs).

## Paths

Relative paths are resolved against the directory of **the file they are written in**: the project, a job file or a task file. That covers task file paths, `cwd` and `stdout_file`; a `cwd` from [`defaults`](#defaults) is relative to the file the defaults are in. The project `env_file` and job file paths are relative to the project file. None are relative to your current directory. Commands in `cmd:` tasks without `cwd` run in your current directory.
