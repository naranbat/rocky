# Conditions

A task with `if` is a branch. If the condition holds, its `then` tasks run; otherwise its `else` tasks run. Either map may be left out, but not both. The job then continues with the task after the branch.

```yaml
tasks:
  A:
    cmd: [curl, -sS, --fail-with-body, '${BASE}/stats']
  route:
    if: { this: tasks.A.stdout.count, gt: 10 }
    then:
      B:
        cmd: [curl, -sS, --fail-with-body, -X, POST, '${BASE}/big']
    else:
      warn:                                   # else-if: nest a branch
        if:                                   # a list: every condition must hold
          - { this: tasks.A.stdout.message, contains: warning }
          - not: { this: tasks.A.stdout.message, matches: "^ignore" }
        then:
          C:
            cmd: [curl, -sS, --fail-with-body, -X, POST, '${BASE}/warn']
  D:                                          # always runs after the branch
    cmd: [curl, -sS, --fail-with-body, '${BASE}/done']
```

Example: the `conditional` job in [`examples/httpbin.yaml`](../examples/httpbin.yaml). Run it as is for the `then` branch, or set `COUNT` to `5` in [`examples/httpbin.env`](../examples/httpbin.env) for the `else` branch.

## Branch fields

| Field | Notes |
|---|---|
| `if` | The condition. |
| `then` | Tasks to run when it is true. |
| `else` | Tasks to run when it is false. |

Like every task, a branch is named by its key, which labels it in errors and `run`'s log lines; a branch has no result to save.

A branch can't also have `cmd`, `timeout` or `allow_failure`.

## Condition shape

A condition is either `this` with **one or more** operators, or exactly one of `all`, `any`, `not`, or a list.

- **Several operators** must all hold, checked in the table's order and stopping at the first false: `{this: X, gt: 1, lt: 10}` is a range. `exists` can't be combined with other operators.
- **A list** is shorthand for `all`: `if: [{this: A, eq: 1}, {this: B, eq: 2}]` is `if: {all: [...]}`. It works anywhere a condition does, including inside `all`, `any` and `not`.

`this` names the variable or dot path to test, with or without `${}`: `this: tasks.login.exit_code` or `this: ${tasks.login.exit_code}`. Inside `{...}`, quote a `${}` so YAML doesn't end the map at its `}`: `{this: '${tasks.login.exit_code}', eq: 0}`.

| Operator | True when |
|---|---|
| `gt` / `gte` / `lt` / `lte`: number | The variable is `>` / `>=` / `<` / `<=` the number. The number may also be written as a numeric string (`gt: "10"`) or a `${...}` that expands to one (`gt: "${LAST_ID}"`). The variable must be a number or a numeric string (`"15"` counts, surrounding spaces are ignored); otherwise the task fails with ``condition: `X` is not a number (<value>)``. |
| `eq` / `ne`: value | Equal / not equal. If both sides are strings, they compare as text (`"1.1"` ≠ `"1.10"`). Otherwise, if both sides are numbers or numeric strings, they compare as numbers (`200` = `"200"` = `200.0`; integers compare exactly, however large). Otherwise, if either side is a string, they compare as text. Otherwise JSON equality (bool, null, arrays, objects). |
| `contains`: value | String: contains the value as a substring. Array: some element `eq` the value. Object: has that key. Any other type fails the task. |
| `matches`: regex | The regex matches anywhere in the variable's text (use `^`/`$` to anchor). Non-strings are matched against their compact JSON. |
| `exists`: `true` / `false` | The variable is / isn't defined (a `null` value counts as defined). Never fails on an undefined variable, so `{this: tasks.ping.error, exists: true}` tells whether a task with `allow_failure` hit a timeout. To catch a non-zero exit too, use `{this: tasks.ping.ok, eq: false}`. |
| `all: [cond, …]` | Every condition is true. Stops at the first false. `all: []` is true. Same as the bare list `[cond, …]`. |
| `any: [cond, …]` | At least one condition is true. Stops at the first true. `any: []` is false. |
| `not: cond` | The condition is false. |

## Variable lookup

