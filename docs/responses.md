# Responses

Every HTTP task produces the same JSON shape. It is what `rocky exec` prints when the task is last, and what `${tasks.NAME...}` reads. The exception is a timeout or network error with `allow_failure`, whose result is just `{"error": "..."}`; see [Projects](projects.md#failures-and-allow_failure).

```json
{
  "status": 200,
  "headers": { "content-type": "application/json" },
  "body": { "id": 1, "name": "alice" }
}
```

| Field | Value |
|---|---|
| `status` | The numeric HTTP status. |
| `headers` | Header names are **lowercase**. A header sent more than once has its values joined with `", "`. |
| `body` | See below. |
| `body_encoding` | Present only as `"base64"` when the body is binary. |

## Body

- If the response `Content-Type` is JSON (`application/json` or any `*+json`, such as `application/problem+json`) **and** the body parses as JSON, it is embedded as JSON. Key order and number precision are preserved exactly (`1.10` stays `1.10`, large integers are not rounded).
- Any other content type, or a JSON content type with invalid JSON, is embedded as a string.
- If the body isn't valid UTF-8 (images and other binary data), it is base64-encoded and the result includes `"body_encoding": "base64"`.

## Reading parts of it

```sh
rocky exec project.yaml report | jq .status
rocky exec project.yaml report | jq '.headers["x-request-id"]'
rocky exec project.yaml report | jq .body
```

Inside a job, use `${tasks.NAME.status}`, `${tasks.NAME.headers.x-request-id}` or `${tasks.NAME.body.user.id}`. See [Projects](projects.md#using-earlier-tasks-output).

## Limitations

- Joining repeated headers with `", "` mangles multiple `set-cookie` values.
- Header and body keys that contain `.` can't be addressed with a dot path.
