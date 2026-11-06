# Loops

A loop is a task that runs its `do` tasks several times, then lets the job continue with the task after the loop. There are two kinds:

- [`for_each`](#for_each) runs `do` once for each element of a list.
- [`while`](#while) runs `do` again and again while a condition holds.

To run one command again until it succeeds or a condition holds, [`retry`](cmd-tasks.md#retry) is shorter.

## `for_each`

```yaml
tasks:
  users:
    cmd: [curl, -sS, --fail-with-body, '${BASE}/users']
  notify_all:
    for_each: tasks.users.stdout.items        # a list
    as: USER                                  # optional, default ITEM
    do:
      notify:
        cmd: [curl, -sS, --fail-with-body, -X, POST, '${BASE}/notify/${USER.id}']
  done:                                       # runs once, after every element
    cmd: [curl, -sS, --fail-with-body, '${BASE}/done']
```

Example: [`examples/httpbin/jobs/loop.yaml`](../examples/httpbin/jobs/loop.yaml).

| Field | Notes |
|---|---|
| `for_each` | Required. A variable name or dot path, with or without `${}`. Its value must be a list. |
| `as` | Optional, default `ITEM`. The name of the current element inside `do`. Must be a [variable name](variables.md#names) other than `tasks`. |
| `do` | Required, non-empty. The tasks to run for each element. |
| `on_failure` | Optional, `stop` (the default) or `next`. With `next`, a failing `do` task skips the rest of that element's tasks and the loop goes on with the next element. See [Behaviour](#behaviour). |

A `for_each` loop can't also have `cmd`, `timeout`, `while` or `attempts`.

### The list

- `for_each` is looked up like a condition's `this`, in the [usual order](variables.md#lookup-order).
- The value must be a JSON array. Arrays come from `tasks.NAME...` or an outer loop's element. Values from env files and the environment are strings, so they can't be looped over.
- A non-list value fails the task with ``for_each: `X` is not a list (…)``. An undefined variable fails it too.
- An empty list runs nothing.

### The current element

- Inside `do`, the element is `${ITEM}` (or the `as` name). Dot paths work for objects: `${USER.id}`, `when: { this: USER.id, eq: 7 }`.
- Where a value is exactly `${USER}`, the element keeps its JSON type; see [typed values](variables.md#typed-values).
- When the loop ends, the `as` variable goes back to the value an outer loop gave it, or stops being set by the loop.

## `while`

A `while` loop checks its condition before each run of its `do` tasks; once it's false, the job continues with the task after the loop. If it's false the first time, `do` never runs.

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

| Field | Notes |
|---|---|
| `while` | Required. A [condition](conditions.md#condition-shape), evaluated like `when`'s. An undefined variable fails the task, except with `exists`. |
| `attempts` | Optional, at least 1. The most times `do` may run, like [`retry`](cmd-tasks.md#retry)'s `attempts`. If the condition still holds after that many runs, the task fails with ``while: still true after N attempts``. Without it, the loop has no limit. |
| `do` | Required, non-empty. The tasks to run on each iteration. |
| `on_failure` | Optional, `stop` (the default) or `next`. With `next`, a failing `do` task skips the rest of that iteration; the condition is checked again next. A skipped iteration counts toward `attempts`. |

A `while` loop can't also have `cmd`, `timeout`, `for_each` or `as`.

## Behaviour

These apply to both kinds of loop.

- **Name:** like every task, a loop is named by its key, which labels it in errors and `run`'s log lines (`for_each` and `while` [events](run.md#output)).
- **Not a cmd task:** a loop can't have `allow_failure`, which is for cmd tasks: a loop with it fails to load, pointing to `on_failure: next`. It doesn't take [`defaults`](projects.md#defaults), though the cmd tasks inside it do.
- **`when`:** a loop can have [`when`](conditions.md), checked once before the loop starts. Any task inside `do` can have its own `when`, checked on each iteration.
- **Sequential:** iterations run one after another, never in parallel. Nothing pauses between them unless a `do` task does (`sleep`).
- **Variables persist:** `tasks.NAME` results set inside `do` stay available after the loop. Each iteration overwrites the last, so the final iteration's values win.
- **Failures:** a task that fails without `allow_failure` stops the whole job, even in the middle of a loop, unless the loop (or an outer loop) has `on_failure: next`. Then a non-zero exit, a timeout or a [`retry`](cmd-tasks.md#retry) that ran out anywhere inside `do`, including nested loops, skips the rest of that iteration of the innermost such loop, which goes on with its next element or condition check. The failed task's result stays in `tasks.NAME`. Other errors still stop the job: an undefined variable, a missing program or `cwd`, a `while` loop that runs out of `attempts`, or a `for_each` value that isn't a list.
- **Nesting:** loops can be nested inside each other. Give nested `for_each` loops different `as` names.
- **Result:** if the loop is the last task, the printed result is the last task that ran in the last iteration (for an empty list, the task before the loop).
