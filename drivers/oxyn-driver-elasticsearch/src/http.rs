//! The thin, bounded HTTP layer every request goes through.
//!
//! One client per session, built once: no proxy, no redirect, TLS verified
//! unless the connection is Local and asked otherwise (`crate::options`).
//! Every response is read in chunks and capped at [`RESPONSE_BYTE_LIMIT`]
//! **before** anything decodes it: `reqwest`'s `json()` would collect a body
//! of any size first ([I-06](../../../CLAUDE.md#i-06)).

use std::time::Duration;

use oxyn_core::{OxynError, Result, StatementIntent};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use reqwest::{Client, Method, Response};
use serde_json::Value;
use url::Url;

use crate::error::{response_error, transport_error};
use crate::options::{ConnectSpec, TlsMode};

/// The most a single response may weigh before decoding: 16 MiB.
///
/// A search page of a few hundred ordinary documents fits several times; a
/// page that does not is refused with a message that says how to narrow it
/// (`size`, `_source` filtering), rather than decoded into several times its
/// size in memory. Pages of a continued search are each bounded by it.
pub(crate) const RESPONSE_BYTE_LIMIT: usize = 16 << 20;

/// How long establishing the TCP and TLS connection may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The header the Tasks API reports, which finds a request's task to cancel.
pub(crate) const OPAQUE_ID: &str = "x-opaque-id";

/// Percent-encodes a name Oxyn puts in a path — an index, a task, an id —
/// so that a name holding `/`, `?`, `#` or `%` stays one segment
/// ([I-10](../../../CLAUDE.md#i-10)).
pub(crate) fn encode_segment(segment: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            for nibble in [byte >> 4, byte & 0x0f] {
                encoded.push(char::from(
                    HEX.get(usize::from(nibble)).copied().unwrap_or(b'0'),
                ));
            }
        }
    }
    encoded
}

/// HTTP state for one session. It has no `Debug`: it holds the
/// `Authorization` header.
pub(crate) struct Http {
    base: Url,
    client: Client,
    authorization: Option<HeaderValue>,
}

/// A request body, as the server's content type names it.
pub(crate) enum Body {
    /// One JSON document, the user's text byte for byte.
    Json(String),
    /// Newline-delimited JSON, for `_bulk` and `_msearch`.
    Ndjson(String),
}

/// One request.
pub(crate) struct Call<'a> {
    pub(crate) method: Method,
    /// An API path, absolute on the cluster: `/logs/_search?q=x`.
    pub(crate) path: &'a str,
    pub(crate) body: Option<Body>,
    /// Marks the request so the Tasks API can find it.
    pub(crate) opaque_id: Option<&'a str>,
    pub(crate) timeout: Option<Duration>,
    /// What the request does, which decides how a lost answer is classified.
    pub(crate) intent: StatementIntent,
}

impl<'a> Call<'a> {
    /// A read Oxyn composes itself: introspection, tasks, pings.
    pub(crate) fn read(method: Method, path: &'a str) -> Self {
        Self {
            method,
            path,
            body: None,
            opaque_id: None,
            timeout: None,
            intent: StatementIntent::Read,
        }
    }

    pub(crate) fn with_json(mut self, body: &Value) -> Self {
        self.body = Some(Body::Json(body.to_string()));
        self
    }
}

/// A successful response, read whole and bounded.
pub(crate) struct Reply {
    pub(crate) status: u16,
    /// Whether the server said the body is JSON (`application/json`, and the
    /// vendor variants of the compatibility headers).
    pub(crate) json: bool,
    pub(crate) bytes: Vec<u8>,
}

impl Reply {
    /// The body as JSON.
    pub(crate) fn value(&self) -> Result<Value> {
        serde_json::from_slice(&self.bytes).map_err(|_| {
            OxynError::Query("the server's answer is not the JSON it announced".to_owned())
        })
    }
}

impl Http {
    /// The client of a connection.
    ///
    /// `no_proxy`: a proxy chosen by the system would carry the credentials
    /// and the user's data through a host nobody configured for this
    /// connection. No redirect: `reqwest` would follow it with the
    /// `Authorization` header to wherever it points.
    pub(crate) fn new(spec: ConnectSpec) -> Result<Self> {
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .tcp_keepalive(Duration::from_secs(30))
            .danger_accept_invalid_certs(spec.tls == Some(TlsMode::Require))
            .https_only(spec.tls.is_some())
            .build()
            .map_err(|_| OxynError::Config("the HTTP client cannot be built".to_owned()))?;
        Ok(Self {
            base: spec.base,
            client,
            authorization: spec.authorization,
        })
    }

