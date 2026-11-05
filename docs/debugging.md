# Debugging

`exec` prints only the result of the last task that ran, on stdout. It doesn't show the tasks before it; `run` logs each task as it finishes.

## `run` log lines

`run` writes one JSON line on **stdout** per task as it finishes, plus a line per skipped task and loop, and a line when the run ends. `exec` doesn't write them.

```
$ rocky run project.yaml | jq -c '{event, task, ok}'
{"event":"task","task":"stats","ok":true}
{"event":"task","task":"big","ok":true}
{"event":"skip","task":"warn","ok":null}
{"event":"done","task":null,"ok":null}
```

| `event` | Meaning |
|---|---|
| `task` | a cmd task finished, or one run of a task with [`retry`](cmd-tasks.md#retry) did. `ok: false` with `allowed: true` means it failed with `allow_failure`, or inside a loop with `on_failure: next`, and the job continued |
| `skip` | a task's [`when`](conditions.md) didn't hold, so it didn't run; without `task`, the whole job was skipped |
| `for_each` | a loop started |
| `done` | the run finished; `failed_task` names the task that stopped it and `reason` says why, or both are `null` |
| `error` | the run stopped with an error |
| `stopped` | a job's schedule stopped; `error` says why |

A task that can't start or finish as a command result (for example, an undefined variable or a missing program) gets no `task` line; the `error` line names it. A program-reported failure, such as `curl` exiting non-zero after a DNS or connection error, does get a `task` line with `ok: false` and its `exit_code`. With `allow_failure`, the job continues after that result; an allowed timeout instead has an `error` field. All fields are listed in [Run](run.md#output).

## No dry run

There is no mode that resolves tasks without running them. Every command, including every `curl` request, really executes. To try a job safely, point it at a test server such as https://httpbin.org.
