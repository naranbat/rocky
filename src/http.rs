use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::time::Duration;

use reqwest::header::{CONTENT_TYPE, HeaderName, HeaderValue};
use reqwest::redirect::Policy;
use reqwest::{Client, Method, Request};
use serde_json::Value;

use crate::error::Error;
use crate::request::RequestSpec;
use crate::response::{Response, body_value, headers_map};

const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// HTTP clients, one per TLS and redirect setting, built on first use so later
/// requests reuse their connections.
#[derive(Default)]
pub struct Clients(HashMap<(bool, bool), Client>);

impl Clients {
    fn get(&mut self, spec: &RequestSpec) -> Result<&Client, Error> {
        match self.0.entry((spec.insecure, spec.follow_redirects)) {
            Entry::Occupied(entry) => Ok(entry.into_mut()),
            Entry::Vacant(entry) => {
                let mut client = Client::builder().danger_accept_invalid_certs(spec.insecure);
                if !spec.follow_redirects {
                    client = client.redirect(Policy::none());
                }
                Ok(entry.insert(client.build()?))
            }
        }
    }
}

/// Builds the request on `client` without sending it. `timeout` is in seconds and defaults to 30.
pub fn build(client: &Client, spec: &RequestSpec, timeout: Option<u64>) -> Result<Request, Error> {
    let method = parse_method(&spec.method)?;

    let mut builder = client
        .request(method, &spec.url)
        .timeout(Duration::from_secs(timeout_secs(timeout)));
    if !spec.query.is_empty() {
        builder = builder.query(&spec.query);
    }
    for (name, value) in &spec.headers {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| Error::InvalidHeader(name.clone()))?;
        let header_value =
            HeaderValue::from_str(value).map_err(|_| Error::InvalidHeader(name.clone()))?;
        builder = builder.header(header_name, header_value);
    }
    if let Some(json) = &spec.json {
        let has_content_type = spec
            .headers
            .keys()
            .any(|name| name.eq_ignore_ascii_case(CONTENT_TYPE.as_str()));
        if !has_content_type {
            builder = builder.header(CONTENT_TYPE, "application/json");
        }
        builder = builder.body(json.to_string());
    }
    Ok(builder.build()?)
}

pub async fn send(
    clients: &mut Clients,
    spec: &RequestSpec,
    timeout: Option<u64>,
    capture_body: bool,
) -> Result<Response, Error> {
    let client = clients.get(spec)?;
    let request = build(client, spec, timeout)?;
    let map_err = |err: reqwest::Error| {
        if err.is_timeout() {
            Error::Timeout(timeout_secs(timeout))
        } else {
            Error::Http(err)
        }
    };

    let mut response = client.execute(request).await.map_err(map_err)?;
    let status = response.status().as_u16();
    let headers = headers_map(response.headers());
    if !capture_body {
        // Read to the end, so the timeout covers the transfer and the connection is reused.
        while response.chunk().await.map_err(map_err)?.is_some() {}
        return Ok(Response {
            status,
            headers,
            body: Value::Null,
            body_encoding: None,
        });
    }
    let bytes = response.bytes().await.map_err(map_err)?;
    let content_type = headers.get(CONTENT_TYPE.as_str()).map(String::as_str);
    let (body, body_encoding) = body_value(content_type, &bytes);

    Ok(Response {
        status,
        headers,
        body,
        body_encoding,
    })
}

fn timeout_secs(timeout: Option<u64>) -> u64 {
    timeout.unwrap_or(DEFAULT_TIMEOUT_SECS)
}

