# Rocky

A CLI for automating repeatable work: chain commands into YAML projects, run on demand or on a schedule. A project file holds named jobs; each job runs tasks in order, where later tasks read earlier results. Run a job with `rocky exec`, or run jobs on a schedule with `rocky run`, which can also run jobs on request from `rocky trigger` over a Unix socket, and get the result back as JSON. HTTP requests are commands too: run `curl`, whose JSON output is parsed so later tasks can read its fields.

## Install

Needs a Rust toolchain (stable) and a Unix system (Linux or macOS).

```sh
cargo install --path .     # puts `rocky` on your PATH
# or
cargo build --release      # binary at target/release/rocky
```

## Quick start

```sh
rocky exec examples/httpbin.yaml login | jq .stdout   # needs network access, curl and jq
```

These projects in [`examples/`](examples/) run as they are:

- [`httpbin.yaml`](examples/httpbin.yaml): `curl` requests, other commands, branches, loops and timeouts against https://httpbin.org
- [`imports.yaml`](examples/imports.yaml): the same, with every job in its own file under [`jobs/`](examples/jobs/)
- [`env-file.yaml`](examples/env-file.yaml): the env file [`example.env`](examples/example.env), against httpbin.org
- [`cmd-env.yaml`](examples/cmd-env.yaml): values passed to commands with `env`; only runs commands
- [`pipe.yaml`](examples/pipe.yaml): commands connected with `pipe`, without a shell; only runs commands
- [`values.yaml`](examples/values.yaml): fixed values at project and job level; only runs commands

[`tasks/`](examples/tasks/) holds task files.

Start your own project from a template:

```sh
rocky new project project.yaml             # a project with one job, `example`
rocky validate project.yaml                # check a project without running it
rocky exec project.yaml example
rocky run project.yaml --socket /tmp/rocky.sock   # runs until stopped; jobs run on request
rocky trigger /tmp/rocky.sock example             # from another terminal
rocky new job jobs/deploy.yaml             # a job file
rocky new task tasks/login.yaml            # a task file
rocky new project full.yaml --level full   # every field, with comments
```

## Documentation

Each feature has its own page in [`docs/`](docs/README.md):

- [Command line](docs/cli.md): `exec`, `run`, `validate` and `new`, flags, output, exit codes
- [Projects](docs/projects.md): jobs, tasks, fields, validation, using earlier tasks' output
- [Cmd tasks](docs/cmd-tasks.md): running programs, pipes, `stdin`, reading and writing files, HTTP requests with `curl`
- [Env](docs/env.md): `${NAME}` substitution and env files
- [Conditions](docs/conditions.md): `if` / `then` / `else`, `switch`, `while`
- [Loops](docs/loops.md): `for_each` / `as` / `do`
- [Debugging](docs/debugging.md): `run` log lines
- [Run](docs/run.md) and [schedules](docs/schedules.md): running jobs on a schedule, or when a client of a Unix socket asks
- [Service](docs/service.md): running `rocky run` in the background with systemd (Linux) or launchd (macOS)

## Limitations

- `tasks.NAME...` dot paths can't address keys that contain `.`.
- No dry run: commands always run.
- `timeout` is in whole seconds.
- Ctrl-C, SIGTERM, SIGHUP (closing the terminal) and SIGQUIT stop `rocky` and the commands it runs; `kill -9` can't be caught, so it leaves running commands behind.
- Unix only: commands run in their own process group, which needs Unix signals.
