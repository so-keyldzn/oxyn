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

/// The output format of every statement: names and types announced first,
/// then rows ([`crate::types`]).
pub const FORMAT: &str = "RowBinaryWithNamesAndTypes";

/// How long opening the TCP and TLS connection may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// A request the driver sends.
#[derive(Debug, Clone, Default)]
pub struct Call {
    /// The statement, as the user or the driver wrote it.
    pub text: String,
    /// The id `KILL QUERY` targets.
    pub query_id: String,
    /// The database unqualified names resolve in.
    pub database: Option<String>,
    /// Server settings for this request only.
    pub settings: Vec<(&'static str, String)>,
    /// Query parameters, bound by the server to `{name:Type}` placeholders.
    pub params: Vec<(&'static str, String)>,
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
        let client = Client::builder()
            .no_proxy()
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
                query.append_pair(&format!("param_{name}"), value);
            }
        }
        let response = self
            .client
            .post(url)
            .headers(self.credentials())
            .body(call.text.clone())
            .send()
            .await
            .map_err(Failure::Transport)?;

        let headers = response.headers();
        let text = |name: &str| {
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        let written_rows = text("x-clickhouse-summary").and_then(|summary| written_rows(&summary));
        if response.status() != StatusCode::OK {
            let code = text("x-clickhouse-exception-code").and_then(|code| code.parse().ok());
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
    fn the_summary_gives_the_written_rows() {
        assert_eq!(
            written_rows(r#"{"read_rows":"1","written_rows":"49000","elapsed_ns":"1"}"#),
            Some(49_000)
        );
        assert_eq!(written_rows(r#"{"written_rows":7}"#), Some(7));
        assert_eq!(written_rows("not json"), None);
        assert_eq!(written_rows(r#"{"read_rows":"1"}"#), None);
    }
}
