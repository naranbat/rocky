# Debugging

## Check the project

`rocky validate` loads the project in the current directory, its `rocky.yaml` and job files, and runs every [load-time check](projects.md#validation) without running a command. Some problems only show up when a job runs: a missing `env_file`, an undefined variable.

## See what `exec` runs

`exec` prints only the result of the last task that ran, on stdout. It doesn't show the tasks before it. `--verbose` (or `-v`) prints each command to stderr just before it runs, with `${VAR}`s filled in (secrets included):

```
$ rocky exec login --verbose > result.json
+ [login] curl -s -H 'Authorization: Bearer abc123' https://example.com/login | jq .token
```

See [`rocky exec`](cli.md#rocky-exec-job).

## Read `run`'s log lines

`run` writes one JSON line on **stdout** per task as it finishes, plus a line per skipped task, loop, `while` check and `trigger` task, and a `done` (or `error`) line when the run ends. `exec` doesn't write them. They are easy to filter with `jq`:

```
$ rocky run | jq -c '{event, task, ok}'
{"event":"task","task":"stats","ok":true}
{"event":"task","task":"big","ok":true}
{"event":"skip","task":"warn","ok":null}
{"event":"done","task":null,"ok":null}
```

The same lines are in `.rocky/logs/`. Every event and field is listed in [Run](run.md#output).

## No dry run

There is no mode that resolves tasks without running them. Every command, including every `curl` request, really executes. To try a job safely, point it at a test server such as https://httpbin.org.
