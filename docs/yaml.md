# Writing YAML

Rocky project, job and task files use YAML. `rocky validate <PROJECT>` checks both YAML syntax and Rocky's fields without running commands.

## Editor schema

The repository includes [`schema/rocky.schema.json`](../schema/rocky.schema.json), which provides completion and early validation in editors that support JSON Schema for YAML. For the YAML Language Server, put this comment at the top of a project file and adjust the relative path as needed:

```yaml
# yaml-language-server: $schema=./schema/rocky.schema.json
jobs:
  example:
    tasks:
      hello:
        cmd: [echo, hello]
```

The comment is ignored by Rocky. Job and task files are validated when their project loads; the published schema describes complete project files.

## Quoting

Block mappings are the safest form for values containing `${...}`:

```yaml
env:
  TOKEN: ${API_TOKEN}
```

Inside a flow collection (`{...}` or `[...]`), quote `${...}` so its braces are not read as YAML structure:

```yaml
env: {TOKEN: '${API_TOKEN}'}
```

Quote a string when it:

- is a numeric command argument, such as `"5"`
- looks like a number, boolean, date or time but must remain text
- contains `#`, `: `, `{`, `}`, `[`, `]`, `*`, `&`, `!` or other YAML punctuation
- starts or ends with spaces

Single quotes are usually simplest. To include a single quote inside one, double it: `'can''t'`.

## Command arguments are strings

`cmd` and every `pipe` stage are non-empty lists of strings. Quote arguments that YAML would otherwise parse as another type:

```yaml
tasks:
  wait:
    cmd: [sleep, "5"]
```

This is invalid because `5` is a YAML number, not a string:

```yaml
tasks:
  wait:
    cmd: [sleep, 5]
```

## Typed values

Outside `cmd` and `pipe`, YAML types are useful. Project and job `values` and a cmd task's `stdin` may contain strings, numbers, booleans, lists, objects and `null`:

```yaml
values:
  RETRIES: 3
  ENABLED: true
  TAGS: [admin, beta]
```

A field that is exactly `${NAME}` keeps the looked-up JSON type where typed substitution is supported; surrounding text makes it a string. See [Typed values](env.md#typed-values).

## Durations and times

Durations are strings made from a whole number and one unit: `s`, `m` or `h`. Write `30s`, `5m`, `2h` or `90m`; integer seconds, fractions and combined units are not accepted.

Quote wall-clock times and dates:

```yaml
schedule:
  daily: "09:30"
```

## Migrating older project files

Current Rocky syntax intentionally rejects three older forms:

```yaml
# Write duration strings, not integer seconds.
timeout: 30s

# Keep weekly fields together.
schedule:
  weekly:
    days: [mon, fri]
    time: "09:30"

# Pass runtime inputs to a task file with `with`.
tasks:
  greet:
    file: tasks/greet.yaml
    with:
      WHO: alice
```

Replace `timeout: 30` or retry `delay: 2` with `30s` or `2s`; replace sibling `weekly` and `time` fields with the nested form; and replace `values` next to a task `file` with `with`. Project, job and job-file `values` do not change.

## Validate before running

```sh
rocky validate project.yaml
```

Validation catches unknown fields, wrong YAML types, invalid conditions and schedules, bad task combinations, missing imported files and duplicate names. It does not run any command.
