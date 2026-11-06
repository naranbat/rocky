# Task files

Every job already lives in its own file in `jobs/` (see [Projects](projects.md#job-files)). A task can live in its own YAML file too, so several jobs can share it. All files are loaded and checked when the project loads, and again each time `run` runs a job. `rocky new task` writes a starter task file (see [`rocky new`](cli.md#rocky-new-kind)).

```yaml
# jobs/report.yaml
tasks:
  signin: ../tasks/login.yaml    # a task file, named by its key; relative to this file
  today:
    cmd: [date]
```

```yaml
# tasks/login.yaml: one task of any kind, including a loop; no `name`
cmd: [curl, -sS, --fail-with-body, https://httpbin.org/anything/login]
```

- A task file holds one task, without a `name`: the key that points at it names it, so one file can be used under several names. It can use other task files, inside a loop too, but not itself, directly or through other files.
- A task file entry is a path, or `{file: path, with: {...}, when: ...}` to give the file [inputs of its own](#reusing-a-task-file). Everything else goes in the file. To change another field for one use, write the task inline.
- Keep task files outside `jobs/` (for example in `tasks/`): every `.yaml` file in `jobs/` is a job.
- Paths are relative to the file they are written in; see [Paths](projects.md#paths).

Example: [`examples/httpbin/`](../examples/httpbin/), whose jobs `files` and `report` use [`tasks/login.yaml`](../examples/httpbin/tasks/login.yaml).

## Reusing a task file

A task entry can name its file under `file`, with `with` inputs that the file reads as `${NAME}`, so one file can be used with different inputs:

```yaml
tasks:
  alice: {file: ../tasks/greet.yaml, with: {WHO: alice}}
  bob:
    file: ../tasks/greet.yaml
    with:
      WHO: bob
  anyone: ../tasks/greet.yaml       # WHO from the job's or project's values
```

```yaml
# tasks/greet.yaml
cmd: [echo, 'hello ${WHO}']
```

- Only `file`, `with` and `when` are allowed; everything else goes in the file. `with` only works next to `file`. If the file has its own `when` too, both must hold; the entry's is checked first, and the file's can read the entry's `with` inputs.
- The inputs apply while that task runs, including tasks nested in it, and win over the job's and the project's values (see [lookup order](variables.md#lookup-order)). Afterwards the earlier values are back.
- Names are uppercase. `${...}` in an input is expanded when the task starts, so it can pass a loop element or an earlier task's output into the file, e.g. `with: {USER: '${ITEM}'}` or `with: {TOKEN: '${tasks.login.stdout.token}'}`. A value that is exactly `${NAME}` keeps its JSON type, `$${` is a literal `${`, and an undefined variable fails the task. Quote `${...}` inside a flow map (`{...}`); see [Writing YAML](yaml.md#quoting).
- A task file can itself be a `{file, with}` entry; the outer entry's inputs win over the inner one's.
