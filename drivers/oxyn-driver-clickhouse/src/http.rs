//! One statement, one HTTP request.
//!
//! ClickHouse's HTTP interface is stateless when no `session_id` is sent, and
//! this driver sends none (ADR-0060): every request carries everything that
//! decides its meaning — the database, the output format, the read-only
//! bound, a `query_id` the driver chose. Nothing a previous request did can
//! change what the next one means, which is what the driver contract asks of
//! session state.
//!
//! The password travels in the `X-ClickHouse-Key` header, marked sensitive so
//! that `reqwest`'s own `Debug` does not print it, and never in the URL, which
//! proxies and the server's logs keep ([I-03](../../../CLAUDE.md#i-03)).

use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{Client, Response, StatusCode};

use crate::error::{MAX_MESSAGE_BYTES, ServerError};
use crate::options::{ConnectSpec, TlsMode};
use crate::params::Bound;

/// The output format of every statement: names and types announced first,
/// then rows ([`crate::types`]).
pub const FORMAT: &str = "RowBinaryWithNamesAndTypes";

/// What an answer that ClickHouse did not give is reported as. Fixed: a
/// login page's text has nothing to tell a professional.
const NOT_THE_SERVER: &str = "the answer did not come from the ClickHouse server for this \
                              statement: a redirect or a proxy stood in between, and the \
                              driver follows neither";

/// How long opening the TCP and TLS connection may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// A request the driver sends.
///
/// `Debug` is written by hand: the user's bound values never reach it, as
/// [`oxyn_core::ExecRequest`] keeps its own out ([I-03](../../../CLAUDE.md#i-03)).
#[derive(Clone, Default)]
pub struct Call {
    /// The statement, as the user or the driver wrote it.
    pub text: String,
    /// The id `KILL QUERY` targets.
    pub query_id: String,
    /// The database unqualified names resolve in.
    pub database: Option<String>,
    /// Server settings for this request only.
    pub settings: Vec<(&'static str, String)>,
    /// The driver's own query parameters, bound by the server to
    /// `{name:Type}` placeholders.
    pub params: Vec<(&'static str, String)>,
    /// The values the user bound, by placeholder name; `None` is `NULL`.
    pub bound: Vec<Bound>,
}

impl std::fmt::Debug for Call {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Call")
            .field("query_id", &self.query_id)
            .field("database", &self.database)
            .field("settings", &self.settings)
            .field("bound", &self.bound.len())
            .finish_non_exhaustive()
    }
}

/// What the response's headers say, before its body is read.
#[derive(Debug)]
pub struct Reply {
    /// The format the server answers in; absent when no result set follows.
    pub format: Option<String>,
    /// The zone of the query, for instants that declare none.
    pub zone: Option<String>,
    /// The marker an exception raised mid-body carries.
    pub exception_tag: Option<String>,
    /// Rows written, from the summary the headers carry when they are sent.
    pub written_rows: Option<u64>,
    /// The body.
    pub response: Response,
}

/// A ClickHouse HTTP endpoint, with the account that talks to it.
#[derive(Debug, Clone)]
pub struct Http {
    client: Client,
    spec: ConnectSpec,
}

impl Http {
    /// The client of a connection.
    ///
    /// `no_proxy`: a proxy chosen by the system would carry the password
    /// header and the user's data through a host nobody configured for this
    /// connection. TLS is `rustls` on `aws-lc-rs` with the system's trust
    /// store, the workspace's provider (RESEARCH-NOTES).
    ///
    /// # Errors
    /// [`oxyn_core::OxynError::Config`] if the TLS stack cannot be built.
    pub fn new(spec: ConnectSpec) -> oxyn_core::Result<Self> {
        // No redirect: `reqwest` would follow it with the password header,
        // which it does not know to strip, and turn the POST into a GET.
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .tcp_keepalive(Duration::from_secs(30))
            .danger_accept_invalid_certs(spec.tls() == TlsMode::Require)
            .https_only(spec.tls() != TlsMode::Disable)
            .build()
            .map_err(|_| {
                oxyn_core::OxynError::Config("the HTTP client cannot be built".to_owned())
            })?;
        Ok(Self { client, spec })
    }