    /// Sends a request and reads its answer, bounded.
    ///
    /// A `HEAD` answers with its status whatever it is: `404` is how
    /// `HEAD /logs` says the index does not exist.
    ///
    /// # Errors
    /// The server's refusal, classified from its status and `error.type`; a
    /// lost exchange, ambiguous when the request may have changed something
    /// (I-13); an answer above [`RESPONSE_BYTE_LIMIT`].
    pub(crate) async fn send(&self, call: Call<'_>) -> Result<Reply> {
        let head = call.method == Method::HEAD;
        self.exchange(call, head).await
    }

    /// Sends a request and returns its answer whatever its status: the
    /// catalog tells an API the server does not have (`404`, `400`) from a
    /// refusal it must report.
    ///
    /// # Errors
    /// A lost exchange; an answer above [`RESPONSE_BYTE_LIMIT`].
    pub(crate) async fn send_any_status(&self, call: Call<'_>) -> Result<Reply> {
        self.exchange(call, true).await
    }

    async fn exchange(&self, call: Call<'_>, any_status: bool) -> Result<Reply> {
        let url = self.url(call.path)?;
        let mut request = self.client.request(call.method, url);
        if let Some(authorization) = &self.authorization {
            request = request.header(AUTHORIZATION, authorization.clone());
        }
        if let Some(opaque) = call.opaque_id {
            request = request.header(OPAQUE_ID, opaque);
        }
        request = match call.body {
            Some(Body::Json(text)) => request.header(CONTENT_TYPE, "application/json").body(text),
            Some(Body::Ndjson(text)) => request
                .header(CONTENT_TYPE, "application/x-ndjson")
                .body(text),
            None => request,
        };
        if let Some(timeout) = call.timeout {
            request = request.timeout(timeout);
        }
        let response = request.send().await.map_err(|error| {
            // A refused connection never carried the request: nothing can
            // have been applied, whatever the request was.
            if error.is_connect() {
                transport_error(StatementIntent::Read, "the cluster cannot be reached")
            } else if error.is_timeout() {
                transport_error(call.intent, "the cluster did not answer in time")
            } else {
                transport_error(call.intent, "the HTTP exchange with the cluster failed")
            }
        })?;
        let status = response.status().as_u16();
        let json = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains("json"));
        let success = response.status().is_success() || any_status;
        let bytes = read_bounded(response, call.intent, success).await?;
        if !success {
            return Err(response_error(status, &bytes, call.intent));
        }
        Ok(Reply {
            status,
            json,
            bytes,
        })
    }

    /// A JSON read Oxyn composes itself.
    pub(crate) async fn read_json(&self, path: &str) -> Result<Value> {
        self.send(Call::read(Method::GET, path)).await?.value()
    }

    /// The URL of an API path on the configured origin.
    ///
    /// Joined as a relative path, so that a proxy prefix of the configured
    /// URL stays and a first segment like `remote:logs` (cross-cluster
    /// search) is not read as a URL scheme. A path that would still leave
    /// the origin is refused: the credentials go only where they were
    /// configured.
    fn url(&self, path: &str) -> Result<Url> {
        let relative = format!("./{}", path.trim_start_matches('/'));
        let url = self
            .base
            .join(&relative)
            .map_err(|_| OxynError::Config("the API path is not a valid URL path".to_owned()))?;
        if url.origin() != self.base.origin() {
            return Err(OxynError::Config(
                "the API path leaves the configured cluster".to_owned(),
            ));
        }
        Ok(url)
    }
}

