# Variables

Write `${NAME}` to insert a value. `NAME` can be a plain name (`${API_TOKEN}`) or a dot path into a JSON value (`${steps.login.body.user.id}`, `${USER.id}`).

```yaml
env_file: example.env
jobs:
  user:
    steps:
      fetch:
        get:
          url: https://httpbin.org/anything/users/42
          headers:
            Authorization: Bearer ${API_TOKEN}   # from the env_file or the environment
```

There is no `vars` field: a value you'd hard-code goes straight into the step, and a value that changes between machines or is secret goes in the [env file](env-files.md). To keep a value between runs, use [state](state.md).

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

Env files expand `${NAME}` themselves when they're read; see [Env files](env-files.md#format).

An undefined variable fails the step: ``variable `NAME` is not defined (checked steps, states, loop items, env_file, environment)``.

## Where values come from

| Source | Type |
|---|---|
| earlier steps, as `steps.NAME...` ([Projects](projects.md#using-earlier-steps-output)) | any JSON type |
| loop element from [`for_each`](loops.md) | any JSON type |
| job [state](state.md), as `states.NAME` | any JSON type |
| the project's [env file](env-files.md) | string |
| the process environment | string |

These only fill in `${...}`. A command doesn't see env file values in its environment unless its step lists them under [`env`](cmd-steps.md#options): `env: {TOKEN: ${API_TOKEN}}`.

## Names

A loop's `as` name must be uppercase: an uppercase letter or `_`, then uppercase letters, digits or `_` (`ITEM`, `USER_ID`, `_TMP2`). A name under `states` or a step key must be an identifier: a letter or `_`, then letters, digits or `_` (`count`, `user_id`); `steps` and `states` are reserved. Anything else fails when the project loads. Names from an env file or the environment aren't checked.

## Lookup order

First match wins, for every kind of step, and for conditions, `switch` and `for_each`:

1. loop variables, `steps.NAME...` and `states.NAME`
2. project `env_file`
3. the process environment

## Typed values

Values from env files and the environment are strings. Values from loops, `steps` and `states` keep their JSON type when a **`json`** or **`stdin`** field, a `set` value or a condition operator value is **exactly** `${NAME}`. Anywhere else, they're rendered as text, with objects and arrays as compact JSON.

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
