# Rocky

A CLI for automating repeatable work. A project is a YAML file of named jobs; each job runs its tasks (commands) in order, and later tasks can read earlier tasks' output. Run a job once with `rocky exec`, or keep `rocky run` going to run jobs on a schedule or on request from `rocky trigger`.

## Install

Needs a Rust toolchain (stable) and a Unix system (Linux or macOS).

```sh
cargo install --path .     # puts `rocky` on your PATH
# or
cargo build --release      # binary at target/release/rocky
```

## Quick start

Create a project from the template, then add a second task that reads the first one's output:

```sh
rocky new project project.yaml
```

```yaml
jobs:
  example:
    tasks:
      hello:
        cmd: [echo, hello]           # program + arguments, no shell involved
      shout:
        cmd: [tr, a-z, A-Z]
        stdin: ${tasks.hello.stdout} # output of the earlier task
```

```sh
rocky validate project.yaml        # check it without running anything
rocky exec project.yaml example    # run the job once
```

`exec` prints the result of the last task as JSON:

```json
{
  "ok": true,
  "exit_code": 0,
  "stdout": "HELLO",
  "stderr": ""
}
```

Larger projects are in [`examples/`](examples/).

## Running jobs

```sh
rocky exec project.yaml example --verbose        # print each command before it runs
rocky run project.yaml                           # run scheduled jobs until stopped
rocky run project.yaml --socket /tmp/rocky.sock  # ...and accept requests on a socket
rocky trigger /tmp/rocky.sock example            # ask a running `run` to run a job
```

To keep `rocky run` going in the background, see [Service](docs/service.md).

## Templates

```sh
rocky new project project.yaml             # a project with one job
rocky new job jobs/deploy.yaml             # a job in its own file
rocky new task tasks/login.yaml            # a task in its own file
rocky new project full.yaml --level full   # every field, with comments
```

## Agent skill

[`skills/rocky/`](skills/rocky/SKILL.md) teaches Claude Code and OpenCode agents to write, validate and run rocky projects. Link it into your agent's skills directory:

```sh
ln -s "$PWD/skills/rocky" ~/.claude/skills/rocky             # Claude Code
ln -s "$PWD/skills/rocky" ~/.config/opencode/skills/rocky    # OpenCode
```

OpenCode also reads `~/.claude/skills/`, so the first link covers both.

## Documentation

All pages are in [`docs/`](docs/README.md).

- **Running things**
  - [Command line](docs/cli.md): `exec`, `run`, `trigger`, `validate` and `new`, flags, output, exit codes
  - [Projects](docs/projects.md): jobs, tasks, fields, validation, using earlier tasks' output
- **Tasks**
  - [Cmd tasks](docs/cmd-tasks.md): running programs, pipes, `stdin`, reading and writing files
- **Data between tasks**
  - [Env](docs/env.md): `${NAME}` substitution and env files
- **Control flow**
  - [Conditions](docs/conditions.md): `if` / `then` / `else`, `switch`, `while`
  - [Loops](docs/loops.md): `for_each` / `as` / `do`
- **Operating**
  - [Run](docs/run.md): running jobs on a schedule, or when a client of a Unix socket asks
  - [Schedules](docs/schedules.md): `every`, `hourly`, `daily`, `weekly` and `at`
  - [Service](docs/service.md): running `rocky run` in the background with systemd (Linux) or launchd (macOS)
  - [Debugging](docs/debugging.md): `run` log lines

## Limitations

- Unix only: commands run in their own process group, which needs Unix signals.
- `timeout` is in whole seconds, minutes or hours (`30`, `30s`, `5m`, `2h`).
- `tasks.NAME...` dot paths can't address keys that contain `.`.
- Ctrl-C, SIGTERM, SIGHUP (closing the terminal) and SIGQUIT stop `rocky` and the commands it runs; `kill -9` can't be caught, so it leaves running commands behind.
