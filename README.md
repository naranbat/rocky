# rocky

A CLI for automating repeatable work: chain HTTP requests and commands into YAML projects, run on demand or on a schedule. A project file holds named jobs; each job runs steps in order, where later steps read earlier results, and can keep named values between runs. Run a job with `rocky exec`, or run jobs on a schedule with `rocky run`, and get the result back as JSON, with status, headers, and body kept separate.

## Install

Needs a Rust toolchain (stable) and a Unix system (Linux or macOS).

```sh
cargo install --path .     # puts `rocky` on your PATH
# or
cargo build --release      # binary at target/release/rocky
```

## Quick start

```sh
rocky exec examples/httpbin.yaml login | jq .body   # needs network access and jq
```

These projects in [`examples/`](examples/) run as they are:

- [`httpbin.yaml`](examples/httpbin.yaml): requests, commands, branches, loops and timeouts against https://httpbin.org
- [`imports.yaml`](examples/imports.yaml): the same, with every job in its own file under [`jobs/`](examples/jobs/)
- [`env-file.yaml`](examples/env-file.yaml): the env file [`example.env`](examples/example.env), against httpbin.org
- [`cmd-env.yaml`](examples/cmd-env.yaml): values passed to commands with `env`; only runs commands
- [`state.yaml`](examples/state.yaml): a counter kept between runs; only runs commands

[`steps/`](examples/steps/) holds step files.

Start your own project from a template:

```sh
rocky new project project.yaml             # a project with one job, `example`
rocky validate project.yaml                # check a project without running it
rocky exec project.yaml example
rocky new job jobs/deploy.yaml             # a job file
rocky new step steps/login.yaml            # a step file; asks cmd or http
rocky new project full.yaml --level full   # every field, with comments
```

## Documentation

Each feature has its own page in [`docs/`](docs/README.md):

- [Command line](docs/cli.md): `exec`, `run`, `validate` and `new`, flags, output, exit codes
- [Projects](docs/projects.md): jobs, steps, fields, validation, using earlier steps' output
- [HTTP steps](docs/http-steps.md): HTTP request fields (`get:`, `post:`, … or `http:`) and behaviour
- [Responses](docs/responses.md): the `{status, headers, body}` result
- [Cmd steps](docs/cmd-steps.md): running programs, `stdin`, reading and writing files
- [Env](docs/env.md): `${NAME}` substitution and env files
- [State](docs/state.md): values kept between runs with a job's `states` and `set`
- [Conditions](docs/conditions.md): `if` / `then` / `else`, `switch`
- [Loops](docs/loops.md): `for_each` / `as` / `do`
- [Debugging](docs/debugging.md): `run` log lines
- [Run](docs/run.md) and [schedules](docs/schedules.md): running jobs on a schedule

## Limitations

- **Variables in `url` are not percent-encoded.** A value containing `&`, `#`, or `?` changes the URL. Put such values under `query:`, which is encoded.
- **Repeated response headers are joined with `", "`**, which mangles multiple `set-cookie` values.
- **No cookie store between steps.** Pass session cookies yourself. For a single cookie without attributes, `Cookie: ${steps.login.headers.set-cookie}` works; anything more needs a cmd step to extract the `name=value` pairs.
- `steps.NAME...` dot paths can't address keys that contain `.`.
- Request bodies are JSON only (no form or multipart).
- No dry run: requests are always sent.
- `timeout` is in whole seconds.
- Ctrl-C, SIGTERM, SIGHUP (closing the terminal) and SIGQUIT stop `rocky` and the commands it runs; `kill -9` can't be caught, so it leaves running commands behind.
- Unix only: commands run in their own process group, which needs Unix signals.
