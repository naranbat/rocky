use std::collections::BTreeMap;
use std::fmt;

use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};

/// An HTTP request: an `http` step's fields.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RequestSpec {
    /// Required in `http`; set from the key in `get`, `post`, `put`, `patch` and `delete`.
    /// Its presence is checked when the project loads; its value when the request is sent.
    #[serde(default)]
    pub method: String,
    pub url: String,
    #[serde(default, deserialize_with = "crate::project::unique")]
    pub headers: BTreeMap<String, String>,
    #[serde(default, deserialize_with = "crate::project::unique")]
    pub query: BTreeMap<String, String>,
    pub json: Option<serde_json::Value>,
    /// Skip TLS certificate verification.
    #[serde(default)]
    pub insecure: bool,
    #[serde(default = "default_true")]
    pub follow_redirects: bool,
}

fn default_true() -> bool {
    true
}

/// A `get`, `post`, `put`, `patch` or `delete` step's value: the URL, or the `http` fields
/// (its method comes from the key).
#[derive(Clone, Debug, PartialEq)]
pub struct Shorthand(pub RequestSpec);

impl<'de> Deserialize<'de> for Shorthand {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ShorthandVisitor;

        impl<'de> Visitor<'de> for ShorthandVisitor {
            type Value = Shorthand;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a URL or a map of request fields")
            }

            fn visit_str<E: de::Error>(self, url: &str) -> Result<Shorthand, E> {
                let fields = de::value::MapDeserializer::<_, E>::new(std::iter::once(("url", url)));
                RequestSpec::deserialize(fields).map(Shorthand)
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Shorthand, A::Error> {
                RequestSpec::deserialize(de::value::MapAccessDeserializer::new(map)).map(Shorthand)
            }
        }

        deserializer.deserialize_any(ShorthandVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_str(yaml: &str) -> Result<RequestSpec, serde_yaml::Error> {
        serde_yaml::from_str(yaml)
    }

    #[test]
    fn loads_full_spec() {
        let spec = load_str(
            "method: POST\n\
             url: https://example.com/users\n\
             headers:\n  Authorization: Bearer x\n\
             query:\n  verbose: \"true\"\n\
             json:\n  name: alice\n  tags: [a, b]\n",
        )
        .unwrap();
        assert_eq!(spec.method, "POST");
        assert_eq!(spec.url, "https://example.com/users");
        assert_eq!(spec.headers["Authorization"], "Bearer x");
        assert_eq!(spec.query["verbose"], "true");
        assert_eq!(
            spec.json,
            Some(serde_json::json!({"name": "alice", "tags": ["a", "b"]}))
        );
    }

    #[test]
    fn optional_fields_default_to_empty() {
        let spec = load_str("method: GET\nurl: https://example.com\n").unwrap();
        assert!(spec.headers.is_empty());
        assert!(spec.query.is_empty());
        assert_eq!(spec.json, None);
        assert!(!spec.insecure);
        assert!(spec.follow_redirects);
    }

    #[test]
    fn loads_client_options() {
        let spec =
            load_str("method: GET\nurl: x\ninsecure: true\nfollow_redirects: false\n").unwrap();
        assert!(spec.insecure);
        assert!(!spec.follow_redirects);
    }

    #[test]
    fn shorthand_is_a_url_or_fields() {
        let Shorthand(spec) = serde_yaml::from_str("https://example.com").unwrap();
        assert_eq!(spec.url, "https://example.com");
        assert_eq!(spec.method, "");
        assert!(spec.follow_redirects);

        let Shorthand(spec) =
            serde_yaml::from_str("{url: x, json: {a: 1}, follow_redirects: false}").unwrap();
        assert_eq!(spec.url, "x");
        assert_eq!(spec.json, Some(serde_json::json!({"a": 1})));
        assert!(!spec.follow_redirects);

        assert!(serde_yaml::from_str::<Shorthand>("[x]").is_err());
    }

    #[test]
    fn rejects_unknown_fields_and_missing_url() {
        for (yaml, expected) in [
            (
                "method: GET\nurl: x\ntimeout: 5\n",
                "unknown field `timeout`",
            ),
            (
                "vars:\n  BASE: https://example.com\nmethod: GET\nurl: ${BASE}\n",
                "unknown field `vars`",
            ),
            (
                "method: GET\nurl: https://example.com\nheader: {}\n",
                "unknown field `header`",
            ),
            ("method: GET\n", "missing field `url`"),
        ] {
            let err = load_str(yaml).unwrap_err();
            assert!(err.to_string().contains(expected), "{yaml}: {err}");
        }
    }
}
