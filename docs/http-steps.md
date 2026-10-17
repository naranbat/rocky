# HTTP steps

An HTTP step sends one request and produces a [response](responses.md) `{status, headers, body}`. Write the request under its method, `get:`, `post:`, `put:`, `patch:` or `delete:` (see [Method keys](#method-keys)), or under `http:` with a `method`; its `json` is sent as the JSON body. To reuse a request, put the whole step in a [step file](projects.md#job-and-step-files).

```yaml
vars:
  BASE: https://httpbin.org
jobs:
  users:
    steps:
      list:
        get:
          url: ${BASE}/get
          query:
            page: "2"

      # A request that is just a URL fits on one line.
      ping:
        get: "${BASE}/get"
        timeout: 5

      # Expect a 404 and handle it instead of stopping the job.
      lookup:
        get: ${BASE}/status/404
        allow_failure: true
      missing:
        if: { this: steps.lookup.status, eq: 404 }
        then:
          create:
            post:
              url: ${BASE}/anything/users
              query:
                source: ${steps.list.body.args.page}   # query values are percent-encoded
              json:
                name: alice
                seen: ${steps.ping.body.headers}       # exactly ${...}: sent as a JSON object
```

## Fields

| Field | Type | Default | Notes |
|---|---|---|---|
| `method` | string | required, except with a [method key](#method-keys) | `GET`, `POST`, `PUT`, `PATCH`, `DELETE`, `HEAD`, `OPTIONS`, `TRACE` or `CONNECT`, in any case (`get` → `GET`). Anything else fails when the step runs (not at load) with ``invalid HTTP method `X` ``. |
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
  get: ${BASE}/get                    # http: { method: GET, url: ${BASE}/get }
create:
  post:                               # http: { method: POST, url: ..., json: ... }
    url: ${BASE}/anything/users
    json: { name: alice }
```

A step has only one of `http`, `get`, `post`, `put`, `patch` and `delete`. A method key can't also have `method`, and `http` needs one. Other methods (`HEAD`, `OPTIONS`, …) use `http`.

## Where `${NAME}` works

`url`, header **values**, `query` **values**, and any string inside `json`. It is not expanded in header names, query keys or `json` keys. See [Variables](variables.md).

## Behaviour

- **Variables** are expanded before sending. The step's own `vars` come before those of its enclosing steps, the job and the project. See [Variables](variables.md#vars-levels).
- **Failure:** a 4xx or 5xx status fails the step. Use `allow_failure: true` to continue. See [Projects](projects.md#failures-and-allow_failure).
- **Result:** saved as `steps.<name>` (the step's key), read as `${steps.<name>.status}`, `${steps.<name>.headers.<lowercase-name>}` and `${steps.<name>.body...}`.
- **Content type:** `Content-Type: application/json` is added when there is a `json`, unless you set a `Content-Type` header yourself (the name is matched in any case).
- **Encoding:** `query` values are percent-encoded, but the `url` is used as written. A variable in `url` that contains `&`, `#` or `?` changes the URL, so put such values under `query:`.
- **Bodies are JSON only** (`json`). There is no form or multipart support yet.
- **Independent requests:** each step has its own `insecure` and `follow_redirects`, and no cookies are shared between steps. Connections are reused within a run by steps with the same `insecure` and `follow_redirects`. Pass cookies yourself: `Cookie: ${steps.login.headers.set-cookie}`.
- **`timeout` is set on the step,** not the request (default 30s): `{get: ..., timeout: 5}`. Exceeding it fails with `request timed out after 5s`.
- **Network errors** (connection refused, DNS failure, TLS errors) fail the step with `request failed`, followed by the cause. Unless `allow_failure` is set, they stop the job.
