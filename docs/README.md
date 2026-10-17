# rocky documentation

One page per feature. For installing and a first run, see the [project README](../README.md).

## Running things

| Page | What it covers |
|---|---|
| [Command line](cli.md) | `rocky exec`, `rocky run`, `rocky validate` and `rocky new`, their flags, output, and exit codes |
| [Projects](projects.md) | The project file: jobs, tasks, fields, job and task files, validation, and using earlier tasks' output |

## Tasks

| Page | What it covers |
|---|---|
| [HTTP tasks](http-tasks.md) | HTTP requests (`get:`, `post:`, … or `http:`): their fields and behaviour |
| [Responses](responses.md) | The `{status, headers, body}` result of an HTTP task |
| [Cmd tasks](cmd-tasks.md) | Running programs with `cmd:`, `stdin`, reading and writing files |

## Data between tasks

| Page | What it covers |
|---|---|
| [Env](env.md) | `${NAME}` substitution, `.env` files, typed values, dot paths, and lookup order |
| [State](state.md) | Keeping values between runs with a job's `states` and `set` |

## Control flow

| Page | What it covers |
|---|---|
| [Conditions](conditions.md) | `if` / `then` / `else` branches, `switch`, and their operators |
| [Loops](loops.md) | `for_each` / `as` / `do` over a list |

## Operating

| Page | What it covers |
|---|---|
| [Debugging](debugging.md) | `run`'s JSON log lines |
| [Run](run.md) | Running a project's scheduled jobs |
| [Schedules](schedules.md) | `every`, `hourly`, `daily`, `weekly` and `at` |
