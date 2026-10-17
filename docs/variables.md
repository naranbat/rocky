# Variables

Write `${NAME}` to insert a value. `NAME` can be a plain name (`${TOKEN}`) or a dot path into a JSON value (`${steps.login.body.user.id}`, `${USER.id}`).

```yaml
vars:
  BASE: https://httpbin.org
  USER_ID: "42"
jobs:
  user:
    steps:
      fetch:
        get:
          url: ${BASE}/anything/users/${USER_ID}
          headers:
            Authorization: Bearer ${API_TOKEN}   # from the environment or an env_file
```

## Where `${...}` is expanded

| Expanded | Not expanded |
|---|---|
| HTTP `url` | header names, `query` keys, `json` keys |
| HTTP header values | `env` names, a [condition](conditions.md)'s `this` and a `switch` (they name a variable, not `${...}`) |
| HTTP `query` values | [`for_each`](loops.md) (it names a variable directly) |
| any string inside HTTP `json` | |
| each [`cmd`](cmd-steps.md) argument | schedules, `matches` patterns, switch `cases` keys |
| a cmd step's `cwd` | |
| any string inside a cmd step's `stdin` | |
| any string inside a cmd step's [`env`](cmd-steps.md#options) values | |
| condition operator values other than `matches` | |
| [`set`](state.md#set) values | |
| values in project `vars` ([using other vars](#vars-that-use-other-vars)) | values in job and step `vars` (kept literally) |

Env files expand `${NAME}` themselves when they're read; see [Env files](env-files.md#format).

An undefined variable fails the step: ``variable `NAME` is not defined (checked steps, states, loop items, vars, env_file, environment)``.

## Where values come from

| Source | Type |
|---|---|
| earlier steps, as `steps.NAME...` ([Projects](projects.md#using-earlier-steps-output)) | any JSON type |
| loop element from [`for_each`](loops.md) | any JSON type |
| job [state](state.md), as `states.NAME` | any JSON type |
| `vars` on a step, a job or the project | string (a project var set by [`set`](#updating-vars-at-runtime): any JSON type) |
| the project's [env file](env-files.md) | string |
| the process environment | string |

These only fill in `${...}`. A command doesn't see `vars` or env file values in its environment unless its step lists them under [`env`](cmd-steps.md#options): `env: {TOKEN: ${API_TOKEN}}`.

## Names

A name under any `vars` or in a loop's `as` must be uppercase: an uppercase letter or `_`, then uppercase letters, digits or `_` (`TOKEN`, `USER_ID`, `_TMP2`). A name under `states` or a step key must be an identifier: a letter or `_`, then letters, digits or `_` (`count`, `user_id`); `steps` and `states` are reserved. Anything else, such as `user_id` under `vars`, `my-var` or `a.b`, fails when the project loads. Names from an env file or the environment aren't checked.

## `vars` levels

`vars` can go on the project, on a job, and on any step. They're all maps of name → string.

```yaml
vars: {BASE: https://httpbin.org, WHO: project}
jobs:
  users:
    vars: {WHO: job}                       # every step in `users`
    steps:
      who:
        cmd: [echo, '${WHO}']              # → job
      ids:
        cmd: [echo, '[1, 2]']
      each:
        for_each: steps.ids.stdout         # a list
        vars: {WHO: loop}                  # this loop and every step in its `do`
        do:
          item:
            cmd: [echo, '${WHO} ${ITEM}']  # → loop 1, loop 2, ...
          own:
            cmd: [echo, '${WHO}']
            vars: {WHO: step}              # this step only → step
      greet: steps/greet.yaml              # a step file: its `vars` go in the file
```

- On a branch, switch or loop, `vars` apply to its own `if`, `switch` or `for_each` check, and to every step nested inside it.
- A step's `vars` don't reach the steps after it.
- A [step file](projects.md#job-and-step-files) keeps its `vars` in the file; the entry that uses it is just a path, so it can't override them. To vary them, inline the step instead.

### Vars that use other vars

A project `vars` value can contain `${NAME}` too:

```yaml
vars:
  PROJECTS_DIR: ${HOME}/projects                  # from the environment
  BACKEND_DIR: ${PROJECTS_DIR}/backend
jobs:
  build:
    steps:
      version:
        cmd: [cat, VERSION]
        cwd: ${BACKEND_DIR}
```

- **Project `vars`** are expanded from the other project vars (with any values `set` has given them), the env file and the process environment. They're expanded again for every step, so after `set: {vars: {B: new}}`, a var `A: prefix-${B}` is `prefix-new`. Each value is checked when the run starts: two project vars that refer to each other (`A: ${B}`, `B: ${A}`) stop the run before any step: ``variable `A` refers to itself through its own value``. So does a project var that uses an undefined name, even if nothing uses that var.
- **Job and step `vars`** are not expanded: `IMAGE: backend:${TAG}` is the literal text `backend:${TAG}`. Use the `${...}` directly where the value is needed instead.
- A project var can't build on itself: `DIR: ${DIR}/sub` refers to itself.
- The result is always a string. Write `$${` for a literal `${` in a project var value.

## Updating vars at runtime

A `set` step with `vars` sets project `vars` while the project runs, so one job can hand values to the others: a token one job fetches and others use, a counter several jobs bump.

```yaml
vars:
  TICKS: "0"                                # the starting value
schedules:
  tick: {every: 2s}
  report: {every: 5s}
jobs:
  tick:
    steps:
      inc:
        cmd: [expr, '${TICKS}', +, '1']
      keep:
        set:
          vars:
            TICKS: ${steps.inc.stdout}
  report:
    steps:
      print:
        cmd: [echo, 'ticks so far: ${TICKS}']
```

```sh
rocky run examples/set-vars.yaml
```

Example: [`examples/set-vars.yaml`](../examples/set-vars.yaml).

- **Only project `vars`:** every name under `set.vars` must be declared in the project's `vars`. This is checked when the project loads. Job and step `vars` can't be updated.
- **Values:** each value has `${...}` substituted, and a value that is exactly `${NAME}` keeps its JSON type, so a var can then hold a number, list or object, with dot paths into it (`${USERS.0.id}`). Every value is worked out first, then all are set together.
- **Seen by:** the steps after it, in this job and in every other one, including runs that are in the middle of running. When two jobs update the same name, the last update wins. A later step that fails doesn't undo the update.
- **Shadowed:** a job or step `vars` entry with the same name still wins over an updated value (see [lookup order](#lookup-order)). That applies to the bare name only: a dot path such as `${U.0.id}` reads the updated project value.
- **Where it goes:** anywhere a step can go: inside a branch, switch or loop, or in a step file. Only `vars` may go with it, and the same `set` can also set [state](state.md#set). It has no result: it isn't saved under `steps`, and it doesn't count as the last step that ran.

Updates are kept in memory only:

- **`rocky run`:** every scheduled job sees the same updated values for as long as `run` runs. A var that has never been updated still follows the project file, which is reloaded on every run; an updated one keeps its new value until `run` stops.
- **`rocky exec`:** runs a single job, so each run starts from the project's values and only that run sees its updates.

To keep a value between processes, use [State](state.md) instead.

## Lookup order

First match wins, for every kind of step, and for conditions, `switch` and `for_each`:

1. loop variables, `steps.NAME...` and `states.NAME`
2. the step's own `vars`, then those of each enclosing branch, switch or loop, from the innermost outwards
3. job `vars`
4. project `vars` (the value from the last [`set`](#updating-vars-at-runtime), if any)
5. project `env_file`
6. the process environment

## Typed values

Values from `vars`, env files and the environment are strings, except a project var set by `set`, which keeps the type it was set with. Values from loops, `steps`, `states` and `set` keep their JSON type when a **`json`** or **`stdin`** field, a `set` value or a condition operator value is **exactly** `${NAME}`. Anywhere else, they're rendered as text, with objects and arrays as compact JSON.

```yaml
# after a step `stats` that printed {"count": 5, "tags": ["a", "b"]}
json:
  count: ${steps.stats.stdout.count}       # → 5          (number)
  tags: ${steps.stats.stdout.tags}         # → ["a","b"]  (array)
  label: n=${steps.stats.stdout.count}     # → "n=5"      (text)
  both: ${steps.stats.stdout.count}${steps.stats.stdout.count} # → "55" (text: not a single variable)
url: https://x/items/${steps.stats.stdout.count}   # → https://x/items/5
```

So `users: ${steps.fetch.body}` sends the whole fetched object, not a string. For a cmd step's stdin, the typed value is then written as compact JSON. Values in a cmd step's `env` are always text, so an object or array there becomes compact JSON either way.

## Dot paths

- Each `.` steps into an object key. A numeric segment indexes an array: `${steps.list.body.items.0.id}`.
- A name that exists exactly as written is used before it is split at `.`.
- Keys that themselves contain `.` can't be addressed.

## Escaping

- `$${` produces a literal `${`: `$${COUNT}` sends `${COUNT}`. In a `sh -c` script that reads a variable from [`env`](cmd-steps.md#options), write `$NAME`, or `$${NAME:-default}` for shell syntax that needs braces.
- A `$` not followed by `{` is left alone.
- An unclosed `${` is an error: ``unclosed `${` in `…` ``.
