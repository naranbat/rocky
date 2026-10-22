use std::collections::BTreeMap;

use base64::Engine;
use reqwest::header::HeaderMap;
use serde::Serialize;
use serde_json::Value;

/// An HTTP response with its parts kept separate for independent use.
#[derive(Debug, Serialize, PartialEq)]
pub struct Response {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Value,
    /// `"base64"` when the body was not valid UTF-8.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_encoding: Option<&'static str>,
}

/// Lowercase header names to values; repeated headers are joined with `", "`.
pub fn headers_map(headers: &HeaderMap) -> BTreeMap<String, String> {
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    for (name, value) in headers {
        let value = String::from_utf8_lossy(value.as_bytes()).into_owned();
        map.entry(name.as_str().to_string())
            .and_modify(|existing| {
                existing.push_str(", ");
                existing.push_str(&value);
            })
            .or_insert(value);
    }
    map
}

/// Embeds a raw body: parsed JSON when the content type is JSON and it parses,
/// otherwise UTF-8 text, otherwise base64 (with encoding `"base64"`).
pub fn body_value(content_type: Option<&str>, bytes: &[u8]) -> (Value, Option<&'static str>) {
    if content_type.is_some_and(is_json)
        && let Ok(value) = serde_json::from_slice(bytes)
    {
        return (value, None);
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => (Value::String(text.to_string()), None),
        Err(_) => (
            Value::String(base64::engine::general_purpose::STANDARD.encode(bytes)),
            Some("base64"),
        ),
    }
}

pub(crate) fn is_json(content_type: &str) -> bool {
    let media_type = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    media_type == "application/json" || media_type.ends_with("+json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const JSON: Option<&str> = Some("application/json; charset=utf-8");

    #[test]
    fn repeated_headers_are_joined() {
        let mut headers = HeaderMap::new();
        headers.append("set-cookie", "a".parse().unwrap());
        headers.append("set-cookie", "b".parse().unwrap());
        assert_eq!(headers_map(&headers)["set-cookie"], "a, b");
    }

    #[test]
    fn json_body_is_embedded_as_json() {
        assert_eq!(body_value(JSON, br#"{"id":1}"#), (json!({"id": 1}), None));
    }

    #[test]
    fn json_suffix_content_type_is_parsed() {
        let (value, _) = body_value(Some("application/problem+json"), br#"{"a":1}"#);
        assert_eq!(value, json!({"a": 1}));
    }

    #[test]
    fn json_key_order_and_big_numbers_are_preserved() {
        let raw = r#"{"z":1,"a":12345678901234567890123,"m":1.10}"#;
        let (value, _) = body_value(JSON, raw.as_bytes());
        assert_eq!(
            value.to_string(),
            r#"{"z":1,"a":12345678901234567890123,"m":1.10}"#
        );
    }

    #[test]
    fn invalid_json_with_json_content_type_is_text() {
        assert_eq!(body_value(JSON, b"oops").0, json!("oops"));
    }

    #[test]
    fn text_content_type_is_never_parsed() {
        assert_eq!(body_value(Some("text/plain"), b"42").0, json!("42"));
        assert_eq!(body_value(None, b"true").0, json!("true"));
    }

    #[test]
    fn empty_body_is_empty_string() {
        assert_eq!(body_value(JSON, b""), (json!(""), None));
    }

    #[test]
    fn binary_body_is_base64() {
        assert_eq!(
            body_value(Some("image/png"), &[0xff, 0x00, 0xfe]),
            (json!("/wD+"), Some("base64"))
        );
    }
}
