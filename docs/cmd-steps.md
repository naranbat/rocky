# Cmd steps

A cmd step runs a program and produces `{exit_code, stdout, stderr}`. It runs the program **directly**, not through a shell. Its `stdin`, if any, is the command's standard input.

```yaml
jobs:
  titles:
    steps:
      show:
        get: "https://httpbin.org/json"

      # Process an HTTP body with jq. A JSON value as stdin is written as compact JSON,
      # and jq's JSON output is parsed, so `steps.titles.stdout` is an array.
      titles:
        cmd: [jq, -c, '[.slideshow.slides[].title]']
        stdin: ${steps.show.body}

      # Need a pipe? Pass values to `sh -c` as positional arguments ($1, $2, …)
      # instead of writing ${...} inside the script. `_` fills $0.
      slug:
        cmd: [sh, -c, 'printf "%s" "$1" | tr " " -', _, "${steps.show.body.slideshow.title}"]

      # A non-zero exit doesn't have to stop the job.
      config:
        cmd: [test, -f, config.toml]
        allow_failure: true
      fallback:
        if: { this: steps.config.exit_code, ne: 0 }
        then:
          note:
            cmd: [echo, "config.toml not found, using defaults"]
```

Example: the `mixed` job in [`examples/httpbin.yaml`](../examples/httpbin.yaml).

## Fields

| Field | Notes |
|---|---|
| `cmd` | A non-empty list: the program, then its arguments. A single string such as `cmd: "ls -la"` is rejected; write `[ls, -la]`. |

The common step field `allow_failure` works as for HTTP steps. See [Projects](projects.md#step-fields).

## Arguments and substitution

- `${...}` is substituted inside each list item. A value always stays **one** argument, however many spaces or quotes it contains, so it can't inject extra arguments or shell syntax.
- Strings are inserted as-is. Numbers, objects and arrays are inserted as compact JSON.
- For pipes, redirects or globbing, call a shell yourself: `cmd: [sh, -c, "ls | wc -l"]`. Don't put `${...}` inside the `sh -c` script, because the shell would interpret the value. Pass it as a positional argument, as in the `slug` example above.
- Variables are looked up in earlier results (loop variables, `steps` and `states`), then the project `env_file`, then the environment. See [Variables](variables.md#lookup-order).

## `stdin`

- `${...}` is substituted in it. A string is written as-is; any other value (an object, an array, or a field that is exactly `${NAME}` holding one) is written as compact JSON, so `stdin: ${steps.fetch.body}` is ready for `jq`.
- Without `stdin`, the command's standard input is empty (`/dev/null`), so it never waits for keyboard input.
- A command may exit without reading all of its input; that isn't an error.

## Result

| Field | Value |
|---|---|
| `exit_code` | The exit code, or `-1` if the process was killed by a signal. |
| `stdout` | Parsed as JSON if the whole output is valid JSON (`echo 42` gives the number `42`). Otherwise text with trailing newlines (`\n`, `\r`) removed. `null` with `capture_stdout: false`. |
| `stderr` | Always text, with trailing newlines removed. |

Invalid UTF-8 in `stdout` or `stderr` is replaced with `�`.

A non-zero exit code fails the step unless `allow_failure: true`. A command that times out with `allow_failure` has no exit code or output; its result is just `` {"error": "command `x` timed out after 5s"} ``. See [Projects](projects.md#failures-and-allow_failure).

## Options

```yaml
build:
  cmd: [make, release]
  timeout: 300            # seconds
  cwd: ../app             # relative to the file this step is in
  stdin: ${steps.notes.stdout}
  env:
    TOKEN: ${API_TOKEN}
```

| Field | Default | Meaning |
|---|---|---|
| `timeout` | none | Seconds before the command is killed. The step then fails with ``command `make` timed out after 300s``. Processes the command started itself (e.g. under `sh -c`) are killed too. |
| `cwd` | your current directory | Directory to run in. `${...}` is substituted first; a relative result is resolved against the directory of the file the step is written in. A missing directory fails the step with `` `cwd` <path> is not a directory ``, even with `allow_failure`. |
| `stdin` | empty | Written to the command's standard input, like `cmd < file`. See [`stdin`](#stdin). |
| `env` | none | Map of name → value, added to the environment the command inherits (replacing a variable of the same name). `${...}` is substituted; a string is used as-is and any other value is written as compact JSON, as for `stdin`. A name can't appear twice. |
| `capture_stdout` | `true` | `false` sends stdout to `/dev/null` instead of keeping it in memory, and the result's `stdout` is `null`. Use it for commands with large output you don't need. `stderr` and `exit_code` are still kept. |

Example of `env`: [`examples/cmd-env.yaml`](../examples/cmd-env.yaml).

`cwd`, `stdin`, `env` and `capture_stdout` only work on cmd steps; on any other step they are an error. `timeout` also works on [HTTP](http-steps.md) steps.

## Reading and writing files

There is no separate file step; use a command. Add `cwd: .` so the path is relative to the file the step is written in, not your current directory.

```yaml
save:
  cmd: [sh, -c, 'cat > "$1"', _, "out/${DATE}.json"]     # the path is $1, never inside the script
  stdin: ${steps.fetch.body}                             # written as compact JSON
  cwd: .

load:
  cmd: [cat, "out/${DATE}.json"]                         # valid JSON stdout is parsed, so later
  cwd: .                                                 # steps can read ${steps.load.stdout.slideshow.title}
```

- A missing file makes `cat` exit non-zero, which fails the step unless `allow_failure: true`.
- `cat > "$1"` replaces an existing file; use `cat >> "$1"` to append. The parent directory must already exist.

## Environment

- Without `cwd`, commands run in **your current directory**, not the project file's directory.
- They inherit the environment of `rocky`. Env file values are **not** exported to them; list the ones a command needs under [`env`](#options), or pass them as arguments or `stdin`.
- Without `timeout`, a command that never exits blocks the job until `rocky` is stopped (Ctrl-C, SIGTERM, SIGHUP or SIGQUIT), which kills it and everything it started.
- A program that can't be found or started fails the step with ``failed to start command `x` ``, even with `allow_failure`.
