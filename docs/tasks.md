# Tasks

Each entry in a job's `tasks` (and in a loop's `do`) is one task, named by its key. Tasks run one at a time, in the order they are written.

## Task kinds

Each task is exactly one of the following. Any of them can also have [`when`](conditions.md), to run only if a condition holds.

| Kind | Keys | Page |
|---|---|---|
| Command | `cmd`, optionally `pipe` | [Cmd tasks](cmd-tasks.md) |
| For-each loop | `for_each`, `as`, `do`, `on_failure` | [Loops](loops.md) |
| While loop | `while`, `attempts`, `do`, `on_failure` | [Loops](loops.md#while) |
| Trigger | `trigger:` a job name | [Triggers](triggers.md) |
| Task file | a string: the path of a task file, or `{file, with}` | [Task files](files.md) |

## Task fields

| Field | Applies to | Notes |
|---|---|---|
| `cmd` | cmd tasks | Required, and must not be empty. |
| `pipe` | cmd tasks | Optional further commands connected to `cmd` by pipes. See [Pipes](cmd-tasks.md#pipes). |
| `timeout` | cmd tasks | How long before the command is killed and the task fails: a [duration](yaml.md#durations-and-times) such as `30s`, `5m` or `2h`. No default. |
| `cwd`, `stdin`, `env`, `capture_stdout`, `stdout_file` | cmd tasks | Working directory, standard input, environment variables, and whether stdout is kept or written to a file. See [Cmd tasks](cmd-tasks.md#options). |
| `retry` | cmd tasks | Run the command again until it succeeds or a condition holds. See [Retry](cmd-tasks.md#retry). |
| `allow_failure` | cmd tasks | `true` to continue the job when the task fails. See [Failures](#failures-and-allow_failure). |
| `on_failure` | loops | `next` to skip to the next iteration when a `do` task fails; default `stop`. See [Loops](loops.md#behaviour). |
| `when` | every task | Run the task only if this [condition](conditions.md) holds; otherwise skip it. |
| `for_each`, `as`, `do` | for-each loops | Runs `do` once per list item. A loop has no `cmd`, `timeout` or `allow_failure`. |
| `while`, `attempts`, `do` | while loops | Runs `do` while a condition holds; `attempts` optionally limits iterations. A loop has no `cmd`, `timeout` or `allow_failure`. |
| `trigger` | triggers | The name of a job of the project to queue a run of. A trigger has no other fields. See [Triggers](triggers.md). |

The task's name is its key. It saves a cmd task's result as `tasks.<name>`, and labels the task in errors and log lines (see [labels](cli.md#errors-and-exit-codes)). It must be a [variable name](variables.md#names) other than `tasks`, and two tasks in the same map can't share it. There is no `name` field.

Unknown keys are an error.

## Results

What goes into a cmd task is its `cmd`, `stdin` and `env`. What comes out is its result, `{ok, exit_code, stdout, stderr}`, saved as `tasks.<name>`. The fields are described in [Cmd tasks](cmd-tasks.md#result).

## Using earlier tasks' output

Each cmd task's result is saved as `tasks.<key>`, and later tasks read it with a dot path:

| Reference | Value |
|---|---|
| `${tasks.login.stdout.user.id}` | a field of a command's JSON output, such as a `curl` response body |
| `${tasks.today.stdout}` | a command's output (parsed JSON if it is valid JSON, otherwise text; `null` with `capture_stdout: false`) |
| `${tasks.check.exit_code}`, `${tasks.check.stderr}` | a command's exit code and error output |
| `${tasks.check.ok}` | `false` if the task failed in any way (non-zero exit, timeout, exhausted `retry`), otherwise `true` |

- **Where it works:** anywhere `${...}` works (see [where `${...}` is expanded](variables.md#where--is-expanded)). In a condition's `this` or `for_each`, the `${}` is optional: `this: tasks.login.exit_code` and `this: ${tasks.login.exit_code}` are the same.
- **Typed values:** a `stdin` field that is exactly `${tasks...}` keeps its JSON type, so `stdin: ${tasks.fetch.stdout}` writes the whole object as compact JSON, ready for `jq`. See [typed values](variables.md#typed-values).
- **Every task:** each cmd task is saved under its key. Keys only need to be unique within one map, so a task nested in a loop can share a name with another; the later result replaces the earlier one.
- **`tasks` is reserved:** a variable named `tasks` fails when the project loads (see [names](variables.md#names)).

## Failures and `allow_failure`

A cmd task **fails** when its exit code isn't `0`. `curl` exits `0` for any HTTP status unless told otherwise: pass `--fail-with-body` to make a 4xx or 5xx fail the task (exit code 22) while keeping the body.

- **Without `allow_failure`:** the job stops. `rocky exec` prints the failed task's result, then reports ``task `name` exited with code 3`` and exits `1`.
- **With `allow_failure: true`:** the job continues. The result is still saved as `tasks.<name>`, so a later task's `when` can test `${tasks.x.exit_code}`.
- **Inside a loop with `on_failure: next`:** the rest of that iteration is skipped and the loop goes on with its next element or condition check (see [Loops](loops.md#behaviour)).

`allow_failure: true` also lets the job continue after a **timeout** (a command that runs past its `timeout`) or a [`retry`](cmd-tasks.md#retry) whose `until` never held. The task has no exit code or output, so its result is just the error, e.g. `tasks.<name>` is ``{"ok": false, "error": "command `curl` timed out after 5s"}``. To react to any failure, a non-zero exit or a timeout alike, test `ok`: `when: {this: tasks.<name>.ok, eq: false}`.

Other errors stop the job no matter what `allow_failure` says: an undefined variable, a missing program, a missing `cwd`. Without `allow_failure`, so do timeouts. See [Command line](cli.md#errors-and-exit-codes).
