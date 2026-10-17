# Debugging

`exec` prints only the result of the last task that ran, on stdout. It doesn't show the tasks before it; `run` logs each task as it finishes.

## `run` log lines

`run` writes one JSON line on **stdout** per task as it finishes, plus a line per branch or switch decision and loop, and a line when the run ends. `exec` doesn't write them.

```
$ rocky run project.yaml | jq -c '{event, task, arm, ok}'
{"event":"task","task":"stats","arm":null,"ok":true}
{"event":"branch","task":"route","arm":"then","ok":null}
{"event":"task","task":"big","arm":null,"ok":true}
{"event":"done","task":null,"arm":null,"ok":null}
```

| `event` | Meaning |
|---|---|
| `task` | an HTTP or cmd task finished. `ok: false` with `allowed: true` means it failed with `allow_failure` and the job continued |
| `branch` | a branch chose `then` or `else` |
| `switch` | a switch chose a case or `default` |
| `for_each` | a loop started |
| `done` | the run finished; `failed_task` names the task that stopped it and `reason` says why, or both are `null` |
| `error` | the run stopped with an error |
| `stopped` | a job's schedule stopped; `error` says why |

A task that ends in an error (an undefined variable, a network error) gets no `task` line; the `error` line names it. With `allow_failure`, a timeout or network error instead gives a `task` line with `ok: false` and `error`. All fields are listed in [Run](run.md#output).

## No dry run

There is no mode that resolves requests without sending them. Every HTTP request and command really executes. To try a job safely, point it at a test server such as https://httpbin.org.