    /// The connection parameters.
    #[must_use]
    pub const fn spec(&self) -> &ConnectSpec {
        &self.spec
    }

    /// Sends a statement and returns once the server has answered its
    /// headers.
    ///
    /// A status other than 200 is read here, bounded, and returned as the
    /// server's error: the body is then the exception's text.
    ///
    /// # Errors
    /// The transport error, untouched for [`crate::error::map_transport_error`]
    /// to classify; or the server's refusal.
    pub async fn send(&self, call: &Call) -> Result<Reply, Failure> {
        let mut url = self.spec.base().clone();
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("query_id", &call.query_id);
            query.append_pair("default_format", FORMAT);
            if let Some(database) = &call.database {
                query.append_pair("database", database);
            }
            for (name, value) in &call.settings {
                query.append_pair(name, value);
            }
            for (name, value) in &call.params {
                query.append_pair(&format!("param_{name}"), &escape_param(value));
            }
            for (name, value) in &call.bound {
                // `\N` is the server's NULL; any other text is escaped, so a
                // value spelled `\N` stays that text.
                let value = value
                    .as_deref()
                    .map_or_else(|| r"\N".to_owned(), escape_param);
                query.append_pair(&format!("param_{name}"), &value);
            }
        }
        let response = self
            .client
            .post(url)
            .headers(self.credentials())
            .body(call.text.clone())
            .send()
            .await
            // `reqwest`'s error carries the URL, and the URL the bound values.
            .map_err(|error| Failure::Transport(error.without_url()))?;

        let headers = response.headers();
        let text = |name: &str| {
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        let written_rows = text("x-clickhouse-summary").and_then(|summary| written_rows(&summary));
        // A redirect is not followed and a proxy's page is not the server's
        // answer: the statement's own id, or an exception code for a request
        // refused before it had one, is what says ClickHouse answered.
        let ours = text("x-clickhouse-query-id").as_deref() == Some(call.query_id.as_str());
        let code: Option<u32> =
            text("x-clickhouse-exception-code").and_then(|code| code.parse().ok());
        if !ours && (response.status() == StatusCode::OK || code.is_none()) {
            return Err(Failure::Server(ServerError {
                code: None,
                message: NOT_THE_SERVER.to_owned(),
                written_rows: None,
            }));
        }
        if response.status() != StatusCode::OK {
            let body = read_bounded(response, MAX_MESSAGE_BYTES).await;
            return Err(Failure::Server(ServerError::from_text(
                code,
                &body,
                written_rows,
            )));
        }
        Ok(Reply {
            format: text("x-clickhouse-format"),
            zone: text("x-clickhouse-timezone"),
            exception_tag: text("x-clickhouse-exception-tag"),
            written_rows,
            response,
        })
    }

