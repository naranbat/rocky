# Debugging

`exec` prints only the result of the last step that ran, on stdout. It doesn't show the steps before it; `run` logs each step as it finishes.

## `run` log lines

`run` writes one JSON line on **stdout** per step as it finishes, plus a line per branch or switch decision, loop and `set`, and a line when the run ends. `exec` doesn't write them.

```
$ rocky run project.yaml | jq -c '{event, step, arm, ok}'
{"event":"step","step":"stats","arm":null,"ok":true}
{"event":"branch","step":"route","arm":"then","ok":null}
{"event":"step","step":"big","arm":null,"ok":true}
{"event":"done","step":null,"arm":null,"ok":null}
```

| `event` | Meaning |
|---|---|
| `step` | an HTTP or cmd step finished. `ok: false` with `allowed: true` means it failed with `allow_failure` and the job continued |
| `branch` | a branch chose `then` or `else` |
| `switch` | a switch chose a case or `default` |
| `for_each` | a loop started |
| `set` | a `set` step set state; logged just before the state file is written |
| `done` | the run finished; `failed_step` names the step that stopped it and `reason` says why, or both are `null` |
| `error` | the run stopped with an error |
| `stopped` | a job's schedule stopped; `error` says why |

A step that ends in an error (an undefined variable, a network error) gets no `step` line; the `error` line names it. With `allow_failure`, a timeout or network error instead gives a `step` line with `ok: false` and `error`. All fields are listed in [Run](run.md#output).

## No dry run

There is no mode that resolves requests without sending them. Every HTTP request and command really executes. To try a job safely, point it at a test server such as https://httpbin.org.
