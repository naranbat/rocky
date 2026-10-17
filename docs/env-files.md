# Env files

An `env_file` loads variables from a dotenv file. Use it to keep secrets such as API tokens out of the YAML that you commit.

```sh
# example.env
API_TOKEN=example-token
export REGION="eu-west-1"   # `export` and quotes are allowed
```

```yaml
env_file: example.env        # relative to the project file
jobs:
  user:
    steps:
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

Examples: [`examples/vars-env.yaml`](../examples/vars-env.yaml) and [`examples/cmd-env.yaml`](../examples/cmd-env.yaml) with [`examples/example.env`](../examples/example.env).

## Where it can go

Only at the top level of a project, with a path relative to the project file. It applies to every step of every job. Job files, step files and HTTP requests can't have one: `env_file` there is an unknown field.

See [Variables](variables.md#lookup-order) for how it ranks against other sources. In short: step, job and project `vars` all beat the project `env_file`.

## Format

- One `KEY=value` per line. Lines starting with `#` are comments.
- Values may be single- or double-quoted.
- A leading `export ` is ignored.
- Every value is a string.
- Unquoted and double-quoted values expand `${NAME}`. Always use the braces: a bare `$NAME` stops at the first character that isn't a letter or digit, so `$API_TOKEN` reads `$API` followed by `_TOKEN`, and inside double quotes a bare `$NAME` just before the closing quote fails to parse. Names come from the process environment, then earlier lines of the file, so an exported variable wins over an earlier line. An undefined name becomes empty. Single-quote a value that contains a literal `$`, such as a password: `PASSWORD='pa$$word'`.

## Behaviour

- The file only provides values for `${...}`. It never changes the environment of the `rocky` process or of the commands that [cmd steps](cmd-steps.md) run; pass a value to a command with the step's `env`.
- The file is read when each run starts.
- A missing or malformed file fails the run with `failed to load env file <path>`. This is checked at run time, not when the project loads.
- Values are never masked. A cmd step that echoes a token, or a response that returns one, shows it in full when `exec` prints it as the result.
