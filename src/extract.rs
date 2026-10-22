use serde_json::{Map, Value};

/// Looks up a variable by exact name, or by dot path when the name contains `.`:
/// `tasks.login.body.token` selects `login.body.token` from the `tasks` variable.
pub fn lookup(values: &Map<String, Value>, name: &str) -> Option<Value> {
    if let Some(value) = values.get(name) {
        return Some(value.clone());
    }
    let (root, path) = name.split_once('.')?;
    select(values.get(root)?, path)
}

/// Selects a value by dot path (`body.items.0.id`). Numeric segments index into arrays.
pub(crate) fn select(value: &Value, path: &str) -> Option<Value> {
    path.split('.')
        .try_fold(value, |current, segment| match current {
            Value::Object(map) => map.get(segment),
            Value::Array(items) => segment.parse::<usize>().ok().and_then(|i| items.get(i)),
            _ => None,
        })
        .cloned()
}

/// Strings as-is; anything else as compact JSON text.
pub fn text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn envelope() -> Value {
        json!({
            "status": 200,
            "headers": {"content-type": "application/json"},
            "body": {"token": "abc", "items": [{"id": 7}]},
        })
    }

    #[test]
    fn selects_status_header_and_nested_body() {
        let env = envelope();
        assert_eq!(select(&env, "status"), Some(json!(200)));
        assert_eq!(
            select(&env, "headers.content-type"),
            Some(json!("application/json"))
        );
        assert_eq!(select(&env, "body.token"), Some(json!("abc")));
        assert_eq!(select(&env, "body.items.0.id"), Some(json!(7)));
    }

    #[test]
    fn selects_whole_body() {
        let env = envelope();
        assert_eq!(select(&env, "body"), Some(env["body"].clone()));
    }

    #[test]
    fn lookup_prefers_exact_name_then_dot_path() {
        let values = json!({"A": 1, "a.b": "exact", "tasks": {"login": envelope()}});
        let values = values.as_object().unwrap();
        assert_eq!(lookup(values, "A"), Some(json!(1)));
        assert_eq!(lookup(values, "a.b"), Some(json!("exact")));
        assert_eq!(lookup(values, "tasks.login.body.token"), Some(json!("abc")));
        assert_eq!(lookup(values, "tasks.login.status"), Some(json!(200)));
        assert_eq!(lookup(values, "tasks.other.status"), None);
        assert_eq!(lookup(values, "NOPE"), None);
    }

    #[test]
    fn missing_path_is_none() {
        let env = envelope();
        assert_eq!(select(&env, "body.nope"), None);
        assert_eq!(select(&env, "body.items.5"), None);
        assert_eq!(select(&env, "body.items.x"), None);
        assert_eq!(select(&env, "status.deeper"), None);
        assert_eq!(select(&env, ""), None);
    }
}