/// Accepts standard methods in any case; rejects everything else.
fn parse_method(raw: &str) -> Result<Method, Error> {
    Ok(match raw.to_ascii_uppercase().as_str() {
        "GET" => Method::GET,
        "POST" => Method::POST,
        "PUT" => Method::PUT,
        "PATCH" => Method::PATCH,
        "DELETE" => Method::DELETE,
        "HEAD" => Method::HEAD,
        "OPTIONS" => Method::OPTIONS,
        "TRACE" => Method::TRACE,
        "CONNECT" => Method::CONNECT,
        _ => return Err(Error::InvalidMethod(raw.to_string())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{body_json, body_string, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn get_sends_query_and_headers() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users"))
            .and(query_param("page", "2"))
            .and(header("authorization", "Bearer abc"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-request-id", "r1")
                    .set_body_json(json!({"users": []})),
            )
            .mount(&server)
            .await;

        let mut req = RequestSpec::new("GET", &format!("{}/users", server.uri()));
        req.query.insert("page".into(), "2".into());
        req.headers
            .insert("Authorization".into(), "Bearer abc".into());

        let res = send(&mut Clients::default(), &req, None, true)
            .await
            .unwrap();
        assert_eq!(res.status, 200);
        assert_eq!(res.headers["x-request-id"], "r1");
        assert_eq!(res.body, json!({"users": []}));
    }

    #[tokio::test]
    async fn post_sends_json_body_with_content_type() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(header("content-type", "application/json"))
            .and(body_json(json!({"name": "alice"})))
            .respond_with(ResponseTemplate::new(201))
            .mount(&server)
            .await;

        let mut req = RequestSpec::new("POST", &server.uri());
        req.json = Some(json!({"name": "alice"}));

        let res = send(&mut Clients::default(), &req, None, true)
            .await
            .unwrap();
        assert_eq!(res.status, 201);
        assert_eq!(res.body, json!(""));
    }

    #[tokio::test]
    async fn user_content_type_is_kept() {
        let server = MockServer::start().await;
        Mock::given(header("content-type", "application/vnd.api+json"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let mut req = RequestSpec::new("POST", &server.uri());
        req.headers
            .insert("Content-Type".into(), "application/vnd.api+json".into());
        req.json = Some(json!({}));

        assert_eq!(
            send(&mut Clients::default(), &req, None, true)
                .await
                .unwrap()
                .status,
            200
        );
    }

    #[tokio::test]
    async fn string_json_is_sent_as_json_string_whatever_the_content_type() {
        let server = MockServer::start().await;
        Mock::given(body_string("\"hello\""))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let mut req = RequestSpec::new("POST", &server.uri());
        req.headers
            .insert("Content-Type".into(), "text/plain".into());
        req.json = Some(json!("hello"));

        assert_eq!(
            send(&mut Clients::default(), &req, None, true)
                .await
                .unwrap()
                .status,
            200
        );
    }

    #[tokio::test]
    async fn non_json_body_and_error_status_are_returned() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
            .mount(&server)
            .await;

        let res = send(
            &mut Clients::default(),
            &RequestSpec::new("GET", &server.uri()),
            None,
            true,
        )
        .await
        .unwrap();
        assert_eq!(res.status, 404);
        assert_eq!(res.body, json!("not found"));
    }

    #[tokio::test]
    async fn uncaptured_body_is_null() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-request-id", "r1")
                    .set_body_string("big"),
            )
            .mount(&server)
            .await;

        let req = RequestSpec::new("GET", &server.uri());
        let res = send(&mut Clients::default(), &req, None, false)
            .await
            .unwrap();
        assert_eq!(res.status, 200);
        assert_eq!(res.headers["x-request-id"], "r1");
        assert_eq!(res.body, Value::Null);
        assert_eq!(res.body_encoding, None);
    }

    #[test]
    fn lowercase_method_is_normalized() {
        let request = build(
            &Client::new(),
            &RequestSpec::new("patch", "http://localhost"),
            None,
        )
        .unwrap();
        assert_eq!(request.method(), Method::PATCH);
    }

    #[test]
    fn unknown_method_errors() {
        for raw in ["BAD METHOD", "FETCH"] {
            let err = build(
                &Client::new(),
                &RequestSpec::new(raw, "http://localhost"),
                None,
            )
            .unwrap_err();
            assert!(matches!(err, Error::InvalidMethod(_)), "{raw}");
        }
    }

    #[tokio::test]
    async fn slow_response_times_out() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(3)))
            .mount(&server)
            .await;

        let req = RequestSpec::new("GET", &server.uri());
        let err = send(&mut Clients::default(), &req, Some(1), true)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Timeout(1)));
    }

    #[tokio::test]
    async fn redirects_not_followed_when_disabled() {
        let server = MockServer::start().await;
        Mock::given(path("/old"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", "/new"))
            .mount(&server)
            .await;
        Mock::given(path("/new"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let mut clients = Clients::default();
        let mut req = RequestSpec::new("GET", &format!("{}/old", server.uri()));
        assert_eq!(
            send(&mut clients, &req, None, true).await.unwrap().status,
            200
        );

        req.follow_redirects = false;
        let res = send(&mut clients, &req, None, true).await.unwrap();
        assert_eq!(res.status, 302);
        assert_eq!(res.headers["location"], "/new");
    }

    #[tokio::test]
    async fn clients_are_reused_per_setting() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let mut clients = Clients::default();
        let mut req = RequestSpec::new("GET", &server.uri());
        send(&mut clients, &req, None, true).await.unwrap();
        send(&mut clients, &req, Some(5), true).await.unwrap();
        assert_eq!(clients.0.len(), 1);

        req.insecure = true;
        send(&mut clients, &req, None, true).await.unwrap();
        assert_eq!(clients.0.len(), 2);
    }
}
