# State

A job can keep named values from one run to the next: a cursor of the last item it processed, a counter, a timestamp. Declare them with defaults under the job's `states`, read them as `${states.NAME}`, and change them with a `set` task.

```yaml
jobs:
  counter:
    states:
      COUNT: 0                             # the default, used on the first run
    tasks:
      inc:
        cmd: [expr, '${states.COUNT}', +, '1']
      save:
        set:
          states:
            COUNT: ${tasks.inc.stdout}     # kept for the next run
      report:
        cmd: [echo, 'run number ${states.COUNT}']
```

```sh
rocky exec examples/state.yaml counter   # stdout: run number 1
rocky exec examples/state.yaml counter   # stdout: run number 2
```

Example: [`examples/state.yaml`](../examples/state.yaml).

## The `states` field

`states` goes on a job (inline or in a job file), not on the project or a task. It is a map of name → default value, and a default can be any YAML value: a string, number, boolean, list or map. Names follow the [variable name rule](env.md#names).

## Reading state

`states.NAME` works wherever a variable does: `${states.COUNT}` in `${...}`, and `states.COUNT` (without `${}`) in an `if` condition, a `switch` or a `for_each`. Values keep their JSON type, so `json: {after: "${states.LAST_ID}"}` sends a number if the state holds one. Dot paths reach inside: `${states.SEEN.0}`. See [Env](env.md).

## `set`

A `set` task's `states` sets one or more state names. Each value has `${...}` substituted, and a value that is exactly `${NAME}` keeps its JSON type (see [typed values](env.md#typed-values)).

```yaml
remember:
  set:
    states:
      LAST_ID: ${tasks.fetch.body.items.0.id}   # a number stays a number
      UPDATED: 'at ${tasks.today.stdout}'       # text
```

- Every value is worked out first, then all are set together, so `${states.X}` inside a `set` reads the value from before it.
- The new values are written to the state file right away. A later task that fails doesn't undo them.
- Tasks after it, including later in the same run, see the new values.
- It can go anywhere a task can: inside a branch, switch or loop, and in a task file.
- No other key may go with it. It has no result: it isn't saved under `tasks`, and it doesn't count as the last task that ran.
- Every name under `set.states` must be declared in the job's `states`. This is checked when the project loads.

## The state file

Each job's state is stored in `states/<job>.json` next to the project file, e.g. `states/counter.json`:

```json
{
  "COUNT": 2
}
```

- **First run:** if the file doesn't exist, it is created (with the `states` directory) from the defaults.
- **Later runs:** each declared name takes its value from the file. A name added to `states` since the last run takes its default; a name removed from `states` is dropped from the file at the next `set` of `states`.
- A job without `states` never creates a file.
- **One file per job:** two jobs can use the same state name; each keeps its own value and can't read the other's. Renaming a job starts it from its defaults and leaves the old file behind.
- To start over, delete the file. To change a value by hand, edit it; it must stay a JSON object.
- A file that isn't a JSON object stops the run: ``invalid state file examples/states/counter.json`` (the path is next to the project).

## `states` is reserved

`states` is a variable like `tasks`: a loop variable named `states` (in `as`) fails when the project loads.
