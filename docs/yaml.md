# Writing YAML

Rocky's `rocky.yaml` and job files use YAML. `rocky validate` checks both YAML syntax and Rocky's fields without running commands: unknown fields, wrong YAML types, invalid conditions and schedules, bad task combinations and duplicate names. See [Validation](projects.md#validation).

## Editor schema

The repository includes [`schema/job.schema.json`](../schema/job.schema.json) for job files and [`schema/rocky.schema.json`](../schema/rocky.schema.json) for `rocky.yaml`, which provide completion and early validation in editors that support JSON Schema for YAML. For the YAML Language Server, put this comment at the top of a job file and adjust the path as needed:

```yaml
# yaml-language-server: $schema=../schema/job.schema.json
tasks:
  hello:
    cmd: [echo, hello]
```

The comment is ignored by Rocky.

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

A field that is exactly `${NAME}` keeps the looked-up JSON type where typed substitution is supported; surrounding text makes it a string. See [Typed values](variables.md#typed-values).

## Durations and times

Durations (`timeout`, `retry.delay`, a schedule's `every` and `delay`) are strings made from a whole number and one unit: `s`, `m` or `h`. Write `30s`, `5m`, `2h` or `90m`. Integer seconds (`30`), fractions (`1.5m`), combined units (`1h30m`), milliseconds and other units (`1d`) are not accepted; write `90m` or `24h` instead.

Quote wall-clock times and dates:

```yaml
schedule:
  daily: "09:30"
```

## Removed syntax

Current Rocky syntax rejects these older forms:

| Old form | Write instead |
|---|---|
| `timeout: 30` or retry `delay: 2` (integer seconds) | `30s`, `2s` |
| `weekly` and `time` as sibling fields of `schedule` | `weekly: {days: [mon, fri], time: "09:30"}` |
| A task kept in its own file (`name: path.yaml`, or `file` with `with`) | the task written inline in the job; `file` and `with` fail as unknown fields |
| `if`/`then`/`else` or `switch`/`cases`/`default` on a task | [`when`](conditions.md) on each task it should gate; the old keys fail as ``invalid YAML in <file>`` with ``unknown field `if` `` (or `switch`) |
| `when` on a job | [`when`](conditions.md) on each task it should gate; fails as ``invalid YAML in <file>`` with ``unknown field `when` `` |
| `max` on a `while` loop | `attempts`; `max` fails with `` `max` is now `attempts` `` |

```yaml
timeout: 30s

schedule:
  weekly:
    days: [mon, fri]
    time: "09:30"
```