`this` is looked up in this order: loop variables and `tasks.NAME...`; then a [task file entry's](projects.md#reusing-a-task-file) `values`; then job `values`; then project `values`; then the project `env_file`; then the environment. An undefined variable fails the task, except with `exists`.

Values from env files and the environment are strings, which is why numeric strings are accepted by the number operators.

## Behaviour

- Operator values other than `matches` may use `${...}`, expanded when the condition runs, to compare two variables: `{this: ID, gt: "${LAST_ID}"}`. A value that is exactly `"${NAME}"` keeps its JSON type; `$${` is a literal `${`. If a number operator's value doesn't expand to a number, the task fails with ``condition: `gt` needs a number, got ...``. `matches` is fixed.
- Tasks inside `then`/`else` are named by their keys and can contain further branches and [loops](loops.md). Their results are available to later tasks.
- If the chosen map is empty (only `then` given and the condition is false), nothing runs and the job continues.

## Switch

A task with `switch` compares one variable against each key of `cases`, in order, and runs the tasks of the first case that matches. If none matches, its `default` tasks run. The job then continues with the task after the switch.

```yaml
tasks:
  check:
    cmd: [curl, -sS, -o, /dev/null, -w, '%{http_code}', '${BASE}/health']   # prints the status code
  route:
    switch: tasks.check.stdout                # a variable or dot path; ${} optional
    cases:                                    # value: tasks to run
      200:
        healthy:
          cmd: [echo, healthy]
      503:
        down:
          cmd: [echo, down]
    default:                                  # optional
      unexpected:
        cmd: [echo, unexpected]
```

| Field | Notes |
|---|---|
| `switch` | Required. The variable to compare, looked up like a condition's [`this`](#variable-lookup). An undefined variable fails the task. |
| `cases` | Required, non-empty. A map of value → tasks, tried in order. Each value is compared with the [`eq`](#condition-shape) rules, so `200` matches `"200"`; it's fixed (`${...}` isn't expanded). The exact same key can't appear twice; keys that are only equal by these rules (`200` and `"200"`) both load, and only the first can match. Each case needs at least one task. |
| `default` | Optional. The tasks to run when no case matches. Without it, nothing runs. |

A switch can't also have `cmd`, `timeout`, `allow_failure`, `if`, `then` or `else`. Tasks inside a case or `default` behave like those inside `then`/`else`.

## While

A task with `while` is a loop that repeats while a condition holds. The condition is checked before each run of its `do` tasks; once it's false, the job continues with the task after the loop. If it's false the first time, `do` never runs.

```yaml
tasks:
  check:
    cmd: [curl, -sS, -o, /dev/null, -w, '%{http_code}', '${BASE}/health']
  wait:
    while: { this: tasks.check.stdout, ne: 200 }   # any condition
    attempts: 10                                    # optional
    do:
      pause:
        cmd: [sleep, '5']
      check:                                        # updates tasks.check for the next check
        cmd: [curl, -sS, -o, /dev/null, -w, '%{http_code}', '${BASE}/health']
```

To run one command again until it succeeds or a condition holds, [`retry`](cmd-tasks.md#retry) is shorter: no separate first run, and no copy of the command inside `do`.

| Field | Notes |
|---|---|
| `while` | Required. A [condition](#condition-shape), evaluated like `if`'s. An undefined variable fails the task, except with `exists`. |
| `attempts` | Optional, at least 1. The most times `do` may run, like [`retry`](cmd-tasks.md#retry)'s `attempts`. If the condition still holds after that many runs, the task fails with ``while: still true after N attempts``. Without it, the loop has no limit. It used to be called `max`, which now fails to load with `` `max` is now `attempts` ``. |
| `do` | Required, non-empty. The tasks to run on each iteration. |
| `on_failure` | Optional, `stop` (the default) or `next`. With `next`, a failing `do` task skips the rest of that iteration; the condition is checked again next. A skipped iteration counts toward `attempts`. |

A `while` loop can't also have `cmd`, `timeout`, `allow_failure`, `if`, `then`, `else`, `switch`, `cases`, `default`, `for_each` or `as`. Like [`for_each`](loops.md#behaviour), results set inside `do` stay available after the loop, a failing task stops the job unless a loop around it has `on_failure: next`, and nothing pauses between iterations unless a `do` task does (`sleep`).

## Errors

Conditions are checked when the project loads, before any task runs. A non-boolean for `exists`, an invalid regex or an unknown key fails as ``invalid YAML in <file>``, with the cause. An operator without `this`, `exists` combined with another operator, `this` mixed with `all`/`any`/`not`, or a non-number for `gt`/`gte`/`lt`/`lte` (other than a `${...}`) fails as an invalid condition, e.g.:

```
Error: job `w`

Caused by:
    invalid task: `route`: invalid condition: `gt` needs a number, got "ten"
```
