# Conditions

Any job or task can have `when`, a condition. If the condition holds, it runs; if not, it is skipped and the job goes on with the next task. Without `when`, it always runs. There are no `if`/`then`/`else` or `switch` blocks: each task carries its own condition, so to run several tasks under the same condition, give each one that `when`.

The same conditions are used by a [`while` loop](loops.md#while) and by [`retry`](cmd-tasks.md#retry)'s `until`.

```yaml
tasks:
  A:
    cmd: [curl, -sS, --fail-with-body, '${BASE}/stats']
  B:
    when: { this: tasks.A.stdout.count, gt: 10 }
    cmd: [curl, -sS, --fail-with-body, -X, POST, '${BASE}/big']
  C:
    when:                                     # a list: every condition must hold
      - { this: tasks.A.stdout.count, lte: 10 }
      - { this: tasks.A.stdout.message, contains: warning }
      - not: { this: tasks.A.stdout.message, matches: "^ignore" }
    cmd: [curl, -sS, --fail-with-body, -X, POST, '${BASE}/warn']
  D:                                          # no `when`: always runs
    cmd: [curl, -sS, --fail-with-body, '${BASE}/done']
```

Example: the `conditional` job in [`examples/httpbin.yaml`](../examples/httpbin.yaml). Run it as is to run `big`, or set `COUNT` to `5` in [`examples/httpbin.env`](../examples/httpbin.env) to run `warn` instead.

## Where `when` goes

| On | Checked | When false |
|---|---|---|
| a job | before its first task, against `values`, the `env_file` and the environment (no `tasks.*` yet) | no task runs; `exec` prints `null` |
| a cmd task | when the job reaches the task | the task doesn't run |
| a `for_each` or `while` loop | once, when the job reaches the loop | the whole loop is skipped |
| a `trigger` task | when the job reaches it | the job isn't queued |
| a [task file entry](files.md#reusing-a-task-file) | when the job reaches it, with the entry's `with` inputs set | the task doesn't run |

- **A skipped task has no result.** `tasks.NAME` stays undefined (or keeps the result of an earlier task with that name), so test it with `exists`: `when: {this: tasks.B, exists: false}` runs a task only if `B` was skipped.
- **Task files:** `when` can sit next to `file` and `with`. If the task file has its own `when` too, both must hold; the entry's is checked first. The file's `when` can read the entry's `with` inputs.
- **Job files:** put `when` in the job file. A `jobs` entry with `file` can't have it.
- With `run`, a skipped task writes a `skip` line with its `task`; a skipped job writes a `skip` line without one, then its `done` line.

## Condition shape

A condition is either `this` with **one or more** operators, or exactly one of `all`, `any`, `not`, or a list.

- **Several operators** must all hold, checked in the table's order and stopping at the first false: `{this: X, gt: 1, lt: 10}` is a range. `exists` can't be combined with other operators.
- **A list** is shorthand for `all`: `when: [{this: A, eq: 1}, {this: B, eq: 2}]` is `when: {all: [...]}`. It works anywhere a condition does, including inside `all`, `any` and `not`.

`this` names the variable or dot path to test, with or without `${}`: `this: tasks.login.exit_code` or `this: ${tasks.login.exit_code}`. Inside `{...}`, quote a `${}` so YAML doesn't end the map at its `}`: `{this: '${tasks.login.exit_code}', eq: 0}`.

`this` is looked up in the [usual order](variables.md#lookup-order). An undefined variable fails the task, except with `exists`. Values from env files and the environment are strings, which is why numeric strings are accepted by the number operators.

## Operators

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

Operator values other than `matches` may use `${...}`, expanded when the condition runs, to compare two variables: `{this: ID, gt: "${LAST_ID}"}`. A value that is exactly `"${NAME}"` keeps its JSON type; `$${` is a literal `${`. If a number operator's value doesn't expand to a number, the task fails with ``condition: `gt` needs a number, got ...``. `matches` is fixed.

## Errors

Conditions are checked when the project loads, before any task runs. A non-boolean for `exists`, an invalid regex or an unknown key fails as ``invalid YAML in <file>``, with the cause. An operator without `this`, `exists` combined with another operator, `this` mixed with `all`/`any`/`not`, or a non-number for `gt`/`gte`/`lt`/`lte` (other than a `${...}`) fails as an invalid condition, e.g.:

```
Error: job `w`

Caused by:
    invalid task: `route`: invalid condition: `gt` needs a number, got "ten"
```

A task that still uses the removed `if`/`then`/`else` or `switch` fails to load; see [Removed syntax](yaml.md#removed-syntax).
