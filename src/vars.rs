use serde_json::Value;

use crate::error::Error;
use crate::extract;
use crate::request::RequestSpec;

/// Replaces every `${VAR}` in the spec's url, header values, query values,
/// and `json` string leaves, looking `VAR` up with `lookup`.
pub(crate) fn apply_with(
    mut spec: RequestSpec,
    lookup: &dyn Fn(&str) -> Option<Value>,
) -> Result<RequestSpec, Error> {
    spec.url = substitute(&spec.url, lookup)?;
    for value in spec.headers.values_mut() {
        *value = substitute(value, lookup)?;
    }
    for value in spec.query.values_mut() {
        *value = substitute(value, lookup)?;
    }
    if let Some(json) = spec.json.as_mut() {
        substitute_value(json, lookup)?;
    }
    Ok(spec)
}

pub(crate) fn substitute_value(
    value: &mut Value,
    lookup: &dyn Fn(&str) -> Option<Value>,
) -> Result<(), Error> {
    match value {
        // A string that is exactly `${NAME}` takes the looked-up value with its JSON type.
        Value::String(s) => match whole_var(s) {
            Some(name) => {
                *value = lookup(name).ok_or_else(|| Error::UndefinedVar(name.to_string()))?;
            }
            None => *s = substitute(s, lookup)?,
        },
        Value::Array(items) => {
            for item in items {
                substitute_value(item, lookup)?;
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                substitute_value(item, lookup)?;
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
    Ok(())
}

fn whole_var(s: &str) -> Option<&str> {
    let name = s.strip_prefix("${")?.strip_suffix('}')?;
    (!name.contains('}') && !name.contains("${")).then_some(name)
}

/// Expands `${NAME}`; `$${` is an escape for a literal `${`.
pub(crate) fn substitute(
    input: &str,
    lookup: &dyn Fn(&str) -> Option<Value>,
) -> Result<String, Error> {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('$') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        if let Some(after) = tail.strip_prefix("$${") {
            out.push_str("${");
            rest = after;
        } else if let Some(after) = tail.strip_prefix("${") {
            let end = after
                .find('}')
                .ok_or_else(|| Error::UnclosedVar(input.to_string()))?;
            let name = &after[..end];
            let value = lookup(name).ok_or_else(|| Error::UndefinedVar(name.to_string()))?;
            out.push_str(&extract::text(&value));
            rest = &after[end + 1..];
        } else {
            out.push('$');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn lookup(name: &str) -> Option<Value> {
        match name {
            "TOKEN" => Some(json!("abc")),
            "ID" => Some(json!("42")),
            "COUNT" => Some(json!(5)),
            "TAGS" => Some(json!(["a"])),
            _ => None,
        }
    }

    fn spec(url: &str) -> RequestSpec {
        RequestSpec {
            method: "GET".to_string(),
            url: url.to_string(),
            headers: BTreeMap::new(),
            query: BTreeMap::new(),
            json: None,
            insecure: false,
            follow_redirects: true,
        }
    }

    #[test]
    fn double_dollar_escapes_literal() {
        assert_eq!(
            substitute("$${X} $${ID}${ID} $${open", &lookup).unwrap(),
            "${X} ${ID}42 ${open"
        );
    }

    #[test]
    fn escaped_whole_body_string_stays_literal() {
        let mut spec = spec("x");
        spec.json = Some(json!({"tpl": "$${COUNT}"}));
        let spec = apply_with(spec, &lookup).unwrap();
        assert_eq!(spec.json, Some(json!({"tpl": "${COUNT}"})));
    }

    #[test]
    fn substitutes_multiple_vars() {
        assert_eq!(
            substitute("/u/${ID}?t=${TOKEN}", &lookup).unwrap(),
            "/u/42?t=abc"
        );
    }

    #[test]
    fn leaves_plain_text_alone() {
        assert_eq!(
            substitute("no vars $ here {}", &lookup).unwrap(),
            "no vars $ here {}"
        );
    }

    #[test]
    fn non_string_values_render_as_json_text() {
        assert_eq!(
            substitute("/items/${COUNT}?t=${TAGS}", &lookup).unwrap(),
            r#"/items/5?t=["a"]"#
        );
    }

    #[test]
    fn missing_var_errors() {
        let err = substitute("${NOPE}", &lookup).unwrap_err();
        assert!(matches!(err, Error::UndefinedVar(name) if name == "NOPE"));
    }

    #[test]
    fn unclosed_var_errors() {
        let err = substitute("${TOKEN", &lookup).unwrap_err();
        assert!(matches!(err, Error::UnclosedVar(_)));
    }

    #[test]
    fn applies_to_all_spec_fields() {
        let mut spec = spec("https://example.com/u/${ID}");
        spec.headers = [("Authorization".to_string(), "Bearer ${TOKEN}".to_string())].into();
        spec.query = [("id".to_string(), "${ID}".to_string())].into();
        spec.json = Some(json!({"user": {"id": "${ID}", "tags": ["${TOKEN}", 1, true]}}));
        let spec = apply_with(spec, &lookup).unwrap();
        assert_eq!(spec.url, "https://example.com/u/42");
        assert_eq!(spec.headers["Authorization"], "Bearer abc");
        assert_eq!(spec.query["id"], "42");
        assert_eq!(
            spec.json,
            Some(json!({"user": {"id": "42", "tags": ["abc", 1, true]}}))
        );
    }

    #[test]
    fn whole_body_var_keeps_json_type() {
        let mut spec = spec("x");
        spec.json = Some(json!({
            "count": "${COUNT}",
            "tags": "${TAGS}",
            "label": "n=${COUNT}",
            "two": "${COUNT}${COUNT}",
        }));
        let spec = apply_with(spec, &lookup).unwrap();
        assert_eq!(
            spec.json,
            Some(json!({"count": 5, "tags": ["a"], "label": "n=5", "two": "55"}))
        );
    }

    #[test]
    fn whole_body_var_undefined_errors() {
        let mut spec = spec("x");
        spec.json = Some(json!({"a": "${NOPE}"}));
        let err = apply_with(spec, &lookup).unwrap_err();
        assert!(matches!(err, Error::UndefinedVar(name) if name == "NOPE"));
    }
}
