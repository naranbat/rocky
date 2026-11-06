# rocky documentation

One page per topic. For installing and a first run, see the [project README](../README.md).

## Getting started

| Page | What it covers |
|---|---|
| [Projects](projects.md) | The project file and its `jobs`, project and job fields, `defaults`, execution order, validation and paths |
| [Writing YAML](yaml.md) | Editor schema, quoting, durations, and syntax that was removed |

## Writing projects

| Page | What it covers |
|---|---|
| [Tasks](tasks.md) | Task kinds and fields, results, using earlier tasks' output, failures and `allow_failure` |
| [Cmd tasks](cmd-tasks.md) | Running programs with `cmd:`, pipes, `stdin`, options, `retry`, reading and writing files |
| [Variables](variables.md) | `${NAME}` substitution, `values`, `.env` files, lookup order, typed values and dot paths |
| [Conditions](conditions.md) | `when` on tasks, and condition operators |
| [Loops](loops.md) | `for_each` and `while` loops |
| [Triggers](triggers.md) | `trigger` tasks, and running jobs over a socket with `rocky trigger` |

## Running

| Page | What it covers |
|---|---|
| [Command line](cli.md) | `rocky exec`, `run`, `trigger`, `validate` and `new`, their flags, output, and exit codes |
| [Run](run.md) | Running a project's scheduled jobs, and its JSON log lines |
| [Schedules](schedules.md) | `every`, `hourly`, `daily`, `weekly`, `at` and `run_on_start` |
| [Service](service.md) | Running `rocky run` in the background with systemd or launchd |
| [Debugging](debugging.md) | `validate`, `exec --verbose`, reading `run`'s logs |
