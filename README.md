# Rocky

A CLI for automating repeatable work. A project is a `rocky.yaml` that lists its jobs, each in its own YAML file, and holds settings they share; each job runs its tasks (commands) in order, and later tasks can read earlier tasks' output. Run a job once with `rocky exec`, or keep `rocky run` going to run jobs on a schedule or on request from `rocky trigger`.

## Install

Needs a Rust toolchain (stable) and a Unix system (Linux or macOS).

```sh
cargo install --path crates/rocky     # puts `rocky` on your PATH
# or
cargo build --release      # binary at target/release/rocky
```

## Quick start

Create a project from the template, then add a second task to `jobs/example.yaml` that reads the first one's output:

```sh
mkdir automation && cd automation
rocky new project                  # writes rocky.yaml and jobs/example.yaml
```

```yaml
# jobs/example.yaml: job `example`
tasks:
  hello:
    cmd: [echo, hello]             # program + arguments, no shell involved
  shout:
    cmd: [tr, a-z, A-Z]
    stdin: ${tasks.hello.stdout}   # output of the earlier task
```

```sh
rocky validate                     # check it without running anything
rocky exec example                 # run the job once
```

`exec`, `run` and `validate` load `rocky.yaml` in the current directory, or another project file with `-f`:

```yaml
# rocky.yaml
jobs:
  example: jobs/example.yaml       # job name: its file, relative to this file
```

```sh
rocky run -f nightly.yaml          # logs to .rocky/logs/<date>-nightly.log
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

Larger projects are in [`examples/`](examples/), one directory each.

## Running jobs

```sh
rocky exec example --verbose         # print each command before it runs
rocky run                            # run scheduled jobs until stopped
rocky run --socket /tmp/rocky.sock   # ...and accept requests on a socket
rocky trigger /tmp/rocky.sock example   # ask a running `run` to run a job
```

See [Command line](docs/cli.md) for every flag, [Schedules](docs/schedules.md) for when jobs run, and [Service](docs/service.md) to keep `rocky run` going in the background.

## Templates

```sh
rocky new project                       # a project with one job, here
rocky new job deploy                    # jobs/deploy.yaml; list it under `jobs` in rocky.yaml
rocky new project full --level full     # every field, with comments, in full/
```

## Builder

`rocky-builder` is a terminal UI for building projects: it edits job files, and runs and validates them through the same library as the CLI. It is a separate binary, which `cargo install --path crates/rocky` doesn't install:

```sh
cargo install --path crates/rocky-builder   # puts `rocky-builder` on your PATH
rocky-builder path/to/project               # or no argument, to pick or create one
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

- **Getting started:** [Projects](docs/projects.md), [Writing YAML](docs/yaml.md)
- **Writing projects:** [Tasks](docs/tasks.md), [Cmd tasks](docs/cmd-tasks.md), [Variables](docs/variables.md), [Conditions](docs/conditions.md), [Loops](docs/loops.md), [Triggers](docs/triggers.md)
- **Running:** [Command line](docs/cli.md), [Run](docs/run.md), [Schedules](docs/schedules.md), [Service](docs/service.md), [Debugging](docs/debugging.md)

## Limitations

- Unix only: commands run in their own process group, which needs Unix signals.
- [Durations](docs/yaml.md#durations-and-times) take one whole unit: no fractions (`1.5m`), combined units (`1h30m`) or milliseconds.
- Dot paths can't step into a key that contains `.` (e.g. `{"user.id": 7}` in a task's stdout).
- Ctrl-C, SIGTERM, SIGHUP (closing the terminal) and SIGQUIT stop `rocky` and the commands it runs; `kill -9` can't be caught, so it leaves running commands behind.
