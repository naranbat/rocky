# HTTP tasks

An HTTP task sends one request and produces a [response](responses.md) `{status, headers, body}`. Write the request under its method, `get:`, `post:`, `put:`, `patch:` or `delete:` (see [Method keys](#method-keys)), or under `http:` with a `method`; its `json` is sent as the JSON body. To reuse a request, put the whole task in a [task file](projects.md#job-and-task-files).

```yaml
jobs:
  users:
    tasks:
      list:
        get:
          url: https://httpbin.org/get
          query:
            page: "2"

      # A request that is just a URL fits on one line.
      ping:
        get: "https://httpbin.org/get"
        timeout: 5

      # Expect a 404 and handle it instead of stopping the job.
      lookup:
        get: https://httpbin.org/status/404
        allow_failure: true
      missing:
        if: { this: tasks.lookup.status, eq: 404 }
        then:
          create:
            post:
              url: https://httpbin.org/anything/users
              query:
                source: ${tasks.list.body.args.page}   # query values are percent-encoded
              json:
                name: alice
                seen: ${tasks.ping.body.headers}       # exactly ${...}: sent as a JSON object
```

## Fields

| Field | Type | Default | Notes |
|---|---|---|---|
| `method` | string | required, except with a [method key](#method-keys) | `GET`, `POST`, `PUT`, `PATCH`, `DELETE`, `HEAD`, `OPTIONS`, `TRACE` or `CONNECT`, in any case (`get` → `GET`). Anything else fails when the task runs (not at load) with ``invalid HTTP method `X` ``. |
| `url` | string | required | Must include the scheme (`https://…`). |
| `headers` | map of string → string | none | An invalid name or value fails with ``invalid header `X` ``. A name can't appear twice; names differing only in case are both sent, under the same name. |
| `query` | map of string → string | none | Appended to the URL and percent-encoded. A key can't appear twice. |
| `json` | any YAML value | none | Sent as the JSON body. Key order is preserved. A string is sent as a JSON string, with quotes, whatever the `Content-Type`. |
| `insecure` | bool | `false` | Skip TLS certificate checks, e.g. for self-signed internal servers. |
| `follow_redirects` | bool | `true` | Set to `false` to get the 3xx response itself. |

Unknown fields are an error, so typos fail loudly.

## Method keys

`get`, `post`, `put`, `patch` and `delete` are shorthands for `http` with that `method`. The value is the URL, or the other `http` fields:

```yaml
ping:
  get: https://httpbin.org/get                    # http: { method: GET, url: https://httpbin.org/get }
create:
  post:                               # http: { method: POST, url: ..., json: ... }
    url: https://httpbin.org/anything/users
    json: { name: alice }
```

A task has only one of `http`, `get`, `post`, `put`, `patch` and `delete`. A method key can't also have `method`, and `http` needs one. Other methods (`HEAD`, `OPTIONS`, …) use `http`.

## Where `${NAME}` works

`url`, header **values**, `query` **values**, and any string inside `json`. It is not expanded in header names, query keys or `json` keys. See [Env](env.md).

## Behaviour

- **Variables** are expanded before sending. See [Env](env.md#lookup-order).
- **Failure:** a 4xx or 5xx status fails the task. Use `allow_failure: true` to continue. See [Projects](projects.md#failures-and-allow_failure).
- **Result:** saved as `tasks.<name>` (the task's key), read as `${tasks.<name>.status}`, `${tasks.<name>.headers.<lowercase-name>}` and `${tasks.<name>.body...}`.
- **Content type:** `Content-Type: application/json` is added when there is a `json`, unless you set a `Content-Type` header yourself (the name is matched in any case).
- **Encoding:** `query` values are percent-encoded, but the `url` is used as written. A variable in `url` that contains `&`, `#` or `?` changes the URL, so put such values under `query:`.
- **Bodies are JSON only** (`json`). There is no form or multipart support yet.
- **Independent requests:** each task has its own `insecure` and `follow_redirects`, and no cookies are shared between tasks. Connections are reused within a run by tasks with the same `insecure` and `follow_redirects`. Pass cookies yourself: `Cookie: ${tasks.login.headers.set-cookie}`.
- **`timeout` is set on the task,** not the request (default 30s): `{get: ..., timeout: 5}`. Exceeding it fails with `request timed out after 5s`.
- **`capture_body: false`** on the task reads the body without keeping it in memory, and the result's `body` is `null`. Use it for large responses you don't need. `status` and `headers` are still kept.
- **Network errors** (connection refused, DNS failure, TLS errors) fail the task with `request failed`, followed by the cause. Unless `allow_failure` is set, they stop the job.
