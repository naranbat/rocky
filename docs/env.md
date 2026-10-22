# Env

Write `${NAME}` to insert a value. `NAME` can be a plain name (`${API_TOKEN}`) or a dot path into a JSON value (`${tasks.login.body.user.id}`, `${USER.id}`). Plain names come from `values`, the project's env file or the process environment; `tasks` and loop items come from the run itself.

```sh
# example.env
API_TOKEN=example-token
export REGION="eu-west-1"   # `export` and quotes are allowed
```

```yaml
env_file: example.env        # relative to the project file
jobs:
  user:
    tasks:
      fetch:
        get:
          url: https://httpbin.org/anything
          headers:
            Authorization: Bearer ${API_TOKEN}
      region:
        cmd: [sh, -c, 'echo "deploying to $REGION"']
        env:                 # a command sees env file values only when listed here
          REGION: ${REGION}
```

Examples: [`examples/env-file.yaml`](../examples/env-file.yaml) and [`examples/cmd-env.yaml`](../examples/cmd-env.yaml) with [`examples/example.env`](../examples/example.env).

A value you'd hard-code goes in [`values`](#values), and a value that changes between machines or is secret goes in the env file.

## Values

Fixed values written in the YAML, read as `${NAME}`.

```yaml
values:                      # the project's: seen by every task of every job
  GREETING: hello
  SITE:
    name: rocky
    retries: 3
jobs:
  greet:
    values:                  # the job's: seen by this job's tasks only
      GREETING: hi           # wins over the project's GREETING in this job
    tasks:
      say:
        cmd: [echo, '${GREETING} from ${SITE.name}, ${SITE.retries} retries']
```

Example: [`examples/values.yaml`](../examples/values.yaml).

- `values` can go at the top level of a project and in a job, inline or in a job file. Tasks nested in branches, switches and loops, and task files, see their job's values. Tasks can't have their own `values`.
- A value can be a string, number, boolean, list or object, and keeps its JSON type (see [typed values](#typed-values)). Use a dot path to read inside one: `${SITE.name}`.
- Values are used as written: `${...}` inside a value is not expanded.
- A job value with the same name as a project value replaces it for that job. A job entry that [reuses a job file](projects.md#reusing-a-job-file) can set `values` too; they replace the file's.

## Env file

Use an `env_file` to keep secrets such as API tokens out of the YAML that you commit.

### Where it can go

Only at the top level of a project, with a path relative to the project file. It applies to every task of every job. Job files, task files and HTTP requests can't have one: `env_file` there is an unknown field.

### Format

- One `KEY=value` per line. Lines starting with `#` are comments.
- Values may be single- or double-quoted.
- A leading `export ` is ignored.
- Every value is a string.
- Unquoted and double-quoted values expand `${NAME}`. Always use the braces: a bare `$NAME` stops at the first character that isn't a letter or digit, so `$API_TOKEN` reads `$API` followed by `_TOKEN`, and inside double quotes a bare `$NAME` just before the closing quote fails to parse. Names come from the process environment, then earlier lines of the file, so an exported variable wins over an earlier line. An undefined name becomes empty. Single-quote a value that contains a literal `$`, such as a password: `PASSWORD='pa$$word'`.

### Behaviour

- The file only provides values for `${...}`. It never changes the environment of the `rocky` process or of the commands that [cmd tasks](cmd-tasks.md) run; pass a value to a command with the task's `env`.
- The file is read when each run starts.
- A missing or malformed file fails the run with `failed to load env file <path>`. This is checked at run time, not when the project loads.
- Values are never masked. A cmd task that echoes a token, or a response that returns one, shows it in full when `exec` prints it as the result.

## Where `${...}` is expanded

| Expanded | Not expanded |
|---|---|
| HTTP `url` | header names, `query` keys, `json` keys |
| HTTP header values | `env` names, a [condition](conditions.md)'s `this` and a `switch` (they name a variable, not `${...}`) |
| HTTP `query` values | [`for_each`](loops.md) (it names a variable directly) |
| any string inside HTTP `json` | |
| each [`cmd`](cmd-tasks.md) argument | schedules, `matches` patterns, switch `cases` keys |
| a cmd task's `cwd` | |
| any string inside a cmd task's `stdin` | |
| any string inside a cmd task's [`env`](cmd-tasks.md#options) values | |
| condition operator values other than `matches` | |

An undefined variable fails the task: ``variable `NAME` is not defined (checked tasks, loop items, values, env_file, environment)``.

## Where values come from

| Source | Type |
|---|---|
| earlier tasks, as `tasks.NAME...` ([Projects](projects.md#using-earlier-tasks-output)) | any JSON type |
| loop element from [`for_each`](loops.md) | any JSON type |
| the job's [`values`](#values) | any JSON type |
| the project's [`values`](#values) | any JSON type |
| the project's [env file](#env-file) | string |
| the process environment | string |

These only fill in `${...}`. A command doesn't see env file values in its environment unless its task lists them under [`env`](cmd-tasks.md#options): `env: {TOKEN: ${API_TOKEN}}`.

## Lookup order

First match wins, for every kind of task, and for conditions, `switch` and `for_each`:

1. loop variables and `tasks.NAME...`
2. job `values`
3. project `values`
4. project `env_file`
5. the process environment

## Names

A `values` name and a loop's `as` name must be uppercase: an uppercase letter or `_`, then uppercase letters, digits or `_` (`ITEM`, `USER_ID`, `_TMP2`). A task key must be an identifier: a letter or `_`, then letters, digits or `_` (`count`, `user_id`); `tasks` is reserved. Anything else fails when the project loads. Names from an env file or the environment aren't checked.

## Typed values

Values from env files and the environment are strings. Values from `values`, loops and `tasks` keep their JSON type when a **`json`** or **`stdin`** field or a condition operator value is **exactly** `${NAME}`. Anywhere else, they're rendered as text, with objects and arrays as compact JSON.

```yaml
# after a task `stats` that printed {"count": 5, "tags": ["a", "b"]}
json:
  count: ${tasks.stats.stdout.count}       # → 5          (number)
  tags: ${tasks.stats.stdout.tags}         # → ["a","b"]  (array)
  label: n=${tasks.stats.stdout.count}     # → "n=5"      (text)
  both: ${tasks.stats.stdout.count}${tasks.stats.stdout.count} # → "55" (text: not a single variable)
url: https://x/items/${tasks.stats.stdout.count}   # → https://x/items/5
```

So `users: ${tasks.fetch.body}` sends the whole fetched object, not a string. For a cmd task's stdin, the typed value is then written as compact JSON. Values in a cmd task's `env` are always text, so an object or array there becomes compact JSON either way.

## Dot paths

- Each `.` tasks into an object key. A numeric segment indexes an array: `${tasks.list.body.items.0.id}`.
- A name that exists exactly as written is used before it is split at `.`.
- Keys that themselves contain `.` can't be addressed.

## Escaping

- `$${` produces a literal `${`: `$${COUNT}` sends `${COUNT}`. In a `sh -c` script that reads a variable from [`env`](cmd-tasks.md#options), write `$NAME`, or `$${NAME:-default}` for shell syntax that needs braces.
- A `$` not followed by `{` is left alone.
- An unclosed `${` is an error: ``unclosed `${` in `…` ``.