/// Reads a body, never past [`RESPONSE_BYTE_LIMIT`].
///
/// `success` says the request was applied: a write whose answer is too large
/// to read is reported as such, never as failed — re-running it would apply
/// it twice (I-13).
async fn read_bounded(
    mut response: Response,
    intent: StatementIntent,
    success: bool,
) -> Result<Vec<u8>> {
    let too_large = || {
        if success && intent.is_mutating() {
            OxynError::OutcomeUnknown(
                "the cluster answered success, but its answer exceeds 16 MiB and was not read: \
                 per-item results are unknown; check the data before running it again"
                    .to_owned(),
            )
        } else {
            OxynError::Query(
                "the answer exceeds the 16 MiB a single response may take: lower `size`, \
                 filter `_source`, or page with a point in time"
                    .to_owned(),
            )
        }
    };
    if response
        .content_length()
        .is_some_and(|length| length > RESPONSE_BYTE_LIMIT as u64)
    {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    loop {
        let chunk = response
            .chunk()
            .await
            .map_err(|_| transport_error(intent, "the answer was cut off mid-stream"))?;
        let Some(chunk) = chunk else {
            return Ok(bytes);
        };
        if chunk.len() > RESPONSE_BYTE_LIMIT.saturating_sub(bytes.len()) {
            return Err(too_large());
        }
        bytes.extend_from_slice(&chunk);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use oxyn_core::{ConnectionConfig, DriverId, Environment};
    use oxyn_driver::Credentials;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    use super::*;
    use crate::driver::elasticsearch_metadata;

    /// A loopback server that answers every connection with `response`, raw.
    pub(crate) async fn canned(response: Vec<u8>) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a loopback port is free");
        let port = listener.local_addr().expect("an address").port();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut buffer = [0_u8; 4096];
                let mut read = Vec::new();
                while !read.windows(4).any(|window| window == b"\r\n\r\n") {
                    match socket.read(&mut buffer).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => read.extend_from_slice(buffer.get(..n).unwrap_or_default()),
                    }
                }
                let _ = socket.write_all(&response).await;
                let _ = socket.shutdown().await;
            }
        });
        port
    }

    pub(crate) fn http_to(port: u16) -> Http {
        let config = ConnectionConfig::new("test", DriverId::elasticsearch())
            .with_environment(Environment::Local)
            .with_param("url", format!("http://127.0.0.1:{port}"));
        let spec =
            ConnectSpec::from_config(&elasticsearch_metadata(), &config, &Credentials::new())
                .expect("a valid configuration");
        Http::new(spec).expect("a client")
    }

    fn answer(status: &str, body: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
             connection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    #[tokio::test]
    async fn a_path_cannot_leave_the_configured_origin() {
        let http = http_to(9);
        assert!(
            http.url("//elsewhere.example/_search")
                .is_ok_and(|url| { url.host_str() == Some("127.0.0.1") })
        );
        let cross_cluster = http.url("/remote:logs/_search").expect("a relative path");
        assert_eq!(cross_cluster.path(), "/remote:logs/_search");
    }

    #[tokio::test]
    async fn a_refusal_carries_the_servers_type_and_reason() {
        let port = canned(answer(
            "404 Not Found",
            r#"{"error":{"type":"index_not_found_exception","reason":"no such index [logs]"},"status":404}"#,
        ))
        .await;
        let error = http_to(port)
            .read_json("/logs/_mapping")
            .await
            .expect_err("refused");
        let text = error.to_string();
        assert!(text.contains("index_not_found_exception"), "{text}");
        assert!(text.contains("no such index [logs]"), "{text}");
        assert_eq!(error.class(), oxyn_core::ErrorClass::Permanent);
    }

    #[tokio::test]
    async fn a_head_answers_its_status_rather_than_failing() {
        let port = canned(
            b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_vec(),
        )
        .await;
        let reply = http_to(port)
            .send(Call::read(Method::HEAD, "/logs"))
            .await
            .expect("an answer");
        assert_eq!(reply.status, 404);
    }

    #[tokio::test]
    async fn an_answer_cut_off_mid_stream_is_a_transport_failure_never_a_result() {
        // Announces 1000 bytes, sends 20, closes.
        let port = canned(
            b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 1000\r\n\r\n{\"hits\":{\"hits\":[".to_vec(),
        )
        .await;
        let http = http_to(port);
        let read = Call::read(Method::GET, "/_search");
        let error = http.send(read).await.err().expect("cut off");
        assert_eq!(error.class(), oxyn_core::ErrorClass::Transient);
        let write = Call {
            intent: StatementIntent::Write,
            ..Call::read(Method::POST, "/_bulk")
        };
        let error = http.send(write).await.err().expect("cut off");
        assert_eq!(error.class(), oxyn_core::ErrorClass::Ambiguous);
    }

    #[tokio::test]
    async fn an_announced_oversized_answer_is_refused_before_it_is_read() {
        let port = canned(
            format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n",
                RESPONSE_BYTE_LIMIT + 1
            )
            .into_bytes(),
        )
        .await;
        let http = http_to(port);
        let read = http.send(Call::read(Method::GET, "/_search")).await;
        assert!(matches!(read, Err(OxynError::Query(_))));
        let write = Call {
            intent: StatementIntent::Write,
            ..Call::read(Method::POST, "/_bulk")
        };
        assert!(matches!(
            http.send(write).await,
            Err(OxynError::OutcomeUnknown(_))
        ));
    }
}