    fn credentials(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Ok(user) = HeaderValue::from_str(self.spec.user()) {
            headers.insert("x-clickhouse-user", user);
        }
        if let Some(password) = self.spec.password()
            && let Ok(mut key) = HeaderValue::from_str(password)
        {
            key.set_sensitive(true);
            headers.insert("x-clickhouse-key", key);
        }
        headers
    }
}

/// Why a request failed before its body could be read.
#[derive(Debug)]
pub enum Failure {
    /// The exchange itself failed.
    Transport(reqwest::Error),
    /// The server answered with an exception.
    Server(ServerError),
}

/// A query parameter's value, escaped the way the server unescapes it.
///
/// ClickHouse 26.3 reads a `param_*` value with the escapes of its TSV
/// format: `a\b` arrives as `a` and a backspace, and a raw tab or line feed
/// is refused (observed, RESEARCH-NOTES). A table named `x\` would then be
/// looked up as another name — or as a name an attacker chose. Every
/// backslash and control character the format escapes is written as its
/// escape, so the value the server binds is the one the driver holds.
fn escape_param(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => escaped.push_str(r"\\"),
            '\t' => escaped.push_str(r"\t"),
            '\n' => escaped.push_str(r"\n"),
            '\r' => escaped.push_str(r"\r"),
            '\0' => escaped.push_str(r"\0"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// `written_rows` from an `X-ClickHouse-Summary` JSON object, whose values the
/// server quotes as strings.
fn written_rows(summary: &str) -> Option<u64> {
    let value: serde_json::Value = serde_json::from_str(summary).ok()?;
    match value.get("written_rows")? {
        serde_json::Value::String(text) => text.parse().ok(),
        serde_json::Value::Number(number) => number.as_u64(),
        _ => None,
    }
}

/// Reads at most `limit` bytes of a body, for an error message.
async fn read_bounded(mut response: Response, limit: usize) -> Vec<u8> {
    let mut out = Vec::new();
    while let Ok(Some(chunk)) = response.chunk().await {
        let room = limit.saturating_sub(out.len());
        out.extend_from_slice(chunk.get(..room.min(chunk.len())).unwrap_or(&[]));
        if out.len() >= limit {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parameter_value_reaches_the_server_as_the_driver_holds_it() {
        assert_eq!(escape_param(r"x\`; DROP"), r"x\\`; DROP");
        assert_eq!(escape_param("a\tb\nc\rd\0e"), r"a\tb\nc\rd\0e");
        assert_eq!(escape_param("plain é 'q'"), "plain é 'q'");
    }

    #[test]
    fn the_summary_gives_the_written_rows() {
        assert_eq!(
            written_rows(r#"{"read_rows":"1","written_rows":"49000","elapsed_ns":"1"}"#),
            Some(49_000)
        );
        assert_eq!(written_rows(r#"{"written_rows":7}"#), Some(7));
        assert_eq!(written_rows("not json"), None);
        assert_eq!(written_rows(r#"{"read_rows":"1"}"#), None);
    }

    /// A server that answers every request with `response`, and counts the
    /// connections it accepted.
    async fn canned(
        response: &'static str,
    ) -> (u16, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a local port");
        let port = listener.local_addr().expect("bound").port();
        let accepted = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = accepted.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let mut request = vec![0; 4096];
                let _ = socket.read(&mut request).await;
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        (port, accepted)
    }

    fn http_to(port: u16) -> Http {
        let mut config =
            oxyn_core::ConnectionConfig::new("canned", oxyn_core::DriverId::clickhouse())
                .with_environment(oxyn_core::Environment::Local);
        for (key, value) in [
            ("host", "127.0.0.1".to_owned()),
            ("port", port.to_string()),
            ("user", "app".to_owned()),
            ("sslmode", "disable".to_owned()),
        ] {
            config = config.with_param(key, value);
        }
        let credentials = oxyn_driver::Credentials::new()
            .with_password(secrecy::SecretString::from("hunter2-secret"));
        let spec =
            ConnectSpec::from_config(&crate::driver::clickhouse_metadata(), &config, &credentials)
                .expect("valid");
        Http::new(spec).expect("a client")
    }

    #[tokio::test]
    async fn an_answer_that_is_not_the_servers_is_never_believed_nor_followed() {
        for response in [
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/login\r\nContent-Length: 0\r\n\r\n",
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok",
            "HTTP/1.1 502 Bad Gateway\r\nContent-Length: 4\r\n\r\ngone",
        ] {
            let (port, accepted) = canned(response).await;
            let call = Call {
                text: "INSERT INTO t VALUES (1)".to_owned(),
                query_id: "ours".to_owned(),
                ..Call::default()
            };
            let outcome = http_to(port).send(&call).await;
            let Err(Failure::Server(error)) = outcome else {
                panic!("believed: {response}");
            };
            assert_eq!(error.message, NOT_THE_SERVER);
            assert_eq!(error.code, None);
            assert_eq!(
                accepted.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "followed: {response}"
            );
        }
    }
}
