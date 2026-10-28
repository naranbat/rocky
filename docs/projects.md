# Projects

A project is a YAML file of named **jobs**. A job is a map of named **tasks** that `rocky exec` executes in order, or that `rocky run` runs on its [schedule](schedules.md). Jobs and tasks are both named by their key, and can be written inline or kept in their own YAML files (see [Job and task files](#job-and-task-files)). A task runs a **command** (an HTTP request is a `curl` command), and every task can use the result of any earlier task. `exec` prints the result of the last task that ran as JSON.

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

      save:                        # write a file: the path is $1
        cmd: [sh, -c, 'cat > "$1"', _, report.json]
        cwd: .                     # relative to the project file
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
| `values` | Map of uppercase name → fixed value (any YAML value: string, number, boolean, list or object), read as `${NAME}` by every job. See [Env](env.md#values). |
| `env_file` | A `.env` file shared by all jobs, relative to the project file. See [Env](env.md#env-file). |
| `jobs` | Required, at least one. Map of job name → job, → the path of a job file, or → a job file with its own `schedule` and `values` (see [Reusing a job file](#reusing-a-job-file)). |

## Job fields

| Field | Notes |
|---|---|
| `schedule` | Optional. When `rocky run` runs this job. See [Schedules](schedules.md). `exec` checks it but doesn't run it. |
| `values` | Map of uppercase name → fixed value, read as `${NAME}` by this job's tasks; wins over a project value of the same name. See [Env](env.md#values). |
| `tasks` | Required, and must not be empty. Map of task name → task, or → the path of a task file. Tasks run in the order they are written. |

## Task kinds

Each entry in `tasks` (and in `then`, `else`, `do`, each switch case and `default`) is exactly one of the following.

| Kind | Keys | Page |
|---|---|---|
| Command | `cmd:` | [Cmd tasks](cmd-tasks.md) |
| Branch | `if`, `then`, `else` | [Conditions](conditions.md) |
| Switch | `switch`, `cases`, `default` | [Conditions](conditions.md#switch) |
| Loop | `for_each`, `as`, `do` | [Loops](loops.md) |
| Task file | a string: the path of a task file | [Job and task files](#job-and-task-files) |

## Task fields

| Field | Applies to | Notes |
|---|---|---|
| `cmd` | cmd tasks | Required, and must not be empty. |
| `timeout` | cmd tasks | How long before the command is killed and the task fails: seconds (`300`) or a duration (`30s`, `5m`, `2h`). No default. |
| `cwd`, `stdin`, `env`, `capture_stdout` | cmd tasks | Working directory, standard input, environment variables, and whether stdout is kept. See [Cmd tasks](cmd-tasks.md#options). |
| `allow_failure` | cmd tasks, loops | `true` to continue when the task fails; on a loop, to skip to the next iteration when a `do` task fails. |
| `if`, `then`, `else` | branches | A branch has no `cmd`, `timeout` or `allow_failure`. |
| `switch`, `cases`, `default` | switches | Like a branch: no `cmd`, `timeout` or `allow_failure`, and no `if`, `then` or `else`. |
| `for_each`, `as`, `do` | loops | A loop has no `cmd` or `timeout`, and no `if`, `then`, `else`, `switch`, `cases` or `default`. |

The task's name is its key. It saves a cmd task's result as `tasks.<name>`, and labels the task in errors and log lines (see [labels](cli.md#errors-and-exit-codes)). It must be a [variable name](env.md#names) other than `tasks`, and two tasks in the same map can't share it. There is no `name` field.

Unknown keys are an error.

## Results

What goes into a task is its `cmd`, `stdin` and `env`. What comes out is its result, `{exit_code, stdout, stderr}`, saved as `tasks.<name>`.

`${...}` is substituted in `stdin`, and a field that is exactly `${NAME}` keeps its JSON type (see [typed values](env.md#typed-values)). For stdin, a string is used as-is and any other value is written as compact JSON, so `stdin: ${tasks.fetch.stdout}` is ready for `jq`.

## Using earlier tasks' output

Each cmd task's result is saved as `tasks.<key>`, and later tasks read it with a dot path:

| Reference | Value |
|---|---|
| `${tasks.login.stdout.user.id}` | a field of a command's JSON output, such as a `curl` response body |
| `${tasks.today.stdout}` | a command's output (parsed JSON if it is valid JSON, otherwise text; `null` with `capture_stdout: false`) |
| `${tasks.check.exit_code}`, `${tasks.check.stderr}` | a command's exit code and error output |

- **Where it works:** anywhere `${...}` works. In an `if` condition, `switch` or `for_each`, the `${}` is optional: `this: tasks.login.exit_code` and `this: ${tasks.login.exit_code}` are the same.
- **Typed values:** a `stdin` field that is exactly `${tasks...}` keeps its JSON type, so `stdin: ${tasks.fetch.stdout}` writes the whole object.
- **Every task:** each cmd task is saved under its key. Keys only need to be unique within one map, so a task nested in a branch or loop can share a name with another; the later result replaces the earlier one.
- **`tasks` is reserved:** a variable named `tasks` fails when the project loads (see [names](env.md#names)).

## Failures and `allow_failure`

A cmd task **fails** when its exit code isn't `0`. `curl` exits `0` for any HTTP status unless told otherwise: pass `--fail-with-body` to make a 4xx or 5xx fail the task (exit code 22) while keeping the body.

- **Without `allow_failure`:** the job stops. `rocky exec` prints the failed task's result, then reports ``task `name` exited with code 3`` and exits `1`.
- **With `allow_failure: true`:** the job continues. The result is still saved as `tasks.<name>`, so you can branch on `${tasks.x.exit_code}`.
- **Inside a loop with `allow_failure: true`:** the rest of that iteration is skipped and the loop goes on with its next element or condition check (see [Loops](loops.md#behaviour)).

`allow_failure: true` also lets the job continue after a **timeout** (a command that runs past its `timeout`). The task has no exit code, so its result is just the error, e.g. `tasks.<name>` is ``{"error": "command `curl` timed out after 5s"}``. Branch on it with `if: {this: tasks.<name>.error, exists: true}` (see [Conditions](conditions.md#condition-shape)).

Other errors stop the job no matter what `allow_failure` says: an undefined variable, a missing program. Without `allow_failure`, so do timeouts. See [Command line](cli.md#errors-and-exit-codes).

## Execution

- Tasks run one at a time, in order. There is no parallelism.
- Branches, switches and loops run their nested tasks in place, then continue with the next task.
- The printed result is the last cmd task that ran, including one inside a branch or loop. If no cmd task ran at all (every branch skipped), the printed result is `null`.

## Validation

The whole project, every job including nested branches and loops, is checked when it loads, before any task runs. `rocky validate <PROJECT>` runs these checks alone, without running anything. These fail immediately:

- no `jobs`, an empty `tasks`, an empty `cmd`, `do`, `cases`, or `then` *and* `else`
- `tasks`, `then`, `else`, `do` or `default` written as a list instead of a map, or two tasks with the same key in one map
- a task without `cmd` that isn't a branch, switch, loop or task file
- a job entry with both `file` and `tasks`, or with neither
- `cwd`, `stdin`, `env` or `capture_stdout` without `cmd`; `timeout` on a branch, switch or loop; `then`/`else` without `if`; `cases`/`default` without `switch`; `do`/`as` without `for_each`
- a missing task or job file, a task file with a `name` field, or a task file that uses itself, directly or through other files
- a name that appears twice in `jobs`, a cmd task's `env`, or a switch's `cases`
- a [variable name](env.md#names) in `as` that isn't uppercase (`A-Z`, `0-9`, `_`), or a task key that isn't an identifier or is `tasks`
- an invalid [condition](conditions.md) or [schedule](schedules.md)
- unknown keys anywhere, including `schedules` in the project and `vars` anywhere

Errors inside a job are reported under its name:

```
Error: job `report`

Caused by:
    invalid task: `today` has an empty `cmd`
```

Some problems only show up when the job runs: a missing `env_file` (loaded when the job starts), an undefined variable (when its task runs).

## Job and task files

A job or a task can live in its own YAML file. All files are loaded and checked when the project loads, and again each time `run` runs a job.

```yaml
# project.yaml
jobs:
  report: jobs/report.yaml       # a job file
  quick:
    tasks:
      login: tasks/login.yaml    # a task file, named by its key

# jobs/report.yaml: the same fields as an inline job
tasks:
  signin: ../tasks/login.yaml    # relative to this file
  today:
    cmd: [date]

# tasks/login.yaml: one task of any kind, including a branch or loop; no `name`
cmd: [curl, -sS, --fail-with-body, https://httpbin.org/anything/login]
```

- A task file holds one task, without a `name`: the key that points at it names it, so one file can be used under several names. It can use other task files, inside a branch or loop too, but not itself, directly or through other files.
- A task file entry is just a path, so everything else goes in the file. To change a field for one use, write the task inline.
- A job file has the job fields `schedule`, `values` and `tasks`. The project keeps `env_file`. To run one job file with different values or schedules, see below.

Example: [`imports.yaml`](../examples/imports.yaml), whose jobs all come from [`jobs/`](../examples/jobs/) and share its env file.

### Reusing a job file

A job entry can name its file under `file`, with its own `schedule` and `values`, so one file can run as several jobs:

```yaml
jobs:
  titles:
    file: jobs/fetch.yaml
    values:
      TARGET: https://httpbin.org/json
  news:
    file: jobs/fetch.yaml
    schedule: {every: 1h}
    values:
      TARGET: https://example.com/news

# jobs/fetch.yaml: an ordinary job file
tasks:
  show:
    cmd: [curl, -sS, --fail-with-body, '${TARGET}']
```

- Only `file`, `schedule` and `values` are allowed; the tasks come from the file.
- The entry's `values` win over the file's, which win over the project's.
- The entry's `schedule` replaces the file's; without one, the file's is kept.
- A value the file uses but nobody sets is an undefined variable, which fails when its task runs.

Example: `echo_health` in [`imports.yaml`](../examples/imports.yaml).

## Paths

Relative paths are resolved against the directory of **the file they are written in**: the project, a job file or a task file. That covers task file paths and `cwd`. The project `env_file` and job file paths are relative to the project file. None are relative to your current directory. Commands in `cmd:` tasks without `cwd` run in your current directory.
