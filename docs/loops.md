# Loops

A step with `for_each` is a loop. Its `do` steps run once for each element of a list, in order, and the job then continues with the step after the loop.

```yaml
steps:
  users:
    get: "${BASE}/users"
  notify_all:
    for_each: steps.users.body.items          # a list
    as: USER                                  # optional, default ITEM
    do:
      notify:
        post: "${BASE}/notify/${USER.id}"
  done:                                       # runs once, after every element
    get: "${BASE}/done"
```

Example: the `loop` job in [`examples/httpbin.yaml`](../examples/httpbin.yaml).

## Loop fields

| Field | Notes |
|---|---|
| `for_each` | Required. A variable name or dot path, **without** `${}`. Its value must be a list. |
| `as` | Optional, default `ITEM`. The name of the current element inside `do`. Must be a [variable name](variables.md#names) other than `steps` or `states`. |
| `do` | Required, non-empty. The steps to run for each element. |

Like every step, a loop is named by its key, which labels it in errors and `run`'s log lines.

A loop can't also have `http`, `cmd`, `timeout`, `allow_failure`, `if`, `then`, `else`, `switch`, `cases` or `default`.

## The list

- `for_each` is looked up like a [condition's `this`](conditions.md#variable-lookup).
- The value must be a JSON array. Arrays come from `steps.NAME...`, `states.NAME`, or an outer loop's element. Values from env files and the environment are strings, so they can't be looped over.
- A non-list value fails the step with ``for_each: `X` is not a list (…)``. An undefined variable fails it too.
- An empty list runs nothing.

## The current element

- Inside `do`, the element is `${ITEM}` (or the `as` name). Dot paths work for objects: `${USER.id}`, `if: { this: USER.id, eq: 7 }`.
- Where a value is exactly `${USER}`, the element keeps its JSON type; see [typed values](variables.md#typed-values).
- When the loop ends, the `as` variable goes back to the value an outer loop gave it, or stops being set by the loop.

## Behaviour

- **Sequential:** elements run one after another, never in parallel.
- **Variables persist:** `steps.NAME` results set inside `do` stay available after the loop. Each iteration overwrites the last, so the final element's values win.
- **Failures:** a step that fails without `allow_failure` stops the whole job, even in the middle of a loop.
- **Nesting:** loops and [branches](conditions.md) can be nested inside each other. Give nested loops different `as` names.
- **Result:** if the loop is the last step, the printed result is the last step that ran in the last iteration (for an empty list, the step before the loop).
